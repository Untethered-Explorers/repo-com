use std::error::Error;
use std::fmt;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::Duration;

use repo_com_foundation::RepoComError;
use rusqlite::{
    Connection, Error as RusqliteError, ErrorCode, OpenFlags, OptionalExtension, Transaction,
    TransactionBehavior, params,
};

use crate::migrations::{
    MigrationError, MigrationReport, apply_initial_migration, current_schema_version,
};
use crate::paths::{PathError, create_user_only_database, database_path};

/// The bounded busy timeout used by newly opened state connections.
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 250;
/// The minimum SQLite version accepted by a release artifact.
pub const REQUIRED_SQLITE_VERSION: &str = "3.53.4";
/// The numeric form of [`REQUIRED_SQLITE_VERSION`] used by SQLite's API.
pub const REQUIRED_SQLITE_VERSION_NUMBER: i32 = 3_053_004;

/// A typed state-layer result.
pub type StateResult<T> = Result<T, StateError>;

/// Compatibility name for the typed state error.
pub type StoreError = StateError;

/// Compatibility name for typed non-destructive integrity failures.
pub type IntegrityError = StateError;

/// A safe, typed state-store failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    /// The OS user-data path could not be resolved or secured.
    Path(PathError),
    /// A required identifier was empty or malformed.
    InvalidIdentifier {
        /// The identifier family.
        kind: &'static str,
    },
    /// A repository identity must exist before repository-scoped state is
    /// written.
    RepositoryNotFound {
        /// The requested repository ID.
        repository_id: String,
    },
    /// A scoped object was not visible in the requested repository.
    NotFound {
        /// The state family.
        entity: &'static str,
        /// The requested repository ID.
        repository_id: String,
        /// The requested object ID.
        object_id: String,
    },
    /// A database constraint rejected a state mutation.
    Constraint {
        /// The state family or operation.
        entity: &'static str,
        /// A safe database diagnostic.
        message: String,
    },
    /// SQLite detected a corrupt database.  The original file is retained.
    CorruptDatabase {
        /// Database path, when the store is file-backed.
        path: Option<PathBuf>,
        /// Safe integrity diagnostic.
        message: String,
    },
    /// The database was created by an unsupported future schema.
    UnsupportedSchema {
        /// Database path, when the store is file-backed.
        path: Option<PathBuf>,
        /// Found `user_version`.
        found: i64,
        /// Version supported by this binary.
        supported: i64,
    },
    /// A forward migration failed and was rolled back.
    MigrationFailed {
        /// Database path, when the store is file-backed.
        path: Option<PathBuf>,
        /// Safe migration diagnostic.
        message: String,
    },
    /// A busy/locked operation exhausted the configured timeout.
    LockTimeout {
        /// Database path, when the store is file-backed.
        path: Option<PathBuf>,
        /// Operation that timed out.
        operation: &'static str,
        /// Configured timeout in milliseconds.
        timeout_ms: u64,
    },
    /// The linked SQLite runtime is older than the release requirement.
    UnsupportedSqliteRuntime {
        /// Minimum required version.
        required: String,
        /// Version reported by the linked library.
        found: String,
    },
    /// A transaction could not be completed.
    Transaction {
        /// Safe transaction diagnostic.
        message: String,
    },
    /// An ordinary SQLite operation failed.
    Sqlite {
        /// Database path, when the store is file-backed.
        path: Option<PathBuf>,
        /// Operation being attempted.
        operation: &'static str,
        /// Safe SQLite diagnostic.
        message: String,
    },
}

impl StateError {
    /// Returns whether this is one of the non-destructive integrity failures.
    #[must_use]
    pub const fn is_integrity_failure(&self) -> bool {
        matches!(
            self,
            Self::CorruptDatabase { .. }
                | Self::UnsupportedSchema { .. }
                | Self::MigrationFailed { .. }
                | Self::LockTimeout { .. }
                | Self::UnsupportedSqliteRuntime { .. }
        )
    }

    /// Returns a stable, non-sensitive error category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Path(_) | Self::InvalidIdentifier { .. } | Self::NotFound { .. } => "state-usage",
            Self::RepositoryNotFound { .. } => "repository-not-found",
            Self::Constraint { .. }
            | Self::CorruptDatabase { .. }
            | Self::UnsupportedSchema { .. }
            | Self::MigrationFailed { .. }
            | Self::LockTimeout { .. }
            | Self::UnsupportedSqliteRuntime { .. }
            | Self::Transaction { .. }
            | Self::Sqlite { .. } => "storage-integrity",
        }
    }

    /// Converts this error to the foundation's safe storage error category.
    #[must_use]
    pub fn to_repo_com_error(&self) -> RepoComError {
        RepoComError::storage_integrity(self.to_string())
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => write!(formatter, "{error}"),
            Self::InvalidIdentifier { kind } => write!(formatter, "invalid {kind} identifier"),
            Self::RepositoryNotFound { repository_id } => {
                write!(formatter, "repository is not registered: {repository_id}")
            }
            Self::NotFound {
                entity,
                repository_id,
                object_id,
            } => write!(
                formatter,
                "{entity} not found in repository {repository_id}: {object_id}"
            ),
            Self::Constraint { entity, message } => {
                write!(formatter, "{entity} constraint failed: {message}")
            }
            Self::CorruptDatabase { path, message } => write!(
                formatter,
                "SQLite database is corrupt{}: {message}",
                path_suffix(path.as_deref())
            ),
            Self::UnsupportedSchema {
                path,
                found,
                supported,
            } => write!(
                formatter,
                "unsupported SQLite schema version {found}; expected {supported}{}",
                path_suffix(path.as_deref())
            ),
            Self::MigrationFailed { path, message } => write!(
                formatter,
                "SQLite migration failed{}: {message}",
                path_suffix(path.as_deref())
            ),
            Self::LockTimeout {
                path,
                operation,
                timeout_ms,
            } => write!(
                formatter,
                "SQLite {operation} timed out after {timeout_ms} ms{}",
                path_suffix(path.as_deref())
            ),
            Self::UnsupportedSqliteRuntime { required, found } => write!(
                formatter,
                "SQLite runtime {found} is unsupported; release state requires {required} or newer"
            ),
            Self::Transaction { message } => {
                write!(formatter, "SQLite transaction failed: {message}")
            }
            Self::Sqlite {
                path,
                operation,
                message,
            } => write!(
                formatter,
                "SQLite {operation} failed{}: {message}",
                path_suffix(path.as_deref())
            ),
        }
    }
}

impl Error for StateError {}

impl From<StateError> for RepoComError {
    fn from(error: StateError) -> Self {
        error.to_repo_com_error()
    }
}

impl From<PathError> for StateError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

impl From<MigrationError> for StateError {
    fn from(error: MigrationError) -> Self {
        match error {
            MigrationError::UnsupportedSchema { found, supported } => Self::UnsupportedSchema {
                path: None,
                found,
                supported,
            },
            MigrationError::ApplyFailed { message, .. }
            | MigrationError::VerificationFailed { message }
            | MigrationError::VersionRead { message }
            | MigrationError::Sqlite { message, .. } => Self::MigrationFailed {
                path: None,
                message,
            },
            MigrationError::InvalidVersion(version) => Self::MigrationFailed {
                path: None,
                message: format!("invalid schema version {version}"),
            },
        }
    }
}

fn path_suffix(path: Option<&Path>) -> String {
    path.map_or_else(String::new, |value| format!(" ({})", value.display()))
}

/// The linked SQLite runtime and its release-policy check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteRuntime {
    /// Version string returned by SQLite.
    pub version: String,
    /// Numeric version returned by SQLite.
    pub version_number: i32,
}

impl SqliteRuntime {
    /// Returns whether the runtime meets the release minimum.
    #[must_use]
    pub const fn meets_release_requirement(&self) -> bool {
        self.version_number >= REQUIRED_SQLITE_VERSION_NUMBER
    }
}

/// Returns the linked SQLite version string.
#[must_use]
pub fn sqlite_version() -> String {
    rusqlite::version().to_owned()
}

/// Returns the linked SQLite runtime information.
#[must_use]
pub fn sqlite_runtime() -> SqliteRuntime {
    SqliteRuntime {
        version: rusqlite::version().to_owned(),
        version_number: rusqlite::version_number(),
    }
}

/// Asserts that the linked SQLite runtime is suitable for a release artifact.
///
/// Development and contract tests can use [`StateStore::open_path`] without
/// this strict gate when the local development SQLite is newer than the
/// bundled baseline but still below the eventual release floor.  Release code
/// must call this function (or [`StateStore::open_path_for_release`]) before
/// accepting operational state.
pub fn assert_sqlite_runtime() -> StateResult<SqliteRuntime> {
    let runtime = sqlite_runtime();
    if runtime.meets_release_requirement() {
        Ok(runtime)
    } else {
        Err(StateError::UnsupportedSqliteRuntime {
            required: REQUIRED_SQLITE_VERSION.to_owned(),
            found: runtime.version,
        })
    }
}

/// Compatibility alias for [`assert_sqlite_runtime`].
pub fn assert_runtime_sqlite_version() -> StateResult<SqliteRuntime> {
    assert_sqlite_runtime()
}

/// Options controlling one state-store connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateStoreOptions {
    /// Busy timeout applied to the connection.
    pub busy_timeout: Duration,
    /// Whether the release SQLite minimum is enforced while opening.
    pub require_release_sqlite: bool,
}

impl Default for StateStoreOptions {
    fn default() -> Self {
        Self {
            busy_timeout: Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            require_release_sqlite: false,
        }
    }
}

impl StateStoreOptions {
    /// Creates development/default options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a bounded busy timeout.
    #[must_use]
    pub const fn with_busy_timeout(mut self, timeout: Duration) -> Self {
        self.busy_timeout = timeout;
        self
    }

    /// Enables the strict release SQLite runtime gate.
    #[must_use]
    pub const fn require_release_sqlite(mut self, required: bool) -> Self {
        self.require_release_sqlite = required;
        self
    }

    /// Enables the strict release SQLite runtime gate.
    #[must_use]
    pub const fn release(mut self) -> Self {
        self.require_release_sqlite = true;
        self
    }
}

/// A byte snapshot used only to put a failed, non-destructive open back at its
/// exact pre-open image.  It is never used to repair or migrate a database.
struct DatabaseFileSnapshot {
    main: Option<Vec<u8>>,
    wal: Option<Vec<u8>>,
    shm: Option<Vec<u8>>,
}

impl DatabaseFileSnapshot {
    fn capture(path: &Path) -> Option<Self> {
        Some(Self {
            main: read_existing(path).ok()?,
            wal: read_existing(&sidecar_path(path, "-wal")).ok()?,
            shm: read_existing(&sidecar_path(path, "-shm")).ok()?,
        })
    }

