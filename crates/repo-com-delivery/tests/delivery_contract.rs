use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use serde::Serialize;

use repo_com_approval::{
    ApprovalClock, ApprovalInstant, ApprovalPreview, ApprovalRecord, ApprovalService,
    OperatorConfirmation as ApprovalConfirmation, OverrideReasonCode,
};
use repo_com_config::{
    AutoSendEntry, DestinationConfig, DiscordConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};
use repo_com_draft_content::{ContentRenderer, RenderedMessage};
use repo_com_draft_model::{DraftModel, DraftRequest};
use repo_com_draft_safety::SecretScanner;
use repo_com_foundation::TtyMode;
use repo_com_policy::{PolicyRegistry, PolicyTuple};
use repo_com_send_eligibility::{
    EligibilityAuthority, EligibilityDecision, OutboundCorrection, RevalidationFacts,
};
use repo_com_state::{
    AuditEventInput, DraftInput, DraftRevisionInput, RepositoryInput, StateStore, database_path_in,
};

use crate::{
    ClaimDisposition, ClaimRequest, DeliveryCoordinator, DeliveryState, TransitionRequest,
};

const REPOSITORY: &str = "acme/delivery";
const DRAFT_ID: &str = "draft-delivery";
const CREATED_AT: u64 = 1_000;
const NOW: u64 = 1_100;
const EXPIRES_AT: u64 = 5_000;
const STATE_TIMESTAMP: &str = "2026-01-01T00:00:00Z";
const BODY: &str = "build failed on main";
const FINDING_BODY: &str =
    "deploy failed\nAuthorization: Bearer synthetic-not-a-real-token-1234567890";

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let sequence = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "repo-com-delivery-contract-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create delivery test directory");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Scenario {
    _temp: TempDir,
    state: StateStore,
    config: ResolvedConfig,
    rendered: RenderedMessage,
    tuple: PolicyTuple,
    decision: EligibilityDecision,
    approval_preview: ApprovalPreview,
    approval: Option<ApprovalRecord>,
}

impl Scenario {
    fn request(&self, now: u64, timestamp: &str) -> ClaimRequest<'_> {
        ClaimRequest::new(
            &self.decision,
            &self.config,
            &self.rendered,
            self.tuple.clone(),
            now,
            timestamp,
        )
    }
}

struct FixedClock(ApprovalInstant);

impl ApprovalClock for FixedClock {
    fn now(&self) -> ApprovalInstant {
        self.0.clone()
    }
}

fn config(policy: bool) -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: "201".to_owned(),
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
            auto_send: if policy {
                vec![AutoSendEntry {
                    event_type: "build_failed".to_owned(),
                    destination: "release".to_owned(),
                    severity: "high".to_owned(),
                }]
            } else {
                Vec::new()
            },
        },
        Path::new("/repo-com-delivery-contract/.repo-com.toml"),
    )
    .expect("synthetic delivery config is valid")
}

fn tuple() -> PolicyTuple {
    PolicyTuple::new("build_failed", "release", "high")
}

fn instant(seconds: u64) -> ApprovalInstant {
    ApprovalInstant::new(seconds, STATE_TIMESTAMP).expect("synthetic approval instant")
}

fn make_scenario(policy: bool, approve: bool) -> Scenario {
    make_scenario_with_body(policy, approve, BODY)
}

