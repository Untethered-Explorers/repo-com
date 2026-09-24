use std::{collections::BTreeMap, path::Path};

use crate::{
    APPROVAL_LIFETIME_SECONDS, ApprovalBindingField, ApprovalCheck, ApprovalClock,
    ApprovalDisposition, ApprovalError, ApprovalInstant, ApprovalInvalidReason, ApprovalService,
    OperatorConfirmation, OverrideReasonCode, PreviewInvalidReason,
};
use repo_com_config::{
    AutoSendEntry, DestinationConfig, DiscordConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};
use repo_com_draft_model::{DraftMetadata, DraftModel, DraftPreview, DraftRequest};
use repo_com_foundation::TtyMode;
use repo_com_policy::{
    OperatorConfirmation as PolicyOperatorConfirmation, PolicyRegistry, PolicyTuple,
};
use repo_com_state::{DraftInput, DraftRevisionInput, RepositoryInput, StateStore};

const REPOSITORY: &str = "acme/widgets";
const DRAFT_ID: &str = "draft-approval";
const CREATED_AT: u64 = 1_000;
const BODY: &str = "build failed on main";
const STATE_TIMESTAMP: &str = "2026-01-01T00:00:00Z";
const CLOCK_TIMESTAMP: &str = "2026-01-01T00:10:00Z";
const FINDING_TEXT: &str =
    "deploy failed\nAuthorization: Bearer synthetic-not-a-real-token-1234567890";

struct FixedClock(ApprovalInstant);

impl FixedClock {
    fn new(unix_seconds: u64) -> Self {
        Self(
            ApprovalInstant::new(unix_seconds, CLOCK_TIMESTAMP)
                .expect("the synthetic clock timestamp is canonical UTC"),
        )
    }
}

impl ApprovalClock for FixedClock {
    fn now(&self) -> ApprovalInstant {
        self.0.clone()
    }
}

fn config(release_channel: &str, include_policy: bool) -> ResolvedConfig {
    let auto_send = include_policy
        .then(|| AutoSendEntry {
            event_type: "build_failed".to_owned(),
            destination: "release".to_owned(),
            severity: "high".to_owned(),
        })
        .into_iter()
        .collect();
    config_with_auto_send(release_channel, auto_send)
}

fn config_with_auto_send(release_channel: &str, auto_send: Vec<AutoSendEntry>) -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: release_channel.to_owned(),
            allowed_mentions: Vec::new(),
        },
    );
    destinations.insert(
        "staging".to_owned(),
        DestinationConfig {
            channel_id: "202".to_owned(),
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
        Path::new("/approval-contract/.repo-com.toml"),
    )
    .expect("the synthetic approval configuration is valid")
}

fn request(alias: &str, body: &str, branch: &str, expiry: u64) -> DraftRequest {
    DraftRequest::new(DRAFT_ID, alias, body, "build_failed", "high")
        .expect("the synthetic draft request is valid")
        .with_metadata(
            DraftMetadata::default()
                .with_branch(branch)
                .expect("branch metadata is valid")
                .with_commit("abc123")
                .expect("commit metadata is valid"),
        )
        .with_expiry_seconds(expiry)
}

fn final_text(body: &str, revision_hash: &str) -> String {
    format!("{body}\n---\nrepo-com-revision: {revision_hash}")
}

fn persist_revision(
    state: &mut StateStore,
    draft: &DraftModel,
    revision_number: u64,
    body: &str,
) -> DraftPreview {
    let revision = draft
        .revision(revision_number)
        .expect("the immutable revision exists");
    let exact_text = final_text(body, revision.content_hash());
    let preview = draft
        .preview_with_rendered_text(revision_number, &exact_text, CREATED_AT)
        .expect("the complete rendered preview is valid");
    let mut input = DraftRevisionInput::new(
        REPOSITORY,
        DRAFT_ID,
        i64::try_from(revision_number).expect("test revision fits i64"),
        revision.content_hash(),
        &exact_text,
        revision.destination_alias().as_str(),
        serde_json::to_string(revision.resolved_destination())
            .expect("resolved destination serializes"),
        STATE_TIMESTAMP,
    );
    input.metadata_json = serde_json::to_string(revision.metadata()).expect("metadata serializes");
    input.expiry_at = Some(revision.expiry().expires_at_unix_seconds().to_string());
    state
        .insert_draft_revision(&input)
        .expect("persist immutable revision");
    preview
}

fn setup(
    body: &str,
    expiry: u64,
    release_channel: &str,
    include_policy: bool,
) -> (ApprovalService, ResolvedConfig, DraftModel, DraftPreview) {
    setup_with_config(config(release_channel, include_policy), body, expiry)
}

