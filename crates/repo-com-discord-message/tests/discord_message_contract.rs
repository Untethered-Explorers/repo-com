use std::{collections::BTreeMap, net::TcpListener, path::Path, process::Command, time::Duration};

use repo_com_config::{
    DestinationConfig, DiscordConfig, MentionConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};
use repo_com_draft_content::{
    ContentRenderer, MAX_DISCORD_MESSAGE_CHARACTERS, MentionError, NONCE_FOOTER_PREFIX, RenderError,
};
use repo_com_draft_model::{AuthorizedReplyReference, DraftModel, DraftRequest};
use repo_com_foundation::ErrorCategory;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    AmbiguousReason, CreateMessageRequest, DiscordApiVersion, DiscordMessageClient,
    MAX_DISCORD_DIRECTED_WAIT, MessageError, MessageTimeouts, PreDispatchFailure, RateLimitClass,
    RateLimitScope, RequestError, SendCertainty, TransportFailure, classify_transport_failure,
};

const CASE_ENV: &str = "REPO_COM_DISCORD_MESSAGE_CONTRACT_CASE";
const BASE_URL_ENV: &str = "REPO_COM_DISCORD_MESSAGE_CONTRACT_BASE_URL";
const BOT_TOKEN_ENV: &str = "REPO_COM_DISCORD_TOKEN";
const SYNTHETIC_BOT_TOKEN: &str =
    "c3ludGhldGljLW1lc3NhZ2UtY29udHJhY3QtYm90LXRva2Vu.c3ludGhldGljLW1lc3NhZ2U.c2lnbmF0dXJl";
const SENSITIVE_RESPONSE_MARKER: &str =
    "synthetic-sensitive-message-response-must-never-enter-diagnostics";
const REPOSITORY_ID: &str = "acme/widgets";
const WORKSPACE_ID: &str = "123456789012345678";
const CHANNEL_ID: &str = "234567890123456789";
const ROLE_ID: &str = "345678901234567890";
const USER_ID: &str = "456789012345678901";
const REPLY_MESSAGE_ID: &str = "567890123456789012";
const CREATED_MESSAGE_ID: &str = "678901234567890123";
const SNAPSHOT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CREATED_AT: u64 = 1_000;

#[test]
fn one_send_uses_exact_v10_post_text_nonce_and_allowlisted_mentions() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_created_message(&server, 200, created_message_body(), &[]).await;

        run_child(&server, "success");

        assert_one_create_request(&server, false).await;
    });
}

#[test]
fn validated_reply_draft_sends_only_the_message_reference_metadata() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_created_message(&server, 200, created_message_body(), &[]).await;

        run_child(&server, "reply");

        assert_one_create_request(&server, true).await;
    });
}

#[test]
fn request_rejects_unlisted_mentions_and_oversized_text_before_http_io() {
    runtime().block_on(async {
        let server = MockServer::start().await;

        run_child(&server, "unlisted-mention");
        run_child(&server, "oversized");

        assert!(
            server
                .received_requests()
                .await
                .expect("request journal")
                .is_empty()
        );
    });
}

#[test]
fn request_surface_rejects_unpinned_api_versions_and_raw_endpoint_paths() {
    assert_eq!(
        DiscordApiVersion::try_from("v9"),
        Err(RequestError::UnsupportedApiVersion)
    );
    assert_eq!(
        DiscordApiVersion::try_from("v10"),
        Ok(DiscordApiVersion::V10)
    );

    runtime().block_on(async {
        let server = MockServer::start().await;
        run_child(&server, "invalid-endpoint");
        assert!(
            server
                .received_requests()
                .await
                .expect("request journal")
                .is_empty()
        );
    });
}

#[test]
fn proactive_route_headers_are_returned_with_server_timing() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_created_message(
            &server,
            200,
            created_message_body(),
            &[
                ("X-RateLimit-Limit", "5"),
                ("X-RateLimit-Remaining", "0"),
                ("X-RateLimit-Reset", "1470173023.123"),
                ("X-RateLimit-Reset-After", "1.75"),
                ("X-RateLimit-Bucket", "synthetic-route-bucket"),
                ("X-RateLimit-Global", "false"),
            ],
        )
        .await;

        run_child(&server, "route-rate-limit");

        assert_one_create_request(&server, false).await;
    });
}

