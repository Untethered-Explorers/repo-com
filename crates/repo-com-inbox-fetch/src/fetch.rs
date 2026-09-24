//! Read-only Discord REST v10 message retrieval and bounded pagination.

use std::{
    collections::HashSet,
    env,
    error::Error,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use repo_com_config::{ResolvedConfig, ResolvedInbound};
use repo_com_discord_client::{BOT_TOKEN_ENV, DISCORD_API_VERSION, DISCORD_BASE_URL};
use repo_com_foundation::{ErrorCategory, RepoComError};
use repo_com_inbox_state::{
    InboundCurrentSnapshotInput, InboundItemInput, InboxPage, InboxState, InboxStateError,
    PageCommitResult, PageItem,
};
use reqwest::{
    Response, StatusCode, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::{
    boundary::{
        BoundaryError, FetchBoundary, MAX_FETCH_PAGES, MAX_RAW_MESSAGES, MESSAGES_PER_PAGE,
        rfc3339_to_unix_millis,
    },
    filter::{
        AcceptedDelivery, AttachmentIndicator, FetchProvenance, FilterContext, RemoteMessage,
        ReplyEvidence, UntrustedInboundEnvelope, should_retain, to_untrusted_envelope,
    },
    reconcile::{ReconciliationError, ReconciliationSummary, reconcile_page_with_reader},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RATE_LIMIT_BODY_BYTES: usize = 16 * 1024;
/// Maximum Discord-directed delay exposed by one read response.
pub const MAX_DISCORD_DIRECTED_WAIT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = "DiscordBot (https://github.com/Untethered-Explorers/repo-com, 0.1.0)";

/// A dynamic rate-limit observation from a read response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReadRateLimitInfo {
    /// Server-provided retry delay, capped at 30 seconds.
    pub retry_after: Option<Duration>,
    /// Server-provided reset delay, capped at 30 seconds.
    pub reset_after: Option<Duration>,
    /// Server-provided absolute reset value, retained as an opaque string.
    pub reset_at: Option<String>,
    /// Server-provided route limit, when present.
    pub limit: Option<u64>,
    /// Server-provided remaining request count, when present.
    pub remaining: Option<u64>,
    /// Whether Discord marked the response as globally limited.
    pub global: bool,
    /// Scope reported by Discord.
    pub scope: ReadRateLimitScope,
    /// Optional route bucket identifier.
    pub bucket: Option<String>,
}

/// Scope reported for a bounded read response.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReadRateLimitScope {
    /// A route or global scope not further identified.
    Route,
    /// A global scope.
    Global,
    /// A bot/user-specific scope.
    User,
    /// A shared resource scope.
    Shared,
    /// No recognized scope was reported.
    #[default]
    Unknown,
}

impl ReadRateLimitInfo {
    fn from_headers(headers: &HeaderMap, limited: bool) -> Self {
        let scope = match header(headers, "x-ratelimit-scope") {
            Some(value) if value.eq_ignore_ascii_case("global") => ReadRateLimitScope::Global,
            Some(value) if value.eq_ignore_ascii_case("user") => ReadRateLimitScope::User,
            Some(value) if value.eq_ignore_ascii_case("shared") => ReadRateLimitScope::Shared,
            Some(value) if value.eq_ignore_ascii_case("route") => ReadRateLimitScope::Route,
            _ if limited => ReadRateLimitScope::Route,
            _ => ReadRateLimitScope::Unknown,
        };
        let global = bool_header(headers, "x-ratelimit-global").unwrap_or(false)
            || scope == ReadRateLimitScope::Global;
        Self {
            retry_after: duration_header(headers, "retry-after"),
            reset_after: duration_header(headers, "x-ratelimit-reset-after"),
            reset_at: bounded_header(headers, "x-ratelimit-reset", 64),
            limit: unsigned_header(headers, "x-ratelimit-limit"),
            remaining: unsigned_header(headers, "x-ratelimit-remaining"),
            global,
            scope,
            bucket: bounded_header(headers, "x-ratelimit-bucket", 128),
        }
    }

    fn include_body(&mut self, seconds: Option<f64>, global: Option<bool>) {
        if let Some(seconds) = seconds.and_then(bounded_duration) {
            self.retry_after = Some(
                self.retry_after
                    .map_or(seconds, |current| current.max(seconds)),
            );
        }
        if global == Some(true) {
            self.global = true;
            self.scope = ReadRateLimitScope::Global;
        }
    }
}

/// A redacted failure from one read-only Discord request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// The environment-only bot token is absent.
    MissingBotToken,
    /// The environment value is not a raw three-part bot token.
    InvalidBotToken,
    /// The production or loopback endpoint is not approved.
    InvalidEndpoint,
    /// The bounded HTTP client could not be built.
    ClientBuild,
    /// Discord rejected the bot credential.
    Authentication,
    /// Discord denied the configured read.
    PermissionDenied,
    /// The configured channel or point message was not found.
    NotFound,
    /// Discord rejected the read as invalid.
    ValidationRejected,
    /// Discord reported a conflicting remote state.
    Conflict,
    /// Discord returned a dynamic 429 response.
    RateLimited { info: ReadRateLimitInfo },
    /// Discord returned a 5xx response.
    Server { status: u16 },
    /// Discord returned an unexpected status.
    UnexpectedStatus { status: u16 },
    /// The request failed before a response was available.
    Transport,
    /// The bounded request timed out.
    Timeout,
    /// A response did not satisfy the minimal typed contract.
    InvalidResponse,
}

