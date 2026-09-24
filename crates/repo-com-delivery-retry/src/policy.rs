use std::{
    error::Error,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use repo_com_delivery::{
    ClaimPermit, DeliveryAttempt, DeliveryCoordinator, DeliveryError, DeliveryState,
    TransitionRequest,
};
use repo_com_discord_message::{
    AmbiguousReason, MessageError, MessageSendOutcome, PreDispatchFailure,
};

pub use repo_com_discord_message::MAX_DISCORD_DIRECTED_WAIT;

/// The total number of transport attempts allowed for one delivery revision.
///
/// This counts the initial request as attempt one.  It is intentionally not a
/// per-call retry count: callers cannot reset the budget by beginning a new
/// recovery loop.
pub const MAX_TRANSPORT_ATTEMPTS: u8 = 3;

/// Default lower and upper bounds for locally generated pre-dispatch jitter.
///
/// These are transport safety bounds, not Discord rate-limit assumptions.  A
/// caller may inject a different bounded range through [`RetryPolicy`].
pub const DEFAULT_MIN_JITTER: Duration = Duration::from_millis(100);
pub const DEFAULT_MAX_JITTER: Duration = Duration::from_secs(1);

/// A redacted reason that a post-dispatch result cannot be proven safe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownReason {
    /// The request or response wait expired after dispatch became possible.
    Timeout,
    /// The connection was interrupted after dispatch became possible.
    ConnectionReset,
    /// Discord returned a server response after dispatch became possible.
    ServerResponse { status: u16 },
    /// A successful response did not contain a valid message identity.
    InvalidResponse,
    /// The response body could not be read after a successful status.
    ResponseReadFailed,
}

impl UnknownReason {
    /// Returns a stable, non-sensitive reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Timeout => "post-dispatch-timeout",
            Self::ConnectionReset => "post-dispatch-connection-reset",
            Self::ServerResponse { .. } => "post-dispatch-server-response",
            Self::InvalidResponse => "invalid-success-response",
            Self::ResponseReadFailed => "response-read-failed",
        }
    }
}

impl fmt::Display for UnknownReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("the outcome is unknown after a timeout"),
            Self::ConnectionReset => {
                formatter.write_str("the outcome is unknown after a connection reset")
            }
            Self::ServerResponse { status } => {
                write!(formatter, "the outcome is unknown after HTTP {status}")
            }
            Self::InvalidResponse => {
                formatter.write_str("the response did not contain a valid message identity")
            }
            Self::ResponseReadFailed => {
                formatter.write_str("the response could not be read completely")
            }
        }
    }
}

/// The immutable local evidence required to reconcile an unknown transport
/// result.  It deliberately carries the exact content needed for comparison,
/// while its `Debug` implementation redacts that content.
#[derive(Clone, Eq, PartialEq)]
pub struct UnknownRecoveryEvidence {
    /// Configured destination alias.
    pub destination_alias: String,
    /// Configured workspace ID.
    pub workspace_id: String,
    /// Configured destination channel ID.
    pub channel_id: String,
    /// Optional dedicated bot author ID, filled by the recovery composition.
    pub bot_author_id: Option<String>,
    /// Durable per-attempt request nonce.
    pub request_nonce: String,
    /// Deterministic nonce rendered into the content.
    pub content_nonce: String,
    /// Exact immutable intended content.
    pub exact_content: String,
}

impl UnknownRecoveryEvidence {
    /// Captures the exact destination and content evidence from a durable
    /// attempt.  The bot author is supplied later by the read-side composition
    /// because it is not stored in the delivery attempt projection.
    #[must_use]
    pub fn from_attempt(attempt: &DeliveryAttempt) -> Self {
        Self {
            destination_alias: attempt.destination_alias.clone(),
            workspace_id: attempt.resolved_destination.workspace_id.clone(),
            channel_id: attempt.resolved_destination.channel_id.clone(),
            bot_author_id: None,
            request_nonce: attempt.request_nonce.clone(),
            content_nonce: attempt.content_nonce.clone(),
            exact_content: attempt.exact_content.clone(),
        }
    }

    /// Adds the configured dedicated bot author for a complete recovery
    /// target.
    #[must_use]
    pub fn with_bot_author_id(mut self, bot_author_id: impl Into<String>) -> Self {
        self.bot_author_id = Some(bot_author_id.into());
        self
    }
}

