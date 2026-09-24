#![forbid(unsafe_code)]
#![doc = "Thin, validated operator and lifecycle command handlers for repo-com."]

pub mod handlers;
pub mod input;

#[cfg(test)]
#[path = "../tests/cli_operations_contract.rs"]
mod cli_operations_contract;

use repo_com_config::ResolvedConfig;
use repo_com_foundation::{
    ColorChoice, CommandOutcome, DiagnosticsChoice, ErrorCategory, GlobalArgs, OutputStreams,
    RepoComError, TtyMode,
};
use repo_com_terminal_operations::render::RenderOptions;
use repo_com_terminal_operations::{
    AuditView, ConfigStatusView, LifecyclePageView, LifecycleView, LocalErrorView,
    OperationsRenderer, OperationsView, PolicyStatusView, StateVerificationView, TerminalWidth,
    field_lines, lines_fit,
};
use serde::{Deserialize, Serialize};

pub use handlers::audit::{AuditQueryRequest, AuditService, LocalAuditService};
pub use handlers::config::{ConfigService, ConfigValidationRequest};
pub use handlers::policy::{
    PolicyActivationRequest, PolicyActivationView, PolicyService, PolicyStatusRequest,
};
pub use handlers::purge::{
    LocalPurgeService, PurgeExecutionRequest, PurgePlanRequest, PurgeService,
};
pub use handlers::state::{
    LifecycleInspectionRequest, LifecycleInspectionResult, StateService, StateVerificationRequest,
    VerificationOnlyService,
};
pub use handlers::{KeyboardOperationsUi, KeyboardUi, OperationsUi, OperationsUiAdapter};
pub use input::*;

/// Result type used by all operations ports and handlers.
pub type OperationsResult<T> = Result<T, RepoComError>;

/// Explicit process context passed from the final executable to handlers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandlerContext {
    /// Whether stdin and stdout are both interactive.
    pub tty_mode: TtyMode,
    /// Injected current Unix time for callers that need an explicit domain time.
    pub now_unix_seconds: u64,
    /// Explicit color choice.
    pub color: ColorChoice,
    /// Explicit diagnostics choice.
    pub diagnostics: DiagnosticsChoice,
}

impl HandlerContext {
    /// Creates a context with safe presentation defaults.
    #[must_use]
    pub const fn new(tty_mode: TtyMode, now_unix_seconds: u64) -> Self {
        Self {
            tty_mode,
            now_unix_seconds,
            color: ColorChoice::Never,
            diagnostics: DiagnosticsChoice::Off,
        }
    }

    /// Carries global presentation choices into the handler context.
    #[must_use]
    pub const fn from_args(args: &GlobalArgs, tty_mode: TtyMode, now_unix_seconds: u64) -> Self {
        Self {
            tty_mode,
            now_unix_seconds,
            color: args.color,
            diagnostics: args.diagnostics,
        }
    }

    /// Returns renderer options for an explicit terminal width.
    #[must_use]
    pub const fn render_options(self, width: usize) -> RenderOptions {
        RenderOptions {
            color: self.color,
            width: TerminalWidth::new(width),
            tty_mode: self.tty_mode,
        }
    }
}

/// A complete typed operations result suitable for a protocol-version-1 data
/// field or a human presentation adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "kebab-case")]
pub enum OperationsOutput {
    /// Configuration validation result.
    Config(ConfigStatusView),
    /// Exact policy status result.
    Policy(PolicyStatusView),
    /// Committed exact policy activation result.
    Activation(PolicyActivationView),
    /// Read-only state verification result.
    State(StateVerificationView),
    /// Bounded local audit result.
    Audit(AuditView),
    /// One exact lifecycle record.
    Lifecycle(LifecycleView),
    /// One bounded lifecycle page.
    LifecyclePage(LifecyclePageView),
    /// Non-mutating purge plan.
    PurgePlan(repo_com_terminal_operations::PurgePlanView),
    /// Confirmed local purge execution result.
    PurgeExecution(repo_com_terminal_operations::PurgeExecutionView),
}

