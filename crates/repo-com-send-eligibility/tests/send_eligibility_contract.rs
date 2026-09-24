use std::{collections::BTreeMap, path::Path};

use crate::{
    EligibilityAuthority, EligibilityBlocker, EligibilityDecision, EligibilityInput,
    OutboundCorrection, RevalidationFacts, evaluate,
};
use repo_com_approval::{
    ApprovalInstant, ApprovalPreview, ApprovalRecord, ApprovalService,
    OperatorConfirmation as ApprovalConfirmation, OverrideReasonCode,
};
use repo_com_config::{
    AutoSendEntry, DestinationConfig, DiscordConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};
use repo_com_draft_content::{ContentRenderer, RenderedMessage};
use repo_com_draft_model::{DraftModel, DraftPreview, DraftRequest};
use repo_com_draft_safety::SecretScanner;
use repo_com_foundation::TtyMode;
use repo_com_policy::{
    OperatorConfirmation as PolicyConfirmation, PolicyRegistry, PolicyTuple,
    activation_records_from_state, evaluate_from_state,
};
use repo_com_state::{DraftInput, DraftRevisionInput, RepositoryInput, StateStore};

const REPOSITORY: &str = "acme/widgets";
const DRAFT_ID: &str = "draft-eligibility";
const CREATED_AT: u64 = 1_000;
const STATE_TIMESTAMP: &str = "2026-01-01T00:00:00Z";
const BODY: &str = "build failed on main";
const FINDING_BODY: &str =
    "deploy failed\nAuthorization: Bearer synthetic-not-a-real-token-1234567890";

struct Scenario {
    service: ApprovalService,
    config: ResolvedConfig,
    draft: DraftModel,
    preview: DraftPreview,
    approval_preview: ApprovalPreview,
    rendered: RenderedMessage,
}

fn config(channel_id: &str, include_policy: bool) -> ResolvedConfig {
    config_with_policies(
        channel_id,
        include_policy
            .then(|| AutoSendEntry {
                event_type: "build_failed".to_owned(),
                destination: "release".to_owned(),
                severity: "high".to_owned(),
            })
            .into_iter()
            .collect(),
    )
}

fn config_with_policies(channel_id: &str, auto_send: Vec<AutoSendEntry>) -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: channel_id.to_owned(),
            allowed_mentions: Vec::new(),
        },
    );

    resolve_model(
        &RepositoryConfig {
            schema_version: 1,
            repository_id: REPOSITORY.to_owned(),
            discord: DiscordConfig {
                workspace_id: "100".to_owned(),
            },
            destinations,
            mentions: BTreeMap::new(),
            inbound: BTreeMap::new(),
            retention: RetentionConfig::default(),
            auto_send,
        },
        Path::new("/send-eligibility-contract/.repo-com.toml"),
    )
    .expect("the synthetic eligibility configuration is valid")
}

fn changed_retention(current: &ResolvedConfig) -> ResolvedConfig {
    let mut model = current.config.clone();
    model.retention.content_days += 1;
    resolve_model(
        &model,
        Path::new("/send-eligibility-contract/.repo-com.toml"),
    )
    .expect("the changed synthetic configuration is valid")
}

fn changed_destination() -> ResolvedConfig {
    config("202", false)
}

fn request(body: &str, expiry: u64) -> DraftRequest {
    DraftRequest::new(DRAFT_ID, "release", body, "build_failed", "high")
        .expect("the synthetic draft request is valid")
        .with_expiry_seconds(expiry)
}

fn tuple() -> PolicyTuple {
    PolicyTuple::new("build_failed", "release", "high")
}

fn instant(seconds: u64) -> ApprovalInstant {
    ApprovalInstant::new(seconds, STATE_TIMESTAMP).expect("the synthetic approval instant is valid")
}

