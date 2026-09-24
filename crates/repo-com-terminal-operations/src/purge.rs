//! Safe purge projections and exact-scope confirmation identities.
//!
//! Planning and execution remain domain-owned.  This module only copies the
//! bounded, non-secret facts needed by a terminal view and the identity a
//! keyboard adapter must echo back.  It never opens SQLite, deletes rows, or
//! creates executable authority.

use std::collections::BTreeMap;
use std::fmt;

use repo_com_foundation::TtyMode;
use repo_com_purge::{
    PurgeCounts as DomainPurgeCounts, PurgeExecution as DomainPurgeExecution, PurgePlan, PurgeScope,
};
use serde::{Deserialize, Serialize};

/// Local provenance for a purge presentation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PurgeProvenance {
    /// A non-mutating local plan.
    LocalPlan,
    /// A result returned by the local purge domain.
    LocalExecution,
    /// A local error or blocked domain result.
    LocalError,
}

impl PurgeProvenance {
    /// Returns the stable text label used in human output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalPlan => "local plan",
            Self::LocalExecution => "local execution",
            Self::LocalError => "local error",
        }
    }
}

impl fmt::Display for PurgeProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Exact count-only purge counts safe for a terminal preview.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeCountsView {
    /// Content rows selected for irreversible removal or replacement.
    pub content_rows: u64,
    /// Metadata rows selected for removal or clearing.
    pub metadata_rows: u64,
    /// Total selected rows.
    pub total_rows: u64,
    /// Per-table counts in deterministic table-name order.
    pub table_rows: BTreeMap<String, u64>,
}

impl PurgeCountsView {
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
}

impl From<&DomainPurgeCounts> for PurgeCountsView {
    fn from(counts: &DomainPurgeCounts) -> Self {
        Self {
            content_rows: counts.content_rows,
            metadata_rows: counts.metadata_rows,
            total_rows: counts.total_rows,
            table_rows: counts.table_rows.clone(),
        }
    }
}

/// A complete non-mutating purge plan projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgePlanView {
    /// Plan schema version.
    pub schema_version: u8,
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact requested scope.
    pub scope: PurgeScope,
    /// Exact cutoff Unix seconds.
    pub cutoff_unix_seconds: u64,
    /// Exact canonical cutoff text.
    pub cutoff_utc: String,
    /// Current local configuration hash.
    pub config_hash: String,
    /// Complete count-only preview.
    pub counts: PurgeCountsView,
    /// Current local state fingerprint.
    pub state_fingerprint: String,
    /// Exact plan hash to confirm.
    pub plan_hash: String,
    /// Explicitly states that this view is planning, not execution.
    pub execution_performed: bool,
    /// Provenance label.
    pub provenance: PurgeProvenance,
}

impl PurgePlanView {
    /// Projects a domain plan without executing it.
    #[must_use]
    pub fn from_plan(plan: &PurgePlan) -> Self {
        Self {
            schema_version: plan.schema_version,
            repository_id: plan.repository_id.clone(),
            scope: plan.scope,
            cutoff_unix_seconds: plan.cutoff.unix_seconds,
            cutoff_utc: plan.cutoff.utc.clone(),
            config_hash: plan.config_hash.clone(),
            counts: PurgeCountsView::from(&plan.counts),
            state_fingerprint: plan.state_fingerprint.clone(),
            plan_hash: plan.plan_hash.clone(),
            execution_performed: false,
            provenance: PurgeProvenance::LocalPlan,
        }
    }

    /// Returns the exact confirmation identity for this plan.
    #[must_use]
    pub fn confirmation_identity(&self) -> PurgeConfirmationIdentity {
        PurgeConfirmationIdentity {
            repository_id: self.repository_id.clone(),
            scope: self.scope,
            cutoff_unix_seconds: self.cutoff_unix_seconds,
            cutoff_utc: self.cutoff_utc.clone(),
            config_hash: self.config_hash.clone(),
            plan_hash: self.plan_hash.clone(),
        }
    }
}

impl From<&PurgePlan> for PurgePlanView {
    fn from(plan: &PurgePlan) -> Self {
        Self::from_plan(plan)
    }
}

/// State of a local purge result shown to an operator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PurgeExecutionState {
    /// The plan was only presented; no rows were changed.
    Planned,
    /// A confirmed plan committed successfully.
    Executed,
    /// A transaction rolled back and no rows changed.
    RolledBack,
    /// State changed and a fresh plan is required.
    ReplanRequired,
    /// A local operation failed before a committed result.
    Failed,
}

impl PurgeExecutionState {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Executed => "executed",
            Self::RolledBack => "rolled-back",
            Self::ReplanRequired => "replan-required",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for PurgeExecutionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A count-only result or blocked result from confirmed local purge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeExecutionView {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact plan hash associated with the result.
    pub plan_hash: String,
    /// Result state.
    pub state: PurgeExecutionState,
    /// Counts committed, when execution succeeded.
    pub counts: Option<PurgeCountsView>,
    /// Count-only local audit event ID, when execution succeeded.
    pub audit_event_id: Option<String>,
    /// Canonical execution timestamp, when execution succeeded.
    pub executed_at: Option<String>,
    /// Stable local error code, when blocked or failed.
    pub error_code: Option<String>,
    /// Redacted local error detail, when blocked or failed.
    pub error_detail: Option<String>,
    /// Explicit operator next action.
    pub next_action: String,
    /// Provenance label.
    pub provenance: PurgeProvenance,
}

impl PurgeExecutionView {
    /// Projects a successful local execution result.
    #[must_use]
    pub fn from_execution(execution: &DomainPurgeExecution) -> Self {
        Self {
            repository_id: execution.repository_id.clone(),
            plan_hash: execution.plan_hash.clone(),
            state: PurgeExecutionState::Executed,
            counts: Some(PurgeCountsView::from(&execution.counts)),
            audit_event_id: Some(execution.audit_event_id.clone()),
            executed_at: Some(execution.executed_at.clone()),
            error_code: None,
            error_detail: None,
            next_action:
                "Inspect the local count-only audit evidence; do not reuse this plan hash."
                    .to_owned(),
            provenance: PurgeProvenance::LocalExecution,
        }
    }

