use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use repo_com_audit::REDACTED;
use repo_com_audit_query::AuditFilter;
use repo_com_state::{
    AuditEventInput, DeliveryAttemptInput, DraftInput, DraftRevisionInput,
    InboundCurrentSnapshotInput, InboundItemInput, ReplyLinkInput, RepositoryInput, StateStore,
    database_path_in,
};
use rusqlite::Connection;
use serde_json::json;

use crate::{
    ACKNOWLEDGEMENT_OBJECT, ARCHIVE_OBJECT, AUDIT_OBJECT, CheckStatus, DELIVERY_ATTEMPT_OBJECT,
    DRAFT_OBJECT, DRAFT_REVISION_OBJECT, EvidenceSource, INBOUND_ITEM_OBJECT, InspectionRequest,
    LifecycleError, LifecycleInspector, LifecycleObject, LifecycleProjection, MAX_PAGE_SIZE,
    PageRequest, REPLY_LINK_OBJECT, ReadReceiptStatus, RemoteSnapshotState, ReplyClaimStatus,
    StateVerifier, VerificationRequest, verify_state,
};

const REPOSITORY_A: &str = "repo-a";
const REPOSITORY_B: &str = "repo-b";
const BASE_TIME: &str = "2026-01-01T00:00:00Z";
const LATER_TIME: &str = "2026-01-01T00:01:00Z";
const SECRET_BODY: &str = "private team message that must not escape diagnostics";
const SECRET_TOKEN: &str = "MTIzNDU2.Gabcde.fghijklmnopqrstuvwxyz123456";
const ITEM_A: &str = "item-a";
const ITEM_B: &str = "item-b";
const REMOTE_MESSAGE_A: &str = "500000000000000001";
const REPLY_REMOTE_MESSAGE_A: &str = "500000000000000002";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "repo-com-lifecycle-contract-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create lifecycle temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn store_and_path() -> (TempDir, PathBuf, StateStore) {
    let temp = TempDir::new();
    let path = database_path_in(temp.path());
    let store = StateStore::open_path(&path).expect("open state store");
    (temp, path, store)
}

fn register(store: &mut StateStore, repository_id: &str) {
    store
        .upsert_repository(&RepositoryInput::new(
            repository_id,
            format!("workspace-{repository_id}"),
            format!("config-{repository_id}"),
            BASE_TIME,
        ))
        .expect("register repository");
}

fn add_draft(
    store: &mut StateStore,
    repository_id: &str,
    draft_id: &str,
    body: &str,
    reply_to: Option<&str>,
) {
    let mut draft = DraftInput::new(
        repository_id,
        draft_id,
        "build_failed",
        "release",
        BASE_TIME,
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
        format!("channel-{repository_id}"),
        BASE_TIME,
    );
    revision.reply_to_inbound_item_id = reply_to.map(str::to_owned);
    revision.metadata_json = json!({
        "message": SECRET_BODY,
        "authorization": format!("Bot {SECRET_TOKEN}"),
        "status": "visible"
    })
    .to_string();
    store
        .insert_draft_revision(&revision)
        .expect("insert draft revision");
}

fn add_inbound(store: &mut StateStore, repository_id: &str, item_id: &str) {
    let first = InboundItemInput::new(
        repository_id,
        item_id,
        format!("channel-{repository_id}"),
        format!("author-{repository_id}"),
        SECRET_BODY,
        BASE_TIME,
    );
    let current = InboundCurrentSnapshotInput::new(
        repository_id,
        item_id,
        Some(format!("current-{SECRET_BODY}")),
        false,
        LATER_TIME,
    );
    store
        .store_inbound_item(&first, &current)
        .expect("store inbound item");
}

fn add_local_markers(store: &mut StateStore, repository_id: &str, item_id: &str) {
    store
        .acknowledge_inbound(repository_id, &[item_id], BASE_TIME)
        .expect("acknowledge local item");
    store
        .archive_inbound(repository_id, &[item_id], LATER_TIME)
        .expect("archive local item");
}

fn add_audit(store: &mut StateStore, repository_id: &str, event_id: &str, transition: &str) {
    let mut input = AuditEventInput::new(
        repository_id,
        event_id,
        "draft",
        "draft-out",
        transition,
        BASE_TIME,
        "system",
        "success",
    );
    input.metadata_json = json!({
        "message": SECRET_BODY,
        "bot_token": SECRET_TOKEN,
        "status": "visible"
    })
    .to_string();
    store
        .append_audit_event(&input)
        .expect("append local audit evidence");
}

