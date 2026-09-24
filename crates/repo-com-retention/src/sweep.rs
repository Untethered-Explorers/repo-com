//! Repository-scoped, transactional automatic retention sweeps.
//!
//! The sweeper only opens the local SQLite state supplied by the caller.  It
//! does not know about Discord, a scheduler, a remote audit service, or a
//! backup target.  Every invocation uses one immediate transaction: a failure
//! at any phase rolls back content redaction, metadata removal, and the audit
//! summary together.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use repo_com_state::{AuditEventInput, StateError, StateResult, StateStore, StateTransaction};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::policy::{
    CONTENT_EXPIRED_MARKER, RetentionClock, RetentionCutoffs, RetentionPolicy,
    RetentionPolicyError, SECONDS_PER_DAY, calculate_cutoffs, parse_rfc3339_nanos,
};

const DRAFT_UPDATE_TRIGGER: &str = "draft_revisions_immutable_update";
const DRAFT_DELETE_TRIGGER: &str = "draft_revisions_immutable_delete";
const INBOUND_UPDATE_TRIGGER: &str = "inbound_items_first_snapshot_immutable_update";
const INBOUND_DELETE_TRIGGER: &str = "inbound_items_first_snapshot_immutable_delete";
const AUDIT_DELETE_TRIGGER: &str = "audit_events_append_only_delete";

const DRAFT_UPDATE_TRIGGER_SQL: &str = "CREATE TRIGGER draft_revisions_immutable_update\nBEFORE UPDATE ON draft_revisions\nBEGIN\n    SELECT RAISE(ABORT, 'draft revisions are immutable');\nEND;";
const DRAFT_DELETE_TRIGGER_SQL: &str = "CREATE TRIGGER draft_revisions_immutable_delete\nBEFORE DELETE ON draft_revisions\nBEGIN\n    SELECT RAISE(ABORT, 'draft revisions are immutable');\nEND;";
const INBOUND_UPDATE_TRIGGER_SQL: &str = "CREATE TRIGGER inbound_items_first_snapshot_immutable_update\nBEFORE UPDATE ON inbound_items\nBEGIN\n    SELECT RAISE(ABORT, 'inbound first snapshots are immutable');\nEND;";
const INBOUND_DELETE_TRIGGER_SQL: &str = "CREATE TRIGGER inbound_items_first_snapshot_immutable_delete\nBEFORE DELETE ON inbound_items\nBEGIN\n    SELECT RAISE(ABORT, 'inbound first snapshots are immutable');\nEND;";
const AUDIT_DELETE_TRIGGER_SQL: &str = "CREATE TRIGGER audit_events_append_only_delete\nBEFORE DELETE ON audit_events\nBEGIN\n    SELECT RAISE(ABORT, 'audit events are append-only');\nEND;";

/// The phase in which a sweep failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SweepPhase {
    /// Repository/schema and timestamp preflight.
    Preflight,
    /// Draft or inbound text replacement.
    Content,
    /// Non-content metadata removal.
    Metadata,
    /// Append-only retention summary.
    Audit,
    /// Commit of the complete transaction.
    Commit,
    /// A caller-supplied mutation after a successful sweep.
    NewMutation,
}

impl fmt::Display for SweepPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Preflight => "preflight",
            Self::Content => "content",
            Self::Metadata => "metadata",
            Self::Audit => "audit",
            Self::Commit => "commit",
            Self::NewMutation => "new-mutation",
        })
    }
}

/// A deterministic test seam inside the retention transaction.
///
/// Production callers use the default options.  A selected point is reached
/// only after the named phase has performed the corresponding work, so an
/// `After*` point can prove rollback of a partial sweep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePoint {
    /// Fail before any content replacement.
    BeforeContent,
    /// Fail after this many content rows have been replaced.
    AfterContent(usize),
    /// Fail after content replacement and before metadata removal.
    BeforeMetadata,
    /// Fail after this many metadata rows have been removed.
    AfterMetadata(usize),
    /// Fail after metadata removal and before the summary audit append.
    BeforeAudit,
    /// Fail after the summary audit append and before commit.
    AfterAudit,
    /// Fail immediately before committing the transaction.
    BeforeCommit,
}

impl fmt::Display for FailurePoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeContent => formatter.write_str("before-content"),
            Self::AfterContent(count) => write!(formatter, "after-content-{count}"),
            Self::BeforeMetadata => formatter.write_str("before-metadata"),
            Self::AfterMetadata(count) => write!(formatter, "after-metadata-{count}"),
            Self::BeforeAudit => formatter.write_str("before-audit"),
            Self::AfterAudit => formatter.write_str("after-audit"),
            Self::BeforeCommit => formatter.write_str("before-commit"),
        }
    }
}

/// Options for one explicit sweep.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SweepOptions {
    /// Optional deterministic failure point used by contract tests.
    pub failure_point: Option<FailurePoint>,
}

impl SweepOptions {
    /// Creates ordinary options with no injected failure.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            failure_point: None,
        }
    }

    /// Selects a deterministic failure point.
    #[must_use]
    pub const fn with_failure_point(mut self, point: FailurePoint) -> Self {
        self.failure_point = Some(point);
        self
    }
}

/// A typed blocking storage-integrity result.
///
/// A caller must not continue with a new state mutation after receiving this
/// error.  The source is retained for local diagnostics, while the stable
/// `code` remains the integration boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockingStorageIntegrity {
    /// Exact repository scope.
    pub repository_id: String,
    /// Current clock value in canonical UTC form.
    pub as_of: String,
    /// Content cutoff used by the failed sweep.
    pub content_cutoff: String,
    /// Metadata cutoff used by the failed sweep.
    pub metadata_cutoff: String,
    /// Phase that failed.
    pub phase: SweepPhase,
    /// Optional local state error, never message content.
    pub source: Option<Box<StateError>>,
}

impl BlockingStorageIntegrity {
    /// Returns the stable protocol/storage category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "storage-integrity"
    }

    /// Returns whether this result blocks a new mutation.
    #[must_use]
    pub const fn blocks_mutation(&self) -> bool {
        true
    }

    /// Compatibility alias for [`Self::blocks_mutation`].
    #[must_use]
    pub const fn mutation_blocked(&self) -> bool {
        self.blocks_mutation()
    }
}

