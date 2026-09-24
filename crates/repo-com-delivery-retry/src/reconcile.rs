use std::{error::Error, fmt};

use repo_com_delivery::{DeliveryAttempt, DeliveryState};

use crate::policy::{Clock, UnknownRecoveryEvidence};

/// The minimum observation window before an absence decision is possible.
pub const ABSENCE_OBSERVATION_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The minimum number of complete successful reads required for absence.
pub const MIN_SUCCESSFUL_READS: u32 = 3;

/// The only Discord API version a reconciliation reader may use.
pub const RECONCILIATION_API_VERSION: &str = "v10";

/// Immutable evidence required to reconcile one unknown delivery.
#[derive(Clone, Eq, PartialEq)]
pub struct RecoveryTarget {
    /// Configured destination alias from the exact revision.
    pub destination_alias: String,
    /// Configured workspace ID.
    pub workspace_id: String,
    /// Configured destination channel ID.
    pub channel_id: String,
    /// Dedicated bot author ID.
    pub bot_author_id: String,
    /// Deterministic nonce rendered into the exact content.
    pub nonce: String,
    /// Exact immutable content, including its nonce footer.
    pub exact_content: String,
    /// Unix time at which the local attempt entered `unknown`.
    pub unknown_since_unix_seconds: u64,
}

impl fmt::Debug for RecoveryTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryTarget")
            .field("destination_alias", &self.destination_alias)
            .field("workspace_id", &self.workspace_id)
            .field("channel_id", &self.channel_id)
            .field("bot_author_id", &self.bot_author_id)
            .field("nonce", &self.nonce)
            .field("exact_content", &"[REDACTED]")
            .field(
                "unknown_since_unix_seconds",
                &self.unknown_since_unix_seconds,
            )
            .finish()
    }
}

impl RecoveryTarget {
    /// Creates and validates an exact reconciliation target.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        destination_alias: impl Into<String>,
        workspace_id: impl Into<String>,
        channel_id: impl Into<String>,
        bot_author_id: impl Into<String>,
        nonce: impl Into<String>,
        exact_content: impl Into<String>,
        unknown_since_unix_seconds: u64,
    ) -> Result<Self, ReconciliationError> {
        let target = Self {
            destination_alias: destination_alias.into(),
            workspace_id: workspace_id.into(),
            channel_id: channel_id.into(),
            bot_author_id: bot_author_id.into(),
            nonce: nonce.into(),
            exact_content: exact_content.into(),
            unknown_since_unix_seconds,
        };
        target.validate()?;
        Ok(target)
    }

    /// Builds a target from a durable unknown attempt and an observed start
    /// time.  The attempt's destination and content are copied without
    /// normalization or re-rendering.
    pub fn from_attempt(
        attempt: &DeliveryAttempt,
        bot_author_id: impl Into<String>,
        unknown_since_unix_seconds: u64,
    ) -> Result<Self, ReconciliationError> {
        Self::new(
            attempt.destination_alias.clone(),
            attempt.resolved_destination.workspace_id.clone(),
            attempt.resolved_destination.channel_id.clone(),
            bot_author_id,
            attempt.content_nonce.clone(),
            attempt.exact_content.clone(),
            unknown_since_unix_seconds,
        )
    }

    /// Builds a complete target from transport evidence and the configured bot
    /// author.
    pub fn from_evidence(
        evidence: &UnknownRecoveryEvidence,
        bot_author_id: impl Into<String>,
        unknown_since_unix_seconds: u64,
    ) -> Result<Self, ReconciliationError> {
        let bot_author_id = bot_author_id.into();
        if evidence
            .bot_author_id
            .as_ref()
            .is_some_and(|author| author != &bot_author_id)
        {
            return Err(ReconciliationError::InvalidTarget("bot author id"));
        }
        Self::new(
            evidence.destination_alias.clone(),
            evidence.workspace_id.clone(),
            evidence.channel_id.clone(),
            bot_author_id,
            evidence.content_nonce.clone(),
            evidence.exact_content.clone(),
            unknown_since_unix_seconds,
        )
    }

    /// Returns the request that a read-only adapter must service.
    #[must_use]
    pub fn request(&self) -> ReconciliationRequest {
        ReconciliationRequest {
            target: self.clone(),
        }
    }

    fn validate(&self) -> Result<(), ReconciliationError> {
        if !valid_component(&self.destination_alias, 128) {
            return Err(ReconciliationError::InvalidTarget("destination alias"));
        }
        if !valid_component(&self.workspace_id, 128) {
            return Err(ReconciliationError::InvalidTarget("workspace id"));
        }
        if !valid_component(&self.channel_id, 128) {
            return Err(ReconciliationError::InvalidTarget("channel id"));
        }
        if !valid_component(&self.bot_author_id, 128) {
            return Err(ReconciliationError::InvalidTarget("bot author id"));
        }
        if !valid_component(&self.nonce, 256) {
            return Err(ReconciliationError::InvalidTarget("nonce"));
        }
        if self.exact_content.is_empty() {
            return Err(ReconciliationError::InvalidTarget("exact content"));
        }
        Ok(())
    }
}

