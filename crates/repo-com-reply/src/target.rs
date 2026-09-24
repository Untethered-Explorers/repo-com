//! Repository-scoped validation of retained inbound reply targets.

use std::{error::Error, fmt};

use repo_com_config::ResolvedConfig;
use repo_com_draft_model::DraftModel;
use repo_com_inbox_state::{
    InboundCurrentSnapshotRecord, InboundItemRecord, InboxState, InboxStateError,
};
use repo_com_send_eligibility::{EligibilityBlocker, EligibilityDecision};
use repo_com_state::StateError;
use serde::Serialize;
use sha2::{Digest, Sha256};

const SECONDS_PER_DAY: u64 = 86_400;
const MAX_REPOSITORY_ID_BYTES: usize = 256;
const MAX_LOCAL_ID_BYTES: usize = 128;
const MAX_DISCORD_ID_BYTES: usize = 32;

/// A side-effect-free, repository-scoped target lookup request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyTargetRequest {
    repository_id: String,
    inbound_item_id: String,
    now_unix_seconds: u64,
}

impl ReplyTargetRequest {
    /// Creates one request. The item ID is only a lookup key; a Discord-shaped
    /// ID is not authorized until an exact retained local item is found.
    pub fn new(
        repository_id: impl Into<String>,
        inbound_item_id: impl Into<String>,
        now_unix_seconds: u64,
    ) -> Result<Self, ReplyTargetError> {
        let repository_id = repository_id.into();
        let inbound_item_id = inbound_item_id.into();
        if !is_valid_repository_id(&repository_id) || !is_discord_id(&inbound_item_id) {
            return Err(ReplyTargetError::InvalidRequest);
        }
        Ok(Self {
            repository_id,
            inbound_item_id,
            now_unix_seconds,
        })
    }

    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn inbound_item_id(&self) -> &str {
        &self.inbound_item_id
    }

    #[must_use]
    pub const fn now_unix_seconds(&self) -> u64 {
        self.now_unix_seconds
    }
}

/// One fully validated, deterministic target for a Discord reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyTarget {
    repository_id: String,
    workspace_id: String,
    destination_alias: String,
    channel_id: String,
    inbound_item_id: String,
    authorization_reference: String,
    validated_snapshot_hash: String,
    config_hash: String,
    target_expires_at_unix_seconds: u64,
    validated_at_unix_seconds: u64,
}

impl ReplyTarget {
    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    #[must_use]
    pub fn destination_alias(&self) -> &str {
        &self.destination_alias
    }

    #[must_use]
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    #[must_use]
    pub fn inbound_item_id(&self) -> &str {
        &self.inbound_item_id
    }

    #[must_use]
    pub fn authorization_reference(&self) -> &str {
        &self.authorization_reference
    }

    #[must_use]
    pub fn validated_snapshot_hash(&self) -> &str {
        &self.validated_snapshot_hash
    }

    #[must_use]
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    #[must_use]
    pub const fn target_expires_at_unix_seconds(&self) -> u64 {
        self.target_expires_at_unix_seconds
    }

    #[must_use]
    pub const fn validated_at_unix_seconds(&self) -> u64 {
        self.validated_at_unix_seconds
    }

    pub(crate) fn ensure_usable_for_draft(
        &self,
        config: &ResolvedConfig,
        now_unix_seconds: u64,
    ) -> Result<(), ReplyTargetError> {
        if self.repository_id != config.config.repository_id {
            return Err(ReplyTargetError::RepositoryMismatch);
        }
        if self.workspace_id != config.config.discord.workspace_id {
            return Err(ReplyTargetError::WorkspaceMismatch);
        }
        if self.config_hash != config.canonical_hash() {
            return Err(ReplyTargetError::AuthorizationChanged);
        }
        let destination = config
            .destination(&self.destination_alias)
            .ok_or(ReplyTargetError::ChannelNotConfigured)?;
        if destination.alias != self.destination_alias
            || destination.workspace_id != self.workspace_id
            || destination.channel_id != self.channel_id
        {
            return Err(ReplyTargetError::AuthorizationChanged);
        }
        if now_unix_seconds >= self.target_expires_at_unix_seconds {
            return Err(ReplyTargetError::TargetExpired);
        }
        Ok(())
    }