#[test]
fn global_user_and_shared_429_scopes_use_dynamic_capped_retry_timing() {
    runtime().block_on(async {
        for (case, scope) in [
            ("global-429", RateLimitScope::Global),
            ("user-429", RateLimitScope::User),
            ("shared-429", RateLimitScope::Shared),
        ] {
            let server = MockServer::start().await;
            let headers: &[(&str, &str)] = match case {
                "global-429" => &[
                    ("Retry-After", "17.25"),
                    ("X-RateLimit-Global", "true"),
                    ("X-RateLimit-Scope", "global"),
                ],
                "user-429" => &[("X-RateLimit-Scope", "user")],
                _ => &[("Retry-After", "45"), ("X-RateLimit-Scope", "shared")],
            };
            mount_created_message(
                &server,
                429,
                json!({
                    "message": SENSITIVE_RESPONSE_MARKER,
                    "retry_after": match case {
                        "global-429" => 16.5,
                        "user-429" => 6.5,
                        _ => 99.0,
                    },
                    "global": case == "global-429"
                }),
                headers,
            )
            .await;

            run_child(&server, case);

            let requests = server.received_requests().await.expect("request journal");
            assert_eq!(requests.len(), 1, "{case}");
            assert_eq!(requests[0].method, reqwest::Method::POST, "{case}");
            assert!(matches!(
                scope,
                RateLimitScope::Global | RateLimitScope::User | RateLimitScope::Shared
            ));
        }
    });
}

#[test]
fn focused_http_errors_are_typed_redacted_and_never_retried() {
    runtime().block_on(async {
        for case in [
            "status-400",
            "status-401",
            "status-403",
            "status-404",
            "status-409",
            "status-503",
        ] {
            let server = MockServer::start().await;
            let status = case
                .strip_prefix("status-")
                .and_then(|value| value.parse::<u16>().ok())
                .expect("numeric fixture status");
            mount_created_message(
                &server,
                status,
                json!({
                    "message": SENSITIVE_RESPONSE_MARKER,
                    "credential_echo": SENSITIVE_RESPONSE_MARKER
                }),
                &[],
            )
            .await;

            run_child(&server, case);

            let requests = server.received_requests().await.expect("request journal");
            assert_eq!(requests.len(), 1, "{case}");
            assert_eq!(requests[0].method, reqwest::Method::POST, "{case}");
        }
    });
}

#[test]
fn connect_failure_is_proven_before_dispatch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
    let address = listener.local_addr().expect("loopback address");
    drop(listener);
    let base_url = format!("http://{address}");
    run_child_with_base("connect-failure", &base_url);
}

#[test]
fn pre_dispatch_timeout_is_distinct_from_post_dispatch_timeout() {
    let pre_dispatch = classify_transport_failure(TransportFailure::ConnectTimeout);
    assert_eq!(
        pre_dispatch,
        MessageError::PreDispatch {
            failure: PreDispatchFailure::ConnectTimeout
        }
    );
    assert_eq!(pre_dispatch.certainty(), SendCertainty::ProvenNotSent);
    assert!(pre_dispatch.was_proven_not_sent());

    let post_dispatch = classify_transport_failure(TransportFailure::AmbiguousTimeout);
    assert_eq!(
        post_dispatch,
        MessageError::Ambiguous {
            reason: AmbiguousReason::Timeout
        }
    );
    assert_eq!(post_dispatch.certainty(), SendCertainty::Ambiguous);
    assert!(post_dispatch.is_ambiguous());
}

#[test]
fn delayed_post_dispatch_timeout_is_ambiguous_and_never_retried() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(message_path()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(250))
                    .set_body_json(json!({
                        "id": CREATED_MESSAGE_ID,
                        "channel_id": CHANNEL_ID,
                        "message": SENSITIVE_RESPONSE_MARKER
                    })),
            )
            .mount(&server)
            .await;

        run_child(&server, "post-dispatch-timeout");

        let requests = server.received_requests().await.expect("request journal");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, reqwest::Method::POST);
    });
}

#[test]
fn bearer_credentials_and_missing_environment_token_are_rejected_before_http_io() {
    runtime().block_on(async {
        let server = MockServer::start().await;

        run_child_with_token("bearer-token", "Bearer synthetic-user-token");
        run_child_without_token("missing-token");

        assert!(
            server
                .received_requests()
                .await
                .expect("request journal")
                .is_empty()
        );
    });
}

