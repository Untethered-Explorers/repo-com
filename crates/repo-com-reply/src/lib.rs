#![forbid(unsafe_code)]
#![doc = "Validated, draft-only threaded replies with accepted-delivery linkage for repo-com."]

#[cfg(test)]
#[path = "../tests/reply_contract.rs"]
mod reply_contract;

mod draft;
mod link;
mod target;

use std::{error::Error, fmt};

pub use draft::{CreatedReply, ReplyCommandService, ReplyDraftFactory, ReplyDraftRequest};
pub use link::{ReplyAcceptedLink, ReplyLinkStatus};
pub use target::{
    ReplyTarget, ReplyTargetError, ReplyTargetRequest, ReplyTargetRevalidation,
    ReplyTargetValidator,
};

use repo_com_draft_model::DraftError;
use repo_com_state::StateError;

/// Stable failures from target validation, draft creation, and accepted linkage.
///
/// Variants never retain inbound content, rendered copy, credentials, or raw
/// untrusted error text.
#[derive(Debug)]
pub enum ReplyServiceError {
    /// The stored target or its current authorization failed validation.
    Target(ReplyTargetError),
    /// The normal draft model rejected the reply request.
    Draft(DraftError),
    /// A timestamp was not canonical UTC RFC 3339 or disagreed with injected time.
    InvalidTimestamp,
    /// A delivery projection was not in an accepted state.
    InvalidAcceptedState,
    /// An accepted delivery had no Discord message identifier.
    MissingAcceptedRemoteMessage,
    /// The inbound item had no local reply-draft link.
    ReplyLinkMissing,
    /// The accepted delivery did not match the local reply link or draft.
    ReplyLinkMismatch,
    /// Existing local audit evidence disagreed with the requested link.
    AuditConflict,
    /// Repository-scoped state or transaction persistence failed.
    State(StateError),
    /// Deterministic non-secret metadata serialization failed.
    Serialization(serde_json::Error),
}

impl ReplyServiceError {
    /// Returns a stable machine-readable reason code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Target(error) => error.code(),
            Self::Draft(_) => "draft-rejected",
            Self::InvalidTimestamp => "invalid-timestamp",
            Self::InvalidAcceptedState => "delivery-not-accepted",
            Self::MissingAcceptedRemoteMessage => "accepted-message-id-missing",
            Self::ReplyLinkMissing => "reply-link-missing",
            Self::ReplyLinkMismatch => "reply-link-mismatch",
            Self::AuditConflict => "audit-conflict",
            Self::State(_) | Self::Serialization(_) => "local-state-failed",
        }
    }
}

impl fmt::Display for ReplyServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ReplyServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Target(error) => Some(error),
            Self::Draft(error) => Some(error),
            Self::State(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ReplyTargetError> for ReplyServiceError {
    fn from(error: ReplyTargetError) -> Self {
        Self::Target(error)
    }
}

impl From<DraftError> for ReplyServiceError {
    fn from(error: DraftError) -> Self {
        Self::Draft(error)
    }
}

impl From<StateError> for ReplyServiceError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<serde_json::Error> for ReplyServiceError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}
