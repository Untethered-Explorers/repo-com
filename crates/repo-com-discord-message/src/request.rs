use std::{collections::BTreeSet, error::Error, fmt};

use repo_com_config::{MentionKind, MentionTarget};
use repo_com_draft_content::{
    MAX_DISCORD_MESSAGE_CHARACTERS, NONCE_FOOTER_PREFIX, RenderedMessage, discord_character_count,
};
use serde::Serialize;

const REQUIRED_API_VERSION: &str = "v10";

/// A local validation failure that occurs before any message request is sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    /// Only Discord REST API v10 is accepted.
    UnsupportedApiVersion,
    /// The destination was not an already-resolved configured channel.
    RawChannelId,
    /// A resolved mention was not present in the destination allowlist.
    UnlistedMention,
    /// A mention was not a valid role or user snowflake.
    InvalidMention,
    /// The same role or user appeared more than once.
    DuplicateMention,
    /// The rendered message had no text.
    TextEmpty,
    /// The rendered message exceeded Discord's 2,000-character limit.
    TextTooLong { length: usize, maximum: usize },
    /// The deterministic nonce was not a lowercase SHA-256 value.
    InvalidNonce,
    /// The exact text did not end in exactly one generated nonce footer.
    NonceFooterMismatch,
    /// Reply evidence did not match the rendered repository and destination.
    ReplyReferenceMismatch,
    /// The bounded request body could not be serialized.
    Serialization,
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedApiVersion => "only Discord REST API v10 is supported",
            Self::RawChannelId => "a raw channel identifier cannot be used as a destination",
            Self::UnlistedMention => "a mention is not in the destination allowlist",
            Self::InvalidMention => "a mention target is not a valid role or user",
            Self::DuplicateMention => "a mention target appears more than once",
            Self::TextEmpty => "the rendered message text is empty",
            Self::TextTooLong { .. } => "the rendered message exceeds Discord's text limit",
            Self::InvalidNonce => "the deterministic nonce is invalid",
            Self::NonceFooterMismatch => "the rendered nonce footer is invalid",
            Self::ReplyReferenceMismatch => {
                "the validated reply reference does not match the rendered destination"
            }
            Self::Serialization => "the message body could not be serialized",
        })
    }
}

impl Error for RequestError {}

/// The only Discord REST API version representable by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscordApiVersion {
    /// Discord REST API v10.
    V10,
}

impl DiscordApiVersion {
    /// Returns the wire path segment.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V10 => REQUIRED_API_VERSION,
        }
    }
}

impl TryFrom<&str> for DiscordApiVersion {
    type Error = RequestError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value == REQUIRED_API_VERSION {
            Ok(Self::V10)
        } else {
            Err(RequestError::UnsupportedApiVersion)
        }
    }
}

/// One validated, non-rich Discord create-message request.
///
/// The only public constructor consumes [`RenderedMessage`], so callers cannot
/// substitute a raw channel, unlisted mention, oversized text, or an
/// unvalidated reply reference. Attachments and embeds have no representation.
#[derive(Clone, Eq, PartialEq)]
pub struct CreateMessageRequest {
    api_version: DiscordApiVersion,
    destination_alias: String,
    channel_id: String,
    content: String,
    nonce: String,
    allowed_mentions: AllowedMentionsPayload,
    message_reference: Option<MessageReferencePayload>,
}

impl CreateMessageRequest {
    /// Builds one request from already-rendered, already-resolved draft data.
    pub fn from_rendered(rendered: &RenderedMessage) -> Result<Self, RequestError> {
        let metadata = rendered.request_metadata();
        let destination = rendered.resolved_destination();
        let channel_id = validate_resolved_destination(destination)?;
        let content = validate_text(rendered, &metadata)?;
        let allowed_mentions = validate_mentions(rendered, metadata.allowed_mentions())?;
        let message_reference = metadata
            .message_reference()
            .map(|reference| validate_reply_reference(rendered, reference))
            .transpose()?;

        Ok(Self {
            api_version: DiscordApiVersion::V10,
            destination_alias: destination.alias.clone(),
            channel_id,
            content,
            nonce: metadata.nonce().as_str().to_owned(),
            allowed_mentions,
            message_reference,
        })
    }

    /// Returns the fixed API version used to build the route.
    #[must_use]
    pub const fn api_version(&self) -> DiscordApiVersion {
        self.api_version
    }

    /// Returns the configured alias that produced the resolved destination.
    #[must_use]
    pub fn destination_alias(&self) -> &str {
        &self.destination_alias
    }

    /// Returns the resolved Discord channel snowflake.
    #[must_use]
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// Returns the exact rendered text, including its deterministic nonce footer.
    #[must_use]
    pub fn exact_text(&self) -> &str {
        &self.content
    }

    /// Returns the deterministic draft nonce.
    #[must_use]
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// Returns the referenced message ID for a validated reply draft.
    #[must_use]
    pub fn referenced_message_id(&self) -> Option<&str> {
        self.message_reference
            .as_ref()
            .map(|reference| reference.message_id.as_str())
    }

    pub(crate) fn wire_payload(&self) -> CreateMessagePayload<'_> {
        CreateMessagePayload {
            content: &self.content,
            nonce: &self.nonce,
            allowed_mentions: &self.allowed_mentions,
            message_reference: self.message_reference.as_ref(),
        }
    }

    pub(crate) fn body_bytes(&self) -> Result<Vec<u8>, RequestError> {
        serde_json::to_vec(&self.wire_payload()).map_err(|_| RequestError::Serialization)
    }
}

