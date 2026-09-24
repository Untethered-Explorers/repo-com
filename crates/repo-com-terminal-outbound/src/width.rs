//! Deterministic width-aware helpers for terminal presentation.
//!
//! The renderer never truncates a value to make it fit.  Instead, it wraps at
//! the requested width and uses an explicit continuation indent for labeled
//! fields.  The minimum accepted width is deliberately 80 columns.

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
///
/// This intentionally implements the small, portable subset needed by the
/// renderer rather than depending on terminal-specific width heuristics.  ASCII
/// and common combining marks have the expected widths, and CJK/full-width
/// ranges occupy two columns.  Unknown characters occupy one column.
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
/// assertions.  The renderer never uses escapes to carry semantic meaning.
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
            // Preserve a non-CSI escape as literal text.  It is safer than
            // silently dropping an unexpected operator-provided value.
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

/// Wraps text at `width` columns without dropping or normalizing any content.
///
/// Existing line boundaries are preserved.  A line that is wider than the
/// budget is split at the exact display-width boundary, including long hashes
/// and other unbreakable security values.
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
    let prefix = format!("{label}: ");
    let continuation = " ".repeat(prefix.chars().count());
    let value = if value.is_empty() { "(none)" } else { value };
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

/// Renders a multiline labeled block.  Every source line is retained, and an
/// empty value is represented explicitly rather than becoming invisible.
#[must_use]
pub fn block_lines(label: &str, value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut output = vec![format!("{label}:")];
    let value = if value.is_empty() { "(none)" } else { value };
    let value_lines = wrap_text(value, width.saturating_sub(2).max(1));
    for line in value_lines {
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
    matches!(code,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f)
}

fn is_wide(code: u32) -> bool {
    matches!(code,
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
            | 0x20000..=0x3fffd)
}

#[cfg(test)]
mod tests {
    use super::{display_width, field_lines, lines_fit, wrap_text};

    #[test]
    fn wraps_long_security_values_without_truncation() {
        let value = "a".repeat(173);
        let lines = wrap_text(&value, 80);
        assert!(lines.iter().all(|line| line.len() <= 80));
        assert_eq!(lines.concat(), value);
    }

    #[test]
    fn field_continuations_remain_labeled_and_fit() {
        let lines = field_lines("Revision hash", &"b".repeat(100), 80);
        assert!(lines_fit(&lines.join("\n"), 80));
        assert!(lines[0].starts_with("Revision hash: "));
        assert!(lines[1].starts_with(&" ".repeat("Revision hash: ".len())));
    }

    #[test]
    fn ansi_sequences_do_not_change_visible_width() {
        assert_eq!(display_width("\u{1b}[31mred\u{1b}[0m"), 3);
    }
}
