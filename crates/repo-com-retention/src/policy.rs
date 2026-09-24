//! Pure retention policy validation and deterministic UTC cutoff calculation.
//!
//! This module has no state, network, or clock side effects.  Callers provide
//! the current time through [`RetentionClock`], which keeps sweeps reproducible
//! in contract tests and prevents an ambient wall clock from changing a
//! retention decision implicitly.

use std::error::Error;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use repo_com_config::{RepositoryConfig, ResolvedConfig, RetentionConfig};
use serde::{Deserialize, Serialize};

/// The default number of days for which draft and inbound text is retained.
pub const DEFAULT_CONTENT_DAYS: u32 = 30;
/// The default number of days for which non-content metadata is retained.
pub const DEFAULT_METADATA_DAYS: u32 = 365;
/// The inclusive minimum content retention period.
pub const MIN_CONTENT_DAYS: u32 = 1;
/// The inclusive maximum content retention period.
pub const MAX_CONTENT_DAYS: u32 = 365;
/// The inclusive minimum metadata retention period.
pub const MIN_METADATA_DAYS: u32 = 30;
/// The inclusive maximum metadata retention period.
pub const MAX_METADATA_DAYS: u32 = 3_650;
/// The number of seconds in one retention day.
///
/// Retention periods are elapsed UTC days.  The cutoff is inclusive: a row
/// whose timestamp is exactly the cutoff is expired.
pub const SECONDS_PER_DAY: u64 = 86_400;
/// The stable replacement written over expired text.
pub const CONTENT_EXPIRED_MARKER: &str = "[content-expired]";

/// Compatibility names for callers that use the shorter constant spelling.
pub const DEFAULT_CONTENT_RETENTION_DAYS: u32 = DEFAULT_CONTENT_DAYS;
/// Compatibility name for [`DEFAULT_METADATA_DAYS`].
pub const DEFAULT_METADATA_RETENTION_DAYS: u32 = DEFAULT_METADATA_DAYS;
/// Compatibility name for [`MIN_CONTENT_DAYS`].
pub const MIN_CONTENT_RETENTION_DAYS: u32 = MIN_CONTENT_DAYS;
/// Compatibility name for [`MAX_CONTENT_DAYS`].
pub const MAX_CONTENT_RETENTION_DAYS: u32 = MAX_CONTENT_DAYS;
/// Compatibility name for [`MIN_METADATA_DAYS`].
pub const MIN_METADATA_RETENTION_DAYS: u32 = MIN_METADATA_DAYS;
/// Compatibility name for [`MAX_METADATA_DAYS`].
pub const MAX_METADATA_RETENTION_DAYS: u32 = MAX_METADATA_DAYS;

/// A validated content and metadata retention policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    /// Days for which draft and inbound text remains readable.
    pub content_days: u32,
    /// Days for which non-content metadata remains retained.
    pub metadata_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            content_days: DEFAULT_CONTENT_DAYS,
            metadata_days: DEFAULT_METADATA_DAYS,
        }
    }
}

