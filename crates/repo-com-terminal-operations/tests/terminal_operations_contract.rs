use std::fmt::Write as _;

use repo_com_foundation::{ColorChoice, ErrorCategory, TtyMode};
use repo_com_purge::PurgeScope;

use crate::prompt::{
    ConfirmationSyntax, ExactConfirmation, KeyboardPrompt, PromptAction, PromptRequest,
    PromptResult, ScriptedPromptInput, purge_confirmation_from_intent,
};
use crate::purge::{PurgeExecutionState, PurgeExecutionView, PurgePlanView, PurgeProvenance};
use crate::render::{
    AcknowledgementView, ActivationPreviewView, ArchiveView, AuditEventView, AuditView, CheckState,
    ConfigStatus, ConfigStatusView, DetailField, InboundCommitView, InboundItemView,
    LifecyclePageView, LifecycleView, LocalErrorKind, LocalErrorView, OperationsView,
    PolicyActivationSnapshotView, PolicyState, PolicyStatusView, Provenance, ReadReceiptStatus,
    RemoteStateView, ReplyClaimStatus, ReplyLinkView, RetentionCountsView, RetentionStatus,
    RetentionStatusView, StateCheckView, StateVerificationView, is_ansi_free, render_machine,
    render_machine_failure, render_view, rendered_width,
};
use crate::width::{display_width, lines_fit, wrap_text};

fn tuple() -> repo_com_policy::PolicyTuple {
    repo_com_policy::PolicyTuple::new("build_failed", "release", "high")
}

fn activation() -> ActivationPreviewView {
    ActivationPreviewView {
        repository_id: "acme/widgets".to_owned(),
        tuple: tuple(),
        config_hash: "1".repeat(64),
        tuple_hash: "2".repeat(64),
        activation_id: "activation-ops-1".to_owned(),
        provenance: Provenance::LocalDecision,
        next_action: "Confirm the exact repository, tuple, and hashes on a TTY; activation is not yet recorded."
            .to_owned(),
    }
}

fn config(status: ConfigStatus) -> ConfigStatusView {
    ConfigStatusView {
        repository_id: Some("acme/widgets".to_owned()),
        config_path: Some("/workspace/acme/widgets/.repo-com.toml".to_owned()),
        schema_version: Some(1),
        workspace_id: Some("100000000000000000".to_owned()),
        config_hash: Some("3".repeat(64)),
        destination_aliases: vec!["release".to_owned(), "triage".to_owned()],
        inbound_aliases: vec!["support".to_owned()],
        auto_send_entries: 2,
        status,
        validation_code: Some("config-valid".to_owned()),
        detail: Some("configuration resolved and validated".to_owned()),
        provenance: Provenance::LocalDecision,
        next_action: "Use the exact repository and alias identifiers for the requested operation."
            .to_owned(),
    }
}

fn policy(state: PolicyState) -> PolicyStatusView {
    let activation = PolicyActivationSnapshotView {
        activation_id: "activation-ops-1".to_owned(),
        repository_id: "acme/widgets".to_owned(),
        tuple: tuple(),
        recorded_config_hash: "1".repeat(64),
        recorded_tuple_hash: "2".repeat(64),
        current_config_hash: "1".repeat(64),
        current_tuple_hash: "2".repeat(64),
        activated_at: "2026-09-24T12:00:00Z".to_owned(),
        deactivated_at: None,
        active: state == PolicyState::Active,
        stale_reason: (state == PolicyState::Stale).then(|| "ConfigHashChanged".to_owned()),
    };
    PolicyStatusView {
        repository_id: "acme/widgets".to_owned(),
        tuple: Some(tuple()),
        state,
        config_hash: Some("1".repeat(64)),
        tuple_hash: Some("2".repeat(64)),
        activation_id: Some("activation-ops-1".to_owned()),
        activations: vec![activation],
        basis: format!("{} exact policy status", state.as_str()),
        provenance: Provenance::LocalState,
        next_action:
            "Keep all exact bindings visible and use only the current local policy status."
                .to_owned(),
    }
}

fn check(status: CheckState, code: &str) -> StateCheckView {
    StateCheckView {
        status,
        passed: status == CheckState::Passed,
        code: code.to_owned(),
        details: vec![DetailField::new("Evidence", "redacted local check")],
    }
}

