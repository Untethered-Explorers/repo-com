#![forbid(unsafe_code)]
#![doc = "Pure, deterministic rendering of immutable repo-com draft revisions."]

#[cfg(test)]
#[path = "../tests/draft_content_contract.rs"]
mod draft_content_contract;

pub mod mentions;
pub mod normalize;
pub mod render;

pub use mentions::{
    AllowedMention, MentionError, MentionResolution, MentionResolver, MentionToken,
    ResolvedMentionToken,
};
pub use normalize::{
    MAX_DISCORD_MESSAGE_CHARACTERS, NormalizeError, NormalizedText, discord_character_count,
    normalize, normalize_text,
};
pub use render::{
    ContentRenderer, ContentResult, DeliveryNonce, LEGACY_NONCE_MARKER, MessageRequestMetadata,
    NONCE_FOOTER_PREFIX, NONCE_MARKER, RenderError, RenderedMessage, contains_nonce_marker,
    render_revision, render_revision_for_bot,
};

/// The opaque, already-authorized reply reference carried by a draft model.
pub use repo_com_draft_model::{
    AuthorizedReplyReference, DecisionBases, DraftPreview, ExactTextSource,
};
pub type MessageReference = AuthorizedReplyReference;
/// Explicit name for the already-authorized reply reference projection.
pub type ValidatedMessageReference = AuthorizedReplyReference;
