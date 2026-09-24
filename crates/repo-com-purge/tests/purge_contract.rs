use repo_com_foundation::TtyMode;
use repo_com_state::{
    ApprovalInput, AuditEventInput, DeliveryAttemptInput, DraftInput, DraftRevisionInput,
    InboundCurrentSnapshotInput, InboundItemInput, InboundTransitionInput, PolicyActivationInput,
    ReplyLinkInput, RepositoryInput, StateStore,
};
use serde_json::Value;

use crate::{
    FailurePoint, PURGED_CONTENT_MARKER, PurgeConfirmation, PurgeCutoff, PurgeError,
    PurgeExecuteOptions, PurgeExecutor, PurgeFailurePoint, PurgePlan, PurgePlanner, PurgeRequest,
    PurgeScope,
};

const NOW: &str = "2026-01-01T00:00:00Z";
const OLD: &str = "2025-12-01T00:00:00Z";
const NEW: &str = "2025-12-31T00:00:00Z";
const REPOSITORY_A: &str = "repo-a";
const REPOSITORY_B: &str = "repo-b";

fn cutoff() -> PurgeCutoff {
    PurgeCutoff::from_rfc3339(NOW).expect("canonical cutoff")
}

fn register(store: &mut StateStore, repository_id: &str, config_hash: &str) {
    store
        .upsert_repository(&RepositoryInput::new(
            repository_id,
            format!("workspace-{repository_id}"),
            config_hash,
            OLD,
        ))
        .expect("register repository");
}

fn add_draft(
    store: &mut StateStore,
    repository_id: &str,
    draft_id: &str,
    timestamp: &str,
    body: &str,
) {
    store
        .create_draft(&DraftInput::new(
            repository_id,
            draft_id,
            "build_failed",
            "release",
            timestamp,
        ))
        .expect("create draft");
    let mut revision = DraftRevisionInput::new(
        repository_id,
        draft_id,
        1,
        format!("hash-{draft_id}"),
        body,
        "release",
        "channel-release",
        timestamp,
    );
    revision.metadata_json = r#"{"branch":"main","commit":"abc123"}"#.to_owned();
    store
        .insert_draft_revision(&revision)
        .expect("create revision");
}

fn add_inbound(
    store: &mut StateStore,
    repository_id: &str,
    item_id: &str,
    timestamp: &str,
    first_content: &str,
    current_content: &str,
) {
    store
        .store_inbound_item(
            &InboundItemInput::new(
                repository_id,
                item_id,
                "channel-inbound",
                "author-local",
                first_content,
                timestamp,
            ),
            &InboundCurrentSnapshotInput::new(
                repository_id,
                item_id,
                Some(current_content.to_owned()),
                false,
                timestamp,
            ),
        )
        .expect("store inbound");
}

fn add_transition(
    store: &mut StateStore,
    repository_id: &str,
    item_id: &str,
    transition_id: &str,
    timestamp: &str,
    content: &str,
) {
    store
        .record_inbound_transition(
            &InboundTransitionInput::new(
                repository_id,
                transition_id,
                item_id,
                "edited",
                Some(content.to_owned()),
                timestamp,
            ),
            Some(&InboundCurrentSnapshotInput::new(
                repository_id,
                item_id,
                Some(content.to_owned()),
                false,
                timestamp,
            )),
        )
        .expect("record transition");
}