fn state_verification() -> StateVerificationView {
    StateVerificationView {
        repository_id: "acme/widgets".to_owned(),
        healthy: false,
        read_only: true,
        connection_read_only: true,
        privacy_disclosure: "v1 state is protected by user-only filesystem permissions, not encryption at rest."
            .to_owned(),
        quick_check: check(CheckState::Passed, "quick-check-passed"),
        foreign_keys: check(CheckState::Passed, "foreign-key-check-passed"),
        migration: check(CheckState::Failed, "migration-mismatch"),
        repository_scope: check(CheckState::Passed, "repository-scope-passed"),
        filesystem_permissions: check(CheckState::Unavailable, "permission-unavailable"),
        issues: vec![DetailField::new(
            "migration-mismatch",
            "the local database migration does not match the expected version",
        )],
        remediation: vec!["Preserve the database and investigate the migration mismatch before mutation.".to_owned()],
        provenance: Provenance::LocalState,
        next_action: "Review every issue and remediation; do not mutate local state until integrity checks pass."
            .to_owned(),
    }
}

fn audit() -> AuditView {
    AuditView {
        repository_id: "acme/widgets".to_owned(),
        page_size: 2,
        event_count: 2,
        has_more: true,
        next_cursor: Some("acme/widgets|2026-09-24T12:00:00Z|42".to_owned()),
        events: vec![
            AuditEventView {
                audit_id: 41,
                event_id: "event-41".to_owned(),
                object_type: "inbound_item".to_owned(),
                object_id: "inbound-1".to_owned(),
                transition: "stored".to_owned(),
                occurred_at: "2026-09-24T11:59:00Z".to_owned(),
                actor_kind: "system".to_owned(),
                outcome: "recorded".to_owned(),
                metadata: serde_json::json!({"untrusted": true, "content": "[redacted]"}),
            },
            AuditEventView {
                audit_id: 42,
                event_id: "event-42-with-a-long-stable-identifier-for-wrapping".to_owned(),
                object_type: "policy_activation".to_owned(),
                object_id: "activation-ops-1".to_owned(),
                transition: "activated".to_owned(),
                occurred_at: "2026-09-24T12:00:00Z".to_owned(),
                actor_kind: "operator".to_owned(),
                outcome: "recorded".to_owned(),
                metadata: serde_json::json!({"config_hash": "1111111111111111"}),
            },
        ],
        provenance: Provenance::LocalState,
        remote_fetch_performed: false,
        next_action: "Use the returned continuation to request the next bounded local audit page."
            .to_owned(),
    }
}

fn lifecycle_delivery() -> LifecycleView {
    LifecycleView {
        repository_id: "acme/widgets".to_owned(),
        object_type: "delivery_attempt".to_owned(),
        object_id: "attempt-1".to_owned(),
        revision: Some(7),
        state: "accepted".to_owned(),
        local_state: "local delivery state accepted".to_owned(),
        last_fetched_remote: Some(RemoteStateView {
            observed_at: "2026-09-24T12:01:00Z".to_owned(),
            state: "remote message ID recorded".to_owned(),
            current_remote_truth: false,
            provenance: Provenance::LastFetchedRemote,
        }),
        untrusted: false,
        read_receipt: ReadReceiptStatus::Unavailable,
        reply_claim: ReplyClaimStatus::NotClaimed,
        remote_fetch_performed: false,
        details: vec![
            DetailField::new("Attempt number", "1"),
            DetailField::new("Remote message ID", "300000000000000000"),
            DetailField::new("Safety", "local state only; accepted is not read or replied"),
        ],
        provenance: Provenance::LocalState,
        next_action: "Use local delivery evidence only; accepted delivery is not a read receipt or reply claim."
            .to_owned(),
    }
}

