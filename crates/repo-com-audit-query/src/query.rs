use std::fmt;
use std::ops::Deref;

use repo_com_audit::{AuditEvent, RedactedAuditEvent};
use repo_com_state::{AuditEventInput as StateAuditEventInput, AuditEventRecord, StateStore};
use rusqlite::named_params;
use serde::Serialize;

use crate::AuditQueryError;
use crate::filter::{AuditCursor, AuditFilter, MAX_PAGE_SIZE};

/// One redacted local audit row together with its stable local sequence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuditQueryEvent {
    /// Monotonic local sequence used for stable pagination.
    pub audit_id: i64,
    /// Redacted event envelope. The writer's defensive redaction boundary is
    /// applied again while reading, not trusted blindly at the query surface.
    #[serde(flatten)]
    pub event: RedactedAuditEvent,
}

impl AuditQueryEvent {
    fn from_record(record: AuditEventRecord) -> Result<Self, AuditQueryError> {
        let audit_id = record.audit_id;
        let input = StateAuditEventInput {
            repository_id: record.repository_id,
            event_id: record.event_id,
            object_type: record.object_type,
            object_id: record.object_id,
            transition: record.transition,
            occurred_at: record.occurred_at,
            actor_kind: record.actor_kind,
            outcome: record.outcome,
            metadata_json: record.metadata_json,
        };
        let event = AuditEvent::from_state_input(&input)
            .map_err(|_| AuditQueryError::UnsafeStoredEvidence)?;
        Ok(Self {
            audit_id,
            event: event.redacted(),
        })
    }
}

impl Deref for AuditQueryEvent {
    type Target = RedactedAuditEvent;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

/// Compatibility name emphasizing that this is local evidence, not telemetry.
pub type LocalAuditEvidence = AuditQueryEvent;

/// One bounded page of local audit evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuditPage {
    /// Repository scope echoed explicitly for callers and serializers.
    pub repository_id: String,
    /// Redacted events in stable chronological order.
    pub events: Vec<AuditQueryEvent>,
    /// Effective requested page bound.
    pub page_size: usize,
    /// Whether more matching local rows exist after this page.
    pub truncated: bool,
    /// Continuation for the next matching page, absent on the final page.
    pub next_cursor: Option<AuditCursor>,
}

impl AuditPage {
    /// Returns whether the local result is incomplete.
    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.truncated
    }

    /// Returns the continuation token, if any.
    #[must_use]
    pub const fn continuation(&self) -> Option<&AuditCursor> {
        self.next_cursor.as_ref()
    }

    /// Returns the page rows using a record-oriented name.
    #[must_use]
    pub fn records(&self) -> &[AuditQueryEvent] {
        &self.events
    }
}

/// A read-only, repository-scoped local audit query service.
pub struct AuditQuery<'store> {
    store: &'store StateStore,
}

impl<'store> AuditQuery<'store> {
    /// Creates a query service over an already opened local state store.
    #[must_use]
    pub const fn new(store: &'store StateStore) -> Self {
        Self { store }
    }

    /// Executes one bounded SELECT query. This method has no mutation, remote,
    /// export, or read-receipt operation.
    pub fn query(&self, filter: &AuditFilter) -> Result<AuditPage, AuditQueryError> {
        filter.validate()?;

        let limit = filter.page_size;
        let fetch_limit = limit
            .checked_add(1)
            .ok_or(AuditQueryError::InvalidFilter { field: "page_size" })?;
        let fetch_limit_i64 = i64::try_from(fetch_limit)
            .map_err(|_| AuditQueryError::InvalidFilter { field: "page_size" })?;
        if fetch_limit > MAX_PAGE_SIZE + 1 {
            return Err(AuditQueryError::InvalidFilter { field: "page_size" });
        }

        let connection = self.store.connection();
        let mut statement = connection
            .prepare(
                "SELECT audit_id, repository_id, event_id, object_type, object_id,
                        transition, occurred_at, actor_kind, outcome, metadata_json
                 FROM audit_events
                 WHERE repository_id = :repository_id
                   AND (:occurred_from IS NULL
                        OR julianday(occurred_at) >= julianday(:occurred_from))
                   AND (:occurred_before IS NULL
                        OR julianday(occurred_at) < julianday(:occurred_before))
                   AND (:object_type IS NULL OR object_type = :object_type)
                   AND (:object_id IS NULL OR object_id = :object_id)
                   AND (:transition IS NULL OR transition = :transition)
                   AND (
                       :cursor_time IS NULL
                       OR julianday(occurred_at) > julianday(:cursor_time)
                       OR (
                           julianday(occurred_at) = julianday(:cursor_time)
                           AND audit_id > :cursor_id
                       )
                   )
                 ORDER BY julianday(occurred_at) ASC, audit_id ASC
                 LIMIT :fetch_limit",
            )
            .map_err(AuditQueryError::from)?;

        let cursor_time = filter
            .cursor
            .as_ref()
            .map(|cursor| cursor.occurred_at.as_str());
        let cursor_id = filter.cursor.as_ref().map(|cursor| cursor.audit_id);
        let rows = statement
            .query_map(
                named_params! {
                    ":repository_id": filter.repository_id.as_str(),
                    ":occurred_from": filter.occurred_from.as_deref(),
                    ":occurred_before": filter.occurred_before.as_deref(),
                    ":object_type": filter.object_type.as_deref(),
                    ":object_id": filter.object_id.as_deref(),
                    ":transition": filter.transition.as_deref(),
                    ":cursor_time": cursor_time,
                    ":cursor_id": cursor_id,
                    ":fetch_limit": fetch_limit_i64,
                },
                row_to_record,
            )
            .map_err(AuditQueryError::from)?;

        let mut records = Vec::with_capacity(fetch_limit);
        for row in rows {
            records.push(row.map_err(AuditQueryError::from)?);
        }

        let truncated = records.len() > limit;
        records.truncate(limit);
        let events = records
            .into_iter()
            .map(AuditQueryEvent::from_record)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if truncated {
            let last = events.last().ok_or(AuditQueryError::Storage)?;
            Some(
                AuditCursor::new(
                    filter.repository_id.as_str(),
                    last.occurred_at.as_str(),
                    last.audit_id,
                )
                .map_err(|_| AuditQueryError::Storage)?,
            )
        } else {
            None
        };

        Ok(AuditPage {
            repository_id: filter.repository_id.clone(),
            events,
            page_size: limit,
            truncated,
            next_cursor,
        })
    }

    /// Alias for [`AuditQuery::query`].
    pub fn query_page(&self, filter: &AuditFilter) -> Result<AuditPage, AuditQueryError> {
        self.query(filter)
    }
}

impl fmt::Debug for AuditQuery<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditQuery")
            .field("local_only", &true)
            .finish()
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEventRecord> {
    Ok(AuditEventRecord {
        audit_id: row.get(0)?,
        repository_id: row.get(1)?,
        event_id: row.get(2)?,
        object_type: row.get(3)?,
        object_id: row.get(4)?,
        transition: row.get(5)?,
        occurred_at: row.get(6)?,
        actor_kind: row.get(7)?,
        outcome: row.get(8)?,
        metadata_json: row.get(9)?,
    })
}