    fn restore(&self, path: &Path) {
        restore_existing(path, self.main.as_deref());
        restore_existing(&sidecar_path(path, "-wal"), self.wal.as_deref());
        restore_existing(&sidecar_path(path, "-shm"), self.shm.as_deref());
    }
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

fn read_existing(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn restore_existing(path: &Path, bytes: Option<&[u8]>) {
    match bytes {
        Some(bytes) => {
            let _ = fs::write(path, bytes);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

/// One local SQLite state store.
///
/// A store owns one connection.  Callers that need concurrent work should open
/// additional stores against the same path; SQLite WAL and the bounded busy
/// timeout then provide the connection-level concurrency boundary.
pub struct StateStore {
    connection: Connection,
    path: Option<PathBuf>,
    runtime: SqliteRuntime,
    busy_timeout: Duration,
    migration: MigrationReport,
    require_release_sqlite: bool,
}

impl fmt::Debug for StateStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateStore")
            .field("path", &self.path)
            .field("runtime", &self.runtime)
            .field("busy_timeout", &self.busy_timeout)
            .field("require_release_sqlite", &self.require_release_sqlite)
            .finish()
    }
}

impl StateStore {
    /// Opens the OS user-data database.
    ///
    /// Release builds use the strict SQLite runtime gate automatically.  Debug
    /// contract tests may use the newer local development engine while still
    /// exercising the explicit `open_for_release` boundary separately.
    pub fn open() -> StateResult<Self> {
        let path = database_path()?;
        let options = StateStoreOptions {
            require_release_sqlite: !cfg!(debug_assertions),
            ..StateStoreOptions::default()
        };
        Self::open_path_with_options(path, options)
    }

    /// Opens an explicitly selected database path.
    pub fn open_path(path: impl AsRef<Path>) -> StateResult<Self> {
        Self::open_path_with_options(path, StateStoreOptions::default())
    }

    /// Compatibility alias for [`StateStore::open_path`].
    pub fn open_with_path(path: impl AsRef<Path>) -> StateResult<Self> {
        Self::open_path(path)
    }

    /// Opens an explicit path using the conventional constructor name.
    pub fn new(path: impl AsRef<Path>) -> StateResult<Self> {
        Self::open_path(path)
    }

    /// Compatibility alias for [`StateStore::open`].
    pub fn open_default() -> StateResult<Self> {
        Self::open()
    }

    /// Opens the default path and enforces the release SQLite minimum.
    pub fn open_for_release() -> StateResult<Self> {
        Self::open_path_for_release(database_path()?)
    }

    /// Opens an explicit path and enforces the release SQLite minimum.
    pub fn open_path_for_release(path: impl AsRef<Path>) -> StateResult<Self> {
        assert_sqlite_runtime()?;
        Self::open_path_with_options(path, StateStoreOptions::default().release())
    }

    /// Compatibility alias for [`StateStore::open_for_release`].
    pub fn open_release() -> StateResult<Self> {
        Self::open_for_release()
    }

    /// Compatibility alias for [`StateStore::open_path_for_release`].
    pub fn open_path_release(path: impl AsRef<Path>) -> StateResult<Self> {
        Self::open_path_for_release(path)
    }

    /// Opens a path with explicit store options.
    pub fn open_path_with_options(
        path: impl AsRef<Path>,
        options: StateStoreOptions,
    ) -> StateResult<Self> {
        if options.require_release_sqlite {
            assert_sqlite_runtime()?;
        }
        let path = path.as_ref().to_path_buf();
        let snapshot = DatabaseFileSnapshot::capture(&path);
        create_user_only_database(&path)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = match Connection::open_with_flags(&path, flags) {
            Ok(connection) => connection,
            Err(error) => {
                let mapped =
                    map_sqlite_error(Some(&path), "open database", options.busy_timeout, error);
                if should_restore_bytes(&mapped)
                    && let Some(snapshot) = snapshot.as_ref()
                {
                    snapshot.restore(&path);
                }
                return Err(mapped);
            }
        };
        let result = Self::from_open_connection(connection, Some(path.clone()), options);
        if result.as_ref().is_err_and(should_restore_bytes)
            && let Some(snapshot) = snapshot.as_ref()
        {
            snapshot.restore(&path);
        }
        result
    }

    /// Opens an in-memory database, primarily for focused contract tests.
    pub fn open_in_memory() -> StateResult<Self> {
        Self::open_in_memory_with_options(StateStoreOptions::default())
    }

    /// Opens an in-memory database with explicit options.
    pub fn open_in_memory_with_options(options: StateStoreOptions) -> StateResult<Self> {
        let connection = Connection::open_in_memory().map_err(|error| {
            map_sqlite_error(None, "open in-memory database", options.busy_timeout, error)
        })?;
        Self::from_open_connection(connection, None, options)
    }

    /// Reopens a file-backed store using the same path.
    pub fn reopen(&self) -> StateResult<Self> {
        match &self.path {
            Some(path) => Self::open_path_with_options(
                path,
                StateStoreOptions {
                    busy_timeout: self.busy_timeout,
                    require_release_sqlite: self.require_release_sqlite,
                },
            ),
            None => Err(StateError::Transaction {
                message: "an in-memory store cannot be reopened".to_owned(),
            }),
        }
    }

    fn from_open_connection(
        mut connection: Connection,
        path: Option<PathBuf>,
        options: StateStoreOptions,
    ) -> StateResult<Self> {
        configure_connection(&connection, path.as_deref(), options)?;
        let migration = apply_initial_migration(&mut connection)
            .map_err(|error| map_migration_error(error, path.as_deref()))?;
        enable_wal(&connection, path.as_deref())?;
        verify_connection_settings(&connection, path.as_deref(), options.busy_timeout)?;
        let runtime = if options.require_release_sqlite {
            assert_sqlite_runtime()?
        } else {
            sqlite_runtime()
        };
        Ok(Self {
            connection,
            path,
            runtime,
            busy_timeout: options.busy_timeout,
            migration,
            require_release_sqlite: options.require_release_sqlite,
        })
    }

    /// Returns the path for a file-backed store.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns the linked SQLite runtime observed when opening this store.
    #[must_use]
    pub const fn runtime(&self) -> &SqliteRuntime {
        &self.runtime
    }

    /// Returns the migration result observed when opening this store.
    #[must_use]
    pub const fn migration_report(&self) -> MigrationReport {
        self.migration
    }

    /// Returns the connection for read-only repository access.
    ///
    /// Mutating convenience methods below always use an explicit transaction.
    /// The connection is exposed so dependent state/audit crates can compose
    /// their own transaction-aware repository calls.
    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Returns the configured busy timeout.
    #[must_use]
    pub const fn busy_timeout(&self) -> Duration {
        self.busy_timeout
    }

    fn sqlite_error(&self, operation: &'static str, error: RusqliteError) -> StateError {
        map_sqlite_error(self.path.as_deref(), operation, self.busy_timeout, error)
    }

    /// Returns the current schema version.
    pub fn schema_version(&self) -> StateResult<i64> {
        current_schema_version(&self.connection)
            .map_err(|error| map_migration_error(error, self.path.as_deref()))
    }

    /// Returns whether foreign-key enforcement is enabled on this connection.
    pub fn foreign_keys_enabled(&self) -> StateResult<bool> {
        self.connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .map(|value| value == 1)
            .map_err(|error| self.sqlite_error("read foreign-key setting", error))
    }

    /// Returns the current journal mode.
    pub fn journal_mode(&self) -> StateResult<String> {
        self.connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .map_err(|error| self.sqlite_error("read journal mode", error))
    }

    /// Runs SQLite's read-only `quick_check` query.
    pub fn quick_check(&self) -> StateResult<()> {
        quick_check(&self.connection, self.path.as_deref())
    }

    /// Begins an immediate, repository-safe transaction.
    pub fn begin_transaction(&mut self) -> StateResult<StateTransaction<'_>> {
        let path = self.path.clone();
        let timeout = self.busy_timeout;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                map_sqlite_error(path.as_deref(), "begin transaction", timeout, error)
            })?;
        Ok(StateTransaction { transaction, path })
    }

