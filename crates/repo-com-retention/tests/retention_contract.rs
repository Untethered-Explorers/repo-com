use std::cell::Cell;

use repo_com_state::{
    ApprovalInput, AuditEventInput, DeliveryAttemptInput, DraftInput, DraftRevisionInput,
    InboundCurrentSnapshotInput, InboundCursorInput, InboundItemInput, InboundTransitionInput,
    ReplyLinkInput, RepositoryInput, StateStore,
};

use crate::{
    CONTENT_EXPIRED_MARKER, DEFAULT_CONTENT_DAYS, DEFAULT_METADATA_DAYS, FailurePoint, FixedClock,
    MAX_CONTENT_DAYS, MAX_METADATA_DAYS, MIN_CONTENT_DAYS, MIN_METADATA_DAYS, RemovedId,
    RetentionError, RetentionPolicy, RetentionPolicyError, RetentionSweeper, SECONDS_PER_DAY,
    SweepOptions, calculate_cutoffs, format_rfc3339_utc, parse_rfc3339_utc,
};

const NOW: &str = "2026-01-01T00:00:00Z";

fn now_seconds() -> u64 {
    parse_rfc3339_utc(NOW).expect("test clock timestamp")
}

fn days_ago(now: u64, days: u64) -> String {
    format_rfc3339_utc(now - days * SECONDS_PER_DAY).expect("test timestamp")
}

fn register(store: &mut StateStore, repository_id: &str) {
    store
        .upsert_repository(&RepositoryInput::new(
            repository_id,
            format!("workspace-{repository_id}"),
            format!("config-{repository_id}"),
            NOW,
        ))
        .expect("register repository");
}

fn add_draft(
    store: &mut StateStore,
    repository_id: &str,
    draft_id: &str,
    timestamp: &str,
    body: &str,
    reply_to: Option<&str>,
) {
    let mut draft = DraftInput::new(
        repository_id,
        draft_id,
        "build_failed",
        "release",
        timestamp,
    );
    draft.reply_to_inbound_item_id = reply_to.map(str::to_owned);
    store.create_draft(&draft).expect("create draft");

    let mut revision = DraftRevisionInput::new(
        repository_id,
        draft_id,
        1,
        format!("hash-{draft_id}"),
        body,
        "release",
        "resolved-release",
        timestamp,
    );
    revision.reply_to_inbound_item_id = reply_to.map(str::to_owned);
    revision.metadata_json = "{\"branch\":\"main\"}".to_owned();
    store
        .insert_draft_revision(&revision)
        .expect("insert draft revision");
}

fn add_inbound(
    store: &mut StateStore,
    repository_id: &str,
    item_id: &str,
    timestamp: &str,
    first_content: &str,
    current_content: &str,
) {
    let first = InboundItemInput::new(
        repository_id,
        item_id,
        "channel-1",
        "human-1",
        first_content,
        timestamp,
    );
    let current = InboundCurrentSnapshotInput::new(
        repository_id,
        item_id,
        Some(current_content.to_owned()),
        false,
        timestamp,
    );
    store
        .store_inbound_item(&first, &current)
        .expect("store inbound item");
}

fn add_transition(
    store: &mut StateStore,
    repository_id: &str,
    item_id: &str,
    transition_id: &str,
    timestamp: &str,
    content: &str,
) {
    let transition = InboundTransitionInput::new(
        repository_id,
        transition_id,
        item_id,
        "edited",
        Some(content.to_owned()),
        timestamp,
    );
    let current = InboundCurrentSnapshotInput::new(
        repository_id,
        item_id,
        Some(content.to_owned()),
        false,
        timestamp,
    );
    store
        .record_inbound_transition(&transition, Some(&current))
        .expect("record inbound transition");
}

fn add_audit(store: &mut StateStore, repository_id: &str, event_id: &str, timestamp: &str) {
    store
        .append_audit_event(&AuditEventInput::new(
            repository_id,
            event_id,
            "test-object",
            "object-1",
            "observed",
            timestamp,
            "system",
            "success",
        ))
        .expect("append audit evidence");
}