fn make_scenario_with_body(policy: bool, approve: bool, body: &str) -> Scenario {
    let temp = TempDir::new();
    let path = database_path_in(&temp.path);
    let mut state = StateStore::open_path(&path).expect("open delivery state");
    let config = config(policy);
    let tuple = tuple();

    state
        .upsert_repository(&RepositoryInput::new(
            REPOSITORY,
            "100",
            config.canonical_hash(),
            STATE_TIMESTAMP,
        ))
        .expect("register delivery repository");
    state
        .create_draft(&DraftInput::new(
            REPOSITORY,
            DRAFT_ID,
            "build_failed",
            "release",
            STATE_TIMESTAMP,
        ))
        .expect("create delivery draft");

    let request = DraftRequest::new(DRAFT_ID, "release", body, "build_failed", "high")
        .expect("valid draft request")
        .with_expiry_seconds(EXPIRES_AT - CREATED_AT);
    let draft = DraftModel::create(request, &config, CREATED_AT).expect("create draft model");
    let revision = draft.current_revision();
    let rendered = ContentRenderer::new()
        .render_revision(revision, CREATED_AT)
        .expect("render delivery revision");
    let draft_preview = draft
        .preview_with_rendered_text(1, rendered.exact_text(), CREATED_AT)
        .expect("create delivery preview");

    let mut revision_input = DraftRevisionInput::new(
        REPOSITORY,
        DRAFT_ID,
        1,
        revision.content_hash(),
        rendered.exact_text(),
        revision.destination_alias().as_str(),
        serde_json::to_string(revision.resolved_destination()).expect("serialize destination"),
        STATE_TIMESTAMP,
    );
    revision_input.metadata_json =
        serde_json::to_string(revision.metadata()).expect("serialize metadata");
    revision_input.expiry_at = Some(EXPIRES_AT.to_string());
    state
        .insert_draft_revision(&revision_input)
        .expect("persist delivery revision");

    if policy {
        let mut registry = PolicyRegistry::new(&mut state);
        registry
            .activate(
                &config.config,
                &tuple,
                repo_com_policy::OperatorConfirmation::confirmed(TtyMode::Tty)
                    .expect("synthetic policy confirmation"),
                STATE_TIMESTAMP,
            )
            .expect("activate exact policy");
    }

    let mut service = ApprovalService::new(state);
    let approval_preview = service
        .preview_at(&draft_preview, &config, &instant(CREATED_AT))
        .expect("build current approval preview");
    let approval = if approve {
        let confirmation = ApprovalConfirmation::confirmed(&approval_preview, TtyMode::Tty)
            .expect("synthetic approval confirmation");
        if approval_preview.scan_result().is_blocked() {
            service
                .override_secret_finding(
                    &draft_preview,
                    &config,
                    OverrideReasonCode::ReviewedFalsePositive,
                    &confirmation,
                    &FixedClock(instant(CREATED_AT)),
                )
                .expect("record exact secret override");
        }
        Some(
            service
                .approve(
                    &draft_preview,
                    &config,
                    &confirmation,
                    &FixedClock(instant(CREATED_AT)),
                )
                .expect("record exact approval"),
        )
    } else {
        None
    };
    let state = service.into_state();

    let policy_decision = repo_com_policy::evaluate_from_state(&state, &config.config, &tuple)
        .expect("evaluate policy basis");
    let scan = SecretScanner::new().scan_rendered(&rendered);
    let destination_hash = sha256_json(&DestinationBinding {
        current: Some(rendered.resolved_destination()),
        revision: rendered.resolved_destination(),
    })
    .expect("hash destination");
    let authority = if let Some(approval) = &approval {
        EligibilityAuthority::HumanApproval {
            approval_id: approval.approval_id.clone(),
            approval_hash: approval.hash().expect("hash approval"),
            expires_at_unix_seconds: approval.expires_at_unix_seconds,
        }
    } else {
        let snapshot = policy_decision
            .snapshot()
            .expect("active policy snapshot")
            .clone();
        let activation_hash = sha256_json(&snapshot).expect("hash policy snapshot");
        EligibilityAuthority::ActivatedPolicy {
            activation_id: snapshot.activation_id,
            activation_hash,
            config_hash: snapshot.current_config_hash,
            tuple_hash: snapshot.current_tuple_hash,
            activated_at: snapshot.activated_at,
        }
    };
    let revalidation = RevalidationFacts {
        repository_id: REPOSITORY.to_owned(),
        repository_hash: sha256_json(&RepositoryBinding {
            repository_id: REPOSITORY,
            workspace_id: "100",
        })
        .expect("hash repository"),
        workspace_id: "100".to_owned(),
        draft_id: DRAFT_ID.to_owned(),
        revision: 1,
        revision_hash: revision.content_hash().to_owned(),
        destination_alias: "release".to_owned(),
        destination_hash,
        exact_text_hash: sha256_bytes(rendered.exact_text().as_bytes()),
        metadata_hash: sha256_json(rendered.metadata()).expect("hash metadata"),
        config_hash: config.canonical_hash(),
        policy_basis_hash: sha256_json(&policy_decision).expect("hash policy basis"),
        scan_hash: sha256_json(&scan).expect("hash scan"),
        approval_preview_hash: approval_preview.preview_hash().to_owned(),
        draft_created_at_unix_seconds: CREATED_AT,
        draft_expires_at_unix_seconds: EXPIRES_AT,
        evaluated_at_unix_seconds: NOW,
    };
    let decision = EligibilityDecision::Eligible {
        revalidation,
        authority,
        correction: OutboundCorrection::NewDraft,
    };

    Scenario {
        _temp: temp,
        state,
        config,
        rendered,
        tuple,
        decision,
        approval_preview,
        approval,
    }
}