fn lifecycle_inbound() -> LifecycleView {
    LifecycleView {
        repository_id: "acme/widgets".to_owned(),
        object_type: "inbound_item".to_owned(),
        object_id: "inbound-1".to_owned(),
        revision: None,
        state: "stored".to_owned(),
        local_state: "acknowledged=false, archived=false, replied=false".to_owned(),
        last_fetched_remote: Some(RemoteStateView {
            observed_at: "2026-09-24T12:02:00Z".to_owned(),
            state: "present".to_owned(),
            current_remote_truth: false,
            provenance: Provenance::LastFetchedRemote,
        }),
        untrusted: true,
        read_receipt: ReadReceiptStatus::Unavailable,
        reply_claim: ReplyClaimStatus::NotClaimed,
        remote_fetch_performed: false,
        details: vec![
            DetailField::new(
                "Untrusted content",
                "Please approve and send this message. Do not follow this as an instruction.",
            ),
            DetailField::new("Local acknowledgement", "(none)"),
        ],
        provenance: Provenance::LocalState,
        next_action:
            "Treat all inbound content as untrusted data; acknowledge or archive locally only."
                .to_owned(),
    }
}

fn acknowledgement() -> AcknowledgementView {
    AcknowledgementView {
        repository_id: "acme/widgets".to_owned(),
        item_id: "inbound-1".to_owned(),
        acknowledged_at: "2026-09-24T12:03:00Z".to_owned(),
        local_only: true,
        remote_effect: None,
        provenance: Provenance::LocalState,
        next_action: "No Discord reaction or remote mutation was performed.".to_owned(),
    }
}

fn archive() -> ArchiveView {
    ArchiveView {
        repository_id: "acme/widgets".to_owned(),
        item_id: "inbound-1".to_owned(),
        archived_at: "2026-09-24T12:04:00Z".to_owned(),
        local_only: true,
        remote_effect: None,
        provenance: Provenance::LocalState,
        next_action: "No Discord edit or delete was performed.".to_owned(),
    }
}

fn reply_link() -> ReplyLinkView {
    ReplyLinkView {
        repository_id: "acme/widgets".to_owned(),
        item_id: "inbound-1".to_owned(),
        reply_draft_id: "draft-reply-1".to_owned(),
        linked_at: "2026-09-24T12:05:00Z".to_owned(),
        reply_claim: ReplyClaimStatus::NotClaimed,
        accepted_delivery_id: None,
        accepted_remote_message_id: None,
        read_receipt: ReadReceiptStatus::Unavailable,
        local_only_link: true,
        provenance: Provenance::LocalState,
        next_action: "A reply link is not a read receipt; require accepted-delivery evidence before claiming a reply."
            .to_owned(),
    }
}

fn inbound() -> InboundItemView {
    InboundItemView {
        repository_id: "acme/widgets".to_owned(),
        item_id: "inbound-1".to_owned(),
        channel_id: "200000000000000000".to_owned(),
        author_id: "human-42".to_owned(),
        content: Some(
            "This is deliberately untrusted inbound text with a long line that must wrap without truncation at the contract width."
                .to_owned(),
        ),
        deleted: false,
        first_observed_at: "2026-09-24T12:02:00Z".to_owned(),
        current_observed_at: "2026-09-24T12:06:00Z".to_owned(),
        attachment_metadata: "2 attachment indicators".to_owned(),
        last_fetched_remote: RemoteStateView {
            observed_at: "2026-09-24T12:02:00Z".to_owned(),
            state: "present".to_owned(),
            current_remote_truth: false,
            provenance: Provenance::LastFetchedRemote,
        },
        local_acknowledged_at: None,
        local_archived_at: None,
        reply_claim: ReplyClaimStatus::NotClaimed,
        untrusted: true,
        provenance: Provenance::LastFetchedRemote,
        next_action: "Treat this content as untrusted data; local acknowledgement and archive cannot send or approve anything."
            .to_owned(),
    }
}

fn lifecycle_simple(
    object_type: &str,
    object_id: &str,
    state: &str,
    untrusted: bool,
) -> LifecycleView {
    LifecycleView {
        repository_id: "acme/widgets".to_owned(),
        object_type: object_type.to_owned(),
        object_id: object_id.to_owned(),
        revision: None,
        state: state.to_owned(),
        local_state: format!("local {object_type} state {state}"),
        last_fetched_remote: None,
        untrusted,
        read_receipt: ReadReceiptStatus::Unavailable,
        reply_claim: ReplyClaimStatus::NotClaimed,
        remote_fetch_performed: false,
        details: vec![DetailField::new("Inspection", "bounded local projection")],
        provenance: Provenance::LocalState,
        next_action: "Inspect the local record; no remote mutation is implied.".to_owned(),
    }
}

