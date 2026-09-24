use std::fmt;

use repo_com_state::{
    AuditEventInput as StateAuditEventInput, AuditEventRecord as StateAuditEventRecord,
};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::redact::{REDACTED, redact_metadata, redact_metadata_checked, redact_text};
use crate::{AuditError, AuditResult};

const MAX_IDENTIFIER_LENGTH: usize = 512;
const MAX_TIMESTAMP_LENGTH: usize = 64;

/// One local, append-only lifecycle transition.
///
/// The event owns a redacted copy of metadata. Callers may still construct an
/// event from raw JSON for validation, but persistence always runs the
/// redaction boundary again immediately before writing.
#[derive(Clone, Eq, PartialEq)]
pub struct AuditEvent {
    /// Repository scope. The value is part of the local evidence key.
    pub repository_id: String,
    /// Stable event identifier supplied by the lifecycle owner.
    pub event_id: String,
    /// Stable object family, such as `draft` or `inbound_item`.
    pub object_type: String,
    /// Stable object identifier within the repository.
    pub object_id: String,
    /// Transition being recorded.
    pub transition: String,
    /// Canonical UTC timestamp in RFC 3339 `Z` form.
    pub occurred_at: String,
    /// Stable actor kind, such as `operator`, `system`, or `agent`.
    pub actor_kind: String,
    /// Stable outcome category.
    pub outcome: String,
    /// Structured metadata after defensive redaction.
    pub metadata: Value,
}

impl fmt::Debug for AuditEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.redacted(), formatter)
    }
}

impl Serialize for AuditEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.redacted().serialize(serializer)
    }
}

/// A serialization-safe audit event.
///
/// This type is the only event representation exposed to diagnostic and
/// protocol serializers.
#[derive(Clone, Eq, PartialEq)]
pub struct RedactedAuditEvent {
    /// Repository scope.
    pub repository_id: String,
    /// Stable event identifier.
    pub event_id: String,
    /// Stable object family.
    pub object_type: String,
    /// Stable object identifier.
    pub object_id: String,
    /// Transition being recorded.
    pub transition: String,
    /// Canonical UTC timestamp.
    pub occurred_at: String,
    /// Stable actor kind.
    pub actor_kind: String,
    /// Stable outcome category.
    pub outcome: String,
    /// Redacted structured metadata.
    pub metadata: Value,
}

impl RedactedAuditEvent {
    fn sanitized(&self) -> Self {
        Self {
            repository_id: safe_envelope_value(&self.repository_id),
            event_id: safe_envelope_value(&self.event_id),
            object_type: safe_envelope_value(&self.object_type),
            object_id: safe_envelope_value(&self.object_id),
            transition: safe_envelope_value(&self.transition),
            occurred_at: safe_envelope_value(&self.occurred_at),
            actor_kind: safe_envelope_value(&self.actor_kind),
            outcome: safe_envelope_value(&self.outcome),
            metadata: redact_metadata(&self.metadata),
        }
    }
}

impl fmt::Debug for RedactedAuditEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let safe = self.sanitized();
        formatter
            .debug_struct("RedactedAuditEvent")
            .field("repository_id", &safe.repository_id)
            .field("event_id", &safe.event_id)
            .field("object_type", &safe.object_type)
            .field("object_id", &safe.object_id)
            .field("transition", &safe.transition)
            .field("occurred_at", &safe.occurred_at)
            .field("actor_kind", &safe.actor_kind)
            .field("outcome", &safe.outcome)
            .field("metadata", &safe.metadata)
            .finish()
    }
}

impl Serialize for RedactedAuditEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let safe = self.sanitized();
        let mut state = serializer.serialize_struct("RedactedAuditEvent", 9)?;
        state.serialize_field("repository_id", &safe.repository_id)?;
        state.serialize_field("event_id", &safe.event_id)?;
        state.serialize_field("object_type", &safe.object_type)?;
        state.serialize_field("object_id", &safe.object_id)?;
        state.serialize_field("transition", &safe.transition)?;
        state.serialize_field("occurred_at", &safe.occurred_at)?;
        state.serialize_field("actor_kind", &safe.actor_kind)?;
        state.serialize_field("outcome", &safe.outcome)?;
        state.serialize_field("metadata", &safe.metadata)?;
        state.end()
    }
}

