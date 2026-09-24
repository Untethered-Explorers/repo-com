use std::{error::Error, fmt, time::Duration};

use repo_com_foundation::{ErrorCategory, RepoComError};
use reqwest::{
    Response, StatusCode, Url,
    header::{AUTHORIZATION, HeaderValue},
    redirect::Policy,
};
use serde::de::DeserializeOwned;

use crate::{
    auth::{AuthError, BOT_TOKEN_ENV, BotToken},
    setup::Remediation,
};

/// The sole Discord REST API version used by this crate.
pub const DISCORD_API_VERSION: &str = "v10";
/// The official Discord REST origin.
pub const DISCORD_BASE_URL: &str = "https://discord.com";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "DiscordBot (https://github.com/Untethered-Explorers/repo-com, 0.1.0)";

/// A safe description of one read operation. It never contains a URL, raw
/// response, credential, or remote content.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestOperation {
    /// Read the authenticated bot identity.
    CurrentBotIdentity,
    /// Read the bot's membership in the configured workspace.
    GuildMembership,
    /// Read workspace roles and metadata.
    Guild,
    /// Read one configured channel.
    Channel,
    /// Read one resolved user mention target.
    GuildMember,
}

impl fmt::Display for RequestOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CurrentBotIdentity => "current bot identity",
            Self::GuildMembership => "workspace membership",
            Self::Guild => "workspace metadata",
            Self::Channel => "configured channel",
            Self::GuildMember => "mention target membership",
        })
    }
}

/// A Discord rate-limit scope reported by response headers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitScope {
    /// A route- or global-scoped limit.
    Route,
    /// A limit shared by multiple scopes.
    Shared,
    /// A user-scoped limit.
    User,
    /// A scope Discord did not identify with a known value.
    Unknown,
}

/// Dynamic rate-limit information. This setup client reports the wait to its
/// caller and never sleeps or retries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimitInfo {
    /// Server-provided `Retry-After`, when it was a valid integer duration.
    pub retry_after: Option<Duration>,
    /// Whether Discord marked the limit global.
    pub global: bool,
    /// Scope reported by `X-RateLimit-Scope`.
    pub scope: RateLimitScope,
}

/// A typed, redacted Discord client failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError {
    /// The required environment variable is absent.
    MissingBotToken,
    /// The environment value is not a raw three-part Discord bot token.
    InvalidBotToken,
    /// A test endpoint was not an origin-only loopback HTTP URL.
    InvalidTestBaseUrl,
    /// The bounded HTTP transport could not be constructed.
    ClientBuild,
    /// Discord returned HTTP 401.
    AuthenticationFailed,
    /// Discord denied a required read.
    PermissionDenied { operation: RequestOperation },
    /// Discord returned HTTP 404 for a required resource.
    NotFound { operation: RequestOperation },
    /// Discord returned an unexpected non-success status.
    UnexpectedStatus {
        operation: RequestOperation,
        status: u16,
    },
    /// Discord returned HTTP 429 and supplied dynamic limit metadata.
    RateLimited {
        operation: RequestOperation,
        info: RateLimitInfo,
    },
    /// Discord returned a 5xx response.
    Server {
        operation: RequestOperation,
        status: u16,
    },
    /// A successful response did not match the minimal typed wire contract.
    InvalidResponse { operation: RequestOperation },
    /// The request failed before a response was received.
    Transport { operation: RequestOperation },
}

