use std::fmt::Write as _;

use repo_com_foundation::{ColorChoice, ErrorCategory, TtyMode};

use crate::preview::{
    ApprovalState, ApprovalView, DeliveryOutcome, DeliveryView, DestinationView, ErrorView,
    MetadataView, OutboundPreview, PolicyActivationView, PolicyState, PolicyView, Provenance,
    SafetyFindingView, SafetyState, SafetyView,
};
use crate::prompt::{
    ExactPreviewIdentity, KeyboardPrompt, PromptAction, PromptRequest, PromptResult,
    ScriptedPromptInput,
};
use crate::render::{
    ApprovalStatusView, OutboundView, PolicyStatusView, RenderOptions, SecretFindingStatusView,
    is_ansi_free, render_machine, render_machine_failure, render_view,
};
use crate::width::{display_width, lines_fit, wrap_text};
use repo_com_policy::PolicyTuple;

fn destination() -> DestinationView {
    DestinationView::new(
        "release",
        "100000000000000000",
        "200000000000000000",
        vec!["on_call=role:300000000000000000".to_owned()],
    )
}

fn metadata() -> MetadataView {
    MetadataView {
        repository_label: Some("widgets".to_owned()),
        branch: Some("main".to_owned()),
        commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
    }
}

fn active_policy() -> PolicyView {
    PolicyView {
        state: PolicyState::Active,
        tuple: Some(PolicyTuple::new("build_failed", "release", "high")),
        config_hash: Some("1".repeat(64)),
        tuple_hash: Some("2".repeat(64)),
        activation_id: Some("activation-1".to_owned()),
        recorded_config_hash: Some("1".repeat(64)),
        recorded_tuple_hash: Some("2".repeat(64)),
        current_config_hash: Some("1".repeat(64)),
        current_tuple_hash: Some("2".repeat(64)),
        activated_at: Some("2026-01-01T00:00:00Z".to_owned()),
        deactivated_at: None,
        stale_reason: None,
        basis: Some("one current exact activation".to_owned()),
        activations: vec![PolicyActivationView {
            activation_id: "activation-1".to_owned(),
            tuple: PolicyTuple::new("build_failed", "release", "high"),
            recorded_config_hash: "1".repeat(64),
            recorded_tuple_hash: "2".repeat(64),
            current_config_hash: "1".repeat(64),
            current_tuple_hash: "2".repeat(64),
            activated_at: "2026-01-01T00:00:00Z".to_owned(),
            deactivated_at: None,
            active: true,
            stale_reason: None,
        }],
    }
}

fn valid_approval() -> ApprovalView {
    ApprovalView {
        state: ApprovalState::Valid,
        approval_id: Some("approval-1".to_owned()),
        approval_hash: Some("3".repeat(64)),
        preview_hash: Some("4".repeat(64)),
        revision_hash: Some("5".repeat(64)),
        exact_text_hash: Some("6".repeat(64)),
        metadata_hash: Some("7".repeat(64)),
        config_hash: Some("8".repeat(64)),
        destination_hash: Some("9".repeat(64)),
        policy_basis_hash: Some("a".repeat(64)),
        scan_hash: Some("b".repeat(64)),
        draft_expires_at_unix_seconds: Some(2_000),
        expires_at_unix_seconds: Some(1_900),
        override_hash: None,
        actor_kind: Some("operator".to_owned()),
        reason: None,
    }
}

fn clear_safety() -> SafetyView {
    SafetyView {
        state: SafetyState::Clear,
        scan_hash: Some("b".repeat(64)),
        ..SafetyView::default()
    }
}

fn finding_safety() -> SafetyView {
    SafetyView {
        state: SafetyState::OverrideRequired,
        scan_hash: Some("c".repeat(64)),
        findings: vec![SafetyFindingView {
            reason_code: "authorization-value".to_owned(),
            source: "rendered-text".to_owned(),
            metadata_field: None,
            start: 12,
            end: 48,
            location: "rendered-text:12..48".to_owned(),
        }],
        ..SafetyView::default()
    }
}