fn add_metadata(store: &mut StateStore, repository_id: &str) {
    add_draft(store, repository_id, "draft-a", OLD, "synthetic draft text");
    add_inbound(
        store,
        repository_id,
        "item-a",
        OLD,
        "synthetic first text",
        "synthetic current text",
    );
    add_transition(
        store,
        repository_id,
        "item-a",
        "transition-a",
        OLD,
        "synthetic transition text",
    );
    store
        .acknowledge_inbound(repository_id, &["item-a"], OLD)
        .expect("acknowledge");
    store
        .archive_inbound(repository_id, &["item-a"], OLD)
        .expect("archive");
    store
        .record_approval(&ApprovalInput::new(
            repository_id,
            "approval-a",
            "draft-a",
            1,
            "operator",
            OLD,
        ))
        .expect("approval");
    store
        .record_delivery_attempt(&DeliveryAttemptInput::new(
            repository_id,
            "attempt-a",
            "draft-a",
            1,
            1,
            "nonce-a",
            OLD,
        ))
        .expect("delivery");
    store
        .activate_policy(&PolicyActivationInput::new(
            repository_id,
            "activation-a",
            "config-a",
            "tuple-a",
            "build_failed",
            "release",
            "high",
            OLD,
        ))
        .expect("policy");
    store
        .advance_inbound_cursor(&repo_com_state::InboundCursorInput::new(
            repository_id,
            "inbound",
            "cursor-a",
            OLD,
        ))
        .expect("cursor");
    store
        .append_audit_event(&AuditEventInput::new(
            repository_id,
            "event-a",
            "test-object",
            "object-a",
            "observed",
            OLD,
            "system",
            "success",
        ))
        .expect("audit");
    store
        .link_inbound_reply(&ReplyLinkInput::new(
            repository_id,
            "item-a",
            "draft-a",
            OLD,
        ))
        .expect("reply link");
}

fn basic_store() -> StateStore {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A, "config-a");
    add_metadata(&mut store, REPOSITORY_A);
    store
}

fn count_rows(store: &StateStore, table: &str, repository_id: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE repository_id = ?1");
    store
        .connection()
        .query_row(&sql, [repository_id], |row| row.get(0))
        .expect("count rows")
}

fn confirmation(plan: &PurgePlan) -> PurgeConfirmation {
    PurgeConfirmation::confirmed(plan, TtyMode::Tty).expect("TTY confirmation")
}

#[test]
fn plans_cover_content_metadata_and_all_with_exact_stable_hashes_without_mutation() {
    let store = basic_store();
    let planner = PurgePlanner::new();
    let content_request = PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff());
    let metadata_request = PurgeRequest::new(REPOSITORY_A, PurgeScope::Metadata, cutoff());
    let all_request = PurgeRequest::new(REPOSITORY_A, PurgeScope::All, cutoff());

    let content = planner
        .plan(&store, &content_request)
        .expect("content plan");
    let content_repeat = planner
        .plan(&store, &content_request)
        .expect("repeat content plan");
    let metadata = planner
        .plan(&store, &metadata_request)
        .expect("metadata plan");
    let all = planner.plan(&store, &all_request).expect("all plan");

    assert_eq!(content.plan_hash, content_repeat.plan_hash);
    assert_eq!(content, content_repeat);
    assert_ne!(content.plan_hash, metadata.plan_hash);
    assert_ne!(content.plan_hash, all.plan_hash);
    assert_eq!(content.counts.content_rows, 5);
    assert_eq!(content.counts.metadata_rows, 0);
    assert_eq!(content.counts.total_rows, 5);
    assert_eq!(metadata.counts.content_rows, 0);
    assert_eq!(metadata.counts.metadata_rows, 9);
    assert_eq!(metadata.counts.table_count("audit_events"), 1);
    assert_eq!(all.counts.content_rows, 9);
    assert_eq!(all.counts.metadata_rows, 8);
    assert_eq!(all.counts.total_rows, 17);

    assert_eq!(
        store
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read draft")
            .expect("draft")
            .body,
        "synthetic draft text"
    );
    assert_eq!(count_rows(&store, "draft_revisions", REPOSITORY_A), 1);
    assert_eq!(count_rows(&store, "audit_events", REPOSITORY_A), 1);
    assert_eq!(content.config_hash, "config-a");
    assert_eq!(content.repository_id, REPOSITORY_A);
    assert_eq!(content.cutoff, cutoff());
    assert_eq!(content.plan_hash.len(), 64);
}

