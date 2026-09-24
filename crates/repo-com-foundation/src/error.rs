use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

/// Stable, provider-neutral error categories used by all command layers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ErrorCategory {
    /// Invalid command usage, input, or schema.
    #[serde(rename = "usage-schema")]
    UsageOrSchema,
    /// A human approval or other operator action is required.
    #[serde(rename = "operator-action-required")]
    OperatorActionRequired,
    /// A policy decision blocks the operation.
    #[serde(rename = "policy-blocked")]
    PolicyBlocked,
    /// The remote identity could not be authenticated.
    #[serde(rename = "authentication")]
    Authentication,
    /// The caller lacks a required remote permission.
    #[serde(rename = "permission")]
    Permission,
    /// A remote resource or state conflicts with the request.
    #[serde(rename = "remote-conflict")]
    RemoteConflict,
    /// A remote operation may have taken effect but is not known.
    #[serde(rename = "unknown-delivery")]
    UnknownDelivery,
    /// Local state failed an integrity or consistency check.
    #[serde(rename = "storage-integrity")]
    StorageIntegrity,
    /// A connectivity or remote rate-limit condition prevented completion.
    #[serde(rename = "connectivity-rate-limit")]
    ConnectivityRateLimit,
    /// An unexpected internal failure occurred.
    #[serde(rename = "internal-failure")]
    InternalFailure,
}

impl ErrorCategory {
    /// Every stable category, in stable category order.
    pub const ALL: [Self; 10] = [
        Self::UsageOrSchema,
        Self::OperatorActionRequired,
        Self::PolicyBlocked,
        Self::Authentication,
        Self::Permission,
        Self::RemoteConflict,
        Self::UnknownDelivery,
        Self::StorageIntegrity,
        Self::ConnectivityRateLimit,
        Self::InternalFailure,
    ];

    /// Returns the stable machine-readable category code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UsageOrSchema => "usage-schema",
            Self::OperatorActionRequired => "operator-action-required",
            Self::PolicyBlocked => "policy-blocked",
            Self::Authentication => "authentication",
            Self::Permission => "permission",
            Self::RemoteConflict => "remote-conflict",
            Self::UnknownDelivery => "unknown-delivery",
            Self::StorageIntegrity => "storage-integrity",
            Self::ConnectivityRateLimit => "connectivity-rate-limit",
            Self::InternalFailure => "internal-failure",
        }
    }

    /// Parses a stable machine-readable category code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.code() == code)
    }

    /// Returns the deterministic process exit code for this category.
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::UsageOrSchema => 2,
            Self::OperatorActionRequired => 3,
            Self::PolicyBlocked => 4,
            Self::Authentication => 5,
            Self::Permission => 6,
            Self::RemoteConflict => 7,
            Self::UnknownDelivery => 8,
            Self::StorageIntegrity => 9,
            Self::ConnectivityRateLimit => 10,
            Self::InternalFailure => 1,
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// A typed command error with a stable category and human-readable detail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepoComError {
    /// Stable process category emitted in protocol JSON.
    pub code: ErrorCategory,
    /// Safe, caller-provided error detail.
    pub message: String,
}

impl RepoComError {
    /// Creates a typed error from a category and message.
    pub fn new(code: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Creates a usage or schema error.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::UsageOrSchema, message)
    }

    /// Creates an operator-action-required error.
    pub fn operator_action_required(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::OperatorActionRequired, message)
    }

    /// Creates a policy-blocked error.
    pub fn policy_blocked(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::PolicyBlocked, message)
    }

    /// Creates an authentication error.
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Authentication, message)
    }

    /// Creates a permission error.
    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Permission, message)
    }

    /// Creates a remote-conflict error.
    pub fn remote_conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::RemoteConflict, message)
    }

    /// Creates an unknown-delivery error.
    pub fn unknown_delivery(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::UnknownDelivery, message)
    }

    /// Creates a storage-integrity error.
    pub fn storage_integrity(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::StorageIntegrity, message)
    }

    /// Creates a connectivity or rate-limit error.
    pub fn connectivity_rate_limit(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::ConnectivityRateLimit, message)
    }

    /// Creates an internal-failure error.
    pub fn internal_failure(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::InternalFailure, message)
    }

    /// Returns the error category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        self.code
    }

    /// Returns the stable protocol code.
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        self.code.code()
    }

    /// Returns the deterministic process exit code.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.code.exit_code()
    }

    /// Splits the error into its stable category and message.
    #[must_use]
    pub fn into_parts(self) -> (ErrorCategory, String) {
        (self.code, self.message)
    }
}

impl fmt::Display for RepoComError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for RepoComError {}

/// Compatibility name for the stable category type.
pub type ProcessCategory = ErrorCategory;

/// Compatibility name for the stable category type.
pub type ErrorCode = ErrorCategory;