impl OperationsOutput {
    /// Returns a stable view label.
    #[must_use]
    pub const fn view_name(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::Policy(_) => "policy",
            Self::Activation(_) => "activation",
            Self::State(_) => "state",
            Self::Audit(_) => "audit",
            Self::Lifecycle(_) => "lifecycle",
            Self::LifecyclePage(_) => "lifecycle-page",
            Self::PurgePlan(_) => "purge-plan",
            Self::PurgeExecution(_) => "purge-execution",
        }
    }

    /// Renders labeled human output without contacting a domain service.
    #[must_use]
    pub fn render_human(&self, options: RenderOptions) -> String {
        match self {
            Self::Config(value) => {
                OperationsRenderer::new(options).render(&OperationsView::Config(value.clone()))
            }
            Self::Policy(value) => {
                OperationsRenderer::new(options).render(&OperationsView::Policy(value.clone()))
            }
            Self::Activation(value) => handlers::policy::render_activation_result(value, options),
            Self::State(value) => OperationsRenderer::new(options)
                .render(&OperationsView::StateVerification(value.clone())),
            Self::Audit(value) => {
                OperationsRenderer::new(options).render(&OperationsView::Audit(value.clone()))
            }
            Self::Lifecycle(value) => {
                OperationsRenderer::new(options).render(&OperationsView::Lifecycle(value.clone()))
            }
            Self::LifecyclePage(value) => OperationsRenderer::new(options)
                .render(&OperationsView::LifecyclePage(value.clone())),
            Self::PurgePlan(value) => {
                OperationsRenderer::new(options).render(&OperationsView::PurgePlan(value.clone()))
            }
            Self::PurgeExecution(value) => OperationsRenderer::new(options)
                .render(&OperationsView::PurgeExecution(value.clone())),
        }
    }

    /// Serializes this output as a protocol-version-1 success envelope.
    pub fn to_protocol_json(&self) -> Result<String, serde_json::Error> {
        CommandOutcome::success(self.clone()).to_json()
    }
}

/// Aggregate domain port used by the thin dispatcher.
pub trait OperationsService:
    ConfigService + PolicyService + StateService + AuditService + PurgeService
{
}

impl<T> OperationsService for T where
    T: ConfigService + PolicyService + StateService + AuditService + PurgeService
{
}

/// Dispatches one parsed operations command and always returns a protocol
/// outcome. All validation and domain failures are converted before the
/// envelope is constructed.
pub fn dispatch<S, U>(
    service: &mut S,
    ui: &mut U,
    context: HandlerContext,
    config: &ResolvedConfig,
    input: OperationsInput,
) -> CommandOutcome<OperationsOutput>
where
    S: OperationsService + ?Sized,
    U: OperationsUi + ?Sized,
{
    CommandOutcome::from_result(dispatch_result(service, ui, context, config, input))
}

/// Parses and dispatches bytes, preserving one protocol object on every
/// malformed-input or domain failure.
pub fn dispatch_json<S, U>(
    service: &mut S,
    ui: &mut U,
    context: HandlerContext,
    config: &ResolvedConfig,
    bytes: &[u8],
) -> CommandOutcome<OperationsOutput>
where
    S: OperationsService + ?Sized,
    U: OperationsUi + ?Sized,
{
    match input::parse(bytes) {
        Ok(parsed) => dispatch(service, ui, context, config, parsed),
        Err(error) => CommandOutcome::failure(error),
    }
}

