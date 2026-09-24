#![forbid(unsafe_code)]
#![doc = "One-attempt Discord REST v10 text message operation with typed delivery ambiguity."]

#[cfg(test)]
#[path = "../tests/discord_message_contract.rs"]
mod discord_message_contract;

mod error;
mod rate_limit;
mod request;

use std::{env, fmt, time::Duration};

use repo_com_discord_client::{BOT_TOKEN_ENV, DISCORD_API_VERSION, DISCORD_BASE_URL};
use reqwest::{
    Response, StatusCode, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use zeroize::Zeroizing;

pub use error::{AmbiguousReason, MessageError, PreDispatchFailure, SendCertainty};
pub use rate_limit::{MAX_DISCORD_DIRECTED_WAIT, RateLimitClass, RateLimitInfo, RateLimitScope};
pub use request::{CreateMessageRequest, DiscordApiVersion, RequestError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "DiscordBot (https://github.com/Untethered-Explorers/repo-com, 0.1.0)";
const MAX_RATE_LIMIT_BODY_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct MessageTimeouts {
    pub(crate) connect: Duration,
    pub(crate) total: Duration,
    pub(crate) response: Duration,
}

const DEFAULT_TIMEOUTS: MessageTimeouts = MessageTimeouts {
    connect: CONNECT_TIMEOUT,
    total: TOTAL_TIMEOUT,
    response: RESPONSE_TIMEOUT,
};

#[derive(Clone, Copy)]
enum EndpointKind {
    Official,
    LoopbackTest,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum TransportFailure {
    ConnectTimeout,
    AmbiguousTimeout,
}

struct BotToken(Zeroizing<String>);

impl BotToken {
    fn from_environment() -> Result<Self, MessageError> {
        let value = env::var(BOT_TOKEN_ENV).map_err(|error| match error {
            env::VarError::NotPresent => MessageError::MissingBotToken,
            env::VarError::NotUnicode(_) => MessageError::InvalidBotToken,
        })?;
        let value = Zeroizing::new(value);
        if is_valid_bot_token(&value) {
            Ok(Self(value))
        } else {
            Err(MessageError::InvalidBotToken)
        }
    }

    fn authorization_value(&self) -> Zeroizing<String> {
        let mut value = Zeroizing::new(String::with_capacity(self.0.len() + 4));
        value.push_str("Bot ");
        value.push_str(self.0.as_str());
        value
    }
}

/// A typed successful response from exactly one create-message attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageSendOutcome {
    message_id: String,
    channel_id: String,
    rate_limit: RateLimitInfo,
}

impl MessageSendOutcome {
    /// Returns the Discord-created message snowflake.
    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    /// Returns the resolved destination echoed by Discord.
    #[must_use]
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// Returns dynamic route or global metadata for later proactive throttling.
    #[must_use]
    pub const fn rate_limit(&self) -> &RateLimitInfo {
        &self.rate_limit
    }
}

/// A dedicated-bot client that exposes only one create-message operation.
pub struct DiscordMessageClient {
    http: reqwest::Client,
    base_url: Url,
    token: BotToken,
    endpoint_kind: EndpointKind,
}

impl DiscordMessageClient {
    /// Constructs the production client with Discord's fixed origin and v10.
    pub fn from_environment() -> Result<Self, MessageError> {
        Self::from_parts(
            BotToken::from_environment()?,
            DISCORD_BASE_URL,
            EndpointKind::Official,
            DEFAULT_TIMEOUTS,
        )
    }

    /// Constructs a synthetic loopback client while still reading the token only
    /// from `REPO_COM_DISCORD_TOKEN`.
    #[doc(hidden)]
    pub fn from_environment_for_test_server(base_url: &str) -> Result<Self, MessageError> {
        Self::from_environment_for_test_server_with_timeouts(base_url, DEFAULT_TIMEOUTS)
    }

    pub(crate) fn from_environment_for_test_server_with_timeouts(
        base_url: &str,
        timeouts: MessageTimeouts,
    ) -> Result<Self, MessageError> {
        Self::from_parts(
            BotToken::from_environment()?,
            base_url,
            EndpointKind::LoopbackTest,
            timeouts,
        )
    }

    /// Returns the only API version this client can route.
    #[must_use]
    pub const fn api_version(&self) -> DiscordApiVersion {
        DiscordApiVersion::V10
    }

    /// Performs exactly one HTTP send attempt and never sleeps or retries.
    pub async fn send(
        &self,
        request: CreateMessageRequest,
    ) -> Result<MessageSendOutcome, MessageError> {
        let url = self.create_message_url(&request)?;
        let body = request.body_bytes()?;
        let authorization = self.token.authorization_value();
        let authorization = HeaderValue::from_str(authorization.as_str())
            .map_err(|_| MessageError::InvalidBotToken)?;

        let response = self
            .http
            .post(url)
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .send()
            .await
            .map_err(|error| classify_reqwest_error(&error))?;
        let status = response.status();

        match status {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(MessageError::ValidationRejected)
            }
            StatusCode::UNAUTHORIZED => Err(MessageError::Authentication),
            StatusCode::FORBIDDEN => Err(MessageError::PermissionDenied),
            StatusCode::NOT_FOUND => Err(MessageError::NotFound),
            StatusCode::CONFLICT => Err(MessageError::Conflict),
            StatusCode::TOO_MANY_REQUESTS => {
                let mut info = RateLimitInfo::from_rate_limited(&response);
                if let Some(body) = rate_limit_body(response).await {
                    info.include_body_retry_after(body.retry_after);
                    info.include_body_global(body.global);
                }
                Err(MessageError::RateLimited { info })
            }
            _ if status.is_server_error() => Err(MessageError::Server {
                status: status.as_u16(),
            }),
            _ if status.is_success() => {
                let rate_limit = RateLimitInfo::from_success(&response);
                let created = response
                    .json::<CreatedMessageWire>()
                    .await
                    .map_err(classify_response_error)?;
                validate_created_message(&created, &request)?;
                Ok(MessageSendOutcome {
                    message_id: created.id,
                    channel_id: created.channel_id,
                    rate_limit,
                })
            }
            _ => Err(MessageError::UnexpectedResponse {
                status: status.as_u16(),
            }),
        }
    }

    fn from_parts(
        token: BotToken,
        base_url: &str,
        endpoint_kind: EndpointKind,
        timeouts: MessageTimeouts,
    ) -> Result<Self, MessageError> {
        let base_url = parse_base_url(base_url, endpoint_kind)?;
        let mut builder = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .read_timeout(timeouts.response)
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if matches!(endpoint_kind, EndpointKind::LoopbackTest) {
            builder = builder.no_proxy();
        }
        let http = builder.build().map_err(|_| MessageError::ClientBuild)?;
        Ok(Self {
            http,
            base_url,
            token,
            endpoint_kind,
        })
    }

    fn create_message_url(&self, request: &CreateMessageRequest) -> Result<Url, MessageError> {
        let mut url = self.base_url.clone();
        let mut path = url
            .path_segments_mut()
            .map_err(|_| MessageError::InvalidEndpoint)?;
        path.clear()
            .push("api")
            .push(request.api_version().as_str())
            .push("channels")
            .push(request.channel_id())
            .push("messages");
        drop(path);
        if self.api_version() != DiscordApiVersion::V10 || DISCORD_API_VERSION != "v10" {
            return Err(MessageError::InvalidEndpoint);
        }
        Ok(url)
    }
}

impl fmt::Debug for DiscordMessageClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let endpoint = match self.endpoint_kind {
            EndpointKind::Official => "official",
            EndpointKind::LoopbackTest => "loopback-test",
        };
        formatter
            .debug_struct("DiscordMessageClient")
            .field("api_version", &DiscordApiVersion::V10)
            .field("transport", &"reqwest-rustls")
            .field("endpoint", &endpoint)
            .field("send_attempts", &1_u8)
            .finish()
    }
}