impl RetentionPolicy {
    /// Validates and constructs a policy.
    pub fn new(content_days: u32, metadata_days: u32) -> Result<Self, RetentionPolicyError> {
        let policy = Self {
            content_days,
            metadata_days,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Validates a policy without changing it.
    pub fn validate(&self) -> Result<(), RetentionPolicyError> {
        if !(MIN_CONTENT_DAYS..=MAX_CONTENT_DAYS).contains(&self.content_days) {
            return Err(RetentionPolicyError::InvalidContentDays {
                value: self.content_days,
            });
        }
        if !(MIN_METADATA_DAYS..=MAX_METADATA_DAYS).contains(&self.metadata_days) {
            return Err(RetentionPolicyError::InvalidMetadataDays {
                value: self.metadata_days,
            });
        }
        if self.metadata_days < self.content_days {
            return Err(RetentionPolicyError::MetadataShorterThanContent {
                content_days: self.content_days,
                metadata_days: self.metadata_days,
            });
        }
        Ok(())
    }

    /// Validates the retention section of a repository configuration.
    pub fn from_config(config: &RetentionConfig) -> Result<Self, RetentionPolicyError> {
        Self::new(config.content_days, config.metadata_days)
    }

    /// Validates the retention section of a complete repository configuration.
    pub fn from_repository_config(config: &RepositoryConfig) -> Result<Self, RetentionPolicyError> {
        Self::from_config(&config.retention)
    }

    /// Validates the retention section of a resolved repository configuration.
    pub fn from_resolved_config(config: &ResolvedConfig) -> Result<Self, RetentionPolicyError> {
        Self::from_repository_config(&config.config)
    }

    /// Compatibility alias for [`Self::new`].
    pub fn from_days(content_days: u32, metadata_days: u32) -> Result<Self, RetentionPolicyError> {
        Self::new(content_days, metadata_days)
    }

    /// Returns whether the policy is inside all configured bounds.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    /// Returns the validated content period.
    #[must_use]
    pub const fn content_days(&self) -> u32 {
        self.content_days
    }

    /// Returns the validated metadata period.
    #[must_use]
    pub const fn metadata_days(&self) -> u32 {
        self.metadata_days
    }

    /// Returns the validated content period.
    #[must_use]
    pub const fn content_retention_days(&self) -> u32 {
        self.content_days
    }

    /// Returns the validated metadata period.
    #[must_use]
    pub const fn metadata_retention_days(&self) -> u32 {
        self.metadata_days
    }

    /// Calculates the two inclusive cutoffs for one repository at one instant.
    pub fn cutoffs(
        &self,
        repository_id: impl Into<String>,
        now_unix_seconds: u64,
    ) -> Result<RetentionCutoffs, RetentionPolicyError> {
        calculate_cutoffs(repository_id, now_unix_seconds, self)
    }
}

/// A typed policy validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetentionPolicyError {
    /// Content retention was outside the inclusive 1–365 day range.
    InvalidContentDays {
        /// The rejected day count.
        value: u32,
    },
    /// Metadata retention was outside the inclusive 30–3,650 day range.
    InvalidMetadataDays {
        /// The rejected day count.
        value: u32,
    },
    /// Metadata retention would remove evidence before content retention.
    MetadataShorterThanContent {
        /// Configured content period.
        content_days: u32,
        /// Configured metadata period.
        metadata_days: u32,
    },
    /// The supplied repository ID was empty, too long, or contained NUL.
    InvalidRepositoryId,
    /// Unix-time arithmetic could not represent a cutoff.
    CutoffUnderflow,
    /// The clock value cannot be represented as canonical UTC text.
    ClockOutOfRange,
    /// A caller supplied a malformed clock timestamp.
    InvalidClockTimestamp,
}

impl fmt::Display for RetentionPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContentDays { value } => {
                write!(
                    formatter,
                    "content retention must be 1-365 days (got {value})"
                )
            }
            Self::InvalidMetadataDays { value } => write!(
                formatter,
                "metadata retention must be 30-3650 days (got {value})"
            ),
            Self::MetadataShorterThanContent {
                content_days,
                metadata_days,
            } => write!(
                formatter,
                "metadata retention ({metadata_days} days) cannot be shorter than content retention ({content_days} days)"
            ),
            Self::InvalidRepositoryId => {
                formatter.write_str("repository ID is invalid for retention")
            }
            Self::CutoffUnderflow => {
                formatter.write_str("retention cutoff is before the Unix epoch")
            }
            Self::ClockOutOfRange => {
                formatter.write_str("retention clock is outside supported UTC range")
            }
            Self::InvalidClockTimestamp => {
                formatter.write_str("retention clock timestamp is invalid")
            }
        }
    }
}

impl Error for RetentionPolicyError {}

/// A deterministic instant expressed as Unix seconds and canonical UTC text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetentionInstant {
    unix_seconds: u64,
    utc: String,
}