    pub(crate) fn ensure_current_records(
        &self,
        item: &InboundItemRecord,
        current: &InboundCurrentSnapshotRecord,
    ) -> Result<(), ReplyTargetError> {
        if item.repository_id != self.repository_id
            || item.item_id != self.inbound_item_id
            || item.channel_id != self.channel_id
            || current.repository_id != self.repository_id
            || current.item_id != self.inbound_item_id
        {
            return Err(ReplyTargetError::TargetChanged);
        }
        if current.deleted {
            return Err(ReplyTargetError::TargetDeleted);
        }
        if current.current_content.is_none() || parse_rfc3339_utc(&current.observed_at).is_none() {
            return Err(ReplyTargetError::InvalidStoredSnapshot);
        }
        if self.snapshot_hash(item, current) != self.validated_snapshot_hash {
            return Err(ReplyTargetError::TargetChanged);
        }
        Ok(())
    }

    fn same_snapshot(&self, other: &Self) -> bool {
        self.repository_id == other.repository_id
            && self.workspace_id == other.workspace_id
            && self.destination_alias == other.destination_alias
            && self.channel_id == other.channel_id
            && self.inbound_item_id == other.inbound_item_id
            && self.authorization_reference == other.authorization_reference
            && self.validated_snapshot_hash == other.validated_snapshot_hash
            && self.config_hash == other.config_hash
            && self.target_expires_at_unix_seconds == other.target_expires_at_unix_seconds
    }

    fn snapshot_hash(
        &self,
        item: &InboundItemRecord,
        current: &InboundCurrentSnapshotRecord,
    ) -> String {
        let input = SnapshotHashInput {
            schema_version: 1,
            repository_id: &self.repository_id,
            workspace_id: &self.workspace_id,
            destination_alias: &self.destination_alias,
            channel_id: &self.channel_id,
            inbound_item_id: &self.inbound_item_id,
            author_id: &item.author_id,
            authorization_reference: &self.authorization_reference,
            config_hash: &self.config_hash,
            first_observed_at: &item.first_observed_at,
            first_content_hash: sha256_text(&item.first_content),
            first_attachments_hash: sha256_text(&item.first_attachments_json),
            current_content_hash: current
                .current_content
                .as_deref()
                .map(sha256_text)
                .unwrap_or_else(|| sha256_text("")),
            current_attachments_hash: sha256_text(&current.current_attachments_json),
            deleted: current.deleted,
        };
        sha256_json(&input)
    }
}

/// Stateless validator over the current local state and resolved configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplyTargetValidator;

impl ReplyTargetValidator {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Resolves exactly one enabled configured inbound alias for one retained item.
    pub fn validate(
        &self,
        state: &InboxState,
        config: &ResolvedConfig,
        request: &ReplyTargetRequest,
    ) -> Result<ReplyTarget, ReplyTargetError> {
        if request.repository_id != config.config.repository_id {
            return Err(ReplyTargetError::RepositoryMismatch);
        }
        let repository = state
            .state()
            .require_repository(&request.repository_id)
            .map_err(ReplyTargetError::State)?;
        if repository.workspace_id != config.config.discord.workspace_id {
            return Err(ReplyTargetError::WorkspaceMismatch);
        }
        if repository.config_hash != config.canonical_hash() {
            return Err(ReplyTargetError::AuthorizationDenied);
        }

        let item = state
            .item(&request.repository_id, &request.inbound_item_id)
            .map_err(map_inbox_error)?
            .ok_or(ReplyTargetError::MissingTarget)?;
        if item.repository_id != request.repository_id
            || item.item_id != request.inbound_item_id
            || !is_discord_id(&item.item_id)
            || !is_discord_id(&item.channel_id)
            || !is_valid_local_id(&item.author_id)
        {
            return Err(ReplyTargetError::InvalidStoredIdentity);
        }

        let mut matching = config
            .inbound
            .values()
            .filter(|candidate| candidate.channel_id == item.channel_id);
        let Some(inbound) = matching.next() else {
            return Err(ReplyTargetError::ChannelNotConfigured);
        };
        if matching.next().is_some() {
            return Err(ReplyTargetError::AmbiguousInboundAlias);
        }
        if !inbound.enabled {
            return Err(ReplyTargetError::AuthorizationDenied);
        }
        if inbound.workspace_id != config.config.discord.workspace_id
            || !is_discord_id(&inbound.workspace_id)
            || !is_discord_id(&inbound.channel_id)
        {
            return Err(ReplyTargetError::InvalidStoredIdentity);
        }
        let destination = config
            .destination(&inbound.alias)
            .ok_or(ReplyTargetError::ChannelNotConfigured)?;
        if destination.alias != inbound.alias
            || destination.workspace_id != inbound.workspace_id
            || destination.channel_id != inbound.channel_id
        {
            return Err(ReplyTargetError::AuthorizationChanged);
        }

        let first_observed = parse_rfc3339_utc(&item.first_observed_at)
            .ok_or(ReplyTargetError::InvalidStoredSnapshot)?;
        let current = state
            .current(&request.repository_id, &request.inbound_item_id)
            .map_err(map_inbox_error)?
            .ok_or(ReplyTargetError::CurrentSnapshotMissing)?;
        if current.repository_id != item.repository_id || current.item_id != item.item_id {
            return Err(ReplyTargetError::InvalidStoredSnapshot);
        }
        if current.deleted {
            return Err(ReplyTargetError::TargetDeleted);
        }
        if current.current_content.is_none() {
            return Err(ReplyTargetError::InvalidStoredSnapshot);
        }
        parse_rfc3339_utc(&current.observed_at).ok_or(ReplyTargetError::InvalidStoredSnapshot)?;

        let retention_days = u64::from(config.config.retention.content_days);
        let lifetime = retention_days
            .checked_mul(SECONDS_PER_DAY)
            .ok_or(ReplyTargetError::InvalidStoredSnapshot)?;
        let target_expires_at_unix_seconds = first_observed
            .checked_add(lifetime)
            .ok_or(ReplyTargetError::InvalidStoredSnapshot)?;
        if request.now_unix_seconds >= target_expires_at_unix_seconds {
            return Err(ReplyTargetError::TargetExpired);
        }

        let config_hash = config.canonical_hash();
        let authorization_reference = format!("inbound-{}", inbound.alias);
        if !is_valid_local_id(&authorization_reference) {
            return Err(ReplyTargetError::InvalidRequest);
        }
        let mut target = ReplyTarget {
            repository_id: request.repository_id.clone(),
            workspace_id: inbound.workspace_id.clone(),
            destination_alias: inbound.alias.clone(),
            channel_id: inbound.channel_id.clone(),
            inbound_item_id: request.inbound_item_id.clone(),
            authorization_reference,
            validated_snapshot_hash: String::new(),
            config_hash,
            target_expires_at_unix_seconds,
            validated_at_unix_seconds: request.now_unix_seconds,
        };
        target.validated_snapshot_hash = target.snapshot_hash(&item, &current);
        Ok(target)
    }