    /// Runs a complete transaction and rolls it back on any typed error.
    pub fn with_transaction<T, F>(&mut self, operation: F) -> StateResult<T>
    where
        F: FnOnce(&mut StateTransaction<'_>) -> StateResult<T>,
    {
        let mut transaction = self.begin_transaction()?;
        let value = operation(&mut transaction)?;
        transaction.commit()?;
        Ok(value)
    }

    /// Compatibility alias for [`StateStore::with_transaction`].
    pub fn transaction<T, F>(&mut self, operation: F) -> StateResult<T>
    where
        F: FnOnce(&mut StateTransaction<'_>) -> StateResult<T>,
    {
        self.with_transaction(operation)
    }

    /// Returns all repository-family repository handles for this connection.
    ///
    /// Callers that use a handle for mutation should obtain it from a
    /// [`StateTransaction`]; the `StateStore` convenience methods below always
    /// create that transaction boundary.
    pub fn repositories(&self) -> Repositories<'_> {
        Repositories::new(&self.connection, self.path.clone())
    }

    /// Registers or updates one repository identity in a transaction.
    pub fn upsert_repository(&mut self, input: &RepositoryInput) -> StateResult<RepositoryRecord> {
        self.with_transaction(|transaction| transaction.repositories().repositories().upsert(input))
    }

    /// Alias for [`StateStore::upsert_repository`].
    pub fn register_repository(
        &mut self,
        input: &RepositoryInput,
    ) -> StateResult<RepositoryRecord> {
        self.upsert_repository(input)
    }

    /// Creates a validated repository scope for a caller.
    pub fn scope(&self, repository_id: impl Into<String>) -> StateResult<RepositoryScope> {
        RepositoryScope::new(repository_id)
    }

    /// Reads a repository identity using its exact scope.
    pub fn repository(
        &self,
        repository_id: impl AsRef<str>,
    ) -> StateResult<Option<RepositoryRecord>> {
        self.repositories().repositories().get(repository_id)
    }

    /// Reads a repository identity or returns a typed not-found error.
    pub fn require_repository(
        &self,
        repository_id: impl AsRef<str>,
    ) -> StateResult<RepositoryRecord> {
        self.repositories().repositories().require(repository_id)
    }

    /// Inserts a draft in an explicit transaction.
    pub fn create_draft(&mut self, input: &DraftInput) -> StateResult<DraftRecord> {
        self.with_transaction(|transaction| transaction.repositories().drafts().create(input))
    }

    /// Compatibility alias for [`StateStore::create_draft`].
    pub fn insert_draft(&mut self, input: &DraftInput) -> StateResult<DraftRecord> {
        self.create_draft(input)
    }

    /// Reads a draft within an exact repository scope.
    pub fn draft(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> StateResult<Option<DraftRecord>> {
        self.repositories().drafts().get(repository_id, draft_id)
    }

    /// Compatibility alias for [`StateStore::draft`].
    pub fn get_draft(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> StateResult<Option<DraftRecord>> {
        self.draft(repository_id, draft_id)
    }

    /// Inserts an immutable draft revision in an explicit transaction.
    pub fn insert_draft_revision(
        &mut self,
        input: &DraftRevisionInput,
    ) -> StateResult<DraftRevisionRecord> {
        self.with_transaction(|transaction| {
            transaction.repositories().drafts().insert_revision(input)
        })
    }

    /// Changes a draft lifecycle state in a repository-scoped transaction.
    pub fn set_draft_status(
        &mut self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        status: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> StateResult<DraftRecord> {
        let repository_id = repository_id.as_ref().to_owned();
        let draft_id = draft_id.as_ref().to_owned();
        let status = status.into();
        let updated_at = updated_at.into();
        self.with_transaction(move |transaction| {
            transaction.repositories().drafts().set_status(
                &repository_id,
                &draft_id,
                &status,
                updated_at,
            )
        })
    }

    /// Reads one exact immutable draft revision.
    pub fn draft_revision(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
    ) -> StateResult<Option<DraftRevisionRecord>> {
        self.repositories()
            .drafts()
            .revision(repository_id, draft_id, revision)
    }

    /// Compatibility alias for [`StateStore::draft_revision`].
    pub fn get_draft_revision(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
    ) -> StateResult<Option<DraftRevisionRecord>> {
        self.draft_revision(repository_id, draft_id, revision)
    }

    /// Records an exact approval in an explicit transaction.
    pub fn record_approval(&mut self, input: &ApprovalInput) -> StateResult<ApprovalRecord> {
        self.with_transaction(|transaction| transaction.repositories().approvals().record(input))
    }

    /// Compatibility alias for [`StateStore::record_approval`].
    pub fn insert_approval(&mut self, input: &ApprovalInput) -> StateResult<ApprovalRecord> {
        self.record_approval(input)
    }

    /// Reads an approval by its exact repository, draft, and revision.
    pub fn approval(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
    ) -> StateResult<Option<ApprovalRecord>> {
        self.repositories()
            .approvals()
            .get(repository_id, draft_id, revision)
    }

    /// Records a policy activation without evaluating policy.
    pub fn activate_policy(
        &mut self,
        input: &PolicyActivationInput,
    ) -> StateResult<PolicyActivationRecord> {
        self.with_transaction(|transaction| {
            transaction
                .repositories()
                .policy_activations()
                .activate(input)
        })
    }

    /// Reads an approval by exact repository, draft, and revision.
    pub fn get_approval(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
    ) -> StateResult<Option<ApprovalRecord>> {
        self.approval(repository_id, draft_id, revision)
    }

    /// Compatibility alias for [`StateStore::activate_policy`].
    pub fn insert_policy_activation(
        &mut self,
        input: &PolicyActivationInput,
    ) -> StateResult<PolicyActivationRecord> {
        self.activate_policy(input)
    }

    /// Reads a policy activation by exact repository and activation ID.
    pub fn policy_activation(
        &self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
    ) -> StateResult<Option<PolicyActivationRecord>> {
        self.repositories()
            .policy_activations()
            .get(repository_id, activation_id)
    }

    /// Deactivates one exact policy activation without deleting its evidence.
    pub fn deactivate_policy(
        &mut self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
        deactivated_at: impl Into<String>,
    ) -> StateResult<PolicyActivationRecord> {
        let repository_id = repository_id.as_ref().to_owned();
        let activation_id = activation_id.as_ref().to_owned();
        let deactivated_at = deactivated_at.into();
        self.with_transaction(move |transaction| {
            transaction.repositories().policy_activations().deactivate(
                &repository_id,
                &activation_id,
                deactivated_at,
            )
        })
    }

    /// Records a delivery attempt in an explicit transaction.
    pub fn record_delivery_attempt(
        &mut self,
        input: &DeliveryAttemptInput,
    ) -> StateResult<DeliveryAttemptRecord> {
        self.with_transaction(|transaction| {
            transaction.repositories().delivery_attempts().record(input)
        })
    }

    /// Updates one exact delivery attempt state in a local transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn set_delivery_attempt_state(
        &mut self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
        state: impl Into<String>,
        completed_at: Option<String>,
        remote_message_id: Option<String>,
        failure_code: Option<String>,
    ) -> StateResult<DeliveryAttemptRecord> {
        let repository_id = repository_id.as_ref().to_owned();
        let attempt_id = attempt_id.as_ref().to_owned();
        let state = state.into();
        self.with_transaction(move |transaction| {
            transaction.repositories().delivery_attempts().set_state(
                &repository_id,
                &attempt_id,
                &state,
                completed_at,
                remote_message_id,
                failure_code,
            )
        })
    }

    /// Compatibility alias for [`StateStore::record_delivery_attempt`].
    pub fn insert_delivery_attempt(
        &mut self,
        input: &DeliveryAttemptInput,
    ) -> StateResult<DeliveryAttemptRecord> {
        self.record_delivery_attempt(input)
    }

    /// Claims one delivery revision atomically in the local store.
    pub fn claim_delivery_attempt(
        &mut self,
        input: &DeliveryAttemptInput,
    ) -> StateResult<DeliveryAttemptRecord> {
        self.with_transaction(|transaction| {
            transaction.repositories().delivery_attempts().claim(input)
        })
    }

    /// Reads a delivery attempt by exact repository and attempt ID.
    pub fn delivery_attempt(
        &self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
    ) -> StateResult<Option<DeliveryAttemptRecord>> {
        self.repositories()
            .delivery_attempts()
            .get(repository_id, attempt_id)
    }

    /// Stores an inbound item and its first/current snapshots atomically.
    pub fn store_inbound_item(
        &mut self,
        item: &InboundItemInput,
        current: &InboundCurrentSnapshotInput,
    ) -> StateResult<InboundItemRecord> {
        self.with_transaction(|transaction| {
            transaction
                .repositories()
                .inbound()
                .store_item(item, current)
        })
    }

    /// Compatibility alias for [`StateStore::delivery_attempt`].
    pub fn get_delivery_attempt(
        &self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
    ) -> StateResult<Option<DeliveryAttemptRecord>> {
        self.delivery_attempt(repository_id, attempt_id)
    }

    /// Stores an inbound item and its first/current snapshots atomically.
    pub fn inbound_item(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<InboundItemRecord>> {
        self.repositories().inbound().item(repository_id, item_id)
    }

    /// Reads the separate current inbound snapshot or deletion marker.
    pub fn inbound_current(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<InboundCurrentSnapshotRecord>> {
        self.repositories()
            .inbound()
            .current(repository_id, item_id)
    }

    /// Updates the separate current snapshot and appends an edit/delete
    /// transition without changing the first snapshot.
    pub fn record_inbound_transition(
        &mut self,
        input: &InboundTransitionInput,
        current: Option<&InboundCurrentSnapshotInput>,
    ) -> StateResult<InboundTransitionRecord> {
        self.with_transaction(|transaction| {
            transaction
                .repositories()
                .inbound()
                .record_transition(input, current)
        })
    }

    /// Compatibility alias for [`StateStore::advance_inbound_cursor`].
    pub fn set_inbound_cursor(
        &mut self,
        input: &InboundCursorInput,
    ) -> StateResult<InboundCursorRecord> {
        self.advance_inbound_cursor(input)
    }

    /// Advances one alias cursor monotonically in the same repository scope.
    pub fn advance_inbound_cursor(
        &mut self,
        input: &InboundCursorInput,
    ) -> StateResult<InboundCursorRecord> {
        self.with_transaction(|transaction| {
            transaction.repositories().inbound().advance_cursor(input)
        })
    }

    /// Reads one alias cursor.
    pub fn inbound_cursor(
        &self,
        repository_id: impl AsRef<str>,
        alias: impl AsRef<str>,
    ) -> StateResult<Option<InboundCursorRecord>> {
        self.repositories().inbound().cursor(repository_id, alias)
    }

    /// Reads an inbound acknowledgement by exact repository and item.
    pub fn inbound_acknowledgement(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<AcknowledgementRecord>> {
        self.repositories()
            .inbound()
            .acknowledgement(repository_id, item_id)
    }

    /// Reads an inbound archive marker by exact repository and item.
    pub fn inbound_archive(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<ArchiveRecord>> {
        self.repositories()
            .inbound()
            .archive_record(repository_id, item_id)
    }

    /// Reads an inbound reply link by exact repository and item.
    pub fn inbound_reply_link(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<ReplyLinkRecord>> {
        self.repositories()
            .inbound()
            .reply_link(repository_id, item_id)
    }

    /// Acknowledges one or more inbound items idempotently and locally.
    pub fn acknowledge_inbound(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        acknowledged_at: impl Into<String>,
    ) -> StateResult<Vec<AcknowledgementRecord>> {
        let repository_id = repository_id.as_ref().to_owned();
        let item_ids = item_ids
            .iter()
            .map(|item| item.as_ref().to_owned())
            .collect::<Vec<_>>();
        let acknowledged_at = acknowledged_at.into();
        self.with_transaction(move |transaction| {
            transaction.repositories().inbound().acknowledge(
                &repository_id,
                &item_ids,
                &acknowledged_at,
            )
        })
    }

    /// Archives one or more inbound items idempotently and locally.
    pub fn archive_inbound(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        archived_at: impl Into<String>,
    ) -> StateResult<Vec<ArchiveRecord>> {
        let repository_id = repository_id.as_ref().to_owned();
        let item_ids = item_ids
            .iter()
            .map(|item| item.as_ref().to_owned())
            .collect::<Vec<_>>();
        let archived_at = archived_at.into();
        self.with_transaction(move |transaction| {
            transaction
                .repositories()
                .inbound()
                .archive(&repository_id, &item_ids, &archived_at)
        })
    }

    /// Links an inbound item to a locally-created reply draft.
    pub fn link_inbound_reply(&mut self, input: &ReplyLinkInput) -> StateResult<ReplyLinkRecord> {
        self.with_transaction(|transaction| transaction.repositories().inbound().link_reply(input))
    }

    /// Appends one local audit event in the caller's current transaction when
    /// called through [`StateTransaction`], or in its own transaction here.
    pub fn append_audit_event(&mut self, input: &AuditEventInput) -> StateResult<AuditEventRecord> {
        self.with_transaction(|transaction| transaction.repositories().audit().append(input))
    }

    /// Reads an audit event by exact repository and event ID.  This is a
    /// bounded identity lookup, not the audit query product owned by
    /// REPO-AUDIT-2.
    pub fn audit_event(
        &self,
        repository_id: impl AsRef<str>,
        event_id: impl AsRef<str>,
    ) -> StateResult<Option<AuditEventRecord>> {
        self.repositories().audit().get(repository_id, event_id)
    }
}

fn configure_connection(
    connection: &Connection,
    path: Option<&Path>,
    options: StateStoreOptions,
) -> StateResult<()> {
    connection
        .busy_timeout(options.busy_timeout)
        .map_err(|error| map_sqlite_error(path, "set busy timeout", options.busy_timeout, error))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| {
            map_sqlite_error(path, "enable foreign keys", options.busy_timeout, error)
        })?;
    let foreign_keys = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map_err(|error| {
            map_sqlite_error(path, "verify foreign keys", options.busy_timeout, error)
        })?;
    if foreign_keys != 1 {
        return Err(StateError::Transaction {
            message: "SQLite foreign-key enforcement did not remain enabled".to_owned(),
        });
    }
    quick_check(connection, path)?;
    Ok(())
}

fn enable_wal(connection: &Connection, path: Option<&Path>) -> StateResult<()> {
    let mode = connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|error| {
            map_sqlite_error(
                path,
                "enable WAL",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })?;
    if mode != "wal" && mode != "memory" {
        return Err(StateError::Transaction {
            message: format!("SQLite refused WAL journal mode (reported {mode})"),
        });
    }
    Ok(())
}

fn verify_connection_settings(
    connection: &Connection,
    path: Option<&Path>,
    timeout: Duration,
) -> StateResult<()> {
    let foreign_keys = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map_err(|error| map_sqlite_error(path, "verify foreign keys", timeout, error))?;
    if foreign_keys != 1 {
        return Err(StateError::Transaction {
            message: "foreign-key enforcement is not enabled".to_owned(),
        });
    }
    let busy_timeout = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
        .map_err(|error| map_sqlite_error(path, "verify busy timeout", timeout, error))?;
    let configured_timeout = i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX);
    if busy_timeout < 0 || busy_timeout > configured_timeout {
        return Err(StateError::Transaction {
            message: format!(
                "SQLite busy timeout {busy_timeout} is outside the bounded configuration"
            ),
        });
    }
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .map_err(|error| map_sqlite_error(path, "verify journal mode", timeout, error))?;
    if journal_mode != "wal" && journal_mode != "memory" {
        return Err(StateError::Transaction {
            message: format!("SQLite journal mode is not WAL: {journal_mode}"),
        });
    }
    Ok(())
}

fn quick_check(connection: &Connection, path: Option<&Path>) -> StateResult<()> {
    let mut statement = connection.prepare("PRAGMA quick_check").map_err(|error| {
        map_sqlite_error(
            path,
            "prepare quick check",
            Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            error,
        )
    })?;
    let mut rows = statement.query([]).map_err(|error| {
        map_sqlite_error(
            path,
            "run quick check",
            Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            error,
        )
    })?;
    let mut result = Ok(());
    while let Some(row) = rows.next().map_err(|error| {
        map_sqlite_error(
            path,
            "read quick check",
            Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            error,
        )
    })? {
        let value = row.get::<_, String>(0).map_err(|error| {
            map_sqlite_error(
                path,
                "read quick check result",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })?;
        if value != "ok" {
            result = Err(StateError::CorruptDatabase {
                path: path.map(Path::to_path_buf),
                message: value,
            });
            break;
        }
    }
    result
}

fn should_restore_bytes(error: &StateError) -> bool {
    matches!(
        error,
        StateError::CorruptDatabase { .. }
            | StateError::UnsupportedSchema { .. }
            | StateError::MigrationFailed { .. }
            | StateError::UnsupportedSqliteRuntime { .. }
    )
}

fn map_migration_error(error: MigrationError, path: Option<&Path>) -> StateError {
    match error {
        MigrationError::UnsupportedSchema { found, supported } => StateError::UnsupportedSchema {
            path: path.map(Path::to_path_buf),
            found,
            supported,
        },
        MigrationError::ApplyFailed { message, .. } => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("database is locked")
                || lower.contains("database table is locked")
                || lower.contains("sqlite_busy")
            {
                StateError::LockTimeout {
                    path: path.map(Path::to_path_buf),
                    operation: "migration",
                    timeout_ms: DEFAULT_BUSY_TIMEOUT_MS,
                }
            } else {
                StateError::MigrationFailed {
                    path: path.map(Path::to_path_buf),
                    message,
                }
            }
        }
        MigrationError::VerificationFailed { message }
        | MigrationError::VersionRead { message }
        | MigrationError::Sqlite { message, .. } => StateError::MigrationFailed {
            path: path.map(Path::to_path_buf),
            message,
        },
        MigrationError::InvalidVersion(version) => StateError::MigrationFailed {
            path: path.map(Path::to_path_buf),
            message: format!("invalid schema version {version}"),
        },
    }
}

fn map_sqlite_error(
    path: Option<&Path>,
    operation: &'static str,
    timeout: Duration,
    error: RusqliteError,
) -> StateError {
    if let RusqliteError::SqliteFailure(code, _) = &error {
        if matches!(
            code.code,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
        ) {
            return StateError::LockTimeout {
                path: path.map(Path::to_path_buf),
                operation,
                timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
            };
        }
        if matches!(
            code.code,
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
        ) {
            return StateError::CorruptDatabase {
                path: path.map(Path::to_path_buf),
                message: error.to_string(),
            };
        }
    }
    if error
        .to_string()
        .to_ascii_lowercase()
        .contains("not a database")
    {
        return StateError::CorruptDatabase {
            path: path.map(Path::to_path_buf),
            message: error.to_string(),
        };
    }
    StateError::Sqlite {
        path: path.map(Path::to_path_buf),
        operation,
        message: error.to_string(),
    }
}

fn validate_id(kind: &'static str, value: &str) -> StateResult<()> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') {
        return Err(StateError::InvalidIdentifier { kind });
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> StateResult<()> {
    if value.is_empty() || value.len() > 128 || value.contains('\0') {
        return Err(StateError::InvalidIdentifier { kind: "timestamp" });
    }
    Ok(())
}

fn ensure_repository(connection: &Connection, repository_id: &str) -> StateResult<()> {
    validate_id("repository", repository_id)?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM repositories WHERE repository_id = ?1",
            [repository_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            map_sqlite_error(
                None,
                "verify repository scope",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(StateError::RepositoryNotFound {
            repository_id: repository_id.to_owned(),
        })
    }
}

fn require_row<T>(
    value: Option<T>,
    entity: &'static str,
    repository_id: &str,
    object_id: &str,
) -> StateResult<T> {
    value.ok_or_else(|| StateError::NotFound {
        entity,
        repository_id: repository_id.to_owned(),
        object_id: object_id.to_owned(),
    })
}

/// A transaction handle whose repositories all share one commit boundary.
pub struct StateTransaction<'conn> {
    transaction: Transaction<'conn>,
    path: Option<PathBuf>,
}

impl fmt::Debug for StateTransaction<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateTransaction")
            .field("path", &self.path)
            .finish()
    }
}

impl Deref for StateTransaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl StateTransaction<'_> {
    /// Returns the underlying connection participating in this transaction.
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.transaction
    }

    /// Returns all repository handles bound to this transaction.
    pub fn repositories(&self) -> Repositories<'_> {
        Repositories::new(&self.transaction, self.path.clone())
    }

    /// Appends an audit event to this transaction.
    pub fn append_audit_event(&self, input: &AuditEventInput) -> StateResult<AuditEventRecord> {
        self.repositories().audit().append(input)
    }

    /// Commits the complete transaction.
    pub fn commit(self) -> StateResult<()> {
        self.transaction.commit().map_err(|error| {
            map_sqlite_error(
                self.path.as_deref(),
                "commit transaction",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })
    }

    /// Explicitly rolls back the complete transaction.
    pub fn rollback(self) -> StateResult<()> {
        self.transaction.rollback().map_err(|error| {
            map_sqlite_error(
                self.path.as_deref(),
                "rollback transaction",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })
    }
}

/// A bundle of repository handles over one connection or transaction.
pub struct Repositories<'conn> {
    repositories: RepositoryRepository<'conn>,
    drafts: DraftRepository<'conn>,
    approvals: ApprovalRepository<'conn>,
    policy_activations: PolicyActivationRepository<'conn>,
    delivery_attempts: DeliveryAttemptRepository<'conn>,
    inbound: InboundRepository<'conn>,
    audit: AuditRepository<'conn>,
}

impl<'conn> Repositories<'conn> {
    fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self {
            repositories: RepositoryRepository::new(connection, path.clone()),
            drafts: DraftRepository::new(connection, path.clone()),
            approvals: ApprovalRepository::new(connection, path.clone()),
            policy_activations: PolicyActivationRepository::new(connection, path.clone()),
            delivery_attempts: DeliveryAttemptRepository::new(connection, path.clone()),
            inbound: InboundRepository::new(connection, path.clone()),
            audit: AuditRepository::new(connection, path),
        }
    }

    /// Returns the repository identity handle.
    pub fn repositories(&self) -> &RepositoryRepository<'conn> {
        &self.repositories
    }

    /// Returns the draft/revision handle.
    pub fn drafts(&self) -> &DraftRepository<'conn> {
        &self.drafts
    }

