use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use repo_com_config::{ResolvedConfig, parse_config, resolve_model};
use repo_com_inbox_state::{InboxState, RepositoryInput};
use serde_json::json;
use tokio::runtime::Builder;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

use crate::fetch::{
    InboundReader, RemoteMessagePage, fetch_and_store_with_reader, fetch_with_reader,
};
use crate::{
    AcceptedDelivery, AttachmentIndicator, DiscordInboundClient, FetchBoundary, FetchError,
    FetchProvenance, FetchRequest, FilterContext, ReadError, ReadRateLimitScope, RemoteMessage,
    ReplyEvidence, filter_messages, rfc3339_to_discord_snowflake, should_retain,
    to_untrusted_envelope,
};

const REPOSITORY_ID: &str = "repo-a";
const OTHER_REPOSITORY_ID: &str = "repo-b";
const WORKSPACE_ID: &str = "123456789012345678";
const CHANNEL_ID: &str = "234567890123456789";
const BOT_USER_ID: &str = "345678901234567890";
const SYNTHETIC_BOT_TOKEN: &str = "c3ludGhldGljLWluYm94LWJvdC10b2tlbi5zeW50aGV0aWMtc2lnbmF0dXJl.c3ludGhldGljLWNvbnRyYWN0.c2lnbmF0dXJl";
const OBSERVED_AT: &str = "2026-01-01T00:00:10.000Z";
const CONFIG_TOML: &str = r#"
schema_version = 1
repository_id = "repo-a"
auto_send = []

[discord]
workspace_id = "123456789012345678"

[destinations.release]
channel_id = "234567890123456789"
allowed_mentions = []

[mentions]

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365
"#;

#[derive(Clone)]
struct FakeReader {
    pages: Arc<Mutex<Vec<RemoteMessagePage>>>,
    points: Arc<Mutex<HashMap<String, Result<RemoteMessage, ReadError>>>>,
    page_calls: Arc<Mutex<Vec<String>>>,
    point_calls: Arc<Mutex<Vec<String>>>,
}