fn preview() -> OutboundPreview {
    OutboundPreview {
        repository_id: "acme/widgets".to_owned(),
        draft_id: "draft-outbound-1".to_owned(),
        revision: 7,
        revision_hash: "5".repeat(64),
        destination: destination(),
        exact_text: "Build failed on main.\nPlease investigate the exact revision before sending any response."
            .to_owned(),
        exact_text_hash: "6".repeat(64),
        metadata: metadata(),
        event_type: "build_failed".to_owned(),
        severity: "high".to_owned(),
        reply_reference: None,
        created_at_unix_seconds: 1_000,
        expires_at_unix_seconds: 2_000,
        approval: valid_approval(),
        policy: active_policy(),
        safety: clear_safety(),
        preview_hash: "4".repeat(64),
        provenance: Provenance::LocalDecision,
    }
}

fn policy_view(state: PolicyState) -> PolicyStatusView {
    let mut policy = active_policy();
    policy.state = state;
    if state == PolicyState::Stale {
        policy.stale_reason = Some("ConfigHashChanged".to_owned());
        policy.recorded_config_hash = Some("f".repeat(64));
    }
    if state == PolicyState::Ambiguous {
        policy.basis = Some("2 matching activations".to_owned());
        if let Some(first) = policy.activations.first().cloned() {
            let mut second = first;
            second.activation_id = "activation-2".to_owned();
            policy.activations.push(second);
        }
    }
    PolicyStatusView::new("acme/widgets", "draft-outbound-1", 7, destination(), policy)
}

fn approval_view(state: ApprovalState) -> ApprovalStatusView {
    let mut approval = valid_approval();
    approval.state = state;
    ApprovalStatusView::new(
        "acme/widgets",
        "draft-outbound-1",
        7,
        destination(),
        approval,
    )
}

fn secret_view(state: SafetyState) -> SecretFindingStatusView {
    let mut safety = if state == SafetyState::Clear {
        clear_safety()
    } else {
        finding_safety()
    };
    safety.state = state;
    if state == SafetyState::OverrideRecorded {
        safety.override_hash = Some("d".repeat(64));
        safety.override_reason = Some("reviewed-false-positive".to_owned());
    }
    let mut view =
        SecretFindingStatusView::new("acme/widgets", "draft-outbound-1", 7, destination(), safety);
    view.preview_hash = Some("4".repeat(64));
    view.exact_text_hash = Some("6".repeat(64));
    view.expires_at_unix_seconds = 2_000;
    view
}

fn delivery(outcome: DeliveryOutcome) -> DeliveryView {
    let mut view = DeliveryView::new(
        "acme/widgets",
        "draft-outbound-1",
        7,
        destination(),
        outcome,
    );
    view.revision_hash = "5".repeat(64);
    view.exact_text = Some("Build failed on main.".to_owned());
    view.exact_text_hash = Some("6".repeat(64));
    view.metadata = metadata();
    view.expires_at_unix_seconds = 2_000;
    view.approval = valid_approval();
    view.policy = active_policy();
    view.safety = clear_safety();
    view.attempt_id = Some("attempt-1".to_owned());
    view.attempt_number = Some(1);
    view
}