    /// Returns the approval handle.
    pub fn approvals(&self) -> &ApprovalRepository<'conn> {
        &self.approvals
    }

    /// Returns the policy activation handle.
    pub fn policy_activations(&self) -> &PolicyActivationRepository<'conn> {
        &self.policy_activations
    }

    /// Returns the delivery attempt handle.
    pub fn delivery_attempts(&self) -> &DeliveryAttemptRepository<'conn> {
        &self.delivery_attempts
    }

    /// Returns the inbound state handle.
    pub fn inbound(&self) -> &InboundRepository<'conn> {
        &self.inbound
    }

    /// Returns the append-only audit handle.
    pub fn audit(&self) -> &AuditRepository<'conn> {
        &self.audit
    }
}

/// A repository identity mutation/input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryInput {
    /// Stable configured repository ID.
    pub repository_id: String,
    /// Non-secret Discord workspace ID associated with the repository.
    pub workspace_id: String,
    /// Canonical configuration hash.
    pub config_hash: String,
    /// Creation timestamp in UTC RFC 3339 form.
    pub created_at: String,
    /// Last update timestamp in UTC RFC 3339 form.
    pub updated_at: String,
}

impl RepositoryInput {
    /// Creates a repository identity input.
    pub fn new(
        repository_id: impl Into<String>,
        workspace_id: impl Into<String>,
        config_hash: impl Into<String>,
        timestamp: impl Into<String>,
    ) -> Self {
        let timestamp = timestamp.into();
        Self {
            repository_id: repository_id.into(),
            workspace_id: workspace_id.into(),
            config_hash: config_hash.into(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }
}

/// The stored repository identity.
pub type RepositoryRecord = RepositoryInput;

/// A validated repository scope used by higher-level state workflows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryScope {
    repository_id: String,
}

impl RepositoryScope {
    /// Creates a validated repository scope.
    pub fn new(repository_id: impl Into<String>) -> StateResult<Self> {
        let repository_id = repository_id.into();
        validate_id("repository", &repository_id)?;
        Ok(Self { repository_id })
    }

    /// Returns the configured repository ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.repository_id
    }

    /// Returns the configured repository ID.
    #[must_use]
    pub fn id(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for RepositoryScope {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Input for a draft row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftInput {
    /// Repository scope.
    pub repository_id: String,
    /// Stable draft ID.
    pub draft_id: String,
    /// Event type used by later policy evaluation.
    pub event_type: String,
    /// Repository-local destination alias.
    pub destination_alias: String,
    /// Initial lifecycle state.
    pub status: String,
    /// Optional expiry timestamp.
    pub expiry_at: Option<String>,
    /// Optional inbound item targeted by a reply draft.
    pub reply_to_inbound_item_id: Option<String>,
    /// Non-secret metadata JSON.
    pub metadata_json: String,
    /// Creation/update timestamp.
    pub created_at: String,
    /// Update timestamp.
    pub updated_at: String,
}

impl DraftInput {
    /// Creates a draft input with an initial `draft` state.
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        event_type: impl Into<String>,
        destination_alias: impl Into<String>,
        timestamp: impl Into<String>,
    ) -> Self {
        let timestamp = timestamp.into();
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            event_type: event_type.into(),
            destination_alias: destination_alias.into(),
            status: "draft".to_owned(),
            expiry_at: None,
            reply_to_inbound_item_id: None,
            metadata_json: "{}".to_owned(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }
}

/// Stored draft identity and current revision pointer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftRecord {
    /// Repository scope.
    pub repository_id: String,
    /// Draft ID.
    pub draft_id: String,
    /// Event type.
    pub event_type: String,
    /// Destination alias.
    pub destination_alias: String,
    /// Current lifecycle state.
    pub status: String,
    /// Highest inserted revision.
    pub current_revision: i64,
    /// Optional expiry.
    pub expiry_at: Option<String>,
    /// Optional inbound target.
    pub reply_to_inbound_item_id: Option<String>,
    /// Non-secret metadata JSON.
    pub metadata_json: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Update timestamp.
    pub updated_at: String,
}

/// Input for an immutable draft revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftRevisionInput {
    /// Repository scope.
    pub repository_id: String,
    /// Draft ID.
    pub draft_id: String,
    /// Monotonic revision number.
    pub revision: i64,
    /// Content hash.
    pub content_hash: String,
    /// Exact rendered content.
    pub body: String,
    /// Non-secret metadata JSON.
    pub metadata_json: String,
    /// Destination alias snapshot.
    pub destination_alias: String,
    /// Resolved destination snapshot.
    pub resolved_destination: String,
    /// Optional expiry.
    pub expiry_at: Option<String>,
    /// Revision lifecycle state.
    pub lifecycle_state: String,
    /// Optional inbound target.
    pub reply_to_inbound_item_id: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
}

impl DraftRevisionInput {
    /// Creates a revision input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: i64,
        content_hash: impl Into<String>,
        body: impl Into<String>,
        destination_alias: impl Into<String>,
        resolved_destination: impl Into<String>,
        timestamp: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            content_hash: content_hash.into(),
            body: body.into(),
            metadata_json: "{}".to_owned(),
            destination_alias: destination_alias.into(),
            resolved_destination: resolved_destination.into(),
            expiry_at: None,
            lifecycle_state: "draft".to_owned(),
            reply_to_inbound_item_id: None,
            created_at: timestamp.into(),
        }
    }
}

/// Stored immutable draft revision.
pub type DraftRevisionRecord = DraftRevisionInput;

/// Input for an exact approval record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalInput {
    /// Repository scope.
    pub repository_id: String,
    /// Stable approval ID.
    pub approval_id: String,
    /// Draft ID.
    pub draft_id: String,
    /// Exact approved revision.
    pub revision: i64,
    /// Approval state.
    pub approval_state: String,
    /// Actor kind, never a credential.
    pub actor_kind: String,
    /// Optional non-secret operator reference.
    pub operator_reference: Option<String>,
    /// Approval timestamp.
    pub approved_at: String,
    /// Optional revocation timestamp.
    pub revoked_at: Option<String>,
}

impl ApprovalInput {
    /// Creates an approved record.
    pub fn new(
        repository_id: impl Into<String>,
        approval_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: i64,
        actor_kind: impl Into<String>,
        approved_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            approval_id: approval_id.into(),
            draft_id: draft_id.into(),
            revision,
            approval_state: "approved".to_owned(),
            actor_kind: actor_kind.into(),
            operator_reference: None,
            approved_at: approved_at.into(),
            revoked_at: None,
        }
    }
}

/// Stored approval record.
pub type ApprovalRecord = ApprovalInput;

/// Input for an exact policy activation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyActivationInput {
    /// Repository scope.
    pub repository_id: String,
    /// Stable activation ID.
    pub activation_id: String,
    /// Canonical configuration hash.
    pub config_hash: String,
    /// Exact policy tuple hash.
    pub policy_tuple_hash: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact destination alias.
    pub destination_alias: String,
    /// Exact severity.
    pub severity: String,
    /// Activation timestamp.
    pub activated_at: String,
}

impl PolicyActivationInput {
    /// Creates an active policy input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository_id: impl Into<String>,
        activation_id: impl Into<String>,
        config_hash: impl Into<String>,
        policy_tuple_hash: impl Into<String>,
        event_type: impl Into<String>,
        destination_alias: impl Into<String>,
        severity: impl Into<String>,
        activated_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            activation_id: activation_id.into(),
            config_hash: config_hash.into(),
            policy_tuple_hash: policy_tuple_hash.into(),
            event_type: event_type.into(),
            destination_alias: destination_alias.into(),
            severity: severity.into(),
            activated_at: activated_at.into(),
        }
    }
}

/// Stored policy activation record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyActivationRecord {
    /// Repository scope.
    pub repository_id: String,
    /// Activation ID.
    pub activation_id: String,
    /// Canonical configuration hash.
    pub config_hash: String,
    /// Exact policy tuple hash.
    pub policy_tuple_hash: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact destination alias.
    pub destination_alias: String,
    /// Exact severity.
    pub severity: String,
    /// Activation timestamp.
    pub activated_at: String,
    /// Deactivation timestamp.
    pub deactivated_at: Option<String>,
    /// Whether the activation is active.
    pub active: bool,
}

/// Input for a delivery attempt or claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryAttemptInput {
    /// Repository scope.
    pub repository_id: String,
    /// Stable attempt ID.
    pub attempt_id: String,
    /// Draft ID.
    pub draft_id: String,
    /// Exact draft revision.
    pub revision: i64,
    /// Monotonic attempt number for this revision.
    pub attempt_number: i64,
    /// Unique claim nonce.
    pub claim_nonce: String,
    /// Attempt state.
    pub state: String,
    /// Claim timestamp.
    pub claimed_at: String,
    /// Completion timestamp.
    pub completed_at: Option<String>,
    /// Optional remote message ID; this crate never calls Discord.
    pub remote_message_id: Option<String>,
    /// Optional safe failure code.
    pub failure_code: Option<String>,
}

impl DeliveryAttemptInput {
    /// Creates a claimed attempt input.
    pub fn new(
        repository_id: impl Into<String>,
        attempt_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: i64,
        attempt_number: i64,
        claim_nonce: impl Into<String>,
        claimed_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            attempt_id: attempt_id.into(),
            draft_id: draft_id.into(),
            revision,
            attempt_number,
            claim_nonce: claim_nonce.into(),
            state: "claimed".to_owned(),
            claimed_at: claimed_at.into(),
            completed_at: None,
            remote_message_id: None,
            failure_code: None,
        }
    }
}

/// Stored delivery attempt record.
pub type DeliveryAttemptRecord = DeliveryAttemptInput;

