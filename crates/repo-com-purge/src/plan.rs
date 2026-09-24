//! Non-mutating, deterministic local purge planning.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use repo_com_retention::{format_rfc3339_utc, parse_rfc3339_utc};
use repo_com_state::StateStore;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The stable hash algorithm used for a purge plan.
pub const HASH_ALGORITHM: &str = "sha256";
/// Number of hexadecimal characters in a SHA-256 digest.
pub const HASH_HEX_LENGTH: usize = 64;
/// The current purge-plan schema version.
pub const PURGE_PLAN_SCHEMA_VERSION: u8 = 1;
/// The irreversible marker used when a content-only purge removes text while
/// preserving its local identity and non-content evidence.
pub const PURGED_CONTENT_MARKER: &str = "[content-purged]";

/// Result type used by the purge boundary.
pub type PurgeResult<T> = Result<T, PurgeError>;

/// A safe, typed purge failure.
///
/// The variants deliberately contain only stable categories and non-secret
/// identifiers. SQLite diagnostics and row values are not copied into this
/// error type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PurgeError {
    /// The repository identifier is empty, too long, or contains NUL.
    InvalidRepositoryId,
    /// The cutoff is not a representable canonical UTC instant.
    InvalidCutoff,
    /// A stored timestamp could not be interpreted safely.
    InvalidStoredTimestamp,
    /// The requested repository is not present in the local state store.
    RepositoryNotFound {
        /// The requested repository identifier.
        repository_id: String,
    },
    /// The current repository configuration hash is missing or malformed.
    InvalidConfigurationHash,
    /// A caller-supplied configuration hash is stale.
    ConfigurationHashMismatch,
    /// The confirmation was not collected in an interactive TTY.
    TtyRequired,
    /// The confirmation names a different repository.
    RepositoryMismatch,
    /// The confirmation names a different scope.
    ScopeMismatch,
    /// The confirmation names a different cutoff.
    CutoffMismatch,
    /// The plan hash does not describe the supplied plan fields.
    PlanHashMismatch,
    /// The current state no longer matches the confirmed plan.
    ReplanRequired,
    /// The exact plan was already executed by this executor.
    PlanAlreadyExecuted,
    /// The plan structure is invalid.
    InvalidPlan,
    /// A local state operation failed without exposing its database value.
    Storage {
        /// Stable operation category.
        operation: &'static str,
    },
    /// Canonical plan serialization failed.
    Serialization,
    /// A deterministic test failure was injected.
    InjectedFailure {
        /// Safe failure-point label.
        point: String,
    },
}

impl PurgeError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRepositoryId => "purge-invalid-repository",
            Self::InvalidCutoff => "purge-invalid-cutoff",
            Self::InvalidStoredTimestamp => "purge-invalid-stored-timestamp",
            Self::RepositoryNotFound { .. } => "purge-repository-not-found",
            Self::InvalidConfigurationHash => "purge-invalid-configuration-hash",
            Self::ConfigurationHashMismatch => "purge-configuration-hash-mismatch",
            Self::TtyRequired => "purge-tty-required",
            Self::RepositoryMismatch => "purge-repository-mismatch",
            Self::ScopeMismatch => "purge-scope-mismatch",
            Self::CutoffMismatch => "purge-cutoff-mismatch",
            Self::PlanHashMismatch => "purge-plan-hash-mismatch",
            Self::ReplanRequired => "purge-replan-required",
            Self::PlanAlreadyExecuted => "purge-plan-already-executed",
            Self::InvalidPlan => "purge-invalid-plan",
            Self::Storage { .. } => "purge-storage",
            Self::Serialization => "purge-serialization",
            Self::InjectedFailure { .. } => "purge-injected-failure",
        }
    }
}

