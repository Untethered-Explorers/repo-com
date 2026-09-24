//! Read-only SQLite integrity and filesystem verification.
//!
//! The verifier opens an existing database with SQLite read-only flags. It
//! never calls the state store's migration path, never creates a missing file,
//! and never changes permissions. A report describes observed failures and
//! safe operator remediation; it is not a repair result.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use repo_com_state::{PermissionModel, SCHEMA_VERSION, is_user_only};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

const VERIFY_BUSY_TIMEOUT_MS: u64 = 250;
const MAX_FOREIGN_KEY_VIOLATIONS: usize = 101;

/// The migration version understood by this verifier.
pub const EXPECTED_MIGRATION_VERSION: i64 = SCHEMA_VERSION;
/// Required v1 disclosure carried by verification output and operator renderers.
pub const V1_STORAGE_RESIDUAL_RISK: &str = "v1 state is protected by user-only filesystem permissions, not encryption at rest; local account access, backups, and filesystem snapshots may read retained content.";

/// Stable result of one verifier check.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check ran and passed.
    Passed,
    /// The check ran and found a failure.
    Failed,
    /// The check could not run safely.
    Unavailable,
}

/// Quick-check result with no raw SQLite diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuickCheckVerification {
    /// Structured status.
    pub status: CheckStatus,
    /// Convenience boolean equivalent of `status == Passed`.
    pub passed: bool,
    /// Stable machine-readable category.
    pub code: &'static str,
}

/// Foreign-key verification result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ForeignKeyVerification {
    /// Structured status.
    pub status: CheckStatus,
    /// Convenience boolean equivalent of `status == Passed`.
    pub passed: bool,
    /// Whether the connection-local foreign-key setting was enabled.
    pub enabled: bool,
    /// Number of violations observed, capped at the bounded scan limit.
    pub violation_count: usize,
    /// Whether the scan stopped at its bound.
    pub bounded: bool,
    /// Stable machine-readable category.
    pub code: &'static str,
}

/// Migration-version verification result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MigrationVerification {
    /// Structured status.
    pub status: CheckStatus,
    /// Convenience boolean equivalent of `status == Passed`.
    pub passed: bool,
    /// Expected SQLite `user_version`.
    pub expected: i64,
    /// Observed SQLite `user_version`, when readable.
    pub found: Option<i64>,
    /// Stable machine-readable category.
    pub code: &'static str,
}

/// Repository-scope verification result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RepositoryScopeVerification {
    /// Structured status.
    pub status: CheckStatus,
    /// Convenience boolean equivalent of `status == Passed`.
    pub passed: bool,
    /// Exact requested repository scope.
    pub repository_id: String,
    /// Whether the repository identity row exists locally.
    pub repository_found: bool,
    /// Stable machine-readable category.
    pub code: &'static str,
}

/// Filesystem permission verification result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PermissionVerification {
    /// Structured status.
    pub status: CheckStatus,
    /// Convenience boolean equivalent of `status == Passed`.
    pub passed: bool,
    /// Current platform permission model.
    pub model: String,
    /// Whether the path is a regular file rather than a symlink.
    pub file_present: bool,
    /// Whether the database file is user-only on this platform.
    pub file_user_only: bool,
    /// Whether the containing directory is user-only on this platform.
    pub parent_user_only: bool,
    /// Stable machine-readable category.
    pub code: &'static str,
}

/// One safe issue and remediation pair.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerificationIssue {
    /// Stable machine-readable issue code.
    pub code: &'static str,
    /// Short non-sensitive explanation.
    pub summary: &'static str,
    /// Safe operator action; this verifier performs none of it.
    pub remediation: &'static str,
}

