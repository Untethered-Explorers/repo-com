//! Bounded, read-only point checks for recently stored inbound messages.

use std::{error::Error, fmt};

use repo_com_config::{ResolvedConfig, ResolvedInbound};
use repo_com_inbox_state::{
    InboundCurrentSnapshotInput, InboundTransitionInput, InboxState, InboxStateError,
};
use serde::Serialize;

use crate::{
    boundary::{MAX_POINT_CHECKS, rfc3339_to_unix_millis},
    fetch::{DiscordInboundClient, InboundReader, ReadError},
    filter::{AttachmentIndicator, RemoteMessage, UntrustedInboundEnvelope},
};

/// A point-check result for one stored message.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PointCheckResult {
    /// Stored remote item ID.
    pub item_id: String,
    /// Configured channel used for the exact point read.
    pub channel_id: String,
    /// Observed state.
    pub state: PointCheckState,
    /// Current remote content for an edit observation.
    pub current_content: Option<String>,
    /// Current attachment indicators for an edit observation.
    pub current_attachment_indicators: Vec<AttachmentIndicator>,
    /// Local observation timestamp.
    pub observed_at: String,
    /// Stable redacted read error code when the probe did not complete.
    pub error_code: Option<String>,
}

impl fmt::Debug for PointCheckResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PointCheckResult")
            .field("item_id", &self.item_id)
            .field("channel_id", &self.channel_id)
            .field("state", &self.state)
            .field("current_content", &"[REDACTED]")
            .field(
                "current_attachment_indicators",
                &self.current_attachment_indicators,
            )
            .field("observed_at", &self.observed_at)
            .field("error_code", &self.error_code)
            .finish()
    }
}

impl PointCheckResult {
    /// Returns whether this probe observed an edit.
    #[must_use]
    pub fn is_edit(&self) -> bool {
        matches!(self.state, PointCheckState::Edited)
    }

    /// Returns whether this probe observed a deletion.
    #[must_use]
    pub fn is_deleted(&self) -> bool {
        matches!(self.state, PointCheckState::Deleted)
    }
}

/// The state observed by one exact point read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointCheckState {
    /// The current remote projection matched the stored page projection.
    Unchanged,
    /// The message remains present with changed content or edit evidence.
    Edited,
    /// Discord returned 404 for the exact stored message.
    Deleted,
    /// The read failed without changing local state.
    Failed,
}

/// Explicit continuation when accepted items remain beyond the probe budget.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProbeContinuation {
    /// Zero-based offset to use when the remaining page is retried.
    pub next_offset: usize,
    /// IDs of the remaining recently stored items, in deterministic order.
    pub remaining_item_ids: Vec<String>,
}

impl ProbeContinuation {
    /// Returns whether another point-check pass is indicated.
    #[must_use]
    pub fn has_more(&self) -> bool {
        !self.remaining_item_ids.is_empty()
    }
}

/// Summary of one bounded reconciliation pass.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReconciliationSummary {
    /// Number of point reads attempted, never above 100.
    pub attempted: usize,
    /// Results in item order.
    pub checks: Vec<PointCheckResult>,
    /// Remaining recently stored items, if the hard probe budget stopped the pass.
    pub continuation: Option<ProbeContinuation>,
}

impl ReconciliationSummary {
    /// Returns whether another explicit reconciliation pass is indicated.
    #[must_use]
    pub fn has_continuation(&self) -> bool {
        self.continuation
            .as_ref()
            .is_some_and(ProbeContinuation::has_more)
    }

    /// Returns the number of recorded edits.
    #[must_use]
    pub fn edited_count(&self) -> usize {
        self.checks.iter().filter(|check| check.is_edit()).count()
    }

    /// Returns the number of recorded deletions.
    #[must_use]
    pub fn deleted_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.is_deleted())
            .count()
    }
}

/// Compatibility names for callers using the feature terminology.
pub type PointReadResult = PointCheckResult;
/// Compatibility name for the bounded reconciliation summary.
pub type ReconcileSummary = ReconciliationSummary;

/// A safe local/transport reconciliation error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationError {
    /// The configured inbound alias is disabled.
    DisabledAlias,
    /// A repository, channel, timestamp, or item identity was invalid.
    InvalidIdentity(&'static str),
    /// The point reader returned a non-404 read failure.
    Read(ReadError),
    /// The item was not present in local state after page commit.
    MissingItem,
    /// A local state transition could not be recorded.
    State(InboxStateError),
}