    /// Revalidates the retained target and persisted reply revision before the
    /// caller evaluates normal approval, policy, and safety eligibility.
    pub fn revalidate_before_eligibility(
        &self,
        state: &InboxState,
        config: &ResolvedConfig,
        target: &ReplyTarget,
        draft: &DraftModel,
        now_unix_seconds: u64,
    ) -> Result<ReplyTargetRevalidation, ReplyTargetError> {
        self.revalidate_draft_target(state, config, target, draft, now_unix_seconds)
    }

    /// Repeats target and persisted-draft checks immediately before a separate
    /// delivery claim. This method never creates or consumes a claim.
    pub fn revalidate_for_delivery(
        &self,
        state: &InboxState,
        config: &ResolvedConfig,
        target: &ReplyTarget,
        draft: &DraftModel,
        decision: &EligibilityDecision,
        now_unix_seconds: u64,
    ) -> Result<ReplyTargetRevalidation, ReplyTargetError> {
        let EligibilityDecision::Eligible { revalidation, .. } = decision else {
            return Err(ReplyTargetError::EligibilityBlocked(
                decision
                    .blocker()
                    .unwrap_or(EligibilityBlocker::CurrentStateInvalid),
            ));
        };
        let revision = draft.current_revision();
        if revalidation.evaluated_at_unix_seconds > now_unix_seconds
            || revalidation.repository_id != target.repository_id
            || revalidation.workspace_id != target.workspace_id
            || revalidation.draft_id != draft.draft_id().as_str()
            || revalidation.revision != revision.number()
            || revalidation.revision_hash != revision.content_hash()
            || revalidation.destination_alias != target.destination_alias
            || revalidation.config_hash != config.canonical_hash()
            || revalidation.draft_created_at_unix_seconds
                != revision.expiry().created_at_unix_seconds()
            || revalidation.draft_expires_at_unix_seconds
                != revision.expiry().expires_at_unix_seconds()
            || now_unix_seconds >= revision.expiry().expires_at_unix_seconds()
        {
            return Err(ReplyTargetError::EligibilityFactsChanged);
        }
        self.revalidate_draft_target(state, config, target, draft, now_unix_seconds)
    }