/// Complete read-only state verification report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StateVerificationReport {
    /// Repository scope requested by the caller.
    pub repository_id: String,
    /// True only when every required check passed.
    pub healthy: bool,
    /// Always true for this API; retained to make the boundary explicit.
    pub read_only: bool,
    /// The v1 local-state privacy boundary and its residual risk.
    pub privacy_disclosure: String,
    /// Whether the connection entered its query-only read boundary.
    pub connection_read_only: bool,
    /// SQLite quick-check result.
    pub quick_check: QuickCheckVerification,
    /// Foreign-key setting and violation result.
    pub foreign_keys: ForeignKeyVerification,
    /// Migration-version result.
    pub migration: MigrationVerification,
    /// Repository-scope result.
    pub repository_scope: RepositoryScopeVerification,
    /// Filesystem-permission result.
    pub filesystem_permissions: PermissionVerification,
    /// Top-level convenience booleans for renderers.
    pub quick_check_ok: bool,
    /// Top-level convenience boolean for foreign keys.
    pub foreign_keys_ok: bool,
    /// Top-level convenience boolean for migration.
    pub migration_ok: bool,
    /// Top-level convenience boolean for repository scope.
    pub repository_scope_ok: bool,
    /// Top-level convenience boolean for filesystem permissions.
    pub filesystem_permissions_ok: bool,
    /// All observed issues in deterministic check order.
    pub issues: Vec<VerificationIssue>,
    /// Deduplicated safe remediation text in deterministic order.
    pub remediation: Vec<String>,
}

impl StateVerificationReport {
    /// Returns whether all required checks passed.
    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        self.healthy
    }

    /// Returns whether the verifier was read-only.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Returns the required v1 storage privacy disclosure.
    #[must_use]
    pub fn residual_risk(&self) -> &str {
        &self.privacy_disclosure
    }

    /// Returns whether quick-check passed.
    #[must_use]
    pub const fn quick_check_passed(&self) -> bool {
        self.quick_check_ok
    }

    /// Returns whether foreign-key verification passed.
    #[must_use]
    pub const fn foreign_keys_passed(&self) -> bool {
        self.foreign_keys_ok
    }

    /// Returns whether migration verification passed.
    #[must_use]
    pub const fn migration_passed(&self) -> bool {
        self.migration_ok
    }

    /// Returns whether repository-scope verification passed.
    #[must_use]
    pub const fn repository_scope_passed(&self) -> bool {
        self.repository_scope_ok
    }

    /// Returns whether filesystem-permission verification passed.
    #[must_use]
    pub const fn filesystem_permissions_passed(&self) -> bool {
        self.filesystem_permissions_ok
    }
}

/// Input to one read-only state verification.
#[derive(Clone, Eq, PartialEq)]
pub struct VerificationRequest {
    /// Existing local SQLite database path.
    pub database_path: PathBuf,
    /// Exact repository scope to verify.
    pub repository_id: String,
    /// Expected SQLite `user_version`.
    pub expected_migration: i64,
}

impl fmt::Debug for VerificationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerificationRequest")
            .field("path_present", &!self.database_path.as_os_str().is_empty())
            .field(
                "repository_valid",
                &valid_repository_id(&self.repository_id),
            )
            .field("expected_migration", &self.expected_migration)
            .finish()
    }
}

impl VerificationRequest {
    /// Creates a request using the current schema version.
    #[must_use]
    pub fn new(database_path: impl AsRef<Path>, repository_id: impl Into<String>) -> Self {
        Self {
            database_path: database_path.as_ref().to_path_buf(),
            repository_id: repository_id.into(),
            expected_migration: EXPECTED_MIGRATION_VERSION,
        }
    }

    /// Changes the expected migration version for a compatibility check.
    #[must_use]
    pub const fn with_expected_migration(mut self, expected_migration: i64) -> Self {
        self.expected_migration = expected_migration;
        self
    }

    /// Compatibility alias for [`Self::with_expected_migration`].
    #[must_use]
    pub const fn with_schema_version(self, schema_version: i64) -> Self {
        self.with_expected_migration(schema_version)
    }
}

/// Stateless read-only state verifier.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StateVerifier;

