use std::collections::BTreeMap;
use std::path::Path;

use repo_com_approval::{ApprovalRecord, OverrideReasonCode, SecretOverrideRecord};
use repo_com_config::{
    DestinationConfig, DiscordConfig, InboundConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};
use repo_com_discord_client::{BotIdentity, SetupReport, WorkspaceMembership};
use repo_com_draft_model::DraftMetadata;
use repo_com_foundation::{CommandOutcome, RepoComError, TtyMode};
use repo_com_terminal_outbound::{
    ApprovalView, DestinationView, ExactConfirmation, ExactPreviewIdentity, MetadataView,
    OutboundPreview, PolicyView, PromptAction, PromptResult, Provenance, RenderOptions,
    SafetyState, SafetyView,
};
use serde_json::{Value, json};

use crate::{
    CreateDraft, DraftIdentity, DraftService, DraftView, HandlerContext, InboundAction,
    InboundActionRequest, InboundActionResult, InboundTrust, InboxFetchCommand, InboxFetchResult,
    InboxFetchService, InboxLifecycleService, MessagingOutput, MessagingResult, OutboundUi,
    ReplyCreateRequest, ReplyDraftView, ReplyService, SendRequest, SendService, SetupService,
    UpdateDraft, dispatch_json, human_output_fits, machine_output_streams, render_human_outcome,
};

const REPOSITORY: &str = "acme/widgets";
const DRAFT_ID: &str = "draft-1";
const WORKSPACE: &str = "100000000000000001";
const CHANNEL: &str = "200000000000000001";
const BOT_USER: &str = "300000000000000001";
const ITEM_ID: &str = "400000000000000001";
const NOW: u64 = 1_500;

fn config() -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: CHANNEL.to_owned(),
            allowed_mentions: Vec::new(),
        },
    );
    let mut inbound = BTreeMap::new();
    inbound.insert("release".to_owned(), InboundConfig { enabled: true });
    resolve_model(
        &RepositoryConfig {
            schema_version: 1,
            repository_id: REPOSITORY.to_owned(),
            discord: DiscordConfig {
                workspace_id: WORKSPACE.to_owned(),
            },
            destinations,
            mentions: BTreeMap::new(),
            inbound,
            retention: RetentionConfig::default(),
            auto_send: Vec::new(),
        },
        Path::new("/repo-com-cli-messaging/.repo-com.toml"),
    )
    .expect("synthetic messaging configuration is valid")
}

fn draft_view(config: &ResolvedConfig) -> DraftView {
    DraftView::from_parts(
        REPOSITORY,
        DRAFT_ID,
        1,
        "a".repeat(64),
        "release",
        config
            .destination("release")
            .expect("release destination")
            .clone(),
        "build failed on main",
        DraftMetadata::default(),
        "build_failed",
        "high",
        1_000,
        2_000,
    )
}

fn preview(config: &ResolvedConfig, finding: bool) -> OutboundPreview {
    let safety = if finding {
        SafetyView {
            state: SafetyState::OverrideRequired,
            scan_hash: Some("c".repeat(64)),
            ..SafetyView::default()
        }
    } else {
        SafetyView {
            state: SafetyState::Clear,
            scan_hash: Some("b".repeat(64)),
            ..SafetyView::default()
        }
    };
    OutboundPreview {
        repository_id: REPOSITORY.to_owned(),
        draft_id: DRAFT_ID.to_owned(),
        revision: 1,
        revision_hash: "a".repeat(64),
        destination: DestinationView::from(
            config.destination("release").expect("release destination"),
        ),
        exact_text: "build failed on main".to_owned(),
        exact_text_hash: "d".repeat(64),
        metadata: MetadataView::default(),
        event_type: "build_failed".to_owned(),
        severity: "high".to_owned(),
        reply_reference: None,
        created_at_unix_seconds: 1_000,
        expires_at_unix_seconds: 2_000,
        approval: ApprovalView::default(),
        policy: PolicyView::default(),
        safety,
        preview_hash: "e".repeat(64),
        provenance: Provenance::LocalDecision,
    }
}