impl fmt::Display for PurgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepositoryId => {
                formatter.write_str("purge repository identifier is invalid")
            }
            Self::InvalidCutoff => formatter.write_str("purge cutoff is not canonical UTC"),
            Self::InvalidStoredTimestamp => {
                formatter.write_str("local state contains an invalid timestamp")
            }
            Self::RepositoryNotFound { .. } => {
                formatter.write_str("the requested repository is not present in local state")
            }
            Self::InvalidConfigurationHash => {
                formatter.write_str("the local repository configuration hash is invalid")
            }
            Self::ConfigurationHashMismatch => {
                formatter.write_str("the local configuration hash no longer matches the plan")
            }
            Self::TtyRequired => formatter.write_str("confirmed purge requires an interactive TTY"),
            Self::RepositoryMismatch => {
                formatter.write_str("purge confirmation repository mismatch")
            }
            Self::ScopeMismatch => formatter.write_str("purge confirmation scope mismatch"),
            Self::CutoffMismatch => formatter.write_str("purge confirmation cutoff mismatch"),
            Self::PlanHashMismatch => formatter.write_str("purge plan hash mismatch"),
            Self::ReplanRequired => formatter
                .write_str("purge state changed; a fresh plan and confirmation are required"),
            Self::PlanAlreadyExecuted => {
                formatter.write_str("the confirmed purge plan was already executed")
            }
            Self::InvalidPlan => formatter.write_str("purge plan is invalid"),
            Self::Storage { operation } => write!(formatter, "local purge {operation} failed"),
            Self::Serialization => formatter.write_str("purge plan serialization failed"),
            Self::InjectedFailure { point } => {
                write!(formatter, "injected purge failure at {point}")
            }
        }
    }
}

impl Error for PurgeError {}

/// The local categories that an explicit purge may target.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PurgeScope {
    /// Remove or irreversibly replace content while retaining local evidence.
    Content,
    /// Remove non-content metadata and lifecycle rows.
    Metadata,
    /// Remove all operational local state for the repository. The cutoff is
    /// still bound to confirmation and the plan hash, but full-state scope
    /// intentionally does not narrow the local deletion set. The repository
    /// identity and the post-commit count-only audit record remain as the
    /// minimum local evidence anchor.
    All,
}

impl PurgeScope {
    /// Compatibility spelling for callers that use the feature-document name.
    #[allow(non_upper_case_globals)]
    pub const AllState: Self = Self::All;

    /// Returns the stable serialized spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Metadata => "metadata",
            Self::All => "all",
        }
    }

    /// Returns whether this scope includes the complete operational state.
    #[must_use]
    pub const fn is_all(self) -> bool {
        matches!(self, Self::All)
    }
}

impl fmt::Display for PurgeScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One canonical UTC cutoff used to select purge candidates.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeCutoff {
    /// Unix seconds represented by the cutoff.
    pub unix_seconds: u64,
    /// Canonical UTC RFC 3339 text.
    pub utc: String,
}

impl PurgeCutoff {
    /// Constructs a cutoff and verifies that both representations agree.
    pub fn new(unix_seconds: u64, utc: impl Into<String>) -> PurgeResult<Self> {
        let utc = utc.into();
        let expected = format_rfc3339_utc(unix_seconds).ok_or(PurgeError::InvalidCutoff)?;
        if utc != expected {
            return Err(PurgeError::InvalidCutoff);
        }
        Ok(Self { unix_seconds, utc })
    }

    /// Constructs a canonical cutoff from Unix seconds.
    pub fn from_unix_seconds(unix_seconds: u64) -> PurgeResult<Self> {
        Self::new(
            unix_seconds,
            format_rfc3339_utc(unix_seconds).ok_or(PurgeError::InvalidCutoff)?,
        )
    }

    /// Parses a UTC RFC 3339 cutoff and canonicalizes it to whole seconds.
    pub fn from_rfc3339(value: &str) -> PurgeResult<Self> {
        let unix_seconds = parse_rfc3339_utc(value).ok_or(PurgeError::InvalidCutoff)?;
        Self::from_unix_seconds(unix_seconds)
    }

    /// Returns the Unix-second representation.
    #[must_use]
    pub const fn unix_seconds(&self) -> u64 {
        self.unix_seconds
    }

    /// Returns the canonical UTC representation.
    #[must_use]
    pub fn utc(&self) -> &str {
        &self.utc
    }
}

/// The non-mutating planner input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeRequest {
    /// Exact repository scope.
    pub repository_id: String,
    /// Requested local category.
    pub scope: PurgeScope,
    /// Candidate cutoff.
    pub cutoff: PurgeCutoff,
    /// Optional caller expectation for the current configuration hash.
    pub expected_config_hash: Option<String>,
}

impl PurgeRequest {
    /// Creates a request. The current configuration hash is read from local
    /// state and recorded in the resulting plan.
    #[must_use]
    pub fn new(repository_id: impl Into<String>, scope: PurgeScope, cutoff: PurgeCutoff) -> Self {
        Self {
            repository_id: repository_id.into(),
            scope,
            cutoff,
            expected_config_hash: None,
        }
    }

    /// Adds a stale-configuration check to the request.
    #[must_use]
    pub fn with_expected_config_hash(mut self, config_hash: impl Into<String>) -> Self {
        self.expected_config_hash = Some(config_hash.into());
        self
    }