impl StateVerifier {
    /// Creates a verifier.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the v1 storage privacy disclosure for operator-facing output.
    #[must_use]
    pub const fn residual_risk() -> &'static str {
        V1_STORAGE_RESIDUAL_RISK
    }

    /// Verifies one existing database without creating or changing it.
    #[must_use]
    pub fn verify(&self, request: &VerificationRequest) -> StateVerificationReport {
        let repository_id = safe_repository_id(&request.repository_id);
        let permissions = verify_permissions(&request.database_path);
        let mut issues = Vec::new();
        let mut remediation = Vec::new();

        if !permissions.passed {
            push_issue(
                &mut issues,
                &mut remediation,
                "filesystem-permissions",
                "database filesystem permissions are not user-only",
                "Have an operator restore user-only permissions through a separately approved process; this verifier does not change them.",
            );
        }

        let connection = match open_read_only(&request.database_path) {
            Ok(connection) => Some(connection),
            Err(_) => {
                push_issue(
                    &mut issues,
                    &mut remediation,
                    "database-open",
                    "the existing database could not be opened read-only",
                    "Preserve the database and sidecars and investigate the local SQLite error; do not delete or recreate state.",
                );
                None
            }
        };

        let Some(connection) = connection else {
            let quick_check = QuickCheckVerification {
                status: CheckStatus::Unavailable,
                passed: false,
                code: "quick-check-unavailable",
            };
            let foreign_keys = ForeignKeyVerification {
                status: CheckStatus::Unavailable,
                passed: false,
                enabled: false,
                violation_count: 0,
                bounded: true,
                code: "foreign-key-check-unavailable",
            };
            let migration = MigrationVerification {
                status: CheckStatus::Unavailable,
                passed: false,
                expected: request.expected_migration,
                found: None,
                code: "migration-unavailable",
            };
            let scope = RepositoryScopeVerification {
                status: CheckStatus::Unavailable,
                passed: false,
                repository_id: repository_id.clone(),
                repository_found: false,
                code: "repository-scope-unavailable",
            };
            return build_report(
                repository_id,
                quick_check,
                foreign_keys,
                migration,
                scope,
                permissions,
                false,
                issues,
                remediation,
            );
        };

        let connection_read_only = configure_read_only_connection(&connection).is_ok();
        if !connection_read_only {
            push_issue(
                &mut issues,
                &mut remediation,
                "connection-read-only",
                "SQLite could not enter the read-only verification boundary",
                "Preserve the database and use an operator-approved SQLite inspection process; do not repair or recreate it here.",
            );
        }

        let quick_check = verify_quick_check(&connection);
        if !quick_check.passed {
            push_issue(
                &mut issues,
                &mut remediation,
                quick_check.code,
                "SQLite quick_check did not pass",
                "Preserve the database and sidecars and investigate with an operator-approved SQLite process; this verifier never repairs or recreates state.",
            );
        }

        let foreign_keys = verify_foreign_keys(&connection);
        if !foreign_keys.passed {
            push_issue(
                &mut issues,
                &mut remediation,
                foreign_keys.code,
                "SQLite foreign-key verification did not pass",
                "Preserve the database and resolve the local integrity issue through an operator-approved process; no automatic repair is attempted.",
            );
        }

        let migration = verify_migration(&connection, request.expected_migration);
        if !migration.passed {
            push_issue(
                &mut issues,
                &mut remediation,
                migration.code,
                "SQLite migration version did not match the expected schema",
                "Keep the existing database and consult an operator-approved upgrade procedure; this verifier never migrates or rolls back state.",
            );
        }

        let scope = verify_repository_scope(&connection, &repository_id);
        if !scope.passed {
            push_issue(
                &mut issues,
                &mut remediation,
                scope.code,
                "the requested repository scope is not present in local state",
                "Select a repository that exists in this local database; do not copy or expose rows from another repository.",
            );
        }

        build_report(
            repository_id,
            quick_check,
            foreign_keys,
            migration,
            scope,
            permissions,
            connection_read_only,
            issues,
            remediation,
        )
    }

    /// Compatibility alias for [`Self::verify`].
    #[must_use]
    pub fn verify_request(&self, request: &VerificationRequest) -> StateVerificationReport {
        self.verify(request)
    }

    /// Convenience path-based verification using the current schema version.
    #[must_use]
    pub fn verify_path(
        &self,
        database_path: impl AsRef<Path>,
        repository_id: impl Into<String>,
    ) -> StateVerificationReport {
        self.verify(&VerificationRequest::new(database_path, repository_id))
    }

    /// Compatibility alias for [`Self::verify_path`].
    #[must_use]
    pub fn check_path(
        &self,
        database_path: impl AsRef<Path>,
        repository_id: impl Into<String>,
    ) -> StateVerificationReport {
        self.verify_path(database_path, repository_id)
    }
}

/// Verifies an existing database path without creating a verifier value.
#[must_use]
pub fn verify_state(
    database_path: impl AsRef<Path>,
    repository_id: impl Into<String>,
) -> StateVerificationReport {
    StateVerifier::new().verify_path(database_path, repository_id)
}