/// Input for an inbound first snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundItemInput {
    /// Repository scope.
    pub repository_id: String,
    /// Remote message/item ID.
    pub item_id: String,
    /// Channel ID.
    pub channel_id: String,
    /// Human author ID.
    pub author_id: String,
    /// First observed content.
    pub first_content: String,
    /// First attachment indicators JSON.
    pub first_attachments_json: String,
    /// First observation timestamp.
    pub first_observed_at: String,
    /// Local creation timestamp.
    pub created_at: String,
}

impl InboundItemInput {
    /// Creates an inbound first-snapshot input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository_id: impl Into<String>,
        item_id: impl Into<String>,
        channel_id: impl Into<String>,
        author_id: impl Into<String>,
        first_content: impl Into<String>,
        first_observed_at: impl Into<String>,
    ) -> Self {
        let first_observed_at = first_observed_at.into();
        Self {
            repository_id: repository_id.into(),
            item_id: item_id.into(),
            channel_id: channel_id.into(),
            author_id: author_id.into(),
            first_content: first_content.into(),
            first_attachments_json: "[]".to_owned(),
            created_at: first_observed_at.clone(),
            first_observed_at,
        }
    }
}

/// Stored first inbound snapshot.
pub type InboundItemRecord = InboundItemInput;

/// Input for the current remote snapshot or deletion marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundCurrentSnapshotInput {
    /// Repository scope.
    pub repository_id: String,
    /// Remote item ID.
    pub item_id: String,
    /// Current content, or `None` for a deletion marker.
    pub current_content: Option<String>,
    /// Current attachment indicators JSON.
    pub current_attachments_json: String,
    /// Whether the remote item is deleted.
    pub deleted: bool,
    /// Observation timestamp.
    pub observed_at: String,
}

impl InboundCurrentSnapshotInput {
    /// Creates a current snapshot input.
    pub fn new(
        repository_id: impl Into<String>,
        item_id: impl Into<String>,
        current_content: Option<String>,
        deleted: bool,
        observed_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            item_id: item_id.into(),
            current_content,
            current_attachments_json: "[]".to_owned(),
            deleted,
            observed_at: observed_at.into(),
        }
    }
}

/// Stored current inbound snapshot.
pub type InboundCurrentSnapshotRecord = InboundCurrentSnapshotInput;

/// Input for an inbound edit/delete/lifecycle transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundTransitionInput {
    /// Repository scope.
    pub repository_id: String,
    /// Stable transition ID.
    pub transition_id: String,
    /// Remote item ID.
    pub item_id: String,
    /// Transition type.
    pub transition_type: String,
    /// Optional observed content.
    pub content: Option<String>,
    /// Transition timestamp.
    pub occurred_at: String,
    /// Non-secret metadata JSON.
    pub metadata_json: String,
}

impl InboundTransitionInput {
    /// Creates a transition input.
    pub fn new(
        repository_id: impl Into<String>,
        transition_id: impl Into<String>,
        item_id: impl Into<String>,
        transition_type: impl Into<String>,
        content: Option<String>,
        occurred_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            transition_id: transition_id.into(),
            item_id: item_id.into(),
            transition_type: transition_type.into(),
            content,
            occurred_at: occurred_at.into(),
            metadata_json: "{}".to_owned(),
        }
    }
}

/// Stored inbound transition.
pub type InboundTransitionRecord = InboundTransitionInput;

/// Input for one per-alias cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundCursorInput {
    /// Repository scope.
    pub repository_id: String,
    /// Repository-local alias.
    pub alias: String,
    /// Opaque remote cursor value.
    pub cursor: String,
    /// Update timestamp.
    pub updated_at: String,
}

impl InboundCursorInput {
    /// Creates a cursor input.
    pub fn new(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        cursor: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            alias: alias.into(),
            cursor: cursor.into(),
            updated_at: updated_at.into(),
        }
    }
}

/// Stored cursor record.
pub type InboundCursorRecord = InboundCursorInput;

/// Stored acknowledgement record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcknowledgementRecord {
    /// Repository scope.
    pub repository_id: String,
    /// Remote item ID.
    pub item_id: String,
    /// First local acknowledgement timestamp.
    pub acknowledged_at: String,
}

/// Stored archive record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveRecord {
    /// Repository scope.
    pub repository_id: String,
    /// Remote item ID.
    pub item_id: String,
    /// First local archive timestamp.
    pub archived_at: String,
}

/// Input for a local inbound/reply link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyLinkInput {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item ID.
    pub item_id: String,
    /// Local reply draft ID.
    pub reply_draft_id: String,
    /// Link timestamp.
    pub linked_at: String,
}

impl ReplyLinkInput {
    /// Creates a reply-link input.
    pub fn new(
        repository_id: impl Into<String>,
        item_id: impl Into<String>,
        reply_draft_id: impl Into<String>,
        linked_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            item_id: item_id.into(),
            reply_draft_id: reply_draft_id.into(),
            linked_at: linked_at.into(),
        }
    }
}

/// Stored reply-link record.
pub type ReplyLinkRecord = ReplyLinkInput;

/// Input for one append-only local audit event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEventInput {
    /// Repository scope.
    pub repository_id: String,
    /// Stable event ID.
    pub event_id: String,
    /// State object family.
    pub object_type: String,
    /// State object ID.
    pub object_id: String,
    /// Transition name.
    pub transition: String,
    /// UTC timestamp.
    pub occurred_at: String,
    /// Actor kind.
    pub actor_kind: String,
    /// Outcome.
    pub outcome: String,
    /// Redacted/non-secret metadata JSON.
    pub metadata_json: String,
}

impl AuditEventInput {
    /// Creates an audit event input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository_id: impl Into<String>,
        event_id: impl Into<String>,
        object_type: impl Into<String>,
        object_id: impl Into<String>,
        transition: impl Into<String>,
        occurred_at: impl Into<String>,
        actor_kind: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            event_id: event_id.into(),
            object_type: object_type.into(),
            object_id: object_id.into(),
            transition: transition.into(),
            occurred_at: occurred_at.into(),
            actor_kind: actor_kind.into(),
            outcome: outcome.into(),
            metadata_json: "{}".to_owned(),
        }
    }
}

/// Stored audit event record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEventRecord {
    /// Monotonic local sequence.
    pub audit_id: i64,
    /// Repository scope.
    pub repository_id: String,
    /// Stable event ID.
    pub event_id: String,
    /// State object family.
    pub object_type: String,
    /// State object ID.
    pub object_id: String,
    /// Transition name.
    pub transition: String,
    /// UTC timestamp.
    pub occurred_at: String,
    /// Actor kind.
    pub actor_kind: String,
    /// Outcome.
    pub outcome: String,
    /// Redacted/non-secret metadata JSON.
    pub metadata_json: String,
}

/// Repository identity persistence.
pub struct RepositoryRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> RepositoryRepository<'conn> {
    /// Creates a repository identity handle over a connection or transaction.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Inserts or updates one repository identity.
    pub fn upsert(&self, input: &RepositoryInput) -> StateResult<RepositoryRecord> {
        validate_id("repository", &input.repository_id)?;
        validate_id("workspace", &input.workspace_id)?;
        validate_id("config hash", &input.config_hash)?;
        validate_timestamp(&input.created_at)?;
        validate_timestamp(&input.updated_at)?;
        self.connection
            .execute(
                "INSERT INTO repositories(repository_id, workspace_id, config_hash, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(repository_id) DO UPDATE SET
                     workspace_id = excluded.workspace_id,
                     config_hash = excluded.config_hash,
                     updated_at = excluded.updated_at",
                params![
                    input.repository_id,
                    input.workspace_id,
                    input.config_hash,
                    input.created_at,
                    input.updated_at
                ],
            )
            .map_err(|error| map_repository_error(self.path.as_deref(), "repository", "upsert repository", error))?;
        self.require(&input.repository_id)
    }

    /// Reads one repository identity with its exact ID.
    pub fn get(&self, repository_id: impl AsRef<str>) -> StateResult<Option<RepositoryRecord>> {
        let repository_id = repository_id.as_ref();
        validate_id("repository", repository_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, workspace_id, config_hash, created_at, updated_at
                 FROM repositories WHERE repository_id = ?1",
                [repository_id],
                |row| {
                    Ok(RepositoryRecord {
                        repository_id: row.get(0)?,
                        workspace_id: row.get(1)?,
                        config_hash: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "repository", "read repository", error)
            })
    }

    /// Reads one repository identity or returns a typed not-found error.
    pub fn require(&self, repository_id: impl AsRef<str>) -> StateResult<RepositoryRecord> {
        let repository_id = repository_id.as_ref();
        require_row(
            self.get(repository_id)?,
            "repository",
            repository_id,
            repository_id,
        )
    }
}

/// Draft and immutable-revision persistence.
pub struct DraftRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> DraftRepository<'conn> {
    /// Creates a draft handle over a connection or transaction.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Creates a draft after revalidating its repository scope.
    pub fn create(&self, input: &DraftInput) -> StateResult<DraftRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("draft", &input.draft_id)?;
        validate_id("event type", &input.event_type)?;
        validate_id("destination alias", &input.destination_alias)?;
        validate_timestamp(&input.created_at)?;
        validate_timestamp(&input.updated_at)?;
        self.connection
            .execute(
                "INSERT INTO drafts(
                    repository_id, draft_id, event_type, destination_alias, status,
                    current_revision, expiry_at, reply_to_inbound_item_id, metadata_json,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9, ?10)",
                params![
                    input.repository_id,
                    input.draft_id,
                    input.event_type,
                    input.destination_alias,
                    input.status,
                    input.expiry_at,
                    input.reply_to_inbound_item_id,
                    input.metadata_json,
                    input.created_at,
                    input.updated_at
                ],
            )
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "draft", "create draft", error)
            })?;
        self.require(&input.repository_id, &input.draft_id)
    }

    /// Reads one draft within its exact repository scope.
    pub fn get(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> StateResult<Option<DraftRecord>> {
        let repository_id = repository_id.as_ref();
        let draft_id = draft_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("draft", draft_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, draft_id, event_type, destination_alias, status,
                        current_revision, expiry_at, reply_to_inbound_item_id, metadata_json,
                        created_at, updated_at
                 FROM drafts WHERE repository_id = ?1 AND draft_id = ?2",
                params![repository_id, draft_id],
                |row| {
                    Ok(DraftRecord {
                        repository_id: row.get(0)?,
                        draft_id: row.get(1)?,
                        event_type: row.get(2)?,
                        destination_alias: row.get(3)?,
                        status: row.get(4)?,
                        current_revision: row.get(5)?,
                        expiry_at: row.get(6)?,
                        reply_to_inbound_item_id: row.get(7)?,
                        metadata_json: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "draft", "read draft", error)
            })
    }

    /// Reads one draft or returns a typed not-found error.
    pub fn require(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> StateResult<DraftRecord> {
        let repository_id = repository_id.as_ref();
        let draft_id = draft_id.as_ref();
        require_row(
            self.get(repository_id, draft_id)?,
            "draft",
            repository_id,
            draft_id,
        )
    }

    /// Inserts one immutable revision and advances the draft pointer.
    pub fn insert_revision(&self, input: &DraftRevisionInput) -> StateResult<DraftRevisionRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("draft", &input.draft_id)?;
        validate_id("content hash", &input.content_hash)?;
        validate_id("destination alias", &input.destination_alias)?;
        validate_id("resolved destination", &input.resolved_destination)?;
        validate_timestamp(&input.created_at)?;
        if input.revision <= 0 {
            return Err(StateError::InvalidIdentifier { kind: "revision" });
        }
        ensure_draft(
            self.connection,
            &input.repository_id,
            &input.draft_id,
            self.path.as_deref(),
        )?;
        self.connection
            .execute(
                "INSERT INTO draft_revisions(
                    repository_id, draft_id, revision, content_hash, body, metadata_json,
                    destination_alias, resolved_destination, expiry_at, lifecycle_state,
                    reply_to_inbound_item_id, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    input.repository_id,
                    input.draft_id,
                    input.revision,
                    input.content_hash,
                    input.body,
                    input.metadata_json,
                    input.destination_alias,
                    input.resolved_destination,
                    input.expiry_at,
                    input.lifecycle_state,
                    input.reply_to_inbound_item_id,
                    input.created_at
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "draft revision",
                    "insert revision",
                    error,
                )
            })?;
        self.connection
            .execute(
                "UPDATE drafts
                 SET current_revision = CASE WHEN current_revision < ?3 THEN ?3 ELSE current_revision END,
                     updated_at = CASE WHEN current_revision < ?3 THEN ?4 ELSE updated_at END
                 WHERE repository_id = ?1 AND draft_id = ?2",
                params![input.repository_id, input.draft_id, input.revision, input.created_at],
            )
            .map_err(|error| map_repository_error(self.path.as_deref(), "draft", "advance revision", error))?;
        Ok(input.clone())
    }

    /// Reads one exact immutable revision.
    pub fn revision(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
    ) -> StateResult<Option<DraftRevisionRecord>> {
        let repository_id = repository_id.as_ref();
        let draft_id = draft_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("draft", draft_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, draft_id, revision, content_hash, body, metadata_json,
                        destination_alias, resolved_destination, expiry_at, lifecycle_state,
                        reply_to_inbound_item_id, created_at
                 FROM draft_revisions
                 WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
                params![repository_id, draft_id, revision],
                |row| {
                    Ok(DraftRevisionRecord {
                        repository_id: row.get(0)?,
                        draft_id: row.get(1)?,
                        revision: row.get(2)?,
                        content_hash: row.get(3)?,
                        body: row.get(4)?,
                        metadata_json: row.get(5)?,
                        destination_alias: row.get(6)?,
                        resolved_destination: row.get(7)?,
                        expiry_at: row.get(8)?,
                        lifecycle_state: row.get(9)?,
                        reply_to_inbound_item_id: row.get(10)?,
                        created_at: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "draft revision",
                    "read revision",
                    error,
                )
            })
    }

    /// Lists immutable revisions in deterministic order.
    pub fn revisions(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> StateResult<Vec<DraftRevisionRecord>> {
        let repository_id = repository_id.as_ref();
        let draft_id = draft_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("draft", draft_id)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT repository_id, draft_id, revision, content_hash, body, metadata_json,
                        destination_alias, resolved_destination, expiry_at, lifecycle_state,
                        reply_to_inbound_item_id, created_at
                 FROM draft_revisions
                 WHERE repository_id = ?1 AND draft_id = ?2
                 ORDER BY revision ASC",
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "draft revision",
                    "prepare revisions",
                    error,
                )
            })?;
        let rows = statement
            .query_map(params![repository_id, draft_id], |row| {
                Ok(DraftRevisionRecord {
                    repository_id: row.get(0)?,
                    draft_id: row.get(1)?,
                    revision: row.get(2)?,
                    content_hash: row.get(3)?,
                    body: row.get(4)?,
                    metadata_json: row.get(5)?,
                    destination_alias: row.get(6)?,
                    resolved_destination: row.get(7)?,
                    expiry_at: row.get(8)?,
                    lifecycle_state: row.get(9)?,
                    reply_to_inbound_item_id: row.get(10)?,
                    created_at: row.get(11)?,
                })
            })
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "draft revision",
                    "query revisions",
                    error,
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            map_repository_error(
                self.path.as_deref(),
                "draft revision",
                "read revisions",
                error,
            )
        })
    }

    /// Changes a draft's lifecycle state without mutating any revision.
    pub fn set_status(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        status: impl AsRef<str>,
        updated_at: impl Into<String>,
    ) -> StateResult<DraftRecord> {
        let repository_id = repository_id.as_ref();
        let draft_id = draft_id.as_ref();
        let status = status.as_ref();
        let updated_at = updated_at.into();
        validate_id("repository", repository_id)?;
        validate_id("draft", draft_id)?;
        validate_timestamp(&updated_at)?;
        let changed = self
            .connection
            .execute(
                "UPDATE drafts SET status = ?3, updated_at = ?4
                 WHERE repository_id = ?1 AND draft_id = ?2",
                params![repository_id, draft_id, status, updated_at],
            )
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "draft", "set status", error)
            })?;
        if changed == 0 {
            return Err(StateError::NotFound {
                entity: "draft",
                repository_id: repository_id.to_owned(),
                object_id: draft_id.to_owned(),
            });
        }
        self.require(repository_id, draft_id)
    }
}