    /// Compatibility alias for [`Self::with_expected_config_hash`].
    #[must_use]
    pub fn with_config_hash(self, config_hash: impl Into<String>) -> Self {
        self.with_expected_config_hash(config_hash)
    }

    fn validate(&self) -> PurgeResult<()> {
        validate_identifier(&self.repository_id).map_err(|_| PurgeError::InvalidRepositoryId)?;
        PurgeCutoff::new(self.cutoff.unix_seconds, &self.cutoff.utc)?;
        if let Some(config_hash) = &self.expected_config_hash {
            validate_configuration_hash(config_hash)?;
        }
        Ok(())
    }
}

/// Exact counts for a plan, broken down by stable local table/category name.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeCounts {
    /// Content-bearing rows selected for irreversible removal.
    pub content_rows: u64,
    /// Metadata/lifecycle rows selected for removal or irreversible clearing.
    pub metadata_rows: u64,
    /// Sum of the two category counts.
    pub total_rows: u64,
    /// Per-table counts, useful for a complete exact preview.
    pub table_rows: BTreeMap<String, u64>,
}

impl PurgeCounts {
    /// Returns the content count.
    #[must_use]
    pub const fn content_count(&self) -> u64 {
        self.content_rows
    }

    /// Returns the metadata count.
    #[must_use]
    pub const fn metadata_count(&self) -> u64 {
        self.metadata_rows
    }

    /// Returns the total count.
    #[must_use]
    pub const fn total_count(&self) -> u64 {
        self.total_rows
    }

    /// Returns one exact table count.
    #[must_use]
    pub fn table_count(&self, table: &str) -> u64 {
        self.table_rows.get(table).copied().unwrap_or(0)
    }

    fn from_work(work: &PurgeWork, scope: PurgeScope) -> Self {
        let mut counts = Self::default();
        match scope {
            PurgeScope::Content => {
                add_content_counts(&mut counts, work);
            }
            PurgeScope::Metadata => {
                add_metadata_counts(&mut counts, work);
            }
            PurgeScope::All => {
                add_content_counts(&mut counts, work);
                // Draft parents are operational state even when a draft has no
                // immutable revision. The all-state executor deletes those
                // parents as part of the content/operational branch.
                add_count(&mut counts, "drafts", work.metadata_drafts.len());
                counts.content_rows = counts
                    .content_rows
                    .saturating_add(work.metadata_drafts.len() as u64);
                add_standalone_metadata_counts(&mut counts, work);
            }
        }
        counts.total_rows = counts.content_rows.saturating_add(counts.metadata_rows);
        counts
    }
}

/// A complete, non-mutating local purge preview.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgePlan {
    /// Plan schema version.
    pub schema_version: u8,
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact requested category.
    pub scope: PurgeScope,
    /// Exact candidate cutoff.
    pub cutoff: PurgeCutoff,
    /// Configuration hash read from the current local repository row.
    pub config_hash: String,
    /// Exact category and table counts.
    pub counts: PurgeCounts,
    /// SHA-256 fingerprint of the current local rows; no row value is exposed.
    pub state_fingerprint: String,
    /// SHA-256 hash of all plan material, including the fingerprint and counts.
    pub plan_hash: String,
}

impl PurgePlan {
    fn from_material(material: &PlanMaterial) -> PurgeResult<Self> {
        let plan_hash = hash_bytes(&material.canonical_bytes()?);
        Ok(Self {
            schema_version: material.schema_version,
            repository_id: material.repository_id.clone(),
            scope: material.scope,
            cutoff: material.cutoff.clone(),
            config_hash: material.config_hash.clone(),
            counts: material.counts.clone(),
            state_fingerprint: material.state_fingerprint.clone(),
            plan_hash,
        })
    }

