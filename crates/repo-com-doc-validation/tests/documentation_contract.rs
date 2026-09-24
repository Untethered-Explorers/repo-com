use std::fs;
use std::path::Path;

use repo_com_cli_messaging::{self, MessagingCommand};
use repo_com_cli_operations::{self, OperationsCommand};
use repo_com_config::{MentionTarget, RetentionConfig, SCHEMA_VERSION};
use repo_com_delivery::DeliveryState;
use repo_com_foundation::{
    ColorChoice, DiagnosticsChoice, ErrorCategory, GlobalArgs, OutputFormat, PROTOCOL_VERSION,
    TtyMode,
};
use repo_com_retention::{
    DEFAULT_CONTENT_DAYS, DEFAULT_METADATA_DAYS, MAX_CONTENT_DAYS, MAX_METADATA_DAYS,
    MIN_CONTENT_DAYS, MIN_METADATA_DAYS,
};
use serde_json::{Value, json};

use crate::{scan_forbidden_patterns, validate_repository};

#[test]
fn owned_documents_satisfy_topic_and_claim_contract() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("documentation crate is two levels below the repository root");
    let findings = validate_repository(repository_root);
    assert!(
        findings.is_empty(),
        "documentation findings:\n{}",
        findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn synthetic_credential_shapes_are_rejected_without_committing_credentials() {
    let token = [
        "A".repeat(24),
        format!("b{}", "=".repeat(4)),
        "c".repeat(27),
    ]
    .join(".");
    let authorization = format!("Authorization: {}", "x".repeat(24));
    let multiline_authorization = format!("Authorization:\nBearer {}", "y".repeat(24));
    let private_key = format!("-----BEGIN {}-----", "PRIVATE KEY");
    let multiline_private_key = "-----BEGIN\nPRIVATE KEY-----\nsynthetic".to_owned();
    let real_message = "real team message: synthetic content";

    for (name, value) in [
        ("token", token),
        ("authorization", authorization),
        ("multiline-authorization", multiline_authorization),
        ("private-key", private_key),
        ("multiline-private-key", multiline_private_key),
        ("real-message", real_message.to_owned()),
    ] {
        let findings = scan_forbidden_patterns("synthetic.md", &value);
        assert!(
            !findings.is_empty(),
            "synthetic {name} pattern was not rejected"
        );
    }
}

#[test]
fn required_negative_disclosures_and_unknown_recovery_boundary_are_allowed() {
    let text = "\
        There is no encryption at rest. The operator guide does not provide read receipts. \
        The product does not collect telemetry, does not automatically resend an unknown \
        outcome, and cannot prove current remote state. The operator must use read-only \
        reconciliation before any separately authorized new attempt.";

    assert!(scan_forbidden_patterns("negative-disclosure.md", text).is_empty());
}

#[test]
fn positive_unsupported_claims_are_rejected() {
    let text =
        "repo-com provides read receipts and response analytics, and release approval is complete.";

    let findings = scan_forbidden_patterns("positive-claim.md", text);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "read-receipt-claim")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "response-analytics-claim")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "human-approval-claim")
    );

    let boundary_text = "The product is compatible with live Discord, human sign-off is recorded, and telemetry is available.";
    let boundary_findings = scan_forbidden_patterns("positive-boundary.md", boundary_text);
    assert!(
        boundary_findings
            .iter()
            .any(|finding| finding.code == "live-compatibility-claim")
    );
    assert!(
        boundary_findings
            .iter()
            .any(|finding| finding.code == "human-approval-claim")
    );
    assert!(
        boundary_findings
            .iter()
            .any(|finding| finding.code == "telemetry-claim")
    );
}