fn ensure_draft(
    connection: &Connection,
    repository_id: &str,
    draft_id: &str,
    path: Option<&Path>,
) -> StateResult<()> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM drafts WHERE repository_id = ?1 AND draft_id = ?2",
            params![repository_id, draft_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            map_sqlite_error(
                path,
                "verify draft scope",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(StateError::NotFound {
            entity: "draft",
            repository_id: repository_id.to_owned(),
            object_id: draft_id.to_owned(),
        })
    }
}

/// Approval persistence.
pub struct ApprovalRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> ApprovalRepository<'conn> {
    /// Creates an approval handle over a connection or transaction.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Records an approval for one exact immutable revision.
    pub fn record(&self, input: &ApprovalInput) -> StateResult<ApprovalRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("approval", &input.approval_id)?;
        validate_id("draft", &input.draft_id)?;
        validate_id("actor", &input.actor_kind)?;
        validate_timestamp(&input.approved_at)?;
        if input.revision <= 0 {
            return Err(StateError::InvalidIdentifier { kind: "revision" });
        }
        ensure_revision(
            self.connection,
            &input.repository_id,
            &input.draft_id,
            input.revision,
            self.path.as_deref(),
        )?;
        self.connection
            .execute(
                "INSERT INTO approvals(
                    repository_id, approval_id, draft_id, revision, approval_state,
                    actor_kind, operator_reference, approved_at, revoked_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    input.repository_id,
                    input.approval_id,
                    input.draft_id,
                    input.revision,
                    input.approval_state,
                    input.actor_kind,
                    input.operator_reference,
                    input.approved_at,
                    input.revoked_at
                ],
            )
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "approval", "record approval", error)
            })?;
        self.get(&input.repository_id, &input.draft_id, input.revision)?
            .ok_or_else(|| StateError::NotFound {
                entity: "approval",
                repository_id: input.repository_id.clone(),
                object_id: input.approval_id.clone(),
            })
    }

    /// Reads an approval by exact repository, draft, and revision.
    pub fn get(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
    ) -> StateResult<Option<ApprovalRecord>> {
        let repository_id = repository_id.as_ref();
        let draft_id = draft_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("draft", draft_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, approval_id, draft_id, revision, approval_state,
                        actor_kind, operator_reference, approved_at, revoked_at
                 FROM approvals
                 WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
                params![repository_id, draft_id, revision],
                |row| {
                    Ok(ApprovalRecord {
                        repository_id: row.get(0)?,
                        approval_id: row.get(1)?,
                        draft_id: row.get(2)?,
                        revision: row.get(3)?,
                        approval_state: row.get(4)?,
                        actor_kind: row.get(5)?,
                        operator_reference: row.get(6)?,
                        approved_at: row.get(7)?,
                        revoked_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "approval", "read approval", error)
            })
    }
}

fn ensure_revision(
    connection: &Connection,
    repository_id: &str,
    draft_id: &str,
    revision: i64,
    path: Option<&Path>,
) -> StateResult<()> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM draft_revisions
             WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
            params![repository_id, draft_id, revision],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            map_sqlite_error(
                path,
                "verify revision scope",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(StateError::NotFound {
            entity: "draft revision",
            repository_id: repository_id.to_owned(),
            object_id: format!("{draft_id}:{revision}"),
        })
    }
}

/// Policy activation persistence.  Matching and authorization belong to the
/// policy task; this repository only stores exact activation evidence.
pub struct PolicyActivationRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> PolicyActivationRepository<'conn> {
    /// Creates a policy activation handle.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Inserts an exact activation record.
    pub fn activate(&self, input: &PolicyActivationInput) -> StateResult<PolicyActivationRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("activation", &input.activation_id)?;
        validate_id("config hash", &input.config_hash)?;
        validate_id("policy tuple hash", &input.policy_tuple_hash)?;
        validate_id("event type", &input.event_type)?;
        validate_id("destination alias", &input.destination_alias)?;
        validate_id("severity", &input.severity)?;
        validate_timestamp(&input.activated_at)?;
        self.connection
            .execute(
                "INSERT INTO policy_activations(
                    repository_id, activation_id, config_hash, policy_tuple_hash,
                    event_type, destination_alias, severity, activated_at, deactivated_at, active
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 1)",
                params![
                    input.repository_id,
                    input.activation_id,
                    input.config_hash,
                    input.policy_tuple_hash,
                    input.event_type,
                    input.destination_alias,
                    input.severity,
                    input.activated_at
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "policy activation",
                    "activate policy",
                    error,
                )
            })?;
        self.require(&input.repository_id, &input.activation_id)
    }

    /// Reads an activation by exact repository and activation ID.
    pub fn get(
        &self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
    ) -> StateResult<Option<PolicyActivationRecord>> {
        let repository_id = repository_id.as_ref();
        let activation_id = activation_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("activation", activation_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, activation_id, config_hash, policy_tuple_hash,
                        event_type, destination_alias, severity, activated_at,
                        deactivated_at, active
                 FROM policy_activations
                 WHERE repository_id = ?1 AND activation_id = ?2",
                params![repository_id, activation_id],
                row_to_policy_activation,
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "policy activation",
                    "read activation",
                    error,
                )
            })
    }

    /// Reads an activation or returns a typed not-found error.
    pub fn require(
        &self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
    ) -> StateResult<PolicyActivationRecord> {
        let repository_id = repository_id.as_ref();
        let activation_id = activation_id.as_ref();
        require_row(
            self.get(repository_id, activation_id)?,
            "policy activation",
            repository_id,
            activation_id,
        )
    }

    /// Deactivates an exact activation without deleting its evidence.
    pub fn deactivate(
        &self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
        deactivated_at: impl Into<String>,
    ) -> StateResult<PolicyActivationRecord> {
        let repository_id = repository_id.as_ref();
        let activation_id = activation_id.as_ref();
        let deactivated_at = deactivated_at.into();
        validate_id("repository", repository_id)?;
        validate_id("activation", activation_id)?;
        validate_timestamp(&deactivated_at)?;
        let changed = self
            .connection
            .execute(
                "UPDATE policy_activations
                 SET active = 0, deactivated_at = ?3
                 WHERE repository_id = ?1 AND activation_id = ?2 AND active = 1",
                params![repository_id, activation_id, deactivated_at],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "policy activation",
                    "deactivate policy",
                    error,
                )
            })?;
        if changed == 0 {
            return Err(StateError::NotFound {
                entity: "active policy activation",
                repository_id: repository_id.to_owned(),
                object_id: activation_id.to_owned(),
            });
        }
        self.require(repository_id, activation_id)
    }
}

fn row_to_policy_activation(row: &rusqlite::Row<'_>) -> rusqlite::Result<PolicyActivationRecord> {
    Ok(PolicyActivationRecord {
        repository_id: row.get(0)?,
        activation_id: row.get(1)?,
        config_hash: row.get(2)?,
        policy_tuple_hash: row.get(3)?,
        event_type: row.get(4)?,
        destination_alias: row.get(5)?,
        severity: row.get(6)?,
        activated_at: row.get(7)?,
        deactivated_at: row.get(8)?,
        active: row.get::<_, i64>(9)? == 1,
    })
}

