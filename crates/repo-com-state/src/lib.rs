#![forbid(unsafe_code)]

pub mod migrations;
pub mod paths;
pub mod store;

#[cfg(test)]
#[path = "../tests/state_contract.rs"]
mod state_contract;

pub use migrations::{
    INITIAL_MIGRATION_SQL, MIGRATION_ARRAY, MIGRATIONS, MigrationError, MigrationReport,
    SCHEMA_VERSION, apply_initial_migration, current_schema_version,
};
pub use paths::{
    APPLICATION_DIRECTORY, DATABASE_FILE_NAME, PathError, PermissionInspection, PermissionModel,
    application_data_dir, create_user_only_database, database_path, database_path_in,
    inspect_user_only_database, is_user_only, resolve_application_data_dir, resolve_database_path,
    resolve_user_data_dir, state_database_path, user_data_root,
};
pub use store::{
    AcknowledgementRecord, ApprovalInput, ApprovalRecord, ApprovalRepository, ArchiveRecord,
    AuditEventInput, AuditEventRecord, AuditRepository, DEFAULT_BUSY_TIMEOUT_MS,
    DeliveryAttemptInput, DeliveryAttemptRecord, DeliveryAttemptRepository, DraftInput,
    DraftRecord, DraftRepository, DraftRevisionInput, DraftRevisionRecord,
    InboundCurrentSnapshotInput, InboundCurrentSnapshotRecord, InboundCursorInput,
    InboundCursorRecord, InboundItemInput, InboundItemRecord, InboundRepository,
    InboundTransitionInput, InboundTransitionRecord, IntegrityError, PolicyActivationInput,
    PolicyActivationRecord, PolicyActivationRepository, REQUIRED_SQLITE_VERSION,
    REQUIRED_SQLITE_VERSION_NUMBER, ReplyLinkInput, ReplyLinkRecord, Repositories, RepositoryInput,
    RepositoryRecord, RepositoryRepository, RepositoryScope, SqliteRuntime, StateError,
    StateResult, StateStore, StateStoreOptions, StateTransaction, StoreError,
    assert_runtime_sqlite_version, assert_sqlite_runtime, sqlite_runtime, sqlite_version,
};