impl ReadError {
    /// Returns a stable, redacted error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingBotToken => "missing-bot-token",
            Self::InvalidBotToken => "invalid-bot-token",
            Self::InvalidEndpoint => "invalid-discord-endpoint",
            Self::ClientBuild => "client-build-failed",
            Self::Authentication => "discord-authentication-failed",
            Self::PermissionDenied => "discord-read-permission-denied",
            Self::NotFound => "discord-read-not-found",
            Self::ValidationRejected => "discord-read-validation-rejected",
            Self::Conflict => "discord-read-conflict",
            Self::RateLimited { .. } => "discord-read-rate-limited",
            Self::Server { .. } => "discord-read-server-error",
            Self::UnexpectedStatus { .. } => "discord-read-unexpected-status",
            Self::Transport => "discord-read-transport-error",
            Self::Timeout => "discord-read-timeout",
            Self::InvalidResponse => "discord-read-invalid-response",
        }
    }

    /// Returns the provider-neutral error category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::MissingBotToken | Self::InvalidBotToken | Self::InvalidEndpoint => {
                ErrorCategory::UsageOrSchema
            }
            Self::Authentication => ErrorCategory::Authentication,
            Self::PermissionDenied => ErrorCategory::Permission,
            Self::Conflict => ErrorCategory::RemoteConflict,
            Self::ValidationRejected => ErrorCategory::UsageOrSchema,
            Self::RateLimited { .. } | Self::Transport | Self::Timeout | Self::Server { .. } => {
                ErrorCategory::ConnectivityRateLimit
            }
            Self::ClientBuild | Self::UnexpectedStatus { .. } | Self::InvalidResponse => {
                ErrorCategory::InternalFailure
            }
            Self::NotFound => ErrorCategory::UsageOrSchema,
        }
    }

    /// Returns whether an explicit caller may retry this read later.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Server { .. } | Self::Transport | Self::Timeout
        )
    }

    /// Converts this error to a redacted foundation error value.
    #[must_use]
    pub fn to_repo_com_error(&self) -> RepoComError {
        RepoComError::new(self.category(), self.to_string())
    }
}

/// Compatibility name for the dynamic read-limit projection.
pub type FetchRateLimitInfo = ReadRateLimitInfo;
/// Compatibility name matching the shared Discord client spelling.
pub type RateLimitInfo = ReadRateLimitInfo;

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingBotToken => {
                "the dedicated Discord bot token is missing; set REPO_COM_DISCORD_TOKEN"
            }
            Self::InvalidBotToken => {
                "the Discord credential is not a raw dedicated bot token; rotate REPO_COM_DISCORD_TOKEN"
            }
            Self::InvalidEndpoint => "the Discord endpoint is not an approved origin",
            Self::ClientBuild => "the bounded Discord read client could not be built",
            Self::Authentication => {
                "Discord rejected the dedicated bot credentials; rotate REPO_COM_DISCORD_TOKEN"
            }
            Self::PermissionDenied => "Discord denied the configured inbound read",
            Self::NotFound => "Discord did not find the configured inbound resource",
            Self::ValidationRejected => "Discord rejected the inbound read as invalid",
            Self::Conflict => "Discord reported an inbound read conflict",
            Self::RateLimited { .. } => "Discord rate-limited the inbound read",
            Self::Server { status } => {
                return write!(
                    formatter,
                    "Discord returned an inbound server error: {status}"
                );
            }
            Self::UnexpectedStatus { status } => {
                return write!(formatter, "Discord returned an inbound status: {status}");
            }
            Self::Transport => "the Discord inbound read failed before a response",
            Self::Timeout => "the bounded Discord inbound read timed out",
            Self::InvalidResponse => "Discord returned an invalid inbound response",
        })
    }
}

impl Error for ReadError {}

/// Why a bounded fetch has more work or has reached a hard limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationReason {
    /// The remote page stream reported no continuation.
    Complete,
    /// The ten-page budget stopped the operation.
    PageLimit,
    /// The one-thousand-raw-message budget stopped the operation.
    MessageLimit,
    /// Both hard bounds were reached together.
    BothLimits,
}

impl ContinuationReason {
    /// Returns whether a caller should issue another explicit fetch.
    #[must_use]
    pub const fn has_more(self) -> bool {
        !matches!(self, Self::Complete)
    }
}

/// Explicit continuation metadata for a bounded fetch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchContinuation {
    /// Opaque cursor to use for the next explicit fetch.
    pub next_cursor: Option<String>,
    /// The same continuation expressed as a boundary input.
    pub next_boundary: Option<FetchBoundary>,
    /// Reason this operation stopped.
    pub reason: ContinuationReason,
    /// Number of pages read in this operation.
    pub pages_fetched: usize,
    /// Number of raw messages counted, including filtered messages.
    pub raw_messages_seen: usize,
    /// Whether the page budget was exhausted.
    pub page_limit_reached: bool,
    /// Whether the raw-message budget was exhausted.
    pub message_limit_reached: bool,
    /// Remaining page budget.
    pub pages_remaining: usize,
    /// Remaining raw-message budget.
    pub raw_messages_remaining: usize,
}

impl FetchContinuation {
    /// Returns whether an explicit next fetch is indicated.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some() && self.reason.has_more()
    }
}

/// A single explicit inbound fetch request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchRequest {
    /// Repository identity in the current configuration.
    pub repository_id: String,
    /// Repository-local inbound alias, never a raw channel ID.
    pub alias: String,
    /// Exactly one cursor or RFC 3339 boundary.
    pub boundary: FetchBoundary,
    /// Dedicated bot user ID used for direct-mention evidence.
    pub bot_user_id: String,
    /// Optional deterministic observation time.
    pub retrieved_at: Option<String>,
}