impl fmt::Debug for UnknownRecoveryEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnknownRecoveryEvidence")
            .field("destination_alias", &self.destination_alias)
            .field("workspace_id", &self.workspace_id)
            .field("channel_id", &self.channel_id)
            .field("bot_author_id", &self.bot_author_id)
            .field("request_nonce", &self.request_nonce)
            .field("content_nonce", &self.content_nonce)
            .field("exact_content", &"[REDACTED]")
            .finish()
    }
}

/// A result from exactly one transport attempt.
///
/// The type carries dispatch certainty explicitly.  In particular, a server
/// response is represented as [`UnknownReason::ServerResponse`] rather than a
/// retryable failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportOutcome {
    /// Discord returned a validated message identifier.
    Accepted { message_id: String },
    /// Discord returned HTTP 429 and supplied optional dynamic delay data.
    RateLimited { retry_after: Option<Duration> },
    /// The request was proven not to have left the local process.
    PreDispatch { failure: PreDispatchFailure },
    /// Discord or local request validation definitively rejected the request.
    DefinitiveFailure { code: String },
    /// Dispatch may have reached Discord; no automatic resend is permitted.
    Unknown { reason: UnknownReason },
}

impl TransportOutcome {
    /// Builds an accepted result.
    #[must_use]
    pub fn accepted(message_id: impl Into<String>) -> Self {
        Self::Accepted {
            message_id: message_id.into(),
        }
    }

    /// Builds a rate-limited result from a server-directed delay.
    #[must_use]
    pub fn rate_limited(retry_after: Option<Duration>) -> Self {
        Self::RateLimited { retry_after }
    }

    /// Builds a proven pre-dispatch result.
    #[must_use]
    pub const fn pre_dispatch(failure: PreDispatchFailure) -> Self {
        Self::PreDispatch { failure }
    }

    /// Builds a definitive, non-retryable result.
    #[must_use]
    pub fn definitive_failure(code: impl Into<String>) -> Self {
        Self::DefinitiveFailure { code: code.into() }
    }

    /// Builds an ambiguous post-dispatch result.
    #[must_use]
    pub const fn unknown(reason: UnknownReason) -> Self {
        Self::Unknown { reason }
    }

    /// Converts the one-attempt message adapter's typed result.
    #[must_use]
    pub fn from_message_result(result: Result<MessageSendOutcome, MessageError>) -> Self {
        match result {
            Ok(outcome) => Self::accepted(outcome.message_id()),
            Err(error) => Self::from_message_error(&error),
        }
    }

    /// Converts a typed message error without retaining a response body.
    #[must_use]
    pub fn from_message_error(error: &MessageError) -> Self {
        match error {
            MessageError::PreDispatch { failure } => Self::PreDispatch { failure: *failure },
            MessageError::RateLimited { info } => Self::RateLimited {
                retry_after: info.retry_after,
            },
            MessageError::Server { status } => Self::Unknown {
                reason: UnknownReason::ServerResponse { status: *status },
            },
            MessageError::Ambiguous { reason } => Self::Unknown {
                reason: match reason {
                    AmbiguousReason::Timeout => UnknownReason::Timeout,
                    AmbiguousReason::ConnectionInterrupted => UnknownReason::ConnectionReset,
                    AmbiguousReason::InvalidResponse => UnknownReason::InvalidResponse,
                    AmbiguousReason::ResponseReadFailed => UnknownReason::ResponseReadFailed,
                },
            },
            _ => Self::DefinitiveFailure {
                code: error.code().to_owned(),
            },
        }
    }

    /// Classifies a small HTTP status fixture for policy tests and adapters.
    #[must_use]
    pub fn from_status(status: u16, retry_after: Option<Duration>) -> Self {
        match status {
            429 => Self::RateLimited { retry_after },
            500..=599 => Self::Unknown {
                reason: UnknownReason::ServerResponse { status },
            },
            _ => Self::DefinitiveFailure {
                code: format!("http-{status}"),
            },
        }
    }