fn count_rows(state: &StateStore, table: &str) -> i64 {
    state
        .connection()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count delivery rows")
}

#[test]
fn claim_commits_before_single_permit_and_replays_existing_outcome() {
    let scenario = make_scenario(true, false);
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let first = DeliveryCoordinator::new(
        scenario
            .state
            .reopen()
            .expect("reopen state for first claim"),
    )
    .claim(&request)
    .expect("first claim must commit");
    assert_eq!(first.disposition, ClaimDisposition::NewlyClaimed);
    assert!(first.network_authorized());
    assert_eq!(first.attempt.state, DeliveryState::Claimed);
    assert_eq!(first.attempt.attempt_number, 1);
    assert_eq!(first.attempt.request_nonce, scenario.rendered.nonce());

    let mut coordinator = DeliveryCoordinator::new(
        scenario
            .state
            .reopen()
            .expect("reopen state for duplicate caller"),
    );
    let duplicate = coordinator
        .claim(&scenario.request(NOW, STATE_TIMESTAMP))
        .expect("duplicate claim returns recorded outcome");
    assert_eq!(duplicate.disposition, ClaimDisposition::Existing);
    assert!(!duplicate.network_authorized());
    assert_eq!(duplicate.attempt.attempt_id, first.attempt.attempt_id);
    assert_eq!(duplicate.attempt.state, DeliveryState::Claimed);
    assert_eq!(count_rows(coordinator.state(), "delivery_attempts"), 1);
    let claim_metadata: String = coordinator
        .state()
        .connection()
        .query_row(
            "SELECT metadata_json FROM audit_events WHERE object_type = 'delivery_attempt'",
            [],
            |row| row.get(0),
        )
        .expect("read redacted claim metadata");
    assert!(!claim_metadata.contains(BODY));
}