fn setup_with_config(
    resolved: ResolvedConfig,
    body: &str,
    expiry: u64,
) -> (ApprovalService, ResolvedConfig, DraftModel, DraftPreview) {
    let mut state = StateStore::open_in_memory().expect("open in-memory state");
    state
        .upsert_repository(&RepositoryInput::new(
            REPOSITORY,
            "100",
            resolved.canonical_hash(),
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
    let draft = DraftModel::create(
        request("release", body, "main", expiry),
        &resolved,
        CREATED_AT,
    )
    .expect("create immutable draft");
    let preview = persist_revision(&mut state, &draft, 1, body);
    (ApprovalService::new(state), resolved, draft, preview)
}

fn confirm_tty(
    service: &ApprovalService,
    draft: &DraftPreview,
    config: &ResolvedConfig,
) -> OperatorConfirmation {
    let preview = service
        .preview_at(draft, config, &FixedClock::new(CREATED_AT + 1).0)
        .expect("build complete preview");
    OperatorConfirmation::confirmed(&preview, TtyMode::Tty).expect("TTY confirmation is available")
}

fn assert_invalid(check: &ApprovalCheck, expected: &ApprovalInvalidReason) {
    assert_eq!(
        check.disposition(),
        &ApprovalDisposition::Invalid(expected.clone()),
        "unexpected approval disposition: {:?}",
        check.disposition()
    );
}

#[test]
fn tty_confirmation_records_exact_binding_and_replay_is_idempotent() {
    let (mut service, resolved, draft, draft_preview) = setup(BODY, 5_000, "201", false);
    let confirmation = confirm_tty(&service, &draft_preview, &resolved);

    let non_tty = OperatorConfirmation::from_preview(
        &service
            .preview_at(&draft_preview, &resolved, &FixedClock::new(1_001).0)
            .expect("non-TTY preview"),
        TtyMode::NonTty,
    );
    assert_eq!(
        service.approve(&draft_preview, &resolved, &non_tty, &FixedClock::new(1_001)),
        Err(ApprovalError::TtyRequired)
    );
    assert!(
        service
            .state()
            .approval(REPOSITORY, DRAFT_ID, 1)
            .expect("approval lookup")
            .is_none()
    );

    let first = service
        .approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_100),
        )
        .expect("TTY approval succeeds");
    assert_eq!(first.repository_id, REPOSITORY);
    assert_eq!(first.draft_id, DRAFT_ID);
    assert_eq!(first.revision, 1);
    assert_eq!(first.revision_hash, draft.current_revision().content_hash());
    assert_eq!(first.config_hash, resolved.canonical_hash());
    assert_eq!(first.destination_hash, {
        service
            .preview_at(&draft_preview, &resolved, &FixedClock::new(1_100).0)
            .expect("preview")
            .destination_hash()
            .to_owned()
    });
    assert_eq!(first.approved_at_unix_seconds, 1_100);
    assert_eq!(
        first.expires_at_unix_seconds,
        1_100 + APPROVAL_LIFETIME_SECONDS
    );

    let preview = service
        .preview_at(&draft_preview, &resolved, &FixedClock::new(1_101).0)
        .expect("preview includes complete text and metadata");
    assert_eq!(
        preview.exact_text(),
        final_text(BODY, draft.current_revision().content_hash())
    );
    assert_eq!(preview.metadata().branch(), Some("main"));
    assert_eq!(preview.resolved_destination().channel_id, "201");

    let check = service
        .check_current(&draft_preview, &resolved, &FixedClock::new(1_101))
        .expect("current approval check");
    assert!(check.is_valid(), "{:?}", check.disposition());
    assert_eq!(check.approval(), Some(&first));
    assert!(check.revalidation().approval_hash.is_some());

    let replay = service
        .approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_200),
        )
        .expect("exact confirmation replay");
    assert_eq!(replay, first, "replay must not extend or replace approval");
    let audit = service
        .state()
        .audit_event(REPOSITORY, &first.audit_event_id)
        .expect("audit lookup")
        .expect("approval audit event");
    assert!(audit.metadata_json.contains(&first.revision_hash));
    assert!(audit.metadata_json.contains(&first.config_hash));
    assert!(!audit.metadata_json.contains(BODY));
    assert!(!audit.metadata_json.contains("main"));
}

