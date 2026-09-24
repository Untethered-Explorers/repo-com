//! Read-only state verification and lifecycle inspection command ports.

use std::path::PathBuf;

use repo_com_audit_query::{AuditCursor, AuditFilter};
use repo_com_foundation::RepoComError;
use repo_com_lifecycle::{
    EXPECTED_MIGRATION_VERSION, InspectionRequest, LifecycleError, LifecycleObject, LifecyclePage,
    LifecyclePage as BoundedPage, LifecycleProjection, PageRequest, StateVerificationReport,
    StateVerifier, VerificationRequest,
};
use repo_com_terminal_operations::{
    LifecyclePageView, LifecycleView, LocalErrorView, StateVerificationView,
};

use crate::OperationsResult;
use crate::input::{LifecycleInspectInput, StateVerifyInput};

/// A read-only state verification request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateVerificationRequest {
    /// Exact repository scope.
    pub repository_id: String,
    /// Existing local database path.
    pub database_path: PathBuf,
    /// Expected SQLite user version.
    pub expected_migration: i64,
}

impl From<StateVerifyInput> for StateVerificationRequest {
    fn from(value: StateVerifyInput) -> Self {
        Self {
            repository_id: value.repository_id,
            database_path: PathBuf::from(value.database_path),
            expected_migration: value
                .expected_migration
                .unwrap_or(EXPECTED_MIGRATION_VERSION),
        }
    }
}

impl StateVerificationRequest {
    /// Projects this command request into the domain verifier request.
    #[must_use]
    pub fn to_domain(&self) -> VerificationRequest {
        VerificationRequest::new(&self.database_path, &self.repository_id)
            .with_expected_migration(self.expected_migration)
    }
}

/// An explicit read-only lifecycle inspection request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleInspectionRequest {
    /// Exact repository scope.
    pub repository_id: String,
    /// Stable object family.
    pub object_type: String,
    /// Exact object identifier, when the object family requires one.
    pub object_id: Option<String>,
    /// Positive revision for a draft revision.
    pub revision: Option<i64>,
    /// Explicit bounded page request.
    pub page: PageRequest,
    /// Explicit retained-content opt-in.
    pub include_retained_content: bool,
}

impl TryFrom<LifecycleInspectInput> for LifecycleInspectionRequest {
    type Error = RepoComError;

    fn try_from(value: LifecycleInspectInput) -> Result<Self, Self::Error> {
        let page_size = value.page_size()?;
        let mut page = PageRequest::new(page_size);
        if let Some(after) = value.after {
            page = page.with_after(after);
        }
        Ok(Self {
            repository_id: value.repository_id,
            object_type: value.object_type,
            object_id: value.object_id,
            revision: value.revision,
            page,
            include_retained_content: value.include_retained_content,
        })
    }
}

impl LifecycleInspectionRequest {
    /// Converts the validated selector into the lifecycle owner's request.
    ///
    /// This conversion is deliberately total only after input validation. The
    /// domain inspector still validates repository existence, object scope,
    /// page bounds, and retained-content handling.
    pub fn to_domain(&self) -> Result<InspectionRequest, RepoComError> {
        let object = match self.object_type.as_str() {
            "repository" => LifecycleObject::Repository,
            "draft" => LifecycleObject::Draft {
                draft_id: self.object_id.clone().ok_or_else(|| {
                    RepoComError::usage("draft lifecycle inspection requires object_id")
                })?,
            },
            "draft_revision" => LifecycleObject::DraftRevision {
                draft_id: self.object_id.clone().ok_or_else(|| {
                    RepoComError::usage("draft revision inspection requires object_id")
                })?,
                revision: self.revision.ok_or_else(|| {
                    RepoComError::usage("draft revision inspection requires revision")
                })?,
            },
            "delivery_attempt" => LifecycleObject::DeliveryAttempt {
                attempt_id: self.object_id.clone().ok_or_else(|| {
                    RepoComError::usage("delivery attempt inspection requires object_id")
                })?,
            },
            "inbound_item" => LifecycleObject::InboundItem {
                item_id: self.object_id.clone().ok_or_else(|| {
                    RepoComError::usage("inbound item inspection requires object_id")
                })?,
            },
            "acknowledgement" => LifecycleObject::Acknowledgement {
                item_id: self.object_id.clone().ok_or_else(|| {
                    RepoComError::usage("acknowledgement inspection requires object_id")
                })?,
            },
            "archive" => LifecycleObject::Archive {
                item_id: self
                    .object_id
                    .clone()
                    .ok_or_else(|| RepoComError::usage("archive inspection requires object_id"))?,
            },
            "reply_link" => LifecycleObject::ReplyLink {
                item_id: self.object_id.clone().ok_or_else(|| {
                    RepoComError::usage("reply link inspection requires object_id")
                })?,
            },
            "audit_transition" => {
                let mut filter =
                    AuditFilter::new(&self.repository_id).with_page_size(self.page.limit);
                if let Some(after) = &self.page.after {
                    let parts: Vec<&str> = after.split('|').collect();
                    if parts.len() != 3 || parts[0] != self.repository_id {
                        return Err(RepoComError::usage(
                            "lifecycle audit cursor is invalid for this repository",
                        ));
                    }
                    let audit_id = parts[2]
                        .parse::<i64>()
                        .map_err(|_| RepoComError::usage("lifecycle audit cursor is invalid"))?;
                    let cursor = AuditCursor::new(parts[0], parts[1], audit_id)
                        .map_err(|_| RepoComError::usage("lifecycle audit cursor is invalid"))?;
                    filter = filter.with_cursor(cursor);
                }
                LifecycleObject::AuditTransitions { filter }
            }
            _ => {
                return Err(RepoComError::usage(
                    "lifecycle object_type is not supported",
                ));
            }
        };
        let request = InspectionRequest::new(self.repository_id.clone(), object)
            .with_page(self.page.clone())
            .with_retained_content(self.include_retained_content);
        Ok(request)
    }
}