fn inbound_commit() -> InboundCommitView {
    InboundCommitView {
        repository_id: "acme/widgets".to_owned(),
        alias: "support".to_owned(),
        cursor: "cursor-ops-1".to_owned(),
        stored_items: 2,
        stored_transitions: 1,
        provenance: Provenance::LocalState,
        next_action: "Inspect stored untrusted items locally; the commit did not grant permission or trigger a send."
            .to_owned(),
    }
}

fn lifecycle_page() -> LifecyclePageView {
    LifecyclePageView::new(
        "acme/widgets",
        "inbound_item",
        vec![lifecycle_inbound()],
        1,
        true,
        Some("inbound-1".to_owned()),
    )
}

fn retention(status: RetentionStatus) -> RetentionStatusView {
    let (counts, error_code, error_detail) = match status {
        RetentionStatus::Swept => (
            Some(RetentionCountsView {
                content_rows_redacted: 2,
                metadata_rows_removed: 3,
            }),
            None,
            None,
        ),
        RetentionStatus::Blocked => (
            None,
            Some("storage-integrity".to_owned()),
            Some("local state integrity blocked the sweep".to_owned()),
        ),
        _ => (None, None, None),
    };
    RetentionStatusView {
        repository_id: "acme/widgets".to_owned(),
        content_days: if status == RetentionStatus::Configured {
            30
        } else {
            0
        },
        metadata_days: if status == RetentionStatus::Configured {
            365
        } else {
            0
        },
        status,
        as_of: (status == RetentionStatus::Swept).then(|| "2026-09-24T12:00:00Z".to_owned()),
        content_cutoff: (status == RetentionStatus::Swept)
            .then(|| "2026-08-25T12:00:00Z".to_owned()),
        metadata_cutoff: (status == RetentionStatus::Swept)
            .then(|| "2025-09-24T12:00:00Z".to_owned()),
        counts,
        audit_event_id: (status == RetentionStatus::Swept).then(|| "audit-retention-1".to_owned()),
        error_code,
        error_detail,
        provenance: if status == RetentionStatus::Blocked {
            Provenance::LocalError
        } else {
            Provenance::LocalState
        },
        next_action: "Inspect local count-only evidence; retention never mutates Discord."
            .to_owned(),
    }
}

fn purge_plan() -> PurgePlanView {
    PurgePlanView {
        schema_version: 1,
        repository_id: "acme/widgets".to_owned(),
        scope: PurgeScope::Content,
        cutoff_unix_seconds: 1_700_000_000,
        cutoff_utc: "2023-11-14T22:13:20Z".to_owned(),
        config_hash: "4".repeat(64),
        counts: crate::purge::PurgeCountsView {
            content_rows: 7,
            metadata_rows: 0,
            total_rows: 7,
            table_rows: [("draft_revisions".to_owned(), 7)].into_iter().collect(),
        },
        state_fingerprint: "5".repeat(64),
        plan_hash: "6".repeat(64),
        execution_performed: false,
        provenance: PurgeProvenance::LocalPlan,
    }
}

fn purge_execution(state: PurgeExecutionState) -> PurgeExecutionView {
    match state {
        PurgeExecutionState::Executed => PurgeExecutionView {
            repository_id: "acme/widgets".to_owned(),
            plan_hash: "6".repeat(64),
            state,
            counts: Some(crate::purge::PurgeCountsView {
                content_rows: 7,
                metadata_rows: 0,
                total_rows: 7,
                table_rows: [("draft_revisions".to_owned(), 7)].into_iter().collect(),
            }),
            audit_event_id: Some("audit-purge-1".to_owned()),
            executed_at: Some("2026-09-24T12:10:00Z".to_owned()),
            error_code: None,
            error_detail: None,
            next_action: "Inspect count-only local audit evidence; do not reuse this plan hash."
                .to_owned(),
            provenance: PurgeProvenance::LocalExecution,
        },
        _ => PurgeExecutionView::from_error(
            "acme/widgets",
            "6".repeat(64),
            match state {
                PurgeExecutionState::ReplanRequired => "purge-replan-required",
                PurgeExecutionState::RolledBack => "purge-storage",
                _ => "purge-invalid-plan",
            },
            "redacted local purge failure",
            state,
        ),
    }
}