impl FetchRequest {
    /// Creates a request with a generated observation time at fetch time.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        boundary: FetchBoundary,
        bot_user_id: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            alias: alias.into(),
            boundary,
            bot_user_id: bot_user_id.into(),
            retrieved_at: None,
        }
    }

    /// Creates a request from optional cursor and time inputs, rejecting both
    /// or neither before any transport is attempted.
    pub fn from_parts(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        cursor: Option<&str>,
        time: Option<&str>,
        bot_user_id: impl Into<String>,
    ) -> Result<Self, BoundaryError> {
        Ok(Self::new(
            repository_id,
            alias,
            FetchBoundary::from_parts(cursor, time)?,
            bot_user_id,
        ))
    }

    /// Creates a cursor-bound request.
    pub fn with_cursor(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        cursor: impl Into<String>,
        bot_user_id: impl Into<String>,
    ) -> Result<Self, BoundaryError> {
        Ok(Self::new(
            repository_id,
            alias,
            FetchBoundary::cursor(cursor)?,
            bot_user_id,
        ))
    }

    /// Creates an RFC 3339-bound request.
    pub fn with_time(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        time: impl Into<String>,
        bot_user_id: impl Into<String>,
    ) -> Result<Self, BoundaryError> {
        Ok(Self::new(
            repository_id,
            alias,
            FetchBoundary::time(time)?,
            bot_user_id,
        ))
    }

    /// Sets a deterministic local observation time.
    #[must_use]
    pub fn with_retrieved_at(mut self, value: impl Into<String>) -> Self {
        self.retrieved_at = Some(value.into());
        self
    }

    /// Returns the configured observation time, if explicitly supplied.
    #[must_use]
    pub fn retrieved_at(&self) -> Option<&str> {
        self.retrieved_at.as_deref()
    }
}

/// Result of one bounded remote fetch.
#[derive(Clone, Eq, PartialEq)]
pub struct FetchResult {
    /// Repository scope.
    pub repository_id: String,
    /// Configured inbound alias.
    pub alias: String,
    /// Configured channel ID.
    pub channel_id: String,
    /// Accepted untrusted envelopes in deterministic ID order.
    pub items: Vec<UntrustedInboundEnvelope>,
    /// Number of raw messages counted before filtering.
    pub raw_messages: usize,
    /// Number of HTTP/read pages consumed.
    pub pages_fetched: usize,
    /// Cursor that should be durably stored after this result.
    pub authoritative_cursor: String,
    /// Explicit continuation metadata.
    pub continuation: FetchContinuation,
    /// Latest dynamic rate-limit metadata observed.
    pub rate_limit: Option<ReadRateLimitInfo>,
}

impl fmt::Debug for FetchResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchResult")
            .field("repository_id", &self.repository_id)
            .field("alias", &self.alias)
            .field("channel_id", &self.channel_id)
            .field("items", &self.items)
            .field("raw_messages", &self.raw_messages)
            .field("pages_fetched", &self.pages_fetched)
            .field("authoritative_cursor", &self.authoritative_cursor)
            .field("continuation", &self.continuation)
            .field("rate_limit", &self.rate_limit)
            .finish()
    }
}

impl FetchResult {
    /// Returns whether another explicit bounded fetch is indicated.
    #[must_use]
    pub fn has_continuation(&self) -> bool {
        self.continuation.has_more()
    }
}

/// Result after a page was committed and point checks were attempted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredFetchResult {
    /// Remote fetch result.
    pub fetch: FetchResult,
    /// Atomic local page commit result.
    pub commit: PageCommitResult,
    /// Read-only point-check result.
    pub reconciliation: ReconciliationSummary,
}

/// A safe failure from inbound fetching or its local commit boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FetchError {
    /// The required token is absent.
    MissingBotToken,
    /// The token shape is invalid.
    InvalidBotToken,
    /// The endpoint is not approved.
    InvalidEndpoint,
    /// The HTTP client could not be built.
    ClientBuild,
    /// The boundary input is invalid.
    Boundary(BoundaryError),
    /// The requested repository does not match the current configuration.
    RepositoryMismatch,
    /// The alias is not configured in the current repository.
    UnknownAlias,
    /// The alias is configured but disabled.
    DisabledAlias,
    /// A bot or local identity field is invalid.
    InvalidIdentity,
    /// A supplied local observation time is invalid.
    InvalidObservationTime,
    /// A read-only Discord request failed.
    Read(ReadError),
    /// The local page commit failed.
    Storage(InboxStateError),
    /// A post-commit point-check operation failed.
    Reconciliation(ReconciliationError),
}