#[test]
fn request_rejects_attachments_embeds_and_unlisted_mention_fields() {
    let rendered = rendered_message(false, "Build failed: @oncall @owner");
    let request = CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
    let body = serde_json::to_value(request.wire_payload()).expect("request JSON");

    assert_eq!(
        body.as_object()
            .expect("request object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["allowed_mentions", "content", "nonce"]
    );
    assert!(body.get("attachments").is_none());
    assert!(body.get("embeds").is_none());
    assert!(body.get("components").is_none());
    assert!(body.get("poll").is_none());
    assert_eq!(
        body["allowed_mentions"],
        json!({
            "parse": [],
            "roles": [ROLE_ID],
            "users": [USER_ID],
            "replied_user": false
        })
    );
    assert_eq!(body["content"], rendered.exact_text());
    assert_eq!(
        body["nonce"],
        rendered.nonce(),
        "the request nonce must match the immutable revision"
    );
    assert!(
        body["content"]
            .as_str()
            .expect("text")
            .ends_with(&format!("{NONCE_FOOTER_PREFIX}{}", rendered.nonce()))
    );
    assert_eq!(request.destination_alias(), "release");
    assert_eq!(request.channel_id(), CHANNEL_ID);
}

#[test]
fn invalid_success_identity_is_ambiguous_and_redacted() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_created_message(
            &server,
            200,
            json!({
                "id": "not-a-snowflake",
                "channel_id": CHANNEL_ID,
                "message": SENSITIVE_RESPONSE_MARKER
            }),
            &[],
        )
        .await;

        run_child(&server, "invalid-success");

        let requests = server.received_requests().await.expect("request journal");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, reqwest::Method::POST);
        assert_eq!(requests[0].url.path(), message_path());
    });
}

#[test]
fn local_validation_and_remote_failures_map_to_stable_foundation_categories() {
    assert_eq!(
        MessageError::InvalidRequest(RequestError::TextEmpty).category(),
        ErrorCategory::UsageOrSchema
    );
    assert_eq!(
        MessageError::Authentication.category(),
        ErrorCategory::Authentication
    );
    assert_eq!(
        MessageError::PermissionDenied.category(),
        ErrorCategory::Permission
    );
    assert_eq!(
        MessageError::Conflict.category(),
        ErrorCategory::RemoteConflict
    );
    assert_eq!(
        MessageError::Server { status: 503 }.category(),
        ErrorCategory::UnknownDelivery
    );
    assert_eq!(
        MessageError::PreDispatch {
            failure: PreDispatchFailure::ConnectFailed
        }
        .category(),
        ErrorCategory::ConnectivityRateLimit
    );
}

#[test]
#[ignore = "subprocess fixture for environment-only credential tests"]
fn contract_child_process() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let base_url = std::env::var(BASE_URL_ENV).expect("child base URL");
    runtime().block_on(execute_child_case(&case, &base_url));
}

