//! Explicit boundaries and the small amount of time arithmetic needed to
//! turn an RFC 3339 lower bound into a Discord snowflake cursor.

use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

/// The hard page budget for one fetch operation.
pub const MAX_FETCH_PAGES: usize = 10;
/// The hard raw-message budget for one fetch operation.
pub const MAX_RAW_MESSAGES: usize = 1_000;
/// The Discord message-page size used by the adapter.
pub const MESSAGES_PER_PAGE: usize = 100;
/// The hard point-check budget after one stored page.
pub const MAX_POINT_CHECKS: usize = 100;

/// Exactly one explicit lower boundary for a fetch.
///
/// A cursor is retained as an opaque local value. A time boundary is validated
/// as RFC 3339 and converted to the lower-bound Discord snowflake used by the
/// `after` query parameter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchBoundary {
    /// The last event/message ID already durably observed.
    Cursor(String),
    /// An RFC 3339 timestamp lower boundary.
    Time(String),
}

impl FetchBoundary {
    /// Creates a cursor boundary.
    pub fn cursor(value: impl Into<String>) -> Result<Self, BoundaryError> {
        let value = value.into();
        validate_cursor(&value)?;
        Ok(Self::Cursor(value))
    }

    /// Creates and validates an RFC 3339 time boundary.
    pub fn time(value: impl Into<String>) -> Result<Self, BoundaryError> {
        let value = value.into();
        rfc3339_to_unix_millis(&value)?;
        Ok(Self::Time(value))
    }

    /// Creates a boundary from optional cursor and time inputs.
    ///
    /// Supplying both values is an error even when one of them is empty; an
    /// empty value is never silently treated as an omitted boundary.
    pub fn from_parts(cursor: Option<&str>, time: Option<&str>) -> Result<Self, BoundaryError> {
        match (cursor, time) {
            (Some(_), Some(_)) => Err(BoundaryError::BothBoundaries),
            (Some(value), None) => Self::cursor(value),
            (None, Some(value)) => Self::time(value),
            (None, None) => Err(BoundaryError::NeitherBoundary),
        }
    }

    /// Returns the cursor value, if this is a cursor boundary.
    #[must_use]
    pub fn cursor_value(&self) -> Option<&str> {
        match self {
            Self::Cursor(value) => Some(value),
            Self::Time(_) => None,
        }
    }

    /// Returns the RFC 3339 value, if this is a time boundary.
    #[must_use]
    pub fn time_value(&self) -> Option<&str> {
        match self {
            Self::Cursor(_) => None,
            Self::Time(value) => Some(value),
        }
    }

    /// Returns the Discord `after` value for the first request.
    pub fn initial_cursor(&self) -> Result<String, BoundaryError> {
        match self {
            Self::Cursor(value) => {
                validate_cursor(value)?;
                Ok(value.clone())
            }
            Self::Time(value) => Ok(rfc3339_to_discord_snowflake(value)?),
        }
    }

    /// Returns whether this boundary is an explicit message cursor.
    #[must_use]
    pub const fn is_cursor(&self) -> bool {
        matches!(self, Self::Cursor(_))
    }

    /// Returns whether this boundary is an RFC 3339 time boundary.
    #[must_use]
    pub const fn is_time(&self) -> bool {
        matches!(self, Self::Time(_))
    }
}

/// Compatibility spelling for callers that use the feature-interface name.
pub type InboundBoundary = FetchBoundary;
/// Short compatibility spelling for a fetch boundary.
pub type Boundary = FetchBoundary;

/// A safe boundary validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryError {
    /// Both a cursor and a time boundary were supplied.
    BothBoundaries,
    /// Neither boundary was supplied.
    NeitherBoundary,
    /// The cursor was empty, unsafe, or too long.
    InvalidCursor,
    /// The time value was not a valid RFC 3339 timestamp.
    InvalidTimestamp,
    /// The timestamp cannot be represented by a Discord snowflake.
    TimestampOutOfRange,
}

impl BoundaryError {
    /// Returns a stable, redacted error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::BothBoundaries => "inbound-boundary-both",
            Self::NeitherBoundary => "inbound-boundary-neither",
            Self::InvalidCursor => "inbound-cursor-invalid",
            Self::InvalidTimestamp => "inbound-time-invalid",
            Self::TimestampOutOfRange => "inbound-time-out-of-range",
        }
    }
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BothBoundaries => "exactly one of cursor or time boundary may be supplied",
            Self::NeitherBoundary => "one cursor or time boundary is required",
            Self::InvalidCursor => "the inbound cursor is invalid",
            Self::InvalidTimestamp => "the inbound time boundary is not valid RFC 3339",
            Self::TimestampOutOfRange => {
                "the inbound time boundary is outside the Discord snowflake range"
            }
        })
    }
}

impl Error for BoundaryError {}

/// Converts a validated RFC 3339 timestamp to Unix milliseconds.
pub fn rfc3339_to_unix_millis(value: &str) -> Result<i64, BoundaryError> {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || bytes.len() > 40 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(BoundaryError::InvalidTimestamp);
    }
    if bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' {
        return Err(BoundaryError::InvalidTimestamp);
    }

    let year = parse_number(&bytes[0..4]).ok_or(BoundaryError::InvalidTimestamp)?;
    let month = parse_number(&bytes[5..7]).ok_or(BoundaryError::InvalidTimestamp)?;
    let day = parse_number(&bytes[8..10]).ok_or(BoundaryError::InvalidTimestamp)?;
    let hour = parse_number(&bytes[11..13]).ok_or(BoundaryError::InvalidTimestamp)?;
    let minute = parse_number(&bytes[14..16]).ok_or(BoundaryError::InvalidTimestamp)?;
    let (second_text, offset_seconds) = split_timezone(&bytes[17..])?;
    let (second, fraction_millis) = split_second(second_text)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(BoundaryError::InvalidTimestamp);
    }

    let days = days_from_civil(year, month, day);
    let local_millis = days
        .checked_mul(86_400_000)
        .and_then(|value| value.checked_add(i64::from(hour) * 3_600_000))
        .and_then(|value| value.checked_add(i64::from(minute) * 60_000))
        .and_then(|value| value.checked_add(i64::from(second) * 1_000))
        .and_then(|value| value.checked_add(i64::from(fraction_millis)))
        .ok_or(BoundaryError::TimestampOutOfRange)?;
    local_millis
        .checked_sub(i64::from(offset_seconds) * 1_000)
        .ok_or(BoundaryError::TimestampOutOfRange)
}