impl FetchError {
    /// Returns a stable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingBotToken => ReadError::MissingBotToken.code(),
            Self::InvalidBotToken => ReadError::InvalidBotToken.code(),
            Self::InvalidEndpoint => ReadError::InvalidEndpoint.code(),
            Self::ClientBuild => ReadError::ClientBuild.code(),
            Self::Boundary(_) => "inbound-boundary-invalid",
            Self::RepositoryMismatch => "inbound-repository-mismatch",
            Self::UnknownAlias => "inbound-alias-unknown",
            Self::DisabledAlias => "inbound-alias-disabled",
            Self::InvalidIdentity => "inbound-identity-invalid",
            Self::InvalidObservationTime => "inbound-observation-time-invalid",
            Self::Read(error) => error.code(),
            Self::Storage(_) => "inbound-storage-error",
            Self::Reconciliation(_) => "inbound-reconciliation-error",
        }
    }

    /// Returns the provider-neutral error category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::MissingBotToken | Self::InvalidBotToken | Self::InvalidEndpoint => {
                ErrorCategory::UsageOrSchema
            }
            Self::Read(error) => error.category(),
            Self::Storage(_) | Self::Reconciliation(_) => ErrorCategory::StorageIntegrity,
            Self::ClientBuild => ErrorCategory::InternalFailure,
            _ => ErrorCategory::UsageOrSchema,
        }
    }

    /// Returns whether an explicit caller may retry this operation later.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Read(error) => error.is_retryable(),
            Self::Storage(error) => error.is_retryable(),
            Self::Reconciliation(ReconciliationError::Read(error)) => error.is_retryable(),
            Self::Reconciliation(ReconciliationError::State(error)) => error.is_retryable(),
            _ => false,
        }
    }

    /// Converts this error to a redacted foundation error value.
    #[must_use]
    pub fn to_repo_com_error(&self) -> RepoComError {
        RepoComError::new(self.category(), self.to_string())
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBotToken => formatter.write_str(
                "the dedicated Discord bot token is missing; set REPO_COM_DISCORD_TOKEN",
            ),
            Self::InvalidBotToken => formatter
                .write_str("the Discord credential is invalid; rotate REPO_COM_DISCORD_TOKEN"),
            Self::InvalidEndpoint => formatter.write_str("the Discord endpoint is not approved"),
            Self::ClientBuild => formatter.write_str("the Discord read client could not be built"),
            Self::Boundary(error) => write!(formatter, "inbound boundary rejected: {error}"),
            Self::RepositoryMismatch => {
                formatter.write_str("the fetch repository does not match current configuration")
            }
            Self::UnknownAlias => formatter.write_str("the inbound alias is not configured"),
            Self::DisabledAlias => formatter.write_str("the inbound alias is disabled"),
            Self::InvalidIdentity => formatter.write_str("an inbound identity is invalid"),
            Self::InvalidObservationTime => {
                formatter.write_str("the inbound observation time is invalid")
            }
            Self::Read(error) => write!(formatter, "inbound Discord read failed: {error}"),
            Self::Storage(error) => write!(formatter, "inbound page commit failed: {error}"),
            Self::Reconciliation(error) => {
                write!(formatter, "inbound point reconciliation failed: {error}")
            }
        }
    }
}

impl Error for FetchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Boundary(error) => Some(error),
            Self::Read(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::Reconciliation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BoundaryError> for FetchError {
    fn from(error: BoundaryError) -> Self {
        Self::Boundary(error)
    }
}

impl From<ReadError> for FetchError {
    fn from(error: ReadError) -> Self {
        Self::Read(error)
    }
}

impl From<InboxStateError> for FetchError {
    fn from(error: InboxStateError) -> Self {
        Self::Storage(error)
    }
}

impl From<ReconciliationError> for FetchError {
    fn from(error: ReconciliationError) -> Self {
        Self::Reconciliation(error)
    }
}

/// One page returned by the read adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteMessagePage {
    /// Messages in the page.
    pub messages: Vec<RemoteMessage>,
    /// Optional explicit continuation hint supplied by a test adapter.
    pub has_more: Option<bool>,
    /// Dynamic rate-limit metadata.
    pub rate_limit: Option<ReadRateLimitInfo>,
}

/// Internal read abstraction used by the concrete client and focused tests.
pub(crate) trait InboundReader {
    /// Reads one configured-channel message page.
    async fn read_page(
        &self,
        channel_id: &str,
        after: &str,
    ) -> Result<RemoteMessagePage, ReadError>;

    /// Reads one exact configured-channel message.
    async fn read_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<RemoteMessage, ReadError>;
}

/// A dedicated-bot, read-only REST v10 client used by the fetcher.
pub struct DiscordInboundClient {
    http: reqwest::Client,
    base_url: Url,
    token: BotToken,
    endpoint_kind: EndpointKind,
}

impl DiscordInboundClient {
    /// Constructs the production client using the fixed Discord origin.
    pub fn from_environment() -> Result<Self, ReadError> {
        Self::from_parts(
            BotToken::from_environment()?,
            DISCORD_BASE_URL,
            EndpointKind::Official,
        )
    }

    /// Constructs a loopback WireMock client while reading the token only from
    /// `REPO_COM_DISCORD_TOKEN`.
    #[doc(hidden)]
    pub fn from_environment_for_test_server(base_url: &str) -> Result<Self, ReadError> {
        Self::from_parts(
            BotToken::from_environment()?,
            base_url,
            EndpointKind::LoopbackTest,
        )
    }

    /// Returns the pinned API version.
    #[must_use]
    pub const fn api_version(&self) -> &'static str {
        DISCORD_API_VERSION
    }

    #[cfg(test)]
    pub(crate) fn from_synthetic_token_for_test(
        base_url: &str,
        token: &str,
    ) -> Result<Self, ReadError> {
        Self::from_parts(
            BotToken::from_value(token.to_owned())?,
            base_url,
            EndpointKind::LoopbackTest,
        )
    }

    fn from_parts(
        token: BotToken,
        base_url: &str,
        endpoint_kind: EndpointKind,
    ) -> Result<Self, ReadError> {
        let base_url = parse_base_url(base_url, endpoint_kind)?;
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if matches!(endpoint_kind, EndpointKind::LoopbackTest) {
            builder = builder.no_proxy();
        }
        let http = builder.build().map_err(|_| ReadError::ClientBuild)?;
        Ok(Self {
            http,
            base_url,
            token,
            endpoint_kind,
        })
    }

    fn messages_url(&self, channel_id: &str, after: &str) -> Result<Url, ReadError> {
        validate_path_component(channel_id)?;
        validate_query_component(after)?;
        let mut url = self.base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| ReadError::InvalidEndpoint)?;
            path.clear()
                .push("api")
                .push(DISCORD_API_VERSION)
                .push("channels")
                .push(channel_id)
                .push("messages");
        }
        url.query_pairs_mut()
            .append_pair("limit", &MESSAGES_PER_PAGE.to_string())
            .append_pair("after", after);
        Ok(url)
    }

    fn message_url(&self, channel_id: &str, message_id: &str) -> Result<Url, ReadError> {
        validate_path_component(channel_id)?;
        validate_path_component(message_id)?;
        let mut url = self.base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| ReadError::InvalidEndpoint)?;
            path.clear()
                .push("api")
                .push(DISCORD_API_VERSION)
                .push("channels")
                .push(channel_id)
                .push("messages")
                .push(message_id);
        }
        Ok(url)
    }

    async fn get(&self, url: Url) -> Result<Response, ReadError> {
        let authorization = self.token.authorization_value();
        let header = HeaderValue::from_str(authorization.as_str())
            .map_err(|_| ReadError::InvalidBotToken)?;
        self.http
            .get(url)
            .header(AUTHORIZATION, header)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    ReadError::Timeout
                } else {
                    ReadError::Transport
                }
            })
    }
}

