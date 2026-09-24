use std::{error::Error, fmt};

use repo_com_foundation::{ErrorCategory, RepoComError};

use crate::{BOT_TOKEN_ENV, rate_limit::RateLimitInfo, request::RequestError};

/// How safely a failed operation can be interpreted by delivery recovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendCertainty {
    /// The request was proven not to leave the local process.
    ProvenNotSent,
    /// Discord returned a definitive non-creation response.
    ProvenNotCreated,
    /// The request may have created a message and must not be retried automatically.
    Ambiguous,
}

/// A redacted transport failure proven to occur before dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreDispatchFailure {
    /// DNS, socket, or connect-phase failure.
    ConnectFailed,
    /// The bounded connect phase expired.
    ConnectTimeout,
}

/// A redacted reason a post-dispatch outcome cannot be proven.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmbiguousReason {
    /// The request or response wait expired after dispatch became possible.
    Timeout,
    /// The connection ended before a complete response was received.
    ConnectionInterrupted,
    /// A successful HTTP response did not contain the required message identity.
    InvalidResponse,
    /// The response body ended or could not be decoded after a success status.
    ResponseReadFailed,
}

/// A stable, redacted failure from one Discord message operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageError {
    /// The dedicated bot token is absent from the required environment variable.
    MissingBotToken,
    /// The environment value is not a raw three-part bot token.
    InvalidBotToken,
    /// A test endpoint was not an approved origin-only endpoint.
    InvalidEndpoint,
    /// The bounded HTTP client could not be constructed.
    ClientBuild,
    /// Local request validation failed before serialization or transport.
    InvalidRequest(RequestError),
    /// Discord rejected the request as invalid.
    ValidationRejected,
    /// Discord rejected the dedicated bot credentials.
    Authentication,
    /// The bot lacks permission to create the message.
    PermissionDenied,
    /// The configured channel or referenced message was not found.
    NotFound,
    /// Discord reported a conflicting remote state.
    Conflict,
    /// Discord returned a dynamic 429 response.
    RateLimited { info: RateLimitInfo },
    /// Discord returned a 5xx response after dispatch became possible.
    Server { status: u16 },
    /// A non-success status outside the focused known set was returned.
    UnexpectedResponse { status: u16 },
    /// Transport failed before the request could be dispatched.
    PreDispatch { failure: PreDispatchFailure },
    /// Dispatch became possible but the result is unknown.
    Ambiguous { reason: AmbiguousReason },
}

