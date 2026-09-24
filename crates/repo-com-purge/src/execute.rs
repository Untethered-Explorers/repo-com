//! Confirmed, repository-scoped local purge execution.

use std::collections::BTreeSet;
use std::fmt;

use repo_com_audit::AuditEvent;
use repo_com_foundation::TtyMode;
use repo_com_state::{StateStore, StateTransaction};
use rusqlite::{OptionalExtension, params};

use crate::plan::{
    PURGED_CONTENT_MARKER, PurgeCounts, PurgeCutoff, PurgeError, PurgePlan, PurgeRequest,
    PurgeResult, PurgeScope, PurgeWork, build_plan_in_connection, collect_work,
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

/// Maximum number of row operations issued in one local batch.
pub const MAX_PURGE_BATCH_SIZE: usize = 4_096;

/// A deterministic failure point used to prove purge rollback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PurgeFailurePoint {
    /// Fail before content work.
    BeforeContent,
    /// Fail after the selected number of content rows.
    AfterContent(usize),
    /// Fail before metadata work.
    BeforeMetadata,
    /// Fail after the selected number of metadata rows.
    AfterMetadata(usize),
    /// Fail before the count-only audit append.
    BeforeAudit,
    /// Fail after the audit append but before commit.
    AfterAudit,
    /// Fail immediately before commit.
    BeforeCommit,
}

impl fmt::Display for PurgeFailurePoint {
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

/// Compatibility name for the task-local failure injection type.
pub type FailurePoint = PurgeFailurePoint;

/// Options controlling one confirmed local transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeExecuteOptions {
    /// Canonical UTC timestamp placed on the count-only audit event.
    pub executed_at: String,
    /// Maximum number of row operations issued in one bounded batch.
    pub batch_size: usize,
    /// Optional deterministic failure point.
    pub failure_point: Option<PurgeFailurePoint>,
}

impl PurgeExecuteOptions {
    /// Creates ordinary options with a caller-supplied audit timestamp.
    #[must_use]
    pub fn new(executed_at: impl Into<String>) -> Self {
        Self {
            executed_at: executed_at.into(),
            batch_size: 256,
            failure_point: None,
        }
    }

    /// Selects a bounded batch size.
    #[must_use]
    pub const fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Selects a deterministic failure point.
    #[must_use]
    pub const fn with_failure_point(mut self, failure_point: PurgeFailurePoint) -> Self {
        self.failure_point = Some(failure_point);
        self
    }
}

/// A caller-collected exact confirmation. It contains no prompt bypass and no
/// reusable authority: the executor recomputes the current plan in its
/// transaction before every mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeConfirmation {
    /// Repository shown in the exact preview.
    pub repository_id: String,
    /// Scope shown in the exact preview.
    pub scope: PurgeScope,
    /// Cutoff shown in the exact preview.
    pub cutoff: PurgeCutoff,
    /// Configuration hash shown in the exact preview.
    pub config_hash: String,
    /// Plan hash typed or confirmed by the operator.
    pub plan_hash: String,
    /// Explicit caller-supplied TTY mode.
    pub tty_mode: TtyMode,
}

impl PurgeConfirmation {
    /// Copies all exact bindings from a plan. A non-TTY mode remains
    /// representable so the executor can fail closed with a typed result.
    #[must_use]
    pub fn from_plan(plan: &PurgePlan, tty_mode: TtyMode) -> Self {
        Self {
            repository_id: plan.repository_id.clone(),
            scope: plan.scope,
            cutoff: plan.cutoff.clone(),
            config_hash: plan.config_hash.clone(),
            plan_hash: plan.plan_hash.clone(),
            tty_mode,
        }
    }

    /// Creates a confirmation only when the explicit mode is interactive.
    pub fn confirmed(plan: &PurgePlan, tty_mode: TtyMode) -> PurgeResult<Self> {
        if !tty_mode.is_tty() {
            return Err(PurgeError::TtyRequired);
        }
        Ok(Self::from_plan(plan, tty_mode))
    }

    /// Returns the explicit TTY mode collected by the interaction adapter.
    #[must_use]
    pub const fn tty_mode(&self) -> TtyMode {
        self.tty_mode
    }