fn views() -> Vec<(&'static str, OutboundView)> {
    vec![
        ("preview", OutboundView::preview(preview())),
        (
            "policy-active",
            OutboundView::policy(policy_view(PolicyState::Active)),
        ),
        (
            "policy-stale",
            OutboundView::policy(policy_view(PolicyState::Stale)),
        ),
        (
            "policy-not-activated",
            OutboundView::policy(policy_view(PolicyState::NotActivated)),
        ),
        (
            "policy-not-configured",
            OutboundView::policy(policy_view(PolicyState::NotConfigured)),
        ),
        (
            "policy-deactivated",
            OutboundView::policy(policy_view(PolicyState::Deactivated)),
        ),
        (
            "policy-ambiguous",
            OutboundView::policy(policy_view(PolicyState::Ambiguous)),
        ),
        (
            "approval-valid",
            OutboundView::approval(approval_view(ApprovalState::Valid)),
        ),
        (
            "approval-expired",
            OutboundView::approval(approval_view(ApprovalState::Expired)),
        ),
        (
            "approval-missing",
            OutboundView::approval(approval_view(ApprovalState::Missing)),
        ),
        (
            "approval-invalid",
            OutboundView::approval(approval_view(ApprovalState::Invalid)),
        ),
        (
            "approval-stale",
            OutboundView::approval(approval_view(ApprovalState::Stale)),
        ),
        (
            "approval-revoked",
            OutboundView::approval(approval_view(ApprovalState::Revoked)),
        ),
        (
            "approval-override-required",
            OutboundView::approval(approval_view(ApprovalState::OverrideRequired)),
        ),
        (
            "secret-clear",
            OutboundView::secret_finding(secret_view(SafetyState::Clear)),
        ),
        (
            "secret-finding",
            OutboundView::secret_finding(secret_view(SafetyState::Finding)),
        ),
        (
            "secret-override-required",
            OutboundView::secret_finding(secret_view(SafetyState::OverrideRequired)),
        ),
        (
            "secret-expired",
            OutboundView::secret_finding(secret_view(SafetyState::Expired)),
        ),
        (
            "secret-not-evaluated",
            OutboundView::secret_finding(secret_view(SafetyState::NotEvaluated)),
        ),
        (
            "secret-override",
            OutboundView::secret_finding(secret_view(SafetyState::OverrideRecorded)),
        ),
        (
            "delivery-unclaimed",
            OutboundView::delivery(delivery(DeliveryOutcome::Unclaimed)),
        ),
        (
            "delivery-claimed",
            OutboundView::delivery(delivery(DeliveryOutcome::Claimed)),
        ),
        (
            "delivery-accepted",
            OutboundView::delivery(delivery(DeliveryOutcome::Accepted {
                message_id: "300000000000000000".to_owned(),
            })),
        ),
        (
            "delivery-failed",
            OutboundView::delivery(delivery(DeliveryOutcome::Failed {
                code: "permission-denied".to_owned(),
            })),
        ),
        (
            "delivery-retry-wait",
            OutboundView::delivery(delivery(DeliveryOutcome::RetryWait {
                next_attempt: 2,
                delay_seconds: Some(3),
                delay_kind: "discord-directed".to_owned(),
            })),
        ),
        (
            "delivery-unknown",
            OutboundView::delivery(delivery(DeliveryOutcome::Unknown {
                reason: "post-dispatch-timeout".to_owned(),
            })),
        ),
        (
            "delivery-reconciliation-unknown",
            OutboundView::delivery(delivery(DeliveryOutcome::ReconciliationUnknown {
                reason: "no-match-yet".to_owned(),
                successful_reads: 2,
                observation_started_at_unix_seconds: 1_200,
                last_successful_read_at_unix_seconds: Some(1_500),
            })),
        ),
        (
            "delivery-reconciled-accepted",
            OutboundView::delivery(delivery(DeliveryOutcome::ReconciledAccepted {
                message_id: "300000000000000000".to_owned(),
                successful_reads: Some(1),
                observed_at_unix_seconds: Some(1_500),
            })),
        ),
        (
            "delivery-reconciled-absent",
            OutboundView::delivery(delivery(DeliveryOutcome::ReconciledAbsent {
                successful_reads: Some(4),
                observation_started_at_unix_seconds: Some(1_200),
                observed_at_unix_seconds: Some(1_600),
            })),
        ),
        (
            "delivery-unresolved",
            OutboundView::delivery(delivery(DeliveryOutcome::Unresolved {
                reason: "conflicting-content".to_owned(),
                successful_reads: Some(2),
            })),
        ),
        (
            "eligibility-rejected",
            OutboundView::delivery(delivery(DeliveryOutcome::EligibilityRejected {
                blocker: "authority-missing".to_owned(),
            })),
        ),
        (
            "delivery-expired",
            OutboundView::delivery(delivery(DeliveryOutcome::Expired)),
        ),
        (
            "delivery-stale-authority",
            OutboundView::delivery(delivery(DeliveryOutcome::StaleAuthority {
                reason: "approval-state-changed".to_owned(),
            })),
        ),
        (
            "delivery-error",
            OutboundView::delivery(delivery(DeliveryOutcome::Error {
                category: "storage-integrity".to_owned(),
                detail: "redacted local state failure".to_owned(),
            })),
        ),
        (
            "local-error",
            OutboundView::error(ErrorView::new(
                "connectivity-rate-limit",
                "redacted transport diagnostic",
                "retry after the bounded server-directed wait",
            )),
        ),
    ]
}