/// A safe typed error returned by policy validation or a blocked sweep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetentionError {
    /// The policy or cutoff inputs are invalid.
    Policy(RetentionPolicyError),
    /// A local state failure blocks the requested mutation.
    StorageIntegrity(BlockingStorageIntegrity),
}

impl RetentionError {
    /// Returns the stable integration category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Policy(_) => "retention-usage",
            Self::StorageIntegrity(_) => "storage-integrity",
        }
    }

    /// Returns the blocking integrity result, if this is a storage failure.
    #[must_use]
    pub const fn blocking_integrity(&self) -> Option<&BlockingStorageIntegrity> {
        match self {
            Self::Policy(_) => None,
            Self::StorageIntegrity(value) => Some(value),
        }
    }

    /// Compatibility alias for [`Self::blocking_integrity`].
    #[must_use]
    pub const fn integrity(&self) -> Option<&BlockingStorageIntegrity> {
        self.blocking_integrity()
    }
}

impl fmt::Display for RetentionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(error) => write!(formatter, "retention policy is invalid: {error}"),
            Self::StorageIntegrity(value) => write!(
                formatter,
                "retention sweep blocked repository {} during {}: {}",
                value.repository_id,
                value.phase,
                value.code()
            ),
        }
    }
}

impl Error for RetentionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Policy(error) => Some(error),
            Self::StorageIntegrity(value) => {
                value.source.as_deref().map(|error| error as &dyn Error)
            }
        }
    }
}

/// Counts of local rows changed by one sweep.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SweepCounts {
    /// Text rows replaced with [`CONTENT_EXPIRED_MARKER`].
    pub content_rows_redacted: usize,
    /// Non-content metadata rows removed.
    pub metadata_rows_removed: usize,
}

impl SweepCounts {
    /// Returns the content replacement count.
    #[must_use]
    pub const fn content_count(&self) -> usize {
        self.content_rows_redacted
    }

    /// Returns the metadata removal count.
    #[must_use]
    pub const fn metadata_count(&self) -> usize {
        self.metadata_rows_removed
    }
}

/// One stable, repository-scoped ID touched by a sweep.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovedId {
    /// A draft identity removed at metadata expiry.
    Draft {
        /// Repository scope.
        repository_id: String,
        /// Draft ID.
        draft_id: String,
    },
    /// An immutable draft revision removed at metadata expiry.
    DraftRevision {
        /// Repository scope.
        repository_id: String,
        /// Draft ID.
        draft_id: String,
        /// Revision number.
        revision: i64,
    },
    /// An inbound item identity removed at metadata expiry.
    InboundItem {
        /// Repository scope.
        repository_id: String,
        /// Inbound item ID.
        item_id: String,
    },
    /// An inbound transition removed at metadata expiry.
    InboundTransition {
        /// Repository scope.
        repository_id: String,
        /// Transition ID.
        transition_id: String,
    },
    /// A current inbound snapshot removed at metadata expiry.
    InboundCurrentSnapshot {
        /// Repository scope.
        repository_id: String,
        /// Inbound item ID.
        item_id: String,
    },
    /// An acknowledgement removed at metadata expiry.
    Acknowledgement {
        /// Repository scope.
        repository_id: String,
        /// Inbound item ID.
        item_id: String,
    },
    /// An archive marker removed at metadata expiry.
    Archive {
        /// Repository scope.
        repository_id: String,
        /// Inbound item ID.
        item_id: String,
    },
    /// A local reply link removed at metadata expiry.
    ReplyLink {
        /// Repository scope.
        repository_id: String,
        /// Inbound item ID.
        item_id: String,
    },
    /// A delivery attempt removed at metadata expiry.
    DeliveryAttempt {
        /// Repository scope.
        repository_id: String,
        /// Attempt ID.
        attempt_id: String,
    },
    /// An approval removed at metadata expiry.
    Approval {
        /// Repository scope.
        repository_id: String,
        /// Approval ID.
        approval_id: String,
    },
    /// A policy activation removed at metadata expiry.
    PolicyActivation {
        /// Repository scope.
        repository_id: String,
        /// Activation ID.
        activation_id: String,
    },
    /// An inbound cursor removed at metadata expiry.
    InboundCursor {
        /// Repository scope.
        repository_id: String,
        /// Alias.
        alias: String,
    },
    /// An audit event removed at metadata expiry.
    AuditEvent {
        /// Repository scope.
        repository_id: String,
        /// Stable audit event ID.
        event_id: String,
    },
}

/// IDs removed by the metadata phase.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemovedIds {
    /// Stable IDs in deterministic category/identity order.
    pub items: Vec<RemovedId>,
}

impl RemovedIds {
    /// Returns the number of removed IDs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether no metadata IDs were removed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Complete result of one committed sweep.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SweepResult {
    /// Exact clock, repository, and cutoff context.
    pub context: RetentionCutoffs,
    /// Content and metadata row counts.
    pub counts: SweepCounts,
    /// IDs whose text was replaced, retained for deterministic local evidence.
    pub redacted_ids: Vec<RemovedId>,
    /// IDs removed at the later metadata cutoff.
    pub removed_ids: RemovedIds,
    /// Stable count-only audit event ID.
    pub audit_event_id: String,
}

impl SweepResult {
    /// Returns the content replacement count.
    #[must_use]
    pub const fn content_count(&self) -> usize {
        self.counts.content_rows_redacted
    }

    /// Returns the metadata removal count.
    #[must_use]
    pub const fn metadata_count(&self) -> usize {
        self.counts.metadata_rows_removed
    }
}

/// An owned local state store plus one repository's validated retention policy.
pub struct RetentionSweeper<C> {
    state: StateStore,
    repository_id: String,
    policy: RetentionPolicy,
    clock: C,
}