    /// Returns the exact confirmed plan hash.
    #[must_use]
    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }

    /// Validates the confirmation against the exact plan before any write.
    pub fn require_for(&self, plan: &PurgePlan) -> PurgeResult<()> {
        if !self.tty_mode.is_tty() {
            return Err(PurgeError::TtyRequired);
        }
        if self.repository_id != plan.repository_id {
            return Err(PurgeError::RepositoryMismatch);
        }
        if self.scope != plan.scope {
            return Err(PurgeError::ScopeMismatch);
        }
        if self.cutoff != plan.cutoff {
            return Err(PurgeError::CutoffMismatch);
        }
        if self.config_hash != plan.config_hash {
            return Err(PurgeError::ConfigurationHashMismatch);
        }
        if self.plan_hash != plan.plan_hash {
            return Err(PurgeError::PlanHashMismatch);
        }
        Ok(())
    }

    /// Returns a copy with a different repository binding.
    #[must_use]
    pub fn with_repository_id(mut self, repository_id: impl Into<String>) -> Self {
        self.repository_id = repository_id.into();
        self
    }

    /// Returns a copy with a different scope binding.
    #[must_use]
    pub const fn with_scope(mut self, scope: PurgeScope) -> Self {
        self.scope = scope;
        self
    }

    /// Returns a copy with a different cutoff binding.
    #[must_use]
    pub fn with_cutoff(mut self, cutoff: PurgeCutoff) -> Self {
        self.cutoff = cutoff;
        self
    }

    /// Returns a copy with a different configuration-hash binding.
    #[must_use]
    pub fn with_config_hash(mut self, config_hash: impl Into<String>) -> Self {
        self.config_hash = config_hash.into();
        self
    }

    /// Returns a copy with a different plan-hash binding.
    #[must_use]
    pub fn with_plan_hash(mut self, plan_hash: impl Into<String>) -> Self {
        self.plan_hash = plan_hash.into();
        self
    }
}

/// The committed result of a confirmed purge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeExecution {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact plan hash that was committed.
    pub plan_hash: String,
    /// Actual count-only row counts committed by this transaction.
    pub counts: PurgeCounts,
    /// Stable local audit event identifier.
    pub audit_event_id: String,
    /// Canonical UTC audit timestamp.
    pub executed_at: String,
}

impl PurgeExecution {
    /// Returns the committed plan hash.
    #[must_use]
    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }

    /// Returns the committed count-only audit identifier.
    #[must_use]
    pub fn audit_event_id(&self) -> &str {
        &self.audit_event_id
    }
}

/// An owned local state store plus one-shot confirmed-plan safeguards.
pub struct PurgeExecutor {
    state: StateStore,
    failed_plan_hash: Option<String>,
    consumed_plan_hashes: BTreeSet<String>,
}

impl fmt::Debug for PurgeExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PurgeExecutor")
            .field("state", &self.state)
            .field("failed_plan_hash", &self.failed_plan_hash)
            .field("consumed_plan_count", &self.consumed_plan_hashes.len())
            .finish()
    }
}

impl PurgeExecutor {
    /// Creates an executor over an already opened local state store.
    #[must_use]
    pub const fn new(state: StateStore) -> Self {
        Self {
            state,
            failed_plan_hash: None,
            consumed_plan_hashes: BTreeSet::new(),
        }
    }

    /// Compatibility alias for [`Self::new`].
    #[must_use]
    pub fn from_state(state: StateStore) -> Self {
        Self::new(state)
    }

    /// Returns read-only access to the local state store.
    #[must_use]
    pub const fn state(&self) -> &StateStore {
        &self.state
    }

    /// Returns mutable access for explicit local composition.
    pub const fn state_mut(&mut self) -> &mut StateStore {
        &mut self.state
    }

    /// Consumes the executor and returns its local state store.
    #[must_use]
    pub fn into_state(self) -> StateStore {
        self.state
    }

    /// Accepts a newly generated plan after a failed execution attempt.
    ///
    /// A failed attempt is deliberately not retryable with the old plan
    /// value. The caller must generate a fresh plan and explicitly clear the
    /// one-shot failure guard before presenting the new confirmation.
    pub fn clear_failed_plan(&mut self) {
        self.failed_plan_hash = None;
    }

    /// Compatibility alias for [`Self::clear_failed_plan`].
    pub fn accept_replan(&mut self) {
        self.clear_failed_plan();
    }

