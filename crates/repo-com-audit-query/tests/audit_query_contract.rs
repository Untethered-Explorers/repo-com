use repo_com_audit::{AuditEvent, AuditWriter, REDACTED};
use repo_com_state::{AuditEventInput, RepositoryInput, StateStore};
use serde_json::json;

use crate::{AuditCursor, AuditFilter, AuditQuery, AuditQueryError, MAX_PAGE_SIZE};

const REPOSITORY_A: &str = "acme/widgets";
const REPOSITORY_B: &str = "other/project";
const BASE_TIME: &str = "2026-01-01T00:00:00Z";

fn register(store: &mut StateStore, repository_id: &str) {
    store
        .upsert_repository(&RepositoryInput::new(
            repository_id,
            "123456789012345678",
            "config-hash",
            BASE_TIME,
        ))
        .expect("register repository");
}

fn event(
    repository_id: &str,
    event_id: &str,
    object_type: &str,
    object_id: &str,
    transition: &str,
    occurred_at: &str,
) -> AuditEvent {
    AuditEvent::new(
        repository_id,
        event_id,
        object_type,
        object_id,
        transition,
        occurred_at,
        "system",
        "success",
    )
}

fn append(store: &mut StateStore, event: AuditEvent) {
    AuditWriter::new(store)
        .append(&event)
        .expect("append audit event");
}

fn seed_filter_rows(store: &mut StateStore) {
    append(
        store,
        event(
            REPOSITORY_A,
            "a-created-1",
            "draft",
            "draft-1",
            "created",
            "2026-01-01T00:00:00Z",
        ),
    );
    append(
        store,
        event(
            REPOSITORY_A,
            "a-edited-1",
            "draft",
            "draft-1",
            "edited",
            "2026-01-01T00:01:00Z",
        ),
    );
    append(
        store,
        event(
            REPOSITORY_A,
            "a-created-2",
            "draft",
            "draft-2",
            "created",
            "2026-01-01T00:02:00Z",
        ),
    );
    append(
        store,
        event(
            REPOSITORY_A,
            "a-observed-1",
            "inbound_item",
            "item-1",
            "observed",
            "2026-01-01T00:03:00Z",
        ),
    );
    append(
        store,
        event(
            REPOSITORY_A,
            "a-created-3",
            "draft",
            "draft-1",
            "created",
            "2026-01-01T00:04:00Z",
        ),
    );
    append(
        store,
        event(
            REPOSITORY_B,
            "b-created-1",
            "draft",
            "draft-1",
            "created",
            "2026-01-01T00:01:30Z",
        ),
    );
}

fn ids(page: &crate::AuditPage) -> Vec<&str> {
    page.events
        .iter()
        .map(|event| event.event.event_id.as_str())
        .collect()
}

#[test]
fn repository_time_object_and_transition_filters_apply_separately_and_together() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    register(&mut store, REPOSITORY_B);
    seed_filter_rows(&mut store);
    let query = AuditQuery::new(&store);

    let repository_only = query
        .query(&AuditFilter::new(REPOSITORY_A))
        .expect("repository filter");
    assert_eq!(ids(&repository_only).len(), 5);
    assert!(
        repository_only
            .events
            .iter()
            .all(|event| event.event.repository_id == REPOSITORY_A)
    );

    let time_only = query
        .query(
            &AuditFilter::new(REPOSITORY_A)
                .with_time_range("2026-01-01T00:01:00Z", "2026-01-01T00:04:00Z"),
        )
        .expect("time filter");
    assert_eq!(
        ids(&time_only),
        ["a-edited-1", "a-created-2", "a-observed-1"]
    );

    let object_type_only = query
        .query(&AuditFilter::new(REPOSITORY_A).with_object_type("draft"))
        .expect("object type filter");
    assert_eq!(
        ids(&object_type_only),
        ["a-created-1", "a-edited-1", "a-created-2", "a-created-3"]
    );

    let object_id_only = query
        .query(&AuditFilter::new(REPOSITORY_A).with_object_id("draft-1"))
        .expect("object id filter");
    assert_eq!(
        ids(&object_id_only),
        ["a-created-1", "a-edited-1", "a-created-3"]
    );

    let transition_only = query
        .query(&AuditFilter::new(REPOSITORY_A).with_transition("created"))
        .expect("transition filter");
    assert_eq!(
        ids(&transition_only),
        ["a-created-1", "a-created-2", "a-created-3"]
    );

    let combined = query
        .query(
            &AuditFilter::new(REPOSITORY_A)
                .with_time_range("2026-01-01T00:01:00Z", "2026-01-01T00:04:00Z")
                .with_object_type("draft")
                .with_object_id("draft-1")
                .with_transition("edited"),
        )
        .expect("combined filter");
    assert_eq!(ids(&combined), ["a-edited-1"]);
}