    /// Returns the stable error code for a non-success result.
    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        match self {
            Self::DefinitiveFailure { code } => Some(code),
            Self::Unknown { reason } => Some(reason.code()),
            Self::PreDispatch { failure } => Some(match failure {
                PreDispatchFailure::ConnectFailed => "connect-failed",
                PreDispatchFailure::ConnectTimeout => "connect-timeout",
            }),
            Self::RateLimited { .. } => Some("rate-limited"),
            Self::Accepted { .. } => None,
        }
    }

    /// Returns whether this outcome blocks automatic resend as ambiguous.
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    /// Returns whether this outcome is one of the two narrowly safe retry
    /// classes.
    #[must_use]
    pub const fn is_safe_to_retry(&self) -> bool {
        matches!(self, Self::PreDispatch { .. } | Self::RateLimited { .. })
    }
}

impl From<MessageError> for TransportOutcome {
    fn from(error: MessageError) -> Self {
        Self::from_message_error(&error)
    }
}

impl From<&MessageError> for TransportOutcome {
    fn from(error: &MessageError) -> Self {
        Self::from_message_error(error)
    }
}

impl From<&TransportOutcome> for TransportOutcome {
    fn from(outcome: &TransportOutcome) -> Self {
        outcome.clone()
    }
}

/// The reason a retry decision is waiting before another transport attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryDelayKind {
    /// A locally generated, bounded pre-dispatch jitter delay.
    PreDispatchJitter,
    /// A Discord-provided delay, capped by policy.
    DiscordDirected,
}

/// Why a retry runner stopped without authorizing another attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryStopReason {
    /// The three-attempt budget was exhausted.
    AttemptsExhausted,
    /// A 429 response did not provide a usable server-directed delay.
    MissingServerDelay,
    /// The supplied attempt number was outside the legal range.
    InvalidAttemptNumber,
    /// The current durable delivery state is not retryable.
    BlockedState,
}

/// The complete result of classifying one transport attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetryDecision {
    /// The attempt was accepted and no further request is permitted.
    Accepted { message_id: String },
    /// A bounded wait precedes exactly the next numbered attempt.
    Wait {
        next_attempt: u8,
        delay: Duration,
        kind: RetryDelayKind,
    },
    /// The attempt was definitively rejected and is not retryable.
    Failed { code: String },
    /// Dispatch may have reached Discord; reconciliation is required.
    Unknown { reason: UnknownReason },
    /// No further transport attempt is authorized.
    Stop {
        reason: RetryStopReason,
        code: String,
    },
}

impl RetryDecision {
    /// Returns whether this decision permits exactly one subsequent attempt.
    #[must_use]
    pub const fn permits_next_attempt(&self) -> bool {
        matches!(self, Self::Wait { .. })
    }

    /// Returns whether this decision is an ambiguous delivery outcome.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    /// Returns the delivery state implied by a completed classification.
    ///
    /// A [`RetryStopReason::AttemptsExhausted`] result is intentionally not
    /// mapped to `failed` or `unknown`; callers must retain the original
    /// transport evidence and handle the stop explicitly.
    #[must_use]
    pub const fn delivery_state_hint(&self) -> Option<DeliveryState> {
        match self {
            Self::Accepted { .. } => Some(DeliveryState::Accepted),
            Self::Failed { .. } => Some(DeliveryState::Failed),
            Self::Unknown { .. } => Some(DeliveryState::Unknown),
            Self::Wait { .. } => Some(DeliveryState::RetryWait),
            Self::Stop { .. } => None,
        }
    }
}

/// A local validation failure for retry policy configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryPolicyError {
    /// The maximum attempt count must be between one and three.
    InvalidAttemptLimit,
    /// The jitter minimum and maximum are inverted.
    InvalidJitterBounds,
    /// The Discord wait cap is outside the hard safety bound.
    InvalidDiscordWaitCap,
}

impl fmt::Display for RetryPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAttemptLimit => "retry attempt limit must be between one and three",
            Self::InvalidJitterBounds => "pre-dispatch jitter bounds are invalid",
            Self::InvalidDiscordWaitCap => "Discord-directed wait cap exceeds thirty seconds",
        })
    }
}

impl Error for RetryPolicyError {}

/// A deterministic, injectable retry policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    max_attempts: u8,
    min_jitter: Duration,
    max_jitter: Duration,
    max_discord_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: MAX_TRANSPORT_ATTEMPTS,
            min_jitter: DEFAULT_MIN_JITTER,
            max_jitter: DEFAULT_MAX_JITTER,
            max_discord_wait: MAX_DISCORD_DIRECTED_WAIT,
        }
    }
}