    fn revalidate_draft_target(
        &self,
        state: &InboxState,
        config: &ResolvedConfig,
        target: &ReplyTarget,
        draft: &DraftModel,
        now_unix_seconds: u64,
    ) -> Result<ReplyTargetRevalidation, ReplyTargetError> {
        let revision = draft.current_revision();
        let reference = revision
            .reply_reference()
            .ok_or(ReplyTargetError::NotReplyDraft)?;
        if reference.repository_id() != target.repository_id
            || reference.workspace_id() != target.workspace_id
            || reference.channel_id() != target.channel_id
            || reference.inbound_item_id() != target.inbound_item_id
            || reference.message_id() != target.inbound_item_id
            || reference.authorization_reference() != target.authorization_reference
            || reference.validated_snapshot_hash() != target.validated_snapshot_hash
        {
            return Err(ReplyTargetError::TargetChanged);
        }

        target.ensure_usable_for_draft(config, now_unix_seconds)?;
        let stored_draft = state
            .state()
            .draft(&target.repository_id, draft.draft_id().as_str())
            .map_err(ReplyTargetError::State)?
            .ok_or(ReplyTargetError::PersistedDraftChanged)?;
        let stored_revision_number = i64::try_from(revision.number())
            .map_err(|_| ReplyTargetError::PersistedDraftChanged)?;
        if stored_draft.current_revision != stored_revision_number
            || stored_draft.repository_id != target.repository_id
            || stored_draft.destination_alias != target.destination_alias
            || stored_draft.reply_to_inbound_item_id.as_deref()
                != Some(target.inbound_item_id.as_str())
        {
            return Err(ReplyTargetError::PersistedDraftChanged);
        }
        let stored_revision = state
            .state()
            .draft_revision(
                &target.repository_id,
                draft.draft_id().as_str(),
                stored_revision_number,
            )
            .map_err(ReplyTargetError::State)?
            .ok_or(ReplyTargetError::PersistedDraftChanged)?;
        if stored_revision.content_hash != revision.content_hash()
            || stored_revision.destination_alias != target.destination_alias
            || stored_revision.reply_to_inbound_item_id.as_deref()
                != Some(target.inbound_item_id.as_str())
        {
            return Err(ReplyTargetError::PersistedDraftChanged);
        }

        let current = self.validate(
            state,
            config,
            &ReplyTargetRequest::new(
                &target.repository_id,
                &target.inbound_item_id,
                now_unix_seconds,
            )?,
        )?;
        if !target.same_snapshot(&current) {
            return Err(ReplyTargetError::TargetChanged);
        }

        Ok(ReplyTargetRevalidation {
            repository_id: target.repository_id.clone(),
            draft_id: draft.draft_id().as_str().to_owned(),
            revision: revision.number(),
            revision_hash: revision.content_hash().to_owned(),
            validated_snapshot_hash: target.validated_snapshot_hash.clone(),
            config_hash: config.canonical_hash(),
            validated_at_unix_seconds: now_unix_seconds,
        })
    }
}

/// Side-effect-free proof that a reply target was current at the final gate.
///
/// This is not a delivery permit and deliberately carries no network capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyTargetRevalidation {
    repository_id: String,
    draft_id: String,
    revision: u64,
    revision_hash: String,
    validated_snapshot_hash: String,
    config_hash: String,
    validated_at_unix_seconds: u64,
}

impl ReplyTargetRevalidation {
    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn revision_hash(&self) -> &str {
        &self.revision_hash
    }

    #[must_use]
    pub fn validated_snapshot_hash(&self) -> &str {
        &self.validated_snapshot_hash
    }

    #[must_use]
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    #[must_use]
    pub const fn validated_at_unix_seconds(&self) -> u64 {
        self.validated_at_unix_seconds
    }

    #[must_use]
    pub const fn requires_delivery_claim(&self) -> bool {
        true
    }

    #[must_use]
    pub const fn is_delivery_claim(&self) -> bool {
        false
    }
}

/// A safe, typed reply-target failure without inbound content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyTargetError {
    InvalidRequest,
    RepositoryMismatch,
    WorkspaceMismatch,
    AuthorizationChanged,
    MissingTarget,
    InvalidStoredIdentity,
    CurrentSnapshotMissing,
    TargetDeleted,
    TargetExpired,
    ChannelNotConfigured,
    AmbiguousInboundAlias,
    AuthorizationDenied,
    InvalidStoredSnapshot,
    TargetChanged,
    EligibilityBlocked(EligibilityBlocker),
    NotReplyDraft,
    EligibilityFactsChanged,
    PersistedDraftChanged,
    State(StateError),
}

