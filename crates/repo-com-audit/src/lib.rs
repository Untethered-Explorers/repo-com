#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;

use repo_com_state::StateError;

pub mod event;
pub mod redact;
pub mod writer;

#[cfg(test)]
#[path = "../tests/audit_contract.rs"]
mod audit_contract;

/// The result type used by the audit boundary.
pub type AuditResult<T> = Result<T, AuditError>;

/// A safe, typed audit failure.
///
/// Error values never carry event metadata, message text, credentials, or
/// authorization values. State failures retain only the state layer's stable
/// category code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuditError {
    /// A required event field was empty, malformed, or unsafe to persist.
    InvalidEvent {
        /// The field that failed validation, without its value.
        field: &'static str,
    },
    /// Metadata was not a JSON object or could not be represented safely.
    InvalidMetadata,
    /// The event timestamp was not canonical UTC RFC 3339 text.
    InvalidTimestamp,
    /// A diagnostic setting could not be interpreted safely.
    InvalidDiagnosticConfiguration,
    /// Redaction could not guarantee a safe representation.
    RedactionFailed,
    /// The state store rejected the append or transaction operation.
    State {
        /// Stable state error category only.
        code: &'static str,
    },
    /// JSON serialization failed for a safe, internal representation.
    Serialization,
    /// The explicitly enabled diagnostic stream could not be written.
    DiagnosticOutput,
}

impl AuditError {
    /// Returns a stable, non-sensitive error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidEvent { .. } | Self::InvalidMetadata | Self::InvalidTimestamp => {
                "audit-usage"
            }
            Self::InvalidDiagnosticConfiguration => "diagnostics-usage",
            Self::RedactionFailed => "audit-redaction",
            Self::State { .. } => "storage-integrity",
            Self::Serialization => "audit-serialization",
            Self::DiagnosticOutput => "diagnostics-output",
        }
    }

    /// Returns the safe state category, when this is a state failure.
    #[must_use]
    pub fn state_code(&self) -> Option<&str> {
        match self {
            Self::State { code } => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for AuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEvent { field } => {
                write!(formatter, "audit event field is invalid: {field}")
            }
            Self::InvalidMetadata => formatter.write_str("audit metadata is not safe to store"),
            Self::InvalidTimestamp => {
                formatter.write_str("audit timestamp must be canonical UTC text")
            }
            Self::InvalidDiagnosticConfiguration => {
                formatter.write_str("diagnostic configuration is invalid")
            }
            Self::RedactionFailed => formatter.write_str("audit redaction failed closed"),
            Self::State { code } => write!(formatter, "audit state operation failed: {code}"),
            Self::Serialization => formatter.write_str("audit value could not be serialized"),
            Self::DiagnosticOutput => formatter.write_str("diagnostic output could not be written"),
        }
    }
}

impl Error for AuditError {}

impl From<StateError> for AuditError {
    fn from(error: StateError) -> Self {
        Self::State { code: error.code() }
    }
}

impl From<AuditError> for StateError {
    fn from(error: AuditError) -> Self {
        Self::Transaction {
            message: error.code().to_owned(),
        }
    }
}

impl From<serde_json::Error> for AuditError {
    fn from(_: serde_json::Error) -> Self {
        Self::Serialization
    }
}

impl From<std::io::Error> for AuditError {
    fn from(_: std::io::Error) -> Self {
        Self::DiagnosticOutput
    }
}

pub use event::{AuditEvent, AuditEventEnvelope, AuditEventInput, RedactedAuditEvent};
pub use redact::{
    REDACTED, REDACTED_KEY, is_secret_like_field, redact_json, redact_metadata,
    redact_metadata_checked, redact_text,
};
pub use writer::{
    AUDIT_DIAGNOSTICS_ENV, AuditAppendTarget, AuditWriter, DIAGNOSTICS_ENV, DiagnosticConfig,
    DiagnosticEmitter, DiagnosticLevel, DiagnosticRecord, append_in_transaction,
    append_to_transaction, current_diagnostic_config, diagnostics_enabled, disable_diagnostics,
    emit_diagnostic, init_diagnostics, init_diagnostics_from_env, init_diagnostics_with_flag,
    parse_diagnostic_level, render_diagnostic,
};