impl RetentionInstant {
    /// Constructs an instant from Unix seconds and canonical UTC text.
    pub fn new(unix_seconds: u64, utc: impl Into<String>) -> Result<Self, RetentionPolicyError> {
        let expected =
            format_rfc3339_utc(unix_seconds).ok_or(RetentionPolicyError::ClockOutOfRange)?;
        let utc = utc.into();
        if utc != expected {
            return Err(RetentionPolicyError::InvalidClockTimestamp);
        }
        Ok(Self { unix_seconds, utc })
    }

    /// Returns Unix seconds.
    #[must_use]
    pub const fn unix_seconds(&self) -> u64 {
        self.unix_seconds
    }

    /// Returns canonical UTC RFC 3339 text.
    #[must_use]
    pub fn utc(&self) -> &str {
        &self.utc
    }
}

/// An injected source of current time.
pub trait RetentionClock {
    /// Returns the current time as seconds since the Unix epoch.
    fn now_unix_seconds(&self) -> u64;

    /// Returns the current time as seconds since the Unix epoch.
    ///
    /// This short alias keeps the clock boundary convenient for callers that
    /// use the same terminology as the retry and delivery subsystems.
    fn now(&self) -> u64 {
        self.now_unix_seconds()
    }
}

impl<T> RetentionClock for &T
where
    T: RetentionClock + ?Sized,
{
    fn now_unix_seconds(&self) -> u64 {
        (**self).now_unix_seconds()
    }
}

/// Compatibility alias for [`RetentionClock`].
pub use RetentionClock as Clock;

/// A clock fixed at an injected Unix timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedClock {
    now_unix_seconds: u64,
}

impl FixedClock {
    /// Creates a fixed clock.
    #[must_use]
    pub const fn new(now_unix_seconds: u64) -> Self {
        Self { now_unix_seconds }
    }

    /// Creates a fixed clock from an explicit Unix timestamp.
    #[must_use]
    pub const fn from_unix_seconds(now_unix_seconds: u64) -> Self {
        Self::new(now_unix_seconds)
    }

    /// Creates a fixed clock from canonical UTC text.
    pub fn from_rfc3339(value: &str) -> Result<Self, RetentionPolicyError> {
        let seconds =
            parse_rfc3339_utc(value).ok_or(RetentionPolicyError::InvalidClockTimestamp)?;
        Ok(Self::new(seconds))
    }

    /// Returns the fixed timestamp.
    #[must_use]
    pub const fn now(self) -> u64 {
        self.now_unix_seconds
    }
}

impl RetentionClock for FixedClock {
    fn now_unix_seconds(&self) -> u64 {
        self.now_unix_seconds
    }
}

/// A mutable clock useful for tests that need to advance time explicitly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualClock {
    now_unix_seconds: u64,
}

impl ManualClock {
    /// Creates a manual clock.
    #[must_use]
    pub const fn new(now_unix_seconds: u64) -> Self {
        Self { now_unix_seconds }
    }

    /// Returns the current injected timestamp.
    #[must_use]
    pub const fn now(&self) -> u64 {
        self.now_unix_seconds
    }

    /// Advances the clock by a duration.
    pub fn advance(&mut self, duration: Duration) {
        self.now_unix_seconds = self.now_unix_seconds.saturating_add(duration.as_secs());
    }
}

impl RetentionClock for ManualClock {
    fn now_unix_seconds(&self) -> u64 {
        self.now_unix_seconds
    }
}

/// A local wall-clock source.  Production callers may use this, while tests
/// should prefer [`FixedClock`] or [`ManualClock`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemClock;

impl RetentionClock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs())
    }
}