fn add_accepted_delivery(
    store: &mut StateStore,
    repository_id: &str,
    attempt_id: &str,
    draft_id: &str,
    remote_message_id: &str,
) {
    let mut attempt = DeliveryAttemptInput::new(
        repository_id,
        attempt_id,
        draft_id,
        1,
        1,
        format!("nonce-{attempt_id}"),
        BASE_TIME,
    );
    attempt.state = "accepted".to_owned();
    attempt.completed_at = Some(LATER_TIME.to_owned());
    attempt.remote_message_id = Some(remote_message_id.to_owned());
    store
        .record_delivery_attempt(&attempt)
        .expect("record accepted delivery");
}

fn add_reply_evidence(
    store: &mut StateStore,
    repository_id: &str,
    item_id: &str,
    reply_draft_id: &str,
    attempt_id: &str,
    remote_message_id: &str,
) {
    let metadata = json!({
        "schema_version": 1,
        "repository_id": repository_id,
        "inbound_item_id": item_id,
        "target_message_id": item_id,
        "draft_id": reply_draft_id,
        "revision": 1,
        "revision_hash": "hash-reply-draft",
        "accepted_delivery_id": attempt_id,
        "reply_remote_message_id": remote_message_id,
        "human_message_delivery_claimed": false
    });
    let mut input = AuditEventInput::new(
        repository_id,
        "inbound-replied-evidence",
        "inbound_item",
        item_id,
        "replied",
        LATER_TIME,
        "system",
        "accepted",
    );
    input.metadata_json = metadata.to_string();
    store
        .append_audit_event(&input)
        .expect("append accepted reply evidence");
}

fn seed_complete(store: &mut StateStore) {
    register(store, REPOSITORY_A);
    register(store, REPOSITORY_B);
    add_draft(store, REPOSITORY_A, "draft-out", SECRET_BODY, None);
    add_draft(
        store,
        REPOSITORY_A,
        "reply-draft",
        "reply body",
        Some(ITEM_A),
    );
    add_draft(
        store,
        REPOSITORY_B,
        "draft-out",
        "other repository body",
        None,
    );
    add_inbound(store, REPOSITORY_A, ITEM_A);
    add_inbound(store, REPOSITORY_B, ITEM_B);
    add_local_markers(store, REPOSITORY_A, ITEM_A);
    store
        .link_inbound_reply(&ReplyLinkInput::new(
            REPOSITORY_A,
            ITEM_A,
            "reply-draft",
            LATER_TIME,
        ))
        .expect("link local reply draft");
    add_accepted_delivery(
        store,
        REPOSITORY_A,
        "attempt-out",
        "draft-out",
        REMOTE_MESSAGE_A,
    );
    add_accepted_delivery(
        store,
        REPOSITORY_A,
        "attempt-reply",
        "reply-draft",
        REPLY_REMOTE_MESSAGE_A,
    );
    add_audit(store, REPOSITORY_A, "draft-created", "created");
    add_audit(store, REPOSITORY_A, "draft-sent", "sent");
    add_reply_evidence(
        store,
        REPOSITORY_A,
        ITEM_A,
        "reply-draft",
        "attempt-reply",
        REPLY_REMOTE_MESSAGE_A,
    );
}

fn snapshot(path: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut paths = vec![path.to_path_buf()];
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    paths.push(PathBuf::from(wal));
    let mut shm = path.as_os_str().to_owned();
    shm.push("-shm");
    paths.push(PathBuf::from(shm));
    paths
        .into_iter()
        .map(|value| {
            let bytes = fs::read(&value).ok();
            (value, bytes)
        })
        .collect()
}

fn assert_snapshot_unchanged(path: &Path, before: &[(PathBuf, Option<Vec<u8>>)]) {
    for (file, expected) in before {
        assert_eq!(
            &fs::read(file).ok(),
            expected,
            "file changed: {}",
            file.display()
        );
    }
    assert!(path.exists());
}