#[test]
fn stale_config_revision_expiry_destination_approval_activation_and_scan_roll_back() {
    let cases = 7;
    for case in 0..cases {
        let approve = case == 3;
        let mut scenario = make_scenario(true, approve);
        let before_attempts = count_rows(&scenario.state, "delivery_attempts");
        let before_audits = count_rows(&scenario.state, "audit_events");

        match case {
            0 => {
                scenario.config.config.retention.content_days += 1;
            }
            1 => {
                if let EligibilityDecision::Eligible { revalidation, .. } = &mut scenario.decision {
                    revalidation.revision_hash = "0".repeat(64);
                }
            }
            2 => {}
            3 => {
                let approval = scenario.approval.as_ref().expect("approval case");
                let event_id = format!(
                    "approval-recorded-{}",
                    approval
                        .approval_id
                        .strip_prefix("approval-")
                        .expect("approval ID prefix")
                );
                scenario
                    .state
                    .connection()
                    .execute(
                        "UPDATE approvals SET approval_state = 'revoked', revoked_at = ?3
                         WHERE repository_id = ?1 AND draft_id = ?2",
                        rusqlite::params![REPOSITORY, DRAFT_ID, STATE_TIMESTAMP],
                    )
                    .expect("revoke approval fixture");
                assert!(!event_id.is_empty());
            }
            4 => {
                scenario
                    .config
                    .destinations
                    .get_mut("release")
                    .expect("destination")
                    .channel_id = "202".to_owned();
            }
            5 => {
                let activation =
                    repo_com_policy::activation_records_from_state(&scenario.state, REPOSITORY)
                        .expect("read activations")
                        .into_iter()
                        .next()
                        .expect("policy activation");
                PolicyRegistry::new(&mut scenario.state)
                    .deactivate(REPOSITORY, &activation.activation_id, STATE_TIMESTAMP)
                    .expect("deactivate policy fixture");
            }
            6 => {
                if let EligibilityDecision::Eligible { revalidation, .. } = &mut scenario.decision {
                    revalidation.scan_hash = "1".repeat(64);
                }
            }
            _ => unreachable!(),
        }

        let request = if case == 2 {
            scenario.request(EXPIRES_AT, STATE_TIMESTAMP)
        } else {
            scenario.request(NOW, STATE_TIMESTAMP)
        };
        let mut coordinator = DeliveryCoordinator::new(
            scenario
                .state
                .reopen()
                .expect("reopen state for stale case"),
        );
        assert!(
            coordinator.claim(&request).is_err(),
            "stale case {case} unexpectedly claimed"
        );
        assert_eq!(
            count_rows(coordinator.state(), "delivery_attempts"),
            before_attempts
        );
        assert_eq!(
            count_rows(coordinator.state(), "audit_events"),
            before_audits
        );
        let _ = &mut coordinator;
    }
}

#[test]
fn one_hundred_concurrent_callers_commit_one_claim_and_one_network_authorization() {
    let scenario = make_scenario(true, false);
    let path = scenario.state.path().expect("file-backed state").to_owned();
    let config = scenario.config.clone();
    let rendered = scenario.rendered.clone();
    let decision = scenario.decision.clone();
    let tuple = scenario.tuple.clone();
    drop(scenario.state);
    let barrier = Arc::new(Barrier::new(100));

    let results = thread::scope(|scope| {
        let handles = (0..100)
            .map(|_| {
                let path = path.clone();
                let config = config.clone();
                let rendered = rendered.clone();
                let decision = decision.clone();
                let tuple = tuple.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    let state = StateStore::open_path(&path).expect("open concurrent state");
                    let mut coordinator = DeliveryCoordinator::new(state);
                    let request = ClaimRequest::new(
                        &decision,
                        &config,
                        &rendered,
                        tuple,
                        NOW,
                        STATE_TIMESTAMP,
                    );
                    barrier.wait();
                    coordinator.claim(&request)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("concurrent claim thread"))
            .collect::<Vec<_>>()
    });

    assert!(
        results.iter().all(Result::is_ok),
        "concurrent claims failed"
    );
    let claims = results.into_iter().map(Result::unwrap).collect::<Vec<_>>();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.disposition == ClaimDisposition::NewlyClaimed)
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.network_authorized())
            .count(),
        1
    );
    let attempt_ids = claims
        .iter()
        .map(|claim| claim.attempt.attempt_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(attempt_ids.len(), 1);
    let verification = StateStore::open_path(&path).expect("verification state");
    assert_eq!(count_rows(&verification, "delivery_attempts"), 1);
    assert_eq!(count_rows(&verification, "audit_events"), 1);
}

