use std::error::Error;
use std::fmt;

use repo_com_config::{MentionTarget, ResolvedDestination};
use repo_com_draft_model::{
    AuthorizedReplyReference, DraftError, DraftExpiry, DraftMetadata, DraftModel, DraftPreview,
    DraftRevision,
};
use serde::Serialize;

use crate::mentions::{MentionError, MentionResolver, MentionToken};
use crate::normalize::{
    MAX_DISCORD_MESSAGE_CHARACTERS, NormalizeError, discord_character_count, normalize_text,
};

/// The exact prefix used for the deterministic delivery-nonce footer.
pub const NONCE_FOOTER_PREFIX: &str = "\n\n-- repo-com delivery-nonce: ";
/// The marker used to detect an accidentally pre-rendered nonce footer.
pub const NONCE_MARKER: &str = "-- repo-com delivery-nonce:";
/// A compatibility marker rejected to prevent older renderers from being fed
/// their own already-rendered output.
pub const LEGACY_NONCE_MARKER: &str = "-- delivery-nonce";

/// Result type used by the content boundary.
pub type ContentResult<T> = Result<T, RenderError>;

/// A safe, typed rendering failure. It never contains draft body text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// The revision was expired at the supplied deterministic time.
    RevisionExpired { revision: u64 },
    /// The requested revision was not present in the draft.
    RevisionNotFound { draft_id: String, revision: u64 },
    /// The revision's canonical content hash was not a lowercase SHA-256 hex
    /// value.
    InvalidContentHash,
    /// The resolved destination did not match the immutable revision.
    InvalidDestination { field: &'static str },
    /// A configured reply reference no longer matched the revision.
    ReplyReferenceMismatch { field: &'static str },
    /// The normalized body had no semantic text.
    EmptyText,
    /// The body already contained a delivery-nonce marker.
    NonceAlreadyRendered,
    /// The final body plus footer exceeded Discord's 2,000-character limit.
    MessageTooLong { length: usize, maximum: usize },
    /// Mention resolution failed closed.
    Mention(MentionError),
}

impl RenderError {
    /// Returns a stable, non-sensitive error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::RevisionExpired { .. } => "revision-expired",
            Self::RevisionNotFound { .. } => "revision-not-found",
            Self::InvalidContentHash => "invalid-content-hash",
            Self::InvalidDestination { .. } => "invalid-destination",
            Self::ReplyReferenceMismatch { .. } => "reply-reference-mismatch",
            Self::EmptyText => "empty-text",
            Self::NonceAlreadyRendered => "nonce-already-rendered",
            Self::MessageTooLong { .. } => "message-too-long",
            Self::Mention(error) => error.code(),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionExpired { revision } => {
                write!(formatter, "draft revision {revision} is expired")
            }
            Self::RevisionNotFound { draft_id, revision } => {
                write!(formatter, "draft {draft_id} has no revision {revision}")
            }
            Self::InvalidContentHash => formatter.write_str("revision content hash is invalid"),
            Self::InvalidDestination { field } => {
                write!(formatter, "resolved destination field {field} is invalid")
            }
            Self::ReplyReferenceMismatch { field } => {
                write!(
                    formatter,
                    "validated reply reference has mismatched {field}"
                )
            }
            Self::EmptyText => formatter.write_str("normalized draft text is empty"),
            Self::NonceAlreadyRendered => {
                formatter.write_str("draft text already contains a delivery-nonce footer")
            }
            Self::MessageTooLong { length, maximum } => write!(
                formatter,
                "rendered Discord message has {length} characters; maximum is {maximum}"
            ),
            Self::Mention(error) => write!(formatter, "mention resolution failed: {error}"),
        }
    }
}

impl Error for RenderError {}

impl From<MentionError> for RenderError {
    fn from(error: MentionError) -> Self {
        Self::Mention(error)
    }
}

impl From<NormalizeError> for RenderError {
    fn from(error: NormalizeError) -> Self {
        match error {
            NormalizeError::EmptyText => Self::EmptyText,
        }
    }
}

/// A deterministic nonce derived from the immutable revision content hash.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DeliveryNonce(String);