async fn execute_child_case(case: &str, base_url: &str) {
    match case {
        "success" => {
            assert_successful_send(case, false, base_url, |outcome| {
                assert_basic_outcome(outcome);
            })
            .await;
        }
        "reply" => {
            assert_successful_send(case, true, base_url, |outcome| {
                assert_basic_outcome(outcome);
            })
            .await;
        }
        "route-rate-limit" => {
            assert_successful_send(case, false, base_url, |outcome| {
                assert_basic_outcome(outcome);
                let info = outcome.rate_limit();
                assert_eq!(info.class, RateLimitClass::Route);
                assert_eq!(info.scope, RateLimitScope::Route);
                assert_eq!(info.limit, Some(5));
                assert_eq!(info.remaining, Some(0));
                assert_eq!(info.reset_after, Some(Duration::from_millis(1_750)));
                assert_eq!(info.reset_at.as_deref(), Some("1470173023.123"));
                assert_eq!(info.bucket.as_deref(), Some("synthetic-route-bucket"));
                assert_eq!(info.retry_after, None);
            })
            .await;
        }
        "global-429" | "user-429" | "shared-429" => {
            assert_rate_limited(case, base_url).await;
        }
        "status-400" | "status-401" | "status-403" | "status-404" | "status-409" | "status-503" => {
            assert_status_error(case, base_url).await;
        }
        "post-dispatch-timeout" => {
            let rendered = rendered_message(false, "Delayed response");
            let request =
                CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
            let client = DiscordMessageClient::from_environment_for_test_server_with_timeouts(
                base_url,
                MessageTimeouts {
                    connect: Duration::from_millis(100),
                    total: Duration::from_millis(40),
                    response: Duration::from_millis(40),
                },
            )
            .expect("synthetic timeout client");
            let error = client
                .send(request)
                .await
                .expect_err("post-dispatch timeout must be ambiguous");
            assert_eq!(
                error,
                MessageError::Ambiguous {
                    reason: AmbiguousReason::Timeout
                }
            );
            assert_eq!(error.certainty(), SendCertainty::Ambiguous);
            assert_redacted(&client, &error, SENSITIVE_RESPONSE_MARKER);
        }
        "connect-failure" => {
            let rendered = rendered_message(false, "Connection failure");
            let request =
                CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
            let client = test_client(base_url);
            let error = client
                .send(request)
                .await
                .expect_err("closed loopback port must fail before dispatch");
            assert_eq!(
                error,
                MessageError::PreDispatch {
                    failure: PreDispatchFailure::ConnectFailed
                }
            );
            assert_eq!(error.certainty(), SendCertainty::ProvenNotSent);
            assert_redacted(&client, &error, SENSITIVE_RESPONSE_MARKER);
        }
        "invalid-success" => {
            let rendered = rendered_message(false, "Invalid success identity");
            let request =
                CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
            let client = test_client(base_url);
            let error = client
                .send(request)
                .await
                .expect_err("invalid success identity must be ambiguous");
            assert_eq!(
                error,
                MessageError::Ambiguous {
                    reason: AmbiguousReason::InvalidResponse
                }
            );
            assert_redacted(&client, &error, SENSITIVE_RESPONSE_MARKER);
        }
        "unlisted-mention" => {
            let config = resolved_config();
            let draft = create_draft("@unknown", &config, None);
            let error = ContentRenderer::new()
                .render_current(&draft, CREATED_AT)
                .expect_err("unlisted mention must fail before request construction");
            assert!(matches!(
                error,
                RenderError::Mention(MentionError::UnlistedAlias { .. })
            ));
            assert!(!format!("{error:?} {error}").contains("unknown content marker"));
        }
        "oversized" => {
            let config = resolved_config();
            let oversized = "a".repeat(MAX_DISCORD_MESSAGE_CHARACTERS + 1);
            let draft = create_draft(&oversized, &config, None);
            let error = ContentRenderer::new()
                .render_current(&draft, CREATED_AT)
                .expect_err("oversized content must fail before request construction");
            assert!(matches!(error, RenderError::MessageTooLong { .. }));
        }
        "bearer-token" => {
            let error = DiscordMessageClient::from_environment_for_test_server(base_url)
                .expect_err("Bearer user credential must be rejected");
            assert_eq!(error, MessageError::InvalidBotToken);
            assert!(
                error.to_string().contains(BOT_TOKEN_ENV) || !error.to_string().contains("Bearer")
            );
        }
        "missing-token" => {
            let error = DiscordMessageClient::from_environment_for_test_server(base_url)
                .expect_err("missing environment token must be rejected");
            assert_eq!(error, MessageError::MissingBotToken);
        }
        "invalid-endpoint" => {
            let error = DiscordMessageClient::from_environment_for_test_server(
                "https://discord.com/api/v9",
            )
            .expect_err("an unpinned endpoint path must be rejected");
            assert_eq!(error, MessageError::InvalidEndpoint);
        }
        other => panic!("unknown child case {other}"),
    }
}

async fn assert_successful_send<F>(case: &str, reply: bool, base_url: &str, inspect: F)
where
    F: FnOnce(&crate::MessageSendOutcome),
{
    let rendered = rendered_message(reply, "Build failed: @oncall @owner");
    let request = CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
    let client = test_client(base_url);
    let outcome = client
        .send(request)
        .await
        .unwrap_or_else(|error| panic!("{case} should succeed: {error}"));
    assert_eq!(outcome.message_id(), CREATED_MESSAGE_ID);
    assert_eq!(outcome.channel_id(), CHANNEL_ID);
    assert_eq!(client.api_version(), DiscordApiVersion::V10);
    inspect(&outcome);
}

fn assert_basic_outcome(outcome: &crate::MessageSendOutcome) {
    assert_eq!(outcome.message_id(), CREATED_MESSAGE_ID);
    assert_eq!(outcome.channel_id(), CHANNEL_ID);
}