impl<C> RetentionSweeper<C>
where
    C: RetentionClock,
{
    /// Creates a sweeper over an owned local state store.
    pub fn new(
        state: StateStore,
        repository_id: impl Into<String>,
        policy: RetentionPolicy,
        clock: C,
    ) -> Self {
        Self {
            state,
            repository_id: repository_id.into(),
            policy,
            clock,
        }
    }

    /// Compatibility alias for [`Self::new`].
    pub fn from_state(
        state: StateStore,
        repository_id: impl Into<String>,
        policy: RetentionPolicy,
        clock: C,
    ) -> Self {
        Self::new(state, repository_id, policy, clock)
    }

    /// Returns read-only access to the local state store.
    #[must_use]
    pub const fn state(&self) -> &StateStore {
        &self.state
    }

    /// Returns mutable access for explicit state composition.
    pub const fn state_mut(&mut self) -> &mut StateStore {
        &mut self.state
    }

    /// Consumes the sweeper and returns its local state store.
    #[must_use]
    pub fn into_state(self) -> StateStore {
        self.state
    }

    /// Returns the injected clock.
    #[must_use]
    pub const fn clock(&self) -> &C {
        &self.clock
    }

    /// Returns the validated policy.
    #[must_use]
    pub const fn policy(&self) -> RetentionPolicy {
        self.policy
    }

    /// Computes the current deterministic cutoffs without mutating state.
    pub fn cutoffs(&self) -> Result<RetentionCutoffs, RetentionPolicyError> {
        calculate_cutoffs(
            self.repository_id.clone(),
            self.clock.now_unix_seconds(),
            &self.policy,
        )
    }

    /// Runs one explicit retention sweep.
    pub fn sweep(&mut self) -> Result<SweepResult, RetentionError> {
        self.sweep_with_options(SweepOptions::default())
    }

    /// Runs one sweep with explicit deterministic test options.
    pub fn sweep_with_options(
        &mut self,
        options: SweepOptions,
    ) -> Result<SweepResult, RetentionError> {
        sweep_state_at(
            &mut self.state,
            &self.repository_id,
            self.policy,
            self.clock.now_unix_seconds(),
            options,
        )
    }

    /// Runs retention before a new mutation.  The mutation closure is not
    /// called when the sweep returns a blocking integrity result.
    pub fn run_before_mutation<T, F>(&mut self, mutation: F) -> Result<T, RetentionError>
    where
        F: FnOnce(&mut StateStore) -> StateResult<T>,
    {
        self.run_before_mutation_with_options(SweepOptions::default(), mutation)
    }

    /// Runs retention with explicit test options before a new mutation.
    pub fn run_before_mutation_with_options<T, F>(
        &mut self,
        options: SweepOptions,
        mutation: F,
    ) -> Result<T, RetentionError>
    where
        F: FnOnce(&mut StateStore) -> StateResult<T>,
    {
        let sweep = self.sweep_with_options(options)?;
        let context = sweep.context;
        mutation(&mut self.state)
            .map_err(|error| blocking_error(&context, SweepPhase::NewMutation, error))
    }

    /// Compatibility alias for [`Self::run_before_mutation`].
    pub fn sweep_before_mutation<T, F>(&mut self, mutation: F) -> Result<T, RetentionError>
    where
        F: FnOnce(&mut StateStore) -> StateResult<T>,
    {
        self.run_before_mutation(mutation)
    }
}

/// Runs a sweep directly over a borrowed local state store.
pub fn sweep_state<C>(
    state: &mut StateStore,
    repository_id: &str,
    policy: RetentionPolicy,
    clock: C,
) -> Result<SweepResult, RetentionError>
where
    C: RetentionClock,
{
    sweep_state_with_clock(state, repository_id, policy, &clock)
}

/// Runs a sweep with a caller-selected clock reference.
pub fn sweep_state_with_clock<C>(
    state: &mut StateStore,
    repository_id: &str,
    policy: RetentionPolicy,
    clock: &C,
) -> Result<SweepResult, RetentionError>
where
    C: RetentionClock + ?Sized,
{
    sweep_state_at(
        state,
        repository_id,
        policy,
        clock.now_unix_seconds(),
        SweepOptions::default(),
    )
}

/// Runs a sweep at an explicit instant with explicit options.
pub fn sweep_state_at(
    state: &mut StateStore,
    repository_id: &str,
    policy: RetentionPolicy,
    now_unix_seconds: u64,
    options: SweepOptions,
) -> Result<SweepResult, RetentionError> {
    let context = calculate_cutoffs(repository_id, now_unix_seconds, &policy)
        .map_err(RetentionError::Policy)?;
    let mut transaction = state
        .begin_transaction()
        .map_err(|error| blocking_error(&context, SweepPhase::Preflight, error))?;
    let work = sweep_transaction(&mut transaction, &context, options);
    match work {
        Ok((counts, redacted_ids, removed_ids, audit_event_id)) => {
            if options.failure_point == Some(FailurePoint::BeforeCommit) {
                let rollback = transaction.rollback();
                return match rollback {
                    Ok(()) => Err(blocking_error(
                        &context,
                        SweepPhase::Commit,
                        StateError::Transaction {
                            message: "injected retention failure before commit".to_owned(),
                        },
                    )),
                    Err(error) => Err(blocking_error(&context, SweepPhase::Commit, error)),
                };
            }
            transaction
                .commit()
                .map_err(|error| blocking_error(&context, SweepPhase::Commit, error))?;
            Ok(SweepResult {
                context,
                counts,
                redacted_ids,
                removed_ids,
                audit_event_id,
            })
        }
        Err(failure) => {
            let rollback = transaction.rollback();
            match rollback {
                Ok(()) => Err(blocking_error(&context, failure.phase, failure.source)),
                Err(error) => Err(blocking_error(&context, failure.phase, error)),
            }
        }
    }
}

#[derive(Debug)]
struct SweepTransactionFailure {
    phase: SweepPhase,
    source: StateError,
}

impl SweepTransactionFailure {
    fn new(phase: SweepPhase, source: StateError) -> Self {
        Self { phase, source }
    }
}

fn blocking_error(
    context: &RetentionCutoffs,
    phase: SweepPhase,
    source: StateError,
) -> RetentionError {
    RetentionError::StorageIntegrity(BlockingStorageIntegrity {
        repository_id: context.repository_id.clone(),
        as_of: context.as_of.clone(),
        content_cutoff: context.content_cutoff.clone(),
        metadata_cutoff: context.metadata_cutoff.clone(),
        phase,
        source: Some(Box::new(source)),
    })
}

fn injected(point: FailurePoint, phase: SweepPhase) -> SweepTransactionFailure {
    SweepTransactionFailure::new(
        phase,
        StateError::Transaction {
            message: format!("injected retention failure {point}"),
        },
    )
}

fn db_error(operation: &'static str, error: rusqlite::Error) -> StateError {
    StateError::Transaction {
        message: format!("retention {operation} failed: {error}"),
    }
}

fn parse_stored_timestamp(value: &str) -> Result<i128, StateError> {
    parse_rfc3339_nanos(value).ok_or_else(|| StateError::Transaction {
        message: "retention encountered an invalid stored UTC timestamp".to_owned(),
    })
}