impl FakeReader {
    fn new(
        pages: Vec<RemoteMessagePage>,
        points: HashMap<String, Result<RemoteMessage, ReadError>>,
    ) -> Self {
        Self {
            pages: Arc::new(Mutex::new(pages)),
            points: Arc::new(Mutex::new(points)),
            page_calls: Arc::new(Mutex::new(Vec::new())),
            point_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn page_call_count(&self) -> usize {
        self.page_calls.lock().expect("page calls lock").len()
    }

    fn point_call_count(&self) -> usize {
        self.point_calls.lock().expect("point calls lock").len()
    }
}

impl InboundReader for FakeReader {
    async fn read_page(
        &self,
        channel_id: &str,
        after: &str,
    ) -> Result<RemoteMessagePage, ReadError> {
        assert_eq!(channel_id, CHANNEL_ID);
        self.page_calls
            .lock()
            .expect("page calls lock")
            .push(after.to_owned());
        let mut pages = self.pages.lock().expect("pages lock");
        if pages.is_empty() {
            return Err(ReadError::NotFound);
        }
        Ok(pages.remove(0))
    }

    async fn read_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<RemoteMessage, ReadError> {
        assert_eq!(channel_id, CHANNEL_ID);
        self.point_calls
            .lock()
            .expect("point calls lock")
            .push(message_id.to_owned());
        self.points
            .lock()
            .expect("points lock")
            .get(message_id)
            .cloned()
            .ok_or(ReadError::NotFound)?
    }
}

fn config() -> ResolvedConfig {
    let parsed = parse_config(Path::new("inbox-fetch-contract.toml"), CONFIG_TOML)
        .expect("valid synthetic configuration");
    resolve_model(&parsed, Path::new("inbox-fetch-contract.toml"))
        .expect("resolved synthetic configuration")
}

fn request(boundary: FetchBoundary) -> FetchRequest {
    FetchRequest::new(REPOSITORY_ID, "release", boundary, BOT_USER_ID)
        .with_retrieved_at(OBSERVED_AT)
}

fn mention_message(id: u64, author: &str, text: &str) -> RemoteMessage {
    RemoteMessage::new(
        id.to_string(),
        CHANNEL_ID,
        author,
        text,
        "2026-01-01T00:00:01.000Z",
    )
    .with_mentions([BOT_USER_ID.to_owned()])
}

fn reply_message(id: u64, author: &str, referenced_id: &str) -> RemoteMessage {
    mention_message(id, author, "a human reply").with_reply(ReplyEvidence {
        reference_present: true,
        referenced_message_id: Some(referenced_id.to_owned()),
        referenced_channel_id: Some(CHANNEL_ID.to_owned()),
        referenced_guild_id: Some(WORKSPACE_ID.to_owned()),
        referenced_message_deleted: false,
    })
}

fn filter_context(items: &[AcceptedDelivery]) -> FilterContext<'_> {
    FilterContext::new(REPOSITORY_ID, CHANNEL_ID, BOT_USER_ID, items).expect("filter context")
}

fn register(state: &mut InboxState) {
    state
        .register_repository(&RepositoryInput::new(
            REPOSITORY_ID,
            WORKSPACE_ID,
            "synthetic-config-hash",
            OBSERVED_AT,
        ))
        .expect("register repository");
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
}

#[test]
fn boundary_requires_exactly_one_cursor_or_time() {
    assert!(FetchBoundary::from_parts(Some("100"), None).is_ok());
    assert!(FetchBoundary::from_parts(None, Some(OBSERVED_AT)).is_ok());
    assert!(FetchBoundary::from_parts(Some("100"), Some(OBSERVED_AT)).is_err());
    assert!(FetchBoundary::from_parts(None, None).is_err());
    assert!(FetchBoundary::time("not-a-time").is_err());
}

#[test]
fn unknown_disabled_and_raw_channel_aliases_fail_before_http() {
    runtime().block_on(async {
        let reader = FakeReader::new(
            vec![RemoteMessagePage {
                messages: vec![mention_message(5000, "human", "body")],
                has_more: Some(false),
                rate_limit: None,
            }],
            HashMap::new(),
        );
        let unknown = fetch_with_reader(
            &reader,
            &config(),
            &FetchRequest::new(
                REPOSITORY_ID,
                "not-configured",
                FetchBoundary::cursor("1").expect("cursor"),
                BOT_USER_ID,
            ),
            &[],
        )
        .await
        .expect_err("unknown alias");
        assert!(matches!(unknown, FetchError::UnknownAlias));

        let mut disabled = config();
        disabled
            .inbound
            .get_mut("release")
            .expect("release alias")
            .enabled = false;
        let disabled = fetch_with_reader(
            &reader,
            &disabled,
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
        )
        .await
        .expect_err("disabled alias");
        assert!(matches!(disabled, FetchError::DisabledAlias));

        let raw_channel = fetch_with_reader(
            &reader,
            &config(),
            &FetchRequest::new(
                REPOSITORY_ID,
                CHANNEL_ID,
                FetchBoundary::cursor("1").expect("cursor"),
                BOT_USER_ID,
            ),
            &[],
        )
        .await
        .expect_err("raw channel alias");
        assert!(matches!(raw_channel, FetchError::UnknownAlias));
        assert_eq!(reader.page_call_count(), 0);
    });
}

#[test]
fn rate_limit_and_authentication_errors_are_typed_and_redacted() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v10/channels/{CHANNEL_ID}/messages")))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "7")
                    .insert_header("x-ratelimit-scope", "shared")
                    .insert_header("x-ratelimit-bucket", "synthetic-bucket")
                    .set_body_json(json!({
                        "message": "sensitive-rate-limit-body",
                        "retry_after": 9.5,
                        "global": false
                    })),
            )
            .mount(&server)
            .await;
        let client =
            DiscordInboundClient::from_synthetic_token_for_test(&server.uri(), SYNTHETIC_BOT_TOKEN)
                .expect("synthetic client");
        let error = fetch_with_reader(
            &client,
            &config(),
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
        )
        .await
        .expect_err("rate limited");
        let FetchError::Read(ReadError::RateLimited { info }) = error else {
            panic!("expected typed rate limit");
        };
        assert_eq!(info.retry_after, Some(Duration::from_secs_f64(9.5)));
        assert_eq!(info.scope, ReadRateLimitScope::Shared);
        assert_eq!(info.bucket.as_deref(), Some("synthetic-bucket"));
        let diagnostics = format!("{info:?}");
        assert!(!diagnostics.contains("sensitive-rate-limit-body"));
        assert!(!diagnostics.contains(SYNTHETIC_BOT_TOKEN));

        let unauthorized_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v10/channels/{CHANNEL_ID}/messages")))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "message": "sensitive-auth-body"
            })))
            .mount(&unauthorized_server)
            .await;
        let unauthorized_client = DiscordInboundClient::from_synthetic_token_for_test(
            &unauthorized_server.uri(),
            SYNTHETIC_BOT_TOKEN,
        )
        .expect("synthetic client");
        let error = fetch_with_reader(
            &unauthorized_client,
            &config(),
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
        )
        .await
        .expect_err("authentication failure");
        assert!(matches!(error, FetchError::Read(ReadError::Authentication)));
        assert!(!format!("{error:?} {error}").contains("sensitive-auth-body"));
        assert!(!format!("{error:?} {error}").contains(SYNTHETIC_BOT_TOKEN));
    });
}