fn dispatch_result<S, U>(
    service: &mut S,
    ui: &mut U,
    context: HandlerContext,
    config: &ResolvedConfig,
    input: OperationsInput,
) -> OperationsResult<OperationsOutput>
where
    S: OperationsService + ?Sized,
    U: OperationsUi + ?Sized,
{
    match input {
        OperationsInput::ConfigValidate(value) => {
            let output =
                handlers::config::validate(service, config, ConfigValidationRequest::from(value))?;
            Ok(OperationsOutput::Config(output))
        }
        OperationsInput::PolicyStatus(value) => {
            let output =
                handlers::policy::status(service, config, PolicyStatusRequest::from(value))?;
            Ok(OperationsOutput::Policy(output))
        }
        OperationsInput::PolicyActivate(value) => {
            let output = handlers::policy::activate(
                service,
                ui,
                config,
                PolicyActivationRequest::from(value),
                context.tty_mode,
            )?;
            Ok(OperationsOutput::Activation(output))
        }
        OperationsInput::StateVerify(value) => {
            let output = handlers::state::verify(service, StateVerificationRequest::from(value))?;
            Ok(OperationsOutput::State(output))
        }
        OperationsInput::LifecycleInspect(value) => {
            let request = LifecycleInspectionRequest::try_from(value)?;
            match handlers::state::inspect(service, request)? {
                LifecycleInspectionResult::Record(value) => Ok(OperationsOutput::Lifecycle(value)),
                LifecycleInspectionResult::Page(value) => {
                    Ok(OperationsOutput::LifecyclePage(value))
                }
            }
        }
        OperationsInput::AuditQuery(value) => {
            let output = handlers::audit::query(service, AuditQueryRequest::try_from(value)?)?;
            Ok(OperationsOutput::Audit(output))
        }
        OperationsInput::PurgePlan(value) => {
            let plan = handlers::purge::plan(service, PurgePlanRequest::try_from(value)?)?;
            Ok(OperationsOutput::PurgePlan(
                repo_com_terminal_operations::PurgePlanView::from_plan(&plan),
            ))
        }
        OperationsInput::PurgeExecute(value) => {
            let output = handlers::purge::execute(service, ui, value, context.tty_mode)?;
            Ok(OperationsOutput::PurgeExecution(output))
        }
    }
}

/// Returns separated machine output and optional diagnostics.
pub fn machine_output_streams(
    outcome: &CommandOutcome<OperationsOutput>,
    diagnostics: Option<String>,
) -> OperationsResult<OutputStreams> {
    outcome
        .output_streams(diagnostics)
        .map_err(|_| RepoComError::internal_failure("operations outcome could not be serialized"))
}

/// Renders a successful or failed human outcome while keeping protocol output
/// available separately through [`machine_output_streams`].
#[must_use]
pub fn render_human_outcome(
    outcome: &CommandOutcome<OperationsOutput>,
    options: RenderOptions,
) -> String {
    match outcome.status() {
        repo_com_foundation::OutcomeStatus::Success => outcome
            .data()
            .map_or_else(String::new, |data| data.render_human(options)),
        repo_com_foundation::OutcomeStatus::Error => {
            let view = outcome
                .error()
                .map(LocalErrorView::from_repo_com_error)
                .unwrap_or_else(|| {
                    LocalErrorView::new(
                        ErrorCategory::InternalFailure,
                        "internal-failure",
                        "an internal operations error occurred",
                        "inspect the stable failure before retrying",
                    )
                });
            OperationsRenderer::new(options).render(&OperationsView::Error(view))
        }
    }
}

/// Returns whether a rendered human result fits the minimum terminal width.
#[must_use]
pub fn human_output_fits(value: &str, width: usize) -> bool {
    lines_fit(value, TerminalWidth::new(width).columns())
}

/// Renders labeled fields for a committed activation result. This helper is
/// public for final-binary composition without granting any authority.
#[must_use]
pub fn render_activation_fields(value: &PolicyActivationView, width: usize) -> String {
    let columns = TerminalWidth::new(width).columns();
    let tuple = value.tuple.to_string();
    let mut lines = vec!["Policy activation".to_owned()];
    for (label, field) in [
        ("Repository", value.repository_id.as_str()),
        ("Object", "policy activation"),
        ("Activation ID", value.activation_id.as_str()),
        ("Policy tuple", tuple.as_str()),
        ("Configuration hash", value.config_hash.as_str()),
        ("Policy tuple hash", value.tuple_hash.as_str()),
        ("Activated at", value.activated_at.as_str()),
    ] {
        lines.extend(field_lines(label, field, columns));
    }
    lines.extend(field_lines("Outcome", "activated", columns));
    lines.extend(field_lines("Provenance", "local state", columns));
    lines.extend(field_lines("Next action", &value.next_action, columns));
    format!("{}\n", lines.join("\n"))
}