#[test]
fn every_object_projection_is_typed_repository_scoped_and_provenance_explicit() {
    let (_temp, _path, mut store) = store_and_path();
    seed_complete(&mut store);
    let inspector = LifecycleInspector::new(&store);

    let repository = inspector
        .inspect_repository(REPOSITORY_A)
        .expect("repository projection");
    assert_eq!(repository.provenance, EvidenceSource::LocalState);
    assert_eq!(repository.repository_id, REPOSITORY_A);
    assert_eq!(
        inspector
            .inspect(&InspectionRequest::repository(REPOSITORY_A))
            .expect("unified repository projection")
            .object_type(),
        "repository"
    );

    let draft = inspector
        .inspect_draft(REPOSITORY_A, "draft-out")
        .expect("draft projection");
    assert_eq!(draft.draft_id, "draft-out");
    assert_eq!(draft.provenance, EvidenceSource::LocalState);
    assert_eq!(DRAFT_OBJECT, "draft");

    let ordinary_revision = inspector
        .inspect_draft_revision(REPOSITORY_A, "draft-out", 1, false)
        .expect("ordinary revision projection");
    assert!(ordinary_revision.body.is_none());
    assert!(!ordinary_revision.remote_fetch_performed);
    assert_eq!(DRAFT_REVISION_OBJECT, "draft_revision");
    let retained_revision = inspector
        .inspect_draft_revision(REPOSITORY_A, "draft-out", 1, true)
        .expect("retained revision projection");
    assert_eq!(
        retained_revision.body.as_ref().map(|value| value.as_str()),
        Some(SECRET_BODY)
    );
    assert!(retained_revision.retained_content_requested());

    let delivery = inspector
        .inspect_delivery_attempt(REPOSITORY_A, "attempt-out")
        .expect("delivery projection");
    assert!(delivery.is_accepted());
    assert_eq!(delivery.read_receipt, ReadReceiptStatus::Unavailable);
    assert_eq!(delivery.reply_claim, ReplyClaimStatus::NotClaimed);
    assert!(!delivery.read_receipt_claimed());
    assert!(!delivery.replied_claimed());
    assert_eq!(DELIVERY_ATTEMPT_OBJECT, "delivery_attempt");
    assert_eq!(
        delivery
            .last_recorded_remote
            .as_ref()
            .map(|value| value.provenance),
        Some(EvidenceSource::LastRecordedRemoteFetch)
    );

    let ordinary_inbound = inspector
        .inspect_inbound_item(REPOSITORY_A, ITEM_A, false)
        .expect("ordinary inbound projection");
    assert!(ordinary_inbound.untrusted);
    assert_eq!(INBOUND_ITEM_OBJECT, "inbound_item");
    assert!(ordinary_inbound.first_snapshot.content.is_none());
    assert!(ordinary_inbound.current_snapshot.content.is_none());
    assert_eq!(
        ordinary_inbound.last_recorded_remote.state,
        RemoteSnapshotState::Present
    );
    assert!(!ordinary_inbound.last_recorded_remote.current_remote_truth);
    assert_eq!(
        ordinary_inbound.last_recorded_remote.provenance,
        EvidenceSource::LastRecordedRemoteFetch
    );
    assert!(ordinary_inbound.local_state.acknowledged_at.is_some());
    assert!(ordinary_inbound.local_state.archived_at.is_some());
    assert!(ordinary_inbound.replied());

    let retained_inbound = inspector
        .inspect_inbound_item(REPOSITORY_A, ITEM_A, true)
        .expect("retained inbound projection");
    assert_eq!(
        retained_inbound
            .first_snapshot
            .content
            .as_ref()
            .map(|value| value.as_str()),
        Some(SECRET_BODY)
    );
    assert!(retained_inbound.current_snapshot.content.is_some());
    assert!(retained_inbound.retained_content_requested());

    let acknowledgement = inspector
        .inspect_acknowledgement(REPOSITORY_A, ITEM_A)
        .expect("acknowledgement projection");
    assert!(acknowledgement.local_only);
    assert_eq!(ACKNOWLEDGEMENT_OBJECT, "acknowledgement");
    assert!(acknowledgement.remote_effect.is_none());

    let archive = inspector
        .inspect_archive(REPOSITORY_A, ITEM_A)
        .expect("archive projection");
    assert!(archive.local_only);
    assert_eq!(ARCHIVE_OBJECT, "archive");
    assert!(archive.remote_effect.is_none());

    let reply = inspector
        .inspect_reply_link(REPOSITORY_A, ITEM_A)
        .expect("reply link projection");
    assert!(reply.replied);
    assert!(!reply.local_only_link);
    assert_eq!(REPLY_LINK_OBJECT, "reply_link");
    assert_eq!(reply.accepted_delivery_id.as_deref(), Some("attempt-reply"));
    assert_eq!(reply.evidence_status, ReplyClaimStatus::AcceptedEvidence);

    let audit = inspector
        .inspect_audit_transitions(&AuditFilter::new(REPOSITORY_A).with_page_size(2))
        .expect("audit projection");
    assert_eq!(audit.provenance, EvidenceSource::LocalState);
    assert_eq!(AUDIT_OBJECT, "audit_transition");
    assert!(!audit.remote_fetch_performed);
    assert_eq!(audit.page.repository_id, REPOSITORY_A);

    let unified = inspector
        .inspect(&InspectionRequest::inbound_item(REPOSITORY_A, ITEM_A))
        .expect("unified object inspection");
    assert!(matches!(unified, LifecycleProjection::InboundItem(_)));
    assert_eq!(unified.object_type(), INBOUND_ITEM_OBJECT);
}