    /// Returns the stable plan hash.
    #[must_use]
    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }

    /// Returns the exact state fingerprint used to detect stale plans.
    #[must_use]
    pub fn state_fingerprint(&self) -> &str {
        &self.state_fingerprint
    }

    /// Returns the category counts.
    #[must_use]
    pub const fn counts(&self) -> &PurgeCounts {
        &self.counts
    }

    /// Verifies the internal hash and shape without consulting state.
    pub fn validate(&self) -> PurgeResult<()> {
        if self.schema_version != PURGE_PLAN_SCHEMA_VERSION
            || validate_identifier(&self.repository_id).is_err()
            || PurgeCutoff::new(self.cutoff.unix_seconds, &self.cutoff.utc).is_err()
            || validate_configuration_hash(&self.config_hash).is_err()
            || !is_sha256_hex(&self.state_fingerprint)
        {
            return Err(PurgeError::InvalidPlan);
        }
        let material = PlanMaterial::from_plan(self);
        let expected = hash_bytes(&material.canonical_bytes()?);
        if expected != self.plan_hash {
            return Err(PurgeError::PlanHashMismatch);
        }
        Ok(())
    }

    /// Returns whether the plan's stored hash still describes its fields.
    #[must_use]
    pub fn has_valid_hash(&self) -> bool {
        self.validate().is_ok()
    }
}

/// The exact serialized material covered by a plan hash.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PlanMaterial {
    schema_version: u8,
    repository_id: String,
    scope: PurgeScope,
    cutoff: PurgeCutoff,
    config_hash: String,
    counts: PurgeCounts,
    state_fingerprint: String,
}

impl PlanMaterial {
    fn from_plan(plan: &PurgePlan) -> Self {
        Self {
            schema_version: plan.schema_version,
            repository_id: plan.repository_id.clone(),
            scope: plan.scope,
            cutoff: plan.cutoff.clone(),
            config_hash: plan.config_hash.clone(),
            counts: plan.counts.clone(),
            state_fingerprint: plan.state_fingerprint.clone(),
        }
    }

    fn canonical_bytes(&self) -> PurgeResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| PurgeError::Serialization)
    }
}

/// A stateless, side-effect-free planner.
#[derive(Clone, Copy, Debug, Default)]
pub struct PurgePlanner;

impl PurgePlanner {
    /// Creates a planner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Builds a plan using a read-only state connection.
    pub fn plan(&self, state: &StateStore, request: &PurgeRequest) -> PurgeResult<PurgePlan> {
        plan_snapshot(state.connection(), request)
    }

    /// Compatibility alias for [`Self::plan`].
    pub fn preview(&self, state: &StateStore, request: &PurgeRequest) -> PurgeResult<PurgePlan> {
        self.plan(state, request)
    }

    /// Builds a plan without requiring a planner value.
    pub fn plan_state(state: &StateStore, request: &PurgeRequest) -> PurgeResult<PurgePlan> {
        plan_snapshot(state.connection(), request)
    }
}

/// Builds a non-mutating plan from a local SQLite connection.
///
/// A deferred read snapshot is used so the exact counts and fingerprint are
/// taken from one consistent local state view.
pub fn build_plan(connection: &Connection, request: &PurgeRequest) -> PurgeResult<PurgePlan> {
    plan_snapshot(connection, request)
}

/// Builds a plan inside a caller-owned transaction. The executor uses this
/// internal form after it has already begun its immediate write transaction.
pub(crate) fn build_plan_in_connection(
    connection: &Connection,
    request: &PurgeRequest,
) -> PurgeResult<PurgePlan> {
    request.validate()?;
    let config_hash: Option<String> = connection
        .query_row(
            "SELECT config_hash FROM repositories WHERE repository_id = ?1",
            [&request.repository_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| storage("read repository configuration"))?;
    let config_hash = config_hash.ok_or_else(|| PurgeError::RepositoryNotFound {
        repository_id: request.repository_id.clone(),
    })?;
    validate_configuration_hash(&config_hash)?;
    if let Some(expected) = &request.expected_config_hash
        && expected != &config_hash
    {
        return Err(PurgeError::ConfigurationHashMismatch);
    }

    let work = collect_work(connection, request)?;
    let counts = PurgeCounts::from_work(&work, request.scope);
    let state_fingerprint = fingerprint_state(connection, &request.repository_id)?;
    let material = PlanMaterial {
        schema_version: PURGE_PLAN_SCHEMA_VERSION,
        repository_id: request.repository_id.clone(),
        scope: request.scope,
        cutoff: request.cutoff.clone(),
        config_hash,
        counts,
        state_fingerprint,
    };
    PurgePlan::from_material(&material)
}

/// Convenience free function for callers that do not need a planner value.
pub fn plan_purge(state: &StateStore, request: &PurgeRequest) -> PurgeResult<PurgePlan> {
    plan_snapshot(state.connection(), request)
}

/// Takes one deferred read snapshot so counts and the state fingerprint cannot
/// come from different concurrent writer snapshots. The transaction is rolled
/// back rather than committed; planning never changes local state.
fn plan_snapshot(connection: &Connection, request: &PurgeRequest) -> PurgeResult<PurgePlan> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|_| storage("begin plan snapshot"))?;
    let result = build_plan_in_connection(&transaction, request);
    let rollback = transaction.rollback();
    match result {
        Ok(plan) => match rollback {
            Ok(()) => Ok(plan),
            Err(_) => Err(storage("close plan snapshot")),
        },
        Err(error) => {
            let _ = rollback;
            Err(error)
        }
    }
}