/// The two deterministic retention cutoffs for one repository invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionCutoffs {
    /// Exact repository scope.
    pub repository_id: String,
    /// Current time used for the sweep, in Unix seconds.
    pub as_of_unix_seconds: u64,
    /// Current time in canonical UTC RFC 3339 form.
    pub as_of: String,
    /// Inclusive content cutoff in Unix seconds.
    pub content_cutoff_unix_seconds: u64,
    /// Inclusive content cutoff in canonical UTC form.
    pub content_cutoff: String,
    /// Inclusive metadata cutoff in Unix seconds.
    pub metadata_cutoff_unix_seconds: u64,
    /// Inclusive metadata cutoff in canonical UTC form.
    pub metadata_cutoff: String,
}

impl RetentionCutoffs {
    /// Returns the current instant used for the sweep.
    #[must_use]
    pub fn as_of_instant(&self) -> RetentionInstant {
        RetentionInstant {
            unix_seconds: self.as_of_unix_seconds,
            utc: self.as_of.clone(),
        }
    }

    /// Returns the content cutoff as canonical UTC text.
    #[must_use]
    pub fn content_cutoff_utc(&self) -> &str {
        &self.content_cutoff
    }

    /// Returns the metadata cutoff as canonical UTC text.
    #[must_use]
    pub fn metadata_cutoff_utc(&self) -> &str {
        &self.metadata_cutoff
    }

    /// Returns the content cutoff as an instant.
    #[must_use]
    pub fn content_cutoff_instant(&self) -> RetentionInstant {
        RetentionInstant {
            unix_seconds: self.content_cutoff_unix_seconds,
            utc: self.content_cutoff.clone(),
        }
    }

    /// Returns the metadata cutoff as an instant.
    #[must_use]
    pub fn metadata_cutoff_instant(&self) -> RetentionInstant {
        RetentionInstant {
            unix_seconds: self.metadata_cutoff_unix_seconds,
            utc: self.metadata_cutoff.clone(),
        }
    }
}

/// Compatibility name matching the feature interface.
pub type Cutoffs = RetentionCutoffs;
/// Compatibility name for an invocation's clock/repository context.
pub type RetentionContext = RetentionCutoffs;

/// Calculates inclusive cutoffs after validating the policy and repository.
pub fn calculate_cutoffs(
    repository_id: impl Into<String>,
    now_unix_seconds: u64,
    policy: &RetentionPolicy,
) -> Result<RetentionCutoffs, RetentionPolicyError> {
    policy.validate()?;
    let repository_id = repository_id.into();
    validate_repository_id(&repository_id)?;
    let content_seconds = u64::from(policy.content_days)
        .checked_mul(SECONDS_PER_DAY)
        .ok_or(RetentionPolicyError::CutoffUnderflow)?;
    let metadata_seconds = u64::from(policy.metadata_days)
        .checked_mul(SECONDS_PER_DAY)
        .ok_or(RetentionPolicyError::CutoffUnderflow)?;
    let content_cutoff_unix_seconds = now_unix_seconds
        .checked_sub(content_seconds)
        .ok_or(RetentionPolicyError::CutoffUnderflow)?;
    let metadata_cutoff_unix_seconds = now_unix_seconds
        .checked_sub(metadata_seconds)
        .ok_or(RetentionPolicyError::CutoffUnderflow)?;
    let as_of =
        format_rfc3339_utc(now_unix_seconds).ok_or(RetentionPolicyError::ClockOutOfRange)?;
    let content_cutoff = format_rfc3339_utc(content_cutoff_unix_seconds)
        .ok_or(RetentionPolicyError::ClockOutOfRange)?;
    let metadata_cutoff = format_rfc3339_utc(metadata_cutoff_unix_seconds)
        .ok_or(RetentionPolicyError::ClockOutOfRange)?;
    Ok(RetentionCutoffs {
        repository_id,
        as_of_unix_seconds: now_unix_seconds,
        as_of,
        content_cutoff_unix_seconds,
        content_cutoff,
        metadata_cutoff_unix_seconds,
        metadata_cutoff,
    })
}