/// Compatibility name for consumers that call the envelope an input.
pub type AuditEventEnvelope = AuditEvent;

/// Compatibility name matching the persistence input terminology.
pub type AuditEventInput = AuditEvent;

impl AuditEvent {
    /// Creates an event with an empty metadata object.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        event_id: impl Into<String>,
        object_type: impl Into<String>,
        object_id: impl Into<String>,
        transition: impl Into<String>,
        occurred_at: impl Into<String>,
        actor_kind: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self::with_metadata(
            repository_id,
            event_id,
            object_type,
            object_id,
            transition,
            occurred_at,
            actor_kind,
            outcome,
            Value::Object(Map::new()),
        )
    }

    /// Creates an event and immediately stores only a redacted metadata copy.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn with_metadata<M: Into<Value>>(
        repository_id: impl Into<String>,
        event_id: impl Into<String>,
        object_type: impl Into<String>,
        object_id: impl Into<String>,
        transition: impl Into<String>,
        occurred_at: impl Into<String>,
        actor_kind: impl Into<String>,
        outcome: impl Into<String>,
        metadata: M,
    ) -> Self {
        let metadata = metadata.into();
        let metadata = if metadata.is_null() {
            Value::Object(Map::new())
        } else {
            redact_metadata(&metadata)
        };
        Self {
            repository_id: repository_id.into(),
            event_id: event_id.into(),
            object_type: object_type.into(),
            object_id: object_id.into(),
            transition: transition.into(),
            occurred_at: occurred_at.into(),
            actor_kind: actor_kind.into(),
            outcome: outcome.into(),
            metadata,
        }
    }

    /// Descriptive alias for [`AuditEvent::with_metadata`].
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new_with_metadata<M: Into<Value>>(
        repository_id: impl Into<String>,
        event_id: impl Into<String>,
        object_type: impl Into<String>,
        object_id: impl Into<String>,
        transition: impl Into<String>,
        occurred_at: impl Into<String>,
        actor_kind: impl Into<String>,
        outcome: impl Into<String>,
        metadata: M,
    ) -> Self {
        Self::with_metadata(
            repository_id,
            event_id,
            object_type,
            object_id,
            transition,
            occurred_at,
            actor_kind,
            outcome,
            metadata,
        )
    }

    /// Returns a builder-style copy with a different metadata value.
    #[must_use]
    pub fn with_metadata_value<M: Into<Value>>(mut self, metadata: M) -> Self {
        let metadata = metadata.into();
        self.metadata = if metadata.is_null() {
            Value::Object(Map::new())
        } else {
            redact_metadata(&metadata)
        };
        self
    }

    /// Alias for [`AuditEvent::with_metadata`] using builder terminology.
    #[must_use]
    pub fn metadata<M: Into<Value>>(self, metadata: M) -> Self {
        self.with_metadata_value(metadata)
    }

    /// Converts a state input into a validated, redacted audit event.
    pub fn from_state_input(input: &StateAuditEventInput) -> AuditResult<Self> {
        let metadata = serde_json::from_str::<Value>(&input.metadata_json)
            .map_err(|_| AuditError::InvalidMetadata)?;
        let event = Self::with_metadata(
            input.repository_id.clone(),
            input.event_id.clone(),
            input.object_type.clone(),
            input.object_id.clone(),
            input.transition.clone(),
            input.occurred_at.clone(),
            input.actor_kind.clone(),
            input.outcome.clone(),
            metadata,
        );
        event.validate()?;
        Ok(event)
    }

    /// Returns a safe event representation for diagnostics and serialization.
    #[must_use]
    pub fn redacted(&self) -> RedactedAuditEvent {
        RedactedAuditEvent {
            repository_id: safe_envelope_value(&self.repository_id),
            event_id: safe_envelope_value(&self.event_id),
            object_type: safe_envelope_value(&self.object_type),
            object_id: safe_envelope_value(&self.object_id),
            transition: safe_envelope_value(&self.transition),
            occurred_at: safe_envelope_value(&self.occurred_at),
            actor_kind: safe_envelope_value(&self.actor_kind),
            outcome: safe_envelope_value(&self.outcome),
            metadata: redact_metadata(&self.metadata),
        }
    }

    /// Alias for [`AuditEvent::redacted`].
    #[must_use]
    pub fn safe(&self) -> RedactedAuditEvent {
        self.redacted()
    }

    /// Returns the redacted metadata JSON representation.
    pub fn metadata_json(&self) -> AuditResult<String> {
        Ok(serde_json::to_string(&self.redacted().metadata)?)
    }

    /// Validates all envelope fields without persisting anything.
    pub fn validate(&self) -> AuditResult<()> {
        for (field, value) in [
            ("repository_id", &self.repository_id),
            ("event_id", &self.event_id),
            ("object_type", &self.object_type),
            ("object_id", &self.object_id),
            ("transition", &self.transition),
            ("actor_kind", &self.actor_kind),
            ("outcome", &self.outcome),
        ] {
            validate_identifier(field, value)?;
        }
        validate_timestamp(&self.occurred_at)?;
        if !self.metadata.is_object() {
            return Err(AuditError::InvalidMetadata);
        }
        Ok(())
    }

    /// Converts this event into the state crate's redacted persistence input.
    pub fn to_state_input(&self) -> AuditResult<StateAuditEventInput> {
        self.validate()?;
        let safe = self.redacted();
        let metadata = redact_metadata_checked(&safe.metadata)?;
        Ok(StateAuditEventInput {
            repository_id: safe.repository_id,
            event_id: safe.event_id,
            object_type: safe.object_type,
            object_id: safe.object_id,
            transition: safe.transition,
            occurred_at: safe.occurred_at,
            actor_kind: safe.actor_kind,
            outcome: safe.outcome,
            metadata_json: serde_json::to_string(&metadata)?,
        })
    }
}