/// One exact read request.  It contains no mutation capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationRequest {
    /// Immutable target copied from the unknown attempt.
    pub target: RecoveryTarget,
}

impl ReconciliationRequest {
    /// Creates a request for a target.
    #[must_use]
    pub fn new(target: RecoveryTarget) -> Self {
        Self { target }
    }

    /// Returns the fixed API version required by the read adapter.
    #[must_use]
    pub const fn api_version(&self) -> &'static str {
        RECONCILIATION_API_VERSION
    }
}

/// The state of a message observed by a read-only history query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteMessageState {
    /// The current message is present and unedited.
    Present,
    /// The message has an edit history or an edit marker.
    Edited,
    /// The message is deleted or represented by a tombstone.
    Deleted,
}

/// One untrusted message observation.  No field is normalized before matching.
#[derive(Clone, Eq, PartialEq)]
pub struct ObservedMessage {
    /// Discord message snowflake.
    pub message_id: String,
    /// Channel containing the message.
    pub channel_id: String,
    /// Author ID of the message.
    pub author_id: String,
    /// Deterministic nonce observed in the message metadata/footer.
    pub nonce: String,
    /// Exact current message content.
    pub content: String,
    /// Current remote state marker.
    pub state: RemoteMessageState,
}

impl fmt::Debug for ObservedMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservedMessage")
            .field("message_id", &self.message_id)
            .field("channel_id", &self.channel_id)
            .field("author_id", &self.author_id)
            .field("nonce", &self.nonce)
            .field("content", &"[REDACTED]")
            .field("state", &self.state)
            .finish()
    }
}

impl ObservedMessage {
    /// Creates a present message observation.
    #[must_use]
    pub fn new(
        message_id: impl Into<String>,
        channel_id: impl Into<String>,
        author_id: impl Into<String>,
        nonce: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            channel_id: channel_id.into(),
            author_id: author_id.into(),
            nonce: nonce.into(),
            content: content.into(),
            state: RemoteMessageState::Present,
        }
    }

    /// Marks this observation as edited.
    #[must_use]
    pub const fn edited(mut self) -> Self {
        self.state = RemoteMessageState::Edited;
        self
    }

    /// Marks this observation as deleted or tombstoned.
    #[must_use]
    pub const fn deleted(mut self) -> Self {
        self.state = RemoteMessageState::Deleted;
        self
    }
}

/// A complete or explicitly incomplete page returned by a read adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadPage {
    /// Messages returned by the bounded destination read.
    pub messages: Vec<ObservedMessage>,
    /// Whether the read was complete and can count toward absence evidence.
    pub complete: bool,
}

impl ReadPage {
    /// Creates a complete page.
    #[must_use]
    pub fn new(messages: Vec<ObservedMessage>) -> Self {
        Self {
            messages,
            complete: true,
        }
    }