#[test]
fn defaults_and_exact_cutoff_boundaries_are_clock_controlled() {
    let now = now_seconds();
    let policy = RetentionPolicy::default();
    let cutoffs = calculate_cutoffs("repo-a", now, &policy).expect("default cutoffs");

    assert_eq!(policy.content_days, DEFAULT_CONTENT_DAYS);
    assert_eq!(policy.metadata_days, DEFAULT_METADATA_DAYS);
    assert_eq!(cutoffs.repository_id, "repo-a");
    assert_eq!(cutoffs.as_of, NOW);
    assert_eq!(cutoffs.content_cutoff, days_ago(now, 30));
    assert_eq!(cutoffs.metadata_cutoff, days_ago(now, 365));
    assert_eq!(
        cutoffs.content_cutoff_unix_seconds,
        now - (DEFAULT_CONTENT_DAYS as u64 * SECONDS_PER_DAY)
    );
    assert_eq!(
        cutoffs.metadata_cutoff_unix_seconds,
        now - (DEFAULT_METADATA_DAYS as u64 * SECONDS_PER_DAY)
    );

    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, "repo-a");
    let exact = days_ago(now, DEFAULT_CONTENT_DAYS as u64);
    let newer = days_ago(now, DEFAULT_CONTENT_DAYS as u64 - 1);
    add_draft(&mut store, "repo-a", "exact", &exact, "exact text", None);
    add_draft(&mut store, "repo-a", "newer", &newer, "new text", None);
    let metadata_exact = days_ago(now, DEFAULT_METADATA_DAYS as u64);
    let metadata_newer = format_rfc3339_utc(cutoffs.metadata_cutoff_unix_seconds + 1)
        .expect("newer metadata timestamp");
    store
        .advance_inbound_cursor(&InboundCursorInput::new(
            "repo-a",
            "metadata-exact",
            "cursor-exact",
            &metadata_exact,
        ))
        .expect("exact metadata cursor");
    store
        .advance_inbound_cursor(&InboundCursorInput::new(
            "repo-a",
            "metadata-newer",
            "cursor-newer",
            &metadata_newer,
        ))
        .expect("newer metadata cursor");

    let mut sweeper = RetentionSweeper::new(store, "repo-a", policy, FixedClock::new(now));
    let result = sweeper.sweep().expect("boundary sweep");
    assert_eq!(result.counts.content_rows_redacted, 1);
    assert_eq!(result.counts.metadata_rows_removed, 1);
    assert_eq!(
        sweeper
            .state()
            .draft_revision("repo-a", "exact", 1)
            .expect("read exact revision")
            .expect("exact revision")
            .body,
        CONTENT_EXPIRED_MARKER
    );
    assert_eq!(
        sweeper
            .state()
            .draft_revision("repo-a", "newer", 1)
            .expect("read newer revision")
            .expect("newer revision")
            .body,
        "new text"
    );
    assert!(
        sweeper
            .state()
            .inbound_cursor("repo-a", "metadata-exact")
            .expect("read exact metadata cursor")
            .is_none()
    );
    assert!(
        sweeper
            .state()
            .inbound_cursor("repo-a", "metadata-newer")
            .expect("read newer metadata cursor")
            .is_some()
    );
}

#[test]
fn override_ranges_and_metadata_order_are_validated() {
    for (content, metadata) in [
        (MIN_CONTENT_DAYS, MIN_METADATA_DAYS),
        (MAX_CONTENT_DAYS, MAX_METADATA_DAYS),
        (1, 3_650),
    ] {
        let policy = RetentionPolicy::new(content, metadata).expect("allowed override");
        assert_eq!(policy.content_retention_days(), content);
        assert_eq!(policy.metadata_retention_days(), metadata);
    }

    for (content, metadata, expected) in [
        (
            0,
            365,
            RetentionPolicyError::InvalidContentDays { value: 0 },
        ),
        (
            366,
            365,
            RetentionPolicyError::InvalidContentDays { value: 366 },
        ),
        (
            30,
            29,
            RetentionPolicyError::InvalidMetadataDays { value: 29 },
        ),
        (
            30,
            3_651,
            RetentionPolicyError::InvalidMetadataDays { value: 3_651 },
        ),
        (
            31,
            30,
            RetentionPolicyError::MetadataShorterThanContent {
                content_days: 31,
                metadata_days: 30,
            },
        ),
    ] {
        assert_eq!(
            RetentionPolicy::new(content, metadata).expect_err("invalid override"),
            expected
        );
    }
}