#[test]
fn transitions_record_metadata_and_audit_and_retry_wait_is_not_automatic() {
    let scenario = make_scenario(true, false);
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let mut coordinator =
        DeliveryCoordinator::new(scenario.state.reopen().expect("reopen transition state"));
    let claim = coordinator
        .claim(&request)
        .expect("claim transition fixture");
    let accepted = TransitionRequest::new(
        REPOSITORY,
        DRAFT_ID,
        1,
        &claim.attempt.attempt_id,
        DeliveryState::Accepted,
        "2026-01-01T00:00:01Z",
        "system",
    )
    .with_remote_message_id("123456789012345678");
    let accepted = coordinator
        .record_accepted(accepted)
        .expect("record accepted");
    assert_eq!(accepted.state, DeliveryState::Accepted);
    assert_eq!(
        accepted.remote_message_id.as_deref(),
        Some("123456789012345678")
    );
    assert!(accepted.completed_at.is_some());

    let failed_scenario = make_scenario(true, false);
    let failed_request = failed_scenario.request(NOW, STATE_TIMESTAMP);
    let mut failed_coordinator =
        DeliveryCoordinator::new(failed_scenario.state.reopen().expect("reopen failed state"));
    let failed_claim = failed_coordinator
        .claim(&failed_request)
        .expect("claim failed fixture");
    let failed = failed_coordinator
        .record_definitive_failed(
            TransitionRequest::new(
                REPOSITORY,
                DRAFT_ID,
                1,
                &failed_claim.attempt.attempt_id,
                DeliveryState::Failed,
                "2026-01-01T00:00:01Z",
                "system",
            )
            .with_error_code("permission-denied"),
        )
        .expect("record definitive failure");
    assert_eq!(failed.state, DeliveryState::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("permission-denied"));

    let retry_scenario = make_scenario(true, false);
    let retry_request = retry_scenario.request(NOW, STATE_TIMESTAMP);
    let mut retry_coordinator =
        DeliveryCoordinator::new(retry_scenario.state.reopen().expect("reopen retry state"));
    let retry_claim = retry_coordinator
        .claim(&retry_request)
        .expect("claim retry fixture");
    let retry = retry_coordinator
        .record_retry_wait(
            TransitionRequest::new(
                REPOSITORY,
                DRAFT_ID,
                1,
                &retry_claim.attempt.attempt_id,
                DeliveryState::RetryWait,
                "2026-01-01T00:00:01Z",
                "system",
            )
            .with_error_code("connect-failed"),
        )
        .expect("record retry wait");
    assert_eq!(retry.state, DeliveryState::RetryWait);
    let ordinary = retry_coordinator
        .claim(&retry_scenario.request(NOW, STATE_TIMESTAMP))
        .expect("ordinary duplicate observes retry wait");
    assert!(ordinary.is_existing());
    assert!(!ordinary.network_authorized());
    let next = retry_coordinator
        .claim_retry(&retry_scenario.request(NOW, STATE_TIMESTAMP))
        .expect("explicit retry claim");
    assert_eq!(next.disposition, ClaimDisposition::NewlyClaimed);
    assert_eq!(next.attempt.attempt_number, 2);
    assert!(next.network_authorized());

    let unknown_scenario = make_scenario(true, false);
    let unknown_request = unknown_scenario.request(NOW, STATE_TIMESTAMP);
    let mut unknown_coordinator = DeliveryCoordinator::new(
        unknown_scenario
            .state
            .reopen()
            .expect("reopen unknown state"),
    );
    let unknown_claim = unknown_coordinator
        .claim(&unknown_request)
        .expect("claim unknown fixture");
    let unknown = unknown_coordinator
        .record_unknown(
            TransitionRequest::new(
                REPOSITORY,
                DRAFT_ID,
                1,
                &unknown_claim.attempt.attempt_id,
                DeliveryState::Unknown,
                "2026-01-01T00:00:01Z",
                "system",
            )
            .with_error_code("post-dispatch-timeout"),
        )
        .expect("record unknown");
    assert_eq!(unknown.state, DeliveryState::Unknown);
    assert!(unknown.blocks_automatic_retry());
}