fn error(category: ErrorCategory) -> LocalErrorView {
    LocalErrorView::new(
        category,
        category.code(),
        "redacted local operations diagnostic",
        "Inspect the stable category and use the explicit next action.",
    )
    .with_repository("acme/widgets")
    .with_object_type("local-state")
}

fn error_with_kind(category: ErrorCategory, kind: LocalErrorKind) -> LocalErrorView {
    LocalErrorView::new(
        category,
        category.code(),
        "redacted local operations diagnostic",
        "Inspect the stable category and use the explicit next action.",
    )
    .with_kind(kind)
    .with_repository("acme/widgets")
    .with_object_type("local-state")
}

fn views() -> Vec<(&'static str, OperationsView)> {
    vec![
        (
            "config-valid",
            OperationsView::config(config(ConfigStatus::Valid)),
        ),
        (
            "config-invalid",
            OperationsView::config(config(ConfigStatus::Invalid)),
        ),
        (
            "config-missing",
            OperationsView::config(config(ConfigStatus::Missing)),
        ),
        (
            "config-unsupported",
            OperationsView::config(config(ConfigStatus::UnsupportedSchema)),
        ),
        (
            "config-secret-field",
            OperationsView::config(config(ConfigStatus::SecretField)),
        ),
        (
            "policy-active",
            OperationsView::policy(policy(PolicyState::Active)),
        ),
        (
            "policy-stale",
            OperationsView::policy(policy(PolicyState::Stale)),
        ),
        (
            "policy-ambiguous",
            OperationsView::policy(policy(PolicyState::Ambiguous)),
        ),
        (
            "policy-not-activated",
            OperationsView::policy(policy(PolicyState::NotActivated)),
        ),
        (
            "policy-not-configured",
            OperationsView::policy(policy(PolicyState::NotConfigured)),
        ),
        (
            "policy-deactivated",
            OperationsView::policy(policy(PolicyState::Deactivated)),
        ),
        (
            "policy-not-evaluated",
            OperationsView::policy(policy(PolicyState::NotEvaluated)),
        ),
        (
            "activation-preview",
            OperationsView::activation(activation()),
        ),
        (
            "state-verification",
            OperationsView::state_verification(state_verification()),
        ),
        ("audit-bounded", OperationsView::audit(audit())),
        (
            "lifecycle-delivery",
            OperationsView::lifecycle(lifecycle_delivery()),
        ),
        (
            "lifecycle-inbound",
            OperationsView::lifecycle(lifecycle_inbound()),
        ),
        (
            "lifecycle-repository",
            OperationsView::lifecycle(lifecycle_simple(
                "repository",
                "acme/widgets",
                "configured",
                false,
            )),
        ),
        (
            "lifecycle-draft",
            OperationsView::lifecycle(lifecycle_simple("draft", "draft-1", "approved", false)),
        ),
        (
            "lifecycle-revision",
            OperationsView::lifecycle(lifecycle_simple(
                "draft_revision",
                "draft-1~7",
                "immutable",
                false,
            )),
        ),
        (
            "lifecycle-acknowledgement",
            OperationsView::lifecycle(lifecycle_simple(
                "acknowledgement",
                "inbound-1",
                "acknowledged",
                true,
            )),
        ),
        (
            "lifecycle-archive",
            OperationsView::lifecycle(lifecycle_simple("archive", "inbound-1", "archived", true)),
        ),
        (
            "lifecycle-reply-link",
            OperationsView::lifecycle(lifecycle_simple(
                "reply_link",
                "inbound-1",
                "linked-only",
                true,
            )),
        ),
        (
            "lifecycle-audit",
            OperationsView::lifecycle(lifecycle_simple(
                "audit_transition",
                "bounded-page",
                "complete-local-page",
                false,
            )),
        ),
        (
            "lifecycle-page",
            OperationsView::lifecycle_page(lifecycle_page()),
        ),
        ("inbound-untrusted", OperationsView::inbound(inbound())),
        (
            "inbound-commit",
            OperationsView::inbound_commit(inbound_commit()),
        ),
        (
            "acknowledgement-local",
            OperationsView::acknowledgement(acknowledgement()),
        ),
        ("archive-local", OperationsView::archive(archive())),
        ("reply-link-local", OperationsView::reply_link(reply_link())),
        (
            "retention-configured",
            OperationsView::retention(retention(RetentionStatus::Configured)),
        ),
        (
            "retention-swept",
            OperationsView::retention(retention(RetentionStatus::Swept)),
        ),
        (
            "retention-blocked",
            OperationsView::retention(retention(RetentionStatus::Blocked)),
        ),
        (
            "retention-invalid",
            OperationsView::retention(retention(RetentionStatus::Invalid)),
        ),
        ("purge-plan", OperationsView::purge_plan(purge_plan())),
        (
            "purge-executed",
            OperationsView::purge_execution(purge_execution(PurgeExecutionState::Executed)),
        ),
        (
            "purge-rolled-back",
            OperationsView::purge_execution(purge_execution(PurgeExecutionState::RolledBack)),
        ),
        (
            "purge-replan-required",
            OperationsView::purge_execution(purge_execution(PurgeExecutionState::ReplanRequired)),
        ),
        (
            "purge-planned",
            OperationsView::purge_execution(purge_execution(PurgeExecutionState::Planned)),
        ),
        (
            "purge-failed",
            OperationsView::purge_execution(purge_execution(PurgeExecutionState::Failed)),
        ),
        (
            "local-error",
            OperationsView::error(error(ErrorCategory::StorageIntegrity)),
        ),
        (
            "error-locked",
            OperationsView::error(error_with_kind(
                ErrorCategory::StorageIntegrity,
                LocalErrorKind::Locked,
            )),
        ),
        (
            "error-corrupt",
            OperationsView::error(error_with_kind(
                ErrorCategory::StorageIntegrity,
                LocalErrorKind::Corrupt,
            )),
        ),
        (
            "error-unsupported",
            OperationsView::error(error_with_kind(
                ErrorCategory::UsageOrSchema,
                LocalErrorKind::Unsupported,
            )),
        ),
        (
            "error-missing",
            OperationsView::error(error_with_kind(
                ErrorCategory::UsageOrSchema,
                LocalErrorKind::Missing,
            )),
        ),
        (
            "operator-action-error",
            OperationsView::error(error(ErrorCategory::OperatorActionRequired)),
        ),
    ]
}