impl ReconciliationError {
    /// Returns a stable, redacted error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::DisabledAlias => "inbound-reconciliation-disabled",
            Self::InvalidIdentity(_) => "inbound-reconciliation-invalid-identity",
            Self::Read(_) => "inbound-reconciliation-read-error",
            Self::MissingItem => "inbound-reconciliation-missing-item",
            Self::State(_) => "inbound-reconciliation-storage-error",
        }
    }
}

impl fmt::Display for ReconciliationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisabledAlias => formatter.write_str("the inbound alias is disabled"),
            Self::InvalidIdentity(field) => {
                write!(formatter, "reconciliation identity is invalid: {field}")
            }
            Self::Read(error) => write!(formatter, "inbound point read failed: {error}"),
            Self::MissingItem => formatter.write_str("a point-check item is not stored locally"),
            Self::State(error) => write!(formatter, "inbound point state write failed: {error}"),
        }
    }
}

impl Error for ReconciliationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ReadError> for ReconciliationError {
    fn from(error: ReadError) -> Self {
        Self::Read(error)
    }
}

impl From<InboxStateError> for ReconciliationError {
    fn from(error: InboxStateError) -> Self {
        Self::State(error)
    }
}

/// Performs the bounded point-check pass for one stored page using the
/// concrete loopback/production Discord reader.
///
/// The alias is resolved through the current repository configuration. This
/// public entry point therefore cannot be used as an arbitrary-channel read.
pub async fn reconcile_page(
    client: &DiscordInboundClient,
    state: &mut InboxState,
    config: &ResolvedConfig,
    alias: &str,
    items: &[UntrustedInboundEnvelope],
    observed_at: impl Into<String>,
) -> Result<ReconciliationSummary, ReconciliationError> {
    let repository_id = config.config.repository_id.as_str();
    if !safe_component(repository_id) || !safe_component(alias) {
        return Err(ReconciliationError::InvalidIdentity("scope"));
    }
    let inbound = config
        .inbound(alias)
        .ok_or(ReconciliationError::InvalidIdentity("alias"))?;
    if !inbound.enabled {
        return Err(ReconciliationError::DisabledAlias);
    }
    if inbound.alias != alias {
        return Err(ReconciliationError::InvalidIdentity("alias"));
    }
    if inbound.workspace_id != config.config.discord.workspace_id {
        return Err(ReconciliationError::InvalidIdentity("workspace"));
    }
    reconcile_page_with_reader(
        client,
        state,
        repository_id,
        inbound,
        items,
        &observed_at.into(),
    )
    .await
}

/// Internal generic implementation used by the concrete client and focused
/// token-free contract tests.
pub(crate) async fn reconcile_page_with_reader<R: InboundReader>(
    reader: &R,
    state: &mut InboxState,
    repository_id: &str,
    inbound: &ResolvedInbound,
    items: &[UntrustedInboundEnvelope],
    observed_at: &str,
) -> Result<ReconciliationSummary, ReconciliationError> {
    if !inbound.enabled {
        return Err(ReconciliationError::DisabledAlias);
    }
    if !safe_component(repository_id)
        || !is_discord_id(&inbound.workspace_id)
        || !is_discord_id(&inbound.channel_id)
        || rfc3339_to_unix_millis(observed_at).is_err()
    {
        return Err(ReconciliationError::InvalidIdentity("scope"));
    }

    let attempted = items.len().min(MAX_POINT_CHECKS);
    let mut checks = Vec::with_capacity(attempted);
    for envelope in items.iter().take(attempted) {
        if !safe_component(&envelope.remote_message_id)
            || envelope.channel_id != inbound.channel_id
            || envelope.repository_id != repository_id
        {
            return Err(ReconciliationError::InvalidIdentity("item"));
        }
        if state
            .item(repository_id, &envelope.remote_message_id)?
            .is_none()
        {
            return Err(ReconciliationError::MissingItem);
        }

        match reader
            .read_message(&inbound.channel_id, &envelope.remote_message_id)
            .await
        {
            Ok(remote) => {
                let check =
                    record_present_probe(state, repository_id, envelope, remote, observed_at)?;
                checks.push(check);
            }
            Err(ReadError::NotFound) => {
                let check = record_deleted_probe(state, repository_id, envelope, observed_at)?;
                checks.push(check);
            }
            Err(error) if is_terminal_point_error(&error) => {
                return Err(ReconciliationError::Read(error));
            }
            Err(error) => {
                checks.push(PointCheckResult {
                    item_id: envelope.remote_message_id.clone(),
                    channel_id: inbound.channel_id.clone(),
                    state: PointCheckState::Failed,
                    current_content: None,
                    current_attachment_indicators: Vec::new(),
                    observed_at: observed_at.to_owned(),
                    error_code: Some(error.code().to_owned()),
                });
            }
        }
    }

    let continuation = (items.len() > attempted).then(|| ProbeContinuation {
        next_offset: attempted,
        remaining_item_ids: items[attempted..]
            .iter()
            .map(|item| item.remote_message_id.clone())
            .collect(),
    });
    Ok(ReconciliationSummary {
        attempted,
        checks,
        continuation,
    })
}