/// Delivery attempt persistence.
pub struct DeliveryAttemptRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> DeliveryAttemptRepository<'conn> {
    /// Creates a delivery attempt handle.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Records one attempt row.  The caller owns the transaction; no network
    /// operation is performed here.
    pub fn record(&self, input: &DeliveryAttemptInput) -> StateResult<DeliveryAttemptRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("attempt", &input.attempt_id)?;
        validate_id("draft", &input.draft_id)?;
        validate_id("claim nonce", &input.claim_nonce)?;
        validate_timestamp(&input.claimed_at)?;
        if input.revision <= 0 || input.attempt_number <= 0 {
            return Err(StateError::InvalidIdentifier {
                kind: "delivery attempt number",
            });
        }
        ensure_revision(
            self.connection,
            &input.repository_id,
            &input.draft_id,
            input.revision,
            self.path.as_deref(),
        )?;
        self.connection
            .execute(
                "INSERT INTO delivery_attempts(
                    repository_id, attempt_id, draft_id, revision, attempt_number, claim_nonce,
                    state, claimed_at, completed_at, remote_message_id, failure_code
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    input.repository_id,
                    input.attempt_id,
                    input.draft_id,
                    input.revision,
                    input.attempt_number,
                    input.claim_nonce,
                    input.state,
                    input.claimed_at,
                    input.completed_at,
                    input.remote_message_id,
                    input.failure_code
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "delivery attempt",
                    "record attempt",
                    error,
                )
            })?;
        Ok(input.clone())
    }

    /// Atomically claims a revision when no accepted, unknown, or active claim
    /// exists.  The check and insert are intended to run in one transaction.
    pub fn claim(&self, input: &DeliveryAttemptInput) -> StateResult<DeliveryAttemptRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        let existing = self
            .connection
            .query_row(
                "SELECT attempt_id FROM delivery_attempts
                 WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3
                   AND state IN ('claimed', 'unknown', 'accepted')
                 ORDER BY attempt_number ASC LIMIT 1",
                params![input.repository_id, input.draft_id, input.revision],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "delivery attempt",
                    "check claim",
                    error,
                )
            })?;
        if let Some(existing) = existing {
            return Err(StateError::Constraint {
                entity: "delivery claim",
                message: format!("revision already has an active or terminal attempt {existing}"),
            });
        }
        self.record(input)
    }

    /// Reads an attempt by exact repository and attempt ID.
    pub fn get(
        &self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
    ) -> StateResult<Option<DeliveryAttemptRecord>> {
        let repository_id = repository_id.as_ref();
        let attempt_id = attempt_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("attempt", attempt_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, attempt_id, draft_id, revision, attempt_number,
                        claim_nonce, state, claimed_at, completed_at, remote_message_id, failure_code
                 FROM delivery_attempts
                 WHERE repository_id = ?1 AND attempt_id = ?2",
                params![repository_id, attempt_id],
                row_to_delivery_attempt,
            )
            .optional()
            .map_err(|error| map_repository_error(self.path.as_deref(), "delivery attempt", "read attempt", error))
    }

    /// Updates the state of one exact attempt.
    pub fn set_state(
        &self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
        state: impl AsRef<str>,
        completed_at: Option<String>,
        remote_message_id: Option<String>,
        failure_code: Option<String>,
    ) -> StateResult<DeliveryAttemptRecord> {
        let repository_id = repository_id.as_ref();
        let attempt_id = attempt_id.as_ref();
        let state = state.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("attempt", attempt_id)?;
        let changed = self
            .connection
            .execute(
                "UPDATE delivery_attempts
                 SET state = ?3, completed_at = ?4, remote_message_id = ?5, failure_code = ?6
                 WHERE repository_id = ?1 AND attempt_id = ?2",
                params![
                    repository_id,
                    attempt_id,
                    state,
                    completed_at,
                    remote_message_id,
                    failure_code
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "delivery attempt",
                    "set attempt state",
                    error,
                )
            })?;
        if changed == 0 {
            return Err(StateError::NotFound {
                entity: "delivery attempt",
                repository_id: repository_id.to_owned(),
                object_id: attempt_id.to_owned(),
            });
        }
        self.get(repository_id, attempt_id)?
            .ok_or_else(|| StateError::NotFound {
                entity: "delivery attempt",
                repository_id: repository_id.to_owned(),
                object_id: attempt_id.to_owned(),
            })
    }
}

fn row_to_delivery_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryAttemptRecord> {
    Ok(DeliveryAttemptRecord {
        repository_id: row.get(0)?,
        attempt_id: row.get(1)?,
        draft_id: row.get(2)?,
        revision: row.get(3)?,
        attempt_number: row.get(4)?,
        claim_nonce: row.get(5)?,
        state: row.get(6)?,
        claimed_at: row.get(7)?,
        completed_at: row.get(8)?,
        remote_message_id: row.get(9)?,
        failure_code: row.get(10)?,
    })
}

/// Inbound snapshot, cursor, and local lifecycle persistence.
pub struct InboundRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> InboundRepository<'conn> {
    /// Creates an inbound repository handle.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Stores a first snapshot and a separate current snapshot.  Repeating the
    /// exact first snapshot is idempotent; a different first observation is
    /// rejected by the immutable first-snapshot invariant.
    pub fn store_item(
        &self,
        item: &InboundItemInput,
        current: &InboundCurrentSnapshotInput,
    ) -> StateResult<InboundItemRecord> {
        ensure_repository(self.connection, &item.repository_id)?;
        validate_id("item", &item.item_id)?;
        validate_id("channel", &item.channel_id)?;
        validate_id("author", &item.author_id)?;
        validate_timestamp(&item.first_observed_at)?;
        validate_timestamp(&item.created_at)?;
        validate_current(current, &item.repository_id, &item.item_id)?;

        if let Some(existing) = self.item(&item.repository_id, &item.item_id)? {
            if existing.first_content != item.first_content
                || existing.channel_id != item.channel_id
                || existing.author_id != item.author_id
                || existing.first_attachments_json != item.first_attachments_json
                || existing.first_observed_at != item.first_observed_at
                || existing.created_at != item.created_at
            {
                return Err(StateError::Constraint {
                    entity: "inbound first snapshot",
                    message: "a first snapshot cannot be replaced".to_owned(),
                });
            }
        } else {
            self.connection
                .execute(
                    "INSERT INTO inbound_items(
                        repository_id, item_id, channel_id, author_id, first_content,
                        first_attachments_json, first_observed_at, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        item.repository_id,
                        item.item_id,
                        item.channel_id,
                        item.author_id,
                        item.first_content,
                        item.first_attachments_json,
                        item.first_observed_at,
                        item.created_at
                    ],
                )
                .map_err(|error| {
                    map_repository_error(
                        self.path.as_deref(),
                        "inbound item",
                        "store first snapshot",
                        error,
                    )
                })?;
            self.insert_transition_if_absent(&InboundTransitionInput {
                repository_id: item.repository_id.clone(),
                transition_id: format!("created:{}:{}", item.repository_id, item.item_id),
                item_id: item.item_id.clone(),
                transition_type: "created".to_owned(),
                content: Some(item.first_content.clone()),
                occurred_at: item.first_observed_at.clone(),
                metadata_json: "{}".to_owned(),
            })?;
        }
        self.upsert_current(current)?;
        self.item(&item.repository_id, &item.item_id)?
            .ok_or_else(|| StateError::NotFound {
                entity: "inbound item",
                repository_id: item.repository_id.clone(),
                object_id: item.item_id.clone(),
            })
    }

    /// Reads an inbound item's immutable first snapshot.
    pub fn item(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<InboundItemRecord>> {
        let repository_id = repository_id.as_ref();
        let item_id = item_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("item", item_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, item_id, channel_id, author_id, first_content,
                        first_attachments_json, first_observed_at, created_at
                 FROM inbound_items WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                row_to_inbound_item,
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound item",
                    "read first snapshot",
                    error,
                )
            })
    }

    /// Reads the separate current snapshot or deletion marker.
    pub fn current(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<InboundCurrentSnapshotRecord>> {
        let repository_id = repository_id.as_ref();
        let item_id = item_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("item", item_id)?;
        self.connection
            .query_row(
                "SELECT repository_id, item_id, current_content, current_attachments_json,
                        deleted, observed_at
                 FROM inbound_current_snapshots
                 WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                row_to_inbound_current,
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound current snapshot",
                    "read current snapshot",
                    error,
                )
            })
    }

    /// Records an edit/delete transition and optionally replaces only the
    /// current snapshot.  The first snapshot is never touched.
    pub fn record_transition(
        &self,
        input: &InboundTransitionInput,
        current: Option<&InboundCurrentSnapshotInput>,
    ) -> StateResult<InboundTransitionRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("transition", &input.transition_id)?;
        validate_id("item", &input.item_id)?;
        validate_timestamp(&input.occurred_at)?;
        if !matches!(
            input.transition_type.as_str(),
            "created" | "edited" | "deleted" | "acknowledged" | "archived" | "reply_linked"
        ) {
            return Err(StateError::InvalidIdentifier {
                kind: "inbound transition",
            });
        }
        ensure_item(
            self.connection,
            &input.repository_id,
            &input.item_id,
            self.path.as_deref(),
        )?;
        if let Some(snapshot) = current {
            validate_current(snapshot, &input.repository_id, &input.item_id)?;
            self.upsert_current(snapshot)?;
        } else if input.transition_type == "deleted" {
            self.upsert_current(&InboundCurrentSnapshotInput::new(
                input.repository_id.clone(),
                input.item_id.clone(),
                None,
                true,
                input.occurred_at.clone(),
            ))?;
        } else if input.transition_type == "edited" {
            self.upsert_current(&InboundCurrentSnapshotInput::new(
                input.repository_id.clone(),
                input.item_id.clone(),
                input.content.clone(),
                false,
                input.occurred_at.clone(),
            ))?;
        }
        self.insert_transition_if_absent(input)?;
        Ok(input.clone())
    }

    /// Advances a repository/alias cursor monotonically.  A repeated value is
    /// idempotent; a lower value is rejected.
    pub fn advance_cursor(&self, input: &InboundCursorInput) -> StateResult<InboundCursorRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("alias", &input.alias)?;
        validate_id("cursor", &input.cursor)?;
        validate_timestamp(&input.updated_at)?;
        if let Some(existing) = self.cursor(&input.repository_id, &input.alias)? {
            match cursor_order(&existing.cursor, &input.cursor) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => return Ok(existing),
                std::cmp::Ordering::Greater => {
                    return Err(StateError::Constraint {
                        entity: "inbound cursor",
                        message: "cursor cannot move backwards".to_owned(),
                    });
                }
            }
            self.connection
                .execute(
                    "UPDATE inbound_cursors SET cursor = ?3, updated_at = ?4
                     WHERE repository_id = ?1 AND alias = ?2",
                    params![
                        input.repository_id,
                        input.alias,
                        input.cursor,
                        input.updated_at
                    ],
                )
                .map_err(|error| {
                    map_repository_error(
                        self.path.as_deref(),
                        "inbound cursor",
                        "advance cursor",
                        error,
                    )
                })?;
        } else {
            self.connection
                .execute(
                    "INSERT INTO inbound_cursors(repository_id, alias, cursor, updated_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        input.repository_id,
                        input.alias,
                        input.cursor,
                        input.updated_at
                    ],
                )
                .map_err(|error| {
                    map_repository_error(
                        self.path.as_deref(),
                        "inbound cursor",
                        "create cursor",
                        error,
                    )
                })?;
        }
        self.cursor(&input.repository_id, &input.alias)?
            .ok_or_else(|| StateError::NotFound {
                entity: "inbound cursor",
                repository_id: input.repository_id.clone(),
                object_id: input.alias.clone(),
            })
    }

    /// Reads one alias cursor.
    pub fn cursor(
        &self,
        repository_id: impl AsRef<str>,
        alias: impl AsRef<str>,
    ) -> StateResult<Option<InboundCursorRecord>> {
        let repository_id = repository_id.as_ref();
        let alias = alias.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("alias", alias)?;
        self.connection
            .query_row(
                "SELECT repository_id, alias, cursor, updated_at
                 FROM inbound_cursors WHERE repository_id = ?1 AND alias = ?2",
                params![repository_id, alias],
                |row| {
                    Ok(InboundCursorRecord {
                        repository_id: row.get(0)?,
                        alias: row.get(1)?,
                        cursor: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(self.path.as_deref(), "inbound cursor", "read cursor", error)
            })
    }

    /// Acknowledges items locally and idempotently.  No remote operation is
    /// possible through this API.
    pub fn acknowledge(
        &self,
        repository_id: &str,
        item_ids: &[String],
        acknowledged_at: &str,
    ) -> StateResult<Vec<AcknowledgementRecord>> {
        ensure_repository(self.connection, repository_id)?;
        validate_timestamp(acknowledged_at)?;
        let mut records = Vec::with_capacity(item_ids.len());
        for item_id in item_ids {
            validate_id("item", item_id)?;
            ensure_item(
                self.connection,
                repository_id,
                item_id,
                self.path.as_deref(),
            )?;
            let changed = self
                .connection
                .execute(
                    "INSERT OR IGNORE INTO inbound_acknowledgements(repository_id, item_id, acknowledged_at)
                     VALUES (?1, ?2, ?3)",
                    params![repository_id, item_id, acknowledged_at],
                )
                .map_err(|error| {
                    map_repository_error(
                        self.path.as_deref(),
                        "inbound acknowledgement",
                        "acknowledge item",
                        error,
                    )
                })?;
            if changed == 1 {
                self.insert_transition_if_absent(&InboundTransitionInput::new(
                    repository_id.to_owned(),
                    format!("acknowledged:{repository_id}:{item_id}"),
                    item_id.clone(),
                    "acknowledged",
                    None,
                    acknowledged_at,
                ))?;
            }
            let record = self
                .acknowledgement(repository_id, item_id)?
                .ok_or_else(|| StateError::NotFound {
                    entity: "inbound acknowledgement",
                    repository_id: repository_id.to_owned(),
                    object_id: item_id.clone(),
                })?;
            records.push(record);
        }
        Ok(records)
    }

    /// Archives items locally and idempotently.  No remote operation is
    /// possible through this API.
    pub fn archive(
        &self,
        repository_id: &str,
        item_ids: &[String],
        archived_at: &str,
    ) -> StateResult<Vec<ArchiveRecord>> {
        ensure_repository(self.connection, repository_id)?;
        validate_timestamp(archived_at)?;
        let mut records = Vec::with_capacity(item_ids.len());
        for item_id in item_ids {
            validate_id("item", item_id)?;
            ensure_item(
                self.connection,
                repository_id,
                item_id,
                self.path.as_deref(),
            )?;
            let changed = self
                .connection
                .execute(
                    "INSERT OR IGNORE INTO inbound_archives(repository_id, item_id, archived_at)
                     VALUES (?1, ?2, ?3)",
                    params![repository_id, item_id, archived_at],
                )
                .map_err(|error| {
                    map_repository_error(
                        self.path.as_deref(),
                        "inbound archive",
                        "archive item",
                        error,
                    )
                })?;
            if changed == 1 {
                self.insert_transition_if_absent(&InboundTransitionInput::new(
                    repository_id.to_owned(),
                    format!("archived:{repository_id}:{item_id}"),
                    item_id.clone(),
                    "archived",
                    None,
                    archived_at,
                ))?;
            }
            let record = self
                .archive_record(repository_id, item_id)?
                .ok_or_else(|| StateError::NotFound {
                    entity: "inbound archive",
                    repository_id: repository_id.to_owned(),
                    object_id: item_id.clone(),
                })?;
            records.push(record);
        }
        Ok(records)
    }

    /// Links an inbound item to a reply draft in the same repository.
    pub fn link_reply(&self, input: &ReplyLinkInput) -> StateResult<ReplyLinkRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("item", &input.item_id)?;
        validate_id("reply draft", &input.reply_draft_id)?;
        validate_timestamp(&input.linked_at)?;
        ensure_item(
            self.connection,
            &input.repository_id,
            &input.item_id,
            self.path.as_deref(),
        )?;
        ensure_draft(
            self.connection,
            &input.repository_id,
            &input.reply_draft_id,
            self.path.as_deref(),
        )?;
        if let Some(existing) = self.reply_link(&input.repository_id, &input.item_id)? {
            if existing.reply_draft_id == input.reply_draft_id {
                return Ok(existing);
            }
            return Err(StateError::Constraint {
                entity: "inbound reply link",
                message: "inbound item already has a different reply draft".to_owned(),
            });
        }
        self.connection
            .execute(
                "INSERT INTO inbound_reply_links(repository_id, item_id, reply_draft_id, linked_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    input.repository_id,
                    input.item_id,
                    input.reply_draft_id,
                    input.linked_at
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound reply link",
                    "link reply",
                    error,
                )
            })?;
        self.insert_transition_if_absent(&InboundTransitionInput {
            repository_id: input.repository_id.clone(),
            transition_id: format!("reply_linked:{}:{}", input.repository_id, input.item_id),
            item_id: input.item_id.clone(),
            transition_type: "reply_linked".to_owned(),
            content: None,
            occurred_at: input.linked_at.clone(),
            metadata_json: "{}".to_owned(),
        })?;
        self.reply_link(&input.repository_id, &input.item_id)?
            .ok_or_else(|| StateError::NotFound {
                entity: "inbound reply link",
                repository_id: input.repository_id.clone(),
                object_id: input.item_id.clone(),
            })
    }

    /// Reads an acknowledgement by exact repository and item.
    pub fn acknowledgement(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<AcknowledgementRecord>> {
        let repository_id = repository_id.as_ref();
        let item_id = item_id.as_ref();
        self.connection
            .query_row(
                "SELECT repository_id, item_id, acknowledged_at
                 FROM inbound_acknowledgements WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                |row| {
                    Ok(AcknowledgementRecord {
                        repository_id: row.get(0)?,
                        item_id: row.get(1)?,
                        acknowledged_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound acknowledgement",
                    "read acknowledgement",
                    error,
                )
            })
    }

    /// Reads an archive marker by exact repository and item.
    pub fn archive_record(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<ArchiveRecord>> {
        let repository_id = repository_id.as_ref();
        let item_id = item_id.as_ref();
        self.connection
            .query_row(
                "SELECT repository_id, item_id, archived_at
                 FROM inbound_archives WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                |row| {
                    Ok(ArchiveRecord {
                        repository_id: row.get(0)?,
                        item_id: row.get(1)?,
                        archived_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound archive",
                    "read archive",
                    error,
                )
            })
    }

    /// Reads a reply link by exact repository and inbound item.
    pub fn reply_link(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Option<ReplyLinkRecord>> {
        let repository_id = repository_id.as_ref();
        let item_id = item_id.as_ref();
        self.connection
            .query_row(
                "SELECT repository_id, item_id, reply_draft_id, linked_at
                 FROM inbound_reply_links WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                |row| {
                    Ok(ReplyLinkRecord {
                        repository_id: row.get(0)?,
                        item_id: row.get(1)?,
                        reply_draft_id: row.get(2)?,
                        linked_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound reply link",
                    "read reply link",
                    error,
                )
            })
    }

    /// Lists transitions for one exact item in observation order.
    pub fn transitions(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> StateResult<Vec<InboundTransitionRecord>> {
        let repository_id = repository_id.as_ref();
        let item_id = item_id.as_ref();
        let mut statement = self
            .connection
            .prepare(
                "SELECT repository_id, transition_id, item_id, transition_type, content,
                        occurred_at, metadata_json
                 FROM inbound_item_transitions
                 WHERE repository_id = ?1 AND item_id = ?2
                 ORDER BY occurred_at ASC, transition_id ASC",
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound transition",
                    "prepare transitions",
                    error,
                )
            })?;
        let rows = statement
            .query_map(params![repository_id, item_id], |row| {
                Ok(InboundTransitionRecord {
                    repository_id: row.get(0)?,
                    transition_id: row.get(1)?,
                    item_id: row.get(2)?,
                    transition_type: row.get(3)?,
                    content: row.get(4)?,
                    occurred_at: row.get(5)?,
                    metadata_json: row.get(6)?,
                })
            })
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound transition",
                    "query transitions",
                    error,
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            map_repository_error(
                self.path.as_deref(),
                "inbound transition",
                "read transitions",
                error,
            )
        })
    }

    fn upsert_current(&self, input: &InboundCurrentSnapshotInput) -> StateResult<()> {
        self.connection
            .execute(
                "INSERT INTO inbound_current_snapshots(
                    repository_id, item_id, current_content, current_attachments_json,
                    deleted, observed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(repository_id, item_id) DO UPDATE SET
                    current_content = excluded.current_content,
                    current_attachments_json = excluded.current_attachments_json,
                    deleted = excluded.deleted,
                    observed_at = excluded.observed_at",
                params![
                    input.repository_id,
                    input.item_id,
                    input.current_content,
                    input.current_attachments_json,
                    i64::from(input.deleted),
                    input.observed_at
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound current snapshot",
                    "store current snapshot",
                    error,
                )
            })?;
        Ok(())
    }

    fn insert_transition_if_absent(&self, input: &InboundTransitionInput) -> StateResult<()> {
        self.connection
            .execute(
                "INSERT OR IGNORE INTO inbound_item_transitions(
                    repository_id, transition_id, item_id, transition_type, content,
                    occurred_at, metadata_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    input.repository_id,
                    input.transition_id,
                    input.item_id,
                    input.transition_type,
                    input.content,
                    input.occurred_at,
                    input.metadata_json
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "inbound transition",
                    "record transition",
                    error,
                )
            })?;
        Ok(())
    }
}