    /// Creates an explicitly incomplete page.
    #[must_use]
    pub fn incomplete(messages: Vec<ObservedMessage>) -> Self {
        Self {
            messages,
            complete: false,
        }
    }

    /// Creates a complete empty page.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }
}

/// Compatibility name for an observed message.
pub type RemoteMessage = ObservedMessage;
/// Compatibility name for a read page.
pub type ReadObservation = ReadPage;

/// A redacted read failure.  A failed or incomplete read is never absence
/// evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// The bot lacks permission to read the configured destination.
    Unauthorized,
    /// Discord rate-limited the read.
    RateLimited,
    /// The bounded read timed out.
    Timeout,
    /// The read ended before a complete page was available.
    Incomplete,
    /// The transport failed before a response was available.
    Transport,
    /// The response did not satisfy the minimal read contract.
    InvalidResponse,
}

impl ReadError {
    /// Returns a stable, redacted code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "reconciliation-read-unauthorized",
            Self::RateLimited => "reconciliation-read-rate-limited",
            Self::Timeout => "reconciliation-read-timeout",
            Self::Incomplete => "reconciliation-read-incomplete",
            Self::Transport => "reconciliation-read-transport",
            Self::InvalidResponse => "reconciliation-read-invalid-response",
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "the configured destination could not be read: unauthorized",
            Self::RateLimited => "the configured destination read was rate limited",
            Self::Timeout => "the configured destination read timed out",
            Self::Incomplete => "the configured destination read was incomplete",
            Self::Transport => "the configured destination read failed in transport",
            Self::InvalidResponse => "the configured destination read returned invalid data",
        })
    }
}

impl Error for ReadError {}

/// A read-only destination history operation.
///
/// This trait intentionally has one operation.  It has no create, edit,
/// delete, reaction, permission, or other remote mutation method.
pub trait ReconciliationReader {
    /// Reads only the configured destination requested by `request`.
    fn read_destination(&mut self, request: &ReconciliationRequest) -> Result<ReadPage, ReadError>;

    /// Compatibility alias for read adapters that name the operation `read`.
    fn read(&mut self, request: &ReconciliationRequest) -> Result<ReadPage, ReadError> {
        self.read_destination(request)
    }
}

/// Compatibility name for the read-only history interface.
pub use ReconciliationReader as ReadOnlyMessageReader;

/// Durable-in-memory evidence counters for one unknown attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationProgress {
    /// Time from which the five-minute observation window is measured.
    pub observation_started_at_unix_seconds: u64,
    /// Number of complete successful reads with no match.
    pub successful_reads: u32,
    /// Timestamp of the most recent complete successful read.
    pub last_successful_read_at_unix_seconds: Option<u64>,
}

impl ReconciliationProgress {
    /// Creates progress anchored to the target's unknown transition time.
    #[must_use]
    pub fn for_target(target: &RecoveryTarget) -> Self {
        Self {
            observation_started_at_unix_seconds: target.unknown_since_unix_seconds,
            successful_reads: 0,
            last_successful_read_at_unix_seconds: None,
        }
    }

    /// Creates progress anchored to an explicit observation time.
    #[must_use]
    pub const fn new(observation_started_at_unix_seconds: u64) -> Self {
        Self {
            observation_started_at_unix_seconds,
            successful_reads: 0,
            last_successful_read_at_unix_seconds: None,
        }
    }

    /// Returns whether both absence gates are currently satisfied.
    #[must_use]
    pub fn can_prove_absence(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds.saturating_sub(self.observation_started_at_unix_seconds)
            >= ABSENCE_OBSERVATION_WINDOW.as_secs()
            && self.successful_reads >= MIN_SUCCESSFUL_READS
    }
}

