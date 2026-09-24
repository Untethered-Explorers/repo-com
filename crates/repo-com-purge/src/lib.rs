//! Hashed, explicitly confirmed, repository-scoped local purge for repo-com.
//!
//! Planning is read-only. Execution is local-only, requires an explicit TTY
//! confirmation, revalidates the exact plan in an immediate transaction, and
//! commits deletion together with one redacted count-only audit event.

#![forbid(unsafe_code)]

pub mod execute;
pub mod plan;

#[cfg(test)]
#[path = "../tests/purge_contract.rs"]
mod purge_contract;

pub use execute::{
    FailurePoint, MAX_PURGE_BATCH_SIZE, PurgeConfirmation, PurgeExecuteOptions, PurgeExecution,
    PurgeExecutor, PurgeFailurePoint, execute_purge,
};
pub use plan::{
    HASH_ALGORITHM, HASH_HEX_LENGTH, PURGE_PLAN_SCHEMA_VERSION, PURGED_CONTENT_MARKER, PurgeCounts,
    PurgeCutoff, PurgeError, PurgePlan, PurgePlanner, PurgeRequest, PurgeResult, PurgeScope,
    build_plan, plan_purge,
};

/// Compatibility name for the planner input type.
pub type PurgePlanRequest = PurgeRequest;
/// Compatibility name for the committed execution result.
pub type PurgeExecutionResult = PurgeExecution;