#[test]
fn filter_retains_only_human_reply_or_direct_bot_mention() {
    let accepted =
        vec![AcceptedDelivery::new(REPOSITORY_ID, CHANNEL_ID, "900").expect("accepted delivery")];
    let cross_repository = vec![
        AcceptedDelivery::new(OTHER_REPOSITORY_ID, CHANNEL_ID, "900")
            .expect("cross-repository delivery"),
    ];
    let context = filter_context;

    let reply = reply_message(1000, "human-1", "900");
    let mention = mention_message(1001, "human-2", "hello");
    let unrelated = RemoteMessage::new("1002", CHANNEL_ID, "human-3", "hello", OBSERVED_AT);
    let other_bot = mention_message(1003, "other-bot", "hello").bot();
    let repo_com = mention_message(1004, BOT_USER_ID, "hello");
    let cross_repo_reply =
        RemoteMessage::new("1005", CHANNEL_ID, "human-4", "a human reply", OBSERVED_AT).with_reply(
            ReplyEvidence {
                reference_present: true,
                referenced_message_id: Some("900".to_owned()),
                referenced_channel_id: Some(CHANNEL_ID.to_owned()),
                referenced_guild_id: Some(WORKSPACE_ID.to_owned()),
                referenced_message_deleted: false,
            },
        );

    assert!(should_retain(&reply, &context(&accepted)));
    assert!(should_retain(&mention, &context(&accepted)));
    assert!(!should_retain(&unrelated, &context(&accepted)));
    assert!(!should_retain(&other_bot, &context(&accepted)));
    assert!(!should_retain(&repo_com, &context(&accepted)));
    assert!(!should_retain(
        &cross_repo_reply,
        &context(&cross_repository)
    ));
}