impl RetryPolicy {
    /// Maximum number of total transport attempts.
    pub const MAX_ATTEMPTS: u8 = MAX_TRANSPORT_ATTEMPTS;
    /// Maximum Discord-directed wait for one attempt.
    pub const MAX_DISCORD_DIRECTED_WAIT: Duration = MAX_DISCORD_DIRECTED_WAIT;

    /// Creates the safe default policy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_attempts: MAX_TRANSPORT_ATTEMPTS,
            min_jitter: DEFAULT_MIN_JITTER,
            max_jitter: DEFAULT_MAX_JITTER,
            max_discord_wait: MAX_DISCORD_DIRECTED_WAIT,
        }
    }

    /// Creates a policy with an injectable pre-dispatch jitter range.
    pub fn with_jitter_bounds(min: Duration, max: Duration) -> Result<Self, RetryPolicyError> {
        Self::with_limits(MAX_TRANSPORT_ATTEMPTS, min, max, MAX_DISCORD_DIRECTED_WAIT)
    }

    /// Creates a policy with all local limits explicit.
    pub fn with_limits(
        max_attempts: u8,
        min_jitter: Duration,
        max_jitter: Duration,
        max_discord_wait: Duration,
    ) -> Result<Self, RetryPolicyError> {
        if !(1..=MAX_TRANSPORT_ATTEMPTS).contains(&max_attempts) {
            return Err(RetryPolicyError::InvalidAttemptLimit);
        }
        if min_jitter > max_jitter {
            return Err(RetryPolicyError::InvalidJitterBounds);
        }
        if max_discord_wait > MAX_DISCORD_DIRECTED_WAIT {
            return Err(RetryPolicyError::InvalidDiscordWaitCap);
        }
        Ok(Self {
            max_attempts,
            min_jitter,
            max_jitter,
            max_discord_wait,
        })
    }

    /// Returns the total attempt budget.
    #[must_use]
    pub const fn max_attempts(self) -> u8 {
        self.max_attempts
    }

    /// Returns the inclusive lower jitter bound.
    #[must_use]
    pub const fn min_jitter(self) -> Duration {
        self.min_jitter
    }

    /// Returns the inclusive upper jitter bound.
    #[must_use]
    pub const fn max_jitter(self) -> Duration {
        self.max_jitter
    }

    /// Returns the per-attempt Discord wait cap.
    #[must_use]
    pub const fn max_discord_wait(self) -> Duration {
        self.max_discord_wait
    }

    /// Clamps a caller-provided jitter value into the configured range.
    #[must_use]
    pub fn bounded_jitter(self, value: Duration) -> Duration {
        value.max(self.min_jitter).min(self.max_jitter)
    }

    /// Classifies one outcome using the lower jitter bound when no value is
    /// supplied.  The decision itself never sleeps.
    #[must_use]
    pub fn classify<O>(self, attempt_number: u8, outcome: O) -> RetryDecision
    where
        O: Into<TransportOutcome>,
    {
        self.classify_with_jitter(attempt_number, outcome, None)
    }

    /// Compatibility alias for [`Self::classify`].
    #[must_use]
    pub fn decide<O>(self, attempt_number: u8, outcome: O) -> RetryDecision
    where
        O: Into<TransportOutcome>,
    {
        self.classify(attempt_number, outcome)
    }

    /// Classifies one outcome and applies an injected jitter value to a safe
    /// pre-dispatch retry.  Values outside the configured range are clamped,
    /// which makes the upper bound enforceable even for a faulty source.
    #[must_use]
    pub fn classify_with_jitter<O>(
        self,
        attempt_number: u8,
        outcome: O,
        jitter: Option<Duration>,
    ) -> RetryDecision
    where
        O: Into<TransportOutcome>,
    {
        let outcome = outcome.into();
        if attempt_number == 0 || attempt_number > self.max_attempts {
            return RetryDecision::Stop {
                reason: RetryStopReason::InvalidAttemptNumber,
                code: "invalid-attempt-number".to_owned(),
            };
        }

        match outcome {
            TransportOutcome::Accepted { message_id } => RetryDecision::Accepted {
                message_id: message_id.clone(),
            },
            TransportOutcome::DefinitiveFailure { code } => {
                RetryDecision::Failed { code: code.clone() }
            }
            TransportOutcome::Unknown { reason } => RetryDecision::Unknown { reason },
            TransportOutcome::PreDispatch { failure } => {
                if attempt_number >= self.max_attempts {
                    return RetryDecision::Stop {
                        reason: RetryStopReason::AttemptsExhausted,
                        code: match failure {
                            PreDispatchFailure::ConnectFailed => "connect-failed",
                            PreDispatchFailure::ConnectTimeout => "connect-timeout",
                        }
                        .to_owned(),
                    };
                }
                let delay = self.bounded_jitter(jitter.unwrap_or(self.min_jitter));
                RetryDecision::Wait {
                    next_attempt: attempt_number + 1,
                    delay,
                    kind: RetryDelayKind::PreDispatchJitter,
                }
            }
            TransportOutcome::RateLimited { retry_after } => {
                let Some(server_delay) = retry_after else {
                    return RetryDecision::Stop {
                        reason: RetryStopReason::MissingServerDelay,
                        code: "rate-limited-without-server-delay".to_owned(),
                    };
                };
                if attempt_number >= self.max_attempts {
                    return RetryDecision::Stop {
                        reason: RetryStopReason::AttemptsExhausted,
                        code: "rate-limited-attempt-budget-exhausted".to_owned(),
                    };
                }
                RetryDecision::Wait {
                    next_attempt: attempt_number + 1,
                    delay: server_delay.min(self.max_discord_wait),
                    kind: RetryDelayKind::DiscordDirected,
                }
            }
        }
    }
}

