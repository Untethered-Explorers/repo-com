use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::{ErrorCategory, RepoComError};

/// The only protocol version emitted by this foundation.
pub const PROTOCOL_VERSION: u8 = 1;

/// The two valid top-level outcome statuses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeStatus {
    /// The command completed successfully.
    Success,
    /// The command completed with a typed error.
    Error,
}

impl OutcomeStatus {
    /// Returns whether this status represents success.
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns whether this status represents an error.
    #[must_use]
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Error)
    }
}

/// A typed protocol-version-1 command outcome.
///
/// The four serialized fields are always present. Constructors enforce the
/// success/error invariants so operational failures remain protocol objects
/// rather than becoming untyped process output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome<T> {
    #[serde(rename = "protocol_version")]
    protocol_version: u8,
    status: OutcomeStatus,
    data: Option<T>,
    error: Option<RepoComError>,
}

impl<T> CommandOutcome<T> {
    /// Creates a successful outcome carrying data.
    pub fn success(data: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            status: OutcomeStatus::Success,
            data: Some(data),
            error: None,
        }
    }

    /// Creates a successful outcome carrying data.
    pub fn ok(data: T) -> Self {
        Self::success(data)
    }

    /// Creates an error outcome carrying a stable typed error.
    pub fn failure(error: RepoComError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            status: OutcomeStatus::Error,
            data: None,
            error: Some(error),
        }
    }

    /// Creates an error outcome carrying a stable typed error.
    pub fn err(error: RepoComError) -> Self {
        Self::failure(error)
    }

    /// Converts a typed result into a command outcome.
    pub fn from_result(result: Result<T, RepoComError>) -> Self {
        match result {
            Ok(data) => Self::success(data),
            Err(error) => Self::failure(error),
        }
    }

    /// Creates an outcome from protocol parts after validating all invariants.
    pub fn try_new(
        protocol_version: u8,
        status: OutcomeStatus,
        data: Option<T>,
        error: Option<RepoComError>,
    ) -> Result<Self, RepoComError> {
        if protocol_version != PROTOCOL_VERSION {
            return Err(RepoComError::usage(format!(
                "unsupported protocol version {protocol_version}"
            )));
        }

        match status {
            OutcomeStatus::Success if data.is_some() && error.is_none() => {}
            OutcomeStatus::Error if data.is_none() && error.is_some() => {}
            OutcomeStatus::Success => {
                return Err(RepoComError::usage(
                    "a success outcome requires data and no error",
                ));
            }
            OutcomeStatus::Error => {
                return Err(RepoComError::usage(
                    "an error outcome requires an error and no data",
                ));
            }
        }

        Ok(Self {
            protocol_version,
            status,
            data,
            error,
        })
    }

    /// Returns the protocol version carried by this outcome.
    #[must_use]
    pub const fn protocol_version(&self) -> u8 {
        self.protocol_version
    }

    /// Returns the outcome status.
    #[must_use]
    pub const fn status(&self) -> OutcomeStatus {
        self.status
    }

    /// Returns whether the outcome is successful.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Returns whether the outcome represents an error.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.status.is_error()
    }

    /// Returns the successful payload, if present.
    #[must_use]
    pub const fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    /// Returns the typed error, if present.
    #[must_use]
    pub const fn error(&self) -> Option<&RepoComError> {
        self.error.as_ref()
    }

    /// Consumes the outcome and returns its payload, if present.
    #[must_use]
    pub fn into_data(self) -> Option<T> {
        self.data
    }

    /// Consumes the outcome and returns its error, if present.
    #[must_use]
    pub fn into_error(self) -> Option<RepoComError> {
        self.error
    }

    /// Returns the deterministic process exit code for this outcome.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self.status {
            OutcomeStatus::Success => 0,
            OutcomeStatus::Error => match self.error.as_ref() {
                Some(error) => error.exit_code(),
                None => ErrorCategory::InternalFailure.exit_code(),
            },
        }
    }

    /// Validates the protocol version and success/error field invariants.
    pub fn validate(&self) -> Result<(), RepoComError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(RepoComError::usage(format!(
                "unsupported protocol version {}",
                self.protocol_version
            )));
        }

        match self.status {
            OutcomeStatus::Success if self.data.is_some() && self.error.is_none() => Ok(()),
            OutcomeStatus::Error if self.data.is_none() && self.error.is_some() => Ok(()),
            _ => Err(RepoComError::usage(
                "outcome status, data, and error fields are inconsistent",
            )),
        }
    }
}

impl<T> CommandOutcome<T>
where
    T: Serialize,
{
    /// Serializes this outcome as one compact protocol-version-1 JSON object.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Serializes this outcome as a JSON value for callers that need to inspect it.
    pub fn to_json_value(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::to_value(self)
    }

    /// Returns protocol JSON for stdout and optional diagnostics for stderr.
    pub fn output_streams(&self, diagnostics: Option<String>) -> serde_json::Result<OutputStreams> {
        Ok(OutputStreams {
            stdout: self.to_json()?,
            stderr: diagnostics.unwrap_or_default(),
        })
    }

    /// Alias for [`CommandOutcome::output_streams`].
    pub fn machine_output(&self, diagnostics: Option<String>) -> serde_json::Result<OutputStreams> {
        self.output_streams(diagnostics)
    }
}

impl<T> fmt::Display for CommandOutcome<T>
where
    T: Serialize,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_json() {
            Ok(json) => formatter.write_str(&json),
            Err(_) => Err(fmt::Error),
        }
    }
}

/// Separate values for the protocol and diagnostic streams.
///
/// This is data only. The final executable chooses how to write each value;
/// the foundation never writes to stdout or stderr itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputStreams {
    /// Exactly one protocol-version-1 JSON object.
    pub stdout: String,
    /// Optional operator-enabled diagnostics.
    pub stderr: String,
}

impl OutputStreams {
    /// Creates separated stream values.
    pub fn new(stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    /// Returns the protocol stdout value.
    #[must_use]
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Returns the diagnostic stderr value.
    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}

/// Compatibility name for [`OutputStreams`].
pub type ProtocolOutput = OutputStreams;

/// Compatibility name for [`OutputStreams`].
pub type OutputBundle = OutputStreams;

/// Compatibility name for [`OutcomeStatus`].
pub type CommandStatus = OutcomeStatus;