#[test]
fn envelope_is_explicitly_untrusted_and_contains_no_attachment_bytes() {
    let accepted =
        vec![AcceptedDelivery::new(REPOSITORY_ID, CHANNEL_ID, "900").expect("accepted delivery")];
    let context = FilterContext::new(REPOSITORY_ID, CHANNEL_ID, BOT_USER_ID, &accepted)
        .expect("filter context");
    let message =
        mention_message(1000, "human-1", "private text").with_attachments([AttachmentIndicator {
            id: "attachment-1".to_owned(),
            filename: "private.txt".to_owned(),
            content_type: Some("text/plain".to_owned()),
            size: Some(12),
        }]);
    let envelope = to_untrusted_envelope(
        &message,
        &context,
        FetchProvenance::discord(
            REPOSITORY_ID,
            WORKSPACE_ID,
            "release",
            CHANNEL_ID,
            OBSERVED_AT,
        ),
    );
    assert!(envelope.is_untrusted());
    assert_eq!(envelope.text, "private text");
    assert!(envelope.provenance.provider == "discord");
    assert!(envelope.provenance.api_version == "v10");
    assert!(envelope.mentions.direct_bot_mention());
    let json = serde_json::to_string(&envelope).expect("serialize envelope");
    assert!(json.contains("untrusted"));
    assert!(!json.contains("attachment bytes"));
    assert!(!json.contains("\"url\""));
    assert!(!format!("{envelope:?}").contains("private text"));
}

#[test]
fn filter_preserves_deterministic_remote_order() {
    let messages = vec![
        mention_message(1002, "human-2", "second"),
        mention_message(1000, "human-1", "first"),
        mention_message(1001, "human-3", "middle"),
    ];
    let accepted = Vec::new();
    let context = FilterContext::new(REPOSITORY_ID, CHANNEL_ID, BOT_USER_ID, &accepted)
        .expect("filter context");
    let filtered = filter_messages(&messages, &context, |message| {
        FetchProvenance::discord(
            REPOSITORY_ID,
            WORKSPACE_ID,
            "release",
            message.channel_id.clone(),
            OBSERVED_AT,
        )
    });
    let ids = filtered
        .iter()
        .map(|item| item.remote_message_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["1000", "1001", "1002"]);
}

#[test]
fn fetch_stops_at_ten_pages_and_exposes_page_continuation() {
    runtime().block_on(async {
        let mut pages = Vec::new();
        for page_index in 0..10_u64 {
            let messages = (0..1)
                .map(|offset| mention_message(1_000 + page_index * 10 + offset, "human", "body"))
                .collect();
            pages.push(RemoteMessagePage {
                messages,
                has_more: Some(true),
                rate_limit: None,
            });
        }
        let reader = FakeReader::new(pages, HashMap::new());
        let result = fetch_with_reader(
            &reader,
            &config(),
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
        )
        .await
        .expect("bounded fetch");
        assert_eq!(result.pages_fetched, 10);
        assert_eq!(result.raw_messages, 10);
        assert_eq!(result.items.len(), 10);
        assert!(result.has_continuation());
        assert!(result.continuation.page_limit_reached);
        assert!(!result.continuation.message_limit_reached);
        assert_eq!(reader.page_call_count(), 10);
        assert_eq!(result.continuation.next_cursor.as_deref(), Some("1090"));
    });
}

#[test]
fn fetch_stops_at_one_thousand_raw_messages_and_exposes_message_continuation() {
    runtime().block_on(async {
        let mut pages = Vec::new();
        for page_index in 0..10_u64 {
            let messages = (0..100_u64)
                .map(|offset| mention_message(10_000 + page_index * 100 + offset, "human", "body"))
                .collect();
            pages.push(RemoteMessagePage {
                messages,
                has_more: Some(true),
                rate_limit: None,
            });
        }
        let reader = FakeReader::new(pages, HashMap::new());
        let result = fetch_with_reader(
            &reader,
            &config(),
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
        )
        .await
        .expect("bounded fetch");
        assert_eq!(result.pages_fetched, 10);
        assert_eq!(result.raw_messages, 1_000);
        assert_eq!(result.items.len(), 1_000);
        assert!(result.continuation.message_limit_reached);
        assert!(result.continuation.page_limit_reached);
        assert_eq!(
            result.continuation.reason,
            crate::ContinuationReason::BothLimits
        );
    });
}