/// Compatibility alias for [`verify_state`].
#[must_use]
pub fn verify_database(
    database_path: impl AsRef<Path>,
    repository_id: impl Into<String>,
) -> StateVerificationReport {
    verify_state(database_path, repository_id)
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    repository_id: String,
    quick_check: QuickCheckVerification,
    foreign_keys: ForeignKeyVerification,
    migration: MigrationVerification,
    repository_scope: RepositoryScopeVerification,
    filesystem_permissions: PermissionVerification,
    connection_read_only: bool,
    issues: Vec<VerificationIssue>,
    remediation: Vec<String>,
) -> StateVerificationReport {
    let healthy = connection_read_only
        && quick_check.passed
        && foreign_keys.passed
        && migration.passed
        && repository_scope.passed
        && filesystem_permissions.passed;
    StateVerificationReport {
        repository_id,
        healthy,
        read_only: true,
        privacy_disclosure: V1_STORAGE_RESIDUAL_RISK.to_owned(),
        connection_read_only,
        quick_check_ok: quick_check.passed,
        foreign_keys_ok: foreign_keys.passed,
        migration_ok: migration.passed,
        repository_scope_ok: repository_scope.passed,
        filesystem_permissions_ok: filesystem_permissions.passed,
        quick_check,
        foreign_keys,
        migration,
        repository_scope,
        filesystem_permissions,
        issues,
        remediation,
    }
}

fn open_read_only(path: &Path) -> Result<Connection, ()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(());
    }
    let wal = sidecar_path(path, "-wal");
    let shm = sidecar_path(path, "-shm");
    let journal = sidecar_path(path, "-journal");
    if fs::symlink_metadata(&journal).is_ok_and(|metadata| metadata.len() > 0) {
        return Err(());
    }
    let wal_metadata = fs::symlink_metadata(&wal).ok();
    let shm_metadata = fs::symlink_metadata(&shm).ok();
    let wal_present = wal_metadata.as_ref().is_some_and(|metadata| {
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() > 0
    });
    let shm_present = shm_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());

    // A non-empty WAL needs its existing shared-memory index. Opening it
    // without that index would make SQLite create a new sidecar, which is not
    // permitted for this verifier. If both sidecars already exist, ordinary
    // read-only flags can read the committed WAL view without creating state.
    if wal_present {
        if !shm_present {
            return Err(());
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        return Connection::open_with_flags(path, flags).map_err(|_| ());
    }

    // With no pending WAL, immutable mode prevents SQLite from creating a
    // WAL/SHM pair merely to service a read. The verifier never creates a
    // missing target.
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_URI;
    Connection::open_with_flags(read_only_uri(path), flags).map_err(|_| ())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

fn read_only_uri(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let value = {
        let mut value = value;
        if value.as_bytes().get(1) == Some(&b':') && value.as_bytes().first() != Some(&b'/') {
            value.insert(0, '/');
        }
        value
    };
    let mut uri = String::from("file:");
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            uri.push(char::from(byte));
        } else {
            uri.push('%');
            uri.push_str(&format!("{byte:02X}"));
        }
    }
    uri.push_str("?immutable=1");
    uri
}

fn configure_read_only_connection(connection: &Connection) -> Result<(), ()> {
    connection
        .busy_timeout(Duration::from_millis(VERIFY_BUSY_TIMEOUT_MS))
        .map_err(|_| ())?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|_| ())?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|_| ())?;
    Ok(())
}

fn verify_quick_check(connection: &Connection) -> QuickCheckVerification {
    let result = (|| -> Result<bool, rusqlite::Error> {
        let mut statement = connection.prepare("PRAGMA quick_check(1)")?;
        let mut rows = statement.query([])?;
        let mut observed = false;
        while let Some(row) = rows.next()? {
            observed = true;
            if row.get::<_, String>(0)? != "ok" {
                return Ok(false);
            }
        }
        Ok(observed)
    })();
    match result {
        Ok(true) => QuickCheckVerification {
            status: CheckStatus::Passed,
            passed: true,
            code: "quick-check-passed",
        },
        Ok(false) => QuickCheckVerification {
            status: CheckStatus::Failed,
            passed: false,
            code: "quick-check-failed",
        },
        Err(_) => QuickCheckVerification {
            status: CheckStatus::Unavailable,
            passed: false,
            code: "quick-check-unavailable",
        },
    }
}