fn timestamp_is_after_cutoff(timestamp: i128, cutoff: u64) -> bool {
    timestamp > i128::from(cutoff) * 1_000_000_000
}

fn timestamp_is_expired(timestamp: i128, cutoff: u64) -> bool {
    timestamp <= i128::from(cutoff) * 1_000_000_000
}

fn is_expired(timestamp: &str, cutoff: u64) -> Result<bool, StateError> {
    Ok(timestamp_is_expired(
        parse_stored_timestamp(timestamp)?,
        cutoff,
    ))
}

fn ensure_triggers(transaction: &StateTransaction<'_>) -> Result<(), StateError> {
    for name in [
        DRAFT_UPDATE_TRIGGER,
        DRAFT_DELETE_TRIGGER,
        INBOUND_UPDATE_TRIGGER,
        INBOUND_DELETE_TRIGGER,
        AUDIT_DELETE_TRIGGER,
    ] {
        let present: Option<String> = transaction
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| db_error("read retention schema", error))?;
        if present.is_none() {
            return Err(StateError::Transaction {
                message: format!("retention schema trigger {name} is missing"),
            });
        }
    }
    Ok(())
}

fn drop_trigger(transaction: &StateTransaction<'_>, name: &str) -> Result<(), StateError> {
    transaction
        .execute_batch(&format!("DROP TRIGGER {name};"))
        .map_err(|error| db_error("disable retention invariant", error))
}

fn recreate_trigger(transaction: &StateTransaction<'_>, name: &str) -> Result<(), StateError> {
    let sql = match name {
        DRAFT_UPDATE_TRIGGER => DRAFT_UPDATE_TRIGGER_SQL,
        DRAFT_DELETE_TRIGGER => DRAFT_DELETE_TRIGGER_SQL,
        INBOUND_UPDATE_TRIGGER => INBOUND_UPDATE_TRIGGER_SQL,
        INBOUND_DELETE_TRIGGER => INBOUND_DELETE_TRIGGER_SQL,
        AUDIT_DELETE_TRIGGER => AUDIT_DELETE_TRIGGER_SQL,
        _ => {
            return Err(StateError::Transaction {
                message: "retention attempted to recreate an unknown invariant".to_owned(),
            });
        }
    };
    transaction
        .execute_batch(sql)
        .map_err(|error| db_error("restore retention invariant", error))
}

fn maybe_fail(
    options: SweepOptions,
    point: FailurePoint,
    phase: SweepPhase,
) -> Result<(), SweepTransactionFailure> {
    if options.failure_point == Some(point) {
        return Err(injected(point, phase));
    }
    Ok(())
}

fn sweep_transaction(
    transaction: &mut StateTransaction<'_>,
    context: &RetentionCutoffs,
    options: SweepOptions,
) -> Result<(SweepCounts, Vec<RemovedId>, RemovedIds, String), SweepTransactionFailure> {
    ensure_triggers(transaction)
        .map_err(|source| SweepTransactionFailure::new(SweepPhase::Preflight, source))?;
    transaction
        .repositories()
        .repositories()
        .require(&context.repository_id)
        .map_err(|source| SweepTransactionFailure::new(SweepPhase::Preflight, source))?;

    let content = collect_content_work(transaction, context)
        .map_err(|source| SweepTransactionFailure::new(SweepPhase::Preflight, source))?;
    let metadata = collect_metadata_work(transaction, context, &content)
        .map_err(|source| SweepTransactionFailure::new(SweepPhase::Preflight, source))?;

    maybe_fail(options, FailurePoint::BeforeContent, SweepPhase::Content)?;
    let mut counts = SweepCounts::default();
    let mut redacted_ids = Vec::new();
    let disabled_update_triggers = !content.is_empty();
    if disabled_update_triggers {
        drop_trigger(transaction, DRAFT_UPDATE_TRIGGER)
            .and_then(|_| drop_trigger(transaction, INBOUND_UPDATE_TRIGGER))
            .map_err(|source| SweepTransactionFailure::new(SweepPhase::Content, source))?;
    }
    redact_content(
        transaction,
        context,
        &content,
        options,
        &mut counts,
        &mut redacted_ids,
    )?;
    if disabled_update_triggers {
        recreate_trigger(transaction, DRAFT_UPDATE_TRIGGER)
            .and_then(|_| recreate_trigger(transaction, INBOUND_UPDATE_TRIGGER))
            .map_err(|source| SweepTransactionFailure::new(SweepPhase::Content, source))?;
    }

    maybe_fail(options, FailurePoint::BeforeMetadata, SweepPhase::Metadata)?;
    let mut removed_ids = RemovedIds::default();
    if !metadata.is_empty() {
        drop_trigger(transaction, DRAFT_DELETE_TRIGGER)
            .and_then(|_| drop_trigger(transaction, INBOUND_DELETE_TRIGGER))
            .and_then(|_| drop_trigger(transaction, AUDIT_DELETE_TRIGGER))
            .map_err(|source| SweepTransactionFailure::new(SweepPhase::Metadata, source))?;
        remove_metadata(
            transaction,
            context,
            &metadata,
            options,
            &mut counts,
            &mut removed_ids,
        )?;
        recreate_trigger(transaction, DRAFT_DELETE_TRIGGER)
            .and_then(|_| recreate_trigger(transaction, INBOUND_DELETE_TRIGGER))
            .and_then(|_| recreate_trigger(transaction, AUDIT_DELETE_TRIGGER))
            .map_err(|source| SweepTransactionFailure::new(SweepPhase::Metadata, source))?;
    }

    maybe_fail(options, FailurePoint::BeforeAudit, SweepPhase::Audit)?;
    let audit_event_id = append_retention_audit(transaction, context, &counts)
        .map_err(|source| SweepTransactionFailure::new(SweepPhase::Audit, source))?;
    maybe_fail(options, FailurePoint::AfterAudit, SweepPhase::Audit)?;
    Ok((counts, redacted_ids, removed_ids, audit_event_id))
}

#[derive(Clone, Debug, Default)]
struct ContentWork {
    draft_revisions: Vec<(String, i64)>,
    inbound_first: Vec<String>,
    inbound_current: Vec<String>,
    inbound_transitions: Vec<String>,
    fresh_drafts: BTreeSet<String>,
    fresh_inbound_items: BTreeSet<String>,
    all_draft_revisions: BTreeSet<(String, i64)>,
    all_inbound_transitions: BTreeSet<String>,
    all_inbound_current: BTreeSet<String>,
}