#[test]
fn pagination_is_stable_bounded_and_exposes_continuation_metadata() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    for index in 0..5 {
        append(
            &mut store,
            event(
                REPOSITORY_A,
                &format!("event-{index}"),
                "draft",
                "draft-1",
                "created",
                BASE_TIME,
            ),
        );
    }
    let query = AuditQuery::new(&store);
    let first = query
        .query(&AuditFilter::new(REPOSITORY_A).with_page_size(2))
        .expect("first page");
    assert_eq!(ids(&first), ["event-0", "event-1"]);
    assert_eq!(first.page_size, 2);
    assert!(first.truncated);
    assert!(first.has_more());
    let first_cursor = first.next_cursor.clone().expect("first continuation");

    let second = query
        .query(
            &AuditFilter::new(REPOSITORY_A)
                .with_page_size(2)
                .with_cursor(first_cursor),
        )
        .expect("second page");
    assert_eq!(ids(&second), ["event-2", "event-3"]);
    assert!(second.truncated);
    let second_cursor = second.next_cursor.clone().expect("second continuation");

    let third = query
        .query(
            &AuditFilter::new(REPOSITORY_A)
                .with_page_size(2)
                .with_cursor(second_cursor),
        )
        .expect("final page");
    assert_eq!(ids(&third), ["event-4"]);
    assert!(!third.truncated);
    assert!(third.next_cursor.is_none());

    let all_ids = format!("{:?}{:?}{:?}", ids(&first), ids(&second), ids(&third));
    assert_eq!(
        all_ids,
        "[\"event-0\", \"event-1\"][\"event-2\", \"event-3\"][\"event-4\"]"
    );

    let oversized = query.query(&AuditFilter::new(REPOSITORY_A).with_page_size(MAX_PAGE_SIZE + 1));
    assert_eq!(
        oversized,
        Err(AuditQueryError::InvalidFilter { field: "page_size" })
    );
}

#[test]
fn repository_scope_prevents_cross_repository_evidence_from_being_returned() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    register(&mut store, REPOSITORY_B);
    append(
        &mut store,
        event(
            REPOSITORY_A,
            "same-event",
            "draft",
            "same-object",
            "created",
            BASE_TIME,
        ),
    );
    append(
        &mut store,
        event(
            REPOSITORY_B,
            "same-event",
            "draft",
            "same-object",
            "created",
            BASE_TIME,
        ),
    );

    let page = AuditQuery::new(&store)
        .query(&AuditFilter::new(REPOSITORY_A))
        .expect("scoped query");
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event.repository_id, REPOSITORY_A);
    assert!(
        page.events
            .iter()
            .all(|event| event.event.repository_id != REPOSITORY_B)
    );
}