impl DeliveryNonce {
    /// Creates a nonce from a validated lowercase SHA-256 hash.
    pub fn new(value: impl Into<String>) -> ContentResult<Self> {
        let value = value.into();
        if !is_sha256_hex(&value) {
            return Err(RenderError::InvalidContentHash);
        }
        Ok(Self(value))
    }

    /// Creates the nonce for one immutable revision.
    pub fn from_revision(revision: &DraftRevision) -> ContentResult<Self> {
        Self::new(revision.content_hash())
    }

    /// Alias for [`Self::from_revision`].
    pub fn for_revision(revision: &DraftRevision) -> ContentResult<Self> {
        Self::from_revision(revision)
    }

    /// Returns the hash value used as the nonce.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the exact footer, including its leading line break.
    #[must_use]
    pub fn footer(&self) -> String {
        format!("{NONCE_FOOTER_PREFIX}{}", self.0)
    }
}

impl AsRef<str> for DeliveryNonce {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for DeliveryNonce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Metadata that a Discord request builder can consume without treating a
/// reply reference as body text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MessageRequestMetadata {
    nonce: DeliveryNonce,
    allowed_mentions: Vec<MentionTarget>,
    message_reference: Option<AuthorizedReplyReference>,
}

impl MessageRequestMetadata {
    /// Returns the deterministic nonce metadata.
    #[must_use]
    pub const fn nonce(&self) -> &DeliveryNonce {
        &self.nonce
    }

    /// Returns only role/user targets allowed for this destination.
    #[must_use]
    pub fn allowed_mentions(&self) -> &[MentionTarget] {
        &self.allowed_mentions
    }

    /// Returns the allowlisted role/user IDs in deterministic order.
    #[must_use]
    pub fn allowed_mention_ids(&self) -> Vec<String> {
        self.allowed_mentions
            .iter()
            .map(|target| target.id().to_owned())
            .collect()
    }

    /// Returns the already-authorized reply reference, if this is a reply.
    #[must_use]
    pub fn message_reference(&self) -> Option<&AuthorizedReplyReference> {
        self.message_reference.as_ref()
    }
}

/// An immutable, side-effect-free rendered draft projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenderedMessage {
    repository_id: String,
    revision: u64,
    content_hash: String,
    destination_alias: String,
    resolved_destination: ResolvedDestination,
    normalized_body: String,
    exact_text: String,
    nonce: DeliveryNonce,
    nonce_footer: String,
    mentions: Vec<MentionToken>,
    allowed_mentions: Vec<MentionToken>,
    message_reference: Option<AuthorizedReplyReference>,
    metadata: DraftMetadata,
    expiry: DraftExpiry,
}

impl RenderedMessage {
    /// Returns the repository identity captured by the revision.
    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    /// Returns the immutable revision number.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the exact canonical revision hash.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Returns the destination alias.
    #[must_use]
    pub fn destination_alias(&self) -> &str {
        &self.destination_alias
    }

    /// Returns the complete resolved destination snapshot.
    #[must_use]
    pub const fn resolved_destination(&self) -> &ResolvedDestination {
        &self.resolved_destination
    }

    /// Returns the normalized body before the generated footer.
    #[must_use]
    pub fn normalized_body(&self) -> &str {
        &self.normalized_body
    }

    /// Returns the exact final Discord text, including the nonce footer.
    #[must_use]
    pub fn exact_text(&self) -> &str {
        &self.exact_text
    }

    /// Alias for [`Self::exact_text`].
    #[must_use]
    pub fn text(&self) -> &str {
        self.exact_text()
    }

    /// Returns the nonce string used by reconciliation.
    #[must_use]
    pub fn nonce(&self) -> &str {
        self.nonce.as_str()
    }

    /// Returns the typed deterministic nonce.
    #[must_use]
    pub const fn delivery_nonce(&self) -> &DeliveryNonce {
        &self.nonce
    }

    /// Returns the exact generated footer.
    #[must_use]
    pub fn nonce_footer(&self) -> &str {
        &self.nonce_footer
    }