/// A precise reason for a reconciliation result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationReason {
    /// A complete read found no qualifying candidate yet.
    NoMatchYet,
    /// More than one exact qualifying message was observed.
    MultipleMatches,
    /// A qualifying identity carried different content.
    ConflictingContent,
    /// A qualifying identity was edited.
    EditedMatch,
    /// A qualifying identity was deleted or tombstoned.
    DeletedMatch,
    /// The read did not complete successfully.
    ReadFailed { code: String },
    /// The adapter explicitly reported an incomplete page.
    IncompleteRead,
    /// A candidate lacked a valid Discord message identity.
    InvalidObservation,
}

impl ReconciliationReason {
    /// Returns a stable reason code.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::NoMatchYet => "no-match-yet",
            Self::MultipleMatches => "multiple-matches",
            Self::ConflictingContent => "conflicting-content",
            Self::EditedMatch => "edited-match",
            Self::DeletedMatch => "deleted-match",
            Self::ReadFailed { code } => code,
            Self::IncompleteRead => "incomplete-read",
            Self::InvalidObservation => "invalid-observation",
        }
    }

    /// Returns whether this reason is a permanent conflicting-evidence block.
    #[must_use]
    pub const fn is_conflict(&self) -> bool {
        matches!(
            self,
            Self::MultipleMatches
                | Self::ConflictingContent
                | Self::EditedMatch
                | Self::DeletedMatch
                | Self::InvalidObservation
        )
    }
}

/// Result of one reconciliation observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationDecision {
    /// Exactly one matching message was found.
    Accepted {
        /// Known Discord message ID.
        message_id: String,
        /// Number of successful no-match reads recorded before this match.
        successful_reads: u32,
        /// Observation timestamp in Unix seconds.
        observed_at_unix_seconds: u64,
    },
    /// No match is present, but the absence gate is not yet satisfied.
    Unknown {
        /// Complete successful reads accumulated so far.
        successful_reads: u32,
        /// Observation window start.
        observation_started_at_unix_seconds: u64,
        /// Most recent complete read timestamp.
        last_successful_read_at_unix_seconds: Option<u64>,
        /// Why the result remains unknown.
        reason: ReconciliationReason,
    },
    /// Both conservative absence gates were satisfied.
    ReconciledAbsent {
        /// Complete successful reads proving absence.
        successful_reads: u32,
        /// Observation window start.
        observation_started_at_unix_seconds: u64,
        /// Timestamp at which the gate was satisfied.
        observed_at_unix_seconds: u64,
    },
    /// Evidence conflicts or is insufficient to make a safe decision.
    Unresolved {
        /// Conflict or insufficient-evidence reason.
        reason: ReconciliationReason,
        /// Successful reads that remain valid for later observation.
        successful_reads: u32,
    },
}

impl ReconciliationDecision {
    /// Returns the exact delivery state represented by this result.
    #[must_use]
    pub const fn delivery_state(&self) -> DeliveryState {
        match self {
            Self::Accepted { .. } => DeliveryState::ReconciledAccepted,
            Self::Unknown { .. } => DeliveryState::Unknown,
            Self::ReconciledAbsent { .. } => DeliveryState::ReconciledAbsent,
            Self::Unresolved { .. } => DeliveryState::Unresolved,
        }
    }

    /// Reconciliation never manufactures authorization for a new send.
    #[must_use]
    pub const fn permits_automatic_resend(&self) -> bool {
        false
    }

    /// Returns the number of successful no-match reads in the result.
    #[must_use]
    pub const fn successful_reads(&self) -> u32 {
        match self {
            Self::Accepted {
                successful_reads, ..
            }
            | Self::Unknown {
                successful_reads, ..
            }
            | Self::ReconciledAbsent {
                successful_reads, ..
            }
            | Self::Unresolved {
                successful_reads, ..
            } => *successful_reads,
        }
    }

    /// Returns whether the result is an unresolved conflict.
    #[must_use]
    pub const fn is_unresolved(&self) -> bool {
        matches!(self, Self::Unresolved { .. })
    }
}

/// A local reconciliation state/input error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationError {
    /// A target field was empty or unsafe.
    InvalidTarget(&'static str),
    /// The reconciler was called with a different target after it started.
    StaleTarget,
    /// The injected clock moved backwards.
    TimeWentBackwards,
    /// Recovery was entered for a state other than `unknown`.
    InvalidState(DeliveryState),
}

