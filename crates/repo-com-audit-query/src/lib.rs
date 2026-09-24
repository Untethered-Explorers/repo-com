#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;

pub mod filter;
pub mod query;

#[cfg(test)]
#[path = "../tests/audit_query_contract.rs"]
mod audit_query_contract;

/// A typed, non-sensitive local audit-query failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditQueryError {
    /// A filter field or page bound was invalid.
    InvalidFilter {
        /// The invalid field name, never its value.
        field: &'static str,
    },
    /// A continuation cursor was malformed or belonged to another repository.
    InvalidCursor,
    /// A stored row could not be represented safely and was not returned.
    UnsafeStoredEvidence,
    /// The local SQLite evidence store could not be read.
    Storage,
}

impl AuditQueryError {
    /// Returns a stable, non-sensitive error category.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidFilter { .. } | Self::InvalidCursor => "audit-query-usage",
            Self::UnsafeStoredEvidence => "audit-redaction",
            Self::Storage => "storage-integrity",
        }
    }
}

impl fmt::Display for AuditQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFilter { field } => {
                write!(formatter, "audit query filter is invalid: {field}")
            }
            Self::InvalidCursor => formatter.write_str("audit query cursor is invalid"),
            Self::UnsafeStoredEvidence => {
                formatter.write_str("stored audit evidence could not be represented safely")
            }
            Self::Storage => formatter.write_str("local audit evidence could not be read"),
        }
    }
}

impl Error for AuditQueryError {}

impl From<rusqlite::Error> for AuditQueryError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

pub use filter::{AuditCursor, AuditFilter, ContinuationToken, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE};
pub use query::{AuditPage, AuditQuery, AuditQueryEvent, LocalAuditEvidence};