#[derive(Deserialize)]
struct CreatedMessageWire {
    id: String,
    channel_id: String,
}

#[derive(Clone, Copy, Deserialize)]
struct RateLimitBodyWire {
    retry_after: Option<f64>,
    global: Option<bool>,
}

fn validate_created_message(
    created: &CreatedMessageWire,
    request: &CreateMessageRequest,
) -> Result<(), MessageError> {
    if !is_discord_id(&created.id)
        || !is_discord_id(&created.channel_id)
        || created.channel_id != request.channel_id()
    {
        return Err(MessageError::Ambiguous {
            reason: AmbiguousReason::InvalidResponse,
        });
    }
    Ok(())
}

async fn rate_limit_body(mut response: Response) -> Option<RateLimitBodyWire> {
    let mut body = Vec::new();
    loop {
        let chunk = response.chunk().await.ok()?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > MAX_RATE_LIMIT_BODY_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<RateLimitBodyWire>(&body).ok()
}

fn classify_reqwest_error(error: &reqwest::Error) -> MessageError {
    if error.is_connect() {
        MessageError::PreDispatch {
            failure: if error.is_timeout() {
                PreDispatchFailure::ConnectTimeout
            } else {
                PreDispatchFailure::ConnectFailed
            },
        }
    } else {
        MessageError::Ambiguous {
            reason: if error.is_timeout() {
                AmbiguousReason::Timeout
            } else {
                AmbiguousReason::ConnectionInterrupted
            },
        }
    }
}

fn classify_response_error(error: reqwest::Error) -> MessageError {
    let reason = if error.is_timeout() {
        AmbiguousReason::Timeout
    } else if error.is_decode() || error.is_body() {
        AmbiguousReason::ResponseReadFailed
    } else {
        AmbiguousReason::ConnectionInterrupted
    };
    MessageError::Ambiguous { reason }
}

#[cfg(test)]
fn classify_transport_failure(failure: TransportFailure) -> MessageError {
    match failure {
        TransportFailure::ConnectTimeout => MessageError::PreDispatch {
            failure: PreDispatchFailure::ConnectTimeout,
        },
        TransportFailure::AmbiguousTimeout => MessageError::Ambiguous {
            reason: AmbiguousReason::Timeout,
        },
    }
}

fn parse_base_url(raw: &str, endpoint_kind: EndpointKind) -> Result<Url, MessageError> {
    let url = Url::parse(raw).map_err(|_| MessageError::InvalidEndpoint)?;
    let origin_only = matches!(url.path(), "" | "/")
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !origin_only {
        return Err(MessageError::InvalidEndpoint);
    }

    match endpoint_kind {
        EndpointKind::Official => {
            if url.scheme() == "https"
                && url.host_str() == Some("discord.com")
                && matches!(url.port(), None | Some(443))
            {
                Ok(url)
            } else {
                Err(MessageError::InvalidEndpoint)
            }
        }
        EndpointKind::LoopbackTest => {
            if url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
                && url.port().is_some()
            {
                Ok(url)
            } else {
                Err(MessageError::InvalidEndpoint)
            }
        }
    }
}

fn is_valid_bot_token(value: &str) -> bool {
    if value.is_empty() || value.len() > 512 {
        return false;
    }
    let mut segments = value.split('.');
    let first = segments.next().unwrap_or_default();
    let second = segments.next().unwrap_or_default();
    let third = segments.next().unwrap_or_default();
    segments.next().is_none()
        && [first, second, third]
            .iter()
            .all(|segment| !segment.is_empty() && segment.bytes().all(is_token_byte))
}

const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'=')
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}