    /// Creates a typed blocked/failed result for a local domain error.
    #[must_use]
    pub fn from_error(
        repository_id: impl Into<String>,
        plan_hash: impl Into<String>,
        code: impl Into<String>,
        detail: impl Into<String>,
        state: PurgeExecutionState,
    ) -> Self {
        let code = code.into();
        let next_action = match state {
            PurgeExecutionState::ReplanRequired => {
                "Generate a fresh purge plan and obtain a new exact TTY confirmation.".to_owned()
            }
            PurgeExecutionState::RolledBack => {
                "No purge rows changed; investigate the local failure before retrying with a new plan."
                    .to_owned()
            }
            _ => "Do not retry this plan blindly; inspect the redacted local error and generate a new plan."
                .to_owned(),
        };
        Self {
            repository_id: repository_id.into(),
            plan_hash: plan_hash.into(),
            state,
            counts: None,
            audit_event_id: None,
            executed_at: None,
            error_code: Some(code),
            error_detail: Some(detail.into()),
            next_action,
            provenance: PurgeProvenance::LocalError,
        }
    }

    /// Creates a planning-only result from a plan.
    #[must_use]
    pub fn planned(plan: &PurgePlanView) -> Self {
        Self {
            repository_id: plan.repository_id.clone(),
            plan_hash: plan.plan_hash.clone(),
            state: PurgeExecutionState::Planned,
            counts: Some(plan.counts.clone()),
            audit_event_id: None,
            executed_at: None,
            error_code: None,
            error_detail: None,
            next_action: "Review the exact plan; confirmation is required before any local purge."
                .to_owned(),
            provenance: PurgeProvenance::LocalPlan,
        }
    }
}

impl From<&DomainPurgeExecution> for PurgeExecutionView {
    fn from(execution: &DomainPurgeExecution) -> Self {
        Self::from_execution(execution)
    }
}

/// Compatibility name for a purge plan projection.
pub type PurgePlanStatusView = PurgePlanView;

/// Compatibility name for a purge execution projection.
pub type PurgeExecutionResultView = PurgeExecutionView;

/// Exact scope and hash binding echoed by a purge prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeConfirmationIdentity {
    /// Repository scope shown in the plan.
    pub repository_id: String,
    /// Purge scope shown in the plan.
    pub scope: PurgeScope,
    /// Cutoff Unix seconds shown in the plan.
    pub cutoff_unix_seconds: u64,
    /// Canonical cutoff text shown in the plan.
    pub cutoff_utc: String,
    /// Configuration hash shown in the plan.
    pub config_hash: String,
    /// Plan hash shown in the plan.
    pub plan_hash: String,
}

impl PurgeConfirmationIdentity {
    /// Projects an exact identity from a domain plan.
    #[must_use]
    pub fn from_plan(plan: &PurgePlan) -> Self {
        Self {
            repository_id: plan.repository_id.clone(),
            scope: plan.scope,
            cutoff_unix_seconds: plan.cutoff.unix_seconds,
            cutoff_utc: plan.cutoff.utc.clone(),
            config_hash: plan.config_hash.clone(),
            plan_hash: plan.plan_hash.clone(),
        }
    }

    /// Returns whether a typed confirmation is bound to this exact identity.
    #[must_use]
    pub fn matches(&self, confirmation: &PurgeConfirmation) -> bool {
        confirmation.repository_id == self.repository_id
            && confirmation.scope == self.scope
            && confirmation.cutoff_unix_seconds == self.cutoff_unix_seconds
            && confirmation.cutoff_utc == self.cutoff_utc
            && confirmation.config_hash == self.config_hash
            && confirmation.plan_hash == self.plan_hash
    }
}

/// Non-authoritative exact confirmation collected by the keyboard adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeConfirmation {
    /// Repository scope echoed by the operator.
    pub repository_id: String,
    /// Purge scope echoed by the operator.
    pub scope: PurgeScope,
    /// Cutoff Unix seconds echoed by the operator.
    pub cutoff_unix_seconds: u64,
    /// Canonical cutoff text echoed by the operator.
    pub cutoff_utc: String,
    /// Configuration hash echoed by the operator.
    pub config_hash: String,
    /// Plan hash echoed by the operator.
    pub plan_hash: String,
    /// Explicit TTY mode collected by the adapter.
    pub tty_mode: TtyMode,
}

impl PurgeConfirmation {
    /// Creates a confirmation intent from an exact identity.
    #[must_use]
    pub fn from_identity(identity: &PurgeConfirmationIdentity, tty_mode: TtyMode) -> Self {
        Self {
            repository_id: identity.repository_id.clone(),
            scope: identity.scope,
            cutoff_unix_seconds: identity.cutoff_unix_seconds,
            cutoff_utc: identity.cutoff_utc.clone(),
            config_hash: identity.config_hash.clone(),
            plan_hash: identity.plan_hash.clone(),
            tty_mode,
        }
    }
}