impl fmt::Debug for DiscordInboundClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let endpoint = match self.endpoint_kind {
            EndpointKind::Official => "official",
            EndpointKind::LoopbackTest => "loopback-test",
        };
        formatter
            .debug_struct("DiscordInboundClient")
            .field("api_version", &DISCORD_API_VERSION)
            .field("transport", &"reqwest-rustls")
            .field("endpoint", &endpoint)
            .field("read_only", &true)
            .finish()
    }
}

impl InboundReader for DiscordInboundClient {
    async fn read_page(
        &self,
        channel_id: &str,
        after: &str,
    ) -> Result<RemoteMessagePage, ReadError> {
        let response = self.get(self.messages_url(channel_id, after)?).await?;
        let rate_limit = ReadRateLimitInfo::from_headers(response.headers(), false);
        match response.status() {
            StatusCode::UNAUTHORIZED => Err(ReadError::Authentication),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(ReadError::ValidationRejected)
            }
            StatusCode::FORBIDDEN => Err(ReadError::PermissionDenied),
            StatusCode::NOT_FOUND => Err(ReadError::NotFound),
            StatusCode::CONFLICT => Err(ReadError::Conflict),
            StatusCode::TOO_MANY_REQUESTS => {
                let mut info = ReadRateLimitInfo::from_headers(response.headers(), true);
                if let Some(body) = rate_limit_body(response).await {
                    info.include_body(body.0, body.1);
                }
                Err(ReadError::RateLimited { info })
            }
            status if status.is_server_error() => Err(ReadError::Server {
                status: status.as_u16(),
            }),
            status if status.is_success() => {
                let wire: Vec<MessageWire> = decode_json(response).await?;
                let messages = wire
                    .into_iter()
                    .map(RemoteMessage::try_from)
                    .collect::<Result<Vec<_>, _>>()?;
                let has_more = Some(messages.len() == MESSAGES_PER_PAGE);
                Ok(RemoteMessagePage {
                    messages,
                    has_more,
                    rate_limit: Some(rate_limit),
                })
            }
            status => Err(ReadError::UnexpectedStatus {
                status: status.as_u16(),
            }),
        }
    }

    async fn read_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<RemoteMessage, ReadError> {
        let response = self.get(self.message_url(channel_id, message_id)?).await?;
        match response.status() {
            StatusCode::UNAUTHORIZED => Err(ReadError::Authentication),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(ReadError::ValidationRejected)
            }
            StatusCode::FORBIDDEN => Err(ReadError::PermissionDenied),
            StatusCode::NOT_FOUND => Err(ReadError::NotFound),
            StatusCode::CONFLICT => Err(ReadError::Conflict),
            StatusCode::TOO_MANY_REQUESTS => {
                let mut info = ReadRateLimitInfo::from_headers(response.headers(), true);
                if let Some(body) = rate_limit_body(response).await {
                    info.include_body(body.0, body.1);
                }
                Err(ReadError::RateLimited { info })
            }
            status if status.is_server_error() => Err(ReadError::Server {
                status: status.as_u16(),
            }),
            status if status.is_success() => {
                let wire: MessageWire = decode_json(response).await?;
                RemoteMessage::try_from(wire)
            }
            status => Err(ReadError::UnexpectedStatus {
                status: status.as_u16(),
            }),
        }
    }
}

fn client_error_to_fetch(error: ReadError) -> FetchError {
    match error {
        ReadError::MissingBotToken => FetchError::MissingBotToken,
        ReadError::InvalidBotToken => FetchError::InvalidBotToken,
        ReadError::InvalidEndpoint => FetchError::InvalidEndpoint,
        ReadError::ClientBuild => FetchError::ClientBuild,
        other => FetchError::Read(other),
    }
}

/// The concrete, safe public fetch service.
pub struct InboundFetcher {
    client: DiscordInboundClient,
}

impl InboundFetcher {
    /// Constructs a production fetcher.
    pub fn from_environment() -> Result<Self, FetchError> {
        DiscordInboundClient::from_environment()
            .map(|client| Self { client })
            .map_err(client_error_to_fetch)
    }

    /// Constructs a loopback WireMock fetcher while retaining environment-only
    /// production authentication.
    #[doc(hidden)]
    pub fn from_environment_for_test_server(base_url: &str) -> Result<Self, FetchError> {
        DiscordInboundClient::from_environment_for_test_server(base_url)
            .map(|client| Self { client })
            .map_err(client_error_to_fetch)
    }

    /// Returns a redacted client diagnostic.
    #[must_use]
    pub fn client_debug(&self) -> String {
        format!("{:?}", self.client)
    }

    /// Performs one explicit bounded remote fetch.
    pub async fn fetch(
        &self,
        config: &ResolvedConfig,
        request: &FetchRequest,
        accepted_deliveries: &[AcceptedDelivery],
    ) -> Result<FetchResult, FetchError> {
        fetch_with_reader(&self.client, config, request, accepted_deliveries).await
    }

    /// Fetches, atomically commits the resulting page, and then performs the
    /// bounded read-only point checks for that page.
    pub async fn fetch_and_store(
        &self,
        config: &ResolvedConfig,
        request: &FetchRequest,
        accepted_deliveries: &[AcceptedDelivery],
        state: &mut InboxState,
    ) -> Result<StoredFetchResult, FetchError> {
        fetch_and_store_with_reader(&self.client, config, request, accepted_deliveries, state).await
    }
}

