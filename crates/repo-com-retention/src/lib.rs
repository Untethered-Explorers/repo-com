//! Local, transactional automatic retention for repo-com state.

#![forbid(unsafe_code)]

pub mod policy;
pub mod sweep;

#[cfg(test)]
#[path = "../tests/retention_contract.rs"]
mod retention_contract;

pub use policy::{
    CONTENT_EXPIRED_MARKER, Clock, Cutoffs, DEFAULT_CONTENT_DAYS, DEFAULT_CONTENT_RETENTION_DAYS,
    DEFAULT_METADATA_DAYS, DEFAULT_METADATA_RETENTION_DAYS, FixedClock, MAX_CONTENT_DAYS,
    MAX_CONTENT_RETENTION_DAYS, MAX_METADATA_DAYS, MAX_METADATA_RETENTION_DAYS, MIN_CONTENT_DAYS,
    MIN_CONTENT_RETENTION_DAYS, MIN_METADATA_DAYS, MIN_METADATA_RETENTION_DAYS, ManualClock,
    RetentionClock, RetentionContext, RetentionCutoffs, RetentionInstant, RetentionPolicy,
    RetentionPolicyError, SECONDS_PER_DAY, SystemClock, calculate_cutoffs, format_rfc3339_utc,
    parse_canonical_utc, parse_rfc3339_utc,
};
pub use sweep::{
    BlockingStorageIntegrity, FailurePoint, RemovedId, RemovedIds, RetentionError,
    RetentionSweeper, SweepCounts, SweepOptions, SweepPhase, SweepResult, sweep_state,
    sweep_state_at, sweep_state_with_clock,
};

/// Compatibility name for the typed retention boundary error.
pub type RetentionSweepError = RetentionError;
/// Compatibility name for the policy validation error.
pub type PolicyError = RetentionPolicyError;