impl ClientError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingBotToken => "missing-bot-token",
            Self::InvalidBotToken => "invalid-bot-token",
            Self::InvalidTestBaseUrl => "invalid-test-base-url",
            Self::ClientBuild => "client-build-failed",
            Self::AuthenticationFailed => "authentication-failed",
            Self::PermissionDenied { .. } => "permission-denied",
            Self::NotFound { .. } => "remote-not-found",
            Self::UnexpectedStatus { .. } => "unexpected-status",
            Self::RateLimited { .. } => "rate-limited",
            Self::Server { .. } => "server-error",
            Self::InvalidResponse { .. } => "invalid-response",
            Self::Transport { .. } => "transport-error",
        }
    }

    /// Returns the stable provider-neutral error category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::MissingBotToken | Self::InvalidBotToken | Self::InvalidTestBaseUrl => {
                ErrorCategory::UsageOrSchema
            }
            Self::AuthenticationFailed => ErrorCategory::Authentication,
            Self::PermissionDenied { .. } => ErrorCategory::Permission,
            Self::RateLimited { .. } | Self::Transport { .. } => {
                ErrorCategory::ConnectivityRateLimit
            }
            Self::ClientBuild
            | Self::NotFound { .. }
            | Self::UnexpectedStatus { .. }
            | Self::Server { .. }
            | Self::InvalidResponse { .. } => ErrorCategory::InternalFailure,
        }
    }

    /// Returns structured operator remediation without performing a mutation.
    #[must_use]
    pub fn remediation(&self) -> Remediation {
        match self {
            Self::MissingBotToken => Remediation::set_bot_token(),
            Self::InvalidBotToken => Remediation::use_raw_dedicated_bot_token(),
            Self::InvalidTestBaseUrl => Remediation::use_official_endpoint(),
            Self::AuthenticationFailed => Remediation::rotate_bot_token(),
            Self::PermissionDenied { operation } => {
                Remediation::review_remote_read_access(*operation)
            }
            Self::NotFound { operation } => Remediation::correct_configured_resource(*operation),
            Self::UnexpectedStatus { operation, .. } | Self::Server { operation, .. } => {
                Remediation::retry_setup(*operation)
            }
            Self::RateLimited { info, .. } => {
                Remediation::retry_after_rate_limit(info.retry_after.unwrap_or_default())
            }
            Self::ClientBuild => Remediation::retry_setup(RequestOperation::CurrentBotIdentity),
            Self::InvalidResponse { operation } => {
                Remediation::review_api_compatibility(*operation)
            }
            Self::Transport { operation } => Remediation::retry_setup(*operation),
        }
    }

    /// Converts this error to a redacted foundation error value.
    #[must_use]
    pub fn to_repo_com_error(&self) -> RepoComError {
        RepoComError::new(self.category(), self.to_string())
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBotToken => {
                formatter.write_str("the dedicated Discord bot token is missing")
            }
            Self::InvalidBotToken => {
                formatter.write_str("the Discord credential is not a raw dedicated bot token")
            }
            Self::InvalidTestBaseUrl => {
                formatter.write_str("the test Discord endpoint must be an origin-only loopback URL")
            }
            Self::ClientBuild => {
                formatter.write_str("the bounded Discord HTTP client could not be built")
            }
            Self::AuthenticationFailed => {
                formatter.write_str("Discord rejected the dedicated bot credentials")
            }
            Self::PermissionDenied { operation } => {
                write!(formatter, "Discord denied the read for {operation}")
            }
            Self::NotFound { operation } => {
                write!(
                    formatter,
                    "Discord did not find the resource for {operation}"
                )
            }
            Self::UnexpectedStatus { operation, status } => {
                write!(
                    formatter,
                    "Discord returned an unexpected status for {operation}: {status}"
                )
            }
            Self::RateLimited { operation, .. } => {
                write!(formatter, "Discord rate-limited the read for {operation}")
            }
            Self::Server { operation, status } => {
                write!(
                    formatter,
                    "Discord returned a server error for {operation}: {status}"
                )
            }
            Self::InvalidResponse { operation } => {
                write!(
                    formatter,
                    "Discord returned an invalid response for {operation}"
                )
            }
            Self::Transport { operation } => {
                write!(
                    formatter,
                    "the Discord read for {operation} failed before a response"
                )
            }
        }
    }
}

impl Error for ClientError {}

/// The read-only Discord REST client. It owns one zeroizing bot token and
/// exposes no mutation operation.
pub struct DiscordClient {
    http: reqwest::Client,
    base_url: Url,
    token: BotToken,
    loopback_test_server: bool,
}

impl DiscordClient {
    /// Constructs the production client. The only credential source is
    /// `REPO_COM_DISCORD_TOKEN` and the only base URL is Discord's official
    /// origin.
    pub fn from_environment() -> Result<Self, ClientError> {
        Self::from_token_and_base(BotToken::from_environment()?, DISCORD_BASE_URL, false)
    }

    /// Constructs a client for a synthetic loopback WireMock server while still
    /// reading the credential only from `REPO_COM_DISCORD_TOKEN`.
    ///
    /// This deliberately rejects non-loopback hosts, paths, queries, fragments,
    /// credentials, and TLS URLs so a test cannot redirect the production token
    /// to an arbitrary service or API version.
    #[doc(hidden)]
    pub fn from_environment_for_test_server(base_url: &str) -> Result<Self, ClientError> {
        Self::from_token_and_base(BotToken::from_environment()?, base_url, true)
    }