async fn assert_rate_limited(case: &str, base_url: &str) {
    let rendered = rendered_message(false, "Rate-limited request");
    let request = CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
    let client = test_client(base_url);
    let error = client
        .send(request)
        .await
        .expect_err("429 must be returned to the delivery owner");
    let MessageError::RateLimited { info } = &error else {
        panic!("expected a typed rate-limit error, got {error:?}");
    };
    assert_eq!(info.scope, scope_for(case));
    assert_eq!(
        info.class,
        if case == "global-429" {
            RateLimitClass::Global
        } else {
            RateLimitClass::Route
        }
    );
    assert_eq!(
        info.retry_after,
        Some(match case {
            "global-429" => Duration::from_millis(17_250),
            "user-429" => Duration::from_millis(6_500),
            _ => MAX_DISCORD_DIRECTED_WAIT,
        })
    );
    assert_eq!(error.certainty(), SendCertainty::ProvenNotCreated);
    assert_eq!(error.category(), ErrorCategory::ConnectivityRateLimit);
    assert_redacted(&client, &error, SENSITIVE_RESPONSE_MARKER);
}

async fn assert_status_error(case: &str, base_url: &str) {
    let rendered = rendered_message(false, "Status failure");
    let request = CreateMessageRequest::from_rendered(&rendered).expect("valid rendered message");
    let client = test_client(base_url);
    let error = client
        .send(request)
        .await
        .expect_err("fixture status must be classified");
    let (code, certainty, category) = match case {
        "status-400" => (
            "remote-validation-rejected",
            SendCertainty::ProvenNotCreated,
            ErrorCategory::UsageOrSchema,
        ),
        "status-401" => (
            "discord-authentication-failed",
            SendCertainty::ProvenNotCreated,
            ErrorCategory::Authentication,
        ),
        "status-403" => (
            "discord-permission-denied",
            SendCertainty::ProvenNotCreated,
            ErrorCategory::Permission,
        ),
        "status-404" => (
            "discord-resource-not-found",
            SendCertainty::ProvenNotCreated,
            ErrorCategory::UsageOrSchema,
        ),
        "status-409" => (
            "discord-conflict",
            SendCertainty::ProvenNotCreated,
            ErrorCategory::RemoteConflict,
        ),
        "status-503" => (
            "discord-server-error",
            SendCertainty::Ambiguous,
            ErrorCategory::UnknownDelivery,
        ),
        _ => panic!("unknown status case {case}"),
    };
    assert_eq!(error.code(), code);
    assert_eq!(error.certainty(), certainty);
    assert_eq!(error.category(), category);
    if case == "status-401" {
        assert!(error.to_string().contains(BOT_TOKEN_ENV));
    }
    assert_redacted(&client, &error, SENSITIVE_RESPONSE_MARKER);
}

fn scope_for(case: &str) -> RateLimitScope {
    match case {
        "global-429" => RateLimitScope::Global,
        "user-429" => RateLimitScope::User,
        "shared-429" => RateLimitScope::Shared,
        other => panic!("unknown rate-limit case {other}"),
    }
}

fn assert_redacted(client: &DiscordMessageClient, error: &MessageError, marker: &str) {
    let foundation = error.to_repo_com_error();
    let diagnostics = format!(
        "{client:?} {error:?} {error} {foundation:?} {foundation} {}",
        error.code()
    );
    assert!(!diagnostics.contains(SYNTHETIC_BOT_TOKEN));
    assert!(!diagnostics.contains(marker));
}

fn resolved_config() -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: CHANNEL_ID.to_owned(),
            allowed_mentions: vec!["oncall".to_owned(), "owner".to_owned()],
        },
    );
    let mut mentions = BTreeMap::new();
    mentions.insert(
        "oncall".to_owned(),
        MentionConfig {
            target: format!("role:{ROLE_ID}"),
        },
    );
    mentions.insert(
        "owner".to_owned(),
        MentionConfig {
            target: format!("user:{USER_ID}"),
        },
    );
    resolve_model(
        &RepositoryConfig {
            schema_version: 1,
            repository_id: REPOSITORY_ID.to_owned(),
            discord: DiscordConfig {
                workspace_id: WORKSPACE_ID.to_owned(),
            },
            destinations,
            mentions,
            inbound: BTreeMap::new(),
            retention: RetentionConfig::default(),
            auto_send: Vec::new(),
        },
        Path::new("/synthetic/.repo-com.toml"),
    )
    .expect("synthetic configuration")
}