#[test]
fn equivalent_canonical_configuration_order_keeps_the_preview_binding_stable() {
    let first_config = config_with_auto_send(
        "201",
        vec![
            AutoSendEntry {
                event_type: "build_failed".to_owned(),
                destination: "release".to_owned(),
                severity: "high".to_owned(),
            },
            AutoSendEntry {
                event_type: "build_passed".to_owned(),
                destination: "release".to_owned(),
                severity: "low".to_owned(),
            },
        ],
    );
    let (mut service, _resolved, _draft, draft_preview) =
        setup_with_config(first_config.clone(), BODY, 5_000);
    let mut reordered_model = first_config.config.clone();
    reordered_model.auto_send.reverse();
    let reordered = resolve_model(
        &reordered_model,
        Path::new("/approval-contract/.repo-com.toml"),
    )
    .expect("equivalent policy order resolves");

    let first = service
        .preview_at(&draft_preview, &first_config, &FixedClock::new(1_000).0)
        .expect("first canonical preview");
    let second = service
        .preview_at(&draft_preview, &reordered, &FixedClock::new(1_000).0)
        .expect("reordered canonical preview");
    assert_eq!(
        first, second,
        "equivalent canonical data must hash identically"
    );
    let confirmation = OperatorConfirmation::confirmed(&first, TtyMode::Tty)
        .expect("canonical preview confirmation");
    service
        .approve(
            &draft_preview,
            &reordered,
            &confirmation,
            &FixedClock::new(1_000),
        )
        .expect("canonical ordering does not invalidate confirmation");
}

#[test]
fn injected_clock_enforces_approval_and_earlier_draft_expiry_boundaries() {
    let (mut service, resolved, _draft, draft_preview) = setup(BODY, 5_000, "201", false);
    let confirmation = confirm_tty(&service, &draft_preview, &resolved);
    let approval = service
        .approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_000),
        )
        .expect("approve");
    assert_eq!(
        approval.expires_at_unix_seconds,
        1_000 + APPROVAL_LIFETIME_SECONDS
    );
    assert!(
        service
            .check_current(&draft_preview, &resolved, &FixedClock::new(1_899))
            .expect("before approval boundary")
            .is_valid()
    );
    let at_boundary = service
        .check_current(&draft_preview, &resolved, &FixedClock::new(1_900))
        .expect("at approval boundary");
    assert_invalid(&at_boundary, &ApprovalInvalidReason::ApprovalExpired);

    let (mut service, resolved, _draft, draft_preview) = setup(BODY, 500, "201", false);
    let confirmation = confirm_tty(&service, &draft_preview, &resolved);
    let earlier = service
        .approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_000),
        )
        .expect("approve with earlier draft expiry");
    assert_eq!(earlier.expires_at_unix_seconds, 1_500);
    assert!(
        service
            .check_current(&draft_preview, &resolved, &FixedClock::new(1_499))
            .expect("before draft boundary")
            .is_valid()
    );
    let draft_boundary = service
        .check_current(&draft_preview, &resolved, &FixedClock::new(1_500))
        .expect("at earlier draft boundary");
    assert_invalid(
        &draft_boundary,
        &ApprovalInvalidReason::PreviewInvalid(PreviewInvalidReason::DraftExpired),
    );
}