fn is_terminal_point_error(error: &ReadError) -> bool {
    matches!(
        error,
        ReadError::MissingBotToken
            | ReadError::InvalidBotToken
            | ReadError::InvalidEndpoint
            | ReadError::ClientBuild
            | ReadError::Authentication
            | ReadError::PermissionDenied
            | ReadError::ValidationRejected
            | ReadError::Conflict
            | ReadError::UnexpectedStatus { .. }
            | ReadError::InvalidResponse
    )
}

fn record_present_probe(
    state: &mut InboxState,
    repository_id: &str,
    envelope: &UntrustedInboundEnvelope,
    remote: RemoteMessage,
    observed_at: &str,
) -> Result<PointCheckResult, ReconciliationError> {
    if remote.channel_id != envelope.channel_id || remote.id != envelope.remote_message_id {
        return Err(ReconciliationError::InvalidIdentity("point read"));
    }
    let edited = remote.content != envelope.text
        || remote.attachments != envelope.attachment_indicators
        || remote.edited_timestamp.is_some();
    if !edited {
        return Ok(PointCheckResult {
            item_id: envelope.remote_message_id.clone(),
            channel_id: envelope.channel_id.clone(),
            state: PointCheckState::Unchanged,
            current_content: None,
            current_attachment_indicators: Vec::new(),
            observed_at: observed_at.to_owned(),
            error_code: None,
        });
    }

    let attachment_json = serde_json::to_string(&remote.attachments)
        .map_err(|_| ReconciliationError::InvalidIdentity("attachments"))?;
    let transition = InboundTransitionInput::new(
        repository_id,
        format!(
            "probe:{}:edited:{:x}",
            envelope.remote_message_id,
            stable_probe_hash(&remote, observed_at)
        ),
        envelope.remote_message_id.clone(),
        "edited",
        Some(remote.content.clone()),
        observed_at,
    );
    let current = InboundCurrentSnapshotInput::new(
        repository_id,
        envelope.remote_message_id.clone(),
        Some(remote.content.clone()),
        false,
        observed_at,
    );
    let mut current = current;
    current.current_attachments_json = attachment_json;
    state.record_transition(&transition, Some(&current))?;
    Ok(PointCheckResult {
        item_id: envelope.remote_message_id.clone(),
        channel_id: envelope.channel_id.clone(),
        state: PointCheckState::Edited,
        current_content: Some(remote.content),
        current_attachment_indicators: remote.attachments,
        observed_at: observed_at.to_owned(),
        error_code: None,
    })
}

fn record_deleted_probe(
    state: &mut InboxState,
    repository_id: &str,
    envelope: &UntrustedInboundEnvelope,
    observed_at: &str,
) -> Result<PointCheckResult, ReconciliationError> {
    let transition = InboundTransitionInput::new(
        repository_id,
        format!(
            "probe:{}:deleted:{:x}",
            envelope.remote_message_id,
            stable_string_hash(observed_at)
        ),
        envelope.remote_message_id.clone(),
        "deleted",
        None,
        observed_at,
    );
    state.record_transition(&transition, None)?;
    Ok(PointCheckResult {
        item_id: envelope.remote_message_id.clone(),
        channel_id: envelope.channel_id.clone(),
        state: PointCheckState::Deleted,
        current_content: None,
        current_attachment_indicators: Vec::new(),
        observed_at: observed_at.to_owned(),
        error_code: None,
    })
}

fn stable_probe_hash(message: &RemoteMessage, observed_at: &str) -> u64 {
    let attachment_ids = message
        .attachments
        .iter()
        .map(|attachment| attachment.id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut hash = 0xcbf29ce484222325_u64;
    for part in [
        message.content.as_str(),
        message.edited_timestamp.as_deref().unwrap_or(""),
        attachment_ids.as_str(),
        observed_at,
    ] {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn stable_string_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn safe_component(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::PointCheckState;

    #[test]
    fn point_state_spellings_are_stable() {
        assert_eq!(format!("{:?}", PointCheckState::Unchanged), "Unchanged");
        assert_eq!(format!("{:?}", PointCheckState::Edited), "Edited");
        assert_eq!(format!("{:?}", PointCheckState::Deleted), "Deleted");
    }
}