fn approval_record() -> ApprovalRecord {
    ApprovalRecord {
        schema_version: 1,
        approval_id: "approval-1".to_owned(),
        audit_event_id: "approval-recorded-1".to_owned(),
        repository_id: REPOSITORY.to_owned(),
        draft_id: DRAFT_ID.to_owned(),
        revision: 1,
        revision_hash: "a".repeat(64),
        exact_text_hash: "d".repeat(64),
        metadata_hash: "f".repeat(64),
        config_hash: "1".repeat(64),
        destination_hash: "2".repeat(64),
        policy_basis_hash: "3".repeat(64),
        scan_hash: "b".repeat(64),
        preview_hash: "e".repeat(64),
        draft_expires_at_unix_seconds: 2_000,
        approved_at_unix_seconds: 1_500,
        approved_at_utc: "2026-01-01T00:00:00Z".to_owned(),
        expires_at_unix_seconds: 2_400,
        override_hash: None,
        actor_kind: "operator".to_owned(),
    }
}

fn override_record() -> SecretOverrideRecord {
    SecretOverrideRecord {
        schema_version: 1,
        event_id: "secret-override-1".to_owned(),
        repository_id: REPOSITORY.to_owned(),
        draft_id: DRAFT_ID.to_owned(),
        revision: 1,
        revision_hash: "a".repeat(64),
        scan_hash: "c".repeat(64),
        preview_hash: "e".repeat(64),
        reason_code: OverrideReasonCode::ReviewedFalsePositive,
        created_at_unix_seconds: 1_500,
        created_at_utc: "2026-01-01T00:00:00Z".to_owned(),
    }
}

fn delivery(config: &ResolvedConfig) -> repo_com_terminal_outbound::DeliveryView {
    repo_com_terminal_outbound::DeliveryView::new(
        REPOSITORY,
        DRAFT_ID,
        1,
        DestinationView::from(config.destination("release").expect("release destination")),
        repo_com_terminal_outbound::DeliveryOutcome::Accepted {
            message_id: "500000000000000001".to_owned(),
        },
    )
}

fn setup_report() -> SetupReport {
    SetupReport {
        bot: BotIdentity {
            user_id: BOT_USER.to_owned(),
            dedicated_bot: true,
        },
        workspace_id: WORKSPACE.to_owned(),
        workspace_membership: WorkspaceMembership::Member,
        channels: Vec::new(),
        mentions: Vec::new(),
        issues: Vec::new(),
    }
}

fn fetch_result() -> InboxFetchResult {
    InboxFetchResult {
        repository_id: REPOSITORY.to_owned(),
        alias: "release".to_owned(),
        channel_id: CHANNEL.to_owned(),
        items: Vec::new(),
        raw_messages: 0,
        pages_fetched: 1,
        authoritative_cursor: "100".to_owned(),
        continuation: crate::FetchContinuationView {
            next_cursor: None,
            has_more: false,
            reason: "complete".to_owned(),
            pages_fetched: 1,
            raw_messages_seen: 0,
            page_limit_reached: false,
            message_limit_reached: false,
            pages_remaining: 9,
            raw_messages_remaining: 1_000,
        },
        commit: crate::FetchCommitView {
            repository_id: REPOSITORY.to_owned(),
            alias: "release".to_owned(),
            cursor: "100".to_owned(),
            stored_items: 0,
            stored_transitions: 0,
        },
        point_checks_attempted: 0,
        point_checks_edited: 0,
        point_checks_deleted: 0,
        point_check_continuation: None,
        trust: InboundTrust::Untrusted,
    }
}

fn action_result(action: InboundAction) -> InboundActionResult {
    InboundActionResult {
        repository_id: REPOSITORY.to_owned(),
        action,
        item_ids: vec![ITEM_ID.to_owned()],
        at: "2026-01-01T00:00:00Z".to_owned(),
        remote_mutation: false,
    }
}