#[test]
fn content_metadata_alias_destination_config_and_expiry_changes_invalidate() {
    let (mut service, resolved, mut draft, draft_preview) = setup(BODY, 5_000, "201", false);
    let confirmation = confirm_tty(&service, &draft_preview, &resolved);
    service
        .approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_000),
        )
        .expect("baseline approval");

    let forged_exact_text = draft
        .preview_with_rendered_text(1, "forged rendered text", CREATED_AT)
        .expect("caller preview remains structurally valid");
    assert_invalid(
        &service
            .check_current(&forged_exact_text, &resolved, &FixedClock::new(1_001))
            .expect("forged exact-text check"),
        &ApprovalInvalidReason::PreviewInvalid(PreviewInvalidReason::ExactTextChanged),
    );

    let body_revision = draft
        .revise(
            request("release", "different body", "main", 5_000),
            &resolved,
            CREATED_AT,
        )
        .expect("body revision")
        .number();
    let body_preview =
        persist_revision(service.state_mut(), &draft, body_revision, "different body");
    assert_invalid(
        &service
            .check_current(&body_preview, &resolved, &FixedClock::new(1_001))
            .expect("body check"),
        &ApprovalInvalidReason::ApprovalMissing,
    );

    let metadata_revision = draft
        .revise(
            request("release", BODY, "release/1.0", 5_000),
            &resolved,
            CREATED_AT,
        )
        .expect("metadata revision")
        .number();
    let metadata_preview = persist_revision(service.state_mut(), &draft, metadata_revision, BODY);
    assert_invalid(
        &service
            .check_current(&metadata_preview, &resolved, &FixedClock::new(1_001))
            .expect("metadata check"),
        &ApprovalInvalidReason::ApprovalMissing,
    );

    let alias_revision = draft
        .revise(
            request("staging", BODY, "main", 5_000),
            &resolved,
            CREATED_AT,
        )
        .expect("alias revision")
        .number();
    let alias_preview = persist_revision(service.state_mut(), &draft, alias_revision, BODY);
    assert_invalid(
        &service
            .check_current(&alias_preview, &resolved, &FixedClock::new(1_001))
            .expect("alias check"),
        &ApprovalInvalidReason::PreviewInvalid(PreviewInvalidReason::DestinationChanged),
    );

    let changed_destination = config("203", false);
    let destination_revision = draft
        .revise(
            request("release", BODY, "main", 5_000),
            &changed_destination,
            CREATED_AT,
        )
        .expect("destination revision")
        .number();
    let destination_preview =
        persist_revision(service.state_mut(), &draft, destination_revision, BODY);
    assert_invalid(
        &service
            .check_current(
                &destination_preview,
                &changed_destination,
                &FixedClock::new(1_001),
            )
            .expect("destination check"),
        &ApprovalInvalidReason::ApprovalMissing,
    );

    let expiry_revision = draft
        .revise(
            request("release", BODY, "main", 600),
            &changed_destination,
            CREATED_AT,
        )
        .expect("expiry revision")
        .number();
    let expiry_preview = persist_revision(service.state_mut(), &draft, expiry_revision, BODY);
    assert_invalid(
        &service
            .check_current(
                &expiry_preview,
                &changed_destination,
                &FixedClock::new(1_001),
            )
            .expect("expiry check"),
        &ApprovalInvalidReason::ApprovalMissing,
    );

    let (mut config_service, config_preview) = {
        let (mut service, resolved, _draft, preview) = setup(BODY, 5_000, "201", false);
        let confirmation = confirm_tty(&service, &preview, &resolved);
        service
            .approve(&preview, &resolved, &confirmation, &FixedClock::new(1_000))
            .expect("config baseline approval");
        (service, preview)
    };
    let mut changed_model = config("201", false);
    changed_model.config.retention.content_days += 1;
    let changed_config = resolve_model(
        &changed_model.config,
        Path::new("/approval-contract/.repo-com.toml"),
    )
    .expect("changed retention resolves");
    let config_check = config_service
        .check_current(&config_preview, &changed_config, &FixedClock::new(1_001))
        .expect("config check");
    assert_invalid(
        &config_check,
        &ApprovalInvalidReason::BindingChanged(ApprovalBindingField::Config),
    );

    let refreshed_confirmation = confirm_tty(&config_service, &config_preview, &changed_config);
    let rebound = config_service
        .approve(
            &config_preview,
            &changed_config,
            &refreshed_confirmation,
            &FixedClock::new(1_100),
        )
        .expect("current state may receive a fresh exact approval");
    assert_eq!(rebound.config_hash, changed_config.canonical_hash());
    assert!(
        config_service
            .check_current(&config_preview, &changed_config, &FixedClock::new(1_101))
            .expect("rebound approval check")
            .is_valid()
    );
}

#[test]
fn policy_basis_change_invalidates_an_otherwise_exact_approval() {
    let (mut service, resolved, _draft, draft_preview) = setup(BODY, 5_000, "201", true);
    let confirmation = confirm_tty(&service, &draft_preview, &resolved);
    service
        .approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_000),
        )
        .expect("human approval with configured but inactive policy");

    PolicyRegistry::new(service.state_mut())
        .activate(
            &resolved.config,
            &PolicyTuple::new("build_failed", "release", "high"),
            PolicyOperatorConfirmation::confirmed(TtyMode::Tty).expect("policy TTY confirmation"),
            STATE_TIMESTAMP,
        )
        .expect("activate exact policy");

    let changed = service
        .check_current(&draft_preview, &resolved, &FixedClock::new(1_001))
        .expect("policy basis check");
    assert_invalid(
        &changed,
        &ApprovalInvalidReason::BindingChanged(ApprovalBindingField::PolicyBasis),
    );
}