pub(crate) async fn fetch_with_reader<R: InboundReader>(
    reader: &R,
    config: &ResolvedConfig,
    request: &FetchRequest,
    accepted_deliveries: &[AcceptedDelivery],
) -> Result<FetchResult, FetchError> {
    let inbound = resolve_inbound(config, request)?;
    let initial_cursor = request.boundary.initial_cursor()?;
    let retrieved_at = request.retrieved_at.clone().unwrap_or_else(current_rfc3339);
    validate_observation_time(&retrieved_at)?;
    let mut after = initial_cursor.clone();
    let mut seen_ids = HashSet::new();
    let mut items = Vec::new();
    let mut raw_messages = 0_usize;
    let mut pages_fetched = 0_usize;
    let mut last_cursor = initial_cursor.clone();
    let mut latest_rate_limit = None;
    let mut page_has_more = false;

    while pages_fetched < MAX_FETCH_PAGES && raw_messages < MAX_RAW_MESSAGES {
        let page = reader.read_page(&inbound.channel_id, &after).await?;
        pages_fetched += 1;
        latest_rate_limit = page.rate_limit.clone().or(latest_rate_limit);
        if page.messages.len() > MESSAGES_PER_PAGE {
            return Err(ReadError::InvalidResponse.into());
        }
        if page.messages.is_empty() {
            page_has_more = false;
            break;
        }

        let mut page_messages = page.messages;
        page_messages.sort_by(|left, right| compare_ids(&left.id, &right.id));
        for message in &page_messages {
            validate_remote_message(message)?;
            if message.channel_id != inbound.channel_id {
                return Err(ReadError::InvalidResponse.into());
            }
            if !seen_ids.insert(message.id.clone()) {
                return Err(ReadError::InvalidResponse.into());
            }
            if is_not_after(&message.id, &after) {
                return Err(ReadError::InvalidResponse.into());
            }
            raw_messages += 1;
            if raw_messages > MAX_RAW_MESSAGES {
                return Err(ReadError::InvalidResponse.into());
            }
            last_cursor = message.id.clone();
        }

        let context = FilterContext::new(
            &request.repository_id,
            &inbound.channel_id,
            &request.bot_user_id,
            accepted_deliveries,
        )
        .map_err(|_| FetchError::InvalidIdentity)?;
        let provenance = FetchProvenance::discord(
            request.repository_id.clone(),
            inbound.workspace_id.clone(),
            request.alias.clone(),
            inbound.channel_id.clone(),
            retrieved_at.clone(),
        );
        for message in &page_messages {
            if should_retain(message, &context) {
                items.push(to_untrusted_envelope(message, &context, provenance.clone()));
            }
        }

        page_has_more = page
            .has_more
            .unwrap_or(page_messages.len() == MESSAGES_PER_PAGE);
        if !page_has_more {
            break;
        }
        // Discord's `after` strategy returns the oldest matching chunk (the
        // response itself is newest-first); its highest snowflake is the next
        // forward cursor.
        after = last_cursor.clone();
    }

    let page_limit_reached = pages_fetched >= MAX_FETCH_PAGES && page_has_more;
    let message_limit_reached = raw_messages >= MAX_RAW_MESSAGES && page_has_more;
    let reason = match (page_limit_reached, message_limit_reached) {
        (true, true) => ContinuationReason::BothLimits,
        (true, false) => ContinuationReason::PageLimit,
        (false, true) => ContinuationReason::MessageLimit,
        (false, false) => ContinuationReason::Complete,
    };
    let next_cursor = if reason.has_more() {
        Some(last_cursor.clone())
    } else {
        None
    };
    let next_boundary = next_cursor
        .as_deref()
        .map(FetchBoundary::cursor)
        .transpose()
        .map_err(FetchError::Boundary)?;
    let continuation = FetchContinuation {
        next_cursor,
        next_boundary,
        reason,
        pages_fetched,
        raw_messages_seen: raw_messages,
        page_limit_reached,
        message_limit_reached,
        pages_remaining: MAX_FETCH_PAGES.saturating_sub(pages_fetched),
        raw_messages_remaining: MAX_RAW_MESSAGES.saturating_sub(raw_messages),
    };
    items.sort_by(|left, right| compare_ids(&left.remote_message_id, &right.remote_message_id));

    Ok(FetchResult {
        repository_id: request.repository_id.clone(),
        alias: request.alias.clone(),
        channel_id: inbound.channel_id,
        items,
        raw_messages,
        pages_fetched,
        authoritative_cursor: last_cursor,
        continuation,
        rate_limit: latest_rate_limit,
    })
}

