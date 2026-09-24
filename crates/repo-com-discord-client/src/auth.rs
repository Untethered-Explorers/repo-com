use std::env;

use zeroize::Zeroizing;

/// The only environment variable from which this crate accepts a Discord bot
/// token.
pub const BOT_TOKEN_ENV: &str = "REPO_COM_DISCORD_TOKEN";

const MAX_BOT_TOKEN_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthError {
    Missing,
    InvalidFormat,
}

/// An owned, non-cloneable bot credential whose string storage is zeroized on
/// drop. Construction is deliberately private so normal callers cannot supply a
/// token from configuration, command input, or another environment variable.
pub(crate) struct BotToken(Zeroizing<String>);

impl BotToken {
    pub(crate) fn from_environment() -> Result<Self, AuthError> {
        let value = env::var(BOT_TOKEN_ENV).map_err(|error| match error {
            env::VarError::NotPresent => AuthError::Missing,
            env::VarError::NotUnicode(_) => AuthError::InvalidFormat,
        })?;
        let value = Zeroizing::new(value);
        if is_valid_bot_token(&value) {
            Ok(Self(value))
        } else {
            Err(AuthError::InvalidFormat)
        }
    }

    pub(crate) fn authorization_value(&self) -> Zeroizing<String> {
        let capacity = self.0.len().saturating_add("Bot ".len());
        let mut value = Zeroizing::new(String::with_capacity(capacity));
        value.push_str("Bot ");
        value.push_str(self.0.as_str());
        value
    }
}

fn is_valid_bot_token(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_BOT_TOKEN_BYTES {
        return false;
    }

    let mut segments = value.split('.');
    let first = segments.next().unwrap_or_default();
    let second = segments.next().unwrap_or_default();
    let third = segments.next().unwrap_or_default();
    if segments.next().is_some()
        || [first, second, third]
            .iter()
            .any(|segment| segment.is_empty() || !segment.bytes().all(is_token_byte))
    {
        return false;
    }

    // Requiring the documented three-part bot-token shape rejects values that
    // already contain a Bearer/Basic/Bot scheme. Identity is still proven by
    // GET /users/@me, whose `bot` field must be true.
    true
}

const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'=')
}