/// A local clock boundary used by retry and reconciliation code.
///
/// Production callers can use [`SystemClock`].  Contract tests should inject
/// [`ManualClock`] so no wall-clock sleep is needed.
pub trait Clock {
    /// Returns the current Unix time in seconds.
    fn now_unix_seconds(&self) -> u64;

    /// Waits for a bounded delay.
    fn wait(&mut self, delay: Duration) -> Result<(), ClockError>;
}

impl<C> Clock for &mut C
where
    C: Clock + ?Sized,
{
    fn now_unix_seconds(&self) -> u64 {
        (**self).now_unix_seconds()
    }

    fn wait(&mut self, delay: Duration) -> Result<(), ClockError> {
        (**self).wait(delay)
    }
}

/// A small redacted clock failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockError;

impl fmt::Display for ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the injected clock could not complete a wait")
    }
}

impl Error for ClockError {}

/// A deterministic clock that records waits instead of sleeping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualClock {
    now_unix_seconds: u64,
    waits: Vec<Duration>,
}

impl ManualClock {
    /// Creates a manual clock at an injected Unix timestamp.
    #[must_use]
    pub const fn new(now_unix_seconds: u64) -> Self {
        Self {
            now_unix_seconds,
            waits: Vec::new(),
        }
    }

    /// Advances the clock without recording a transport wait.
    pub fn advance(&mut self, duration: Duration) {
        self.now_unix_seconds = self.now_unix_seconds.saturating_add(duration.as_secs());
    }

    /// Returns all waits observed by the clock in order.
    #[must_use]
    pub fn waits(&self) -> &[Duration] {
        &self.waits
    }

    /// Returns the current injected time.
    #[must_use]
    pub const fn now(&self) -> u64 {
        self.now_unix_seconds
    }
}

impl Clock for ManualClock {
    fn now_unix_seconds(&self) -> u64 {
        self.now_unix_seconds
    }

    fn wait(&mut self, delay: Duration) -> Result<(), ClockError> {
        self.waits.push(delay);
        self.advance(delay);
        Ok(())
    }
}

/// A production clock.  The retry policy still caps every requested delay;
/// this type is not used by deterministic contract tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs())
    }

    fn wait(&mut self, delay: Duration) -> Result<(), ClockError> {
        std::thread::sleep(delay);
        Ok(())
    }
}

/// One-attempt transport callback used by [`RetryRunner`].
///
/// The callback must perform at most one HTTP request.  Retry decisions and
/// waits remain outside the callback so a transport implementation cannot
/// silently expand the three-attempt budget.
pub trait OneAttemptTransport {
    /// Performs one transport attempt.
    fn send_attempt(&mut self, attempt_number: u8) -> TransportOutcome;

    /// Compatibility alias for adapters that call the operation an attempt.
    fn attempt(&mut self, attempt_number: u8) -> TransportOutcome {
        self.send_attempt(attempt_number)
    }
}