#[test]
fn pagination_is_bounded_stable_and_scope_bound() {
    let (_temp, _path, mut store) = store_and_path();
    register(&mut store, REPOSITORY_A);
    register(&mut store, REPOSITORY_B);
    for index in 0..5 {
        add_draft(
            &mut store,
            REPOSITORY_A,
            &format!("draft-{index}"),
            "body",
            None,
        );
    }
    add_draft(&mut store, REPOSITORY_B, "draft-b", "other", None);
    for index in 0..3 {
        add_audit(
            &mut store,
            REPOSITORY_A,
            &format!("page-event-{index}"),
            "created",
        );
    }
    let inspector = LifecycleInspector::new(&store);

    let first = inspector
        .list_drafts(REPOSITORY_A, &PageRequest::new(2))
        .expect("first draft page");
    assert_eq!(first.items.len(), 2);
    assert!(first.truncated);
    assert_eq!(first.items[0].draft_id, "draft-0");
    assert_eq!(first.items[1].draft_id, "draft-1");
    let cursor = first.next_after.clone().expect("draft continuation");

    let second = inspector
        .list_drafts(REPOSITORY_A, &PageRequest::new(2).with_after(cursor))
        .expect("second draft page");
    assert_eq!(second.items[0].draft_id, "draft-2");
    assert_eq!(second.items[1].draft_id, "draft-3");
    assert!(second.truncated);
    let final_page = inspector
        .list_drafts(
            REPOSITORY_A,
            &PageRequest::new(2).with_after(second.next_after.expect("second continuation")),
        )
        .expect("final draft page");
    assert_eq!(final_page.items[0].draft_id, "draft-4");
    assert!(!final_page.truncated);
    assert!(final_page.next_after.is_none());

    let foreign_cursor = first.next_after.expect("scope-bound cursor");
    let cross_repository = inspector.list_drafts(
        REPOSITORY_B,
        &PageRequest::new(2).with_after(foreign_cursor),
    );
    assert_eq!(
        cross_repository,
        Err(LifecycleError::CrossRepositoryDenied {
            repository_id: REPOSITORY_B.to_owned()
        })
    );
    assert_eq!(
        inspector.list_drafts(REPOSITORY_A, &PageRequest::new(0)),
        Err(LifecycleError::InvalidPage { field: "limit" })
    );
    assert_eq!(
        inspector.list_drafts(REPOSITORY_A, &PageRequest::new(MAX_PAGE_SIZE + 1)),
        Err(LifecycleError::InvalidPage { field: "limit" })
    );

    let audit_filter = AuditFilter::new(REPOSITORY_A).with_page_size(2);
    let audit_page = inspector
        .inspect_audit_transitions(&audit_filter)
        .expect("first audit page");
    assert!(audit_page.page.truncated);
    let audit_cursor = audit_page.page.next_cursor.clone().expect("audit cursor");
    let audit_second = inspector
        .inspect_audit_transitions(&audit_filter.with_cursor(audit_cursor))
        .expect("second audit page");
    assert!(!audit_second.page.events.is_empty());
}