fn snapshot_text() -> String {
    let mut output = String::new();
    for (name, view) in views() {
        writeln!(&mut output, "===== {name} =====").expect("snapshot string is writable");
        output.push_str(&render_view(&view, RenderOptions::plain_text()));
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
                "/tests/snapshots/terminal_outbound_contract.snap"
            ),
            &actual,
        )
        .expect("snapshot update is writable");
        return;
    }
    assert_eq!(
        actual,
        include_str!("snapshots/terminal_outbound_contract.snap")
    );
}

#[test]
fn every_named_view_is_complete_at_80_columns_without_ansi() {
    for (name, view) in views() {
        let rendered = render_view(&view, RenderOptions::plain_text());
        assert!(is_ansi_free(&rendered), "{name} contains ANSI");
        assert!(lines_fit(&rendered, 80), "{name} exceeds 80 columns");
        for label in [
            "Repository:",
            "Object:",
            "Revision:",
            "Destination:",
            "Outcome:",
            "Next action:",
        ] {
            assert!(rendered.contains(label), "{name} lacks {label}");
        }
    }
}

#[test]
fn basis_and_action_meaning_is_expressed_as_text_labels() {
    let preview = render_view(
        &OutboundView::preview(preview()),
        RenderOptions::plain_text(),
    );
    for label in [
        "Destination:",
        "Revision:",
        "Approval state:",
        "Policy state:",
        "Safety state:",
        "Outcome:",
        "Next action:",
    ] {
        assert!(preview.contains(label), "preview lacks {label}");
    }
    let delivery = render_view(
        &OutboundView::delivery(delivery(DeliveryOutcome::Accepted {
            message_id: "300000000000000000".to_owned(),
        })),
        RenderOptions::plain_text(),
    );
    for label in ["Outcome: accepted", "Provenance:", "Next action:"] {
        assert!(delivery.contains(label), "delivery lacks {label}");
    }
}

#[test]
fn no_color_and_explicit_non_color_are_text_only() {
    assert_eq!(
        RenderOptions::resolve_color(ColorChoice::Always, TtyMode::Tty, true),
        ColorChoice::Never
    );
    let view = OutboundView::preview(preview());
    let explicit = render_view(
        &view,
        RenderOptions::with_color(ColorChoice::Never, TtyMode::Tty),
    );
    let env_safe = render_view(
        &view,
        RenderOptions::from_environment(ColorChoice::Never, TtyMode::Tty),
    );
    assert!(is_ansi_free(&explicit));
    assert!(is_ansi_free(&env_safe));
    assert_eq!(explicit, env_safe);
    let colored = render_view(
        &view,
        RenderOptions::with_color(ColorChoice::Always, TtyMode::Tty),
    );
    assert!(!is_ansi_free(&colored));
    assert!(colored.contains("Outcome:"));
    assert!(colored.contains("Next action:"));
}