/// A failure to run the bounded retry loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryRunError {
    /// The starting attempt was outside one through three.
    InvalidStartAttempt(u8),
    /// A retry would have exceeded the hard three-attempt budget.
    AttemptLimitReached,
    /// The durable state does not permit an automatic transport attempt.
    BlockedState(DeliveryState),
    /// The injected clock failed before another request.
    Clock(ClockError),
}

impl fmt::Display for RetryRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStartAttempt(attempt) => {
                write!(formatter, "invalid starting transport attempt {attempt}")
            }
            Self::AttemptLimitReached => {
                formatter.write_str("the three-attempt transport budget is exhausted")
            }
            Self::BlockedState(state) => {
                write!(
                    formatter,
                    "delivery state {state} does not permit an automatic send"
                )
            }
            Self::Clock(_) => formatter.write_str("retry wait failed"),
        }
    }
}

impl Error for RetryRunError {}

impl From<ClockError> for RetryRunError {
    fn from(error: ClockError) -> Self {
        Self::Clock(error)
    }
}

/// One observed transport attempt and the exact decision made for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryAttemptRecord {
    /// Attempt number used for the request.
    pub attempt_number: u8,
    /// Exact typed transport result.
    pub outcome: TransportOutcome,
    /// Result of policy classification.
    pub decision: RetryDecision,
    /// Delay requested before the next attempt, if any.
    pub delay_before_next: Option<Duration>,
}

impl RetryAttemptRecord {
    /// Returns the attempt number.
    #[must_use]
    pub const fn attempt_number(&self) -> u8 {
        self.attempt_number
    }
}

/// Result of a bounded retry run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryRunResult {
    /// Number of requests actually invoked by this run.
    pub request_count: u8,
    /// Highest attempt number invoked.
    pub last_attempt: u8,
    /// All observed attempts, including the final non-retry result.
    pub attempts: Vec<RetryAttemptRecord>,
    /// Final classification.
    pub decision: RetryDecision,
    /// Exact recovery evidence retained when the result is unknown.
    pub unknown_evidence: Option<UnknownRecoveryEvidence>,
}

impl RetryRunResult {
    /// Returns the actual number of transport requests made.
    #[must_use]
    pub const fn request_count(&self) -> u8 {
        self.request_count
    }

    /// Returns the final attempt number.
    #[must_use]
    pub const fn last_attempt(&self) -> u8 {
        self.last_attempt
    }

    /// Returns whether the run ended in an ambiguous delivery outcome.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        self.decision.is_unknown()
    }

    /// Returns the exact reconciliation evidence attached to an unknown run.
    #[must_use]
    pub const fn unknown_evidence(&self) -> Option<&UnknownRecoveryEvidence> {
        self.unknown_evidence.as_ref()
    }
}

/// A policy-driven runner with an injected clock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryRunner<C> {
    policy: RetryPolicy,
    clock: C,
    jitter: Option<Duration>,
    unknown_evidence: Option<UnknownRecoveryEvidence>,
}

impl<C> RetryRunner<C> {
    /// Creates a runner with the safe default policy.
    pub fn new(clock: C) -> Self {
        Self {
            policy: RetryPolicy::new(),
            clock,
            jitter: None,
            unknown_evidence: None,
        }
    }

    /// Creates a runner with an explicit policy.
    pub fn with_policy(clock: C, policy: RetryPolicy) -> Self {
        Self {
            policy,
            clock,
            jitter: None,
            unknown_evidence: None,
        }
    }

    /// Supplies one deterministic jitter value for pre-dispatch waits.
    #[must_use]
    pub fn with_jitter(mut self, jitter: Duration) -> Self {
        self.jitter = Some(jitter);
        self
    }

    /// Attaches exact destination and content evidence for an unknown result.
    #[must_use]
    pub fn with_unknown_evidence(mut self, evidence: UnknownRecoveryEvidence) -> Self {
        self.unknown_evidence = Some(evidence);
        self
    }

    /// Returns the configured unknown-recovery evidence.
    #[must_use]
    pub const fn unknown_evidence(&self) -> Option<&UnknownRecoveryEvidence> {
        self.unknown_evidence.as_ref()
    }

    /// Returns the policy used by this runner.
    #[must_use]
    pub const fn policy(&self) -> RetryPolicy {
        self.policy
    }

    /// Returns the injected clock.
    #[must_use]
    pub const fn clock(&self) -> &C {
        &self.clock
    }
}