/// Candidate rows selected by one plan. IDs are used only for a local
/// transaction; they are never serialized into a diagnostic or audit value.
#[derive(Clone, Debug, Default)]
pub(crate) struct PurgeWork {
    pub(crate) repository_id: String,
    pub(crate) content_draft_revisions: Vec<(String, i64)>,
    pub(crate) content_inbound_items: Vec<String>,
    pub(crate) content_inbound_current: Vec<String>,
    pub(crate) content_inbound_transitions: Vec<String>,
    pub(crate) metadata_drafts: Vec<String>,
    pub(crate) metadata_draft_revisions: Vec<(String, i64)>,
    pub(crate) metadata_inbound_items: Vec<String>,
    pub(crate) metadata_inbound_current: Vec<String>,
    pub(crate) metadata_inbound_transitions: Vec<String>,
    pub(crate) metadata_approvals: Vec<String>,
    pub(crate) metadata_policy_activations: Vec<String>,
    pub(crate) metadata_delivery_attempts: Vec<String>,
    pub(crate) metadata_inbound_cursors: Vec<String>,
    pub(crate) metadata_acknowledgements: Vec<String>,
    pub(crate) metadata_archives: Vec<String>,
    pub(crate) metadata_reply_links: Vec<String>,
    pub(crate) metadata_audit_events: Vec<String>,
}

pub(crate) fn collect_work(
    connection: &Connection,
    request: &PurgeRequest,
) -> PurgeResult<PurgeWork> {
    let mut work = PurgeWork {
        repository_id: request.repository_id.clone(),
        ..PurgeWork::default()
    };
    let all = request.scope.is_all();
    if matches!(request.scope, PurgeScope::Content | PurgeScope::All) {
        collect_content_work(
            connection,
            &request.repository_id,
            request.cutoff.unix_seconds,
            all,
            &mut work,
        )?;
    }
    if matches!(request.scope, PurgeScope::Metadata | PurgeScope::All) {
        collect_metadata_work(
            connection,
            &request.repository_id,
            request.cutoff.unix_seconds,
            all,
            &mut work,
        )?;
    }
    Ok(work)
}