#[test]
fn long_hashes_findings_and_actions_are_not_truncated() {
    let mut view = secret_view(SafetyState::OverrideRequired);
    view.safety.findings[0].reason_code =
        "a-reason-code-long-enough-to-wrap-at-eighty-columns".to_owned();
    view.safety.findings[0].location = "rendered-text:123456789..123456789".to_owned();
    view.approval.reason = Some(
        "a-safe-reason-that-must-remain-visible-even-when-it-is-longer-than-eighty-columns"
            .to_owned(),
    );
    let rendered = render_view(
        &OutboundView::secret_finding(view),
        RenderOptions::with_width(80),
    );
    assert!(rendered.contains("a-reason-code-long-enough-to-wrap-at-eighty-columns"));
    assert!(rendered.contains("a-safe-reason-that-must-remain-visible"));
    assert!(lines_fit(&rendered, 80));
}

#[test]
fn blocked_delivery_outcomes_expose_stable_categories() {
    let unknown = DeliveryOutcome::Unknown {
        reason: "post-dispatch-timeout".to_owned(),
    };
    assert_eq!(
        unknown.error_category(),
        Some(ErrorCategory::UnknownDelivery)
    );
    assert_eq!(unknown.exit_code(), Some(8));
    let rejected = DeliveryOutcome::EligibilityRejected {
        blocker: "operator-action-required".to_owned(),
    };
    assert_eq!(
        rejected.error_category(),
        Some(ErrorCategory::OperatorActionRequired)
    );
    assert_eq!(rejected.exit_code(), Some(3));
}

#[test]
fn accepted_delivery_does_not_claim_attention_or_response() {
    let rendered = render_view(
        &OutboundView::delivery(delivery(DeliveryOutcome::Accepted {
            message_id: "300000000000000000".to_owned(),
        })),
        RenderOptions::plain_text(),
    );
    assert!(rendered.contains("Outcome: accepted"));
    assert!(rendered.contains("no recipient-attention or response claim"));
    assert!(!rendered.contains("Outcome: read"));
    assert!(!rendered.contains("Outcome: replied"));
}