#[test]
fn content_expiry_preserves_identity_hashes_timestamps_links_and_audit() {
    let now = now_seconds();
    let old = days_ago(now, DEFAULT_CONTENT_DAYS as u64);
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, "repo-a");
    add_inbound(
        &mut store,
        "repo-a",
        "item-a",
        &old,
        "first secret",
        "current secret",
    );
    add_transition(
        &mut store,
        "repo-a",
        "item-a",
        "edit-a",
        &old,
        "edited secret",
    );
    add_draft(
        &mut store,
        "repo-a",
        "draft-a",
        &old,
        "draft secret",
        Some("item-a"),
    );
    store
        .acknowledge_inbound("repo-a", &["item-a"], &old)
        .expect("acknowledge");
    store
        .archive_inbound("repo-a", &["item-a"], &old)
        .expect("archive");
    store
        .link_inbound_reply(&ReplyLinkInput::new("repo-a", "item-a", "draft-a", &old))
        .expect("link reply");
    add_audit(&mut store, "repo-a", "audit-a", &old);

    let original_revision = store
        .draft_revision("repo-a", "draft-a", 1)
        .expect("read original revision")
        .expect("original revision");
    let original_item = store
        .inbound_item("repo-a", "item-a")
        .expect("read original item")
        .expect("original item");
    let original_current = store
        .inbound_current("repo-a", "item-a")
        .expect("read original current")
        .expect("original current");
    let original_link = store
        .inbound_reply_link("repo-a", "item-a")
        .expect("read original link")
        .expect("original link");

    let mut sweeper = RetentionSweeper::new(
        store,
        "repo-a",
        RetentionPolicy::default(),
        FixedClock::new(now),
    );
    let result = sweeper.sweep().expect("content sweep");
    assert!(result.counts.content_rows_redacted >= 5);
    assert!(result.removed_ids.is_empty());
    assert!(result.redacted_ids.iter().any(|id| matches!(
        id,
        RemovedId::DraftRevision { draft_id, .. } if draft_id == "draft-a"
    )));

    let revision = sweeper
        .state()
        .draft_revision("repo-a", "draft-a", 1)
        .expect("read retained revision")
        .expect("retained revision");
    assert_eq!(revision.body, CONTENT_EXPIRED_MARKER);
    assert_eq!(revision.content_hash, original_revision.content_hash);
    assert_eq!(revision.created_at, original_revision.created_at);
    assert_eq!(revision.lifecycle_state, original_revision.lifecycle_state);
    assert_eq!(
        revision.reply_to_inbound_item_id,
        original_revision.reply_to_inbound_item_id
    );
    assert_eq!(revision.metadata_json, original_revision.metadata_json);

    let item = sweeper
        .state()
        .inbound_item("repo-a", "item-a")
        .expect("read retained item")
        .expect("retained item");
    assert_eq!(item.first_content, CONTENT_EXPIRED_MARKER);
    assert_eq!(item.item_id, original_item.item_id);
    assert_eq!(item.author_id, original_item.author_id);
    assert_eq!(item.first_observed_at, original_item.first_observed_at);
    let current = sweeper
        .state()
        .inbound_current("repo-a", "item-a")
        .expect("read retained current")
        .expect("retained current");
    assert_eq!(
        current.current_content.as_deref(),
        Some(CONTENT_EXPIRED_MARKER)
    );
    assert_eq!(current.observed_at, original_current.observed_at);
    assert_eq!(
        sweeper
            .state()
            .inbound_reply_link("repo-a", "item-a")
            .expect("read retained link")
            .expect("retained link"),
        original_link
    );
    assert!(
        sweeper
            .state()
            .inbound_acknowledgement("repo-a", "item-a")
            .expect("ack")
            .is_some()
    );
    assert!(
        sweeper
            .state()
            .inbound_archive("repo-a", "item-a")
            .expect("archive")
            .is_some()
    );
    assert!(
        sweeper
            .state()
            .audit_event("repo-a", "audit-a")
            .expect("audit")
            .is_some()
    );

    let replay = sweeper.sweep().expect("idempotent replay");
    assert_eq!(replay.counts.content_rows_redacted, 0);
    assert_eq!(replay.audit_event_id, result.audit_event_id);
}

#[test]
fn metadata_expiry_removes_only_later_non_content_rows_and_keeps_summary() {
    let now = now_seconds();
    let old = days_ago(now, 400);
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, "repo-a");
    add_draft(&mut store, "repo-a", "old-draft", &old, "old draft", None);
    store
        .record_approval(&ApprovalInput::new(
            "repo-a",
            "approval-a",
            "old-draft",
            1,
            "operator",
            &old,
        ))
        .expect("approval");
    store
        .record_delivery_attempt(&DeliveryAttemptInput::new(
            "repo-a",
            "attempt-a",
            "old-draft",
            1,
            1,
            "nonce-a",
            &old,
        ))
        .expect("delivery attempt");
    add_inbound(
        &mut store,
        "repo-a",
        "old-item",
        &old,
        "old inbound",
        "old inbound",
    );
    store
        .acknowledge_inbound("repo-a", &["old-item"], &old)
        .expect("acknowledge");
    store
        .archive_inbound("repo-a", &["old-item"], &old)
        .expect("archive");
    store
        .advance_inbound_cursor(&repo_com_state::InboundCursorInput::new(
            "repo-a", "inbound", "cursor-a", &old,
        ))
        .expect("cursor");
    add_audit(&mut store, "repo-a", "old-audit", &old);

    let mut sweeper = RetentionSweeper::new(
        store,
        "repo-a",
        RetentionPolicy::default(),
        FixedClock::new(now),
    );
    let result = sweeper.sweep().expect("metadata sweep");
    assert!(result.counts.content_rows_redacted > 0);
    assert!(result.counts.metadata_rows_removed > 0);
    assert!(result.removed_ids.items.iter().any(|id| matches!(
        id,
        RemovedId::Draft { draft_id, .. } if draft_id == "old-draft"
    )));
    assert!(
        sweeper
            .state()
            .draft("repo-a", "old-draft")
            .expect("draft")
            .is_none()
    );
    assert!(
        sweeper
            .state()
            .draft_revision("repo-a", "old-draft", 1)
            .expect("revision")
            .is_none()
    );
    assert!(
        sweeper
            .state()
            .inbound_item("repo-a", "old-item")
            .expect("item")
            .is_none()
    );
    assert!(
        sweeper
            .state()
            .inbound_cursor("repo-a", "inbound")
            .expect("cursor")
            .is_none()
    );
    assert!(
        sweeper
            .state()
            .audit_event("repo-a", "old-audit")
            .expect("old audit")
            .is_none()
    );
    let summary = sweeper
        .state()
        .audit_event("repo-a", &result.audit_event_id)
        .expect("summary")
        .expect("summary event");
    assert!(!summary.metadata_json.contains("old draft"));
    assert!(!summary.metadata_json.contains("old inbound"));
}