fn reply_view(config: &ResolvedConfig) -> ReplyDraftView {
    ReplyDraftView {
        repository_id: REPOSITORY.to_owned(),
        inbound_item_id: ITEM_ID.to_owned(),
        draft_id: "reply-draft-1".to_owned(),
        revision: 1,
        revision_hash: "a".repeat(64),
        destination_alias: "release".to_owned(),
        resolved_destination: config
            .destination("release")
            .expect("release destination")
            .clone(),
        message_reference: None,
        created_at_unix_seconds: 1_000,
        expires_at_unix_seconds: 2_000,
        draft_only: true,
    }
}

#[derive(Default)]
struct FakeService {
    calls: Vec<String>,
    finding_preview: bool,
    domain_error: Option<RepoComError>,
    last_identity: Option<DraftIdentity>,
    last_send: Option<SendRequest>,
    last_config_hash: Option<String>,
    last_fetch: Option<InboxFetchCommand>,
    last_action: Option<InboundActionRequest>,
    last_reply: Option<ReplyCreateRequest>,
}

impl FakeService {
    fn record(&mut self, call: &str) {
        self.calls.push(call.to_owned());
    }
}

impl DraftService for FakeService {
    async fn create(&mut self, _request: CreateDraft) -> MessagingResult<DraftView> {
        self.record("draft.create");
        Ok(draft_view(&config()))
    }

    async fn show(&mut self, identity: DraftIdentity) -> MessagingResult<DraftView> {
        self.record("draft.show");
        self.last_identity = Some(identity);
        if let Some(error) = self.domain_error.clone() {
            return Err(error);
        }
        Ok(draft_view(&config()))
    }

    async fn update(&mut self, _request: UpdateDraft) -> MessagingResult<DraftView> {
        self.record("draft.update");
        Ok(draft_view(&config()))
    }

    async fn preview(
        &mut self,
        _identity: DraftIdentity,
        _now_unix_seconds: u64,
    ) -> MessagingResult<OutboundPreview> {
        self.record("draft.preview");
        Ok(preview(&config(), false))
    }

    async fn approval_preview(
        &mut self,
        _identity: DraftIdentity,
        _now_unix_seconds: u64,
    ) -> MessagingResult<OutboundPreview> {
        self.record("draft.approval-preview");
        Ok(preview(&config(), self.finding_preview))
    }

    async fn approve(
        &mut self,
        _identity: DraftIdentity,
        _confirmation: ExactConfirmation,
        _tty_mode: TtyMode,
        _now_unix_seconds: u64,
    ) -> MessagingResult<ApprovalRecord> {
        self.record("draft.approve");
        Ok(approval_record())
    }

    async fn override_secret_finding(
        &mut self,
        _identity: DraftIdentity,
        _confirmation: ExactConfirmation,
        _tty_mode: TtyMode,
        _reason: OverrideReasonCode,
        _now_unix_seconds: u64,
    ) -> MessagingResult<SecretOverrideRecord> {
        self.record("draft.override");
        Ok(override_record())
    }
}

impl SendService for FakeService {
    async fn send(
        &mut self,
        request: SendRequest,
        config: &ResolvedConfig,
    ) -> MessagingResult<repo_com_terminal_outbound::DeliveryView> {
        self.record("send");
        self.last_config_hash = Some(config.canonical_hash());
        self.last_send = Some(request);
        Ok(delivery(config))
    }
}

impl SetupService for FakeService {
    async fn check_setup(&mut self, _config: &ResolvedConfig) -> MessagingResult<SetupReport> {
        self.record("setup-check");
        Ok(setup_report())
    }
}

impl InboxFetchService for FakeService {
    async fn fetch(
        &mut self,
        request: InboxFetchCommand,
        config: &ResolvedConfig,
    ) -> MessagingResult<InboxFetchResult> {
        self.record("inbox.fetch");
        self.last_config_hash = Some(config.canonical_hash());
        self.last_fetch = Some(request);
        Ok(fetch_result())
    }
}