#[test]
fn operator_protocol_alignment_is_checked_against_implemented_enums_and_defaults() {
    let root = repository_root();
    let operator =
        fs::read_to_string(root.join("docs/operator-guide.md")).expect("operator guide is present");

    let messaging_commands = [
        MessagingCommand::DraftCreate,
        MessagingCommand::DraftShow,
        MessagingCommand::DraftUpdate,
        MessagingCommand::DraftPreview,
        MessagingCommand::DraftApprove,
        MessagingCommand::DraftSecretOverride,
        MessagingCommand::Send,
        MessagingCommand::SetupCheck,
        MessagingCommand::InboxFetch,
        MessagingCommand::InboxAcknowledge,
        MessagingCommand::InboxArchive,
        MessagingCommand::ReplyDraftCreate,
    ];
    for command in messaging_commands {
        assert!(
            operator.contains(command.as_str()),
            "operator guide is missing implemented messaging command {}",
            command.as_str()
        );
    }

    let operations_commands = [
        OperationsCommand::ConfigValidate,
        OperationsCommand::PolicyStatus,
        OperationsCommand::PolicyActivate,
        OperationsCommand::StateVerify,
        OperationsCommand::LifecycleInspect,
        OperationsCommand::AuditQuery,
        OperationsCommand::PurgePlan,
        OperationsCommand::PurgeExecute,
    ];
    for command in operations_commands {
        assert!(
            operator.contains(command.as_str()),
            "operator guide is missing implemented operations command {}",
            command.as_str()
        );
    }

    for line in operator.lines() {
        let Some((_, value)) = line.split_once("\"command\": \"") else {
            continue;
        };
        let command = value.split('"').next().unwrap_or_default();
        if command.contains('<') {
            continue;
        }
        assert!(
            MessagingCommand::parse(command).is_some()
                || OperationsCommand::parse(command).is_some(),
            "operator guide contains an unknown protocol command: {command}"
        );
    }

    let defaults = GlobalArgs::new();
    assert_eq!(defaults.output_format, OutputFormat::Human);
    assert_eq!(defaults.color, ColorChoice::Auto);
    assert_eq!(defaults.diagnostics, DiagnosticsChoice::Off);
    assert!(defaults.config_path.is_none());
    assert_eq!(TtyMode::default(), TtyMode::NonTty);
    assert!(operator.contains(&PROTOCOL_VERSION.to_string()));
    assert!(operator.contains("`human`"));
    assert!(operator.contains("`auto`"));
    assert!(operator.contains("`off`"));

    for category in ErrorCategory::ALL {
        assert!(operator.contains(category.code()));
        assert!(operator.contains(&category.exit_code().to_string()));
    }

    let delivery_states = [
        DeliveryState::Unclaimed,
        DeliveryState::Claimed,
        DeliveryState::Accepted,
        DeliveryState::Failed,
        DeliveryState::RetryWait,
        DeliveryState::Unknown,
        DeliveryState::ReconciledAccepted,
        DeliveryState::ReconciledAbsent,
        DeliveryState::Unresolved,
    ];
    for state in delivery_states {
        assert!(
            operator.contains(state.as_str()),
            "operator guide is missing local delivery state {}",
            state.as_str()
        );
    }

    let retention_values = [
        DEFAULT_CONTENT_DAYS,
        DEFAULT_METADATA_DAYS,
        MIN_CONTENT_DAYS,
        MAX_CONTENT_DAYS,
        MIN_METADATA_DAYS,
        MAX_METADATA_DAYS,
    ];
    for value in retention_values {
        let rendered = value.to_string();
        let comma_rendered = if value == 3_650 {
            "3,650".to_owned()
        } else {
            value.to_string()
        };
        assert!(operator.contains(&rendered) || operator.contains(&comma_rendered));
    }

    assert_eq!(SCHEMA_VERSION, 1);
    let default_retention = RetentionConfig::default();
    assert_eq!(default_retention.content_days, DEFAULT_CONTENT_DAYS);
    assert_eq!(default_retention.metadata_days, DEFAULT_METADATA_DAYS);
    assert!(MentionTarget::parse("role:portable-id").is_some());
    assert!(MentionTarget::parse("user:portable-id").is_some());
    assert!(MentionTarget::parse("channel:portable-id").is_none());
}

