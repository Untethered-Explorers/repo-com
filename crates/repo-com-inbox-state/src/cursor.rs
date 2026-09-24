//! Per-alias cursor values and deterministic local ordering helpers.

use std::cmp::Ordering;

pub use repo_com_state::{InboundCursorInput, InboundCursorRecord};

/// A cursor update within one repository and alias.
pub type CursorInput = InboundCursorInput;
/// The stored cursor for one repository and alias.
pub type CursorRecord = InboundCursorRecord;
/// Compatibility name for a stored alias cursor.
pub type AliasCursor = InboundCursorRecord;

/// Compares two opaque cursor values using the local deterministic rule.
///
/// Numeric values are compared numerically.  Other opaque values use their
/// byte-wise string order.  The cursor itself remains opaque to this crate;
/// this ordering is only the monotonicity rule for the local alias record.
#[must_use]
pub fn compare_cursor_values(previous: &str, next: &str) -> Ordering {
    if let (Ok(left), Ok(right)) = (previous.parse::<u128>(), next.parse::<u128>()) {
        return left.cmp(&right);
    }
    previous.cmp(next)
}

/// Returns whether `next` is a valid forward movement from `previous`.
#[must_use]
pub fn cursor_moves_forward(previous: &str, next: &str) -> bool {
    compare_cursor_values(previous, next) == Ordering::Greater
}

/// Returns whether two cursor values are equal under the local ordering rule.
#[must_use]
pub fn cursor_values_equal(previous: &str, next: &str) -> bool {
    compare_cursor_values(previous, next) == Ordering::Equal
}
