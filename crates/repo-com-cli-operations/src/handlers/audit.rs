//! Bounded local audit inspection command port.

use repo_com_audit_query::{AuditFilter, AuditPage, AuditQuery, AuditQueryError};
use repo_com_foundation::RepoComError;
use repo_com_terminal_operations::{AuditView, LocalErrorView};

use crate::OperationsResult;
use crate::input::AuditQueryInput;

/// A validated repository-scoped audit query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditQueryRequest {
    /// The exact filter passed to the audit owner.
    pub filter: AuditFilter,
}

impl TryFrom<AuditQueryInput> for AuditQueryRequest {
    type Error = RepoComError;

    fn try_from(value: AuditQueryInput) -> Result<Self, Self::Error> {
        let page_size = value.page_size()?;
        let mut filter = AuditFilter::new(&value.repository_id).with_page_size(page_size);
        filter.object_type = value.object_type;
        filter.object_id = value.object_id;
        filter.transition = value.transition;
        filter.occurred_from = value.occurred_from;
        filter.occurred_before = value.occurred_before;
        if let Some(cursor) = value.cursor {
            let parts: Vec<&str> = cursor.split('|').collect();
            if parts.len() != 3 || parts[0] != value.repository_id {
                return Err(RepoComError::usage(
                    "audit cursor is invalid for this repository",
                ));
            }
            let audit_id = parts[2]
                .parse::<i64>()
                .map_err(|_| RepoComError::usage("audit cursor is invalid"))?;
            let cursor = repo_com_audit_query::AuditCursor::new(parts[0], parts[1], audit_id)
                .map_err(|_| RepoComError::usage("audit cursor is invalid"))?;
            filter = filter.with_cursor(cursor);
        }
        filter
            .validate()
            .map_err(|_| RepoComError::usage("audit query filter is invalid"))?;
        Ok(Self { filter })
    }
}

/// Read-only domain port for bounded local audit queries.
pub trait AuditService {
    /// Executes one bounded local query and returns redacted evidence.
    fn query(&mut self, request: AuditQueryRequest) -> OperationsResult<AuditView>;
}

/// Routes one audit query to its read-only owner.
pub fn query<S>(service: &mut S, request: AuditQueryRequest) -> OperationsResult<AuditView>
where
    S: AuditService + ?Sized,
{
    request
        .filter
        .validate()
        .map_err(|_| RepoComError::usage("audit query filter is invalid"))?;
    service.query(request)
}

/// Converts a safe audit-query failure into a command error.
#[must_use]
pub fn map_audit_error(error: &AuditQueryError) -> RepoComError {
    let view = LocalErrorView::from_audit_error(error);
    RepoComError::new(
        view.category,
        format!("{}; next action: {}", view.detail, view.next_action),
    )
}

/// A thin adapter over the audit owner's existing read-only service.
pub struct LocalAuditService<'store> {
    query: AuditQuery<'store>,
}

impl<'store> LocalAuditService<'store> {
    /// Creates an adapter over an already opened state store.
    #[must_use]
    pub const fn new(state: &'store repo_com_state::StateStore) -> Self {
        Self {
            query: AuditQuery::new(state),
        }
    }
}

impl AuditService for LocalAuditService<'_> {
    fn query(&mut self, request: AuditQueryRequest) -> OperationsResult<AuditView> {
        let page: AuditPage = self
            .query
            .query(&request.filter)
            .map_err(|error| map_audit_error(&error))?;
        Ok(AuditView::from_page(&page))
    }
}