    /// Executes a confirmed plan with ordinary bounded options.
    pub fn execute(
        &mut self,
        plan: &PurgePlan,
        confirmation: &PurgeConfirmation,
        executed_at: impl Into<String>,
    ) -> PurgeResult<PurgeExecution> {
        self.execute_with_options(plan, confirmation, PurgeExecuteOptions::new(executed_at))
    }

    /// Compatibility alias for [`Self::execute`].
    pub fn execute_at(
        &mut self,
        plan: &PurgePlan,
        confirmation: &PurgeConfirmation,
        executed_at: impl Into<String>,
    ) -> PurgeResult<PurgeExecution> {
        self.execute(plan, confirmation, executed_at)
    }

    /// Executes with an explicit TTY mode supplied by the caller. A mismatch
    /// between the caller mode and the collected confirmation fails closed.
    pub fn execute_with_tty(
        &mut self,
        plan: &PurgePlan,
        confirmation: &PurgeConfirmation,
        tty_mode: TtyMode,
        executed_at: impl Into<String>,
    ) -> PurgeResult<PurgeExecution> {
        if !tty_mode.is_tty() || confirmation.tty_mode() != tty_mode {
            return Err(PurgeError::TtyRequired);
        }
        self.execute(plan, confirmation, executed_at)
    }

    /// Executes with explicit bounded-batch and failure-injection options.
    pub fn execute_with_options(
        &mut self,
        plan: &PurgePlan,
        confirmation: &PurgeConfirmation,
        options: PurgeExecuteOptions,
    ) -> PurgeResult<PurgeExecution> {
        validate_execution_timestamp(&options.executed_at)?;
        if options.batch_size == 0 || options.batch_size > MAX_PURGE_BATCH_SIZE {
            return Err(PurgeError::InvalidPlan);
        }
        confirmation.require_for(plan)?;
        plan.validate()?;
        if self.failed_plan_hash.as_deref() == Some(plan.plan_hash.as_str()) {
            return Err(PurgeError::ReplanRequired);
        }
        if self.consumed_plan_hashes.contains(&plan.plan_hash) {
            return Err(PurgeError::PlanAlreadyExecuted);
        }

        let request =
            PurgeRequest::new(plan.repository_id.clone(), plan.scope, plan.cutoff.clone());
        let mut transaction = self
            .state
            .begin_transaction()
            .map_err(|_| storage("begin transaction"))?;
        let result = execute_transaction(&mut transaction, plan, &request, &options);
        match result {
            Ok((counts, audit_event_id)) => {
                if options.failure_point == Some(PurgeFailurePoint::BeforeCommit) {
                    let rollback = transaction.rollback();
                    self.failed_plan_hash = Some(plan.plan_hash.clone());
                    return match rollback {
                        Ok(()) => Err(injected(PurgeFailurePoint::BeforeCommit)),
                        Err(_) => Err(storage("rollback transaction")),
                    };
                }
                if transaction.commit().is_err() {
                    self.failed_plan_hash = Some(plan.plan_hash.clone());
                    return Err(storage("commit transaction"));
                }
                self.consumed_plan_hashes.insert(plan.plan_hash.clone());
                Ok(PurgeExecution {
                    repository_id: plan.repository_id.clone(),
                    plan_hash: plan.plan_hash.clone(),
                    counts,
                    audit_event_id,
                    executed_at: options.executed_at,
                })
            }
            Err(error) => {
                let _ = transaction.rollback();
                self.failed_plan_hash = Some(plan.plan_hash.clone());
                Err(error)
            }
        }
    }
}

/// Executes one plan using a caller-owned executor.
pub fn execute_purge(
    mut executor: PurgeExecutor,
    plan: &PurgePlan,
    confirmation: &PurgeConfirmation,
    executed_at: impl Into<String>,
) -> PurgeResult<(PurgeExecution, PurgeExecutor)> {
    let result = executor.execute(plan, confirmation, executed_at)?;
    Ok((result, executor))
}