/// Parses a UTC RFC 3339 timestamp into Unix seconds.
///
/// Fractional seconds are accepted and truncated only after the date/time and
/// offset have been validated.  Callers that need sub-second ordering should
/// use the internal instant conversion during a sweep; storage timestamps in
/// v1 are canonical second timestamps.
pub fn parse_rfc3339_utc(value: &str) -> Option<u64> {
    let nanos = parse_rfc3339_nanos(value)?;
    u64::try_from(nanos.div_euclid(1_000_000_000)).ok()
}

/// Parses a canonical UTC timestamp with a `Z` suffix.
pub fn parse_canonical_utc(value: &str) -> Option<u64> {
    if !value.ends_with('Z') {
        return None;
    }
    parse_rfc3339_utc(value)
}

/// Formats Unix seconds as canonical UTC RFC 3339 text.
pub fn format_rfc3339_utc(unix_seconds: u64) -> Option<String> {
    let total = i64::try_from(unix_seconds).ok()?;
    let days = total.div_euclid(i64::try_from(SECONDS_PER_DAY).ok()?);
    let seconds = total.rem_euclid(i64::try_from(SECONDS_PER_DAY).ok()?);
    let (year, month, day) = civil_from_days(days);
    if !(1970..=9999).contains(&year) {
        return None;
    }
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn validate_repository_id(value: &str) -> Result<(), RetentionPolicyError> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') {
        return Err(RetentionPolicyError::InvalidRepositoryId);
    }
    Ok(())
}

pub(crate) fn parse_rfc3339_nanos(value: &str) -> Option<i128> {
    if !value.is_ascii() || value.len() < 20 {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = decimal(bytes, 0, 4)?;
    let month = decimal(bytes, 5, 2)?;
    let day = decimal(bytes, 8, 2)?;
    let hour = decimal(bytes, 11, 2)?;
    let minute = decimal(bytes, 14, 2)?;
    let second = decimal(bytes, 17, 2)?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let mut suffix = &value[19..];
    let mut fractional_nanos = 0_i128;
    if let Some(rest) = suffix.strip_prefix('.') {
        let fraction_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        if fraction_len == 0 || fraction_len > 9 {
            return None;
        }
        let fraction = &rest[..fraction_len];
        let mut normalized = fraction.to_owned();
        while normalized.len() < 9 {
            normalized.push('0');
        }
        fractional_nanos = normalized.parse::<i128>().ok()?;
        suffix = &rest[fraction_len..];
    }
    let offset_seconds = match suffix {
        "Z" | "z" => 0,
        value if value.len() == 6 && (value.starts_with('+') || value.starts_with('-')) => {
            let sign = if value.starts_with('-') { -1 } else { 1 };
            let hours = decimal(value.as_bytes(), 1, 2)?;
            let minutes = decimal(value.as_bytes(), 4, 2)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            sign * i64::try_from(hours * 3_600 + minutes * 60).ok()?
        }
        _ => return None,
    };
    let days = days_from_civil(
        i64::try_from(year).ok()?,
        i64::try_from(month).ok()?,
        i64::try_from(day).ok()?,
    );
    let seconds = days.checked_mul(i64::try_from(SECONDS_PER_DAY).ok()?)?;
    let seconds = seconds.checked_add(i64::try_from(hour * 3_600 + minute * 60 + second).ok()?)?;
    let seconds = seconds.checked_sub(offset_seconds)?;
    let seconds = i128::from(seconds);
    seconds
        .checked_mul(1_000_000_000)?
        .checked_add(fractional_nanos)
}

fn decimal(bytes: &[u8], start: usize, length: usize) -> Option<u64> {
    let end = start.checked_add(length)?;
    let slice = bytes.get(start..end)?;
    if slice.is_empty() || !slice.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(slice).ok()?.parse().ok()
}

fn is_leap_year(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (
        u64::try_from(year).expect("supported retention years are positive"),
        u64::try_from(month).expect("civil month is positive"),
        u64::try_from(day).expect("civil day is positive"),
    )
}
