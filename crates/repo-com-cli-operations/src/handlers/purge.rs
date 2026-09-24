//! Non-mutating purge planning and TTY-confirmed local purge execution ports.

use repo_com_foundation::{RepoComError, TtyMode};
use repo_com_purge::{
    PurgeConfirmation, PurgeError, PurgeExecution, PurgeExecutor, PurgePlan, PurgePlanner,
    PurgeRequest, PurgeScope,
};
use repo_com_terminal_operations::{
    PromptResult, PurgeConfirmationIdentity, PurgeExecutionView, PurgePlanView,
    purge_confirmation_from_intent,
};

use crate::OperationsResult;
use crate::handlers::OperationsUi;
use crate::input::{PurgeExecuteInput, PurgePlanInput};

/// A validated non-mutating purge plan request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgePlanRequest {
    /// Exact domain request passed to the planner.
    pub request: PurgeRequest,
}

impl TryFrom<PurgePlanInput> for PurgePlanRequest {
    type Error = RepoComError;

    fn try_from(value: PurgePlanInput) -> Result<Self, Self::Error> {
        let scope = parse_scope(&value.scope)?;
        let cutoff = value.cutoff_value()?;
        let mut request = PurgeRequest::new(value.repository_id, scope, cutoff);
        if let Some(hash) = value.expected_config_hash {
            request = request.with_expected_config_hash(hash);
        }
        Ok(Self { request })
    }
}

impl PurgePlanRequest {
    /// Creates a request directly from a domain request.
    #[must_use]
    pub fn from_domain(request: PurgeRequest) -> Self {
        Self { request }
    }
}

/// A confirmed local purge execution request passed to the transaction owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeExecutionRequest {
    /// Exact current plan shown to the operator.
    pub plan: PurgePlan,
    /// Exact UI confirmation bound to that plan and TTY mode.
    pub confirmation: PurgeConfirmation,
    /// Explicit stream mode supplied by the process owner.
    pub tty_mode: TtyMode,
    /// Canonical execution timestamp.
    pub executed_at: String,
}

/// Domain port for non-mutating planning and confirmed local execution.
pub trait PurgeService {
    /// Builds a plan without changing local state.
    fn plan(&mut self, request: PurgePlanRequest) -> OperationsResult<PurgePlan>;

    /// Revalidates and executes a confirmed plan in the owning transaction.
    fn execute(&mut self, request: PurgeExecutionRequest) -> OperationsResult<PurgeExecution>;
}

/// Routes one non-mutating purge plan request.
pub fn plan<S>(service: &mut S, request: PurgePlanRequest) -> OperationsResult<PurgePlan>
where
    S: PurgeService + ?Sized,
{
    service.plan(request)
}

/// Routes confirmed purge execution through the operations UI and owner.
pub fn execute<S, U>(
    service: &mut S,
    ui: &mut U,
    request: PurgeExecuteInput,
    tty_mode: TtyMode,
) -> OperationsResult<PurgeExecutionView>
where
    S: PurgeService + ?Sized,
    U: OperationsUi + ?Sized,
{
    // Planning is safe to construct only after the explicit TTY gate. This
    // prevents automation from creating a reusable confirmation authority.
    tty_mode.require_prompt_allowed()?;
    let scope = parse_scope(&request.scope)?;
    let cutoff = request.cutoff_value()?;
    let plan_request = PurgePlanRequest::from_domain(
        PurgeRequest::new(request.repository_id.clone(), scope, cutoff.clone())
            .with_expected_config_hash(request.config_hash.clone()),
    );
    let plan = service.plan(plan_request)?;
    validate_plan_binding(&plan, &request)?;

    let view = PurgePlanView::from_plan(&plan);
    let prompt_result = ui.request_purge(view);
    let confirmation = require_purge_confirmation(prompt_result, &plan, tty_mode)?;
    let execution = service.execute(PurgeExecutionRequest {
        plan,
        confirmation,
        tty_mode,
        executed_at: request.executed_at,
    })?;
    Ok(PurgeExecutionView::from_execution(&execution))
}