impl fmt::Display for ReconciliationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget(field) => {
                write!(formatter, "invalid reconciliation target: {field}")
            }
            Self::StaleTarget => {
                formatter.write_str("reconciliation target changed after observation")
            }
            Self::TimeWentBackwards => formatter.write_str("reconciliation clock moved backwards"),
            Self::InvalidState(state) => {
                write!(
                    formatter,
                    "reconciliation requires unknown state, found {state}"
                )
            }
        }
    }
}

impl Error for ReconciliationError {}

/// Classifies one complete or incomplete page without performing I/O.
///
/// This pure entry point is useful for adapters and tests.  It increments the
/// successful-read counter only for a complete page with no qualifying
/// conflict.  A match, conflict, or incomplete page never counts as absence
/// evidence.
pub fn classify_observation(
    target: &RecoveryTarget,
    page: &ReadPage,
    now_unix_seconds: u64,
    progress: &mut ReconciliationProgress,
) -> Result<ReconciliationDecision, ReconciliationError> {
    if now_unix_seconds < target.unknown_since_unix_seconds
        || now_unix_seconds < progress.observation_started_at_unix_seconds
    {
        return Err(ReconciliationError::TimeWentBackwards);
    }
    if !page.complete {
        return Ok(ReconciliationDecision::Unresolved {
            reason: ReconciliationReason::IncompleteRead,
            successful_reads: progress.successful_reads,
        });
    }

    let mut exact_matches = Vec::new();
    let mut deleted_match = false;
    let mut edited_match = false;
    let mut conflicting_content = false;
    let mut invalid_observation = false;
    for message in &page.messages {
        if !is_identity_candidate(target, message) {
            continue;
        }
        if !valid_component(&message.message_id, 128) {
            invalid_observation = true;
            continue;
        }
        match message.state {
            RemoteMessageState::Deleted => deleted_match = true,
            RemoteMessageState::Edited => edited_match = true,
            RemoteMessageState::Present if message.content != target.exact_content => {
                conflicting_content = true;
            }
            RemoteMessageState::Present => exact_matches.push(message.message_id.clone()),
        }
    }

    let conflict = if deleted_match {
        Some(ReconciliationReason::DeletedMatch)
    } else if edited_match {
        Some(ReconciliationReason::EditedMatch)
    } else if conflicting_content {
        Some(ReconciliationReason::ConflictingContent)
    } else if invalid_observation {
        Some(ReconciliationReason::InvalidObservation)
    } else {
        None
    };
    if let Some(reason) = conflict {
        return Ok(ReconciliationDecision::Unresolved {
            reason,
            successful_reads: progress.successful_reads,
        });
    }
    if exact_matches.len() > 1 {
        return Ok(ReconciliationDecision::Unresolved {
            reason: ReconciliationReason::MultipleMatches,
            successful_reads: progress.successful_reads,
        });
    }
    if let Some(message_id) = exact_matches.pop() {
        return Ok(ReconciliationDecision::Accepted {
            message_id,
            successful_reads: progress.successful_reads,
            observed_at_unix_seconds: now_unix_seconds,
        });
    }

    progress.successful_reads = progress.successful_reads.saturating_add(1);
    progress.last_successful_read_at_unix_seconds = Some(now_unix_seconds);
    if progress.can_prove_absence(now_unix_seconds) {
        Ok(ReconciliationDecision::ReconciledAbsent {
            successful_reads: progress.successful_reads,
            observation_started_at_unix_seconds: progress.observation_started_at_unix_seconds,
            observed_at_unix_seconds: now_unix_seconds,
        })
    } else {
        Ok(ReconciliationDecision::Unknown {
            successful_reads: progress.successful_reads,
            observation_started_at_unix_seconds: progress.observation_started_at_unix_seconds,
            last_successful_read_at_unix_seconds: progress.last_successful_read_at_unix_seconds,
            reason: ReconciliationReason::NoMatchYet,
        })
    }
}

