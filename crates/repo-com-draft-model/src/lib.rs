#![forbid(unsafe_code)]
#![doc = "Immutable, deterministic outbound draft data and side-effect-free preview facts."]

#[cfg(test)]
#[path = "../tests/draft_model_contract.rs"]
mod draft_model_contract;

pub mod canonical;
pub mod model;
pub mod preview;

pub use canonical::{HASH_ALGORITHM, HASH_HEX_LENGTH};
pub use model::{
    AuthorizedReplyReference, DEFAULT_EXPIRY_SECONDS, DestinationAlias, Draft, DraftBody,
    DraftError, DraftExpiry, DraftId, DraftMetadata, DraftModel, DraftRequest, DraftRevision,
    EventType, MAX_DRAFT_BODY_BYTES, MAX_EXPIRY_SECONDS, MAX_METADATA_VALUE_BYTES, Severity,
};
pub use preview::{
    BasisAuthority, BasisFact, BasisStatus, DecisionBases, DraftPreview, ExactTextSource,
    SendBlocker, SendDecision, UnresolvedBasisReason,
};