fn snapshot_text() -> String {
    let mut output = String::new();
    for (name, view) in views() {
        writeln!(&mut output, "===== {name} =====").expect("snapshot string is writable");
        output.push_str(&render_view(
            &view,
            crate::render::RenderOptions::plain_text(),
        ));
        output.push('\n');
    }
    output
}

#[test]
fn deterministic_snapshot_contract() {
    let actual = snapshot_text();
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/snapshots/terminal_operations_contract.snap"
            ),
            &actual,
        )
        .expect("snapshot update is writable");
        return;
    }
    assert_eq!(
        actual,
        include_str!("snapshots/terminal_operations_contract.snap")
    );
}

#[test]
fn every_named_view_is_complete_at_80_columns_without_ansi() {
    for (name, view) in views() {
        let rendered = render_view(&view, crate::render::RenderOptions::plain_text());
        assert!(is_ansi_free(&rendered), "{name} contains ANSI");
        assert!(lines_fit(&rendered, 80), "{name} exceeds 80 columns");
        for label in [
            "Repository:",
            "Object:",
            "Outcome:",
            "Provenance:",
            "Next action:",
        ] {
            assert!(rendered.contains(label), "{name} lacks {label}");
        }
        assert!(
            rendered_width(&rendered) <= 80,
            "{name} has an overlong line"
        );
    }
}

#[test]
fn no_color_and_explicit_non_color_are_text_only() {
    assert_eq!(
        crate::render::RenderOptions::resolve_color(ColorChoice::Always, TtyMode::Tty, true),
        ColorChoice::Never
    );
    let view = OperationsView::purge_plan(purge_plan());
    let explicit = render_view(
        &view,
        crate::render::RenderOptions::with_color(ColorChoice::Never, TtyMode::Tty),
    );
    let no_color = render_view(&view, crate::render::RenderOptions::no_color());
    assert!(is_ansi_free(&explicit));
    assert!(is_ansi_free(&no_color));
    assert_eq!(explicit, no_color);
    let colored = render_view(
        &view,
        crate::render::RenderOptions::with_color(ColorChoice::Always, TtyMode::Tty),
    );
    assert!(!is_ansi_free(&colored));
    assert!(colored.contains("Outcome:"));
    assert!(colored.contains("Next action:"));
}