impl ReplyTargetError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "reply-target-request-invalid",
            Self::RepositoryMismatch => "reply-repository-mismatch",
            Self::WorkspaceMismatch => "reply-workspace-mismatch",
            Self::AuthorizationChanged => "reply-authorization-changed",
            Self::MissingTarget => "reply-target-missing",
            Self::InvalidStoredIdentity => "reply-stored-identity-invalid",
            Self::CurrentSnapshotMissing => "reply-current-snapshot-missing",
            Self::TargetDeleted => "reply-target-deleted",
            Self::TargetExpired => "reply-target-expired",
            Self::ChannelNotConfigured => "reply-channel-unconfigured",
            Self::AmbiguousInboundAlias => "reply-alias-ambiguous",
            Self::AuthorizationDenied => "reply-target-unauthorized",
            Self::InvalidStoredSnapshot => "reply-stored-snapshot-invalid",
            Self::TargetChanged => "reply-target-changed",
            Self::EligibilityBlocked(_) => "reply-eligibility-blocked",
            Self::NotReplyDraft => "reply-draft-reference-missing",
            Self::EligibilityFactsChanged => "reply-eligibility-facts-changed",
            Self::PersistedDraftChanged => "reply-persisted-draft-changed",
            Self::State(_) => "reply-local-state-failed",
        }
    }
}

impl fmt::Display for ReplyTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ReplyTargetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Serialize)]
struct SnapshotHashInput<'a> {
    schema_version: u8,
    repository_id: &'a str,
    workspace_id: &'a str,
    destination_alias: &'a str,
    channel_id: &'a str,
    inbound_item_id: &'a str,
    author_id: &'a str,
    authorization_reference: &'a str,
    config_hash: &'a str,
    first_observed_at: &'a str,
    first_content_hash: String,
    first_attachments_hash: String,
    current_content_hash: String,
    current_attachments_hash: String,
    deleted: bool,
}

fn map_inbox_error(error: InboxStateError) -> ReplyTargetError {
    ReplyTargetError::State(error.into())
}

fn is_valid_repository_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REPOSITORY_ID_BYTES
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/-_.".contains(character))
}

fn is_valid_local_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LOCAL_ID_BYTES
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

pub(crate) fn is_discord_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DISCORD_ID_BYTES
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn sha256_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_json<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value)
        .expect("reply target hash input contains only serializable primitive values");
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn parse_canonical_utc(value: &str) -> Option<u64> {
    if value.len() != 20 || !value.ends_with('Z') {
        return None;
    }
    parse_rfc3339_utc(value)
}

pub(crate) fn parse_rfc3339_utc(value: &str) -> Option<u64> {
    if !value.is_ascii() || value.len() < 20 {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let suffix = &value[19..];
    let fractional_utc = suffix.len() >= 7
        && suffix.starts_with('.')
        && &suffix[suffix.len() - 6..] == "+00:00"
        && suffix[1..suffix.len() - 6]
            .bytes()
            .all(|byte| byte.is_ascii_digit());
    let valid_suffix = suffix == "Z" || suffix == "z" || suffix == "+00:00" || fractional_utc;
    if !valid_suffix {
        return None;
    }

    let year = decimal(bytes, 0, 4)?;
    let month = decimal(bytes, 5, 2)?;
    let day = decimal(bytes, 8, 2)?;
    let hour = decimal(bytes, 11, 2)?;
    let minute = decimal(bytes, 14, 2)?;
    let second = decimal(bytes, 17, 2)?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(
        i64::try_from(year).ok()?,
        i64::try_from(month).ok()?,
        i64::try_from(day).ok()?,
    );
    let seconds = days.checked_mul(i64::try_from(SECONDS_PER_DAY).ok()?)?;
    let seconds = seconds.checked_add(i64::try_from(hour * 3_600 + minute * 60 + second).ok()?)?;
    u64::try_from(seconds).ok()
}

pub(crate) fn format_rfc3339_utc(unix_seconds: u64) -> Option<String> {
    let total = i64::try_from(unix_seconds).ok()?;
    let seconds_per_day = i64::try_from(SECONDS_PER_DAY).ok()?;
    let days = total.div_euclid(seconds_per_day);
    let seconds = total.rem_euclid(seconds_per_day);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn decimal(bytes: &[u8], start: usize, length: usize) -> Option<u64> {
    let end = start.checked_add(length)?;
    let slice = bytes.get(start..end)?;
    if slice.is_empty() || !slice.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(slice).ok()?.parse().ok()
}

fn is_leap_year(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (
        u64::try_from(year).expect("supported RFC 3339 years are positive"),
        u64::try_from(month).expect("civil month is positive"),
        u64::try_from(day).expect("civil day is positive"),
    )
}
