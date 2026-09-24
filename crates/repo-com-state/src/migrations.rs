use std::fmt;
use std::sync::{Mutex, OnceLock};

use rusqlite::{Connection, OptionalExtension};
use rusqlite_migration::{M, Migrations};

/// The only schema version understood by this crate.
pub const SCHEMA_VERSION: i64 = 1;

/// The complete forward-only version-1 migration.
pub const INITIAL_MIGRATION_SQL: &str = include_str!("../migrations/0001_initial.sql");

/// The migration set used by the state store.
///
/// The set is intentionally a single upward migration.  Down migrations are
/// not defined, which prevents an older binary from destructively rolling back
/// a newer local database.
pub const MIGRATION_ARRAY: &[M<'static>] = &[M::up(INITIAL_MIGRATION_SQL)];
pub const MIGRATIONS: Migrations<'static> = Migrations::from_slice(MIGRATION_ARRAY);

/// A small, typed report of a migration attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationReport {
    /// Schema version observed before the attempt.
    pub from_version: i64,
    /// Schema version observed after the attempt.
    pub to_version: i64,
    /// Number of migrations applied by this attempt.
    pub applied: usize,
}

/// A migration-layer failure that can be classified by the store boundary.
#[derive(Debug)]
pub enum MigrationError {
    /// SQLite rejected the schema version query.
    VersionRead { message: String },
    /// SQLite reported a negative or otherwise invalid `user_version`.
    InvalidVersion(i64),
    /// The database was written by a newer schema.
    UnsupportedSchema { found: i64, supported: i64 },
    /// The migration transaction failed and was rolled back.
    ApplyFailed { from_version: i64, message: String },
    /// The database version claims v1 but required v1 objects are absent.
    VerificationFailed { message: String },
    /// A SQLite operation other than migration failed.
    Sqlite {
        operation: &'static str,
        message: String,
    },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionRead { message } => {
                write!(formatter, "could not read schema version: {message}")
            }
            Self::InvalidVersion(version) => {
                write!(formatter, "invalid SQLite schema version: {version}")
            }
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "unsupported SQLite schema version {found}; this build supports {supported}"
            ),
            Self::ApplyFailed {
                from_version,
                message,
            } => write!(
                formatter,
                "schema migration from version {from_version} failed and was rolled back: {message}"
            ),
            Self::VerificationFailed { message } => {
                write!(formatter, "schema version verification failed: {message}")
            }
            Self::Sqlite { operation, message } => {
                write!(formatter, "SQLite {operation} failed: {message}")
            }
        }
    }
}

impl std::error::Error for MigrationError {}

/// Reads the SQLite `user_version` without changing the database.
pub fn current_schema_version(conn: &Connection) -> Result<i64, MigrationError> {
    let version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| MigrationError::VersionRead {
            message: error.to_string(),
        })?;
    if version < 0 {
        return Err(MigrationError::InvalidVersion(version));
    }
    Ok(version)
}

fn migration_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Applies the version-1 migration exactly once and verifies the resulting
/// schema.  A newer schema is rejected before any migration statement runs.
pub fn apply_initial_migration(conn: &mut Connection) -> Result<MigrationReport, MigrationError> {
    let _migration_guard = migration_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let from_version = current_schema_version(conn)?;
    if from_version > SCHEMA_VERSION {
        return Err(MigrationError::UnsupportedSchema {
            found: from_version,
            supported: SCHEMA_VERSION,
        });
    }

    if from_version == SCHEMA_VERSION {
        verify_schema(conn)?;
        return Ok(MigrationReport {
            from_version,
            to_version: SCHEMA_VERSION,
            applied: 0,
        });
    }

    if let Err(error) = MIGRATIONS.to_latest(conn) {
        // Another process may have completed the same forward migration while
        // this connection was waiting for SQLite's write lock.  Re-read and
        // verify before reporting failure; never restore a successfully
        // committed schema in that race.
        if current_schema_version(conn).ok() == Some(SCHEMA_VERSION) && verify_schema(conn).is_ok()
        {
            return Ok(MigrationReport {
                from_version,
                to_version: SCHEMA_VERSION,
                applied: 0,
            });
        }
        return Err(MigrationError::ApplyFailed {
            from_version,
            message: error.to_string(),
        });
    }

    let to_version = current_schema_version(conn)?;
    if to_version != SCHEMA_VERSION {
        return Err(MigrationError::VerificationFailed {
            message: format!("expected schema version {SCHEMA_VERSION}, found {to_version}"),
        });
    }
    verify_schema(conn)?;

    Ok(MigrationReport {
        from_version,
        to_version,
        applied: (SCHEMA_VERSION - from_version) as usize,
    })
}

/// Verifies that the version marker and all required version-1 tables exist.
pub fn verify_schema(conn: &Connection) -> Result<(), MigrationError> {
    let version = current_schema_version(conn)?;
    if version != SCHEMA_VERSION {
        return Err(MigrationError::VerificationFailed {
            message: format!("expected schema version {SCHEMA_VERSION}, found {version}"),
        });
    }

    let metadata_version = conn
        .query_row(
            "SELECT schema_version FROM schema_metadata WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| MigrationError::VerificationFailed {
            message: format!("schema metadata is unavailable: {error}"),
        })?;
    if metadata_version != SCHEMA_VERSION {
        return Err(MigrationError::VerificationFailed {
            message: format!(
                "schema metadata version {metadata_version} does not match {SCHEMA_VERSION}"
            ),
        });
    }

    const REQUIRED_TABLES: &[&str] = &[
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
    ];
    for table in REQUIRED_TABLES {
        let exists = conn
            .table_exists(None, *table)
            .map_err(|error| MigrationError::Sqlite {
                operation: "schema inspection",
                message: error.to_string(),
            })?;
        if !exists {
            return Err(MigrationError::VerificationFailed {
                message: format!("required table {table} is missing"),
            });
        }
    }

    const REQUIRED_TRIGGERS: &[&str] = &[
        "draft_revisions_immutable_update",
        "draft_revisions_immutable_delete",
        "inbound_items_first_snapshot_immutable_update",
        "inbound_items_first_snapshot_immutable_delete",
        "audit_events_append_only_update",
        "audit_events_append_only_delete",
    ];
    for trigger in REQUIRED_TRIGGERS {
        let exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| MigrationError::Sqlite {
                operation: "schema inspection",
                message: error.to_string(),
            })?
            .is_some();
        if !exists {
            return Err(MigrationError::VerificationFailed {
                message: format!("required trigger {trigger} is missing"),
            });
        }
    }

    let mut foreign_key_check =
        conn.prepare("PRAGMA foreign_key_check")
            .map_err(|error| MigrationError::Sqlite {
                operation: "foreign-key verification",
                message: error.to_string(),
            })?;
    let mut rows = foreign_key_check
        .query([])
        .map_err(|error| MigrationError::Sqlite {
            operation: "foreign-key verification",
            message: error.to_string(),
        })?;
    if rows
        .next()
        .map_err(|error| MigrationError::Sqlite {
            operation: "foreign-key verification",
            message: error.to_string(),
        })?
        .is_some()
    {
        return Err(MigrationError::VerificationFailed {
            message: "foreign-key check reported a violation".to_owned(),
        });
    }
    Ok(())
}