impl fmt::Debug for CreateMessageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateMessageRequest")
            .field("api_version", &self.api_version)
            .field("destination_alias", &self.destination_alias)
            .field("text", &"<redacted>")
            .field("nonce", &self.nonce)
            .field("allowed_role_count", &self.allowed_mentions.roles.len())
            .field("allowed_user_count", &self.allowed_mentions.users.len())
            .field("is_reply", &self.message_reference.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CreateMessagePayload<'a> {
    content: &'a str,
    nonce: &'a str,
    allowed_mentions: &'a AllowedMentionsPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_reference: Option<&'a MessageReferencePayload>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct AllowedMentionsPayload {
    parse: [&'static str; 0],
    roles: Vec<String>,
    users: Vec<String>,
    replied_user: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct MessageReferencePayload {
    message_id: String,
    channel_id: String,
    guild_id: String,
    fail_if_not_exists: bool,
}

fn validate_resolved_destination(
    destination: &repo_com_config::ResolvedDestination,
) -> Result<String, RequestError> {
    if destination.alias.is_empty()
        || !is_bounded_identifier(&destination.alias, 128)
        || !is_discord_id(&destination.channel_id)
    {
        return Err(RequestError::RawChannelId);
    }
    Ok(destination.channel_id.clone())
}

fn validate_text(
    rendered: &RenderedMessage,
    metadata: &repo_com_draft_content::MessageRequestMetadata,
) -> Result<String, RequestError> {
    let text = rendered.exact_text();
    if text.is_empty() {
        return Err(RequestError::TextEmpty);
    }
    let length = discord_character_count(text);
    if length > MAX_DISCORD_MESSAGE_CHARACTERS {
        return Err(RequestError::TextTooLong {
            length,
            maximum: MAX_DISCORD_MESSAGE_CHARACTERS,
        });
    }
    if !is_lowercase_sha256(metadata.nonce().as_str()) {
        return Err(RequestError::InvalidNonce);
    }
    let footer = metadata.nonce().footer();
    if !text.ends_with(&footer) || text.matches(NONCE_FOOTER_PREFIX).count() != 1 {
        return Err(RequestError::NonceFooterMismatch);
    }
    Ok(text.to_owned())
}

fn validate_mentions(
    rendered: &RenderedMessage,
    targets: &[MentionTarget],
) -> Result<AllowedMentionsPayload, RequestError> {
    let configured = rendered
        .resolved_destination()
        .allowed_mentions
        .iter()
        .map(|mention| mention.target.as_prefixed())
        .collect::<BTreeSet<_>>();
    if configured.len() != rendered.resolved_destination().allowed_mentions.len() {
        return Err(RequestError::DuplicateMention);
    }

    let mut seen = BTreeSet::new();
    let mut roles = Vec::new();
    let mut users = Vec::new();
    for target in targets {
        if !matches!(target.kind(), MentionKind::Role | MentionKind::User)
            || !is_discord_id(target.id())
            || MentionTarget::parse(&target.as_prefixed()).as_ref() != Some(target)
        {
            return Err(RequestError::InvalidMention);
        }
        let key = target.as_prefixed();
        if !configured.contains(&key) {
            return Err(RequestError::UnlistedMention);
        }
        if !seen.insert(key) {
            return Err(RequestError::DuplicateMention);
        }
        match target.kind() {
            MentionKind::Role => roles.push(target.id().to_owned()),
            MentionKind::User => users.push(target.id().to_owned()),
        }
    }
    for mention in rendered.mentions() {
        if !seen.contains(&mention.target().as_prefixed()) {
            return Err(RequestError::UnlistedMention);
        }
    }
    roles.sort_unstable();
    users.sort_unstable();
    Ok(AllowedMentionsPayload {
        parse: [],
        roles,
        users,
        replied_user: false,
    })
}

fn validate_reply_reference(
    rendered: &RenderedMessage,
    reference: &repo_com_draft_content::ValidatedMessageReference,
) -> Result<MessageReferencePayload, RequestError> {
    let destination = rendered.resolved_destination();
    if reference.repository_id() != rendered.repository_id()
        || reference.workspace_id() != destination.workspace_id
        || reference.channel_id() != destination.channel_id
    {
        return Err(RequestError::ReplyReferenceMismatch);
    }
    Ok(MessageReferencePayload {
        message_id: reference.message_id().to_owned(),
        channel_id: reference.channel_id().to_owned(),
        guild_id: reference.workspace_id().to_owned(),
        fail_if_not_exists: true,
    })
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_bounded_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use repo_com_config::ResolvedDestination;

    use super::{RequestError, validate_resolved_destination};

    #[test]
    fn raw_channel_source_without_a_resolved_alias_is_rejected() {
        let raw_channel = ResolvedDestination {
            alias: String::new(),
            workspace_id: "123456789012345678".to_owned(),
            channel_id: "234567890123456789".to_owned(),
            allowed_mentions: Vec::new(),
        };

        assert_eq!(
            validate_resolved_destination(&raw_channel),
            Err(RequestError::RawChannelId)
        );
    }
}
