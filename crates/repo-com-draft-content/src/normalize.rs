use std::error::Error;
use std::fmt;

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

/// Discord's documented maximum content length, counted as Unicode scalar
/// values after canonical normalization.
pub const MAX_DISCORD_MESSAGE_CHARACTERS: usize = 2_000;

/// A normalized, non-empty text body ready for deterministic mention parsing.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct NormalizedText(String);

impl NormalizedText {
    /// Returns the normalized text without a generated footer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the normalized text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for NormalizedText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A safe normalization failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizeError {
    /// The input contained no non-whitespace semantic content.
    EmptyText,
}

impl fmt::Display for NormalizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyText => formatter.write_str("normalized draft text is empty"),
        }
    }
}

impl Error for NormalizeError {}

/// Normalizes text without adding message copy.
///
/// Normalization is intentionally narrow and deterministic:
/// - Unicode is composed to NFC, which preserves canonical-equivalent meaning;
/// - CRLF, CR, NEL, U+2028, and U+2029 become LF;
/// - horizontal whitespace at the end of each line is removed; and
/// - trailing empty lines are removed.
///
/// Leading whitespace, internal whitespace, and non-whitespace Unicode content
/// are preserved. No metadata, destination data, or nonce is generated here.
pub fn normalize(input: &str) -> Result<NormalizedText, NormalizeError> {
    let composed: String = input.nfc().collect();
    let mut line_endings_normalized = String::with_capacity(composed.len());
    let mut characters = composed.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    let _ = characters.next();
                }
                line_endings_normalized.push('\n');
            }
            '\u{0085}' | '\u{2028}' | '\u{2029}' => {
                line_endings_normalized.push('\n');
            }
            _ => line_endings_normalized.push(character),
        }
    }

    let mut normalized_lines = line_endings_normalized
        .split('\n')
        .map(trim_horizontal_end)
        .collect::<Vec<_>>()
        .join("\n");
    while normalized_lines.ends_with('\n') {
        normalized_lines.pop();
    }

    if normalized_lines.trim().is_empty() {
        Err(NormalizeError::EmptyText)
    } else {
        Ok(NormalizedText(normalized_lines))
    }
}

/// Returns normalized text as a plain `String` for callers that do not need the
/// wrapper.
pub fn normalize_text(input: &str) -> Result<String, NormalizeError> {
    normalize(input).map(NormalizedText::into_string)
}

/// Returns Discord's character count for already-normalized text.
///
/// Discord specifies a 2,000-character content limit. Rust's `char` count is
/// used deliberately: it counts Unicode scalar values rather than UTF-8 bytes
/// or platform-dependent terminal widths.
#[must_use]
pub fn discord_character_count(text: &str) -> usize {
    text.chars().count()
}

fn trim_horizontal_end(line: &str) -> &str {
    line.trim_end_matches(|character: char| {
        character.is_whitespace()
            && !matches!(
                character,
                '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}'
            )
    })
}