#[test]
fn cross_repository_object_access_is_denied_without_returning_foreign_content() {
    let (_temp, _path, mut store) = store_and_path();
    seed_complete(&mut store);
    let inspector = LifecycleInspector::new(&store);

    assert!(matches!(
        inspector.inspect_draft(REPOSITORY_B, "draft-out"),
        Ok(value) if value.metadata_json != SECRET_BODY
    ));
    assert!(matches!(
        inspector.inspect_inbound_item(REPOSITORY_B, ITEM_A, true),
        Err(LifecycleError::ObjectNotFound {
            object_type: INBOUND_ITEM_OBJECT,
            ..
        })
    ));
    assert!(matches!(
        inspector.inspect(&InspectionRequest::draft(REPOSITORY_B, "draft-out")),
        Ok(LifecycleProjection::Draft(_))
    ));

    let mut request = InspectionRequest::repository(REPOSITORY_A);
    request.object = LifecycleObject::AuditTransitions {
        filter: AuditFilter::new(REPOSITORY_B),
    };
    assert!(matches!(
        inspector.inspect(&request),
        Err(LifecycleError::CrossRepositoryDenied { .. })
    ));
}

#[test]
fn ordinary_diagnostics_redact_content_and_explicit_opt_in_is_debug_safe() {
    let (_temp, _path, mut store) = store_and_path();
    seed_complete(&mut store);
    let inspector = LifecycleInspector::new(&store);

    let ordinary_revision = inspector
        .inspect_draft_revision(REPOSITORY_A, "draft-out", 1, false)
        .expect("ordinary revision");
    let ordinary_json =
        serde_json::to_string(&ordinary_revision).expect("ordinary projection JSON");
    let ordinary_debug = format!("{ordinary_revision:?}");
    assert!(!ordinary_json.contains(SECRET_BODY));
    assert!(!ordinary_json.contains(SECRET_TOKEN));
    assert!(!ordinary_debug.contains(SECRET_BODY));
    assert!(!ordinary_debug.contains(SECRET_TOKEN));
    assert!(ordinary_revision.metadata_json.contains(REDACTED));

    let retained = inspector
        .inspect_draft_revision(REPOSITORY_A, "draft-out", 1, true)
        .expect("retained revision");
    let retained_debug = format!("{retained:?}");
    assert!(!retained_debug.contains(SECRET_BODY));
    assert!(!retained_debug.contains(SECRET_TOKEN));
    let retained_json = serde_json::to_string(&retained).expect("retained projection JSON");
    assert!(retained_json.contains(SECRET_BODY));

    let inbound = inspector
        .inspect_inbound_item(REPOSITORY_A, ITEM_A, false)
        .expect("ordinary inbound");
    let inbound_json = serde_json::to_string(&inbound).expect("inbound projection JSON");
    assert!(!inbound_json.contains(SECRET_BODY));
    let audit_page = inspector
        .inspect_audit_transitions(&AuditFilter::new(REPOSITORY_A))
        .expect("audit projection");
    let audit_json = serde_json::to_string(&audit_page.page.events).expect("audit JSON");
    assert!(!audit_json.contains(SECRET_BODY));
    assert!(!audit_json.contains(SECRET_TOKEN));
    assert!(audit_json.contains(REDACTED));
}

#[test]
fn verifier_reports_healthy_integrity_and_preserves_bytes() {
    let (_temp, path, mut store) = store_and_path();
    register(&mut store, REPOSITORY_A);
    add_draft(&mut store, REPOSITORY_A, "draft-a", SECRET_BODY, None);
    let reader = StateStore::open_path(&path).expect("open read-only lifecycle reader");
    let before = snapshot(&path);

    let report = StateVerifier::new().verify(&VerificationRequest::new(&path, REPOSITORY_A));
    assert!(report.healthy, "unexpected report: {report:?}");
    assert!(report.read_only);
    assert!(report.residual_risk().contains("not encryption at rest"));
    assert!(report.residual_risk().contains("backups"));
    assert!(report.connection_read_only);
    assert_eq!(report.quick_check.status, CheckStatus::Passed);
    assert!(report.quick_check_ok);
    assert!(report.foreign_keys_ok);
    assert!(report.migration_ok);
    assert!(report.repository_scope_ok);
    assert!(report.filesystem_permissions_ok);
    assert!(report.issues.is_empty());

    let inspector = LifecycleInspector::new(&reader);
    let _ = inspector
        .inspect_draft_revision(REPOSITORY_A, "draft-a", 1, false)
        .expect("read-only inspection");
    assert_snapshot_unchanged(&path, &before);
    drop(reader);
}