impl<C: Clock> RetryRunner<C> {
    /// Runs at most three attempts starting at `first_attempt`.
    ///
    /// This low-level entry point is crate-visible so network composition must
    /// use the single-use [`Self::run_after_claim`] boundary below.
    pub(crate) fn run<T: OneAttemptTransport>(
        &mut self,
        transport: &mut T,
        first_attempt: u8,
    ) -> Result<RetryRunResult, RetryRunError> {
        if first_attempt == 0 || first_attempt > self.policy.max_attempts() {
            return Err(RetryRunError::InvalidStartAttempt(first_attempt));
        }
        let mut attempt_number = first_attempt;
        let mut attempts = Vec::new();
        let unknown_evidence = self.unknown_evidence.clone();
        loop {
            let outcome = transport.send_attempt(attempt_number);
            let decision = self
                .policy
                .classify_with_jitter(attempt_number, &outcome, self.jitter);
            let delay_before_next = match &decision {
                RetryDecision::Wait { delay, .. } => Some(*delay),
                _ => None,
            };
            let request_count = u8::try_from(attempts.len())
                .map_err(|_| RetryRunError::AttemptLimitReached)?
                .saturating_add(1);
            attempts.push(RetryAttemptRecord {
                attempt_number,
                outcome,
                decision: decision.clone(),
                delay_before_next,
            });
            match decision {
                RetryDecision::Wait {
                    next_attempt,
                    delay,
                    ..
                } => {
                    if next_attempt > self.policy.max_attempts() || next_attempt <= attempt_number {
                        return Err(RetryRunError::AttemptLimitReached);
                    }
                    self.clock.wait(delay)?;
                    attempt_number = next_attempt;
                }
                final_decision => {
                    let unknown_evidence = final_decision
                        .is_unknown()
                        .then_some(unknown_evidence)
                        .flatten();
                    return Ok(RetryRunResult {
                        request_count,
                        last_attempt: attempt_number,
                        attempts,
                        decision: final_decision,
                        unknown_evidence,
                    });
                }
            }
        }
    }

    /// Runs only after a single-use delivery claim permit has been obtained.
    ///
    /// Consuming the permit is deliberate: a duplicate caller cannot use the
    /// same authorization to start a second transport sequence.  The delivery
    /// coordinator must perform the claim/audit commit before constructing the
    /// permit, and it must obtain a fresh permit through `claim_retry` for a
    /// `retry_wait` attempt.
    pub fn run_after_claim<T: OneAttemptTransport>(
        &mut self,
        transport: &mut T,
        permit: ClaimPermit,
    ) -> Result<RetryRunResult, RetryRunError> {
        let attempt = permit.into_attempt();
        self.run_recorded_attempt(transport, &attempt)
    }

    /// Explicit alias for [`Self::run_after_claim`].
    pub fn run_with_permit<T: OneAttemptTransport>(
        &mut self,
        transport: &mut T,
        permit: ClaimPermit,
    ) -> Result<RetryRunResult, RetryRunError> {
        self.run_after_claim(transport, permit)
    }

    fn run_recorded_attempt<T: OneAttemptTransport>(
        &mut self,
        transport: &mut T,
        attempt: &DeliveryAttempt,
    ) -> Result<RetryRunResult, RetryRunError> {
        let first_attempt = match attempt.state {
            DeliveryState::Claimed => u8::try_from(attempt.attempt_number)
                .map_err(|_| RetryRunError::AttemptLimitReached)?,
            DeliveryState::RetryWait => u8::try_from(attempt.attempt_number)
                .map_err(|_| RetryRunError::AttemptLimitReached)?
                .checked_add(1)
                .ok_or(RetryRunError::AttemptLimitReached)?,
            DeliveryState::Unknown
            | DeliveryState::Unresolved
            | DeliveryState::ReconciledAbsent
            | DeliveryState::ReconciledAccepted
            | DeliveryState::Accepted
            | DeliveryState::Failed
            | DeliveryState::Unclaimed => {
                return Err(RetryRunError::BlockedState(attempt.state));
            }
        };
        if self.unknown_evidence.is_none() {
            self.unknown_evidence = Some(UnknownRecoveryEvidence::from_attempt(attempt));
        }
        self.run(transport, first_attempt)
    }
}