/// Returns whether one observation satisfies all four exact-match predicates.
#[must_use]
pub fn is_exact_match(target: &RecoveryTarget, message: &ObservedMessage) -> bool {
    is_identity_candidate(target, message)
        && valid_component(&message.message_id, 128)
        && message.state == RemoteMessageState::Present
        && message.content == target.exact_content
}

fn is_identity_candidate(target: &RecoveryTarget, message: &ObservedMessage) -> bool {
    message.channel_id == target.channel_id
        && message.author_id == target.bot_author_id
        && message.nonce == target.nonce
}

fn valid_component(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

/// Stateful read-only reconciler for one exact unknown target.
pub struct RecoveryReconciler<R, C> {
    reader: R,
    clock: C,
    target: Option<RecoveryTarget>,
    progress: Option<ReconciliationProgress>,
    terminal: Option<ReconciliationDecision>,
}

impl<R, C> fmt::Debug for RecoveryReconciler<R, C>
where
    R: fmt::Debug,
    C: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryReconciler")
            .field("reader", &self.reader)
            .field("clock", &self.clock)
            .field("target", &self.target)
            .field("progress", &self.progress)
            .field("terminal", &self.terminal)
            .finish()
    }
}

impl<R, C> RecoveryReconciler<R, C> {
    /// Creates a reconciler with an injected reader and clock.
    pub const fn new(reader: R, clock: C) -> Self {
        Self {
            reader,
            clock,
            target: None,
            progress: None,
            terminal: None,
        }
    }

    /// Returns read-only access to the injected reader.
    #[must_use]
    pub const fn reader(&self) -> &R {
        &self.reader
    }

    /// Returns the current observation progress, if a target has started.
    #[must_use]
    pub const fn progress(&self) -> Option<&ReconciliationProgress> {
        self.progress.as_ref()
    }

    /// Returns the target currently being reconciled.
    #[must_use]
    pub const fn target(&self) -> Option<&RecoveryTarget> {
        self.target.as_ref()
    }

    /// Consumes the reconciler and returns its reader and clock.
    #[must_use]
    pub fn into_parts(self) -> (R, C) {
        (self.reader, self.clock)
    }
}

impl<R, C> RecoveryReconciler<R, C>
where
    R: ReconciliationReader,
    C: Clock,
{
    /// Performs one read-only observation for an exact unknown target.
    ///
    /// Read errors and incomplete pages return `Unresolved` without incrementing
    /// successful-read evidence.  A conflict is terminal for this reconciler;
    /// no later read can turn conflicting evidence into an automatic resend.
    pub fn reconcile(
        &mut self,
        target: &RecoveryTarget,
    ) -> Result<ReconciliationDecision, ReconciliationError> {
        target.validate()?;
        if let Some(existing) = &self.target {
            if existing != target {
                return Err(ReconciliationError::StaleTarget);
            }
        } else {
            self.target = Some(target.clone());
            self.progress = Some(ReconciliationProgress::for_target(target));
        }
        if let Some(terminal) = &self.terminal {
            return Ok(terminal.clone());
        }
        let now = self.clock.now_unix_seconds();
        let request = target.request();
        let page = match self.reader.read_destination(&request) {
            Ok(page) => page,
            Err(error) => {
                return Ok(ReconciliationDecision::Unresolved {
                    reason: ReconciliationReason::ReadFailed {
                        code: error.code().to_owned(),
                    },
                    successful_reads: self
                        .progress
                        .as_ref()
                        .map_or(0, |progress| progress.successful_reads),
                });
            }
        };
        let progress = self
            .progress
            .as_mut()
            .ok_or(ReconciliationError::StaleTarget)?;
        let decision = classify_observation(target, &page, now, progress)?;
        if should_cache(&decision) {
            self.terminal = Some(decision.clone());
        }
        Ok(decision)
    }

    /// Reconciles a durable attempt only when it is still `unknown`.
    pub fn reconcile_attempt(
        &mut self,
        attempt: &DeliveryAttempt,
        bot_author_id: impl Into<String>,
    ) -> Result<ReconciliationDecision, ReconciliationError> {
        let now = self.clock.now_unix_seconds();
        self.reconcile_attempt_at(attempt, bot_author_id, now)
    }

    /// Variant of [`Self::reconcile_attempt`] with an explicit unknown-start
    /// timestamp for deterministic observation-window tests.
    pub fn reconcile_attempt_at(
        &mut self,
        attempt: &DeliveryAttempt,
        bot_author_id: impl Into<String>,
        unknown_since_unix_seconds: u64,
    ) -> Result<ReconciliationDecision, ReconciliationError> {
        if attempt.state != DeliveryState::Unknown {
            return Err(ReconciliationError::InvalidState(attempt.state));
        }
        let target =
            RecoveryTarget::from_attempt(attempt, bot_author_id, unknown_since_unix_seconds)?;
        self.reconcile(&target)
    }
}