fn execute_transaction(
    transaction: &mut StateTransaction<'_>,
    plan: &PurgePlan,
    request: &PurgeRequest,
    options: &PurgeExecuteOptions,
) -> PurgeResult<(PurgeCounts, String)> {
    let current = build_plan_in_connection(transaction.connection(), request)?;
    if current.config_hash != plan.config_hash {
        return Err(PurgeError::ConfigurationHashMismatch);
    }
    if current.counts != plan.counts || current.plan_hash != plan.plan_hash {
        return Err(PurgeError::ReplanRequired);
    }
    let work = collect_work(transaction.connection(), request)?;
    let mut counts = PurgeCounts {
        table_rows: plan
            .counts
            .table_rows
            .keys()
            .cloned()
            .map(|table| (table, 0))
            .collect(),
        ..PurgeCounts::default()
    };
    match plan.scope {
        PurgeScope::Content => execute_content(transaction, &work, options, &mut counts)?,
        PurgeScope::Metadata => execute_metadata(transaction, &work, options, &mut counts)?,
        PurgeScope::All => execute_all(transaction, &work, options, &mut counts)?,
    }
    if counts != plan.counts {
        return Err(PurgeError::ReplanRequired);
    }
    maybe_fail(options, PurgeFailurePoint::BeforeAudit)?;
    let audit_event_id = append_purge_audit(transaction, plan, &counts, &options.executed_at)?;
    maybe_fail(options, PurgeFailurePoint::AfterAudit)?;
    Ok((counts, audit_event_id))
}