#[test]
fn point_reconciliation_preserves_first_snapshot_and_records_edit_and_delete() {
    runtime().block_on(async {
        let first = mention_message(2000, "human-1", "first text");
        let edited = RemoteMessage::new(
            "2000",
            CHANNEL_ID,
            "human-1",
            "edited text",
            "2026-01-01T00:00:01.000Z",
        )
        .with_mentions([BOT_USER_ID.to_owned()]);
        let deleted = RemoteMessage::new(
            "2001",
            CHANNEL_ID,
            "human-1",
            "to delete",
            "2026-01-01T00:00:01.000Z",
        )
        .with_mentions([BOT_USER_ID.to_owned()]);
        let pages = vec![RemoteMessagePage {
            messages: vec![first.clone(), deleted.clone()],
            has_more: Some(false),
            rate_limit: None,
        }];
        let points = HashMap::from([
            ("2000".to_owned(), Ok(edited)),
            ("2001".to_owned(), Err(ReadError::NotFound)),
        ]);
        let reader = FakeReader::new(pages, points);
        let mut state = InboxState::open_in_memory().expect("state");
        register(&mut state);
        let stored = fetch_and_store_with_reader(
            &reader,
            &config(),
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
            &mut state,
        )
        .await
        .expect("store and reconcile");
        assert_eq!(stored.reconciliation.attempted, 2);
        assert_eq!(stored.reconciliation.edited_count(), 1);
        assert_eq!(stored.reconciliation.deleted_count(), 1);
        assert_eq!(
            state
                .item(REPOSITORY_ID, "2000")
                .expect("first item")
                .expect("first row")
                .first_content,
            "first text"
        );
        assert_eq!(
            state
                .current(REPOSITORY_ID, "2000")
                .expect("current item")
                .expect("current row")
                .current_content
                .as_deref(),
            Some("edited text")
        );
        assert!(
            state
                .current(REPOSITORY_ID, "2001")
                .expect("deleted current")
                .expect("deleted row")
                .deleted
        );
        assert_eq!(
            state
                .item(REPOSITORY_ID, "2001")
                .expect("first deleted")
                .expect("first deleted row")
                .first_content,
            "to delete"
        );
    });
}

#[test]
fn reconciliation_stops_after_one_hundred_point_checks() {
    runtime().block_on(async {
        let mut points = HashMap::new();
        let mut first_page = Vec::new();
        for index in 0..100_u64 {
            let id = (3_000 + index).to_string();
            let message = mention_message(3_000 + index, "human", "body");
            points.insert(id.clone(), Ok(message.clone()));
            first_page.push(message);
        }
        let second = mention_message(3_100, "human", "body");
        points.insert("3100".to_owned(), Ok(second.clone()));
        let reader = FakeReader::new(
            vec![
                RemoteMessagePage {
                    messages: first_page,
                    has_more: Some(true),
                    rate_limit: None,
                },
                RemoteMessagePage {
                    messages: vec![second],
                    has_more: Some(false),
                    rate_limit: None,
                },
            ],
            points,
        );
        let mut state = InboxState::open_in_memory().expect("state");
        register(&mut state);
        let stored = fetch_and_store_with_reader(
            &reader,
            &config(),
            &request(FetchBoundary::cursor("1").expect("cursor")),
            &[],
            &mut state,
        )
        .await
        .expect("store and bounded reconcile");
        assert_eq!(stored.reconciliation.attempted, 100);
        assert!(stored.reconciliation.has_continuation());
        assert_eq!(reader.point_call_count(), 100);
        assert_eq!(
            stored
                .reconciliation
                .continuation
                .as_ref()
                .expect("probe continuation")
                .remaining_item_ids,
            vec!["3100"]
        );
    });
}