impl From<&StateAuditEventRecord> for AuditEvent {
    fn from(record: &StateAuditEventRecord) -> Self {
        let metadata = serde_json::from_str(&record.metadata_json)
            .unwrap_or_else(|_| Value::Object(Map::new()));
        Self::with_metadata(
            record.repository_id.clone(),
            record.event_id.clone(),
            record.object_type.clone(),
            record.object_id.clone(),
            record.transition.clone(),
            record.occurred_at.clone(),
            record.actor_kind.clone(),
            record.outcome.clone(),
            metadata,
        )
    }
}

fn validate_identifier(field: &'static str, value: &str) -> AuditResult<()> {
    if !is_safe_identifier_text(value) {
        return Err(AuditError::InvalidEvent { field });
    }
    Ok(())
}

fn is_safe_identifier_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_LENGTH
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.' | '/' | ':' | '@' | '+' | '#')
        })
        && redact_text(value) == value
}

fn validate_timestamp(value: &str) -> AuditResult<()> {
    if !is_utc_timestamp(value) {
        return Err(AuditError::InvalidTimestamp);
    }
    Ok(())
}

fn is_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if value.len() < 20
        || value.len() > MAX_TIMESTAMP_LENGTH
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
        return false;
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
        return false;
    }

    if value.len() == 20 {
        return bytes[19] == b'Z';
    }
    if value.len() < 22 || bytes[19] != b'.' || bytes[value.len() - 1] != b'Z' {
        return false;
    }
    bytes[20..value.len() - 1].iter().all(u8::is_ascii_digit)
        && !bytes[20..value.len() - 1].is_empty()
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

fn safe_envelope_value(value: &str) -> String {
    if is_safe_identifier_text(value) {
        value.to_owned()
    } else {
        REDACTED.to_owned()
    }
}
