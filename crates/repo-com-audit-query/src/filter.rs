use std::fmt;

use repo_com_audit::redact_text;
use serde::Serialize;

use crate::AuditQueryError;

/// Default number of local evidence rows returned by one query.
pub const DEFAULT_PAGE_SIZE: usize = 50;

/// Maximum number of local evidence rows returned by one query.
pub const MAX_PAGE_SIZE: usize = 100;

/// A stable continuation point for chronological audit pagination.
///
/// The repository ID is part of the cursor so a continuation from one local
/// evidence scope cannot silently be replayed against another.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct AuditCursor {
    /// Repository that owns the last returned event.
    pub repository_id: String,
    /// Canonical UTC timestamp of the last returned event.
    pub occurred_at: String,
    /// Monotonic local sequence used as the same-timestamp tie breaker.
    pub audit_id: i64,
}

impl AuditCursor {
    /// Creates a validated continuation point.
    pub fn new(
        repository_id: impl Into<String>,
        occurred_at: impl Into<String>,
        audit_id: i64,
    ) -> Result<Self, AuditQueryError> {
        let cursor = Self {
            repository_id: repository_id.into(),
            occurred_at: occurred_at.into(),
            audit_id,
        };
        cursor.validate()?;
        Ok(cursor)
    }

    /// Validates the cursor without exposing any field value in the error.
    pub fn validate(&self) -> Result<(), AuditQueryError> {
        validate_identifier("repository_id", &self.repository_id)
            .map_err(|_| AuditQueryError::InvalidCursor)?;
        parse_timestamp(&self.occurred_at).ok_or(AuditQueryError::InvalidCursor)?;
        if self.audit_id <= 0 {
            return Err(AuditQueryError::InvalidCursor);
        }
        Ok(())
    }

    pub(crate) fn validate_for(&self, repository_id: &str) -> Result<(), AuditQueryError> {
        self.validate()?;
        if self.repository_id != repository_id {
            return Err(AuditQueryError::InvalidCursor);
        }
        Ok(())
    }
}

impl fmt::Debug for AuditCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditCursor")
            .field("valid", &self.validate().is_ok())
            .field("audit_id", &self.audit_id)
            .finish()
    }
}

/// Compatibility name for callers that model a cursor as a continuation token.
pub type ContinuationToken = AuditCursor;

/// Repository-scoped filters for one bounded local audit query.
///
/// `occurred_from` is inclusive and `occurred_before` is exclusive. Optional
/// object and transition filters are exact matches. The repository filter is
/// mandatory and every other condition is combined with it using bound SQL
/// parameters.
#[derive(Clone, Eq, PartialEq)]
pub struct AuditFilter {
    /// Required repository scope.
    pub repository_id: String,
    /// Inclusive lower UTC timestamp bound.
    pub occurred_from: Option<String>,
    /// Exclusive upper UTC timestamp bound.
    pub occurred_before: Option<String>,
    /// Exact object-type match.
    pub object_type: Option<String>,
    /// Exact object-ID match.
    pub object_id: Option<String>,
    /// Exact transition match.
    pub transition: Option<String>,
    /// Requested page size, constrained by [`MAX_PAGE_SIZE`].
    pub page_size: usize,
    /// Optional stable continuation point.
    pub cursor: Option<AuditCursor>,
}

impl AuditFilter {
    /// Creates a repository-only filter with the default page size.
    #[must_use]
    pub fn new(repository_id: impl Into<String>) -> Self {
        Self {
            repository_id: repository_id.into(),
            occurred_from: None,
            occurred_before: None,
            object_type: None,
            object_id: None,
            transition: None,
            page_size: DEFAULT_PAGE_SIZE,
            cursor: None,
        }
    }

    /// Sets an inclusive/exclusive chronological time range.
    #[must_use]
    pub fn with_time_range(
        mut self,
        occurred_from: impl Into<String>,
        occurred_before: impl Into<String>,
    ) -> Self {
        self.occurred_from = Some(occurred_from.into());
        self.occurred_before = Some(occurred_before.into());
        self
    }

    /// Alias for [`AuditFilter::with_time_range`].
    #[must_use]
    pub fn time_range(
        self,
        occurred_from: impl Into<String>,
        occurred_before: impl Into<String>,
    ) -> Self {
        self.with_time_range(occurred_from, occurred_before)
    }

    /// Sets an exact object-type filter.
    #[must_use]
    pub fn with_object_type(mut self, object_type: impl Into<String>) -> Self {
        self.object_type = Some(object_type.into());
        self
    }

    /// Alias for [`AuditFilter::with_object_type`].
    #[must_use]
    pub fn object_type(self, object_type: impl Into<String>) -> Self {
        self.with_object_type(object_type)
    }

    /// Sets an exact object-ID filter.
    #[must_use]
    pub fn with_object_id(mut self, object_id: impl Into<String>) -> Self {
        self.object_id = Some(object_id.into());
        self
    }

