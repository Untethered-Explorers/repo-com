//! Bounded, repository-scoped, read-only lifecycle inspection and state verification.
//!
//! This crate deliberately exposes projections and verification reports only. It
//! does not open a database with write/create flags, apply migrations, repair
//! SQLite state, upload a backup, or provide a Discord transport. Inbound text
//! remains untrusted data, and an accepted delivery is never treated as a read
//! receipt or a reply.

#![forbid(unsafe_code)]

pub mod inspect;
pub mod verify;

#[cfg(test)]
#[path = "../tests/lifecycle_contract.rs"]
mod lifecycle_contract;

use std::error::Error;
use std::fmt;

pub use inspect::{
    ACKNOWLEDGEMENT_OBJECT, ARCHIVE_OBJECT, AUDIT_OBJECT, AuditTransitionsProjection,
    DEFAULT_PAGE_SIZE, DELIVERY_ATTEMPT_OBJECT, DRAFT_OBJECT, DRAFT_REVISION_OBJECT,
    EvidenceSource, INBOUND_ITEM_OBJECT, InboundItemProjection, InboundSnapshotProjection,
    InspectionRequest, LastRecordedDeliveryEvidence, LifecycleInspector, LifecycleObject,
    LifecyclePage, LifecycleProjection, LocalInboundMarkers, MAX_PAGE_SIZE, PageRequest,
    REPLY_LINK_OBJECT, REPOSITORY_OBJECT, ReadReceiptStatus, RemoteSnapshotProjection,
    RemoteSnapshotState, ReplyClaimStatus, ReplyLinkProjection, RepositoryProjection,
    RetainedContent,
};
pub use verify::{
    CheckStatus, EXPECTED_MIGRATION_VERSION, ForeignKeyVerification, MigrationVerification,
    PermissionVerification, QuickCheckVerification, RepositoryScopeVerification,
    StateVerificationReport, StateVerifier, V1_STORAGE_RESIDUAL_RISK, VerificationIssue,
    VerificationRequest, verify_database, verify_state,
};

/// Result type used by lifecycle inspection.
pub type LifecycleResult<T> = Result<T, LifecycleError>;

/// A safe, typed lifecycle-inspection failure.
///
/// Values that could contain message content or credentials are deliberately
/// not retained in this error type.
#[derive(Clone, Eq, PartialEq)]
pub enum LifecycleError {
    /// The repository identifier was empty, too long, or unsafe.
    InvalidRepository,
    /// The requested repository is not registered in local state.
    RepositoryNotFound {
        /// The validated repository scope.
        repository_id: String,
    },
    /// An object identifier or object selector was invalid.
    InvalidObject {
        /// Stable object family, never an object value.
        object_type: &'static str,
    },
    /// The requested object is not visible in the requested repository scope.
    ObjectNotFound {
        /// Stable object family.
        object_type: &'static str,
        /// Validated repository scope.
        repository_id: String,
        /// Validated object identifier.
        object_id: String,
    },
    /// A caller attempted to use a continuation or selector from another scope.
    CrossRepositoryDenied {
        /// The validated requested repository scope.
        repository_id: String,
    },
    /// A page bound or continuation was invalid.
    InvalidPage {
        /// Invalid field name, without its value.
        field: &'static str,
    },
    /// Stored evidence was not safe to project.
    UnsafeStoredEvidence,
    /// The local SQLite evidence store could not be read.
    Storage,
}

impl fmt::Debug for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::InvalidRepository => "invalid-repository",
            Self::RepositoryNotFound { .. } => "repository-not-found",
            Self::InvalidObject { .. } => "invalid-object",
            Self::ObjectNotFound { .. } => "object-not-found",
            Self::CrossRepositoryDenied { .. } => "cross-repository-denied",
            Self::InvalidPage { .. } => "invalid-page",
            Self::UnsafeStoredEvidence => "unsafe-stored-evidence",
            Self::Storage => "storage",
        };
        formatter
            .debug_struct("LifecycleError")
            .field("code", &self.code())
            .field("variant", &variant)
            .finish()
    }
}

impl LifecycleError {
    /// Returns a stable, non-sensitive error category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRepository | Self::InvalidObject { .. } | Self::InvalidPage { .. } => {
                "lifecycle-usage"
            }
            Self::RepositoryNotFound { .. }
            | Self::ObjectNotFound { .. }
            | Self::CrossRepositoryDenied { .. } => "lifecycle-scope",
            Self::UnsafeStoredEvidence => "lifecycle-redaction",
            Self::Storage => "storage-integrity",
        }
    }
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepository => formatter.write_str("lifecycle repository scope is invalid"),
            Self::RepositoryNotFound { .. } => {
                formatter.write_str("the requested repository is not present in local state")
            }
            Self::InvalidObject { object_type } => {
                write!(
                    formatter,
                    "lifecycle object selector is invalid: {object_type}"
                )
            }
            Self::ObjectNotFound { object_type, .. } => {
                write!(
                    formatter,
                    "lifecycle object is not present in the requested scope: {object_type}"
                )
            }
            Self::CrossRepositoryDenied { .. } => {
                formatter.write_str("lifecycle continuation or object crosses repository scope")
            }
            Self::InvalidPage { field } => write!(formatter, "lifecycle page is invalid: {field}"),
            Self::UnsafeStoredEvidence => {
                formatter.write_str("stored lifecycle evidence could not be represented safely")
            }
            Self::Storage => formatter.write_str("local lifecycle evidence could not be read"),
        }
    }
}

impl Error for LifecycleError {}

impl From<rusqlite::Error> for LifecycleError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

impl From<repo_com_state::StateError> for LifecycleError {
    fn from(_: repo_com_state::StateError) -> Self {
        Self::Storage
    }
}