impl MessageError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingBotToken => "missing-bot-token",
            Self::InvalidBotToken => "invalid-bot-token",
            Self::InvalidEndpoint => "invalid-discord-endpoint",
            Self::ClientBuild => "client-build-failed",
            Self::InvalidRequest(_) => "invalid-message-request",
            Self::ValidationRejected => "remote-validation-rejected",
            Self::Authentication => "discord-authentication-failed",
            Self::PermissionDenied => "discord-permission-denied",
            Self::NotFound => "discord-resource-not-found",
            Self::Conflict => "discord-conflict",
            Self::RateLimited { .. } => "discord-rate-limited",
            Self::Server { .. } => "discord-server-error",
            Self::UnexpectedResponse { .. } => "discord-unexpected-response",
            Self::PreDispatch { .. } => "discord-pre-dispatch-failure",
            Self::Ambiguous { .. } => "discord-delivery-ambiguous",
        }
    }

    /// Returns the stable provider-neutral error category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::MissingBotToken
            | Self::InvalidBotToken
            | Self::InvalidEndpoint
            | Self::InvalidRequest(_)
            | Self::ValidationRejected
            | Self::NotFound => ErrorCategory::UsageOrSchema,
            Self::Authentication => ErrorCategory::Authentication,
            Self::PermissionDenied => ErrorCategory::Permission,
            Self::Conflict => ErrorCategory::RemoteConflict,
            Self::RateLimited { .. } | Self::PreDispatch { .. } => {
                ErrorCategory::ConnectivityRateLimit
            }
            Self::Server { .. } | Self::Ambiguous { .. } => ErrorCategory::UnknownDelivery,
            Self::ClientBuild | Self::UnexpectedResponse { .. } => ErrorCategory::InternalFailure,
        }
    }

    /// Returns what delivery recovery may safely infer from this failure.
    #[must_use]
    pub const fn certainty(&self) -> SendCertainty {
        match self {
            Self::MissingBotToken
            | Self::InvalidBotToken
            | Self::InvalidEndpoint
            | Self::ClientBuild
            | Self::InvalidRequest(_)
            | Self::PreDispatch { .. } => SendCertainty::ProvenNotSent,
            Self::ValidationRejected
            | Self::Authentication
            | Self::PermissionDenied
            | Self::NotFound
            | Self::Conflict
            | Self::RateLimited { .. }
            | Self::UnexpectedResponse { .. } => SendCertainty::ProvenNotCreated,
            Self::Server { .. } | Self::Ambiguous { .. } => SendCertainty::Ambiguous,
        }
    }

    /// Returns whether this failure blocks automatic resend as ambiguous.
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        matches!(self.certainty(), SendCertainty::Ambiguous)
    }

    /// Returns whether the request was proven not to leave the local process.
    #[must_use]
    pub const fn was_proven_not_sent(&self) -> bool {
        matches!(self.certainty(), SendCertainty::ProvenNotSent)
    }

    /// Converts this value to a redacted foundation error.
    #[must_use]
    pub fn to_repo_com_error(&self) -> RepoComError {
        RepoComError::new(self.category(), self.to_string())
    }
}

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBotToken => {
                formatter.write_str("the dedicated Discord bot token is missing")
            }
            Self::InvalidBotToken => {
                formatter.write_str("the Discord credential is not a raw dedicated bot token")
            }
            Self::InvalidEndpoint => {
                formatter.write_str("the Discord endpoint must be an approved origin only")
            }
            Self::ClientBuild => {
                formatter.write_str("the bounded Discord message client could not be built")
            }
            Self::InvalidRequest(error) => write!(formatter, "message request rejected: {error}"),
            Self::ValidationRejected => {
                formatter.write_str("Discord rejected the message request as invalid")
            }
            Self::Authentication => write!(
                formatter,
                "Discord rejected the dedicated bot credentials; rotate {BOT_TOKEN_ENV}"
            ),
            Self::PermissionDenied => {
                formatter.write_str("Discord denied permission to create the message")
            }
            Self::NotFound => {
                formatter.write_str("Discord did not find the configured message destination")
            }
            Self::Conflict => formatter.write_str("Discord reported a message conflict"),
            Self::RateLimited { .. } => {
                formatter.write_str("Discord rate-limited the message operation")
            }
            Self::Server { status } => {
                write!(formatter, "Discord returned a server error: {status}")
            }
            Self::UnexpectedResponse { status } => {
                write!(
                    formatter,
                    "Discord returned an unexpected response: {status}"
                )
            }
            Self::PreDispatch { failure } => match failure {
                PreDispatchFailure::ConnectFailed => {
                    formatter.write_str("the Discord message connection failed before dispatch")
                }
                PreDispatchFailure::ConnectTimeout => {
                    formatter.write_str("the Discord message connection timed out before dispatch")
                }
            },
            Self::Ambiguous { reason } => match reason {
                AmbiguousReason::Timeout => {
                    formatter.write_str("the Discord message outcome is ambiguous after a timeout")
                }
                AmbiguousReason::ConnectionInterrupted => formatter
                    .write_str("the Discord message outcome is ambiguous after a connection loss"),
                AmbiguousReason::InvalidResponse => formatter.write_str(
                    "Discord accepted an HTTP response without a valid message identity",
                ),
                AmbiguousReason::ResponseReadFailed => formatter
                    .write_str("the Discord message outcome is ambiguous after a response failure"),
            },
        }
    }
}

impl Error for MessageError {}

impl From<RequestError> for MessageError {
    fn from(error: RequestError) -> Self {
        Self::InvalidRequest(error)
    }
}