#[test]
fn local_audit_failure_rolls_back_attempt_and_transition_state() {
    let mut scenario = make_scenario(true, false);
    let attempt_id = "delivery-draft-delivery-r1";
    let claim_event_id = format!(
        "delivery-claim-{}",
        sha256_bytes(format!("claimed|{REPOSITORY}|{DRAFT_ID}|1|1").as_bytes())
    );
    scenario
        .state
        .append_audit_event(&AuditEventInput::new(
            REPOSITORY,
            claim_event_id,
            "delivery_attempt",
            attempt_id,
            "claimed",
            STATE_TIMESTAMP,
            "system",
            "claimed",
        ))
        .expect("seed conflicting claim audit");
    let audits_before = count_rows(&scenario.state, "audit_events");
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let mut coordinator =
        DeliveryCoordinator::new(scenario.state.reopen().expect("reopen rollback state"));
    assert!(coordinator.claim(&request).is_err());
    assert_eq!(count_rows(coordinator.state(), "delivery_attempts"), 0);
    assert_eq!(
        count_rows(coordinator.state(), "audit_events"),
        audits_before
    );

    let mut clean = make_scenario(true, false);
    let mut clean_coordinator = DeliveryCoordinator::new(
        clean
            .state
            .reopen()
            .expect("reopen transition rollback state"),
    );
    let claim = {
        let clean_request = clean.request(NOW, STATE_TIMESTAMP);
        clean_coordinator
            .claim(&clean_request)
            .expect("claim transition rollback fixture")
    };
    let transition_event_id = format!(
        "delivery-accepted-{}",
        sha256_bytes(
            format!(
                "accepted|{REPOSITORY}|{DRAFT_ID}|{}",
                claim.attempt.attempt_id
            )
            .as_bytes()
        )
    );
    clean
        .state
        .append_audit_event(&AuditEventInput::new(
            REPOSITORY,
            transition_event_id,
            "delivery_attempt",
            &claim.attempt.attempt_id,
            "accepted",
            "2026-01-01T00:00:01Z",
            "system",
            "accepted",
        ))
        .expect("seed conflicting transition audit");
    let transition_audits_before = count_rows(&clean.state, "audit_events");
    let accepted = TransitionRequest::new(
        REPOSITORY,
        DRAFT_ID,
        1,
        &claim.attempt.attempt_id,
        DeliveryState::Accepted,
        "2026-01-01T00:00:01Z",
        "system",
    )
    .with_remote_message_id("123456789012345678");
    assert!(clean_coordinator.record_accepted(accepted).is_err());
    let stored = clean_coordinator
        .state()
        .delivery_attempt(REPOSITORY, &claim.attempt.attempt_id)
        .expect("read rolled-back attempt")
        .expect("attempt remains");
    assert_eq!(stored.state, "claimed");
    assert_eq!(
        count_rows(clean_coordinator.state(), "audit_events"),
        transition_audits_before
    );
}

#[test]
fn crash_after_claim_or_remote_response_never_authorizes_a_second_post() {
    let scenario = make_scenario(true, false);
    let path = scenario.state.path().expect("file-backed state").to_owned();
    let config = scenario.config.clone();
    let rendered = scenario.rendered.clone();
    let decision = scenario.decision.clone();
    let tuple = scenario.tuple.clone();
    drop(scenario.state);

    let first_state = StateStore::open_path(&path).expect("open first process");
    let first = DeliveryCoordinator::new(first_state)
        .claim(&ClaimRequest::new(
            &decision,
            &config,
            &rendered,
            tuple.clone(),
            NOW,
            STATE_TIMESTAMP,
        ))
        .expect("first claim");
    assert!(first.network_authorized());
    let mut simulated_posts = 0usize;
    if first.network_authorized() {
        // The transport owner observed a response, then the process stopped
        // before recording local completion.
        simulated_posts += 1;
    }
    drop(first);

    let mut after_claim =
        DeliveryCoordinator::new(StateStore::open_path(&path).expect("open after claim crash"));
    let replay = after_claim
        .claim(&ClaimRequest::new(
            &decision,
            &config,
            &rendered,
            tuple,
            NOW,
            STATE_TIMESTAMP,
        ))
        .expect("replay after claim crash");
    assert!(replay.is_existing());
    assert!(!replay.network_authorized());
    assert_eq!(simulated_posts, 1);
    assert_eq!(replay.attempt.state, DeliveryState::Claimed);
}