    /// Alias for [`AuditFilter::with_object_id`].
    #[must_use]
    pub fn object_id(self, object_id: impl Into<String>) -> Self {
        self.with_object_id(object_id)
    }

    /// Sets an exact transition filter.
    #[must_use]
    pub fn with_transition(mut self, transition: impl Into<String>) -> Self {
        self.transition = Some(transition.into());
        self
    }

    /// Alias for [`AuditFilter::with_transition`].
    #[must_use]
    pub fn transition(self, transition: impl Into<String>) -> Self {
        self.with_transition(transition)
    }

    /// Sets the requested page size. Invalid sizes are rejected at query time.
    #[must_use]
    pub const fn with_page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size;
        self
    }

    /// Alias for [`AuditFilter::with_page_size`].
    #[must_use]
    pub const fn page_size(self, page_size: usize) -> Self {
        self.with_page_size(page_size)
    }

    /// Sets a stable continuation point.
    #[must_use]
    pub fn with_cursor(mut self, cursor: AuditCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Alias for [`AuditFilter::with_cursor`].
    #[must_use]
    pub fn cursor(self, cursor: AuditCursor) -> Self {
        self.with_cursor(cursor)
    }

    /// Validates all filters before any SQL statement is prepared.
    pub fn validate(&self) -> Result<(), AuditQueryError> {
        validate_identifier("repository_id", &self.repository_id)?;

        if self.page_size == 0 || self.page_size > MAX_PAGE_SIZE {
            return Err(AuditQueryError::InvalidFilter { field: "page_size" });
        }

        for (field, value) in [
            ("object_type", self.object_type.as_deref()),
            ("object_id", self.object_id.as_deref()),
            ("transition", self.transition.as_deref()),
        ] {
            if let Some(value) = value {
                validate_identifier(field, value)?;
            }
        }

        let from = match self.occurred_from.as_deref() {
            Some(value) => Some(
                parse_timestamp(value).ok_or(AuditQueryError::InvalidFilter {
                    field: "occurred_from",
                })?,
            ),
            None => None,
        };
        let before = match self.occurred_before.as_deref() {
            Some(value) => Some(
                parse_timestamp(value).ok_or(AuditQueryError::InvalidFilter {
                    field: "occurred_before",
                })?,
            ),
            None => None,
        };
        if from.is_some_and(|from| before.as_ref().is_some_and(|before| before < &from)) {
            return Err(AuditQueryError::InvalidFilter {
                field: "occurred_before",
            });
        }

        if let Some(cursor) = &self.cursor {
            cursor.validate_for(&self.repository_id)?;
        }

        Ok(())
    }
}

impl fmt::Debug for AuditFilter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditFilter")
            .field("repository_bound", &true)
            .field(
                "has_time_range",
                &(self.occurred_from.is_some() || self.occurred_before.is_some()),
            )
            .field("has_object_type", &self.object_type.is_some())
            .field("has_object_id", &self.object_id.is_some())
            .field("has_transition", &self.transition.is_some())
            .field("page_size", &self.page_size)
            .field("has_cursor", &self.cursor.is_some())
            .finish()
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), AuditQueryError> {
    let safe = !value.is_empty()
        && value.len() <= 512
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.' | '/' | ':' | '@' | '+' | '#')
        })
        && redact_text(value) == value;
    if safe {
        Ok(())
    } else {
        Err(AuditQueryError::InvalidFilter { field })
    }
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct TimestampKey {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    fraction: Box<str>,
}

fn parse_timestamp(value: &str) -> Option<TimestampKey> {
    let bytes = value.as_bytes();
    if value.len() < 20
        || value.len() > 64
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || !bytes[0..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
        || !bytes[11..13].iter().all(u8::is_ascii_digit)
        || !bytes[14..16].iter().all(u8::is_ascii_digit)
        || !bytes[17..19].iter().all(u8::is_ascii_digit)
    {
        return None;
    }

    let year = number(&bytes[0..4]);
    let month = number(&bytes[5..7]);
    let day = number(&bytes[8..10]);
    let hour = number(&bytes[11..13]);
    let minute = number(&bytes[14..16]);
    let second = number(&bytes[17..19]);
    if year == 0
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let fraction = if value.len() == 20 {
        if bytes[19] != b'Z' {
            return None;
        }
        ""
    } else {
        if value.len() < 22 || bytes[19] != b'.' || bytes[value.len() - 1] != b'Z' {
            return None;
        }
        let digits = &bytes[20..value.len() - 1];
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let value = std::str::from_utf8(digits).ok()?.trim_end_matches('0');
        if value.is_empty() { "" } else { value }
    };

    Some(TimestampKey {
        year,
        month,
        day,
        hour,
        minute,
        second,
        fraction: fraction.into(),
    })
}

fn number(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}
