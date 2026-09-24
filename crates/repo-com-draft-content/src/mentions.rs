use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use repo_com_config::{MentionKind, MentionTarget, ResolvedDestination, ResolvedMention};
use serde::Serialize;

/// A named mention that was found in text and resolved from the destination
/// allowlist.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MentionToken {
    alias: String,
    target: MentionTarget,
    rendered: String,
}

impl MentionToken {
    fn new(alias: String, target: MentionTarget) -> Result<Self, MentionError> {
        if !is_valid_alias(&alias) {
            return Err(MentionError::InvalidAlias {
                alias: bounded_alias(&alias),
            });
        }
        if !is_valid_target(&target) {
            return Err(MentionError::InvalidTarget {
                alias: bounded_alias(&alias),
            });
        }
        let rendered = render_target(&target);
        Ok(Self {
            alias,
            target,
            rendered,
        })
    }

    /// Returns the repository-local alias used in the draft body.
    #[must_use]
    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// Returns the validated role or user target.
    #[must_use]
    pub fn target(&self) -> &MentionTarget {
        &self.target
    }

    /// Returns the Discord mention kind.
    #[must_use]
    pub const fn kind(&self) -> MentionKind {
        self.target.kind
    }

    /// Returns the target's identifier without its role/user prefix.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.target.id
    }

    /// Returns the exact Discord text replacing the named alias.
    #[must_use]
    pub fn rendered(&self) -> &str {
        &self.rendered
    }
}

/// The result of resolving mentions in one text body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentionResolution {
    text: String,
    occurrences: Vec<MentionToken>,
    allowed_mentions: Vec<MentionToken>,
}

impl MentionResolution {
    /// Returns text with only allowlisted named mentions expanded.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns every resolved occurrence in source order.
    #[must_use]
    pub fn occurrences(&self) -> &[MentionToken] {
        &self.occurrences
    }

    /// Returns a deterministic, de-duplicated allowed-mention set.
    #[must_use]
    pub fn allowed_mentions(&self) -> &[MentionToken] {
        &self.allowed_mentions
    }

    /// Alias for [`Self::occurrences`] used by request builders.
    #[must_use]
    pub fn mentions(&self) -> &[MentionToken] {
        self.occurrences()
    }
}

/// A safe, typed mention-resolution failure. Errors never retain message text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MentionError {
    /// The configured bot user identifier is not a Discord snowflake.
    InvalidBotUserId,
    /// The resolved destination itself is malformed.
    InvalidDestination,
    /// A configured alias is not a valid repository alias.
    InvalidAlias { alias: String },
    /// A destination contained the same alias more than once.
    DuplicateAlias { alias: String },
    /// A configured role/user target is malformed.
    InvalidTarget { alias: String },
    /// The body used a named alias absent from the destination allowlist.
    UnlistedAlias { alias: String },
    /// The body used a malformed named-alias token.
    MalformedAlias,
    /// The body used a raw Discord mention, mass mention, or raw target form.
    RawDiscordMention,
    /// A configured user target is the bot identity.
    UserBotMention { alias: String },
    /// The body used a mention-like form that this renderer does not support.
    UnsupportedMentionSyntax,
}

impl MentionError {
    /// Returns a stable, non-sensitive error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidBotUserId => "invalid-bot-user-id",
            Self::InvalidDestination => "invalid-destination",
            Self::InvalidAlias { .. } => "invalid-mention-alias",
            Self::DuplicateAlias { .. } => "duplicate-mention-alias",
            Self::InvalidTarget { .. } => "invalid-mention-target",
            Self::UnlistedAlias { .. } => "unlisted-mention-alias",
            Self::MalformedAlias => "malformed-mention-alias",
            Self::RawDiscordMention => "raw-discord-mention",
            Self::UserBotMention { .. } => "user-bot-mention",
            Self::UnsupportedMentionSyntax => "unsupported-mention-syntax",
        }
    }
}