impl InboxLifecycleService for FakeService {
    async fn apply(
        &mut self,
        request: InboundActionRequest,
    ) -> MessagingResult<InboundActionResult> {
        let action = request.action;
        self.record(match action {
            InboundAction::Acknowledge => "inbox.acknowledge",
            InboundAction::Archive => "inbox.archive",
        });
        self.last_action = Some(request);
        Ok(action_result(action))
    }
}

impl ReplyService for FakeService {
    async fn create_reply(
        &mut self,
        request: ReplyCreateRequest,
        config: &ResolvedConfig,
    ) -> MessagingResult<ReplyDraftView> {
        self.record("reply.create");
        self.last_config_hash = Some(config.canonical_hash());
        self.last_reply = Some(request);
        Ok(reply_view(config))
    }
}

struct FakeUi {
    confirm: bool,
    mismatch: bool,
    approval_calls: usize,
    override_calls: usize,
}

impl FakeUi {
    fn new(confirm: bool) -> Self {
        Self {
            confirm,
            mismatch: false,
            approval_calls: 0,
            override_calls: 0,
        }
    }
}

impl OutboundUi for FakeUi {
    fn request_approval(&mut self, preview: OutboundPreview) -> PromptResult {
        self.approval_calls += 1;
        if !self.confirm {
            return PromptResult::Cancelled;
        }
        let identity = ExactPreviewIdentity::from_preview(&preview);
        PromptResult::Confirmed(ExactConfirmation {
            action: PromptAction::Approve,
            repository_id: identity.repository_id,
            draft_id: identity.draft_id,
            revision: identity.revision,
            preview_hash: if self.mismatch {
                "0".repeat(64)
            } else {
                identity.preview_hash
            },
        })
    }

    fn request_secret_override(&mut self, preview: OutboundPreview) -> PromptResult {
        self.override_calls += 1;
        if !self.confirm {
            return PromptResult::Cancelled;
        }
        let identity = ExactPreviewIdentity::from_preview(&preview);
        PromptResult::Confirmed(ExactConfirmation {
            action: PromptAction::OverrideSecretFinding,
            repository_id: identity.repository_id,
            draft_id: identity.draft_id,
            revision: identity.revision,
            preview_hash: if self.mismatch {
                "0".repeat(64)
            } else {
                identity.preview_hash
            },
        })
    }
}

fn envelope(command: &str, input: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "protocol_version": 1,
        "command": command,
        "input": input,
    }))
    .expect("test envelope serializes")
}

fn identity_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "draft_id": DRAFT_ID,
        "revision": 1,
    })
}

fn create_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "draft_id": DRAFT_ID,
        "destination_alias": "release",
        "text": "build failed on main",
        "event_type": "build_failed",
        "severity": "high",
        "created_at": "1970-01-01T00:16:40Z",
        "created_at_unix_seconds": 1_000,
    })
}

fn context(tty_mode: TtyMode) -> HandlerContext {
    HandlerContext::new(tty_mode, NOW)
}