    /// Returns the fixed REST API version used by every request.
    #[must_use]
    pub const fn api_version(&self) -> &'static str {
        DISCORD_API_VERSION
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        operation: RequestOperation,
    ) -> Result<GetOutcome<T>, ClientError> {
        let url = self.v10_url(segments)?;
        let authorization = self.token.authorization_value();
        let header = HeaderValue::from_str(authorization.as_str())
            .map_err(|_| ClientError::InvalidBotToken)?;
        let response = self
            .http
            .get(url)
            .header(AUTHORIZATION, header)
            .send()
            .await
            .map_err(|_| ClientError::Transport { operation })?;
        let status = response.status();

        match status {
            StatusCode::UNAUTHORIZED => Err(ClientError::AuthenticationFailed),
            StatusCode::FORBIDDEN => Ok(GetOutcome::Forbidden),
            StatusCode::NOT_FOUND => Ok(GetOutcome::NotFound),
            StatusCode::TOO_MANY_REQUESTS => Err(ClientError::RateLimited {
                operation,
                info: rate_limit_info(&response),
            }),
            _ if status.is_server_error() => Err(ClientError::Server {
                operation,
                status: status.as_u16(),
            }),
            _ if status.is_success() => response
                .json::<T>()
                .await
                .map(GetOutcome::Found)
                .map_err(|_| ClientError::InvalidResponse { operation }),
            _ => Err(ClientError::UnexpectedStatus {
                operation,
                status: status.as_u16(),
            }),
        }
    }

    fn from_token_and_base(
        token: BotToken,
        base_url: &str,
        loopback_test_server: bool,
    ) -> Result<Self, ClientError> {
        let base_url = parse_base_url(base_url, loopback_test_server)?;
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if loopback_test_server {
            builder = builder.no_proxy();
        }
        let http = builder.build().map_err(|_| ClientError::ClientBuild)?;
        Ok(Self {
            http,
            base_url,
            token,
            loopback_test_server,
        })
    }

    fn v10_url(&self, segments: &[&str]) -> Result<Url, ClientError> {
        if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
            return Err(ClientError::InvalidResponse {
                operation: RequestOperation::CurrentBotIdentity,
            });
        }
        let mut url = self.base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| ClientError::InvalidResponse {
                    operation: RequestOperation::CurrentBotIdentity,
                })?;
            path.clear().push("api").push(DISCORD_API_VERSION);
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }
}

impl fmt::Debug for DiscordClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let endpoint = if self.loopback_test_server {
            "loopback-test"
        } else {
            "official"
        };
        formatter
            .debug_struct("DiscordClient")
            .field("api_version", &DISCORD_API_VERSION)
            .field("transport", &"reqwest-rustls")
            .field("endpoint", &endpoint)
            .finish()
    }
}

pub(crate) enum GetOutcome<T> {
    Found(T),
    NotFound,
    Forbidden,
}

fn parse_base_url(raw: &str, loopback_test_server: bool) -> Result<Url, ClientError> {
    let url = Url::parse(raw).map_err(|_| ClientError::InvalidTestBaseUrl)?;
    let origin_only = matches!(url.path(), "" | "/")
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !origin_only {
        return Err(ClientError::InvalidTestBaseUrl);
    }

    if !loopback_test_server {
        let official = url.scheme() == "https"
            && url.host_str() == Some("discord.com")
            && matches!(url.port(), None | Some(443));
        return if official {
            Ok(url)
        } else {
            Err(ClientError::InvalidTestBaseUrl)
        };
    }

    let loopback = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
        && url.port().is_some();
    if loopback {
        Ok(url)
    } else {
        Err(ClientError::InvalidTestBaseUrl)
    }
}

fn rate_limit_info(response: &Response) -> RateLimitInfo {
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs);
    let global = response
        .headers()
        .get("x-ratelimit-global")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let scope = match response
        .headers()
        .get("x-ratelimit-scope")
        .and_then(|value| value.to_str().ok())
    {
        Some("global") | Some("route") => RateLimitScope::Route,
        Some("shared") => RateLimitScope::Shared,
        Some("user") => RateLimitScope::User,
        _ => RateLimitScope::Unknown,
    };
    RateLimitInfo {
        retry_after,
        global,
        scope,
    }
}

impl From<AuthError> for ClientError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Missing => Self::MissingBotToken,
            AuthError::InvalidFormat => Self::InvalidBotToken,
        }
    }
}

#[must_use]
pub const fn token_environment_variable() -> &'static str {
    BOT_TOKEN_ENV
}

#[cfg(test)]
mod tests {
    use super::parse_base_url;

    #[test]
    fn base_url_accepts_only_an_official_origin_or_loopback_test_origin() {
        assert!(parse_base_url("https://discord.com", false).is_ok());
        assert!(parse_base_url("http://127.0.0.1:1234", true).is_ok());
        assert!(parse_base_url("http://[::1]:1234", true).is_ok());
        assert!(parse_base_url("https://discord.com/api/v9", false).is_err());
        assert!(parse_base_url("https://example.com", true).is_err());
        assert!(parse_base_url("http://127.0.0.1:1234/base", true).is_err());
    }
}