fn validate_plan_binding(
    plan: &PurgePlan,
    request: &PurgeExecuteInput,
) -> Result<(), RepoComError> {
    let cutoff = request.cutoff_value()?;
    if plan.repository_id != request.repository_id
        || plan.scope.as_str() != request.scope
        || plan.cutoff.unix_seconds != cutoff.unix_seconds
        || plan.cutoff.utc != cutoff.utc
        || plan.config_hash != request.config_hash
        || plan.plan_hash != request.plan_hash
    {
        return Err(RepoComError::policy_blocked(
            "purge plan identity or hash changed; generate and confirm a fresh plan",
        ));
    }
    Ok(())
}

fn require_purge_confirmation(
    result: PromptResult,
    plan: &PurgePlan,
    tty_mode: TtyMode,
) -> Result<PurgeConfirmation, RepoComError> {
    let identity = PurgeConfirmationIdentity::from_plan(plan);
    match result {
        PromptResult::Confirmed(intent) => {
            purge_confirmation_from_intent(&intent, &identity, tty_mode).ok_or_else(|| {
                RepoComError::usage("purge confirmation did not match the complete exact plan")
            })?;
            Ok(PurgeConfirmation::from_plan(plan, tty_mode))
        }
        PromptResult::NonTty => Err(RepoComError::operator_action_required(
            "confirmed purge requires an interactive TTY",
        )),
        PromptResult::Cancelled | PromptResult::Defaulted => Err(
            RepoComError::operator_action_required("operator did not confirm the exact purge plan"),
        ),
        PromptResult::InvalidInput { .. }
        | PromptResult::PlanHashChanged { .. }
        | PromptResult::ScopeMismatch { .. } => Err(RepoComError::usage(
            "operator input did not match the exact purge grammar",
        )),
        PromptResult::InputUnavailable { .. } => Err(RepoComError::operator_action_required(
            "operator input was unavailable for purge confirmation",
        )),
    }
}

/// Converts a safe purge-domain failure into a command error.
#[must_use]
pub fn map_purge_error(error: &PurgeError) -> RepoComError {
    let view = repo_com_terminal_operations::LocalErrorView::from_purge_error(error);
    RepoComError::new(
        view.category,
        format!("{}; next action: {}", view.detail, view.next_action),
    )
}

/// A thin adapter over the purge planner and transactional executor owners.
pub struct LocalPurgeService {
    executor: PurgeExecutor,
}

impl LocalPurgeService {
    /// Creates an adapter over an already opened local state store.
    #[must_use]
    pub const fn new(state: repo_com_state::StateStore) -> Self {
        Self {
            executor: PurgeExecutor::new(state),
        }
    }

    /// Returns read-only access to the owned state store.
    #[must_use]
    pub const fn state(&self) -> &repo_com_state::StateStore {
        self.executor.state()
    }
}

impl PurgeService for LocalPurgeService {
    fn plan(&mut self, request: PurgePlanRequest) -> OperationsResult<PurgePlan> {
        PurgePlanner::new()
            .plan(self.executor.state(), &request.request)
            .map_err(|error| map_purge_error(&error))
    }

    fn execute(&mut self, request: PurgeExecutionRequest) -> OperationsResult<PurgeExecution> {
        self.executor
            .execute_with_tty(
                &request.plan,
                &request.confirmation,
                request.tty_mode,
                request.executed_at,
            )
            .map_err(|error| map_purge_error(&error))
    }
}

fn parse_scope(value: &str) -> Result<PurgeScope, RepoComError> {
    match value {
        "content" => Ok(PurgeScope::Content),
        "metadata" => Ok(PurgeScope::Metadata),
        "all" => Ok(PurgeScope::All),
        _ => Err(RepoComError::usage(
            "purge scope must be content, metadata, or all",
        )),
    }
}