#[tokio::test]
async fn every_messaging_command_dispatches_with_protocol_version_one() {
    let config = config();
    let cases = [
        ("draft.create", create_input()),
        ("draft.show", identity_input()),
        (
            "draft.update",
            json!({
                "repository_id": REPOSITORY,
                "draft_id": DRAFT_ID,
                "revision": 1,
                "destination_alias": "release",
                "text": "build failed on main",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "1970-01-01T00:16:40Z",
                "created_at_unix_seconds": 1_000,
            }),
        ),
        ("draft.preview", identity_input()),
        ("draft.approve", identity_input()),
        ("draft.secret-override", identity_input()),
        ("send", identity_input()),
        ("setup-check", json!({ "repository_id": REPOSITORY })),
        (
            "inbox.fetch",
            json!({
                "repository_id": REPOSITORY,
                "alias": "release",
                "cursor": "100",
                "bot_user_id": BOT_USER,
            }),
        ),
        (
            "inbox.acknowledge",
            json!({
                "repository_id": REPOSITORY,
                "item_ids": [ITEM_ID],
                "at": "2026-01-01T00:00:00Z",
            }),
        ),
        (
            "inbox.archive",
            json!({
                "repository_id": REPOSITORY,
                "item_ids": [ITEM_ID],
                "at": "2026-01-01T00:00:00Z",
            }),
        ),
        (
            "reply.draft-create",
            json!({
                "repository_id": REPOSITORY,
                "inbound_item_id": ITEM_ID,
                "draft_id": "reply-draft-1",
                "text": "Thanks, the agent will review this.",
                "event_type": "inbound_reply",
                "severity": "normal",
                "created_at": "1970-01-01T00:16:40Z",
                "created_at_unix_seconds": 1_000,
            }),
        ),
    ];

    for (command, input) in cases {
        let mut service = FakeService {
            finding_preview: command == "draft.secret-override",
            ..FakeService::default()
        };
        let mut ui = FakeUi::new(true);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::Tty),
            &config,
            &envelope(command, input),
        )
        .await;
        let json = outcome.to_json().expect("one outcome JSON");
        let value: Value = serde_json::from_str(&json).expect("one JSON object");
        assert_eq!(value["protocol_version"], 1, "{command}");
        assert_eq!(value["status"], "success", "{command}: {value}");
        let expected_view = match command {
            "draft.create" | "draft.show" | "draft.update" => "draft",
            "draft.preview" => "preview",
            "draft.approve" => "approval",
            "draft.secret-override" => "secret-override",
            "send" => "delivery",
            "setup-check" => "setup",
            "inbox.fetch" => "fetch",
            "inbox.acknowledge" | "inbox.archive" => "lifecycle",
            "reply.draft-create" => "reply-draft",
            _ => unreachable!("test case has an expected view"),
        };
        assert_eq!(value["data"]["view"], expected_view, "{command}");
        let human = render_human_outcome(&outcome, RenderOptions::plain_text());
        assert!(!human.trim().is_empty(), "{command}");
        assert!(human_output_fits(&human, 80), "{command}: {human}");
        if command == "draft.approve" {
            assert_eq!(ui.approval_calls, 1);
        }
        if command == "draft.secret-override" {
            assert_eq!(ui.override_calls, 1);
        }
        if command == "setup-check" {
            let MessagingOutput::Setup(view) = outcome.data().expect("setup data") else {
                panic!("setup command returned the wrong view");
            };
            assert_eq!(view.repository_id, REPOSITORY);
            assert_eq!(view.config_hash.len(), 64);
        }
    }
}

