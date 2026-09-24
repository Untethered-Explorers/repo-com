//! Deterministic width-aware helpers for operations terminal presentation.
//!
//! Values are wrapped at the effective width and are never truncated.  The
//! minimum width is deliberately 80 columns, including for hashes, plan
//! material, findings, and operator instructions.

use std::fmt;

/// The minimum terminal width supported by the presentation contract.
pub const MIN_TERMINAL_WIDTH: usize = 80;

/// A validated terminal width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalWidth(usize);

impl TerminalWidth {
    /// Creates a width, clamping values below the 80-column contract minimum.
    #[must_use]
    pub const fn new(columns: usize) -> Self {
        Self(if columns < MIN_TERMINAL_WIDTH {
            MIN_TERMINAL_WIDTH
        } else {
            columns
        })
    }

    /// Creates the canonical 80-column width.
    #[must_use]
    pub const fn eighty() -> Self {
        Self(MIN_TERMINAL_WIDTH)
    }

    /// Returns the effective number of columns.
    #[must_use]
    pub const fn columns(self) -> usize {
        self.0
    }
}

impl Default for TerminalWidth {
    fn default() -> Self {
        Self::eighty()
    }
}

impl fmt::Display for TerminalWidth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Returns the display width of one Unicode scalar value.
#[must_use]
pub fn character_width(character: char) -> usize {
    if character == '\t' {
        return 4;
    }
    if character.is_control() {
        return 0;
    }
    let code = character as u32;
    if is_combining(code) {
        return 0;
    }
    if is_wide(code) { 2 } else { 1 }
}

/// Returns the visible display width of a string, ignoring ANSI CSI sequences.
#[must_use]
pub fn display_width(value: &str) -> usize {
    strip_ansi(value).chars().map(character_width).sum()
}

/// Removes ANSI CSI escape sequences from a string for width and accessibility
/// assertions. Semantic meaning is always carried by labels, never by ANSI.
#[must_use]
pub fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            result.push(character);
            continue;
        }
        if characters.next() != Some('[') {
            result.push(character);
            continue;
        }
        for escaped in characters.by_ref() {
            if escaped.is_ascii_alphabetic() {
                break;
            }
        }
    }
    result
}

/// Makes control characters visible before they reach a terminal. Newlines are
/// retained as reading boundaries; carriage returns, tabs, escapes, and other
/// controls are represented as printable backslash sequences. This prevents
/// untrusted inbound text from injecting terminal controls while preserving a
/// deterministic, reviewable representation of the value.
#[must_use]
pub fn sanitize_text(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => result.push('\n'),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character.is_control() => {
                result.push_str(&format!("\\u{{{:x}}}", character as u32));
            }
            character => result.push(character),
        }
    }
    result
}

/// Wraps text at `width` columns without dropping or normalizing any content.
/// Existing line boundaries are preserved and unbreakable values are split at
/// the exact visible-width boundary.
#[must_use]
pub fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut output = Vec::new();
    for source_line in value.split('\n') {
        let source_line = source_line.strip_suffix('\r').unwrap_or(source_line);
        if source_line.is_empty() {
            output.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_width = 0;
        for character in source_line.chars() {
            let character_columns = character_width(character);
            if character_columns > 0
                && current_width > 0
                && current_width + character_columns > width
            {
                output.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(character);
            current_width += character_columns;
        }
        if !current.is_empty() || output.is_empty() {
            output.push(current);
        }
    }
    if output.is_empty() {
        output.push(String::new());
    }
    output
}

/// Renders a labeled value with a stable continuation indent.
#[must_use]
pub fn field_lines(label: &str, value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let label = sanitize_text(label);
    let prefix = format!("{label}: ");
    let value = sanitize_text(value);
    let value = if value.is_empty() { "(none)" } else { &value };
    if prefix.chars().count() >= width {
        let mut output = wrap_text(&prefix, width);
        output.extend(wrap_text(value, width));
        return output;
    }
    let continuation = " ".repeat(prefix.chars().count());
    let value_lines = wrap_text(value, width.saturating_sub(prefix.chars().count()).max(1));
    let mut output = Vec::with_capacity(value_lines.len());
    for (index, line) in value_lines.into_iter().enumerate() {
        if index == 0 {
            output.push(format!("{prefix}{line}"));
        } else {
            output.push(format!("{continuation}{line}"));
        }
    }
    output
}

/// Renders a multiline labeled block while retaining every source line.
#[must_use]
pub fn block_lines(label: &str, value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let label = sanitize_text(label);
    let mut output = wrap_text(&format!("{label}:"), width);
    let value = sanitize_text(value);
    let value = if value.is_empty() { "(none)" } else { &value };
    for line in wrap_text(value, width.saturating_sub(2).max(1)) {
        if line.is_empty() {
            output.push("  ".to_owned());
        } else {
            output.push(format!("  {line}"));
        }
    }
    output
}

/// Returns whether every visible line fits the requested width.
#[must_use]
pub fn lines_fit(value: &str, width: usize) -> bool {
    value.lines().all(|line| display_width(line) <= width)
}

fn is_combining(code: u32) -> bool {
    matches!(
        code,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
    )
}

fn is_wide(code: u32) -> bool {
    matches!(
        code,
        0x1100..=0x115f
            | 0x2329..=0x232a
            | 0x2e80..=0xa4cf
            | 0xac00..=0xd7a3
            | 0xf900..=0xfaff
            | 0xfe10..=0xfe19
            | 0xfe30..=0xfe6f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6
            | 0x1f300..=0x1faff
            | 0x20000..=0x3fffd
    )
}

#[cfg(test)]
mod tests {
    use super::{display_width, field_lines, lines_fit, sanitize_text, wrap_text};

    #[test]
    fn wraps_long_values_without_truncation() {
        let value = "x".repeat(241);
        let lines = wrap_text(&value, 80);
        assert_eq!(lines.concat(), value);
        assert!(lines.iter().all(|line| display_width(line) <= 80));
    }

    #[test]
    fn field_continuations_fit_and_keep_the_label() {
        let lines = field_lines("Plan hash", &"a".repeat(100), 80);
        assert!(lines_fit(&lines.join("\n"), 80));
        assert!(lines[0].starts_with("Plan hash: "));
    }

    #[test]
    fn control_characters_are_visible_and_cannot_inject_ansi() {
        let rendered = field_lines("Untrusted", "\u{1b}[31mred\u{1b}[0m\r", 80).join("\n");
        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.contains("\\u{1b}"));
        assert!(rendered.contains("\\r"));
        assert_eq!(sanitize_text("a\nb"), "a\nb");
    }
}