#[test]
fn wiremock_fixture_uses_only_v10_configured_gets_and_bot_auth() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v10/channels/{CHANNEL_ID}/messages")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": "4000",
                "channel_id": CHANNEL_ID,
                "content": "human response",
                "timestamp": "2026-01-01T00:00:01.000Z",
                "author": {"id": "human-1", "bot": false},
                "mentions": [{"id": BOT_USER_ID}],
                "attachments": [{
                    "id": "attachment-1",
                    "filename": "private.txt",
                    "content_type": "text/plain",
                    "size": 4,
                    "url": "https://cdn.example.invalid/private.txt"
                }],
                "type": 0
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v10/channels/{CHANNEL_ID}/messages/4000"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "4000",
                "channel_id": CHANNEL_ID,
                "content": "edited response",
                "timestamp": "2026-01-01T00:00:01.000Z",
                "edited_timestamp": "2026-01-01T00:00:02.000Z",
                "author": {"id": "human-1", "bot": false},
                "mentions": [{"id": BOT_USER_ID}]
            })))
            .mount(&server)
            .await;

        let client =
            DiscordInboundClient::from_synthetic_token_for_test(&server.uri(), SYNTHETIC_BOT_TOKEN)
                .expect("synthetic client");
        let mut state = InboxState::open_in_memory().expect("state");
        register(&mut state);
        let result = fetch_and_store_with_reader(
            &client,
            &config(),
            &request(FetchBoundary::cursor("100").expect("cursor")),
            &[],
            &mut state,
        )
        .await
        .expect("wiremock fetch and reconcile");
        assert_eq!(result.fetch.items.len(), 1);
        assert_eq!(result.reconciliation.edited_count(), 1);
        let requests = server.received_requests().await.expect("request journal");
        assert_eq!(requests.len(), 2);
        for request in &requests {
            assert_allowed_request(request);
        }
        assert!(requests[0].url.path().ends_with("/messages"));
        assert_eq!(requests[0].url.query(), Some("limit=100&after=100"));
        assert!(requests[1].url.path().ends_with("/messages/4000"));
        assert_eq!(
            state
                .item(REPOSITORY_ID, "4000")
                .expect("first item")
                .expect("first row")
                .first_content,
            "human response"
        );
        assert_eq!(
            state
                .current(REPOSITORY_ID, "4000")
                .expect("current item")
                .expect("current row")
                .current_content
                .as_deref(),
            Some("edited response")
        );
    });
}

#[test]
fn time_boundary_is_encoded_as_a_v10_after_cursor() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v10/channels/{CHANNEL_ID}/messages")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        let client =
            DiscordInboundClient::from_synthetic_token_for_test(&server.uri(), SYNTHETIC_BOT_TOKEN)
                .expect("synthetic client");
        let request = FetchRequest::with_time(REPOSITORY_ID, "release", OBSERVED_AT, BOT_USER_ID)
            .expect("time request")
            .with_retrieved_at(OBSERVED_AT);
        let result = fetch_with_reader(&client, &config(), &request, &[])
            .await
            .expect("time fetch");
        assert!(!result.has_continuation());
        let requests = server.received_requests().await.expect("request journal");
        assert_eq!(requests.len(), 1);
        let expected = rfc3339_to_discord_snowflake(OBSERVED_AT).expect("time cursor");
        assert_eq!(
            requests[0].url.query(),
            Some(format!("limit=100&after={expected}").as_str())
        );
        assert_allowed_request(&requests[0]);
    });
}

fn assert_allowed_request(request: &Request) {
    assert_eq!(request.method, reqwest::Method::GET);
    assert!(request.url.path().starts_with("/api/v10/"));
    assert!(
        request
            .url
            .path()
            .contains("/channels/234567890123456789/messages"),
        "{}",
        request.url.path()
    );
    assert!(!request.url.path().contains("/gateway"));
    assert!(!request.url.path().contains("/webhooks"));
    assert!(!request.url.path().contains("/reactions"));
    assert!(!request.url.path().ends_with("/edit"));
    assert!(!request.url.path().ends_with("/delete"));
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some(format!("Bot {SYNTHETIC_BOT_TOKEN}").as_str())
    );
    assert!(request.body.is_empty());
}