#[tokio::test]
async fn malformed_and_unknown_inputs_are_one_typed_failure_object() {
    let config = config();
    let cases = vec![
        b"{not-json".to_vec(),
        envelope("unknown.command", identity_input()),
        serde_json::to_vec(&json!({
            "protocol_version": 2,
            "command": "draft.show",
            "input": identity_input(),
        }))
        .expect("wrong version JSON"),
        serde_json::to_vec(&json!({
            "protocol_version": 1,
            "command": "draft.show",
            "input": identity_input(),
            "extra": true,
        }))
        .expect("unknown envelope JSON"),
        envelope(
            "draft.show",
            json!({
                "repository_id": REPOSITORY,
                "draft_id": DRAFT_ID,
                "revision": 1,
                "unexpected": true,
            }),
        ),
        envelope(
            "draft.create",
            json!({
                "repository_id": REPOSITORY,
                "draft_id": DRAFT_ID,
                "text": "body",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "1970-01-01T00:16:40Z",
                "created_at_unix_seconds": 1_000,
            }),
        ),
        envelope(
            "reply.draft-create",
            json!({
                "repository_id": REPOSITORY,
                "inbound_item_id": ITEM_ID,
                "draft_id": "reply-draft-1",
                "destination_alias": "attacker-selected",
                "text": "Thanks, the agent will review this.",
                "event_type": "inbound_reply",
                "severity": "normal",
                "created_at": "1970-01-01T00:16:40Z",
                "created_at_unix_seconds": 1_000,
            }),
        ),
        envelope(
            "inbox.fetch",
            json!({
                "repository_id": REPOSITORY,
                "alias": "release",
                "cursor": "100",
                "time": "2026-01-01T00:00:00Z",
                "bot_user_id": BOT_USER,
            }),
        ),
        envelope(
            "inbox.fetch",
            json!({
                "repository_id": REPOSITORY,
                "alias": "release",
                "bot_user_id": BOT_USER,
            }),
        ),
    ];
    for bytes in cases {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(true);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &bytes,
        )
        .await;
        let streams = machine_output_streams(&outcome, None).expect("failure machine streams");
        let value: Value = serde_json::from_str(streams.stdout()).expect("exactly one JSON object");
        assert_eq!(streams.stderr(), "");
        assert_eq!(value["protocol_version"], 1);
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "usage-schema");
        assert_eq!(outcome.exit_code(), 2);
        assert!(service.calls.is_empty());
    }
}

#[tokio::test]
async fn exact_identifiers_and_no_default_destination_are_enforced() {
    let config = config();
    let mut service = FakeService::default();
    let mut ui = FakeUi::new(true);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope("draft.show", identity_input()),
    )
    .await;
    assert!(outcome.is_success());
    assert_eq!(service.calls, vec!["draft.show"]);
    let identity = service
        .last_identity
        .as_ref()
        .expect("exact identity observed");
    assert_eq!(identity.repository_id, REPOSITORY);
    assert_eq!(identity.draft_id, DRAFT_ID);
    assert_eq!(identity.revision, 1);

    for invalid_input in [
        json!({ "repository_id": REPOSITORY, "revision": 1 }),
        json!({ "repository_id": REPOSITORY, "draft_id": DRAFT_ID }),
        json!({ "repository_id": REPOSITORY, "draft_id": DRAFT_ID, "revision": 0 }),
        json!({ "repository_id": "invalid repository", "draft_id": DRAFT_ID, "revision": 1 }),
    ] {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(true);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &envelope("draft.show", invalid_input),
        )
        .await;
        assert_eq!(outcome.exit_code(), 2);
        assert!(service.calls.is_empty());
    }

    let mut raw_destination = create_input();
    raw_destination["destination_alias"] = json!(CHANNEL);
    let outcome = crate::input::parse(&envelope("draft.create", raw_destination));
    assert_eq!(
        outcome
            .expect_err("raw channel is not a destination alias")
            .exit_code(),
        2
    );
}

#[tokio::test]
async fn approval_and_secret_override_fail_closed_in_non_tty_mode() {
    let config = config();
    for command in ["draft.approve", "draft.secret-override"] {
        let mut service = FakeService {
            finding_preview: command == "draft.secret-override",
            ..FakeService::default()
        };
        let mut ui = FakeUi::new(true);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &envelope(command, identity_input()),
        )
        .await;
        assert_eq!(outcome.exit_code(), 3, "{command}");
        assert_eq!(ui.approval_calls, 0);
        assert_eq!(ui.override_calls, 0);
        assert!(service.calls.is_empty(), "{command}");
    }
}

#[tokio::test]
async fn tty_confirmation_routes_through_ui_before_domain_authority() {
    let config = config();
    for command in ["draft.approve", "draft.secret-override"] {
        let mut service = FakeService {
            finding_preview: command == "draft.secret-override",
            ..FakeService::default()
        };
        let mut ui = FakeUi::new(true);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::Tty),
            &config,
            &envelope(command, identity_input()),
        )
        .await;
        assert!(outcome.is_success(), "{command}");
        if command == "draft.approve" {
            assert_eq!(ui.approval_calls, 1);
            assert_eq!(ui.override_calls, 0);
        } else {
            assert_eq!(ui.approval_calls, 0);
            assert_eq!(ui.override_calls, 1);
        }
        assert!(
            service
                .calls
                .iter()
                .any(|call| call == "draft.approve" || call == "draft.override")
        );
    }
}