#[test]
fn cutoff_and_configuration_hash_are_bound_to_the_plan() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A, "config-a");
    add_draft(&mut store, REPOSITORY_A, "old", OLD, "old text");
    add_draft(&mut store, REPOSITORY_A, "new", NEW, "new text");

    let old_cutoff = PurgeCutoff::from_rfc3339("2025-12-15T00:00:00Z").expect("old cutoff");
    let plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, old_cutoff.clone()),
        )
        .expect("cutoff plan");
    assert_eq!(plan.counts.content_rows, 1);

    let stale_request = PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, old_cutoff)
        .with_expected_config_hash("stale-config");
    assert_eq!(
        PurgePlanner::new().plan(&store, &stale_request),
        Err(PurgeError::ConfigurationHashMismatch)
    );

    register(&mut store, REPOSITORY_A, "config-b");
    let confirmed = confirmation(&plan);
    let mut executor = PurgeExecutor::new(store);
    assert_eq!(
        executor.execute(&plan, &confirmed, NOW),
        Err(PurgeError::ConfigurationHashMismatch)
    );
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "old", 1)
            .expect("read old")
            .expect("old")
            .body,
        "old text"
    );
}

#[test]
fn confirmation_rejects_non_tty_and_every_exact_binding_mismatch_without_writes() {
    let store = basic_store();
    let plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff()),
        )
        .expect("plan");
    let mut executor = PurgeExecutor::new(store);

    let non_tty = PurgeConfirmation::from_plan(&plan, TtyMode::NonTty);
    assert_eq!(
        executor.execute(&plan, &non_tty, NOW),
        Err(PurgeError::TtyRequired)
    );

    let base = confirmation(&plan);
    let cases = [
        (
            base.clone().with_repository_id(REPOSITORY_B),
            PurgeError::RepositoryMismatch,
        ),
        (
            base.clone().with_scope(PurgeScope::Metadata),
            PurgeError::ScopeMismatch,
        ),
        (
            base.clone().with_cutoff(
                PurgeCutoff::from_rfc3339("2025-01-01T00:00:00Z").expect("mismatch cutoff"),
            ),
            PurgeError::CutoffMismatch,
        ),
        (
            base.clone().with_config_hash("different-config"),
            PurgeError::ConfigurationHashMismatch,
        ),
        (
            base.with_plan_hash("0000000000000000000000000000000000000000000000000000000000000000"),
            PurgeError::PlanHashMismatch,
        ),
    ];
    for (confirmation, expected) in cases {
        assert_eq!(executor.execute(&plan, &confirmation, NOW), Err(expected));
    }
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read draft")
            .expect("draft")
            .body,
        "synthetic draft text"
    );
    assert_eq!(
        count_rows(executor.state(), "audit_events", REPOSITORY_A),
        1
    );
}

#[test]
fn content_execution_is_repository_scoped_and_appends_only_count_evidence() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A, "config-a");
    register(&mut store, REPOSITORY_B, "config-b");
    add_metadata(&mut store, REPOSITORY_A);
    add_metadata(&mut store, REPOSITORY_B);

    let plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff()),
        )
        .expect("content plan");
    let confirmed = confirmation(&plan);
    let mut executor = PurgeExecutor::new(store);
    let result = executor
        .execute_with_options(
            &plan,
            &confirmed,
            PurgeExecuteOptions::new(NOW).with_batch_size(1),
        )
        .expect("content purge");

    assert_eq!(result.repository_id, REPOSITORY_A);
    assert_eq!(result.counts, plan.counts);
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read purged revision")
            .expect("purged revision")
            .body,
        PURGED_CONTENT_MARKER
    );
    assert_eq!(
        executor
            .state()
            .inbound_item(REPOSITORY_A, "item-a")
            .expect("read purged item")
            .expect("purged item")
            .first_content,
        PURGED_CONTENT_MARKER
    );
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_B, "draft-a", 1)
            .expect("read other repository")
            .expect("other repository")
            .body,
        "synthetic draft text"
    );
    assert_eq!(
        count_rows(executor.state(), "audit_events", REPOSITORY_B),
        1
    );

    let audit = executor
        .state()
        .audit_event(REPOSITORY_A, &result.audit_event_id)
        .expect("read purge audit")
        .expect("purge audit");
    assert_eq!(audit.object_type, "purge");
    let metadata: Value = serde_json::from_str(&audit.metadata_json).expect("audit JSON");
    assert_eq!(metadata["category"], "content");
    assert_eq!(metadata["counts"]["primary"], 5);
    assert_eq!(metadata["counts"]["secondary"], 0);
    assert_eq!(metadata["counts"]["total"], 5);
    let serialized = audit.metadata_json;
    for forbidden in [
        "synthetic draft text",
        "synthetic first text",
        "synthetic current text",
        "synthetic transition text",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "audit leaked content: {forbidden}"
        );
    }
}