pub(crate) async fn fetch_and_store_with_reader<R: InboundReader>(
    reader: &R,
    config: &ResolvedConfig,
    request: &FetchRequest,
    accepted_deliveries: &[AcceptedDelivery],
    state: &mut InboxState,
) -> Result<StoredFetchResult, FetchError> {
    let result = fetch_with_reader(reader, config, request, accepted_deliveries).await?;
    let inbound = resolve_inbound(config, request)?;
    let retrieved_at = result
        .items
        .first()
        .map(|item| item.provenance.retrieved_at.clone())
        .unwrap_or_else(current_rfc3339);
    validate_observation_time(&retrieved_at)?;
    let mut page = InboxPage::new(
        result.repository_id.clone(),
        result.alias.clone(),
        result.authoritative_cursor.clone(),
        retrieved_at.clone(),
    );
    for envelope in &result.items {
        let existing = state.item(&result.repository_id, &envelope.remote_message_id)?;
        let first = if let Some(existing) = existing {
            existing
        } else {
            let mut first = InboundItemInput::new(
                result.repository_id.clone(),
                envelope.remote_message_id.clone(),
                result.channel_id.clone(),
                envelope.author.user_id.clone(),
                envelope.text.clone(),
                envelope.timestamp.clone(),
            );
            first.first_attachments_json = attachment_json(&envelope.attachment_indicators)?;
            first.created_at = retrieved_at.clone();
            first
        };
        if first.repository_id != result.repository_id || first.channel_id != result.channel_id {
            return Err(FetchError::InvalidIdentity);
        }
        let mut current = InboundCurrentSnapshotInput::new(
            result.repository_id.clone(),
            envelope.remote_message_id.clone(),
            Some(envelope.text.clone()),
            false,
            retrieved_at.clone(),
        );
        current.current_attachments_json = attachment_json(&envelope.attachment_indicators)?;
        page.add_item(PageItem::new(first, current));
    }
    let commit = state.commit_page(&page)?;
    let reconciliation = reconcile_page_with_reader(
        reader,
        state,
        &result.repository_id,
        &inbound,
        &result.items,
        &retrieved_at,
    )
    .await?;
    Ok(StoredFetchResult {
        fetch: result,
        commit,
        reconciliation,
    })
}

fn resolve_inbound(
    config: &ResolvedConfig,
    request: &FetchRequest,
) -> Result<ResolvedInbound, FetchError> {
    if request.repository_id != config.config.repository_id {
        return Err(FetchError::RepositoryMismatch);
    }
    if !safe_component(&request.repository_id)
        || !safe_component(&request.alias)
        || !is_discord_id(&request.bot_user_id)
    {
        return Err(FetchError::InvalidIdentity);
    }
    let inbound = config
        .inbound(&request.alias)
        .ok_or(FetchError::UnknownAlias)?
        .clone();
    if !inbound.enabled {
        return Err(FetchError::DisabledAlias);
    }
    if inbound.alias != request.alias {
        return Err(FetchError::InvalidIdentity);
    }
    if inbound.workspace_id != config.config.discord.workspace_id
        || !is_discord_id(&inbound.workspace_id)
        || !is_discord_id(&inbound.channel_id)
    {
        return Err(FetchError::InvalidIdentity);
    }
    Ok(inbound)
}

fn attachment_json(attachments: &[AttachmentIndicator]) -> Result<String, FetchError> {
    serde_json::to_string(attachments).map_err(|_| FetchError::InvalidIdentity)
}

fn validate_remote_message(message: &RemoteMessage) -> Result<(), ReadError> {
    if !safe_wire_component(&message.id)
        || !safe_wire_component(&message.channel_id)
        || !safe_wire_component(&message.author_id)
        || message.content.len() > 64 * 1024
        || message.timestamp.is_empty()
        || message.timestamp.len() > 128
        || message
            .edited_timestamp
            .as_deref()
            .is_some_and(|value| value.is_empty() || value.len() > 128)
        || message
            .mentioned_user_ids
            .iter()
            .any(|value| !safe_wire_component(value))
        || message
            .reply
            .referenced_message_id
            .as_deref()
            .is_some_and(|value| !safe_wire_component(value))
        || message
            .reply
            .referenced_channel_id
            .as_deref()
            .is_some_and(|value| !safe_wire_component(value))
        || message
            .reply
            .referenced_guild_id
            .as_deref()
            .is_some_and(|value| !safe_wire_component(value))
        || message
            .webhook_id
            .as_deref()
            .is_some_and(|value| !safe_wire_component(value))
    {
        return Err(ReadError::InvalidResponse);
    }
    for attachment in &message.attachments {
        if !safe_wire_component(&attachment.id) || attachment.filename.len() > 512 {
            return Err(ReadError::InvalidResponse);
        }
    }
    Ok(())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn safe_wire_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| byte.is_ascii_graphic())
        && !value.contains('/')
}

fn validate_path_component(value: &str) -> Result<(), ReadError> {
    if safe_wire_component(value) {
        Ok(())
    } else {
        Err(ReadError::InvalidResponse)
    }
}

fn validate_query_component(value: &str) -> Result<(), ReadError> {
    if !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(ReadError::InvalidResponse)
    }
}

fn validate_observation_time(value: &str) -> Result<(), FetchError> {
    if rfc3339_to_unix_millis(value).is_err() {
        Err(FetchError::InvalidObservationTime)
    } else {
        Ok(())
    }
}

fn is_not_after(candidate: &str, after: &str) -> bool {
    match (candidate.parse::<u128>(), after.parse::<u128>()) {
        (Ok(candidate), Ok(after)) => candidate <= after,
        _ => false,
    }
}