#[tokio::test]
async fn mismatched_exact_confirmation_never_reaches_domain_authority() {
    let config = config();
    for command in ["draft.approve", "draft.secret-override"] {
        let mut service = FakeService {
            finding_preview: command == "draft.secret-override",
            ..FakeService::default()
        };
        let mut ui = FakeUi::new(true);
        ui.mismatch = true;
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::Tty),
            &config,
            &envelope(command, identity_input()),
        )
        .await;
        assert_eq!(outcome.exit_code(), 2, "{command}");
        if command == "draft.approve" {
            assert_eq!(ui.approval_calls, 1);
            assert_eq!(ui.override_calls, 0);
        } else {
            assert_eq!(ui.approval_calls, 0);
            assert_eq!(ui.override_calls, 1);
        }
        assert!(
            !service
                .calls
                .iter()
                .any(|call| call == "draft.approve" || call == "draft.override"),
            "{command}"
        );
    }
}

#[tokio::test]
async fn send_fetch_and_reply_ports_receive_exact_boundary_scoped_requests() {
    let config = config();
    let mut service = FakeService::default();
    let mut ui = FakeUi::new(true);

    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope("send", identity_input()),
    )
    .await;
    assert!(outcome.is_success());
    let send = service.last_send.as_ref().expect("send request observed");
    assert_eq!(send.identity().repository_id, REPOSITORY);
    assert_eq!(send.identity().draft_id, DRAFT_ID);
    assert_eq!(send.identity().revision, 1);
    assert_eq!(send.tty_mode, TtyMode::NonTty);
    assert_eq!(send.now_unix_seconds, NOW);
    assert_eq!(
        service.last_config_hash.as_deref(),
        Some(config.canonical_hash().as_str())
    );
    let send_value = serde_json::to_value(send).expect("serializable send boundary");
    for caller_owned_field in [
        "destination_alias",
        "delivery_id",
        "attempt_id",
        "claim_id",
        "request_nonce",
    ] {
        assert!(send_value.get(caller_owned_field).is_none());
    }

    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope(
            "inbox.fetch",
            json!({
                "repository_id": REPOSITORY,
                "alias": "release",
                "cursor": "100",
                "bot_user_id": BOT_USER,
            }),
        ),
    )
    .await;
    assert!(outcome.is_success());
    let fetch_json = outcome.to_json().expect("fetch result JSON");
    let fetch_value: Value = serde_json::from_str(&fetch_json).expect("fetch object");
    assert_eq!(fetch_value["data"]["trust"], "untrusted");
    let fetch = service.last_fetch.as_ref().expect("fetch request observed");
    assert_eq!(fetch.repository_id, REPOSITORY);
    assert_eq!(fetch.alias, "release");
    assert_eq!(fetch.cursor.as_deref(), Some("100"));
    assert_eq!(fetch.time, None);
    assert!(fetch.to_domain().is_ok());
    assert_eq!(
        service.last_config_hash.as_deref(),
        Some(config.canonical_hash().as_str())
    );

    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope(
            "inbox.fetch",
            json!({
                "repository_id": REPOSITORY,
                "alias": "release",
                "time": "2026-01-01T00:00:00Z",
                "bot_user_id": BOT_USER,
            }),
        ),
    )
    .await;
    assert!(outcome.is_success());
    let fetch = service.last_fetch.as_ref().expect("time fetch observed");
    assert_eq!(fetch.cursor, None);
    assert_eq!(fetch.time.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert!(fetch.to_domain().is_ok());
    assert_eq!(
        service.last_config_hash.as_deref(),
        Some(config.canonical_hash().as_str())
    );

    let calls_before_reply = service.calls.len();
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope(
            "reply.draft-create",
            json!({
                "repository_id": REPOSITORY,
                "inbound_item_id": ITEM_ID,
                "draft_id": "reply-draft-1",
                "text": "Thanks, the agent will review this.",
                "event_type": "inbound_reply",
                "severity": "normal",
                "created_at": "1970-01-01T00:16:40Z",
                "created_at_unix_seconds": 1_000,
            }),
        ),
    )
    .await;
    assert!(outcome.is_success());
    let reply = service.last_reply.as_ref().expect("reply request observed");
    assert_eq!(reply.repository_id, REPOSITORY);
    assert_eq!(reply.inbound_item_id, ITEM_ID);
    assert_eq!(reply.draft_id, "reply-draft-1");
    assert_eq!(reply.to_domain().repository_id(), REPOSITORY);
    assert_eq!(
        service.last_config_hash.as_deref(),
        Some(config.canonical_hash().as_str())
    );
    let json = outcome.to_json().expect("reply outcome JSON");
    let value: Value = serde_json::from_str(&json).expect("reply outcome parses as JSON");
    assert_eq!(value["data"]["draft_only"], true);
    assert!(value["data"].get("message_id").is_none());
    assert_eq!(&service.calls[calls_before_reply..], &["reply.create"]);
}

