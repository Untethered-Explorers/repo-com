//! Exact policy status and operator activation command ports.

use repo_com_config::ResolvedConfig;
use repo_com_foundation::{RepoComError, TtyMode};
use repo_com_policy::{ActivationReceipt, PolicyError, PolicyTuple};
use repo_com_terminal_operations::render::RenderOptions;
use repo_com_terminal_operations::{
    ActivationConfirmationIdentity, ActivationPreviewView, ExactConfirmation, PolicyStatusView,
    PromptAction, PromptResult, field_lines,
};
use serde::{Deserialize, Serialize};

use crate::OperationsResult;
use crate::handlers::OperationsUi;
use crate::input::{PolicyActivationInput, PolicyStatusInput};

/// A read-only exact policy status request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyStatusRequest {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact event/destination/severity tuple.
    pub tuple: PolicyTuple,
}

impl From<PolicyStatusInput> for PolicyStatusRequest {
    fn from(value: PolicyStatusInput) -> Self {
        let tuple = value.tuple();
        Self {
            repository_id: value.repository_id,
            tuple,
        }
    }
}

/// A permission-widening activation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyActivationRequest {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact event/destination/severity tuple.
    pub tuple: PolicyTuple,
    /// Optional caller-selected activation ID.
    pub activation_id: Option<String>,
    /// Explicit canonical activation timestamp.
    pub activated_at: String,
}

impl From<PolicyActivationInput> for PolicyActivationRequest {
    fn from(value: PolicyActivationInput) -> Self {
        let tuple = value.tuple();
        Self {
            repository_id: value.repository_id,
            tuple,
            activation_id: value.activation_id,
            activated_at: value.activated_at,
        }
    }
}

/// Safe projection of a committed exact policy activation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyActivationView {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact policy tuple.
    pub tuple: PolicyTuple,
    /// Stable activation identifier.
    pub activation_id: String,
    /// Configuration hash persisted by the policy owner.
    pub config_hash: String,
    /// Exact tuple hash persisted by the policy owner.
    pub tuple_hash: String,
    /// Activation timestamp persisted by the policy owner.
    pub activated_at: String,
    /// Local-state provenance.
    pub provenance: repo_com_terminal_operations::Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl PolicyActivationView {
    /// Projects a domain receipt after the owning service commits it.
    #[must_use]
    pub fn from_receipt(receipt: &ActivationReceipt) -> Self {
        Self {
            repository_id: receipt.repository_id.clone(),
            tuple: receipt.tuple.clone(),
            activation_id: receipt.activation_id.clone(),
            config_hash: receipt.config_hash.clone(),
            tuple_hash: receipt.tuple_hash.clone(),
            activated_at: receipt.activated_at.clone(),
            provenance: repo_com_terminal_operations::Provenance::LocalState,
            next_action:
                "Keep this exact tuple and hash binding current; re-check policy status before relying on it."
                    .to_owned(),
        }
    }
}

/// Domain port for policy inspection and exact activation.
pub trait PolicyService {
    /// Reads the current status of one exact tuple.
    fn status(
        &mut self,
        config: &ResolvedConfig,
        request: PolicyStatusRequest,
    ) -> OperationsResult<PolicyStatusView>;

    /// Builds a complete non-authoritative activation preview.
    fn activation_preview(
        &mut self,
        config: &ResolvedConfig,
        request: PolicyActivationRequest,
    ) -> OperationsResult<ActivationPreviewView>;

    /// Revalidates and commits one exact activation after UI confirmation.
    fn activate(
        &mut self,
        config: &ResolvedConfig,
        request: PolicyActivationRequest,
        confirmation: ExactConfirmation,
        tty_mode: TtyMode,
    ) -> OperationsResult<ActivationReceipt>;
}

/// Routes a read-only policy status request.
pub fn status<S>(
    service: &mut S,
    config: &ResolvedConfig,
    request: PolicyStatusRequest,
) -> OperationsResult<PolicyStatusView>
where
    S: PolicyService + ?Sized,
{
    require_repository(request.repository_id.as_str(), config)?;
    service.status(config, request)
}

/// Routes exact policy activation through the operations UI and policy owner.
pub fn activate<S, U>(
    service: &mut S,
    ui: &mut U,
    config: &ResolvedConfig,
    request: PolicyActivationRequest,
    tty_mode: TtyMode,
) -> OperationsResult<PolicyActivationView>
where
    S: PolicyService + ?Sized,
    U: OperationsUi + ?Sized,
{
    // Fail before preview construction or any state read in automation.
    tty_mode.require_prompt_allowed()?;
    require_repository(request.repository_id.as_str(), config)?;
    let preview = service.activation_preview(config, request.clone())?;
    let result = ui.request_activation(preview.clone());
    let confirmation = require_activation_confirmation(result, &preview)?;
    let receipt = service.activate(config, request, confirmation, tty_mode)?;
    Ok(PolicyActivationView::from_receipt(&receipt))
}

/// Converts a safe policy-domain failure into the command category.
#[must_use]
pub fn map_policy_error(error: &PolicyError) -> RepoComError {
    error.to_repo_com_error()
}

/// Renders a committed activation projection without performing any action.
#[must_use]
pub fn render_activation_result(view: &PolicyActivationView, options: RenderOptions) -> String {
    let tuple = view.tuple.to_string();
    let mut lines = vec!["Policy activation".to_owned()];
    for (label, field) in [
        ("Repository", view.repository_id.as_str()),
        ("Object", "policy activation"),
        ("Activation ID", view.activation_id.as_str()),
        ("Policy tuple", tuple.as_str()),
        ("Configuration hash", view.config_hash.as_str()),
        ("Policy tuple hash", view.tuple_hash.as_str()),
        ("Activated at", view.activated_at.as_str()),
    ] {
        lines.extend(field_lines(label, field, options.columns()));
    }
    lines.extend(field_lines("Outcome", "activated", options.columns()));
    lines.extend(field_lines("Provenance", "local state", options.columns()));
    lines.extend(field_lines(
        "Next action",
        &view.next_action,
        options.columns(),
    ));
    format!("{}\n", lines.join("\n"))
}

fn require_activation_confirmation(
    result: PromptResult,
    preview: &ActivationPreviewView,
) -> Result<ExactConfirmation, RepoComError> {
    match result {
        PromptResult::Confirmed(confirmation)
            if confirmation.action == PromptAction::ActivatePolicy
                && ActivationConfirmationIdentity::from_preview(preview).matches(&confirmation) =>
        {
            Ok(confirmation)
        }
        PromptResult::Confirmed(_) => Err(RepoComError::usage(
            "policy activation confirmation did not match the complete exact preview",
        )),
        PromptResult::NonTty => Err(RepoComError::operator_action_required(
            "policy activation requires an interactive TTY",
        )),
        PromptResult::Cancelled | PromptResult::Defaulted => {
            Err(RepoComError::operator_action_required(
                "operator did not confirm the exact policy activation",
            ))
        }
        PromptResult::InvalidInput { .. }
        | PromptResult::PlanHashChanged { .. }
        | PromptResult::ScopeMismatch { .. } => Err(RepoComError::usage(
            "operator input did not match the exact policy activation grammar",
        )),
        PromptResult::InputUnavailable { .. } => Err(RepoComError::operator_action_required(
            "operator input was unavailable for policy activation",
        )),
    }
}

fn require_repository(repository_id: &str, config: &ResolvedConfig) -> Result<(), RepoComError> {
    if repository_id == config.config.repository_id {
        Ok(())
    } else {
        Err(RepoComError::usage(
            "policy repository_id does not match the explicit resolved repository",
        ))
    }
}