#[test]
fn query_output_reapplies_redaction_and_never_returns_raw_message_content() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    let raw_body = "private message body that must not escape";
    let token = "MTIzNDU2.Gabcde.fghijklmnopqrstuvwxyz123456";
    let event = AuditEvent::with_metadata(
        REPOSITORY_A,
        "redacted-event",
        "draft",
        "draft-1",
        "created",
        BASE_TIME,
        "system",
        "success",
        json!({
            "message": raw_body,
            "authorization": format!("Bot {token}"),
            "nested": {"content": raw_body, "status": "ok"},
            "status": "visible"
        }),
    );
    append(&mut store, event);

    let mut direct = AuditEventInput::new(
        REPOSITORY_A,
        "direct-raw-event",
        "draft",
        "draft-2",
        "created",
        BASE_TIME,
        "system",
        "success",
    );
    direct.metadata_json = json!({
        "message": raw_body,
        "bot_token": token,
        "status": "visible"
    })
    .to_string();
    store
        .append_audit_event(&direct)
        .expect("append defensive test row");

    let page = AuditQuery::new(&store)
        .query(&AuditFilter::new(REPOSITORY_A))
        .expect("redacted query");
    let serialized = serde_json::to_string(&page).expect("safe page JSON");
    assert!(!serialized.contains(raw_body));
    assert!(!serialized.contains(token));
    assert!(serialized.contains(REDACTED));

    let writer_row = page
        .events
        .iter()
        .find(|event| event.event.event_id == "redacted-event")
        .expect("writer row");
    assert_eq!(writer_row.event.metadata["message"], REDACTED);
    assert_eq!(writer_row.event.metadata["nested"]["content"], REDACTED);
    assert_eq!(writer_row.event.metadata["status"], "visible");

    let direct_row = page
        .events
        .iter()
        .find(|event| event.event.event_id == "direct-raw-event")
        .expect("direct row");
    assert_eq!(direct_row.event.metadata["message"], REDACTED);
    assert_eq!(direct_row.event.metadata["bot_token"], REDACTED);
}

#[test]
fn unsafe_stored_metadata_fails_closed_instead_of_being_copied() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    let mut direct = AuditEventInput::new(
        REPOSITORY_A,
        "malformed-event",
        "draft",
        "draft-1",
        "created",
        BASE_TIME,
        "system",
        "success",
    );
    direct.metadata_json = "not-json".to_owned();
    store
        .append_audit_event(&direct)
        .expect("append malformed test row");

    assert_eq!(
        AuditQuery::new(&store).query(&AuditFilter::new(REPOSITORY_A)),
        Err(AuditQueryError::UnsafeStoredEvidence)
    );
}

#[test]
fn later_remote_transitions_are_new_local_evidence_and_query_is_read_only() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    append(
        &mut store,
        event(
            REPOSITORY_A,
            "first-snapshot",
            "inbound_item",
            "item-1",
            "created",
            BASE_TIME,
        ),
    );
    append(
        &mut store,
        event(
            REPOSITORY_A,
            "later-edit",
            "inbound_item",
            "item-1",
            "remote_edited",
            "2026-01-01T00:01:00Z",
        ),
    );

    let before: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
        .expect("count before query");
    let page = AuditQuery::new(&store)
        .query(&AuditFilter::new(REPOSITORY_A).with_object_id("item-1"))
        .expect("local evidence query");
    let after: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
        .expect("count after query");

    assert_eq!(before, after);
    assert_eq!(ids(&page), ["first-snapshot", "later-edit"]);
    assert!(
        store
            .connection()
            .execute(
                "UPDATE audit_events SET outcome = 'tampered' WHERE event_id = 'first-snapshot'",
                [],
            )
            .is_err()
    );
    assert!(
        store
            .connection()
            .execute(
                "DELETE FROM audit_events WHERE event_id = 'first-snapshot'",
                []
            )
            .is_err()
    );
}

#[test]
fn invalid_filters_and_cross_repository_cursors_fail_closed_without_echoing_values() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, REPOSITORY_A);
    register(&mut store, REPOSITORY_B);
    let query = AuditQuery::new(&store);

    let invalid_time = AuditFilter::new(REPOSITORY_A)
        .with_time_range("2026-01-01T00:00:00+00:00", "2026-01-01T00:01:00Z");
    let error = query.query(&invalid_time).expect_err("invalid time");
    assert_eq!(
        error,
        AuditQueryError::InvalidFilter {
            field: "occurred_from"
        }
    );

    let foreign_cursor = AuditCursor::new(REPOSITORY_B, BASE_TIME, 1).expect("foreign cursor");
    let cursor_error = query
        .query(&AuditFilter::new(REPOSITORY_A).with_cursor(foreign_cursor))
        .expect_err("cross-repository cursor");
    assert_eq!(cursor_error, AuditQueryError::InvalidCursor);

    let secret = "MTIzNDU2.Gabcde.fghijklmnopqrstuvwxyz123456";
    let unsafe_repository = AuditFilter::new(secret);
    let repository_error = query
        .query(&unsafe_repository)
        .expect_err("unsafe repository");
    assert!(!repository_error.to_string().contains(secret));
    assert!(!format!("{unsafe_repository:?}").contains(secret));
}
