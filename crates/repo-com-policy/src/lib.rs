#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;

use repo_com_foundation::{ErrorCategory, RepoComError};
use repo_com_state::StateError;

pub mod activation;
pub mod evaluate;
pub mod hash;

#[cfg(test)]
#[path = "../tests/policy_contract.rs"]
mod policy_contract;

/// A safe, typed policy failure.
///
/// Variants never carry the complete configuration or message content. Hash
/// and tuple values are returned only in successful typed results so callers
/// can revalidate them without relying on diagnostic strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    /// A requested tuple was empty, wildcard-shaped, or ambiguous.
    InvalidPolicyTuple,
    /// The configuration cannot safely describe a policy destination.
    InvalidConfiguration,
    /// The requested exact tuple is not configured for auto-send.
    PolicyNotConfigured,
    /// Duplicate or multiple matching records could broaden authority.
    AmbiguousPolicy,
    /// An authority-creating activation was attempted without a TTY.
    TtyRequired,
    /// An activation ID was empty, too long, or contained control characters.
    InvalidActivationId,
    /// A requested activation ID already belongs to another binding.
    ActivationIdConflict,
    /// An activation or deactivation timestamp was invalid.
    InvalidTimestamp,
    /// No active row matched a permission-reducing tuple deactivation.
    NoActiveActivation,
    /// The repository-scoped state operation failed.
    State(StateError),
}

impl PolicyError {
    /// Returns the stable foundation category for this policy failure.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::TtyRequired => ErrorCategory::OperatorActionRequired,
            Self::PolicyNotConfigured | Self::AmbiguousPolicy => ErrorCategory::PolicyBlocked,
            Self::State(_) => ErrorCategory::StorageIntegrity,
            Self::InvalidPolicyTuple
            | Self::InvalidConfiguration
            | Self::InvalidActivationId
            | Self::ActivationIdConflict
            | Self::InvalidTimestamp
            | Self::NoActiveActivation => ErrorCategory::UsageOrSchema,
        }
    }

    /// Converts this error to a redacted foundation error envelope value.
    #[must_use]
    pub fn to_repo_com_error(&self) -> RepoComError {
        RepoComError::new(self.category(), self.to_string())
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicyTuple => formatter.write_str("policy tuple is not exact"),
            Self::InvalidConfiguration => formatter.write_str("policy configuration is invalid"),
            Self::PolicyNotConfigured => formatter.write_str("exact policy is not configured"),
            Self::AmbiguousPolicy => {
                formatter.write_str("policy configuration or activation is ambiguous")
            }
            Self::TtyRequired => formatter.write_str("interactive operator action requires a TTY"),
            Self::InvalidActivationId => formatter.write_str("policy activation ID is invalid"),
            Self::ActivationIdConflict => {
                formatter.write_str("policy activation ID is already bound")
            }
            Self::InvalidTimestamp => formatter.write_str("policy activation timestamp is invalid"),
            Self::NoActiveActivation => formatter.write_str("no active policy activation matches"),
            Self::State(error) => write!(formatter, "policy state operation failed: {error}"),
        }
    }
}

impl Error for PolicyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StateError> for PolicyError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

pub use activation::{
    ActivationPreview, ActivationReceipt, OperatorConfirmation, PolicyRegistry, PolicyStateBackend,
    activation_records_from_state, evaluate_from_state, policy_error_to_repo_com_error,
    status_from_state,
};
pub use evaluate::{
    ActivationSnapshot, PolicyDecision, PolicyEvaluation, PolicyMatch, PolicyStatus, StaleReason,
    classify_activation_records, configured_policy_tuples, decision_from_status,
    exact_policy_match, find_exact_policy, match_exact_policy, validate_policy_configuration,
};
pub use hash::{
    HASH_ALGORITHM, HASH_HEX_LENGTH, PolicyHashes, PolicyTuple, PolicyTupleError,
    canonical_config_bytes, canonical_config_hash, canonical_config_hash_bytes,
    canonical_tuple_hash, is_sha256_hex,
};
pub use repo_com_foundation::TtyMode;
pub use repo_com_state::PolicyActivationRecord;