#[test]
fn terminal_state_and_remote_message_uniqueness_reject_unsafe_reclaims() {
    let scenario = make_scenario(true, false);
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let mut coordinator =
        DeliveryCoordinator::new(scenario.state.reopen().expect("reopen terminal state"));
    let claim = coordinator.claim(&request).expect("claim terminal fixture");
    let accepted = TransitionRequest::new(
        REPOSITORY,
        DRAFT_ID,
        1,
        &claim.attempt.attempt_id,
        DeliveryState::Accepted,
        "2026-01-01T00:00:01Z",
        "system",
    )
    .with_remote_message_id("123456789012345678");
    coordinator
        .record_accepted(accepted.clone())
        .expect("record terminal acceptance");
    assert!(
        coordinator.record_accepted(accepted).is_ok(),
        "identical replay is idempotent"
    );
    let reclaim = coordinator
        .claim(&request)
        .expect("terminal duplicate returns outcome");
    assert!(reclaim.is_existing());
    assert!(!reclaim.network_authorized());
    assert_eq!(reclaim.attempt.state, DeliveryState::Accepted);
    assert_eq!(count_rows(coordinator.state(), "delivery_attempts"), 1);
    assert_eq!(count_rows(coordinator.state(), "audit_events"), 2);

    let sources = [
        include_str!("../src/lib.rs"),
        include_str!("../src/model.rs"),
        include_str!("../src/claim.rs"),
        include_str!("../src/transition.rs"),
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
            "unsafe remote mutation API: {forbidden}"
        );
    }
}

#[test]
fn approval_authority_is_revalidated_inside_the_same_claim_transaction() {
    let scenario = make_scenario(false, true);
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let mut coordinator =
        DeliveryCoordinator::new(scenario.state.reopen().expect("reopen approval state"));
    let result = coordinator.claim(&request).expect("exact approval claim");
    assert_eq!(
        result.attempt.authority,
        scenario.decision.authority().expect("authority").clone()
    );
    assert_eq!(result.attempt.state, DeliveryState::Claimed);
    assert_eq!(result.attempt.attempt_number, 1);
    assert_eq!(
        result.attempt.scan_hash,
        sha256_json(&SecretScanner::new().scan_rendered(&scenario.rendered)).expect("hash scan")
    );
    assert_eq!(
        scenario.decision.revalidation().approval_preview_hash,
        scenario.approval_preview.preview_hash()
    );
    assert!(scenario.approval.is_some());
}

#[test]
fn exact_secret_override_is_revalidated_before_an_approval_claim() {
    let scenario = make_scenario_with_body(false, true, FINDING_BODY);
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let mut coordinator =
        DeliveryCoordinator::new(scenario.state.reopen().expect("reopen override state"));
    let result = coordinator
        .claim(&request)
        .expect("exact overridden approval claim");
    assert_eq!(result.attempt.state, DeliveryState::Claimed);
    assert!(result.network_authorized());
}

#[test]
fn policy_authority_cannot_bypass_a_secret_finding() {
    let scenario = make_scenario_with_body(true, false, FINDING_BODY);
    let request = scenario.request(NOW, STATE_TIMESTAMP);
    let mut coordinator =
        DeliveryCoordinator::new(scenario.state.reopen().expect("reopen secret state"));
    let error = coordinator
        .claim(&request)
        .expect_err("a blocked scan must not create a claim");
    assert!(matches!(
        error,
        crate::DeliveryError::NotEligible {
            blocker: repo_com_send_eligibility::EligibilityBlocker::UnresolvedSecretFinding
        }
    ));
    assert_eq!(count_rows(coordinator.state(), "delivery_attempts"), 0);
    let delivery_audits: i64 = coordinator
        .state()
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE object_type = 'delivery_attempt'",
            [],
            |row| row.get(0),
        )
        .expect("count delivery audits");
    assert_eq!(delivery_audits, 0);
}

#[derive(Serialize)]
struct RepositoryBinding<'a> {
    repository_id: &'a str,
    workspace_id: &'a str,
}

#[derive(Serialize)]
struct DestinationBinding<'a> {
    current: Option<&'a repo_com_config::ResolvedDestination>,
    revision: &'a repo_com_config::ResolvedDestination,
}

fn sha256_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(value)?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