fn scenario(body: &str, expiry: u64, config: ResolvedConfig, activate_policy: bool) -> Scenario {
    let mut state = StateStore::open_in_memory().expect("open in-memory state");
    state
        .upsert_repository(&RepositoryInput::new(
            REPOSITORY,
            "100",
            config.canonical_hash(),
            STATE_TIMESTAMP,
        ))
        .expect("register repository");
    state
        .create_draft(&DraftInput::new(
            REPOSITORY,
            DRAFT_ID,
            "build_failed",
            "release",
            STATE_TIMESTAMP,
        ))
        .expect("create draft");

    let draft = DraftModel::create(request(body, expiry), &config, CREATED_AT)
        .expect("create immutable draft revision");
    let revision = draft.current_revision();
    let rendered = ContentRenderer::new()
        .render_revision(revision, CREATED_AT)
        .expect("render immutable revision");
    let preview = draft
        .preview_with_rendered_text(1, rendered.exact_text(), CREATED_AT)
        .expect("create exact draft preview");
    let mut revision_input = DraftRevisionInput::new(
        REPOSITORY,
        DRAFT_ID,
        1,
        revision.content_hash(),
        rendered.exact_text(),
        revision.destination_alias().as_str(),
        serde_json::to_string(revision.resolved_destination())
            .expect("resolved destination serializes"),
        STATE_TIMESTAMP,
    );
    revision_input.metadata_json =
        serde_json::to_string(revision.metadata()).expect("metadata serializes");
    revision_input.expiry_at = Some(revision.expiry().expires_at_unix_seconds().to_string());
    state
        .insert_draft_revision(&revision_input)
        .expect("persist immutable revision");

    let mut service = ApprovalService::new(state);
    if activate_policy {
        PolicyRegistry::new(service.state_mut())
            .activate(
                &config.config,
                &tuple(),
                PolicyConfirmation::confirmed(TtyMode::Tty)
                    .expect("synthetic policy TTY confirmation"),
                STATE_TIMESTAMP,
            )
            .expect("activate exact policy");
    }

    let approval_preview = service
        .preview_at(&preview, &config, &instant(CREATED_AT))
        .expect("build baseline approval preview");

    Scenario {
        service,
        config,
        draft,
        preview,
        approval_preview,
        rendered,
    }
}

fn approve(scenario: &mut Scenario, at: u64) -> ApprovalRecord {
    let preview = scenario
        .service
        .preview_at(&scenario.preview, &scenario.config, &instant(at))
        .expect("build complete approval preview");
    let confirmation = ApprovalConfirmation::confirmed(&preview, TtyMode::Tty)
        .expect("synthetic approval TTY confirmation");
    scenario
        .service
        .approve(
            &scenario.preview,
            &scenario.config,
            &confirmation,
            &FixedInstant(instant(at)),
        )
        .expect("record exact approval")
}

fn approve_finding_with_override(scenario: &mut Scenario, at: u64) -> ApprovalRecord {
    let preview = scenario
        .service
        .preview_at(&scenario.preview, &scenario.config, &instant(at))
        .expect("build finding preview");
    let confirmation = ApprovalConfirmation::confirmed(&preview, TtyMode::Tty)
        .expect("synthetic finding TTY confirmation");
    scenario
        .service
        .override_secret_finding(
            &scenario.preview,
            &scenario.config,
            OverrideReasonCode::ReviewedFalsePositive,
            &confirmation,
            &FixedInstant(instant(at)),
        )
        .expect("record exact redacted override");
    scenario
        .service
        .approve(
            &scenario.preview,
            &scenario.config,
            &confirmation,
            &FixedInstant(instant(at)),
        )
        .expect("approve exact overridden revision")
}

struct FixedInstant(ApprovalInstant);

impl repo_com_approval::ApprovalClock for FixedInstant {
    fn now(&self) -> ApprovalInstant {
        self.0.clone()
    }
}

fn evaluate_current(
    scenario: &Scenario,
    config: &ResolvedConfig,
    now: u64,
    tty_mode: TtyMode,
) -> EligibilityDecision {
    let generated_preview = scenario
        .service
        .preview_at(&scenario.preview, config, &instant(now));
    let approval_preview = generated_preview.unwrap_or_else(|_| scenario.approval_preview.clone());
    let approval_check = scenario
        .service
        .check_current(&scenario.preview, config, &FixedInstant(instant(now)))
        .expect("read-only approval check");
    let policy_decision = evaluate_from_state(scenario.service.state(), &config.config, &tuple())
        .expect("read-only policy evaluation");
    let scan_result = SecretScanner::new().scan_rendered(&scenario.rendered);

    evaluate(&EligibilityInput {
        now_unix_seconds: now,
        tty_mode,
        config,
        revision: scenario.draft.current_revision(),
        approval_preview: &approval_preview,
        approval_check: &approval_check,
        policy_decision: &policy_decision,
        scan_result: &scan_result,
    })
}