fn should_cache(decision: &ReconciliationDecision) -> bool {
    match decision {
        ReconciliationDecision::Accepted { .. }
        | ReconciliationDecision::ReconciledAbsent { .. } => true,
        ReconciliationDecision::Unresolved { reason, .. } => reason.is_conflict(),
        ReconciliationDecision::Unknown { .. } => false,
    }
}

/// Compatibility name for [`RecoveryReconciler`].
pub type Reconciler<R, C> = RecoveryReconciler<R, C>;

#[cfg(test)]
mod tests {
    use super::{
        ObservedMessage, ReadPage, ReconciliationDecision, ReconciliationProgress,
        ReconciliationReason, RecoveryTarget,
    };

    fn target() -> RecoveryTarget {
        RecoveryTarget::new(
            "release",
            "100",
            "200",
            "300",
            "nonce",
            "body\nnonce: nonce",
            1_000,
        )
        .expect("target")
    }

    #[test]
    fn all_four_predicates_are_required() {
        let target = target();
        let message = ObservedMessage::new("400", "200", "300", "nonce", "body\nnonce: nonce");
        assert!(super::is_exact_match(&target, &message));
        let mut wrong_channel = message.clone();
        wrong_channel.channel_id = "201".to_owned();
        assert!(!super::is_exact_match(&target, &wrong_channel));
    }

    #[test]
    fn absence_needs_time_and_three_reads() {
        let target = target();
        let mut progress = ReconciliationProgress::for_target(&target);
        let first = super::classify_observation(&target, &ReadPage::empty(), 1_300, &mut progress)
            .expect("first read");
        assert!(matches!(first, ReconciliationDecision::Unknown { .. }));
        let _ = super::classify_observation(&target, &ReadPage::empty(), 1_300, &mut progress)
            .expect("second read");
        let _ = super::classify_observation(&target, &ReadPage::empty(), 1_300, &mut progress)
            .expect("third read");
        let absent = super::classify_observation(&target, &ReadPage::empty(), 1_300, &mut progress)
            .expect("fourth read");
        assert!(matches!(
            absent,
            ReconciliationDecision::ReconciledAbsent {
                successful_reads: 4,
                ..
            }
        ));
        assert!(progress.can_prove_absence(1_300));
        assert!(!progress.can_prove_absence(1_299));
    }

    #[test]
    fn conflicts_are_unresolved() {
        let target = target();
        let mut progress = ReconciliationProgress::for_target(&target);
        let page = ReadPage::new(vec![
            ObservedMessage::new("1", "200", "300", "nonce", "different"),
            ObservedMessage::new("2", "200", "300", "nonce", "different").deleted(),
        ]);
        let decision =
            super::classify_observation(&target, &page, 2_000, &mut progress).expect("classify");
        assert_eq!(decision.successful_reads(), 0);
        assert!(matches!(
            decision,
            ReconciliationDecision::Unresolved {
                reason: ReconciliationReason::DeletedMatch,
                ..
            }
        ));
    }
}