fn rendered_message(reply: bool, body: &str) -> repo_com_draft_content::RenderedMessage {
    let config = resolved_config();
    let reference = reply.then(|| {
        AuthorizedReplyReference::from_validated_target(
            REPOSITORY_ID,
            WORKSPACE_ID,
            CHANNEL_ID,
            "inbound-1",
            REPLY_MESSAGE_ID,
            "authorization-1",
            SNAPSHOT_HASH,
        )
        .expect("validated reply reference")
    });
    let draft = create_draft(body, &config, reference);
    ContentRenderer::new()
        .render_current(&draft, CREATED_AT)
        .expect("rendered draft")
}

fn create_draft(
    body: &str,
    config: &ResolvedConfig,
    reference: Option<AuthorizedReplyReference>,
) -> DraftModel {
    let mut request = DraftRequest::new("draft-1", "release", body, "build_failed", "high")
        .expect("valid draft request");
    if let Some(reference) = reference {
        request = request.with_reply_reference(reference);
    }
    DraftModel::create(request, config, CREATED_AT).expect("valid draft model")
}

fn created_message_body() -> Value {
    json!({
        "id": CREATED_MESSAGE_ID,
        "channel_id": CHANNEL_ID,
        "content": "synthetic Discord response content",
        "nonce": "synthetic-response-nonce"
    })
}

async fn mount_created_message(
    server: &MockServer,
    status: u16,
    body: Value,
    headers: &[(&str, &str)],
) {
    let mut response = ResponseTemplate::new(status).set_body_json(body);
    for (name, value) in headers {
        response = response.insert_header(*name, *value);
    }
    Mock::given(method("POST"))
        .and(path(message_path()))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn assert_one_create_request(server: &MockServer, reply: bool) {
    let requests = server.received_requests().await.expect("request journal");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, reqwest::Method::POST);
    assert_eq!(request.url.path(), message_path());
    assert!(request.url.path().starts_with("/api/v10/"));
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some(format!("Bot {SYNTHETIC_BOT_TOKEN}").as_str())
    );
    let actual = serde_json::from_slice::<Value>(&request.body).expect("JSON request body");
    let rendered = rendered_message(reply, "Build failed: @oncall @owner");
    let expected_request =
        CreateMessageRequest::from_rendered(&rendered).expect("valid expected request");
    let expected =
        serde_json::to_value(expected_request.wire_payload()).expect("expected request JSON");
    assert_eq!(actual, expected);
    assert_eq!(actual["content"], rendered.exact_text());
    assert!(
        actual["content"]
            .as_str()
            .expect("request text")
            .ends_with(&format!("{NONCE_FOOTER_PREFIX}{}", rendered.nonce()))
    );
    assert!(actual.get("attachments").is_none());
    assert!(actual.get("embeds").is_none());
    assert_eq!(
        actual["message_reference"].is_null(),
        !reply,
        "message_reference must be present only for a validated reply draft"
    );
}

fn message_path() -> String {
    format!("/api/v10/channels/{CHANNEL_ID}/messages")
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test Tokio runtime")
}

fn test_client(base_url: &str) -> DiscordMessageClient {
    DiscordMessageClient::from_environment_for_test_server(base_url).expect("synthetic test client")
}

fn run_child(server: &MockServer, case: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, server.uri())
        .output()
        .expect("run message child");
    assert_child_success(output);
}

fn run_child_with_token(case: &str, token: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, "http://127.0.0.1:1")
        .env(BOT_TOKEN_ENV, token)
        .output()
        .expect("run credential child");
    assert_child_success(output);
}

fn run_child_without_token(case: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, "http://127.0.0.1:1")
        .env_remove(BOT_TOKEN_ENV)
        .output()
        .expect("run missing-token child");
    assert_child_success(output);
}

fn run_child_with_base(case: &str, base_url: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, base_url)
        .output()
        .expect("run child with explicit loopback base");
    assert_child_success(output);
}

fn child_command(case: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .arg("--ignored")
        .arg("--nocapture")
        .arg("contract_child_process")
        .env(CASE_ENV, case)
        .env(BOT_TOKEN_ENV, SYNTHETIC_BOT_TOKEN);
    command
}

fn assert_child_success(output: std::process::Output) {
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