#[tokio::test]
async fn machine_streams_keep_diagnostics_separate_and_human_output_is_labeled() {
    let config = config();
    let mut service = FakeService::default();
    let mut ui = FakeUi::new(true);
    let outcome: CommandOutcome<MessagingOutput> = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope("draft.show", identity_input()),
    )
    .await;
    let streams = machine_output_streams(&outcome, Some("operator diagnostic".to_owned()))
        .expect("machine streams");
    let value: Value = serde_json::from_str(streams.stdout()).expect("stdout is one JSON object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(streams.stderr(), "operator diagnostic");
    assert!(!streams.stdout().contains("operator diagnostic"));
    let quiet = machine_output_streams(&outcome, None).expect("quiet machine streams");
    assert_eq!(quiet.stderr(), "");

    let human = render_human_outcome(&outcome, RenderOptions::plain_text());
    assert!(human.contains("Outbound preview"));
    assert!(human.contains("Destination:"));
    assert!(human.contains("Next action:"));
    assert!(human_output_fits(&human, 80));
}

#[tokio::test]
async fn domain_failures_preserve_stable_categories_in_one_json_object() {
    let config = config();
    let mut service = FakeService {
        domain_error: Some(RepoComError::policy_blocked(
            "current policy does not allow send",
        )),
        ..FakeService::default()
    };
    let mut ui = FakeUi::new(true);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope("draft.show", identity_input()),
    )
    .await;
    assert_eq!(outcome.exit_code(), 4);
    assert_eq!(service.calls, vec!["draft.show"]);
    let streams = machine_output_streams(&outcome, None).expect("domain failure streams");
    let value: Value = serde_json::from_str(streams.stdout()).expect("one domain failure object");
    assert_eq!(streams.stderr(), "");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "policy-blocked");
    assert!(value["data"].is_null());
    let human = render_human_outcome(&outcome, RenderOptions::plain_text());
    assert!(human.contains("Messaging error"));
    assert!(human.contains("Error category: policy-blocked"));
    assert!(human_output_fits(&human, 80));
}

#[test]
fn output_views_have_stable_names_and_serializable_data() {
    let output = MessagingOutput::Fetch(fetch_result());
    assert_eq!(output.view_name(), "fetch");
    let json = output.to_protocol_json().expect("output JSON");
    let value: Value = serde_json::from_str(&json).expect("one output object");
    assert_eq!(value["data"]["view"], "fetch");
    assert_eq!(value["data"]["trust"], "untrusted");
    assert_eq!(value["data"]["commit"]["stored_items"], 0);
}