fn execute_content(
    transaction: &StateTransaction<'_>,
    work: &PurgeWork,
    options: &PurgeExecuteOptions,
    counts: &mut PurgeCounts,
) -> PurgeResult<()> {
    maybe_fail(options, PurgeFailurePoint::BeforeContent)?;
    let mut dropped = DroppedTriggers::default();
    if !work.content_draft_revisions.is_empty() {
        drop_trigger(transaction, DRAFT_UPDATE_TRIGGER, &mut dropped)?;
    }
    if !work.content_inbound_items.is_empty() {
        drop_trigger(transaction, INBOUND_UPDATE_TRIGGER, &mut dropped)?;
    }

    for chunk in work.content_draft_revisions.chunks(options.batch_size) {
        for (draft_id, revision) in chunk {
            let changed = transaction
                .execute(
                    "UPDATE draft_revisions SET body = ?3
                     WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?4",
                    params![
                        work.repository_id,
                        draft_id,
                        PURGED_CONTENT_MARKER,
                        revision
                    ],
                )
                .map_err(|_| storage("purge draft content"))?;
            expect_one(changed)?;
            record_count(counts, "draft_revisions", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }
    for chunk in work.content_inbound_items.chunks(options.batch_size) {
        for item_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE inbound_items SET first_content = ?2, first_attachments_json = '[]'
                     WHERE repository_id = ?1 AND item_id = ?3",
                    params![work.repository_id, PURGED_CONTENT_MARKER, item_id],
                )
                .map_err(|_| storage("purge inbound content"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_items", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }
    for chunk in work.content_inbound_current.chunks(options.batch_size) {
        for item_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE inbound_current_snapshots
                     SET current_content = CASE WHEN deleted = 1 THEN NULL ELSE ?2 END,
                         current_attachments_json = '[]'
                     WHERE repository_id = ?1 AND item_id = ?3",
                    params![work.repository_id, PURGED_CONTENT_MARKER, item_id],
                )
                .map_err(|_| storage("purge inbound current content"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_current_snapshots", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }
    for chunk in work.content_inbound_transitions.chunks(options.batch_size) {
        for transition_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE inbound_item_transitions SET content = ?2
                     WHERE repository_id = ?1 AND transition_id = ?3",
                    params![work.repository_id, PURGED_CONTENT_MARKER, transition_id],
                )
                .map_err(|_| storage("purge inbound transition content"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_item_transitions", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }
    dropped.restore(transaction)
}

fn execute_metadata(
    transaction: &StateTransaction<'_>,
    work: &PurgeWork,
    options: &PurgeExecuteOptions,
    counts: &mut PurgeCounts,
) -> PurgeResult<()> {
    maybe_fail(options, PurgeFailurePoint::BeforeMetadata)?;
    let mut dropped = DroppedTriggers::default();
    if !work.metadata_draft_revisions.is_empty() {
        drop_trigger(transaction, DRAFT_UPDATE_TRIGGER, &mut dropped)?;
    }
    if !work.metadata_inbound_items.is_empty() {
        drop_trigger(transaction, INBOUND_UPDATE_TRIGGER, &mut dropped)?;
    }
    if !work.metadata_audit_events.is_empty() {
        drop_trigger(transaction, AUDIT_DELETE_TRIGGER, &mut dropped)?;
    }

    for chunk in work.metadata_drafts.chunks(options.batch_size) {
        for draft_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE drafts SET metadata_json = '{}'
                     WHERE repository_id = ?1 AND draft_id = ?2",
                    params![work.repository_id, draft_id],
                )
                .map_err(|_| storage("purge draft metadata"))?;
            expect_one(changed)?;
            record_count(counts, "drafts", 1, Category::Metadata);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterMetadata(counts.metadata_rows as usize),
            )?;
        }
    }
    for chunk in work.metadata_draft_revisions.chunks(options.batch_size) {
        for (draft_id, revision) in chunk {
            let changed = transaction
                .execute(
                    "UPDATE draft_revisions SET metadata_json = '{}'
                     WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
                    params![work.repository_id, draft_id, revision],
                )
                .map_err(|_| storage("purge revision metadata"))?;
            expect_one(changed)?;
            record_count(counts, "draft_revisions", 1, Category::Metadata);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterMetadata(counts.metadata_rows as usize),
            )?;
        }
    }
    for chunk in work.metadata_inbound_items.chunks(options.batch_size) {
        for item_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE inbound_items SET first_attachments_json = '[]'
                     WHERE repository_id = ?1 AND item_id = ?2",
                    params![work.repository_id, item_id],
                )
                .map_err(|_| storage("purge inbound metadata"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_items", 1, Category::Metadata);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterMetadata(counts.metadata_rows as usize),
            )?;
        }
    }
    for chunk in work.metadata_inbound_current.chunks(options.batch_size) {
        for item_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE inbound_current_snapshots SET current_attachments_json = '[]'
                     WHERE repository_id = ?1 AND item_id = ?2",
                    params![work.repository_id, item_id],
                )
                .map_err(|_| storage("purge current metadata"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_current_snapshots", 1, Category::Metadata);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterMetadata(counts.metadata_rows as usize),
            )?;
        }
    }
    for chunk in work.metadata_inbound_transitions.chunks(options.batch_size) {
        for transition_id in chunk {
            let changed = transaction
                .execute(
                    "UPDATE inbound_item_transitions SET metadata_json = '{}'
                     WHERE repository_id = ?1 AND transition_id = ?2",
                    params![work.repository_id, transition_id],
                )
                .map_err(|_| storage("purge transition metadata"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_item_transitions", 1, Category::Metadata);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterMetadata(counts.metadata_rows as usize),
            )?;
        }
    }
    delete_metadata_rows(transaction, work, options, counts)?;
    dropped.restore(transaction)
}

fn execute_all(
    transaction: &StateTransaction<'_>,
    work: &PurgeWork,
    options: &PurgeExecuteOptions,
    counts: &mut PurgeCounts,
) -> PurgeResult<()> {
    maybe_fail(options, PurgeFailurePoint::BeforeContent)?;
    let mut dropped = DroppedTriggers::default();
    for trigger in [
        DRAFT_UPDATE_TRIGGER,
        DRAFT_DELETE_TRIGGER,
        INBOUND_UPDATE_TRIGGER,
        INBOUND_DELETE_TRIGGER,
        AUDIT_DELETE_TRIGGER,
    ] {
        drop_trigger(transaction, trigger, &mut dropped)?;
    }

    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_reply_links",
        "item_id",
        &work.metadata_reply_links,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_acknowledgements",
        "item_id",
        &work.metadata_acknowledgements,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_archives",
        "item_id",
        &work.metadata_archives,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "delivery_attempts",
        "attempt_id",
        &work.metadata_delivery_attempts,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "approvals",
        "approval_id",
        &work.metadata_approvals,
        Category::Metadata,
    )?;

    for chunk in work.content_draft_revisions.chunks(options.batch_size) {
        for (draft_id, revision) in chunk {
            let changed = transaction
                .execute(
                    "DELETE FROM draft_revisions
                     WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
                    params![work.repository_id, draft_id, revision],
                )
                .map_err(|_| storage("purge draft revision"))?;
            expect_one(changed)?;
            record_count(counts, "draft_revisions", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }
    for chunk in work.content_inbound_current.chunks(options.batch_size) {
        for item_id in chunk {
            let changed = transaction
                .execute(
                    "DELETE FROM inbound_current_snapshots
                     WHERE repository_id = ?1 AND item_id = ?2",
                    params![work.repository_id, item_id],
                )
                .map_err(|_| storage("purge inbound current"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_current_snapshots", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }
    for chunk in work.content_inbound_transitions.chunks(options.batch_size) {
        for transition_id in chunk {
            let changed = transaction
                .execute(
                    "DELETE FROM inbound_item_transitions
                     WHERE repository_id = ?1 AND transition_id = ?2",
                    params![work.repository_id, transition_id],
                )
                .map_err(|_| storage("purge inbound transition"))?;
            expect_one(changed)?;
            record_count(counts, "inbound_item_transitions", 1, Category::Content);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterContent(counts.content_rows as usize),
            )?;
        }
    }

    let mut draft_ids = work
        .content_draft_revisions
        .iter()
        .map(|(draft_id, _)| draft_id.clone())
        .collect::<BTreeSet<_>>();
    draft_ids.extend(work.metadata_drafts.iter().cloned());
    for draft_id in draft_ids {
        let changed = transaction
            .execute(
                "DELETE FROM drafts WHERE repository_id = ?1 AND draft_id = ?2",
                params![work.repository_id, &draft_id],
            )
            .map_err(|_| storage("purge draft parent"))?;
        expect_one(changed)?;
        record_count(counts, "drafts", 1, Category::Content);
        maybe_fail(
            options,
            PurgeFailurePoint::AfterContent(counts.content_rows as usize),
        )?;
    }
    for item_id in &work.content_inbound_items {
        let changed = transaction
            .execute(
                "DELETE FROM inbound_items WHERE repository_id = ?1 AND item_id = ?2",
                params![work.repository_id, item_id],
            )
            .map_err(|_| storage("purge inbound parent"))?;
        expect_one(changed)?;
        record_count(counts, "inbound_items", 1, Category::Content);
        maybe_fail(
            options,
            PurgeFailurePoint::AfterContent(counts.content_rows as usize),
        )?;
    }
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "policy_activations",
        "activation_id",
        &work.metadata_policy_activations,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_cursors",
        "alias",
        &work.metadata_inbound_cursors,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "audit_events",
        "event_id",
        &work.metadata_audit_events,
        Category::Metadata,
    )?;
    dropped.restore(transaction)
}

fn delete_metadata_rows(
    transaction: &StateTransaction<'_>,
    work: &PurgeWork,
    options: &PurgeExecuteOptions,
    counts: &mut PurgeCounts,
) -> PurgeResult<()> {
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_reply_links",
        "item_id",
        &work.metadata_reply_links,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_acknowledgements",
        "item_id",
        &work.metadata_acknowledgements,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_archives",
        "item_id",
        &work.metadata_archives,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "delivery_attempts",
        "attempt_id",
        &work.metadata_delivery_attempts,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "approvals",
        "approval_id",
        &work.metadata_approvals,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "policy_activations",
        "activation_id",
        &work.metadata_policy_activations,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "inbound_cursors",
        "alias",
        &work.metadata_inbound_cursors,
        Category::Metadata,
    )?;
    delete_id_rows(
        transaction,
        work,
        options,
        counts,
        "audit_events",
        "event_id",
        &work.metadata_audit_events,
        Category::Metadata,
    )
}

#[allow(clippy::too_many_arguments)]
fn delete_id_rows(
    transaction: &StateTransaction<'_>,
    work: &PurgeWork,
    options: &PurgeExecuteOptions,
    counts: &mut PurgeCounts,
    table: &str,
    column: &str,
    ids: &[String],
    category: Category,
) -> PurgeResult<()> {
    for chunk in ids.chunks(options.batch_size) {
        for id in chunk {
            let sql = format!("DELETE FROM {table} WHERE repository_id = ?1 AND {column} = ?2");
            let changed = transaction
                .execute(&sql, params![work.repository_id, id])
                .map_err(|_| storage("purge metadata row"))?;
            expect_one(changed)?;
            record_count(counts, table, 1, category);
            maybe_fail(
                options,
                PurgeFailurePoint::AfterMetadata(counts.metadata_rows as usize),
            )?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Category {
    Content,
    Metadata,
}

fn record_count(counts: &mut PurgeCounts, table: &str, amount: u64, category: Category) {
    let entry = counts.table_rows.entry(table.to_owned()).or_insert(0);
    *entry = entry.saturating_add(amount);
    match category {
        Category::Content => counts.content_rows = counts.content_rows.saturating_add(amount),
        Category::Metadata => counts.metadata_rows = counts.metadata_rows.saturating_add(amount),
    }
    counts.total_rows = counts.content_rows.saturating_add(counts.metadata_rows);
}

fn expect_one(changed: usize) -> PurgeResult<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(PurgeError::ReplanRequired)
    }
}

fn maybe_fail(options: &PurgeExecuteOptions, point: PurgeFailurePoint) -> PurgeResult<()> {
    if options.failure_point == Some(point) {
        Err(injected(point))
    } else {
        Ok(())
    }
}

fn injected(point: PurgeFailurePoint) -> PurgeError {
    PurgeError::InjectedFailure {
        point: point.to_string(),
    }
}

fn validate_execution_timestamp(value: &str) -> PurgeResult<()> {
    let cutoff = PurgeCutoff::from_rfc3339(value)?;
    if cutoff.utc == value {
        Ok(())
    } else {
        Err(PurgeError::InvalidCutoff)
    }
}

fn append_purge_audit(
    transaction: &StateTransaction<'_>,
    plan: &PurgePlan,
    counts: &PurgeCounts,
    executed_at: &str,
) -> PurgeResult<String> {
    let event_id = format!("purge-{}", plan.plan_hash);
    if transaction
        .repositories()
        .audit()
        .get(&plan.repository_id, &event_id)
        .map_err(|_| storage("check purge audit"))?
        .is_some()
    {
        return Ok(event_id);
    }
    let metadata = serde_json::json!({
        "schema_version": 1,
        "category": plan.scope.as_str(),
        "cutoff_unix_seconds": plan.cutoff.unix_seconds,
        "counts": {
            "primary": counts.content_rows,
            "secondary": counts.metadata_rows,
            "total": counts.total_rows,
        },
    });
    let event = AuditEvent::with_metadata(
        &plan.repository_id,
        &event_id,
        "purge",
        &plan.repository_id,
        "purged",
        executed_at,
        "operator",
        "success",
        metadata,
    );
    repo_com_audit::append_in_transaction(transaction, &event)
        .map_err(|_| storage("append purge audit"))?;
    Ok(event_id)
}

#[derive(Default)]
struct DroppedTriggers {
    names: Vec<&'static str>,
}

impl DroppedTriggers {
    fn restore(self, transaction: &StateTransaction<'_>) -> PurgeResult<()> {
        for name in self.names {
            recreate_trigger(transaction, name)?;
        }
        Ok(())
    }
}

fn drop_trigger(
    transaction: &StateTransaction<'_>,
    name: &'static str,
    dropped: &mut DroppedTriggers,
) -> PurgeResult<()> {
    let present = transaction
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
            [name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| storage("read local invariant"))?;
    if present.is_none() {
        return Err(PurgeError::InvalidPlan);
    }
    transaction
        .execute_batch(&format!("DROP TRIGGER {name}"))
        .map_err(|_| storage("disable local invariant"))?;
    dropped.names.push(name);
    Ok(())
}

fn recreate_trigger(transaction: &StateTransaction<'_>, name: &str) -> PurgeResult<()> {
    let sql = match name {
        DRAFT_UPDATE_TRIGGER => DRAFT_UPDATE_TRIGGER_SQL,
        DRAFT_DELETE_TRIGGER => DRAFT_DELETE_TRIGGER_SQL,
        INBOUND_UPDATE_TRIGGER => INBOUND_UPDATE_TRIGGER_SQL,
        INBOUND_DELETE_TRIGGER => INBOUND_DELETE_TRIGGER_SQL,
        AUDIT_DELETE_TRIGGER => AUDIT_DELETE_TRIGGER_SQL,
        _ => return Err(PurgeError::InvalidPlan),
    };
    transaction
        .execute_batch(sql)
        .map_err(|_| storage("restore local invariant"))
}

fn storage(operation: &'static str) -> PurgeError {
    PurgeError::Storage { operation }
}