    /// Returns every resolved named mention in source order.
    #[must_use]
    pub fn mentions(&self) -> &[MentionToken] {
        &self.mentions
    }

    /// Returns the deterministic de-duplicated allowlist for the request.
    #[must_use]
    pub fn allowed_mentions(&self) -> &[MentionToken] {
        &self.allowed_mentions
    }

    /// Returns the role/user IDs in the shape consumed by a Discord
    /// `allowed_mentions` policy.
    #[must_use]
    pub fn allowed_mention_ids(&self) -> Vec<String> {
        self.allowed_mentions
            .iter()
            .map(|mention| mention.id().to_owned())
            .collect()
    }

    /// Returns only the target values suitable for a Discord allowed-mentions
    /// policy.
    #[must_use]
    pub fn allowed_mention_targets(&self) -> Vec<MentionTarget> {
        self.allowed_mentions
            .iter()
            .map(|mention| mention.target().clone())
            .collect()
    }

    /// Returns the validated reply reference, carried only as request metadata.
    #[must_use]
    pub fn message_reference(&self) -> Option<&AuthorizedReplyReference> {
        self.message_reference.as_ref()
    }

    /// Returns bounded draft metadata without generating message copy from it.
    #[must_use]
    pub const fn metadata(&self) -> &DraftMetadata {
        &self.metadata
    }

    /// Returns the immutable expiry snapshot.
    #[must_use]
    pub const fn expiry(&self) -> DraftExpiry {
        self.expiry
    }

    /// Builds the request metadata projection consumed by a later transport
    /// adapter. This method does not perform I/O.
    #[must_use]
    pub fn request_metadata(&self) -> MessageRequestMetadata {
        MessageRequestMetadata {
            nonce: self.nonce.clone(),
            allowed_mentions: self.allowed_mention_targets(),
            message_reference: self.message_reference.clone(),
        }
    }

    /// Projects this exact rendered revision into the draft model's
    /// side-effect-free preview, including its unresolved approval, policy, and
    /// safety bases.
    pub fn preview(
        &self,
        draft: &DraftModel,
        at_unix_seconds: u64,
    ) -> Result<DraftPreview, DraftError> {
        draft.preview_with_rendered_text(self.revision, self.exact_text.clone(), at_unix_seconds)
    }
}

/// A pure renderer for one immutable draft revision.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContentRenderer {
    mention_resolver: MentionResolver,
}

