use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use rusqlite::Connection;

use crate::{
    AcknowledgementRecord, ApprovalInput, AuditEventInput, DeliveryAttemptInput, DraftInput,
    DraftRevisionInput, InboundCurrentSnapshotInput, InboundCursorInput, InboundItemInput,
    InboundTransitionInput, PolicyActivationInput, REQUIRED_SQLITE_VERSION, ReplyLinkInput,
    RepositoryInput, SCHEMA_VERSION, StateError, StateStore, assert_sqlite_runtime,
    database_path_in, is_user_only, sqlite_runtime,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "repo-com-state-contract-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary state directory");
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
            "2026-01-01T00:00:00Z",
        ))
        .expect("register repository");
}

fn create_draft_with_revision(
    store: &mut StateStore,
    repository_id: &str,
    draft_id: &str,
    body: &str,
) {
    store
        .create_draft(&DraftInput::new(
            repository_id,
            draft_id,
            "build_failed",
            "release",
            "2026-01-01T00:00:00Z",
        ))
        .expect("create draft");
    store
        .insert_draft_revision(&DraftRevisionInput::new(
            repository_id,
            draft_id,
            1,
            format!("hash-{repository_id}-{draft_id}"),
            body,
            "release",
            format!("channel-{repository_id}"),
            "2026-01-01T00:00:01Z",
        ))
        .expect("create revision");
}

#[test]
fn user_data_layout_and_permissions_are_platform_aware() {
    let root = crate::user_data_root().expect("OS user-data root must be available");
    assert!(root.is_absolute());
    assert!(
        root.join(crate::APPLICATION_DIRECTORY)
            .ends_with(crate::APPLICATION_DIRECTORY)
    );

    let (temp, path, _store) = store_and_path();
    assert!(path.starts_with(temp.path()));
    assert!(path.ends_with(crate::DATABASE_FILE_NAME));
    assert!(is_user_only(&path));
    let inspection = crate::inspect_user_only_database(&path).expect("inspect state file");
    assert!(inspection.user_only);
    assert_eq!(inspection.model, crate::PermissionModel::current());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file_mode = fs::metadata(&path)
            .expect("file metadata")
            .permissions()
            .mode();
        let directory_mode = fs::metadata(path.parent().expect("state directory"))
            .expect("directory metadata")
            .permissions()
            .mode();
        assert_eq!(file_mode & 0o077, 0);
        assert_eq!(directory_mode & 0o077, 0);
    }
}

#[test]
fn in_memory_store_uses_the_same_foreign_key_and_schema_boundary() {
    let store = StateStore::open_in_memory().expect("open in-memory store");
    assert!(store.foreign_keys_enabled().expect("foreign keys"));
    assert_eq!(
        store.schema_version().expect("schema version"),
        SCHEMA_VERSION
    );
    assert_eq!(store.migration_report().applied, 1);
}

#[test]
fn migration_applies_once_and_reopens_without_schema_recreation() {
    let (temp, path, store) = store_and_path();
    assert_eq!(
        store.schema_version().expect("schema version"),
        SCHEMA_VERSION
    );
    assert_eq!(store.migration_report().from_version, 0);
    assert_eq!(store.migration_report().to_version, SCHEMA_VERSION);
    assert_eq!(store.migration_report().applied, 1);
    assert_eq!(
        rusqlite_migration::Migrations::from_slice(crate::migrations::MIGRATION_ARRAY)
            .pending_migrations(store.connection())
            .expect("pending migrations"),
        0
    );

    let before_tables: Vec<String> = store
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare tables")
        .query_map([], |row| row.get(0))
        .expect("query tables")
        .collect::<Result<_, _>>()
        .expect("collect tables");
    assert!(before_tables.contains(&"repositories".to_owned()));
    assert!(before_tables.contains(&"draft_revisions".to_owned()));
    assert!(before_tables.contains(&"inbound_reply_links".to_owned()));
    assert!(before_tables.contains(&"audit_events".to_owned()));

    drop(store);
    let reopened = StateStore::open_path(&path).expect("reopen state store");
    assert_eq!(
        reopened.schema_version().expect("schema version"),
        SCHEMA_VERSION
    );
    assert_eq!(reopened.migration_report().from_version, SCHEMA_VERSION);
    assert_eq!(reopened.migration_report().to_version, SCHEMA_VERSION);
    assert_eq!(reopened.migration_report().applied, 0);
    assert_eq!(
        rusqlite_migration::Migrations::from_slice(crate::migrations::MIGRATION_ARRAY)
            .pending_migrations(reopened.connection())
            .expect("pending migrations"),
        0
    );
    assert!(temp.path().exists());
}