/// Applies one completed retry classification to the existing local delivery
/// state machine.  This helper performs no network I/O and never creates a new
/// claim; the next retry still requires a fresh coordinator claim.
pub fn record_delivery_decision(
    coordinator: &mut DeliveryCoordinator,
    attempt: &DeliveryAttempt,
    decision: &RetryDecision,
    completed_at: impl Into<String>,
    actor_kind: impl Into<String>,
) -> Result<DeliveryAttempt, DeliveryError> {
    let completed_at = completed_at.into();
    let actor_kind = actor_kind.into();
    let request = match decision {
        RetryDecision::Accepted { message_id } => TransitionRequest::new(
            &attempt.repository_id,
            &attempt.draft_id,
            attempt.revision,
            &attempt.attempt_id,
            DeliveryState::Accepted,
            completed_at,
            actor_kind,
        )
        .with_remote_message_id(message_id),
        RetryDecision::Failed { code } => TransitionRequest::new(
            &attempt.repository_id,
            &attempt.draft_id,
            attempt.revision,
            &attempt.attempt_id,
            DeliveryState::Failed,
            completed_at,
            actor_kind,
        )
        .with_error_code(code),
        RetryDecision::Unknown { reason } => TransitionRequest::new(
            &attempt.repository_id,
            &attempt.draft_id,
            attempt.revision,
            &attempt.attempt_id,
            DeliveryState::Unknown,
            completed_at,
            actor_kind,
        )
        .with_error_code(reason.code()),
        RetryDecision::Wait { .. } => TransitionRequest::new(
            &attempt.repository_id,
            &attempt.draft_id,
            attempt.revision,
            &attempt.attempt_id,
            DeliveryState::RetryWait,
            completed_at,
            actor_kind,
        )
        .with_error_code("retry-wait"),
        RetryDecision::Stop { .. } => {
            return Err(DeliveryError::InvalidInput {
                field: "terminal retry stop",
            });
        }
    };
    coordinator.record_transition(&request)
}

/// Named recovery facade used by the messaging composition layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryRecovery<C> {
    runner: RetryRunner<C>,
}

impl<C> DeliveryRecovery<C> {
    /// Creates a recovery facade with the default retry policy.
    pub fn new(clock: C) -> Self {
        Self {
            runner: RetryRunner::new(clock),
        }
    }

    /// Creates a recovery facade with an explicit retry policy.
    pub fn with_policy(clock: C, policy: RetryPolicy) -> Self {
        Self {
            runner: RetryRunner::with_policy(clock, policy),
        }
    }

    /// Applies an injected jitter value to pre-dispatch waits.
    #[must_use]
    pub fn with_jitter(self, jitter: Duration) -> Self {
        Self {
            runner: self.runner.with_jitter(jitter),
        }
    }

    /// Returns the underlying bounded runner.
    #[must_use]
    pub const fn runner(&self) -> &RetryRunner<C> {
        &self.runner
    }

    /// Records a completed classification through the existing delivery state
    /// machine without authorizing another request.
    pub fn record_decision(
        &self,
        coordinator: &mut DeliveryCoordinator,
        attempt: &DeliveryAttempt,
        decision: &RetryDecision,
        completed_at: impl Into<String>,
        actor_kind: impl Into<String>,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        record_delivery_decision(coordinator, attempt, decision, completed_at, actor_kind)
    }
}

impl<C: Clock> DeliveryRecovery<C> {
    /// Runs a bounded transport sequence after an explicit claim state.
    pub fn run_after_claim<T: OneAttemptTransport>(
        &mut self,
        transport: &mut T,
        permit: ClaimPermit,
    ) -> Result<RetryRunResult, RetryRunError> {
        self.runner.run_after_claim(transport, permit)
    }
}

#[cfg(test)]
mod tests {
    use super::{PreDispatchFailure, RetryDecision, RetryPolicy, TransportOutcome, UnknownReason};

    #[test]
    fn five_hundreds_are_unknown_not_retryable() {
        let decision = RetryPolicy::new().classify(1, TransportOutcome::from_status(503, None));
        assert_eq!(
            decision,
            RetryDecision::Unknown {
                reason: UnknownReason::ServerResponse { status: 503 }
            }
        );
    }

    #[test]
    fn message_error_mapping_preserves_dispatch_certainty() {
        assert!(matches!(
            TransportOutcome::from_message_error(
                &repo_com_discord_message::MessageError::PreDispatch {
                    failure: PreDispatchFailure::ConnectFailed
                }
            ),
            TransportOutcome::PreDispatch { .. }
        ));
    }
}