#[test]
fn metadata_execution_removes_metadata_but_preserves_content_and_all_scope_removes_operational_rows()
 {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A, "config-a");
    add_metadata(&mut store, REPOSITORY_A);

    let metadata_plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Metadata, cutoff()),
        )
        .expect("metadata plan");
    let metadata_confirmation = confirmation(&metadata_plan);
    let mut metadata_executor = PurgeExecutor::new(store);
    metadata_executor
        .execute(&metadata_plan, &metadata_confirmation, NOW)
        .expect("metadata purge");
    assert_eq!(
        metadata_executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read retained content")
            .expect("retained content")
            .body,
        "synthetic draft text"
    );
    assert_eq!(
        count_rows(metadata_executor.state(), "audit_events", REPOSITORY_A),
        1
    );
    assert!(
        metadata_executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read metadata-cleared revision")
            .expect("metadata-cleared revision")
            .metadata_json
            == "{}"
    );

    let store = metadata_executor.into_state();
    let all_plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::All, cutoff()),
        )
        .expect("all plan");
    let all_confirmation = confirmation(&all_plan);
    let mut all_executor = PurgeExecutor::new(store);
    let result = all_executor
        .execute(&all_plan, &all_confirmation, NOW)
        .expect("all purge");
    assert_eq!(result.counts, all_plan.counts);
    assert_eq!(count_rows(all_executor.state(), "drafts", REPOSITORY_A), 0);
    assert_eq!(
        count_rows(all_executor.state(), "draft_revisions", REPOSITORY_A),
        0
    );
    assert_eq!(
        count_rows(all_executor.state(), "inbound_items", REPOSITORY_A),
        0
    );
    assert_eq!(
        count_rows(all_executor.state(), "audit_events", REPOSITORY_A),
        1
    );
    assert!(
        all_executor
            .state()
            .repository(REPOSITORY_A)
            .expect("repository identity")
            .is_some()
    );
}