impl ContentRenderer {
    /// Creates a renderer without a configured bot identity.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mention_resolver: MentionResolver::new(),
        }
    }

    /// Adds the bot identity so a configured user target cannot mention it.
    pub fn with_bot_user_id(self, value: impl Into<String>) -> ContentResult<Self> {
        Ok(Self {
            mention_resolver: self.mention_resolver.with_bot_user_id(value)?,
        })
    }

    /// Creates a renderer with a bot identity in one call.
    pub fn for_bot_user_id(value: impl Into<String>) -> ContentResult<Self> {
        Self::new().with_bot_user_id(value)
    }

    /// Returns the configured bot identity, if present.
    #[must_use]
    pub fn bot_user_id(&self) -> Option<&str> {
        self.mention_resolver.bot_user_id()
    }

    /// Renders one revision at an explicit caller-supplied Unix time.
    pub fn render(
        &self,
        revision: &DraftRevision,
        at_unix_seconds: u64,
    ) -> ContentResult<RenderedMessage> {
        if revision.is_expired(at_unix_seconds) {
            return Err(RenderError::RevisionExpired {
                revision: revision.number(),
            });
        }
        validate_revision(revision)?;

        let normalized_body = normalize_text(revision.exact_text())?;
        if contains_nonce_marker(&normalized_body) {
            return Err(RenderError::NonceAlreadyRendered);
        }

        let resolution = self
            .mention_resolver
            .resolve(&normalized_body, revision.resolved_destination())?;
        let resolved_text = resolution.text();
        let nonce = DeliveryNonce::from_revision(revision)?;
        let nonce_footer = nonce.footer();
        let exact_text = format!("{resolved_text}{nonce_footer}");
        let length = discord_character_count(&exact_text);
        if length > MAX_DISCORD_MESSAGE_CHARACTERS {
            return Err(RenderError::MessageTooLong {
                length,
                maximum: MAX_DISCORD_MESSAGE_CHARACTERS,
            });
        }

        Ok(RenderedMessage {
            repository_id: revision.repository_id().to_owned(),
            revision: revision.number(),
            content_hash: revision.content_hash().to_owned(),
            destination_alias: revision.destination_alias().as_str().to_owned(),
            resolved_destination: revision.resolved_destination().clone(),
            normalized_body,
            exact_text,
            nonce,
            nonce_footer,
            mentions: resolution.mentions().to_vec(),
            allowed_mentions: resolution.allowed_mentions().to_vec(),
            message_reference: revision.reply_reference().cloned(),
            metadata: revision.metadata().clone(),
            expiry: revision.expiry(),
        })
    }

    /// Alias for [`Self::render`].
    pub fn render_revision(
        &self,
        revision: &DraftRevision,
        at_unix_seconds: u64,
    ) -> ContentResult<RenderedMessage> {
        self.render(revision, at_unix_seconds)
    }

    /// Renders one exact revision from a draft model.
    pub fn render_draft(
        &self,
        draft: &DraftModel,
        revision: u64,
        at_unix_seconds: u64,
    ) -> ContentResult<RenderedMessage> {
        let snapshot = draft
            .revision(revision)
            .ok_or_else(|| RenderError::RevisionNotFound {
                draft_id: draft.draft_id().as_str().to_owned(),
                revision,
            })?;
        self.render(snapshot, at_unix_seconds)
    }

    /// Renders the current revision from a draft model.
    pub fn render_current(
        &self,
        draft: &DraftModel,
        at_unix_seconds: u64,
    ) -> ContentResult<RenderedMessage> {
        self.render(draft.current_revision(), at_unix_seconds)
    }
}

/// Renders one revision with a renderer that has no bot identity configured.
pub fn render_revision(
    revision: &DraftRevision,
    at_unix_seconds: u64,
) -> ContentResult<RenderedMessage> {
    ContentRenderer::new().render(revision, at_unix_seconds)
}

/// Renders one revision while explicitly rejecting the bot user as a mention.
pub fn render_revision_for_bot(
    revision: &DraftRevision,
    at_unix_seconds: u64,
    bot_user_id: impl Into<String>,
) -> ContentResult<RenderedMessage> {
    ContentRenderer::for_bot_user_id(bot_user_id)?.render(revision, at_unix_seconds)
}

/// Returns whether text appears to contain a previously rendered nonce footer.
#[must_use]
pub fn contains_nonce_marker(text: &str) -> bool {
    text.contains(NONCE_MARKER)
        || text.contains(LEGACY_NONCE_MARKER)
        || text.contains("repo-com delivery nonce:")
}

fn validate_revision(revision: &DraftRevision) -> ContentResult<()> {
    if !is_sha256_hex(revision.content_hash()) {
        return Err(RenderError::InvalidContentHash);
    }
    let destination = revision.resolved_destination();
    if destination.alias != revision.destination_alias().as_str() {
        return Err(RenderError::InvalidDestination { field: "alias" });
    }
    if !is_discord_id(&destination.workspace_id) {
        return Err(RenderError::InvalidDestination {
            field: "workspace_id",
        });
    }
    if !is_discord_id(&destination.channel_id) {
        return Err(RenderError::InvalidDestination {
            field: "channel_id",
        });
    }

    if let Some(reference) = revision.reply_reference() {
        if reference.repository_id() != revision.repository_id() {
            return Err(RenderError::ReplyReferenceMismatch {
                field: "repository_id",
            });
        }
        if reference.workspace_id() != destination.workspace_id {
            return Err(RenderError::ReplyReferenceMismatch {
                field: "workspace_id",
            });
        }
        if reference.channel_id() != destination.channel_id {
            return Err(RenderError::ReplyReferenceMismatch {
                field: "channel_id",
            });
        }
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}