#[test]
fn verifier_reports_foreign_key_and_migration_failures_without_repair() {
    let (temp, path, mut store) = store_and_path();
    register(&mut store, REPOSITORY_A);
    drop(store);
    {
        let connection = Connection::open(&path).expect("open fixture connection");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO inbound_acknowledgements(repository_id, item_id, acknowledged_at)
                 VALUES ('repo-a', 'missing-item', '2026-01-01T00:00:00Z');",
            )
            .expect("inject foreign-key fixture");
    }
    let fk_report = verify_state(&path, REPOSITORY_A);
    assert!(!fk_report.healthy);
    assert!(!fk_report.foreign_keys_ok);
    assert!(fk_report.foreign_keys.violation_count > 0);
    assert!(
        fk_report
            .remediation
            .iter()
            .any(|value| value.contains("repair"))
    );

    let (_migration_temp, migration_path, mut migration_store) = store_and_path();
    register(&mut migration_store, REPOSITORY_A);
    drop(migration_store);
    {
        let connection = Connection::open(&migration_path).expect("open migration fixture");
        connection
            .execute_batch("PRAGMA user_version = 99")
            .expect("set future fixture version");
    }
    let migration_before = snapshot(&migration_path);
    let migration_report = verify_state(&migration_path, REPOSITORY_A);
    assert!(!migration_report.healthy);
    assert!(!migration_report.migration_ok);
    assert_eq!(migration_report.migration.found, Some(99));
    assert_eq!(migration_report.migration.expected, 1);
    assert_snapshot_unchanged(&migration_path, &migration_before);

    let wrong_scope = verify_state(&path, "repo-not-present");
    assert!(!wrong_scope.healthy);
    assert!(!wrong_scope.repository_scope_ok);
    assert!(!wrong_scope.issues.is_empty());

    let _ = temp;
}

#[test]
fn verifier_rejects_unsafe_permissions_and_preserves_failure_bytes() {
    let (temp, path, mut store) = store_and_path();
    register(&mut store, REPOSITORY_A);
    drop(store);
    let before = snapshot(&path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("make fixture permissions unsafe");
    }
    let report = verify_state(&path, REPOSITORY_A);
    assert!(!report.healthy);
    assert!(!report.filesystem_permissions_ok);
    assert!(!report.filesystem_permissions.file_user_only);
    assert_snapshot_unchanged(&path, &before);

    let corrupt_path = database_path_in(temp.path()).with_file_name("corrupt.sqlite3");
    let corrupt_bytes = b"deliberately not sqlite".to_vec();
    fs::write(&corrupt_path, &corrupt_bytes).expect("write corrupt fixture");
    let corrupt_report = verify_state(&corrupt_path, REPOSITORY_A);
    assert!(!corrupt_report.healthy);
    assert_eq!(
        fs::read(&corrupt_path).expect("corrupt bytes"),
        corrupt_bytes
    );
    assert!(corrupt_path.exists());

    let future_path = database_path_in(temp.path()).with_file_name("future.sqlite3");
    {
        let connection = Connection::open(&future_path).expect("create future fixture");
        connection
            .execute_batch("CREATE TABLE future(value TEXT); PRAGMA user_version = 99;")
            .expect("create future schema fixture");
    }
    let future_bytes = fs::read(&future_path).expect("future fixture bytes");
    let future_report = verify_state(&future_path, REPOSITORY_A);
    assert!(!future_report.healthy);
    assert_eq!(
        fs::read(&future_path).expect("future bytes after verify"),
        future_bytes
    );

    let missing_path = database_path_in(temp.path()).with_file_name("missing.sqlite3");
    let missing_report = verify_state(&missing_path, REPOSITORY_A);
    assert!(!missing_report.healthy);
    assert!(!missing_path.exists());
}

#[test]
fn public_boundaries_expose_no_migration_repair_backup_or_remote_operation() {
    let inspect_source = include_str!("../src/inspect.rs");
    let verify_source = include_str!("../src/verify.rs");
    let manifest = include_str!("../Cargo.toml");
    assert!(!inspect_source.contains("StateStore::open_path"));
    assert!(!inspect_source.contains("rusqlite_migration"));
    assert!(!verify_source.contains("apply_initial_migration"));
    assert!(!verify_source.contains("fs::write"));
    assert!(!verify_source.contains("set_permissions"));
    assert!(!manifest.contains("reqwest"));
    assert!(!manifest.contains("tokio"));
}