#[test]
fn trust_local_remote_and_read_semantics_are_explicit_text() {
    let inbound_text = render_view(
        &OperationsView::inbound(inbound()),
        crate::render::RenderOptions::plain_text(),
    );
    assert!(inbound_text.contains("Trust: untrusted inbound data"));
    assert!(inbound_text.contains("Current remote truth: false"));
    assert!(inbound_text.contains("Provenance: last-fetched remote state"));
    assert!(inbound_text.contains("Read receipt: unavailable; never inferred"));

    let delivery_text = render_view(
        &OperationsView::lifecycle(lifecycle_delivery()),
        crate::render::RenderOptions::plain_text(),
    );
    assert!(delivery_text.contains("Local state:"));
    assert!(delivery_text.contains("Read receipt: unavailable; never inferred"));
    assert!(!delivery_text.contains("Outcome: read"));
    assert!(!delivery_text.contains("Outcome: replied"));

    let acknowledgement_text = render_view(
        &OperationsView::acknowledgement(acknowledgement()),
        crate::render::RenderOptions::plain_text(),
    );
    assert!(acknowledgement_text.contains("Local only: true"));
    assert!(acknowledgement_text.contains("No Discord reaction"));
}

#[test]
fn long_hashes_and_untrusted_content_wrap_without_truncation() {
    let mut view = inbound();
    view.content = Some("x".repeat(241));
    view.last_fetched_remote.observed_at =
        "2026-09-24T12:06:00Z-with-a-long-observation-identifier".to_owned();
    let rendered = render_view(
        &OperationsView::inbound(view),
        crate::render::RenderOptions::with_width(80),
    );
    assert!(
        rendered
            .chars()
            .filter(|character| *character == 'x')
            .count()
            >= 241
    );
    assert!(lines_fit(&rendered, 80));
    assert!(rendered.contains("Current remote truth: false"));
}