impl fmt::Display for MentionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBotUserId => formatter.write_str("bot user ID is not a Discord snowflake"),
            Self::InvalidDestination => {
                formatter.write_str("resolved mention destination is invalid")
            }
            Self::InvalidAlias { .. } => formatter.write_str("configured mention alias is invalid"),
            Self::DuplicateAlias { .. } => {
                formatter.write_str("configured mention alias is duplicated")
            }
            Self::InvalidTarget { .. } => {
                formatter.write_str("configured mention target is invalid")
            }
            Self::UnlistedAlias { .. } => formatter.write_str("mention alias is not allowlisted"),
            Self::MalformedAlias => formatter.write_str("mention alias syntax is malformed"),
            Self::RawDiscordMention => formatter.write_str("raw Discord mentions are not allowed"),
            Self::UserBotMention { .. } => {
                formatter.write_str("the bot user cannot be a mention target")
            }
            Self::UnsupportedMentionSyntax => formatter.write_str("mention syntax is unsupported"),
        }
    }
}

impl Error for MentionError {}

/// Resolves named aliases against one immutable destination allowlist.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MentionResolver {
    bot_user_id: Option<String>,
}

impl MentionResolver {
    /// Creates a resolver without a bot identity. Raw bot mention forms are
    /// still rejected; callers that know the bot ID should use
    /// [`Self::with_bot_user_id`].
    #[must_use]
    pub const fn new() -> Self {
        Self { bot_user_id: None }
    }

    /// Adds the bot identity used to reject a configured user target that
    /// would mention the bot itself.
    pub fn with_bot_user_id(mut self, value: impl Into<String>) -> Result<Self, MentionError> {
        let bot_user_id = value.into();
        if !is_discord_id(&bot_user_id) {
            return Err(MentionError::InvalidBotUserId);
        }
        self.bot_user_id = Some(bot_user_id);
        Ok(self)
    }

    /// Returns the configured bot identity, if one was supplied.
    #[must_use]
    pub fn bot_user_id(&self) -> Option<&str> {
        self.bot_user_id.as_deref()
    }

    /// Resolves mentions in `text` using a resolved destination.
    pub fn resolve(
        &self,
        text: &str,
        destination: &ResolvedDestination,
    ) -> Result<MentionResolution, MentionError> {
        if !is_valid_alias(&destination.alias)
            || !is_discord_id(&destination.workspace_id)
            || !is_discord_id(&destination.channel_id)
        {
            return Err(MentionError::InvalidDestination);
        }
        self.resolve_with_allowlist(text, &destination.allowed_mentions)
    }

    /// Resolves mentions using an explicit already-resolved allowlist.
    pub fn resolve_with_allowlist(
        &self,
        text: &str,
        allowlist: &[ResolvedMention],
    ) -> Result<MentionResolution, MentionError> {
        let allowlist = self.validate_allowlist(allowlist)?;
        let mut output = String::with_capacity(text.len());
        let mut occurrences = Vec::new();
        let mut allowed_by_target = BTreeMap::<(MentionKind, String), MentionToken>::new();
        let characters = text.char_indices().collect::<Vec<_>>();
        let mut index = 0;

        while index < characters.len() {
            let (start, character) = characters[index];

            if starts_at(text, start, "<@")
                || starts_at(text, start, "<#")
                || starts_at_case_insensitive(text, start, "role:")
                || starts_at_case_insensitive(text, start, "user:")
            {
                return Err(MentionError::RawDiscordMention);
            }

            if starts_at(text, start, "{{") {
                return Err(MentionError::UnsupportedMentionSyntax);
            }

            if character != '@' {
                output.push(character);
                index += 1;
                continue;
            }

            let Some(next_character) = characters.get(index + 1).map(|(_, value)| *value) else {
                output.push(character);
                index += 1;
                continue;
            };

            if matches!(next_character, '!' | '&' | '#' | ':' | '@')
                || next_character.is_ascii_digit()
            {
                return Err(MentionError::RawDiscordMention);
            }

            if index
                .checked_sub(1)
                .and_then(|previous_index| characters.get(previous_index))
                .is_some_and(|(_, value)| is_identifier_character(*value))
            {
                output.push(character);
                index += 1;
                continue;
            }

            if !next_character.is_ascii_alphabetic() {
                if next_character.is_alphanumeric() {
                    return Err(MentionError::MalformedAlias);
                }
                output.push(character);
                index += 1;
                continue;
            }

            let mut alias_end_index = index + 1;
            while alias_end_index < characters.len()
                && is_alias_character(characters[alias_end_index].1)
            {
                alias_end_index += 1;
            }
            let alias_end = characters
                .get(alias_end_index)
                .map_or(text.len(), |(offset, _)| *offset);
            let alias = &text[start + 1..alias_end];
            let following = characters.get(alias_end_index).map(|(_, value)| *value);
            if following.is_some_and(|value| {
                value.is_alphanumeric() || matches!(value, '!' | '&' | '#' | ':' | '@')
            }) {
                return Err(MentionError::MalformedAlias);
            }
            if alias.eq_ignore_ascii_case("everyone") || alias.eq_ignore_ascii_case("here") {
                return Err(MentionError::RawDiscordMention);
            }

            let (resolved_alias, consumed_alias_end_index) = if allowlist.contains_key(alias) {
                (alias, alias_end_index)
            } else {
                let trimmed = alias.trim_end_matches('.');
                if trimmed.len() < alias.len() && allowlist.contains_key(trimmed) {
                    (trimmed, alias_end_index - (alias.len() - trimmed.len()))
                } else {
                    (alias, alias_end_index)
                }
            };
            let token = resolve_alias(resolved_alias, &allowlist)?;
            output.push_str(token.rendered());
            allowed_by_target
                .entry((token.kind(), token.id().to_owned()))
                .or_insert_with(|| token.clone());
            occurrences.push(token);
            index = consumed_alias_end_index;
        }

        let allowed_mentions = allowed_by_target.into_values().collect();
        Ok(MentionResolution {
            text: output,
            occurrences,
            allowed_mentions,
        })
    }