#[test]
fn concurrent_first_opens_apply_one_forward_migration_without_loss() {
    let temp = TempDir::new();
    let path = database_path_in(temp.path());
    let results = thread::scope(|scope| {
        let handles = (0..4)
            .map(|_| {
                let path = path.clone();
                scope.spawn(move || StateStore::open_path(path))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("opening thread"))
            .collect::<Vec<_>>()
    });
    assert!(results.iter().all(Result::is_ok));
    let verification = StateStore::open_path(&path).expect("verification store");
    assert_eq!(
        verification.schema_version().expect("schema version"),
        SCHEMA_VERSION
    );
}

#[test]
fn all_state_families_are_namespaced_by_repository() {
    let (_temp, _path, mut store) = store_and_path();
    register(&mut store, "repo-a");
    register(&mut store, "repo-b");
    create_draft_with_revision(&mut store, "repo-a", "shared-draft", "body-a");
    create_draft_with_revision(&mut store, "repo-b", "shared-draft", "body-b");

    assert_eq!(
        store
            .draft_revision("repo-a", "shared-draft", 1)
            .expect("read a revision")
            .expect("a revision")
            .body,
        "body-a"
    );
    assert_eq!(
        store
            .draft_revision("repo-b", "shared-draft", 1)
            .expect("read b revision")
            .expect("b revision")
            .body,
        "body-b"
    );
    assert!(
        store
            .draft_revision("repo-a", "missing-draft", 1)
            .expect("read missing revision")
            .is_none()
    );
    let wrong_scope =
        store.set_draft_status("repo-b", "missing-draft", "sent", "2026-01-01T00:00:01Z");
    assert!(matches!(wrong_scope, Err(StateError::NotFound { .. })));
    assert_eq!(
        store
            .draft("repo-a", "shared-draft")
            .expect("read a draft")
            .expect("draft")
            .status,
        "draft"
    );

    let approval = store
        .record_approval(&ApprovalInput::new(
            "repo-a",
            "approval-a",
            "shared-draft",
            1,
            "operator",
            "2026-01-01T00:00:02Z",
        ))
        .expect("record approval");
    assert_eq!(approval.repository_id, "repo-a");
    assert!(
        store
            .approval("repo-b", "shared-draft", 1)
            .expect("read cross approval")
            .is_none()
    );

    store
        .activate_policy(&PolicyActivationInput::new(
            "repo-a",
            "activation-a",
            "config-repo-a",
            "tuple-a",
            "build_failed",
            "release",
            "high",
            "2026-01-01T00:00:03Z",
        ))
        .expect("activate policy");
    assert!(
        store
            .policy_activation("repo-b", "activation-a")
            .expect("read cross activation")
            .is_none()
    );

    store
        .record_delivery_attempt(&DeliveryAttemptInput::new(
            "repo-a",
            "attempt-a",
            "shared-draft",
            1,
            1,
            "nonce-a",
            "2026-01-01T00:00:04Z",
        ))
        .expect("record attempt");
    assert!(
        store
            .delivery_attempt("repo-b", "attempt-a")
            .expect("read cross attempt")
            .is_none()
    );

    let item = InboundItemInput::new(
        "repo-a",
        "inbound-a",
        "channel-a",
        "human-a",
        "first",
        "2026-01-01T00:00:05Z",
    );
    let current = InboundCurrentSnapshotInput::new(
        "repo-a",
        "inbound-a",
        Some("first".to_owned()),
        false,
        "2026-01-01T00:00:05Z",
    );
    store
        .store_inbound_item(&item, &current)
        .expect("store inbound item");
    assert!(
        store
            .inbound_item("repo-b", "inbound-a")
            .expect("read cross inbound")
            .is_none()
    );
    assert!(
        store
            .inbound_cursor("repo-b", "release")
            .expect("read cross cursor")
            .is_none()
    );

    store
        .advance_inbound_cursor(&InboundCursorInput::new(
            "repo-a",
            "release",
            "100",
            "2026-01-01T00:00:06Z",
        ))
        .expect("advance cursor");
    assert_eq!(
        store
            .inbound_cursor("repo-a", "release")
            .expect("read cursor")
            .expect("cursor")
            .cursor,
        "100"
    );

    let acknowledged = store
        .acknowledge_inbound("repo-a", &["inbound-a"], "2026-01-01T00:00:07Z")
        .expect("acknowledge item");
    assert!(matches!(
        acknowledged.as_slice(),
        [AcknowledgementRecord { .. }]
    ));
    assert!(
        store
            .repositories()
            .inbound()
            .acknowledgement("repo-b", "inbound-a")
            .expect("read cross acknowledgement")
            .is_none()
    );

    store
        .archive_inbound("repo-a", &["inbound-a"], "2026-01-01T00:00:08Z")
        .expect("archive item");
    assert!(
        store
            .repositories()
            .inbound()
            .archive_record("repo-b", "inbound-a")
            .expect("read cross archive")
            .is_none()
    );

    let reply = store
        .create_draft(&DraftInput::new(
            "repo-a",
            "reply-a",
            "inbound_reply",
            "release",
            "2026-01-01T00:00:09Z",
        ))
        .expect("create reply draft");
    assert_eq!(reply.repository_id, "repo-a");
    store
        .link_inbound_reply(&ReplyLinkInput::new(
            "repo-a",
            "inbound-a",
            "reply-a",
            "2026-01-01T00:00:10Z",
        ))
        .expect("link reply");
    assert!(
        store
            .repositories()
            .inbound()
            .reply_link("repo-b", "inbound-a")
            .expect("read cross reply link")
            .is_none()
    );

    store
        .append_audit_event(&AuditEventInput::new(
            "repo-a",
            "event-a",
            "draft",
            "shared-draft",
            "created",
            "2026-01-01T00:00:11Z",
            "test",
            "success",
        ))
        .expect("append audit");
    assert!(
        store
            .audit_event("repo-b", "event-a")
            .expect("read cross audit")
            .is_none()
    );
}

#[test]
fn first_snapshots_and_audit_events_are_immutable_at_the_database_boundary() {
    let (_temp, _path, mut store) = store_and_path();
    register(&mut store, "repo-a");
    create_draft_with_revision(&mut store, "repo-a", "draft-a", "immutable");
    let item = InboundItemInput::new(
        "repo-a",
        "item-a",
        "channel-a",
        "human-a",
        "first",
        "2026-01-01T00:00:00Z",
    );
    let current = InboundCurrentSnapshotInput::new(
        "repo-a",
        "item-a",
        Some("first".to_owned()),
        false,
        "2026-01-01T00:00:00Z",
    );
    store
        .store_inbound_item(&item, &current)
        .expect("store item");
    store
        .append_audit_event(&AuditEventInput::new(
            "repo-a",
            "event-a",
            "inbound",
            "item-a",
            "created",
            "2026-01-01T00:00:01Z",
            "test",
            "success",
        ))
        .expect("append event");

    assert!(
        store
            .connection()
            .execute(
                "UPDATE draft_revisions SET body = 'changed' WHERE repository_id = ?1",
                ["repo-a"],
            )
            .is_err()
    );
    assert!(
        store
            .connection()
            .execute(
                "UPDATE inbound_items SET first_content = 'changed' WHERE repository_id = ?1",
                ["repo-a"],
            )
            .is_err()
    );
    assert!(
        store
            .connection()
            .execute("UPDATE audit_events SET outcome = 'changed'", [])
            .is_err()
    );
    assert!(
        store
            .connection()
            .execute(
                "DELETE FROM audit_events WHERE repository_id = ?1",
                ["repo-a"]
            )
            .is_err()
    );
    assert_eq!(
        store
            .draft_revision("repo-a", "draft-a", 1)
            .expect("read immutable revision")
            .expect("revision")
            .body,
        "immutable"
    );
    assert_eq!(
        store
            .inbound_item("repo-a", "item-a")
            .expect("read immutable inbound")
            .expect("item")
            .first_content,
        "first"
    );
}

#[test]
fn explicit_failure_rolls_back_state_and_audit_together() {
    let (_temp, _path, mut store) = store_and_path();
    let result = store.with_transaction(|transaction| {
        transaction
            .repositories()
            .repositories()
            .upsert(&RepositoryInput::new(
                "repo-a",
                "workspace-a",
                "config-a",
                "2026-01-01T00:00:00Z",
            ))?;
        transaction
            .repositories()
            .audit()
            .append(&AuditEventInput::new(
                "repo-a",
                "event-a",
                "repository",
                "repo-a",
                "created",
                "2026-01-01T00:00:01Z",
                "test",
                "success",
            ))?;
        Err::<(), _>(StateError::Transaction {
            message: "injected failure".to_owned(),
        })
    });
    assert!(matches!(result, Err(StateError::Transaction { .. })));
    assert!(
        store
            .repository("repo-a")
            .expect("read rolled back repository")
            .is_none()
    );
    assert!(
        store
            .audit_event("repo-a", "event-a")
            .expect("read rolled back audit")
            .is_none()
    );
}

#[test]
fn wal_readers_see_committed_state_while_writer_is_open_and_busy_timeout_is_bounded() {
    let temp = TempDir::new();
    let path = database_path_in(temp.path());
    let mut writer = StateStore::open_path(&path).expect("open writer");
    let reader = StateStore::open_path(&path).expect("open reader");
    register(&mut writer, "repo-a");
    assert_eq!(writer.journal_mode().expect("journal mode"), "wal");
    assert!(writer.foreign_keys_enabled().expect("foreign keys"));

    let transaction = writer
        .begin_transaction()
        .expect("begin writer transaction");
    transaction
        .repositories()
        .repositories()
        .upsert(&RepositoryInput::new(
            "repo-a",
            "workspace-updated",
            "config-updated",
            "2026-01-01T00:00:01Z",
        ))
        .expect("update in transaction");
    assert_eq!(
        reader
            .repository("repo-a")
            .expect("WAL reader")
            .expect("repository")
            .workspace_id,
        "workspace-repo-a"
    );

    let competing_path = path.clone();
    let result = thread::spawn(move || {
        let mut competitor = StateStore::open_path(competing_path).expect("open competitor");
        competitor.upsert_repository(&RepositoryInput::new(
            "repo-b",
            "workspace-b",
            "config-b",
            "2026-01-01T00:00:02Z",
        ))
    })
    .join()
    .expect("competing thread");
    assert!(matches!(result, Err(StateError::LockTimeout { .. })));
    transaction.rollback().expect("rollback writer");

    register(&mut writer, "repo-b");
    let all = thread::scope(|scope| {
        let handles = (0..4)
            .map(|index| {
                let path = path.clone();
                scope.spawn(move || {
                    let mut store = StateStore::open_path(path).expect("open concurrent writer");
                    store.upsert_repository(&RepositoryInput::new(
                        format!("repo-{index}"),
                        "workspace",
                        "config",
                        "2026-01-01T00:00:03Z",
                    ))
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("concurrent writer thread"))
            .collect::<Vec<_>>()
    });
    assert!(all.iter().all(Result::is_ok));
    let verification = StateStore::open_path(&path).expect("verification store");
    for index in 0..4 {
        assert!(
            verification
                .repository(format!("repo-{index}"))
                .expect("read committed repository")
                .is_some()
        );
    }
}

#[test]
fn corrupt_and_unsupported_databases_are_typed_and_byte_preserving() {
    let corrupt_temp = TempDir::new();
    let corrupt_path = database_path_in(corrupt_temp.path());
    fs::create_dir_all(corrupt_path.parent().expect("corrupt parent"))
        .expect("create corrupt parent");
    let corrupt_bytes = b"this is deliberately not a SQLite database".to_vec();
    fs::write(&corrupt_path, &corrupt_bytes).expect("write corrupt bytes");
    let corrupt_error =
        StateStore::open_path(&corrupt_path).expect_err("corrupt database must fail");
    assert!(matches!(corrupt_error, StateError::CorruptDatabase { .. }));
    assert_eq!(
        fs::read(&corrupt_path).expect("read corrupt bytes"),
        corrupt_bytes
    );

    let unsupported_temp = TempDir::new();
    let unsupported_path = database_path_in(unsupported_temp.path());
    fs::create_dir_all(unsupported_path.parent().expect("unsupported parent"))
        .expect("create unsupported parent");
    {
        let connection = Connection::open(&unsupported_path).expect("create unsupported database");
        connection
            .execute("CREATE TABLE future_state(value TEXT)", [])
            .expect("create future table");
        connection
            .execute_batch("PRAGMA user_version = 99")
            .expect("set future version");
    }
    let unsupported_bytes = fs::read(&unsupported_path).expect("read unsupported bytes");
    let unsupported_error =
        StateStore::open_path(&unsupported_path).expect_err("unsupported database must fail");
    assert!(matches!(
        unsupported_error,
        StateError::UnsupportedSchema {
            found: 99,
            supported: 1,
            ..
        }
    ));
    assert_eq!(
        fs::read(&unsupported_path).expect("read unsupported bytes after failure"),
        unsupported_bytes
    );

    let migration_temp = TempDir::new();
    let migration_path = database_path_in(migration_temp.path());
    fs::create_dir_all(migration_path.parent().expect("migration parent"))
        .expect("create migration parent");
    {
        let connection =
            Connection::open(&migration_path).expect("create migration conflict database");
        connection
            .execute("CREATE TABLE repositories(conflicting_column TEXT)", [])
            .expect("create conflicting table");
    }
    let migration_bytes = fs::read(&migration_path).expect("read migration conflict bytes");
    let migration_error =
        StateStore::open_path(&migration_path).expect_err("migration conflict must fail");
    assert!(
        matches!(migration_error, StateError::MigrationFailed { .. }),
        "unexpected migration error: {migration_error:?}"
    );
    assert_eq!(
        fs::read(&migration_path).expect("read migration conflict bytes after failure"),
        migration_bytes
    );
}

#[test]
fn runtime_boundary_reports_the_linked_version_and_release_floor() {
    let runtime = sqlite_runtime();
    assert!(!runtime.version.is_empty());
    let result = assert_sqlite_runtime();
    if runtime.version_number >= crate::REQUIRED_SQLITE_VERSION_NUMBER {
        assert!(result.is_ok());
    } else {
        assert!(matches!(
            result,
            Err(StateError::UnsupportedSqliteRuntime { .. })
        ));
    }
    assert_eq!(REQUIRED_SQLITE_VERSION, "3.53.4");
}

#[test]
fn inbound_current_and_local_lifecycle_changes_are_idempotent() {
    let (_temp, _path, mut store) = store_and_path();
    register(&mut store, "repo-a");
    let item = InboundItemInput::new(
        "repo-a",
        "item-a",
        "channel-a",
        "human-a",
        "first",
        "2026-01-01T00:00:00Z",
    );
    let current = InboundCurrentSnapshotInput::new(
        "repo-a",
        "item-a",
        Some("first".to_owned()),
        false,
        "2026-01-01T00:00:00Z",
    );
    store
        .store_inbound_item(&item, &current)
        .expect("store item");

    store
        .record_inbound_transition(
            &InboundTransitionInput::new(
                "repo-a",
                "edit-1",
                "item-a",
                "edited",
                Some("edited".to_owned()),
                "2026-01-01T00:00:01Z",
            ),
            Some(&InboundCurrentSnapshotInput::new(
                "repo-a",
                "item-a",
                Some("edited".to_owned()),
                false,
                "2026-01-01T00:00:01Z",
            )),
        )
        .expect("record edit");
    assert_eq!(
        store
            .inbound_item("repo-a", "item-a")
            .expect("read first")
            .expect("item")
            .first_content,
        "first"
    );
    assert_eq!(
        store
            .repositories()
            .inbound()
            .current("repo-a", "item-a")
            .expect("read current")
            .expect("current")
            .current_content
            .as_deref(),
        Some("edited")
    );

    store
        .record_inbound_transition(
            &InboundTransitionInput::new(
                "repo-a",
                "delete-1",
                "item-a",
                "deleted",
                None,
                "2026-01-01T00:00:02Z",
            ),
            None,
        )
        .expect("record deletion");
    assert!(
        store
            .repositories()
            .inbound()
            .current("repo-a", "item-a")
            .expect("read deletion")
            .expect("current")
            .deleted
    );

    let first_ack = store
        .acknowledge_inbound("repo-a", &["item-a"], "2026-01-01T00:00:03Z")
        .expect("first acknowledgement");
    let second_ack = store
        .acknowledge_inbound("repo-a", &["item-a"], "2026-01-01T00:00:04Z")
        .expect("repeat acknowledgement");
    assert_eq!(first_ack[0].acknowledged_at, second_ack[0].acknowledged_at);
    let first_archive = store
        .archive_inbound("repo-a", &["item-a"], "2026-01-01T00:00:05Z")
        .expect("first archive");
    let second_archive = store
        .archive_inbound("repo-a", &["item-a"], "2026-01-01T00:00:06Z")
        .expect("repeat archive");
    assert_eq!(first_archive[0].archived_at, second_archive[0].archived_at);
    assert!(
        store
            .advance_inbound_cursor(&InboundCursorInput::new(
                "repo-a",
                "release",
                "100",
                "2026-01-01T00:00:07Z",
            ))
            .is_ok()
    );
    assert!(
        store
            .advance_inbound_cursor(&InboundCursorInput::new(
                "repo-a",
                "release",
                "99",
                "2026-01-01T00:00:08Z",
            ))
            .is_err()
    );
}

#[test]
fn release_open_asserts_before_creating_or_mutating_state() {
    let temp = TempDir::new();
    let path = database_path_in(temp.path());
    let result = StateStore::open_path_for_release(&path);
    if result.is_ok() {
        assert!(path.exists());
    } else {
        assert!(matches!(
            result,
            Err(StateError::UnsupportedSqliteRuntime { .. })
        ));
        assert!(!path.exists());
    }
}

#[test]
fn migration_sql_contains_all_v1_families_and_forward_only_version_marker() {
    let sql = crate::INITIAL_MIGRATION_SQL;
    for table in [
        "repositories",
        "drafts",
        "draft_revisions",
        "approvals",
        "policy_activations",
        "delivery_attempts",
        "inbound_cursors",
        "inbound_items",
        "inbound_current_snapshots",
        "inbound_item_transitions",
        "inbound_acknowledgements",
        "inbound_archives",
        "inbound_reply_links",
        "audit_events",
    ] {
        assert!(sql.contains(table), "missing state family {table}");
    }
    assert!(!sql.contains("DROP TABLE"));
    assert!(!sql.contains("DOWN"));
    assert!(crate::MIGRATIONS.validate().is_ok());
}