#[test]
fn documented_protocol_inputs_parse_through_both_handler_crates() {
    let messaging_cases = [
        (
            "draft.create",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-1",
                "destination_alias": "release",
                "text": "synthetic validation message",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "2026-01-01T00:00:00Z",
                "created_at_unix_seconds": 1_767_225_600_u64,
                "expires_in_seconds": 3_600_u64,
                "metadata": {}
            }),
        ),
        (
            "draft.show",
            json!({"repository_id": "acme/widgets", "draft_id": "draft-1", "revision": 1}),
        ),
        (
            "draft.update",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-1",
                "revision": 1,
                "destination_alias": "release",
                "text": "synthetic revised message",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "2026-01-01T00:00:00Z",
                "created_at_unix_seconds": 1_767_225_600_u64,
                "metadata": {}
            }),
        ),
        (
            "draft.preview",
            json!({"repository_id": "acme/widgets", "draft_id": "draft-1", "revision": 1}),
        ),
        (
            "draft.approve",
            json!({"repository_id": "acme/widgets", "draft_id": "draft-1", "revision": 1}),
        ),
        (
            "draft.secret-override",
            json!({"repository_id": "acme/widgets", "draft_id": "draft-1", "revision": 1}),
        ),
        (
            "send",
            json!({"repository_id": "acme/widgets", "draft_id": "draft-1", "revision": 1}),
        ),
        ("setup-check", json!({"repository_id": "acme/widgets"})),
        (
            "inbox.fetch",
            json!({
                "repository_id": "acme/widgets",
                "alias": "release",
                "cursor": "123",
                "bot_user_id": "100"
            }),
        ),
        (
            "inbox.acknowledge",
            json!({"repository_id": "acme/widgets", "item_ids": ["100"], "at": "2026-01-01T00:00:00Z"}),
        ),
        (
            "inbox.archive",
            json!({"repository_id": "acme/widgets", "item_ids": ["100"], "at": "2026-01-01T00:00:00Z"}),
        ),
        (
            "reply.draft-create",
            json!({
                "repository_id": "acme/widgets",
                "inbound_item_id": "100",
                "draft_id": "reply-1",
                "text": "synthetic reply draft",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "2026-01-01T00:00:00Z",
                "created_at_unix_seconds": 1_767_225_600_u64,
                "metadata": {}
            }),
        ),
    ];
    for (command, input) in messaging_cases {
        let envelope = json!({"protocol_version": 1, "command": command, "input": input});
        assert!(
            repo_com_cli_messaging::parse(envelope.to_string().as_bytes()).is_ok(),
            "documented messaging input did not parse: {command}"
        );
    }

    let operations_cases = [
        ("config.validate", json!({"repository_id": "acme/widgets"})),
        (
            "policy.status",
            json!({
                "repository_id": "acme/widgets",
                "event_type": "build_failed",
                "destination_alias": "release",
                "severity": "high"
            }),
        ),
        (
            "policy.activate",
            json!({
                "repository_id": "acme/widgets",
                "event_type": "build_failed",
                "destination_alias": "release",
                "severity": "high",
                "activated_at": "2026-01-01T00:00:00Z"
            }),
        ),
        (
            "state.verify",
            json!({"repository_id": "acme/widgets", "database_path": "state.sqlite3"}),
        ),
        (
            "lifecycle.inspect",
            json!({"repository_id": "acme/widgets", "object_type": "repository", "page_size": 10}),
        ),
        (
            "audit.query",
            json!({"repository_id": "acme/widgets", "page_size": 10}),
        ),
        (
            "purge.plan",
            json!({
                "repository_id": "acme/widgets",
                "scope": "content",
                "cutoff": "2026-01-01T00:00:00Z"
            }),
        ),
        (
            "purge.execute",
            json!({
                "repository_id": "acme/widgets",
                "scope": "content",
                "cutoff": "2026-01-01T00:00:00Z",
                "config_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "plan_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "executed_at": "2026-01-01T00:00:00Z"
            }),
        ),
    ];
    for (command, input) in operations_cases {
        let envelope = json!({"protocol_version": 1, "command": command, "input": input});
        assert!(
            repo_com_cli_operations::parse(envelope.to_string().as_bytes()).is_ok(),
            "documented operations input did not parse: {command}"
        );
    }
}

#[test]
fn protocol_output_documents_all_serialized_fields() {
    let success = repo_com_foundation::CommandOutcome::success(json!({"ok": true}));
    let success_value: Value =
        serde_json::from_str(&success.to_json().expect("serializes")).expect("valid success JSON");
    assert_eq!(success_value.as_object().map(serde_json::Map::len), Some(4));
    assert!(success_value.get("data").is_some_and(Value::is_object));
    assert!(success_value.get("error").is_some_and(Value::is_null));

    let error: repo_com_foundation::CommandOutcome<Value> =
        repo_com_foundation::CommandOutcome::failure(repo_com_foundation::RepoComError::usage(
            "synthetic safe detail",
        ));
    let error_value: Value =
        serde_json::from_str(&error.to_json().expect("serializes")).expect("valid error JSON");
    assert_eq!(error_value.as_object().map(serde_json::Map::len), Some(4));
    assert!(error_value.get("data").is_some_and(Value::is_null));
    assert!(error_value.get("error").is_some_and(Value::is_object));
}

fn repository_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("documentation crate is two levels below the repository root")
        .to_path_buf()
}