    fn validate_allowlist(
        &self,
        allowlist: &[ResolvedMention],
    ) -> Result<BTreeMap<String, MentionToken>, MentionError> {
        let mut validated = BTreeMap::new();
        for mention in allowlist {
            if !is_valid_alias(&mention.alias) {
                return Err(MentionError::InvalidAlias {
                    alias: bounded_alias(&mention.alias),
                });
            }
            if !is_valid_target(&mention.target) {
                return Err(MentionError::InvalidTarget {
                    alias: bounded_alias(&mention.alias),
                });
            }
            if mention.target.kind() == MentionKind::User
                && self.bot_user_id.as_deref() == Some(mention.target.id())
            {
                return Err(MentionError::UserBotMention {
                    alias: bounded_alias(&mention.alias),
                });
            }
            let token = MentionToken::new(mention.alias.clone(), mention.target.clone())?;
            if validated.insert(mention.alias.clone(), token).is_some() {
                return Err(MentionError::DuplicateAlias {
                    alias: bounded_alias(&mention.alias),
                });
            }
        }
        Ok(validated)
    }
}

/// Compatibility name for the resolved occurrence type.
pub type ResolvedMentionToken = MentionToken;
/// Compatibility name for an allowlisted mention target.
pub type AllowedMention = MentionToken;

fn resolve_alias(
    alias: &str,
    allowlist: &BTreeMap<String, MentionToken>,
) -> Result<MentionToken, MentionError> {
    allowlist
        .get(alias)
        .cloned()
        .ok_or_else(|| MentionError::UnlistedAlias {
            alias: bounded_alias(alias),
        })
}

fn render_target(target: &MentionTarget) -> String {
    match target.kind() {
        MentionKind::Role => format!("<@&{}>", target.id()),
        MentionKind::User => format!("<@{}>", target.id()),
    }
}

fn starts_at(text: &str, start: usize, prefix: &str) -> bool {
    text.get(start..)
        .is_some_and(|tail| tail.starts_with(prefix))
}

fn starts_at_case_insensitive(text: &str, start: usize, prefix: &str) -> bool {
    text.get(start..).is_some_and(|tail| {
        tail.get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
    })
}

fn is_valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 64
        && alias
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn is_alias_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || "-_.".contains(character)
}

fn is_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || "-_.".contains(character)
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_valid_target(target: &MentionTarget) -> bool {
    is_discord_id(target.id())
        && matches!(target.kind(), MentionKind::Role | MentionKind::User)
        && MentionTarget::parse(&target.as_prefixed()).as_ref() == Some(target)
}

fn bounded_alias(alias: &str) -> String {
    alias.chars().take(64).collect()
}