#[test]
fn secret_override_requires_tty_exact_preview_and_redacts_the_matched_value() {
    let (mut service, resolved, mut draft, draft_preview) =
        setup(FINDING_TEXT, 5_000, "201", false);
    let finding_preview = service
        .preview_at(&draft_preview, &resolved, &FixedClock::new(1_000).0)
        .expect("finding preview");
    assert!(finding_preview.scan_result().is_blocked());
    let confirmation = OperatorConfirmation::confirmed(&finding_preview, TtyMode::Tty)
        .expect("finding confirmation");
    let non_tty = OperatorConfirmation::from_preview(&finding_preview, TtyMode::NonTty);
    assert_eq!(
        service.override_secret_finding(
            &draft_preview,
            &resolved,
            OverrideReasonCode::ReviewedFalsePositive,
            &non_tty,
            &FixedClock::new(1_000),
        ),
        Err(ApprovalError::TtyRequired)
    );
    let override_event_id = format!("secret-override-{}", finding_preview.preview_hash());
    assert!(
        service
            .state()
            .audit_event(REPOSITORY, &override_event_id)
            .expect("override audit lookup")
            .is_none()
    );

    let first = service
        .override_secret_finding(
            &draft_preview,
            &resolved,
            OverrideReasonCode::ReviewedFalsePositive,
            &confirmation,
            &FixedClock::new(1_000),
        )
        .expect("TTY override");
    assert_eq!(first.reason_code, OverrideReasonCode::ReviewedFalsePositive);
    assert_eq!(first.revision, 1);
    assert_eq!(first.scan_hash, finding_preview.scan_hash());
    let audit = service
        .state()
        .audit_event(REPOSITORY, &first.event_id)
        .expect("override audit lookup")
        .expect("override audit event");
    assert!(audit.metadata_json.contains("reviewed-false-positive"));
    assert!(audit.metadata_json.contains(finding_preview.scan_hash()));
    assert!(!audit.metadata_json.contains("synthetic-not-a-real-token"));
    assert!(!audit.metadata_json.contains("Authorization"));

    let replay = service
        .override_secret_finding(
            &draft_preview,
            &resolved,
            OverrideReasonCode::ReviewedFalsePositive,
            &confirmation,
            &FixedClock::new(1_100),
        )
        .expect("override replay");
    assert_eq!(
        replay, first,
        "override replay must not create new authority"
    );
    let replay_audit = service
        .state()
        .audit_event(REPOSITORY, &first.event_id)
        .expect("replayed override audit lookup")
        .expect("replayed override audit event");
    assert_eq!(replay_audit.audit_id, audit.audit_id);

    let approval_confirmation = OperatorConfirmation::confirmed(&finding_preview, TtyMode::Tty)
        .expect("approval confirmation after override");
    service
        .approve(
            &draft_preview,
            &resolved,
            &approval_confirmation,
            &FixedClock::new(1_100),
        )
        .expect("approval after exact override");
    assert!(
        service
            .check_current(&draft_preview, &resolved, &FixedClock::new(1_101))
            .expect("finding approval check")
            .is_valid()
    );

    let next_revision = draft
        .revise(
            request("release", "clean replacement", "main", 5_000),
            &resolved,
            CREATED_AT,
        )
        .expect("replacement revision")
        .number();
    let next_preview = persist_revision(
        service.state_mut(),
        &draft,
        next_revision,
        "clean replacement",
    );
    assert_eq!(
        service.override_secret_finding(
            &next_preview,
            &resolved,
            OverrideReasonCode::ReviewedFalsePositive,
            &confirmation,
            &FixedClock::new(1_200),
        ),
        Err(ApprovalError::ConfirmationMismatch)
    );
}

#[test]
fn approval_requires_override_when_scan_is_blocked() {
    let (mut service, resolved, _draft, draft_preview) = setup(FINDING_TEXT, 5_000, "201", false);
    let preview = service
        .preview_at(&draft_preview, &resolved, &FixedClock::new(1_000).0)
        .expect("finding preview");
    let confirmation =
        OperatorConfirmation::confirmed(&preview, TtyMode::Tty).expect("finding confirmation");
    assert_eq!(
        service.approve(
            &draft_preview,
            &resolved,
            &confirmation,
            &FixedClock::new(1_000),
        ),
        Err(ApprovalError::SecretOverrideRequired)
    );
    assert!(
        service
            .state()
            .approval(REPOSITORY, DRAFT_ID, 1)
            .expect("approval lookup")
            .is_none()
    );
}

#[test]
fn public_approval_surface_has_no_outbound_edit_or_delete_operation() {
    let sources = [
        include_str!("../src/lib.rs"),
        include_str!("../src/service.rs"),
        include_str!("../src/override.rs"),
    ]
    .concat();
    for forbidden in [
        "pub fn edit",
        "pub fn delete",
        "pub async fn edit",
        "pub async fn delete",
        "pub fn edit_message",
        "pub fn delete_message",
    ] {
        assert!(
            !sources.contains(forbidden),
            "approval API unexpectedly exposes {forbidden}"
        );
    }
}