fn assert_sha256(value: &str) {
    assert_eq!(value.len(), 64, "not a SHA-256 hex digest: {value}");
    assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

fn assert_complete_facts(facts: &RevalidationFacts) {
    assert_eq!(facts.repository_id, REPOSITORY);
    assert_sha256(&facts.repository_hash);
    assert_eq!(facts.draft_id, DRAFT_ID);
    assert_eq!(facts.revision, 1);
    assert_sha256(&facts.revision_hash);
    assert_sha256(&facts.destination_hash);
    assert_sha256(&facts.exact_text_hash);
    assert_sha256(&facts.metadata_hash);
    assert_sha256(&facts.config_hash);
    assert_sha256(&facts.policy_basis_hash);
    assert_sha256(&facts.scan_hash);
    assert_sha256(&facts.approval_preview_hash);
}

#[test]
fn valid_exact_approval_is_eligible_with_complete_revalidation_hashes() {
    let mut scenario = scenario(BODY, 5_000, config("201", false), false);
    let approval = approve(&mut scenario, 1_000);
    let decision = evaluate_current(&scenario, &scenario.config.clone(), 1_100, TtyMode::Tty);

    assert!(decision.is_eligible(), "{decision:?}");
    assert!(decision.requires_atomic_revalidation());
    assert!(!decision.is_send_claim());
    assert_eq!(decision.correction(), Some(OutboundCorrection::NewDraft));
    assert_complete_facts(decision.revalidation());
    assert_eq!(
        decision.revalidation().config_hash,
        scenario.config.canonical_hash()
    );
    match decision.authority() {
        Some(EligibilityAuthority::HumanApproval {
            approval_id,
            approval_hash,
            expires_at_unix_seconds,
        }) => {
            assert_eq!(approval_id, &approval.approval_id);
            assert_sha256(approval_hash);
            assert_eq!(*expires_at_unix_seconds, approval.expires_at_unix_seconds);
        }
        other => panic!("unexpected authority: {other:?}"),
    }
}

#[test]
fn valid_exact_activated_policy_is_eligible_without_approval() {
    let scenario = scenario(BODY, 5_000, config("201", true), true);
    let decision = evaluate_current(&scenario, &scenario.config.clone(), 1_100, TtyMode::Tty);

    assert!(decision.is_eligible(), "{decision:?}");
    assert_complete_facts(decision.revalidation());
    assert!(
        scenario
            .service
            .state()
            .approval(REPOSITORY, DRAFT_ID, 1)
            .expect("approval lookup")
            .is_none()
    );
    match decision.authority() {
        Some(EligibilityAuthority::ActivatedPolicy {
            activation_id,
            activation_hash,
            config_hash,
            tuple_hash,
            ..
        }) => {
            assert!(!activation_id.is_empty());
            assert_sha256(activation_hash);
            assert_eq!(config_hash, &scenario.config.canonical_hash());
            assert_sha256(tuple_hash);
        }
        other => panic!("unexpected authority: {other:?}"),
    }
}

#[test]
fn missing_authority_fails_closed_and_non_tty_requires_operator_action() {
    let scenario = scenario(BODY, 5_000, config("201", false), false);
    let interactive = evaluate_current(&scenario, &scenario.config.clone(), 1_100, TtyMode::Tty);
    let automated = evaluate_current(&scenario, &scenario.config.clone(), 1_100, TtyMode::NonTty);

    assert_eq!(
        interactive.blocker(),
        Some(EligibilityBlocker::AuthorityMissing)
    );
    assert_eq!(
        automated.blocker(),
        Some(EligibilityBlocker::OperatorActionRequired)
    );
    assert!(!interactive.is_eligible());
    assert!(!automated.is_eligible());
}

#[test]
fn approval_and_earlier_draft_expiry_boundaries_fail_closed() {
    let mut approved = scenario(BODY, 5_000, config("201", false), false);
    approve(&mut approved, 1_000);
    let expired_approval =
        evaluate_current(&approved, &approved.config.clone(), 1_900, TtyMode::Tty);
    assert_eq!(
        expired_approval.blocker(),
        Some(EligibilityBlocker::ApprovalExpired)
    );

    let mut earlier_draft = scenario(BODY, 500, config("201", false), false);
    approve(&mut earlier_draft, 1_000);
    let expired_draft = evaluate_current(
        &earlier_draft,
        &earlier_draft.config.clone(),
        1_500,
        TtyMode::Tty,
    );
    assert_eq!(
        expired_draft.blocker(),
        Some(EligibilityBlocker::DraftExpired)
    );
}

#[test]
fn stale_activation_and_changed_config_fail_closed() {
    let activated = scenario(BODY, 5_000, config("201", true), true);
    let stale = evaluate_current(
        &activated,
        &changed_retention(&activated.config),
        1_100,
        TtyMode::Tty,
    );
    assert_eq!(
        stale.blocker(),
        Some(EligibilityBlocker::StalePolicyActivation)
    );

    let mut approved = scenario(BODY, 5_000, config("201", false), false);
    approve(&mut approved, 1_000);
    let changed = evaluate_current(
        &approved,
        &changed_retention(&approved.config),
        1_100,
        TtyMode::Tty,
    );
    assert_eq!(changed.blocker(), Some(EligibilityBlocker::ConfigChanged));
}

#[test]
fn changed_destination_resolution_fails_closed() {
    let mut approved = scenario(BODY, 5_000, config("201", false), false);
    approve(&mut approved, 1_000);
    let changed = evaluate_current(&approved, &changed_destination(), 1_100, TtyMode::Tty);

    assert_eq!(
        changed.blocker(),
        Some(EligibilityBlocker::DestinationChanged)
    );
    assert!(!changed.is_eligible());
}

#[test]
fn unresolved_scan_finding_blocks_unless_exact_override_is_bound() {
    let unresolved_scenario = scenario(FINDING_BODY, 5_000, config("201", false), false);
    let unresolved = evaluate_current(
        &unresolved_scenario,
        &unresolved_scenario.config.clone(),
        1_100,
        TtyMode::Tty,
    );
    assert_eq!(
        unresolved.blocker(),
        Some(EligibilityBlocker::UnresolvedSecretFinding)
    );

    let mut overridden = scenario(FINDING_BODY, 5_000, config("201", false), false);
    approve_finding_with_override(&mut overridden, 1_000);
    let resolved = evaluate_current(&overridden, &overridden.config.clone(), 1_100, TtyMode::Tty);
    assert!(resolved.is_eligible(), "{resolved:?}");
    assert_complete_facts(resolved.revalidation());
}

#[test]
fn non_tty_can_consume_existing_authority_but_creates_none() {
    let mut approved = scenario(BODY, 5_000, config("201", false), false);
    approve(&mut approved, 1_000);
    let approval_before = approved
        .service
        .state()
        .approval(REPOSITORY, DRAFT_ID, 1)
        .expect("approval lookup")
        .is_some();
    let activations_before = activation_records_from_state(approved.service.state(), REPOSITORY)
        .expect("activation lookup")
        .len();
    let override_event = format!(
        "secret-override-{}",
        approved.approval_preview.preview_hash()
    );
    let override_before = approved
        .service
        .state()
        .audit_event(REPOSITORY, &override_event)
        .expect("override audit lookup")
        .is_some();
    let approval_decision =
        evaluate_current(&approved, &approved.config.clone(), 1_100, TtyMode::NonTty);
    assert!(approval_decision.is_eligible());
    assert_eq!(
        approved
            .service
            .state()
            .approval(REPOSITORY, DRAFT_ID, 1)
            .expect("approval lookup")
            .is_some(),
        approval_before
    );
    assert_eq!(
        activation_records_from_state(approved.service.state(), REPOSITORY)
            .expect("activation lookup")
            .len(),
        activations_before
    );
    assert_eq!(
        approved
            .service
            .state()
            .audit_event(REPOSITORY, &override_event)
            .expect("override audit lookup")
            .is_some(),
        override_before
    );

    let policy = scenario(BODY, 5_000, config("201", true), true);
    let policy_approval_before = policy
        .service
        .state()
        .approval(REPOSITORY, DRAFT_ID, 1)
        .expect("approval lookup")
        .is_some();
    let policy_activations_before =
        activation_records_from_state(policy.service.state(), REPOSITORY)
            .expect("activation lookup")
            .len();
    let policy_override_event =
        format!("secret-override-{}", policy.approval_preview.preview_hash());
    let policy_override_before = policy
        .service
        .state()
        .audit_event(REPOSITORY, &policy_override_event)
        .expect("override audit lookup")
        .is_some();
    let policy_decision = evaluate_current(&policy, &policy.config.clone(), 1_100, TtyMode::NonTty);
    assert!(policy_decision.is_eligible());
    assert_eq!(
        policy
            .service
            .state()
            .approval(REPOSITORY, DRAFT_ID, 1)
            .expect("approval lookup")
            .is_some(),
        policy_approval_before
    );
    assert_eq!(
        activation_records_from_state(policy.service.state(), REPOSITORY)
            .expect("activation lookup")
            .len(),
        policy_activations_before
    );
    assert_eq!(
        policy
            .service
            .state()
            .audit_event(REPOSITORY, &policy_override_event)
            .expect("override audit lookup")
            .is_some(),
        policy_override_before
    );
}

#[test]
fn outbound_corrections_require_new_draft_or_validated_reply_with_no_mutation_api() {
    let mut approved = scenario(BODY, 5_000, config("201", false), false);
    approve(&mut approved, 1_000);
    let decision = evaluate_current(&approved, &approved.config.clone(), 1_100, TtyMode::Tty);
    assert_eq!(decision.correction(), Some(OutboundCorrection::NewDraft));
    assert_ne!(
        decision.correction(),
        Some(OutboundCorrection::ValidatedThreadedReply)
    );

    let sources = [
        include_str!("../src/lib.rs"),
        include_str!("../src/decision.rs"),
        include_str!("../src/evaluator.rs"),
    ]
    .concat();
    for forbidden in [
        "pub fn edit",
        "pub fn delete",
        "pub async fn edit",
        "pub async fn delete",
        "pub fn edit_message",
        "pub fn delete_message",
        "pub fn claim",
        "pub async fn claim",
        "pub fn approve",
        "pub fn activate",
        "pub fn override",
    ] {
        assert!(
            !sources.contains(forbidden),
            "eligibility API unexpectedly exposes {forbidden}"
        );
    }
}