fn verify_foreign_keys(connection: &Connection) -> ForeignKeyVerification {
    let enabled = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map(|value| value == 1)
        .unwrap_or(false);
    let result = (|| -> Result<(usize, bool), rusqlite::Error> {
        let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
        let mut rows = statement.query([])?;
        let mut count = 0;
        while let Some(_row) = rows.next()? {
            count += 1;
            if count >= MAX_FOREIGN_KEY_VIOLATIONS {
                return Ok((count, false));
            }
        }
        Ok((count, true))
    })();
    match result {
        Ok((violation_count, bounded)) if enabled && violation_count == 0 => {
            ForeignKeyVerification {
                status: CheckStatus::Passed,
                passed: true,
                enabled,
                violation_count,
                bounded,
                code: "foreign-keys-passed",
            }
        }
        Ok((violation_count, bounded)) => ForeignKeyVerification {
            status: CheckStatus::Failed,
            passed: false,
            enabled,
            violation_count,
            bounded,
            code: if enabled {
                "foreign-key-violations"
            } else {
                "foreign-keys-disabled"
            },
        },
        Err(_) => ForeignKeyVerification {
            status: CheckStatus::Unavailable,
            passed: false,
            enabled,
            violation_count: 0,
            bounded: true,
            code: "foreign-key-check-unavailable",
        },
    }
}

fn verify_migration(connection: &Connection, expected: i64) -> MigrationVerification {
    match connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)) {
        Ok(found) if found == expected => MigrationVerification {
            status: CheckStatus::Passed,
            passed: true,
            expected,
            found: Some(found),
            code: "migration-passed",
        },
        Ok(found) => MigrationVerification {
            status: CheckStatus::Failed,
            passed: false,
            expected,
            found: Some(found),
            code: "migration-mismatch",
        },
        Err(_) => MigrationVerification {
            status: CheckStatus::Unavailable,
            passed: false,
            expected,
            found: None,
            code: "migration-unavailable",
        },
    }
}

fn verify_repository_scope(
    connection: &Connection,
    repository_id: &str,
) -> RepositoryScopeVerification {
    if !valid_repository_id(repository_id) {
        return RepositoryScopeVerification {
            status: CheckStatus::Failed,
            passed: false,
            repository_id: REDACTED_REPOSITORY.to_owned(),
            repository_found: false,
            code: "repository-scope-invalid",
        };
    }
    let found = connection
        .query_row(
            "SELECT 1 FROM repositories WHERE repository_id = ?1 LIMIT 1",
            [repository_id],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    RepositoryScopeVerification {
        status: if found {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        passed: found,
        repository_id: repository_id.to_owned(),
        repository_found: found,
        code: if found {
            "repository-scope-passed"
        } else {
            "repository-scope-mismatch"
        },
    }
}

const REDACTED_REPOSITORY: &str = "[REDACTED]";

fn safe_repository_id(value: &str) -> String {
    if valid_repository_id(value) {
        value.to_owned()
    } else {
        REDACTED_REPOSITORY.to_owned()
    }
}

fn valid_repository_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '.' | '/' | ':' | '@' | '+' | '#' | '='
                )
        })
        && repo_com_audit::redact_text(value) == value
}

fn verify_permissions(path: &Path) -> PermissionVerification {
    let model = match PermissionModel::current() {
        PermissionModel::PosixModes => "posix-owner-modes",
        PermissionModel::WindowsInheritedAcl => "windows-inherited-acl",
    };
    let metadata = fs::symlink_metadata(path).ok();
    let file_present = metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
    let file_user_only = file_present && is_user_only(path);
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_user_only = parent_user_only(parent);
    let passed = file_present && file_user_only && parent_user_only;
    PermissionVerification {
        status: if passed {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        passed,
        model: model.to_owned(),
        file_present,
        file_user_only,
        parent_user_only,
        code: if passed {
            "permissions-passed"
        } else {
            "permissions-unsafe"
        },
    }
}

fn parent_user_only(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

fn push_issue(
    issues: &mut Vec<VerificationIssue>,
    remediation: &mut Vec<String>,
    code: &'static str,
    summary: &'static str,
    fix: &'static str,
) {
    issues.push(VerificationIssue {
        code,
        summary,
        remediation: fix,
    });
    if !remediation.iter().any(|value| value == fix) {
        remediation.push(fix.to_owned());
    }
}

impl fmt::Display for StateVerificationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "state verification {} for repository {}",
            if self.healthy { "healthy" } else { "failed" },
            self.repository_id
        )
    }
}