fn compare_ids(left: &str, right: &str) -> std::cmp::Ordering {
    match (left.parse::<u128>(), right.parse::<u128>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn attachment_wire_to_indicator(
    attachment: AttachmentWire,
) -> Result<AttachmentIndicator, ReadError> {
    let id = attachment.id.ok_or(ReadError::InvalidResponse)?;
    if !safe_wire_component(&id) {
        return Err(ReadError::InvalidResponse);
    }
    Ok(AttachmentIndicator {
        id,
        filename: attachment.filename.unwrap_or_default(),
        content_type: attachment.content_type,
        size: attachment.size,
    })
}

#[derive(Clone, Copy)]
enum EndpointKind {
    Official,
    LoopbackTest,
}

struct BotToken(Zeroizing<String>);

impl BotToken {
    fn from_environment() -> Result<Self, ReadError> {
        let value = env::var(BOT_TOKEN_ENV).map_err(|error| match error {
            env::VarError::NotPresent => ReadError::MissingBotToken,
            env::VarError::NotUnicode(_) => ReadError::InvalidBotToken,
        })?;
        Self::from_value(value)
    }

    fn from_value(value: String) -> Result<Self, ReadError> {
        let value = Zeroizing::new(value);
        if is_valid_bot_token(&value) {
            Ok(Self(value))
        } else {
            Err(ReadError::InvalidBotToken)
        }
    }

    fn authorization_value(&self) -> Zeroizing<String> {
        let mut value = Zeroizing::new(String::with_capacity(self.0.len() + 4));
        value.push_str("Bot ");
        value.push_str(self.0.as_str());
        value
    }
}

impl fmt::Debug for BotToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BotToken([REDACTED])")
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

fn parse_base_url(raw: &str, endpoint_kind: EndpointKind) -> Result<Url, ReadError> {
    let url = Url::parse(raw).map_err(|_| ReadError::InvalidEndpoint)?;
    let origin_only = matches!(url.path(), "" | "/")
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !origin_only {
        return Err(ReadError::InvalidEndpoint);
    }
    match endpoint_kind {
        EndpointKind::Official => {
            if url.scheme() == "https"
                && url.host_str() == Some("discord.com")
                && matches!(url.port(), None | Some(443))
            {
                Ok(url)
            } else {
                Err(ReadError::InvalidEndpoint)
            }
        }
        EndpointKind::LoopbackTest => {
            if url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
                && url.port().is_some()
            {
                Ok(url)
            } else {
                Err(ReadError::InvalidEndpoint)
            }
        }
    }
}

async fn decode_json<T: DeserializeOwned>(mut response: Response) -> Result<T, ReadError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            ReadError::Timeout
        } else {
            ReadError::Transport
        }
    })? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ReadError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ReadError::InvalidResponse)
}

async fn rate_limit_body(mut response: Response) -> Option<(Option<f64>, Option<bool>)> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RATE_LIMIT_BODY_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: RateLimitBodyWire = serde_json::from_slice(&bytes).ok()?;
    Some((body.retry_after, body.global))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn bool_header(headers: &HeaderMap, name: &str) -> Option<bool> {
    header(headers, name).and_then(|value| match value.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    })
}

fn unsigned_header(headers: &HeaderMap, name: &str) -> Option<u64> {
    header(headers, name).and_then(|value| value.parse::<u64>().ok())
}

fn duration_header(headers: &HeaderMap, name: &str) -> Option<Duration> {
    header(headers, name)
        .and_then(|value| value.parse::<f64>().ok())
        .and_then(bounded_duration)
}

fn bounded_header(headers: &HeaderMap, name: &str, maximum: usize) -> Option<String> {
    let value = header(headers, name)?;
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        None
    } else {
        Some(value.to_owned())
    }
}

fn bounded_duration(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(
        seconds.min(MAX_DISCORD_DIRECTED_WAIT.as_secs_f64()),
    ))
}

fn current_rfc3339() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_i64, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        });
    format_unix_millis(millis)
}

fn format_unix_millis(millis: i64) -> String {
    let seconds = millis.div_euclid(1_000);
    let millis_part = millis.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis_part:03}Z",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60,
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[derive(Debug, Deserialize)]
struct MessageWire {
    id: String,
    channel_id: String,
    #[serde(default)]
    content: Option<String>,
    timestamp: String,
    #[serde(default)]
    edited_timestamp: Option<String>,
    author: AuthorWire,
    #[serde(default)]
    mentions: Vec<MentionUserWire>,
    #[serde(default)]
    attachments: Vec<AttachmentWire>,
    #[serde(default)]
    message_reference: Option<ReferenceWire>,
    #[serde(default)]
    referenced_message: Option<serde_json::Value>,
    #[serde(default)]
    webhook_id: Option<String>,
    #[serde(rename = "type", default)]
    message_type: u64,
}

#[derive(Debug, Deserialize)]
struct AuthorWire {
    id: String,
    #[serde(default)]
    bot: bool,
}

#[derive(Debug, Deserialize)]
struct MentionUserWire {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AttachmentWire {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ReferenceWire {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    guild_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RateLimitBodyWire {
    retry_after: Option<f64>,
    global: Option<bool>,
}

impl TryFrom<MessageWire> for RemoteMessage {
    type Error = ReadError;

    fn try_from(wire: MessageWire) -> Result<Self, Self::Error> {
        if !safe_wire_component(&wire.id)
            || !safe_wire_component(&wire.channel_id)
            || !safe_wire_component(&wire.author.id)
        {
            return Err(ReadError::InvalidResponse);
        }
        let reply = match wire.message_reference {
            None => ReplyEvidence::none(),
            Some(reference) => ReplyEvidence {
                reference_present: true,
                referenced_message_id: reference.message_id,
                referenced_channel_id: reference.channel_id,
                referenced_guild_id: reference.guild_id,
                referenced_message_deleted: wire.message_type == 19
                    && wire.referenced_message.is_none(),
            },
        };
        let attachments = wire
            .attachments
            .into_iter()
            .map(attachment_wire_to_indicator)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            id: wire.id,
            channel_id: wire.channel_id,
            author_id: wire.author.id,
            author_is_bot: wire.author.bot,
            webhook_id: wire.webhook_id,
            content: wire.content.unwrap_or_default(),
            timestamp: wire.timestamp,
            edited_timestamp: wire.edited_timestamp,
            mentioned_user_ids: wire
                .mentions
                .into_iter()
                .map(|mention| mention.id)
                .collect(),
            reply,
            attachments,
            message_type: wire.message_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadRateLimitScope, current_rfc3339, format_unix_millis};

    #[test]
    fn current_time_format_is_rfc3339_like_without_content() {
        let value = format_unix_millis(0);
        assert_eq!(value, "1970-01-01T00:00:00.000Z");
        assert!(current_rfc3339().ends_with('Z'));
        assert_eq!(ReadRateLimitScope::default(), ReadRateLimitScope::Unknown);
    }
}
