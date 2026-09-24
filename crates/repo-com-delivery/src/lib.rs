#![forbid(unsafe_code)]
#![doc = "Atomic, duplicate-safe local delivery claims for one exact repo-com draft revision."]

#[cfg(test)]
#[path = "../tests/delivery_contract.rs"]
mod delivery_contract;

mod claim;
mod model;
mod transition;

pub use claim::{DeliveryCoordinator, DeliveryError};
pub use model::{
    ClaimDisposition, ClaimInput, ClaimPermit, ClaimRequest, ClaimResult, DeliveryAttempt,
    DeliveryState, TransitionRequest,
};

/// Compatibility name for the coordinator's typed failure.
pub type ClaimError = DeliveryError;
/// Compatibility name for a claim call's recorded result.
pub type ClaimOutcome = ClaimResult;
/// Compatibility name for the durable state enum.
pub type DeliveryAttemptState = DeliveryState;
/// Compatibility name for transition input.
pub type TransitionInput = TransitionRequest;

/// Stable schema marker for delivery-owned audit metadata.
pub const DELIVERY_METADATA_SCHEMA_VERSION: u32 = 1;