#[test]
fn json_view_is_one_deterministic_value() {
    let json = OutboundView::preview(preview())
        .to_json()
        .expect("preview serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("one JSON value");
    assert_eq!(value["view"], "preview");
    assert_eq!(value["repository_id"], "acme/widgets");
}

#[test]
fn machine_mode_uses_one_protocol_envelope_and_stable_error_category() {
    let success = render_machine(&OutboundView::preview(preview())).expect("success JSON");
    let success_value: serde_json::Value =
        serde_json::from_str(&success).expect("one success JSON object");
    assert_eq!(success_value["protocol_version"], 1);
    assert_eq!(success_value["status"], "success");
    assert_eq!(success_value["data"]["view"], "preview");

    let failure = render_machine_failure(&ErrorView::new(
        "storage-integrity",
        "redacted detail",
        "repair local state",
    ))
    .expect("failure JSON");
    let failure_value: serde_json::Value =
        serde_json::from_str(&failure).expect("one failure JSON object");
    assert_eq!(failure_value["protocol_version"], 1);
    assert_eq!(failure_value["status"], "error");
    assert_eq!(failure_value["error"]["code"], "storage-integrity");
    let automatic_failure = render_machine(&OutboundView::error(ErrorView::new(
        "operator-action-required",
        "redacted detail",
        "run on a TTY",
    )))
    .expect("automatic failure JSON");
    let automatic_value: serde_json::Value =
        serde_json::from_str(&automatic_failure).expect("one automatic failure object");
    assert_eq!(automatic_value["status"], "error");
    assert_eq!(automatic_value["error"]["code"], "operator-action-required");
}

#[test]
fn prompt_cancel_default_invalid_expiry_and_non_tty_are_distinct() {
    let cases = [
        (vec!["n"], TtyMode::Tty, 1_500, false),
        (vec![""], TtyMode::Tty, 1_500, false),
        (vec!["not-a-command"], TtyMode::Tty, 1_500, false),
        (vec!["y"], TtyMode::NonTty, 1_500, false),
        (vec!["y"], TtyMode::Tty, 2_000, false),
        (vec!["y"], TtyMode::Tty, 1_900, false),
    ];
    let expected = [
        PromptResult::Cancelled,
        PromptResult::Defaulted,
        PromptResult::InvalidInput {
            input: "invalid input (redacted)".to_owned(),
        },
        PromptResult::NonTty,
        PromptResult::Expired,
        PromptResult::Expired,
    ];
    for ((lines, tty, now, _), expected) in cases.into_iter().zip(expected) {
        let input = ScriptedPromptInput::new(lines);
        let mut prompt = KeyboardPrompt::new(input, tty, now);
        assert_eq!(
            prompt.request(&PromptRequest::approval(preview())),
            expected
        );
        if matches!(tty, TtyMode::NonTty) || now >= 1_900 {
            assert_eq!(prompt.input().calls(), 0);
        }
    }
}

#[test]
fn exact_approval_and_override_prompts_bind_the_complete_preview() {
    let input = ScriptedPromptInput::new([format!("approve {}", "4".repeat(64))]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty, 1_500);
    let result = prompt.request_exact_approval(preview());
    let confirmation = result.confirmation().expect("approval confirmation");
    assert_eq!(confirmation.action, PromptAction::Approve);
    assert!(confirmation.matches(&ExactPreviewIdentity::from_preview(&preview())));
    assert!(prompt.input().prompts()[0].contains("Exact text:"));
    assert!(prompt.input().prompts()[0].contains("keyboard:"));

    let input = ScriptedPromptInput::new([format!("override {}", "4".repeat(64))]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty, 1_500);
    let mut override_preview = preview();
    override_preview.safety = finding_safety();
    let result = prompt.request_secret_override(override_preview);
    assert_eq!(
        result.confirmation().expect("override confirmation").action,
        PromptAction::OverrideSecretFinding
    );
}

#[test]
fn non_tty_never_invokes_the_secret_override_prompt() {
    let input = ScriptedPromptInput::new(["override"]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::NonTty, 1_500);
    let result = prompt.request_secret_override(preview());
    assert_eq!(result, PromptResult::NonTty);
    assert_eq!(prompt.input().calls(), 0);
}

#[test]
fn exact_hash_mismatch_is_invalid_and_non_authoritative() {
    let input = ScriptedPromptInput::new([format!("approve {}", "f".repeat(64))]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty, 1_500);
    let result = prompt.request_exact_approval(preview());
    assert!(matches!(result, PromptResult::PreviewHashMismatch { .. }));
    assert!(!result.is_confirmed());
}

#[test]
fn width_helper_preserves_all_content_when_wrapping() {
    let value = "x".repeat(241);
    let wrapped = wrap_text(&value, 80);
    assert_eq!(wrapped.concat(), value);
    assert!(wrapped.iter().all(|line| display_width(line) <= 80));
}

#[test]
fn prompt_text_is_plain_and_contains_keyboard_actions() {
    let input = ScriptedPromptInput::new(["n"]);
    let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty, 1_500)
        .with_render_options(RenderOptions::with_color(ColorChoice::Never, TtyMode::Tty));
    let result = prompt.request(&PromptRequest::approval(preview()));
    assert_eq!(result, PromptResult::Cancelled);
    let text = &prompt.input().prompts()[0];
    assert!(is_ansi_free(text));
    assert!(text.contains("[Y/yes]") || text.contains("Y/yes"));
    assert!(text.contains("N/no"));
    assert!(text.contains("Enter"));
    assert!(text.contains("non-authoritative"));
    assert!(text.contains("Focus:"));
    assert!(lines_fit(text, 80));
}