#[test]
fn injected_partial_failure_rolls_back_and_requires_a_fresh_plan_before_retry() {
    let store = basic_store();
    let plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff()),
        )
        .expect("plan");
    let confirmed = confirmation(&plan);
    let mut executor = PurgeExecutor::new(store);
    let failure = executor.execute_with_options(
        &plan,
        &confirmed,
        PurgeExecuteOptions::new(NOW)
            .with_batch_size(1)
            .with_failure_point(PurgeFailurePoint::AfterContent(1)),
    );
    assert!(matches!(failure, Err(PurgeError::InjectedFailure { .. })));
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read rolled-back revision")
            .expect("rolled-back revision")
            .body,
        "synthetic draft text"
    );
    assert_eq!(
        executor
            .state()
            .inbound_item(REPOSITORY_A, "item-a")
            .expect("read rolled-back item")
            .expect("rolled-back item")
            .first_content,
        "synthetic first text"
    );
    assert_eq!(
        count_rows(executor.state(), "audit_events", REPOSITORY_A),
        1
    );
    assert!(
        executor
            .state()
            .connection()
            .execute(
                "UPDATE draft_revisions SET body = 'tampered'
                 WHERE repository_id = ?1 AND draft_id = 'draft-a'",
                [REPOSITORY_A],
            )
            .is_err()
    );
    assert!(
        executor
            .state()
            .connection()
            .execute(
                "DELETE FROM inbound_items WHERE repository_id = ?1 AND item_id = 'item-a'",
                [REPOSITORY_A],
            )
            .is_err()
    );
    assert_eq!(
        executor.execute(&plan, &confirmed, NOW),
        Err(PurgeError::ReplanRequired)
    );

    let fresh = PurgePlanner::new()
        .plan(
            executor.state(),
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff()),
        )
        .expect("fresh plan");
    let fresh_confirmation = confirmation(&fresh);
    executor.clear_failed_plan();
    executor
        .execute(&fresh, &fresh_confirmation, NOW)
        .expect("fresh confirmed retry");
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read committed revision")
            .expect("committed revision")
            .body,
        PURGED_CONTENT_MARKER
    );
}

#[test]
fn injected_audit_failure_rolls_back_content_and_summary_together() {
    let store = basic_store();
    let plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff()),
        )
        .expect("plan");
    let confirmed = confirmation(&plan);
    let mut executor = PurgeExecutor::new(store);
    let result = executor.execute_with_options(
        &plan,
        &confirmed,
        PurgeExecuteOptions::new(NOW).with_failure_point(PurgeFailurePoint::AfterAudit),
    );
    assert!(matches!(result, Err(PurgeError::InjectedFailure { .. })));
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read rolled-back revision")
            .expect("rolled-back revision")
            .body,
        "synthetic draft text"
    );
    assert_eq!(
        count_rows(executor.state(), "audit_events", REPOSITORY_A),
        1
    );
}

#[test]
fn current_state_change_invalidates_a_previously_built_plan_before_mutation() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A, "config-a");
    add_draft(&mut store, REPOSITORY_A, "draft-a", OLD, "old text");
    let plan = PurgePlanner::new()
        .plan(
            &store,
            &PurgeRequest::new(REPOSITORY_A, PurgeScope::Content, cutoff()),
        )
        .expect("initial plan");
    add_draft(&mut store, REPOSITORY_A, "draft-b", OLD, "new local text");
    let confirmed = confirmation(&plan);
    let mut executor = PurgeExecutor::new(store);
    assert_eq!(
        executor.execute(&plan, &confirmed, NOW),
        Err(PurgeError::ReplanRequired)
    );
    assert_eq!(
        count_rows(executor.state(), "draft_revisions", REPOSITORY_A),
        2
    );
    assert_eq!(
        executor
            .state()
            .draft_revision(REPOSITORY_A, "draft-a", 1)
            .expect("read unchanged")
            .expect("unchanged")
            .body,
        "old text"
    );
}

#[test]
fn public_surface_has_no_remote_or_discord_mutation_capability() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in ["discord", "reqwest", "telemetry", "analytics"] {
        assert!(
            !manifest.to_ascii_lowercase().contains(forbidden),
            "manifest unexpectedly contains {forbidden}"
        );
    }
    let source = [
        include_str!("../src/lib.rs"),
        include_str!("../src/plan.rs"),
        include_str!("../src/execute.rs"),
    ]
    .concat();
    for forbidden in [
        "DiscordClient",
        "reqwest",
        "delete_message",
        "edit_message",
        "react_to",
        "telemetry",
    ] {
        assert!(
            !source.contains(forbidden),
            "purge API unexpectedly contains {forbidden}"
        );
    }
}

#[test]
fn failure_point_alias_is_available_for_deterministic_rollback_tests() {
    let point = FailurePoint::AfterMetadata(2);
    assert_eq!(point, PurgeFailurePoint::AfterMetadata(2));
    assert_eq!(point.to_string(), "after-metadata-2");
}