/// Converts a validated RFC 3339 timestamp to a lower-bound Discord
/// snowflake suitable for the `after` message query.
pub fn rfc3339_to_discord_snowflake(value: &str) -> Result<String, BoundaryError> {
    const DISCORD_EPOCH_MILLIS: i128 = 1_420_070_400_000;
    const SNOWFLAKE_SEQUENCE_BITS: u32 = 22;

    let millis = i128::from(rfc3339_to_unix_millis(value)?);
    let elapsed = millis
        .checked_sub(DISCORD_EPOCH_MILLIS)
        .ok_or(BoundaryError::TimestampOutOfRange)?;
    if elapsed < 0 {
        return Err(BoundaryError::TimestampOutOfRange);
    }
    let snowflake = elapsed
        .checked_shl(SNOWFLAKE_SEQUENCE_BITS)
        .ok_or(BoundaryError::TimestampOutOfRange)?;
    let value = u64::try_from(snowflake).map_err(|_| BoundaryError::TimestampOutOfRange)?;
    Ok(value.to_string())
}

fn validate_cursor(value: &str) -> Result<(), BoundaryError> {
    if value.is_empty() || value.len() > 20 || value.parse::<u64>().is_err() {
        return Err(BoundaryError::InvalidCursor);
    }
    Ok(())
}

fn parse_number(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
}

fn split_timezone(value: &[u8]) -> Result<(&[u8], i32), BoundaryError> {
    if value.last() == Some(&b'Z') {
        return Ok((&value[..value.len() - 1], 0));
    }
    let sign_position = value
        .iter()
        .rposition(|byte| matches!(byte, b'+' | b'-'))
        .ok_or(BoundaryError::InvalidTimestamp)?;
    if sign_position + 6 != value.len() || value[sign_position + 3] != b':' {
        return Err(BoundaryError::InvalidTimestamp);
    }
    let hours = parse_number(&value[sign_position + 1..sign_position + 3])
        .ok_or(BoundaryError::InvalidTimestamp)?;
    let minutes = parse_number(&value[sign_position + 4..sign_position + 6])
        .ok_or(BoundaryError::InvalidTimestamp)?;
    if hours > 23 || minutes > 59 {
        return Err(BoundaryError::InvalidTimestamp);
    }
    let magnitude =
        i32::try_from(hours * 3_600 + minutes * 60).map_err(|_| BoundaryError::InvalidTimestamp)?;
    let offset = if value[sign_position] == b'+' {
        magnitude
    } else {
        -magnitude
    };
    Ok((&value[..sign_position], offset))
}

fn split_second(value: &[u8]) -> Result<(u32, u32), BoundaryError> {
    let (second_text, fraction_text) = match value.iter().position(|byte| *byte == b'.') {
        Some(position) => (&value[..position], Some(&value[position + 1..])),
        None => (value, None),
    };
    let second = parse_number(second_text).ok_or(BoundaryError::InvalidTimestamp)?;
    let fraction_millis = match fraction_text {
        None => 0,
        Some(fraction) if fraction.is_empty() || fraction.len() > 9 => {
            return Err(BoundaryError::InvalidTimestamp);
        }
        Some(fraction) => {
            if !fraction.iter().all(u8::is_ascii_digit) {
                return Err(BoundaryError::InvalidTimestamp);
            }
            let mut millis = 0_u32;
            for index in 0..3 {
                millis = millis * 10 + fraction.get(index).map_or(0, |byte| u32::from(byte - b'0'));
            }
            millis
        }
    };
    Ok((second, fraction_millis))
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant's civil-date conversion, expressed with i64 so the
// subtraction remains checked for malformed/extreme input.
fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{BoundaryError, FetchBoundary, rfc3339_to_unix_millis};

    #[test]
    fn exactly_one_boundary_is_required() {
        assert_eq!(
            FetchBoundary::from_parts(None, None),
            Err(BoundaryError::NeitherBoundary)
        );
        assert_eq!(
            FetchBoundary::from_parts(Some("123"), Some("2026-01-01T00:00:00Z")),
            Err(BoundaryError::BothBoundaries)
        );
        assert!(FetchBoundary::from_parts(Some("123"), None).is_ok());
        assert!(FetchBoundary::from_parts(None, Some("2026-01-01T00:00:00Z")).is_ok());
        assert!(FetchBoundary::cursor("not-a-snowflake").is_err());
    }

    #[test]
    fn rfc3339_accepts_offsets_and_calendar_boundaries() {
        assert_eq!(rfc3339_to_unix_millis("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(
            rfc3339_to_unix_millis("2024-02-29T23:59:59.123+01:30"),
            Ok(1_709_245_799_123)
        );
        assert!(FetchBoundary::time("2023-02-29T00:00:00Z").is_err());
        assert!(FetchBoundary::time("2026-01-01T00:00:00").is_err());
    }
}