/// Result shape returned by a lifecycle domain port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleInspectionResult {
    /// One exact lifecycle record.
    Record(LifecycleView),
    /// One bounded page of lifecycle records.
    Page(LifecyclePageView),
}

impl LifecycleInspectionResult {
    /// Projects a domain projection into a single-record presentation view.
    #[must_use]
    pub fn from_projection(projection: &LifecycleProjection) -> Self {
        Self::Record(LifecycleView::from_projection(projection))
    }

    /// Projects a domain page whose rows already have a UI projection.
    #[must_use]
    pub fn from_page<T>(
        page: &LifecyclePage<T>,
        object_type: &str,
        project: impl Fn(&T) -> LifecycleView,
    ) -> Self
    where
        T: Clone,
    {
        Self::Page(LifecyclePageView::new(
            page.repository_id.clone(),
            object_type,
            page.items.iter().map(project).collect(),
            page.page_size,
            page.truncated,
            page.next_after.clone(),
        ))
    }

    /// Alias for [`Self::from_page`].
    #[must_use]
    pub fn from_bounded_page<T>(
        page: &BoundedPage<T>,
        object_type: &str,
        project: impl Fn(&T) -> LifecycleView,
    ) -> Self
    where
        T: Clone,
    {
        Self::from_page(page, object_type, project)
    }
}

/// Domain port for read-only verification and inspection.
pub trait StateService {
    /// Verifies an existing local state database without creating or repairing it.
    fn verify(
        &mut self,
        request: StateVerificationRequest,
    ) -> OperationsResult<StateVerificationView>;

    /// Inspects one bounded lifecycle object without mutation.
    fn inspect(
        &mut self,
        request: LifecycleInspectionRequest,
    ) -> OperationsResult<LifecycleInspectionResult>;
}

/// Routes one read-only state verification request.
pub fn verify<S>(
    service: &mut S,
    request: StateVerificationRequest,
) -> OperationsResult<StateVerificationView>
where
    S: StateService + ?Sized,
{
    service.verify(request)
}

/// Routes one bounded read-only lifecycle inspection request.
pub fn inspect<S>(
    service: &mut S,
    request: LifecycleInspectionRequest,
) -> OperationsResult<LifecycleInspectionResult>
where
    S: StateService + ?Sized,
{
    // Validate the request shape once at the handler boundary. The lifecycle
    // service remains responsible for current repository and object checks.
    request.to_domain()?;
    service.inspect(request)
}

/// Converts a safe lifecycle-domain failure into a command error.
#[must_use]
pub fn map_lifecycle_error(error: &LifecycleError) -> RepoComError {
    let view = LocalErrorView::from_lifecycle_error(error);
    RepoComError::new(
        view.category,
        format!("{}; next action: {}", view.detail, view.next_action),
    )
}

/// Converts a verifier report into the operations presentation projection.
#[must_use]
pub fn project_verification(report: &StateVerificationReport) -> StateVerificationView {
    StateVerificationView::from_report(report)
}

/// Provides a stateless verifier adapter for callers that only need the
/// read-only verification operation. It performs no migration or repair.
#[derive(Clone, Copy, Debug, Default)]
pub struct VerificationOnlyService;

impl StateService for VerificationOnlyService {
    fn verify(
        &mut self,
        request: StateVerificationRequest,
    ) -> OperationsResult<StateVerificationView> {
        Ok(project_verification(
            &StateVerifier::new().verify(&request.to_domain()),
        ))
    }

    fn inspect(
        &mut self,
        _request: LifecycleInspectionRequest,
    ) -> OperationsResult<LifecycleInspectionResult> {
        Err(RepoComError::usage(
            "lifecycle inspection requires a state-backed service",
        ))
    }
}
