#![forbid(unsafe_code)]
#![doc = "Bounded safe retry and read-only reconciliation for repo-com Discord delivery."]

#[cfg(test)]
#[path = "../tests/delivery_retry_contract.rs"]
mod delivery_retry_contract;

mod policy;
mod reconcile;

pub use policy::{
    Clock, ClockError, DEFAULT_MAX_JITTER, DEFAULT_MIN_JITTER, DeliveryRecovery,
    MAX_DISCORD_DIRECTED_WAIT, MAX_TRANSPORT_ATTEMPTS, ManualClock, OneAttemptTransport,
    RetryAttemptRecord, RetryDecision, RetryDelayKind, RetryPolicy, RetryPolicyError,
    RetryRunError, RetryRunResult, RetryRunner, RetryStopReason, SystemClock, TransportOutcome,
    UnknownReason, UnknownRecoveryEvidence, record_delivery_decision,
};
pub use reconcile::{
    ABSENCE_OBSERVATION_WINDOW, MIN_SUCCESSFUL_READS, ObservedMessage, RECONCILIATION_API_VERSION,
    ReadError, ReadObservation, ReadOnlyMessageReader, ReadPage, Reconciler,
    ReconciliationDecision, ReconciliationError, ReconciliationProgress, ReconciliationReader,
    ReconciliationReason, ReconciliationRequest, RecoveryReconciler, RecoveryTarget, RemoteMessage,
    RemoteMessageState, classify_observation, is_exact_match,
};
pub use repo_com_discord_message::{RateLimitInfo, SendCertainty};

/// Compatibility name for the complete recovery facade.
pub type Recovery = DeliveryRecovery<ManualClock>;

/// Compatibility name for a retry policy.
pub type BoundedRetryPolicy = RetryPolicy;
/// Compatibility name for one transport outcome.
pub type AttemptOutcome = TransportOutcome;
/// Compatibility name for a policy decision.
pub type RetryOutcome = RetryDecision;
/// Compatibility name for a reconciliation result.
pub type ReconciliationOutcome = ReconciliationDecision;