fn collect_content_work(
    connection: &Connection,
    repository_id: &str,
    cutoff: u64,
    all: bool,
    work: &mut PurgeWork,
) -> PurgeResult<()> {
    let mut statement = connection
        .prepare(
            "SELECT draft_id, revision, body, created_at
             FROM draft_revisions WHERE repository_id = ?1
             ORDER BY draft_id ASC, revision ASC",
        )
        .map_err(|_| storage("prepare draft content scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| storage("scan draft content rows"))?;
    for row in rows {
        let (draft_id, revision, body, timestamp) =
            row.map_err(|_| storage("read draft content row"))?;
        if (all || eligible(&timestamp, cutoff)?) && (all || body != PURGED_CONTENT_MARKER) {
            work.content_draft_revisions.push((draft_id, revision));
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT item_id, first_content, first_attachments_json, first_observed_at
             FROM inbound_items WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|_| storage("prepare inbound content scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| storage("scan inbound content rows"))?;
    for row in rows {
        let (item_id, content, attachments, timestamp) =
            row.map_err(|_| storage("read inbound content row"))?;
        if (all || eligible(&timestamp, cutoff)?)
            && (all || content != PURGED_CONTENT_MARKER || attachments != "[]")
        {
            work.content_inbound_items.push(item_id);
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT item_id, current_content, current_attachments_json, observed_at
             FROM inbound_current_snapshots WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|_| storage("prepare inbound current scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| storage("scan inbound current rows"))?;
    for row in rows {
        let (item_id, content, attachments, timestamp) =
            row.map_err(|_| storage("read inbound current row"))?;
        let has_content = content
            .as_deref()
            .is_some_and(|value| value != PURGED_CONTENT_MARKER);
        if (all || eligible(&timestamp, cutoff)?) && (all || has_content || attachments != "[]") {
            work.content_inbound_current.push(item_id);
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT transition_id, content, occurred_at
             FROM inbound_item_transitions WHERE repository_id = ?1
             ORDER BY transition_id ASC",
        )
        .map_err(|_| storage("prepare inbound transition scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| storage("scan inbound transition rows"))?;
    for row in rows {
        let (transition_id, content, timestamp) =
            row.map_err(|_| storage("read inbound transition row"))?;
        let has_content = content
            .as_deref()
            .is_some_and(|value| value != PURGED_CONTENT_MARKER);
        if (all || eligible(&timestamp, cutoff)?) && (all || has_content) {
            work.content_inbound_transitions.push(transition_id);
        }
    }
    Ok(())
}

fn collect_metadata_work(
    connection: &Connection,
    repository_id: &str,
    cutoff: u64,
    all: bool,
    work: &mut PurgeWork,
) -> PurgeResult<()> {
    collect_metadata_field_rows(connection, repository_id, cutoff, all, work)?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT approval_id, approved_at FROM approvals WHERE repository_id = ?1 ORDER BY approval_id ASC",
        &mut work.metadata_approvals,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT activation_id, activated_at FROM policy_activations WHERE repository_id = ?1 ORDER BY activation_id ASC",
        &mut work.metadata_policy_activations,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT attempt_id, claimed_at FROM delivery_attempts WHERE repository_id = ?1 ORDER BY attempt_id ASC",
        &mut work.metadata_delivery_attempts,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT alias, updated_at FROM inbound_cursors WHERE repository_id = ?1 ORDER BY alias ASC",
        &mut work.metadata_inbound_cursors,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT item_id, acknowledged_at FROM inbound_acknowledgements WHERE repository_id = ?1 ORDER BY item_id ASC",
        &mut work.metadata_acknowledgements,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT item_id, archived_at FROM inbound_archives WHERE repository_id = ?1 ORDER BY item_id ASC",
        &mut work.metadata_archives,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT item_id, linked_at FROM inbound_reply_links WHERE repository_id = ?1 ORDER BY item_id ASC",
        &mut work.metadata_reply_links,
    )?;
    collect_id_rows(
        connection,
        repository_id,
        cutoff,
        all,
        "SELECT event_id, occurred_at FROM audit_events WHERE repository_id = ?1 ORDER BY event_id ASC",
        &mut work.metadata_audit_events,
    )?;
    Ok(())
}

fn collect_metadata_field_rows(
    connection: &Connection,
    repository_id: &str,
    cutoff: u64,
    all: bool,
    work: &mut PurgeWork,
) -> PurgeResult<()> {
    let mut statement = connection
        .prepare(
            "SELECT draft_id, metadata_json, updated_at
             FROM drafts WHERE repository_id = ?1 ORDER BY draft_id ASC",
        )
        .map_err(|_| storage("prepare draft metadata scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| storage("scan draft metadata rows"))?;
    for row in rows {
        let (draft_id, metadata, timestamp) =
            row.map_err(|_| storage("read draft metadata row"))?;
        if (all || eligible(&timestamp, cutoff)?) && (all || !empty_object(&metadata)) {
            work.metadata_drafts.push(draft_id);
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT draft_id, revision, metadata_json, created_at
             FROM draft_revisions WHERE repository_id = ?1
             ORDER BY draft_id ASC, revision ASC",
        )
        .map_err(|_| storage("prepare revision metadata scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| storage("scan revision metadata rows"))?;
    for row in rows {
        let (draft_id, revision, metadata, timestamp) =
            row.map_err(|_| storage("read revision metadata row"))?;
        if (all || eligible(&timestamp, cutoff)?) && (all || !empty_object(&metadata)) {
            work.metadata_draft_revisions.push((draft_id, revision));
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT item_id, first_attachments_json, first_observed_at
             FROM inbound_items WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|_| storage("prepare inbound metadata scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| storage("scan inbound metadata rows"))?;
    for row in rows {
        let (item_id, attachments, timestamp) =
            row.map_err(|_| storage("read inbound metadata row"))?;
        if (all || eligible(&timestamp, cutoff)?) && (all || attachments != "[]") {
            work.metadata_inbound_items.push(item_id);
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT item_id, current_attachments_json, observed_at
             FROM inbound_current_snapshots WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|_| storage("prepare current metadata scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| storage("scan current metadata rows"))?;
    for row in rows {
        let (item_id, attachments, timestamp) =
            row.map_err(|_| storage("read current metadata row"))?;
        if (all || eligible(&timestamp, cutoff)?) && (all || attachments != "[]") {
            work.metadata_inbound_current.push(item_id);
        }
    }

    let mut statement = connection
        .prepare(
            "SELECT transition_id, metadata_json, occurred_at
             FROM inbound_item_transitions WHERE repository_id = ?1
             ORDER BY transition_id ASC",
        )
        .map_err(|_| storage("prepare transition metadata scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| storage("scan transition metadata rows"))?;
    for row in rows {
        let (transition_id, metadata, timestamp) =
            row.map_err(|_| storage("read transition metadata row"))?;
        if (all || eligible(&timestamp, cutoff)?) && (all || !empty_object(&metadata)) {
            work.metadata_inbound_transitions.push(transition_id);
        }
    }
    Ok(())
}

fn collect_id_rows(
    connection: &Connection,
    repository_id: &str,
    cutoff: u64,
    all: bool,
    sql: &str,
    output: &mut Vec<String>,
) -> PurgeResult<()> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|_| storage("prepare metadata row scan"))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| storage("scan metadata rows"))?;
    for row in rows {
        let (id, timestamp) = row.map_err(|_| storage("read metadata row"))?;
        if all || eligible(&timestamp, cutoff)? {
            output.push(id);
        }
    }
    Ok(())
}

fn add_content_counts(counts: &mut PurgeCounts, work: &PurgeWork) {
    add_count(
        counts,
        "draft_revisions",
        work.content_draft_revisions.len(),
    );
    add_count(counts, "inbound_items", work.content_inbound_items.len());
    add_count(
        counts,
        "inbound_current_snapshots",
        work.content_inbound_current.len(),
    );
    add_count(
        counts,
        "inbound_item_transitions",
        work.content_inbound_transitions.len(),
    );
    counts.content_rows = counts
        .content_rows
        .saturating_add(work.content_draft_revisions.len() as u64)
        .saturating_add(work.content_inbound_items.len() as u64)
        .saturating_add(work.content_inbound_current.len() as u64)
        .saturating_add(work.content_inbound_transitions.len() as u64);
}

fn add_metadata_counts(counts: &mut PurgeCounts, work: &PurgeWork) {
    add_count(counts, "drafts", work.metadata_drafts.len());
    add_count(
        counts,
        "draft_revisions",
        work.metadata_draft_revisions.len(),
    );
    add_count(counts, "inbound_items", work.metadata_inbound_items.len());
    add_count(
        counts,
        "inbound_current_snapshots",
        work.metadata_inbound_current.len(),
    );
    add_count(
        counts,
        "inbound_item_transitions",
        work.metadata_inbound_transitions.len(),
    );
    add_count(counts, "approvals", work.metadata_approvals.len());
    add_count(
        counts,
        "policy_activations",
        work.metadata_policy_activations.len(),
    );
    add_count(
        counts,
        "delivery_attempts",
        work.metadata_delivery_attempts.len(),
    );
    add_count(
        counts,
        "inbound_cursors",
        work.metadata_inbound_cursors.len(),
    );
    add_count(
        counts,
        "inbound_acknowledgements",
        work.metadata_acknowledgements.len(),
    );
    add_count(counts, "inbound_archives", work.metadata_archives.len());
    add_count(
        counts,
        "inbound_reply_links",
        work.metadata_reply_links.len(),
    );
    add_count(counts, "audit_events", work.metadata_audit_events.len());
    counts.metadata_rows = counts.metadata_rows.saturating_add(
        (work.metadata_drafts.len()
            + work.metadata_draft_revisions.len()
            + work.metadata_inbound_items.len()
            + work.metadata_inbound_current.len()
            + work.metadata_inbound_transitions.len()
            + work.metadata_approvals.len()
            + work.metadata_policy_activations.len()
            + work.metadata_delivery_attempts.len()
            + work.metadata_inbound_cursors.len()
            + work.metadata_acknowledgements.len()
            + work.metadata_archives.len()
            + work.metadata_reply_links.len()
            + work.metadata_audit_events.len()) as u64,
    );
}

fn add_standalone_metadata_counts(counts: &mut PurgeCounts, work: &PurgeWork) {
    add_count(counts, "approvals", work.metadata_approvals.len());
    add_count(
        counts,
        "policy_activations",
        work.metadata_policy_activations.len(),
    );
    add_count(
        counts,
        "delivery_attempts",
        work.metadata_delivery_attempts.len(),
    );
    add_count(
        counts,
        "inbound_cursors",
        work.metadata_inbound_cursors.len(),
    );
    add_count(
        counts,
        "inbound_acknowledgements",
        work.metadata_acknowledgements.len(),
    );
    add_count(counts, "inbound_archives", work.metadata_archives.len());
    add_count(
        counts,
        "inbound_reply_links",
        work.metadata_reply_links.len(),
    );
    add_count(counts, "audit_events", work.metadata_audit_events.len());
    counts.metadata_rows = counts.metadata_rows.saturating_add(
        (work.metadata_approvals.len()
            + work.metadata_policy_activations.len()
            + work.metadata_delivery_attempts.len()
            + work.metadata_inbound_cursors.len()
            + work.metadata_acknowledgements.len()
            + work.metadata_archives.len()
            + work.metadata_reply_links.len()
            + work.metadata_audit_events.len()) as u64,
    );
}

fn add_count(counts: &mut PurgeCounts, table: &str, count: usize) {
    let count = count as u64;
    counts.table_rows.insert(table.to_owned(), count);
}

fn eligible(timestamp: &str, cutoff: u64) -> PurgeResult<bool> {
    let seconds = parse_rfc3339_utc(timestamp).ok_or(PurgeError::InvalidStoredTimestamp)?;
    Ok(seconds <= cutoff)
}

fn empty_object(value: &str) -> bool {
    value.trim() == "{}"
}

fn validate_identifier(value: &str) -> Result<(), ()> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') {
        Err(())
    } else {
        Ok(())
    }
}

fn validate_configuration_hash(value: &str) -> PurgeResult<()> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') {
        Err(PurgeError::InvalidConfigurationHash)
    } else {
        Ok(())
    }
}

fn storage(operation: &'static str) -> PurgeError {
    PurgeError::Storage { operation }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == HASH_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Hashes every local operational row without retaining or exposing its value.
fn fingerprint_state(connection: &Connection, repository_id: &str) -> PurgeResult<String> {
    const TABLES: &[(&str, &str)] = &[
        ("drafts", "draft_id"),
        ("draft_revisions", "draft_id, revision"),
        ("approvals", "approval_id"),
        ("policy_activations", "activation_id"),
        ("delivery_attempts", "attempt_id"),
        ("inbound_cursors", "alias"),
        ("inbound_items", "item_id"),
        ("inbound_current_snapshots", "item_id"),
        ("inbound_item_transitions", "transition_id"),
        ("inbound_acknowledgements", "item_id"),
        ("inbound_archives", "item_id"),
        ("inbound_reply_links", "item_id"),
        ("audit_events", "event_id"),
    ];
    let mut digest = Sha256::new();
    for (table, order) in TABLES {
        digest.update(table.as_bytes());
        digest.update([0]);
        let sql = format!("SELECT * FROM {table} WHERE repository_id = ?1 ORDER BY {order} ASC");
        let mut statement = connection
            .prepare(&sql)
            .map_err(|_| storage("prepare state fingerprint"))?;
        let column_count = statement.column_count();
        let mut rows = statement
            .query([repository_id])
            .map_err(|_| storage("scan state fingerprint"))?;
        while let Some(row) = rows.next().map_err(|_| storage("read state fingerprint"))? {
            digest.update((column_count as u64).to_be_bytes());
            for index in 0..column_count {
                encode_value(
                    &mut digest,
                    row.get_ref(index)
                        .map_err(|_| storage("read state fingerprint value"))?,
                );
                digest.update([0xff]);
            }
            digest.update([0xfe]);
        }
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn encode_value(digest: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => digest.update([0]),
        ValueRef::Integer(number) => {
            digest.update([1]);
            digest.update(number.to_be_bytes());
        }
        ValueRef::Real(number) => {
            digest.update([2]);
            digest.update(number.to_be_bytes());
        }
        ValueRef::Text(bytes) => {
            digest.update([3]);
            digest.update((bytes.len() as u64).to_be_bytes());
            digest.update(bytes);
        }
        ValueRef::Blob(bytes) => {
            digest.update([4]);
            digest.update((bytes.len() as u64).to_be_bytes());
            digest.update(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PurgeCutoff, PurgeError};

    #[test]
    fn cutoff_requires_canonical_utc() {
        assert!(PurgeCutoff::from_rfc3339("2026-01-01T00:00:00Z").is_ok());
        assert!(PurgeCutoff::new(0, "1970-01-01T00:00:00Z").is_ok());
        assert_eq!(
            PurgeCutoff::new(0, "1970-01-01T00:00:00+00:00"),
            Err(PurgeError::InvalidCutoff)
        );
    }
}