#[test]
fn injected_partial_failure_rolls_back_and_blocks_new_mutation() {
    let now = now_seconds();
    let old = days_ago(now, 30);
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, "repo-a");
    add_draft(&mut store, "repo-a", "draft-a", &old, "must survive", None);
    add_inbound(
        &mut store,
        "repo-a",
        "item-a",
        &old,
        "inbound must survive",
        "current must survive",
    );

    let mut sweeper = RetentionSweeper::new(
        store,
        "repo-a",
        RetentionPolicy::default(),
        FixedClock::new(now),
    );
    let called = Cell::new(false);
    let error = sweeper
        .run_before_mutation_with_options(
            SweepOptions::new().with_failure_point(FailurePoint::AfterContent(1)),
            |state| {
                called.set(true);
                state.create_draft(&DraftInput::new(
                    "repo-a",
                    "new-draft",
                    "event",
                    "release",
                    NOW,
                ))
            },
        )
        .expect_err("injected sweep must block mutation");
    assert!(!called.get());
    assert!(matches!(
        error,
        RetentionError::StorageIntegrity(ref value)
            if value.blocks_mutation() && value.phase == crate::SweepPhase::Content
    ));

    // The failed transaction must restore both content and schema invariants.
    assert_eq!(
        sweeper
            .state()
            .draft_revision("repo-a", "draft-a", 1)
            .expect("read rolled-back draft")
            .expect("rolled-back draft")
            .body,
        "must survive"
    );
    assert!(sweeper
        .state()
        .connection()
        .execute(
            "UPDATE draft_revisions SET body = 'tampered'\n             WHERE repository_id = 'repo-a' AND draft_id = 'draft-a' AND revision = 1",
            [],
        )
        .is_err());
    let retention_audit_count: i64 = sweeper
        .state()
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE repository_id = 'repo-a' AND object_type = 'retention'",
            [],
            |row| row.get(0),
        )
        .expect("count retention audit rows");
    assert_eq!(retention_audit_count, 0);
}

#[test]
fn one_repository_sweep_never_changes_another_repository() {
    let now = now_seconds();
    let old = days_ago(now, 30);
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, "repo-a");
    register(&mut store, "repo-b");
    add_draft(&mut store, "repo-a", "shared", &old, "repo a text", None);
    add_draft(&mut store, "repo-b", "shared", &old, "repo b text", None);
    add_inbound(
        &mut store,
        "repo-a",
        "shared-item",
        &old,
        "a first",
        "a current",
    );
    add_inbound(
        &mut store,
        "repo-b",
        "shared-item",
        &old,
        "b first",
        "b current",
    );

    let mut sweeper = RetentionSweeper::new(
        store,
        "repo-a",
        RetentionPolicy::default(),
        FixedClock::new(now),
    );
    sweeper.sweep().expect("repo-a sweep");
    assert_eq!(
        sweeper
            .state()
            .draft_revision("repo-a", "shared", 1)
            .expect("repo-a revision")
            .expect("repo-a revision")
            .body,
        CONTENT_EXPIRED_MARKER
    );
    assert_eq!(
        sweeper
            .state()
            .draft_revision("repo-b", "shared", 1)
            .expect("repo-b revision")
            .expect("repo-b revision")
            .body,
        "repo b text"
    );
    assert_eq!(
        sweeper
            .state()
            .inbound_item("repo-b", "shared-item")
            .expect("repo-b item")
            .expect("repo-b item")
            .first_content,
        "b first"
    );
}

#[test]
fn manifest_has_no_remote_or_discord_capability() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("discord"));
    assert!(!manifest.contains("reqwest"));
    assert!(!manifest.contains("telemetry"));
}