impl ContentWork {
    fn is_empty(&self) -> bool {
        self.draft_revisions.is_empty()
            && self.inbound_first.is_empty()
            && self.inbound_current.is_empty()
            && self.inbound_transitions.is_empty()
    }
}

fn collect_content_work(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
) -> Result<ContentWork, StateError> {
    let mut work = ContentWork::default();
    let mut statement = transaction
        .prepare(
            "SELECT draft_id, updated_at FROM drafts WHERE repository_id = ?1 ORDER BY draft_id ASC",
        )
        .map_err(|error| db_error("prepare draft freshness scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan draft freshness rows", error))?;
    for row in rows {
        let (draft_id, updated_at) =
            row.map_err(|error| db_error("read draft freshness row", error))?;
        if timestamp_is_after_cutoff(
            parse_stored_timestamp(&updated_at)?,
            context.content_cutoff_unix_seconds,
        ) {
            work.fresh_drafts.insert(draft_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT draft_id, revision, body, created_at
             FROM draft_revisions WHERE repository_id = ?1
             ORDER BY draft_id ASC, revision ASC",
        )
        .map_err(|error| db_error("prepare draft retention scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| db_error("scan draft retention rows", error))?;
    for row in rows {
        let (draft_id, revision, body, created_at) =
            row.map_err(|error| db_error("read draft retention row", error))?;
        let timestamp = parse_stored_timestamp(&created_at)?;
        work.all_draft_revisions
            .insert((draft_id.clone(), revision));
        if timestamp_is_after_cutoff(timestamp, context.content_cutoff_unix_seconds) {
            work.fresh_drafts.insert(draft_id.clone());
        }
        if body != CONTENT_EXPIRED_MARKER
            && timestamp_is_expired(timestamp, context.content_cutoff_unix_seconds)
        {
            work.draft_revisions.push((draft_id, revision));
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT item_id, first_content, first_observed_at
             FROM inbound_items WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare inbound first retention scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| db_error("scan inbound first retention rows", error))?;
    for row in rows {
        let (item_id, content, observed_at) =
            row.map_err(|error| db_error("read inbound first retention row", error))?;
        let timestamp = parse_stored_timestamp(&observed_at)?;
        if timestamp_is_after_cutoff(timestamp, context.content_cutoff_unix_seconds) {
            work.fresh_inbound_items.insert(item_id.clone());
        }
        if content != CONTENT_EXPIRED_MARKER
            && timestamp_is_expired(timestamp, context.content_cutoff_unix_seconds)
        {
            work.inbound_first.push(item_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT item_id, current_content, observed_at
             FROM inbound_current_snapshots WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare inbound current retention scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| db_error("scan inbound current retention rows", error))?;
    for row in rows {
        let (item_id, content, observed_at) =
            row.map_err(|error| db_error("read inbound current retention row", error))?;
        let timestamp = parse_stored_timestamp(&observed_at)?;
        work.all_inbound_current.insert(item_id.clone());
        if timestamp_is_after_cutoff(timestamp, context.content_cutoff_unix_seconds) {
            work.fresh_inbound_items.insert(item_id.clone());
        }
        if let Some(content) = content
            && content != CONTENT_EXPIRED_MARKER
            && timestamp_is_expired(timestamp, context.content_cutoff_unix_seconds)
        {
            work.inbound_current.push(item_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT transition_id, item_id, content, occurred_at
             FROM inbound_item_transitions WHERE repository_id = ?1
             ORDER BY transition_id ASC",
        )
        .map_err(|error| db_error("prepare inbound transition retention scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| db_error("scan inbound transition retention rows", error))?;
    for row in rows {
        let (transition_id, item_id, content, occurred_at) =
            row.map_err(|error| db_error("read inbound transition retention row", error))?;
        let timestamp = parse_stored_timestamp(&occurred_at)?;
        work.all_inbound_transitions.insert(transition_id.clone());
        if timestamp_is_after_cutoff(timestamp, context.content_cutoff_unix_seconds) {
            work.fresh_inbound_items.insert(item_id);
        }
        if let Some(content) = content
            && content != CONTENT_EXPIRED_MARKER
            && timestamp_is_expired(timestamp, context.content_cutoff_unix_seconds)
        {
            work.inbound_transitions.push(transition_id);
        }
    }
    Ok(work)
}

fn redact_content(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
    work: &ContentWork,
    options: SweepOptions,
    counts: &mut SweepCounts,
    redacted_ids: &mut Vec<RemovedId>,
) -> Result<(), SweepTransactionFailure> {
    for (draft_id, revision) in &work.draft_revisions {
        let changed = transaction
            .execute(
                "UPDATE draft_revisions SET body = ?4
                 WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
                params![
                    &context.repository_id,
                    draft_id,
                    revision,
                    CONTENT_EXPIRED_MARKER
                ],
            )
            .map_err(|error| {
                SweepTransactionFailure::new(
                    SweepPhase::Content,
                    db_error("redact draft text", error),
                )
            })?;
        if changed != 1 {
            return Err(SweepTransactionFailure::new(
                SweepPhase::Content,
                StateError::Transaction {
                    message: "retention draft row changed before redaction".to_owned(),
                },
            ));
        }
        counts.content_rows_redacted += 1;
        redacted_ids.push(RemovedId::DraftRevision {
            repository_id: context.repository_id.clone(),
            draft_id: draft_id.clone(),
            revision: *revision,
        });
        maybe_fail(
            options,
            FailurePoint::AfterContent(counts.content_rows_redacted),
            SweepPhase::Content,
        )?;
    }
    for item_id in &work.inbound_first {
        let changed = transaction
            .execute(
                "UPDATE inbound_items SET first_content = ?3
                 WHERE repository_id = ?1 AND item_id = ?2",
                params![&context.repository_id, item_id, CONTENT_EXPIRED_MARKER],
            )
            .map_err(|error| {
                SweepTransactionFailure::new(
                    SweepPhase::Content,
                    db_error("redact inbound first text", error),
                )
            })?;
        if changed != 1 {
            return Err(SweepTransactionFailure::new(
                SweepPhase::Content,
                StateError::Transaction {
                    message: "retention inbound first row changed before redaction".to_owned(),
                },
            ));
        }
        counts.content_rows_redacted += 1;
        redacted_ids.push(RemovedId::InboundItem {
            repository_id: context.repository_id.clone(),
            item_id: item_id.clone(),
        });
        maybe_fail(
            options,
            FailurePoint::AfterContent(counts.content_rows_redacted),
            SweepPhase::Content,
        )?;
    }
    for item_id in &work.inbound_current {
        let changed = transaction
            .execute(
                "UPDATE inbound_current_snapshots SET current_content = ?3
                 WHERE repository_id = ?1 AND item_id = ?2",
                params![&context.repository_id, item_id, CONTENT_EXPIRED_MARKER],
            )
            .map_err(|error| {
                SweepTransactionFailure::new(
                    SweepPhase::Content,
                    db_error("redact inbound current text", error),
                )
            })?;
        if changed != 1 {
            return Err(SweepTransactionFailure::new(
                SweepPhase::Content,
                StateError::Transaction {
                    message: "retention inbound current row changed before redaction".to_owned(),
                },
            ));
        }
        counts.content_rows_redacted += 1;
        redacted_ids.push(RemovedId::InboundCurrentSnapshot {
            repository_id: context.repository_id.clone(),
            item_id: item_id.clone(),
        });
        maybe_fail(
            options,
            FailurePoint::AfterContent(counts.content_rows_redacted),
            SweepPhase::Content,
        )?;
    }
    for transition_id in &work.inbound_transitions {
        let item_id: String = transaction
            .query_row(
                "SELECT item_id FROM inbound_item_transitions
                 WHERE repository_id = ?1 AND transition_id = ?2",
                params![&context.repository_id, transition_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                SweepTransactionFailure::new(
                    SweepPhase::Content,
                    db_error("read inbound transition owner", error),
                )
            })?;
        let changed = transaction
            .execute(
                "UPDATE inbound_item_transitions SET content = ?3
                 WHERE repository_id = ?1 AND transition_id = ?2",
                params![
                    &context.repository_id,
                    transition_id,
                    CONTENT_EXPIRED_MARKER
                ],
            )
            .map_err(|error| {
                SweepTransactionFailure::new(
                    SweepPhase::Content,
                    db_error("redact inbound transition text", error),
                )
            })?;
        if changed != 1 {
            return Err(SweepTransactionFailure::new(
                SweepPhase::Content,
                StateError::Transaction {
                    message: "retention inbound transition changed before redaction".to_owned(),
                },
            ));
        }
        counts.content_rows_redacted += 1;
        redacted_ids.push(RemovedId::InboundTransition {
            repository_id: context.repository_id.clone(),
            transition_id: transition_id.clone(),
        });
        // Keep the owner lookup above explicit: it verifies that the transition
        // still belongs to the selected repository before its text is replaced.
        let _ = item_id;
        maybe_fail(
            options,
            FailurePoint::AfterContent(counts.content_rows_redacted),
            SweepPhase::Content,
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct MetadataWork {
    drafts: BTreeSet<String>,
    inbound_items: BTreeSet<String>,
    draft_revisions: BTreeSet<(String, i64)>,
    inbound_transitions: BTreeSet<String>,
    inbound_current: BTreeSet<String>,
    acknowledgements: BTreeSet<String>,
    archives: BTreeSet<String>,
    reply_links: BTreeSet<String>,
    delivery_attempts: BTreeSet<String>,
    approvals: BTreeSet<String>,
    policy_activations: BTreeSet<String>,
    inbound_cursors: BTreeSet<String>,
    audit_events: BTreeSet<String>,
}

impl MetadataWork {
    fn is_empty(&self) -> bool {
        self.drafts.is_empty()
            && self.inbound_items.is_empty()
            && self.draft_revisions.is_empty()
            && self.inbound_transitions.is_empty()
            && self.inbound_current.is_empty()
            && self.acknowledgements.is_empty()
            && self.archives.is_empty()
            && self.reply_links.is_empty()
            && self.delivery_attempts.is_empty()
            && self.approvals.is_empty()
            && self.policy_activations.is_empty()
            && self.inbound_cursors.is_empty()
            && self.audit_events.is_empty()
    }
}

fn collect_metadata_work(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
    content: &ContentWork,
) -> Result<MetadataWork, StateError> {
    let mut work = MetadataWork::default();
    let mut statement = transaction
        .prepare(
            "SELECT draft_id, updated_at FROM drafts WHERE repository_id = ?1 ORDER BY draft_id ASC",
        )
        .map_err(|error| db_error("prepare draft metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan draft metadata rows", error))?;
    for row in rows {
        let (draft_id, updated_at) =
            row.map_err(|error| db_error("read draft metadata row", error))?;
        if is_expired(&updated_at, context.metadata_cutoff_unix_seconds)?
            && !content.fresh_drafts.contains(&draft_id)
        {
            work.drafts.insert(draft_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT item_id, created_at FROM inbound_items WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare inbound metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan inbound metadata rows", error))?;
    for row in rows {
        let (item_id, created_at) =
            row.map_err(|error| db_error("read inbound metadata row", error))?;
        if is_expired(&created_at, context.metadata_cutoff_unix_seconds)?
            && !content.fresh_inbound_items.contains(&item_id)
        {
            work.inbound_items.insert(item_id);
        }
    }

    for (draft_id, revision) in &content.all_draft_revisions {
        if work.drafts.contains(draft_id) {
            work.draft_revisions.insert((draft_id.clone(), *revision));
        }
    }

    collect_child_metadata(transaction, context, &mut work)?;
    collect_standalone_metadata(transaction, context, &mut work)?;
    Ok(work)
}

fn collect_child_metadata(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
    work: &mut MetadataWork,
) -> Result<(), StateError> {
    let mut statement = transaction
        .prepare(
            "SELECT item_id, acknowledged_at FROM inbound_acknowledgements
             WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare acknowledgement metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan acknowledgement metadata rows", error))?;
    for row in rows {
        let (item_id, timestamp) =
            row.map_err(|error| db_error("read acknowledgement metadata row", error))?;
        if work.inbound_items.contains(&item_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.acknowledgements.insert(item_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT item_id, archived_at FROM inbound_archives
             WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare archive metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan archive metadata rows", error))?;
    for row in rows {
        let (item_id, timestamp) =
            row.map_err(|error| db_error("read archive metadata row", error))?;
        if work.inbound_items.contains(&item_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.archives.insert(item_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT item_id, observed_at FROM inbound_current_snapshots
             WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare current snapshot metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan current snapshot metadata rows", error))?;
    for row in rows {
        let (item_id, timestamp) =
            row.map_err(|error| db_error("read current snapshot metadata row", error))?;
        if work.inbound_items.contains(&item_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.inbound_current.insert(item_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT transition_id, item_id, occurred_at FROM inbound_item_transitions
             WHERE repository_id = ?1 ORDER BY transition_id ASC",
        )
        .map_err(|error| db_error("prepare transition metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| db_error("scan transition metadata rows", error))?;
    for row in rows {
        let (transition_id, item_id, timestamp) =
            row.map_err(|error| db_error("read transition metadata row", error))?;
        if work.inbound_items.contains(&item_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.inbound_transitions.insert(transition_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT item_id, reply_draft_id, linked_at FROM inbound_reply_links
             WHERE repository_id = ?1 ORDER BY item_id ASC",
        )
        .map_err(|error| db_error("prepare reply-link metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| db_error("scan reply-link metadata rows", error))?;
    for row in rows {
        let (item_id, reply_draft_id, timestamp) =
            row.map_err(|error| db_error("read reply-link metadata row", error))?;
        if work.inbound_items.contains(&item_id)
            || work.drafts.contains(&reply_draft_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.reply_links.insert(item_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT attempt_id, draft_id, claimed_at FROM delivery_attempts
             WHERE repository_id = ?1 ORDER BY attempt_id ASC",
        )
        .map_err(|error| db_error("prepare delivery metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| db_error("scan delivery metadata rows", error))?;
    for row in rows {
        let (attempt_id, draft_id, timestamp) =
            row.map_err(|error| db_error("read delivery metadata row", error))?;
        if work.drafts.contains(&draft_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.delivery_attempts.insert(attempt_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT approval_id, draft_id, approved_at FROM approvals
             WHERE repository_id = ?1 ORDER BY approval_id ASC",
        )
        .map_err(|error| db_error("prepare approval metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| db_error("scan approval metadata rows", error))?;
    for row in rows {
        let (approval_id, draft_id, timestamp) =
            row.map_err(|error| db_error("read approval metadata row", error))?;
        if work.drafts.contains(&draft_id)
            || is_expired(&timestamp, context.metadata_cutoff_unix_seconds)?
        {
            work.approvals.insert(approval_id);
        }
    }

    Ok(())
}

fn collect_standalone_metadata(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
    work: &mut MetadataWork,
) -> Result<(), StateError> {
    let mut statement = transaction
        .prepare(
            "SELECT activation_id, activated_at FROM policy_activations
             WHERE repository_id = ?1 ORDER BY activation_id ASC",
        )
        .map_err(|error| db_error("prepare policy metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan policy metadata rows", error))?;
    for row in rows {
        let (activation_id, timestamp) =
            row.map_err(|error| db_error("read policy metadata row", error))?;
        if is_expired(&timestamp, context.metadata_cutoff_unix_seconds)? {
            work.policy_activations.insert(activation_id);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT alias, updated_at FROM inbound_cursors
             WHERE repository_id = ?1 ORDER BY alias ASC",
        )
        .map_err(|error| db_error("prepare cursor metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan cursor metadata rows", error))?;
    for row in rows {
        let (alias, timestamp) =
            row.map_err(|error| db_error("read cursor metadata row", error))?;
        if is_expired(&timestamp, context.metadata_cutoff_unix_seconds)? {
            work.inbound_cursors.insert(alias);
        }
    }

    let mut statement = transaction
        .prepare(
            "SELECT event_id, occurred_at FROM audit_events
             WHERE repository_id = ?1 ORDER BY event_id ASC",
        )
        .map_err(|error| db_error("prepare audit metadata scan", error))?;
    let rows = statement
        .query_map([&context.repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| db_error("scan audit metadata rows", error))?;
    for row in rows {
        let (event_id, timestamp) =
            row.map_err(|error| db_error("read audit metadata row", error))?;
        if is_expired(&timestamp, context.metadata_cutoff_unix_seconds)? {
            work.audit_events.insert(event_id);
        }
    }
    Ok(())
}

fn remove_metadata(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
    work: &MetadataWork,
    options: SweepOptions,
    counts: &mut SweepCounts,
    removed_ids: &mut RemovedIds,
) -> Result<(), SweepTransactionFailure> {
    // Delete dependent rows before their parents.  Every predicate includes
    // repository_id, so an identically named object in another repository is
    // never visible to this sweep.
    for draft_id in &work.drafts {
        delete_rows(
            transaction,
            "DELETE FROM inbound_reply_links WHERE repository_id = ?1 AND reply_draft_id = ?2",
            params![&context.repository_id, draft_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for item_id in &work.inbound_items {
        delete_rows(
            transaction,
            "DELETE FROM inbound_reply_links WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
        delete_rows(
            transaction,
            "DELETE FROM inbound_acknowledgements WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
        delete_rows(
            transaction,
            "DELETE FROM inbound_archives WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
        delete_rows(
            transaction,
            "DELETE FROM inbound_current_snapshots WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
        delete_rows(
            transaction,
            "DELETE FROM inbound_item_transitions WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for attempt_id in &work.delivery_attempts {
        delete_rows(
            transaction,
            "DELETE FROM delivery_attempts WHERE repository_id = ?1 AND attempt_id = ?2",
            params![&context.repository_id, attempt_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for approval_id in &work.approvals {
        delete_rows(
            transaction,
            "DELETE FROM approvals WHERE repository_id = ?1 AND approval_id = ?2",
            params![&context.repository_id, approval_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for (draft_id, revision) in &work.draft_revisions {
        delete_rows(
            transaction,
            "DELETE FROM draft_revisions WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
            params![&context.repository_id, draft_id, revision],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for draft_id in &work.drafts {
        delete_rows(
            transaction,
            "DELETE FROM drafts WHERE repository_id = ?1 AND draft_id = ?2",
            params![&context.repository_id, draft_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for item_id in &work.inbound_items {
        delete_rows(
            transaction,
            "DELETE FROM inbound_items WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for item_id in &work.acknowledgements {
        delete_rows(
            transaction,
            "DELETE FROM inbound_acknowledgements WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for item_id in &work.archives {
        delete_rows(
            transaction,
            "DELETE FROM inbound_archives WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for item_id in &work.reply_links {
        delete_rows(
            transaction,
            "DELETE FROM inbound_reply_links WHERE repository_id = ?1 AND item_id = ?2",
            params![&context.repository_id, item_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for activation_id in &work.policy_activations {
        delete_rows(
            transaction,
            "DELETE FROM policy_activations WHERE repository_id = ?1 AND activation_id = ?2",
            params![&context.repository_id, activation_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for alias in &work.inbound_cursors {
        delete_rows(
            transaction,
            "DELETE FROM inbound_cursors WHERE repository_id = ?1 AND alias = ?2",
            params![&context.repository_id, alias],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }
    for event_id in &work.audit_events {
        delete_rows(
            transaction,
            "DELETE FROM audit_events WHERE repository_id = ?1 AND event_id = ?2",
            params![&context.repository_id, event_id],
            SweepPhase::Metadata,
            counts,
            options,
        )?;
    }

    removed_ids.items = removed_id_list(context, work);
    Ok(())
}

fn delete_rows<P>(
    transaction: &StateTransaction<'_>,
    sql: &str,
    params: P,
    phase: SweepPhase,
    counts: &mut SweepCounts,
    options: SweepOptions,
) -> Result<(), SweepTransactionFailure>
where
    P: rusqlite::Params,
{
    let changed = transaction.execute(sql, params).map_err(|error| {
        SweepTransactionFailure::new(phase, db_error("remove retained metadata", error))
    })?;
    counts.metadata_rows_removed += changed;
    maybe_fail(
        options,
        FailurePoint::AfterMetadata(counts.metadata_rows_removed),
        phase,
    )
}

fn removed_id_list(context: &RetentionCutoffs, work: &MetadataWork) -> Vec<RemovedId> {
    let mut ids = Vec::new();
    ids.extend(
        work.draft_revisions
            .iter()
            .map(|(draft_id, revision)| RemovedId::DraftRevision {
                repository_id: context.repository_id.clone(),
                draft_id: draft_id.clone(),
                revision: *revision,
            }),
    );
    ids.extend(work.drafts.iter().map(|draft_id| RemovedId::Draft {
        repository_id: context.repository_id.clone(),
        draft_id: draft_id.clone(),
    }));
    ids.extend(
        work.inbound_transitions
            .iter()
            .map(|transition_id| RemovedId::InboundTransition {
                repository_id: context.repository_id.clone(),
                transition_id: transition_id.clone(),
            }),
    );
    ids.extend(
        work.inbound_current
            .iter()
            .map(|item_id| RemovedId::InboundCurrentSnapshot {
                repository_id: context.repository_id.clone(),
                item_id: item_id.clone(),
            }),
    );
    ids.extend(
        work.acknowledgements
            .iter()
            .map(|item_id| RemovedId::Acknowledgement {
                repository_id: context.repository_id.clone(),
                item_id: item_id.clone(),
            }),
    );
    ids.extend(work.archives.iter().map(|item_id| RemovedId::Archive {
        repository_id: context.repository_id.clone(),
        item_id: item_id.clone(),
    }));
    ids.extend(work.reply_links.iter().map(|item_id| RemovedId::ReplyLink {
        repository_id: context.repository_id.clone(),
        item_id: item_id.clone(),
    }));
    ids.extend(
        work.inbound_items
            .iter()
            .map(|item_id| RemovedId::InboundItem {
                repository_id: context.repository_id.clone(),
                item_id: item_id.clone(),
            }),
    );
    ids.extend(
        work.delivery_attempts
            .iter()
            .map(|attempt_id| RemovedId::DeliveryAttempt {
                repository_id: context.repository_id.clone(),
                attempt_id: attempt_id.clone(),
            }),
    );
    ids.extend(
        work.approvals
            .iter()
            .map(|approval_id| RemovedId::Approval {
                repository_id: context.repository_id.clone(),
                approval_id: approval_id.clone(),
            }),
    );
    ids.extend(
        work.policy_activations
            .iter()
            .map(|activation_id| RemovedId::PolicyActivation {
                repository_id: context.repository_id.clone(),
                activation_id: activation_id.clone(),
            }),
    );
    ids.extend(
        work.inbound_cursors
            .iter()
            .map(|alias| RemovedId::InboundCursor {
                repository_id: context.repository_id.clone(),
                alias: alias.clone(),
            }),
    );
    ids.extend(
        work.audit_events
            .iter()
            .map(|event_id| RemovedId::AuditEvent {
                repository_id: context.repository_id.clone(),
                event_id: event_id.clone(),
            }),
    );
    ids
}

fn append_retention_audit(
    transaction: &StateTransaction<'_>,
    context: &RetentionCutoffs,
    counts: &SweepCounts,
) -> Result<String, StateError> {
    let event_id = retention_event_id(context);
    if transaction
        .repositories()
        .audit()
        .get(&context.repository_id, &event_id)?
        .is_some()
    {
        return Ok(event_id);
    }
    let content_days = context
        .as_of_unix_seconds
        .saturating_sub(context.content_cutoff_unix_seconds)
        / SECONDS_PER_DAY;
    let metadata_days = context
        .as_of_unix_seconds
        .saturating_sub(context.metadata_cutoff_unix_seconds)
        / SECONDS_PER_DAY;
    let metadata = serde_json::json!({
        "content_days": content_days,
        "metadata_days": metadata_days,
        "content_rows_redacted": counts.content_rows_redacted,
        "metadata_rows_removed": counts.metadata_rows_removed,
        "content_cutoff_unix_seconds": context.content_cutoff_unix_seconds,
        "metadata_cutoff_unix_seconds": context.metadata_cutoff_unix_seconds,
    });
    let metadata_json = serde_json::to_string(&metadata).map_err(|_| StateError::Transaction {
        message: "retention audit summary could not be serialized".to_owned(),
    })?;
    let mut input = AuditEventInput::new(
        context.repository_id.clone(),
        event_id.clone(),
        "retention",
        context.repository_id.clone(),
        "swept",
        context.as_of.clone(),
        "system",
        "success",
    );
    input.metadata_json = metadata_json;
    transaction.append_audit_event(&input)?;
    Ok(event_id)
}

fn retention_event_id(context: &RetentionCutoffs) -> String {
    let mut digest = Sha256::new();
    for value in [
        context.repository_id.as_str(),
        context.as_of.as_str(),
        context.content_cutoff.as_str(),
        context.metadata_cutoff.as_str(),
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    let encoded = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("retention-{encoded}")
}