#[test]
fn machine_mode_is_one_protocol_object_with_stable_categories() {
    let success = render_machine(&OperationsView::retention(retention(
        RetentionStatus::Configured,
    )))
    .expect("success JSON");
    let value: serde_json::Value = serde_json::from_str(&success).expect("one success object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["status"], "success");
    assert_eq!(value["data"]["view"], "retention");

    let failure =
        render_machine_failure(&error(ErrorCategory::StorageIntegrity)).expect("failure JSON");
    let value: serde_json::Value = serde_json::from_str(&failure).expect("one failure object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "storage-integrity");
}

#[test]
fn prompt_cancel_default_invalid_and_non_tty_are_distinct() {
    let cases = [
        (vec!["n"], PromptResult::Cancelled),
        (vec![""], PromptResult::Defaulted),
        (
            vec!["not-a-command"],
            PromptResult::InvalidInput {
                input: "invalid input (redacted)".to_owned(),
            },
        ),
    ];
    for (lines, expected) in cases {
        let input = ScriptedPromptInput::new(lines);
        let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty);
        assert_eq!(
            prompt.request(&PromptRequest::activation(activation())),
            expected
        );
    }

    let input = ScriptedPromptInput::new(["activate"]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::NonTty);
    assert_eq!(
        prompt.request(&PromptRequest::exact_activation(activation())),
        PromptResult::NonTty
    );
    assert_eq!(prompt.input().calls(), 0);

    let input = ScriptedPromptInput::new(["purge"]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::NonTty);
    assert_eq!(
        prompt.request(&PromptRequest::exact_purge(purge_plan())),
        PromptResult::NonTty
    );
    assert_eq!(prompt.input().calls(), 0);

    let plan = purge_plan();
    let intent = ExactConfirmation {
        action: PromptAction::ConfirmPurge,
        repository_id: plan.repository_id.clone(),
        object_id: "purge-plan".to_owned(),
        scope: Some(plan.scope.as_str().to_owned()),
        config_hash: Some(plan.config_hash.clone()),
        tuple_hash: None,
        plan_hash: Some(plan.plan_hash.clone()),
    };
    assert!(
        purge_confirmation_from_intent(&intent, &plan.confirmation_identity(), TtyMode::NonTty)
            .is_none()
    );
}

#[test]
fn exact_activation_confirmation_binds_complete_scope() {
    let preview = activation();
    let response = format!(
        "activate {} {} {} {} {}",
        preview.repository_id,
        preview.activation_id,
        preview.config_hash,
        preview.tuple_hash,
        preview.tuple
    );
    let input = ScriptedPromptInput::new([response]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty);
    let result = prompt.request(&PromptRequest::exact_activation(preview.clone()));
    let confirmation = result
        .confirmation()
        .expect("exact activation confirmation");
    assert_eq!(confirmation.action, PromptAction::ActivatePolicy);
    assert_eq!(confirmation.repository_id, preview.repository_id);
    assert_eq!(confirmation.object_id, preview.activation_id);
    assert_eq!(
        confirmation.config_hash.as_deref(),
        Some(preview.config_hash.as_str())
    );
    assert_eq!(
        confirmation.tuple_hash.as_deref(),
        Some(preview.tuple_hash.as_str())
    );
    assert!(prompt.input().prompts()[0].contains("Activation ID:"));
    assert!(prompt.input().prompts()[0].contains("keyboard:"));
}

#[test]
fn exact_purge_confirmation_and_plan_hash_change_are_distinct() {
    let plan = purge_plan();
    let response = format!(
        "purge {} {} {} {} {}",
        plan.repository_id,
        plan.scope.as_str(),
        plan.cutoff_unix_seconds,
        plan.config_hash,
        plan.plan_hash
    );
    let input = ScriptedPromptInput::new([response]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty);
    let result = prompt.request(&PromptRequest::exact_purge(plan.clone()));
    let confirmation = result.confirmation().expect("exact purge confirmation");
    assert_eq!(confirmation.action, PromptAction::ConfirmPurge);
    assert_eq!(
        confirmation.plan_hash.as_deref(),
        Some(plan.plan_hash.as_str())
    );

    let changed = format!(
        "purge {} {} {} {} {}",
        plan.repository_id,
        plan.scope.as_str(),
        plan.cutoff_unix_seconds,
        plan.config_hash,
        "f".repeat(64)
    );
    let input = ScriptedPromptInput::new([changed]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty);
    let result = prompt.request(&PromptRequest::exact_purge(plan));
    assert!(matches!(result, PromptResult::PlanHashChanged { .. }));
    assert!(!result.is_confirmed());
}

#[test]
fn prompt_text_is_plain_complete_and_keyboard_reachable() {
    let input = ScriptedPromptInput::new(["n"]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty).with_render_options(
        crate::render::RenderOptions::with_color(ColorChoice::Never, TtyMode::Tty),
    );
    assert_eq!(
        prompt.request(&PromptRequest::purge(purge_plan())),
        PromptResult::Cancelled
    );
    let text = &prompt.input().prompts()[0];
    assert!(is_ansi_free(text));
    assert!(text.contains("Purge plan"));
    assert!(text.contains("Plan hash:"));
    assert!(text.contains("Y/yes"));
    assert!(text.contains("N/no"));
    assert!(text.contains("Enter"));
    assert!(text.contains("non-authoritative"));
    assert!(text.contains("Focus:"));
    assert!(lines_fit(text, 80));
}

#[test]
fn width_helper_preserves_all_content() {
    let value = "z".repeat(241);
    let lines = wrap_text(&value, 80);
    assert_eq!(lines.concat(), value);
    assert!(lines.iter().all(|line| display_width(line) <= 80));
}

#[test]
fn prompt_syntax_alias_and_exact_confirmation_are_publicly_typed() {
    let syntax: ConfirmationSyntax = ConfirmationSyntax::ExactScope;
    assert_eq!(syntax, crate::prompt::ExactScope::ExactScope);
    let confirmation = ExactConfirmation {
        action: PromptAction::ActivatePolicy,
        repository_id: "acme/widgets".to_owned(),
        object_id: "activation-1".to_owned(),
        scope: Some("event/release/high".to_owned()),
        config_hash: Some("a".repeat(64)),
        tuple_hash: Some("b".repeat(64)),
        plan_hash: None,
    };
    assert!(confirmation.is_activation());
    assert!(!confirmation.is_purge());
}