fn validate_current(
    input: &InboundCurrentSnapshotInput,
    repository_id: &str,
    item_id: &str,
) -> StateResult<()> {
    validate_id("repository", &input.repository_id)?;
    validate_id("item", &input.item_id)?;
    validate_timestamp(&input.observed_at)?;
    if input.repository_id != repository_id || input.item_id != item_id {
        return Err(StateError::Constraint {
            entity: "inbound current snapshot",
            message: "current snapshot scope does not match the item".to_owned(),
        });
    }
    if input.deleted && input.current_content.is_some() {
        return Err(StateError::Constraint {
            entity: "inbound current snapshot",
            message: "a deleted marker cannot carry current content".to_owned(),
        });
    }
    Ok(())
}

fn ensure_item(
    connection: &Connection,
    repository_id: &str,
    item_id: &str,
    path: Option<&Path>,
) -> StateResult<()> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM inbound_items WHERE repository_id = ?1 AND item_id = ?2",
            params![repository_id, item_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            map_sqlite_error(
                path,
                "verify inbound item scope",
                Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
                error,
            )
        })?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(StateError::NotFound {
            entity: "inbound item",
            repository_id: repository_id.to_owned(),
            object_id: item_id.to_owned(),
        })
    }
}

fn row_to_inbound_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<InboundItemRecord> {
    Ok(InboundItemRecord {
        repository_id: row.get(0)?,
        item_id: row.get(1)?,
        channel_id: row.get(2)?,
        author_id: row.get(3)?,
        first_content: row.get(4)?,
        first_attachments_json: row.get(5)?,
        first_observed_at: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn row_to_inbound_current(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<InboundCurrentSnapshotRecord> {
    Ok(InboundCurrentSnapshotRecord {
        repository_id: row.get(0)?,
        item_id: row.get(1)?,
        current_content: row.get(2)?,
        current_attachments_json: row.get(3)?,
        deleted: row.get::<_, i64>(4)? == 1,
        observed_at: row.get(5)?,
    })
}

fn cursor_order(previous: &str, next: &str) -> std::cmp::Ordering {
    if let (Ok(left), Ok(right)) = (previous.parse::<u128>(), next.parse::<u128>()) {
        return left.cmp(&right);
    }
    previous.cmp(next)
}

/// Append-only audit persistence.  This type intentionally has no update or
/// delete operation; REPO-AUDIT-2 owns bounded presentation queries later.
pub struct AuditRepository<'conn> {
    connection: &'conn Connection,
    path: Option<PathBuf>,
}

impl<'conn> AuditRepository<'conn> {
    /// Creates an audit repository handle.
    #[must_use]
    pub fn new(connection: &'conn Connection, path: Option<PathBuf>) -> Self {
        Self { connection, path }
    }

    /// Appends one event in the caller's current transaction.
    pub fn append(&self, input: &AuditEventInput) -> StateResult<AuditEventRecord> {
        ensure_repository(self.connection, &input.repository_id)?;
        validate_id("event", &input.event_id)?;
        validate_id("object type", &input.object_type)?;
        validate_id("object", &input.object_id)?;
        validate_id("transition", &input.transition)?;
        validate_id("actor", &input.actor_kind)?;
        validate_id("outcome", &input.outcome)?;
        validate_timestamp(&input.occurred_at)?;
        self.connection
            .execute(
                "INSERT INTO audit_events(
                    repository_id, event_id, object_type, object_id, transition,
                    occurred_at, actor_kind, outcome, metadata_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    input.repository_id,
                    input.event_id,
                    input.object_type,
                    input.object_id,
                    input.transition,
                    input.occurred_at,
                    input.actor_kind,
                    input.outcome,
                    input.metadata_json
                ],
            )
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "audit event",
                    "append audit event",
                    error,
                )
            })?;
        self.get(&input.repository_id, &input.event_id)?
            .ok_or_else(|| StateError::NotFound {
                entity: "audit event",
                repository_id: input.repository_id.clone(),
                object_id: input.event_id.clone(),
            })
    }

    /// Reads one event by exact repository and event ID.
    pub fn get(
        &self,
        repository_id: impl AsRef<str>,
        event_id: impl AsRef<str>,
    ) -> StateResult<Option<AuditEventRecord>> {
        let repository_id = repository_id.as_ref();
        let event_id = event_id.as_ref();
        validate_id("repository", repository_id)?;
        validate_id("event", event_id)?;
        self.connection
            .query_row(
                "SELECT audit_id, repository_id, event_id, object_type, object_id,
                        transition, occurred_at, actor_kind, outcome, metadata_json
                 FROM audit_events WHERE repository_id = ?1 AND event_id = ?2",
                params![repository_id, event_id],
                |row| {
                    Ok(AuditEventRecord {
                        audit_id: row.get(0)?,
                        repository_id: row.get(1)?,
                        event_id: row.get(2)?,
                        object_type: row.get(3)?,
                        object_id: row.get(4)?,
                        transition: row.get(5)?,
                        occurred_at: row.get(6)?,
                        actor_kind: row.get(7)?,
                        outcome: row.get(8)?,
                        metadata_json: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                map_repository_error(
                    self.path.as_deref(),
                    "audit event",
                    "read audit event",
                    error,
                )
            })
    }
}

fn map_repository_error(
    path: Option<&Path>,
    entity: &'static str,
    operation: &'static str,
    error: RusqliteError,
) -> StateError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    if lower.contains("constraint")
        || lower.contains("unique")
        || lower.contains("foreign key")
        || lower.contains("not null")
        || lower.contains("check constraint")
    {
        StateError::Constraint { entity, message }
    } else {
        map_sqlite_error(
            path,
            operation,
            Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            error,
        )
    }
}
