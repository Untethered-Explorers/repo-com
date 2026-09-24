#![forbid(unsafe_code)]

//! Composition for the installed `repo-com` executable.
//!
//! This module owns process concerns only: typed argument routing, explicit
//! stream selection, configuration/state construction, and separation of the
//! protocol, human, and diagnostic outputs.  Feature behavior remains behind
//! the two command-handler crates and their domain ports.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{ArgAction, Parser, Subcommand};
use repo_com_approval::{
    ApprovalInstant, ApprovalService, OperatorConfirmation, OverrideReasonCode,
};
use repo_com_cli_messaging as messaging;
use repo_com_cli_operations as operations;
use repo_com_config::{ConfigResolver, ResolvedConfig};
use repo_com_delivery::{DeliveryCoordinator, DeliveryError, DeliveryState, TransitionRequest};
use repo_com_discord_client::DiscordClient;
use repo_com_discord_message::{CreateMessageRequest, DiscordMessageClient};
use repo_com_draft_content::{ContentRenderer, DeliveryNonce};
use repo_com_draft_model::{AuthorizedReplyReference, DraftMetadata, DraftModel, DraftRequest};
use repo_com_draft_safety::SecretScanner;
use repo_com_foundation::{CommandOutcome, GlobalArgs, RepoComError, TtyMode};
use repo_com_inbox_fetch::{FetchError, InboundFetcher, rfc3339_to_unix_millis};
use repo_com_inbox_state::InboxState;
use repo_com_lifecycle::LifecycleInspector;
use repo_com_policy::{PolicyRegistry, evaluate_from_state};
use repo_com_purge::PurgeExecutor;
use repo_com_reply::{
    ReplyCommandService, ReplyTargetError, ReplyTargetRequest, ReplyTargetValidator,
};
use repo_com_send_eligibility::{EligibilityBlocker, EligibilityEvaluator, EligibilityInput};
use repo_com_state::{DraftInput, DraftRevisionInput, RepositoryInput, StateStore};
use repo_com_terminal_operations::{
    ActivationPreviewView, AuditView, ConfigStatusView, LocalErrorView, OperationsRenderer,
    OperationsView, PolicyStatusView, StateVerificationView,
    render::RenderOptions as OperationsRenderOptions,
};
use repo_com_terminal_outbound::{
    DeliveryView, OutboundPreview, OutboundRenderer, OutboundView,
    RenderOptions as OutboundRenderOptions,
};
use serde_json::{Value, json};

const MAX_STRUCTURED_INPUT_BYTES: u64 = 1024 * 1024;
const PROGRAM_NAME: &str = "repo-com";

/// The installed process entry point.
pub fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().collect();
    let code = execute(args, &mut io::stdin(), &mut io::stdout(), &mut io::stderr());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// A typed global command line.  The command-specific payload is deliberately
/// read from stdin; no command carries an implicit repository, destination,
/// revision, cursor, time boundary, or inbound item flag.
#[derive(Debug, Parser)]
#[command(
    name = PROGRAM_NAME,
    about = "Explicit, repository-scoped Discord workflow commands",
    disable_version_flag = true,
    color = clap::ColorChoice::Never
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,

    /// Optional explicit local state database path.
    #[arg(long = "state", global = true, value_name = "PATH")]
    state_path: Option<PathBuf>,

    /// Print only the package semantic version.
    #[arg(long = "version", global = true, action = ArgAction::SetTrue)]
    version: bool,

    /// Request TTY mode; a non-interactive stream still fails closed.
    #[arg(long = "tty", global = true, conflicts_with = "non_tty")]
    tty: bool,

    /// Explicitly select non-interactive mode.
    #[arg(long = "non-tty", global = true)]
    non_tty: bool,

    #[command(subcommand)]
    command: Option<CommandGroup>,
}

/// The complete command tree.  Dot and kebab protocol spellings are normalized
/// before clap parses them, while these nested names remain the help surface.
#[derive(Clone, Debug, Subcommand)]
enum CommandGroup {
    /// Validate one explicit repository configuration.
    #[command(alias = "cfg")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Inspect or activate an exact policy tuple.
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Create and inspect immutable outbound drafts.
    Draft {
        #[command(subcommand)]
        action: DraftAction,
    },
    /// Dispatch one exact draft revision.
    Send {
        #[command(subcommand)]
        action: Option<SendAction>,
    },
    /// Read-only Discord setup diagnostics.
    #[command(name = "setup-check", alias = "setup.check", alias = "setup_check")]
    SetupCheck,
    /// Read and mark untrusted inbound data.
    Inbox {
        #[command(subcommand)]
        action: InboxAction,
    },
    /// Create a validated reply draft.
    Reply {
        #[command(subcommand)]
        action: ReplyAction,
    },
    /// Query bounded local audit evidence.
    Audit {
        #[command(subcommand)]
        action: AuditAction,
    },
    /// Verify or inspect local state.
    State {
        #[command(subcommand)]
        action: StateAction,
    },
    /// Plan or execute a confirmed local purge.
    Purge {
        #[command(subcommand)]
        action: PurgeAction,
    },
    /// Alias group for the lifecycle inspection operation.
    Lifecycle {
        #[command(subcommand)]
        action: LifecycleAction,
    },
    /// Alias group for the read-only setup operation.
    Setup {
        #[command(subcommand)]
        action: Option<SetupAction>,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum ConfigAction {
    /// Validate the resolved configuration.
    Validate,
}

#[derive(Clone, Debug, Subcommand)]
enum PolicyAction {
    /// Read exact policy status.
    Status,
    /// Preview and activate one exact tuple through a TTY confirmation.
    Activate,
}

#[derive(Clone, Debug, Subcommand)]
enum DraftAction {
    /// Create revision one.
    Create,
    /// Read one exact revision.
    Show,
    /// Create a new immutable revision.
    Update,
    /// Build a side-effect-free exact preview.
    Preview,
    /// Record an exact TTY approval.
    Approve,
    /// Record an exact TTY safety override.
    #[command(name = "secret-override", alias = "secret_override")]
    SecretOverride,
}

#[derive(Clone, Debug, Subcommand)]
enum SendAction {
    /// Dispatch the exact revision (the default leaf spelling is `send`).
    Dispatch,
}

#[derive(Clone, Debug, Subcommand)]
enum InboxAction {
    /// Fetch one explicit bounded page.
    Fetch,
    /// Mark one or more stored items acknowledged locally.
    Acknowledge,
    /// Mark one or more stored items archived locally.
    Archive,
}

#[derive(Clone, Debug, Subcommand)]
enum ReplyAction {
    /// Create a draft linked to a validated inbound target.
    #[command(name = "draft-create", alias = "draft.create", alias = "draft_create")]
    DraftCreate,
}

#[derive(Clone, Debug, Subcommand)]
enum AuditAction {
    /// Query one bounded page.
    Query,
}

#[derive(Clone, Debug, Subcommand)]
enum StateAction {
    /// Verify an existing database without mutation.
    Verify,
    /// Inspect one bounded lifecycle object.
    Inspect,
}

#[derive(Clone, Debug, Subcommand)]
enum PurgeAction {
    /// Build a non-mutating plan.
    Plan,
    /// Execute one exact confirmed plan.
    Execute,
}

#[derive(Clone, Debug, Subcommand)]
enum LifecycleAction {
    /// Inspect one bounded lifecycle object.
    Inspect,
}

#[derive(Clone, Debug, Subcommand)]
enum SetupAction {
    /// Run the read-only setup check.
    Check,
}

/// The two handler families and their canonical command values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Route {
    Operations(operations::OperationsCommand),
    Messaging(messaging::MessagingCommand),
}

impl Route {
    const fn canonical(self) -> &'static str {
        match self {
            Self::Operations(command) => command.as_str(),
            Self::Messaging(command) => command.as_str(),
        }
    }

    const fn is_operations(self) -> bool {
        matches!(self, Self::Operations(_))
    }
}

/// Convert a parsed tree selection into a handler route.
fn route_for(command: Option<CommandGroup>) -> Result<Route, RepoComError> {
    let Some(command) = command else {
        return Err(RepoComError::usage(
            "a command group and action are required; no command is selected by default",
        ));
    };
    let route = match command {
        CommandGroup::Config {
            action: ConfigAction::Validate,
        } => Route::Operations(operations::OperationsCommand::ConfigValidate),
        CommandGroup::Policy {
            action: PolicyAction::Status,
        } => Route::Operations(operations::OperationsCommand::PolicyStatus),
        CommandGroup::Policy {
            action: PolicyAction::Activate,
        } => Route::Operations(operations::OperationsCommand::PolicyActivate),
        CommandGroup::Draft {
            action: DraftAction::Create,
        } => Route::Messaging(messaging::MessagingCommand::DraftCreate),
        CommandGroup::Draft {
            action: DraftAction::Show,
        } => Route::Messaging(messaging::MessagingCommand::DraftShow),
        CommandGroup::Draft {
            action: DraftAction::Update,
        } => Route::Messaging(messaging::MessagingCommand::DraftUpdate),
        CommandGroup::Draft {
            action: DraftAction::Preview,
        } => Route::Messaging(messaging::MessagingCommand::DraftPreview),
        CommandGroup::Draft {
            action: DraftAction::Approve,
        } => Route::Messaging(messaging::MessagingCommand::DraftApprove),
        CommandGroup::Draft {
            action: DraftAction::SecretOverride,
        } => Route::Messaging(messaging::MessagingCommand::DraftSecretOverride),
        CommandGroup::Send {
            action: None | Some(SendAction::Dispatch),
        } => Route::Messaging(messaging::MessagingCommand::Send),
        CommandGroup::SetupCheck => Route::Messaging(messaging::MessagingCommand::SetupCheck),
        CommandGroup::Setup {
            action: None | Some(SetupAction::Check),
        } => Route::Messaging(messaging::MessagingCommand::SetupCheck),
        CommandGroup::Inbox {
            action: InboxAction::Fetch,
        } => Route::Messaging(messaging::MessagingCommand::InboxFetch),
        CommandGroup::Inbox {
            action: InboxAction::Acknowledge,
        } => Route::Messaging(messaging::MessagingCommand::InboxAcknowledge),
        CommandGroup::Inbox {
            action: InboxAction::Archive,
        } => Route::Messaging(messaging::MessagingCommand::InboxArchive),
        CommandGroup::Reply {
            action: ReplyAction::DraftCreate,
        } => Route::Messaging(messaging::MessagingCommand::ReplyDraftCreate),
        CommandGroup::Audit {
            action: AuditAction::Query,
        } => Route::Operations(operations::OperationsCommand::AuditQuery),
        CommandGroup::State {
            action: StateAction::Verify,
        } => Route::Operations(operations::OperationsCommand::StateVerify),
        CommandGroup::State {
            action: StateAction::Inspect,
        }
        | CommandGroup::Lifecycle {
            action: LifecycleAction::Inspect,
        } => Route::Operations(operations::OperationsCommand::LifecycleInspect),
        CommandGroup::Purge {
            action: PurgeAction::Plan,
        } => Route::Operations(operations::OperationsCommand::PurgePlan),
        CommandGroup::Purge {
            action: PurgeAction::Execute,
        } => Route::Operations(operations::OperationsCommand::PurgeExecute),
    };
    Ok(route)
}

/// A process-level result before stream selection.
struct OutputResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

/// Execute one process invocation using injectable streams for contract tests.
fn execute(
    args: Vec<OsString>,
    input: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let requested_json = args_have_json_output(&args);
    let normalized = normalize_command_spelling(args.clone());
    let cli = match Cli::try_parse_from(normalized) {
        Ok(cli) => cli,
        Err(error) => {
            if error.kind() == clap::error::ErrorKind::DisplayHelp {
                return write_text(stdout, &error.render().to_string(), stderr, 0);
            }
            let requested_json = args_have_json_output(&args);
            let repo_error = RepoComError::usage(error.render().to_string());
            return emit_error(
                repo_error,
                Route::Operations(operations::OperationsCommand::ConfigValidate),
                requested_json,
                args_have_diagnostics(&args),
                stdout,
                stderr,
            );
        }
    };
    let requested_json = cli.global.is_machine_mode() || requested_json;
    if cli.version {
        return write_text(
            stdout,
            &format!("{}\n", env!("CARGO_PKG_VERSION")),
            stderr,
            0,
        );
    }
    let route = match route_for(cli.command.clone()) {
        Ok(route) => route,
        Err(error) => {
            return emit_error(
                error,
                Route::Operations(operations::OperationsCommand::ConfigValidate),
                requested_json,
                cli.global.diagnostics_enabled(),
                stdout,
                stderr,
            );
        }
    };

    let tty_mode = explicit_tty_mode(&cli);
    let mut bytes = Vec::new();
    if input
        .take(MAX_STRUCTURED_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return emit_error(
            RepoComError::usage("structured stdin could not be read"),
            route,
            requested_json,
            cli.global.diagnostics_enabled(),
            stdout,
            stderr,
        );
    }
    if bytes.len() as u64 > MAX_STRUCTURED_INPUT_BYTES {
        return emit_error(
            RepoComError::usage("structured stdin exceeds the bounded input size"),
            route,
            requested_json,
            cli.global.diagnostics_enabled(),
            stdout,
            stderr,
        );
    }

    if let Err(error) = validate_route_envelope(route, &bytes) {
        return emit_error(
            error,
            route,
            requested_json,
            cli.global.diagnostics_enabled(),
            stdout,
            stderr,
        );
    }

    let current_dir = match env::current_dir() {
        Ok(path) => path,
        Err(_) => {
            return emit_error(
                RepoComError::usage("the current working directory is unavailable"),
                route,
                requested_json,
                cli.global.diagnostics_enabled(),
                stdout,
                stderr,
            );
        }
    };
    let config = match resolve_config(&cli.global, &current_dir) {
        Ok(config) => config,
        Err(error) => {
            return emit_error(
                error,
                route,
                requested_json,
                cli.global.diagnostics_enabled(),
                stdout,
                stderr,
            );
        }
    };
    let now = current_unix_seconds();
    let diagnostic = cli.global.diagnostics_enabled().then(|| {
        format!(
            "repo-com diagnostics: command={} output={}",
            route.canonical(),
            if requested_json { "json" } else { "human" }
        )
    });

    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| RepoComError::internal_failure("the async runtime could not start"))
        .and_then(|runtime| {
            runtime.block_on(async {
                dispatch_route(
                    route,
                    &cli,
                    &config,
                    &bytes,
                    tty_mode,
                    now,
                    diagnostic.clone(),
                )
                .await
            })
        });

    match result {
        Ok(result) => {
            if write!(stdout, "{}", result.stdout).is_err() {
                let _ = writeln!(stderr, "repo-com: stdout could not be written");
                return 1;
            }
            if !result.stderr.is_empty() && writeln!(stderr, "{}", result.stderr).is_err() {
                return 1;
            }
            result.exit_code
        }
        Err(error) => emit_error(
            error,
            route,
            requested_json,
            cli.global.diagnostics_enabled(),
            stdout,
            stderr,
        ),
    }
}

fn explicit_tty_mode(cli: &Cli) -> TtyMode {
    let stdin_is_tty = io::stdin().is_terminal();
    let stdout_is_tty = io::stdout().is_terminal();
    let detected = TtyMode::from_stream_states(stdin_is_tty, stdout_is_tty);
    if cli.non_tty || cli.global.is_machine_mode() {
        TtyMode::NonTty
    } else if cli.tty {
        // An explicit request cannot manufacture a TTY.  This preserves the
        // foundation rule that either relevant stream being piped fails closed.
        if detected.is_tty() {
            TtyMode::Tty
        } else {
            TtyMode::NonTty
        }
    } else {
        detected
    }
}

fn args_have_json_output(args: &[OsString]) -> bool {
    args.windows(2)
        .any(|window| window[0] == "--output" && window[1] == "json")
        || args.iter().any(|arg| arg == "--output=json")
}

fn args_have_diagnostics(args: &[OsString]) -> bool {
    args.windows(2)
        .any(|window| window[0] == "--diagnostics" && window[1] == "on")
        || args.iter().any(|arg| arg == "--diagnostics=on")
}

fn normalize_command_spelling(args: Vec<OsString>) -> Vec<OsString> {
    let mut normalized = args;
    let mut index = 1;
    let command_index = loop {
        if index >= normalized.len() {
            break index;
        }
        let arg = &normalized[index];
        if matches!(
            arg.to_string_lossy().as_ref(),
            "--config" | "--output" | "--color" | "--diagnostics" | "--state"
        ) {
            index = index.saturating_add(2);
            continue;
        }
        if arg.to_string_lossy().starts_with('-') {
            index += 1;
            continue;
        }
        break index;
    };
    if command_index >= normalized.len() {
        return normalized;
    }
    let index = command_index;
    let value = normalized[index].to_string_lossy().into_owned();
    let replacement = match value.as_str() {
        "config.validate" | "config-validate" | "config_validate" => Some(("config", "validate")),
        "policy.status" | "policy-status" | "policy_status" => Some(("policy", "status")),
        "policy.activate" | "policy-activate" | "policy_activate" => Some(("policy", "activate")),
        "state.verify" | "state-verify" | "state_verify" => Some(("state", "verify")),
        "state.inspect" | "state-inspect" | "state_inspect" | "lifecycle.inspect"
        | "lifecycle-inspect" | "lifecycle_inspect" => Some(("state", "inspect")),
        "audit.query" | "audit-query" | "audit_query" => Some(("audit", "query")),
        "purge.plan" | "purge-plan" | "purge_plan" => Some(("purge", "plan")),
        "purge.execute" | "purge-execute" | "purge_execute" => Some(("purge", "execute")),
        "draft.create" | "draft-create" | "draft_create" => Some(("draft", "create")),
        "draft.show" | "draft-show" | "draft_show" => Some(("draft", "show")),
        "draft.update" | "draft-update" | "draft_update" => Some(("draft", "update")),
        "draft.preview" | "draft-preview" | "draft_preview" => Some(("draft", "preview")),
        "draft.approve" | "draft-approve" | "draft_approve" => Some(("draft", "approve")),
        "draft.secret-override" | "draft-secret-override" | "draft_secret_override" => {
            Some(("draft", "secret-override"))
        }
        "send" => None,
        "setup.check" | "setup_check" => Some(("setup", "check")),
        "inbox.fetch" | "inbox-fetch" | "inbox_fetch" => Some(("inbox", "fetch")),
        "inbox.acknowledge" | "inbox-acknowledge" | "inbox_acknowledge" => {
            Some(("inbox", "acknowledge"))
        }
        "inbox.archive" | "inbox-archive" | "inbox_archive" => Some(("inbox", "archive")),
        "reply.draft-create" | "reply-draft-create" | "reply_draft_create" => {
            Some(("reply", "draft-create"))
        }
        _ => None,
    };
    if let Some((group, action)) = replacement {
        normalized.splice(
            index..=index,
            [OsString::from(group), OsString::from(action)],
        );
    }
    normalized
}

fn validate_route_envelope(route: Route, bytes: &[u8]) -> Result<(), RepoComError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        RepoComError::usage("structured input must be a strict protocol-version-1 command object")
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| RepoComError::usage("structured input must be a JSON object"))?;
    if object.get("protocol_version").and_then(Value::as_u64) != Some(1) {
        return Err(RepoComError::usage(
            "unsupported structured protocol version",
        ));
    }
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| RepoComError::usage("structured input must name one explicit command"))?;
    if !object.contains_key("input") {
        return Err(RepoComError::usage(
            "structured input must contain one command input object",
        ));
    }
    match route {
        Route::Operations(expected) => {
            let actual = operations::OperationsCommand::parse(command)
                .ok_or_else(|| RepoComError::usage("unknown operations command"))?;
            if actual != expected {
                return Err(RepoComError::usage(
                    "structured command does not match the selected command-tree route",
                ));
            }
            operations::input::parse(bytes).map(|_| ())?;
        }
        Route::Messaging(expected) => {
            let actual = messaging::MessagingCommand::parse(command)
                .ok_or_else(|| RepoComError::usage("unknown messaging command"))?;
            if actual != expected {
                return Err(RepoComError::usage(
                    "structured command does not match the selected command-tree route",
                ));
            }
            messaging::input::parse(bytes).map(|_| ())?;
        }
    }
    Ok(())
}

fn resolve_config(global: &GlobalArgs, current_dir: &Path) -> Result<ResolvedConfig, RepoComError> {
    let root = repo_com_config::find_repository_root(current_dir)
        .map_err(|error| error.to_repo_com_error())?;
    ConfigResolver::resolve(global.config_path.as_deref(), current_dir, &root)
        .map_err(|error| error.to_repo_com_error())
}

async fn dispatch_route(
    route: Route,
    cli: &Cli,
    config: &ResolvedConfig,
    bytes: &[u8],
    tty_mode: TtyMode,
    now: u64,
    diagnostic: Option<String>,
) -> Result<OutputResult, RepoComError> {
    match route {
        Route::Operations(command) => {
            dispatch_operations(command, cli, config, bytes, tty_mode, diagnostic)
        }
        Route::Messaging(command) => {
            dispatch_messaging(command, cli, config, bytes, tty_mode, now, diagnostic).await
        }
    }
}

fn dispatch_operations(
    command: operations::OperationsCommand,
    cli: &Cli,
    config: &ResolvedConfig,
    bytes: &[u8],
    tty_mode: TtyMode,
    diagnostic: Option<String>,
) -> Result<OutputResult, RepoComError> {
    let state = if matches!(
        command,
        operations::OperationsCommand::ConfigValidate | operations::OperationsCommand::StateVerify
    ) {
        None
    } else if matches!(
        command,
        operations::OperationsCommand::PolicyActivate | operations::OperationsCommand::PurgeExecute
    ) && tty_mode.is_non_tty()
    {
        // The handler owns the TTY gate. Avoid opening or creating local
        // state before it can fail closed in automation.
        None
    } else {
        Some(open_state(cli.state_path.as_deref())?)
    };
    let mut service = OperationsRuntime { state };
    let mut ui =
        operations::KeyboardOperationsUi::new(repo_com_terminal_operations::StdinPrompt, tty_mode);
    let context =
        operations::HandlerContext::from_args(&cli.global, tty_mode, current_unix_seconds());
    let outcome = operations::dispatch_json(&mut service, &mut ui, context, config, bytes);
    render_operations_outcome(&outcome, &cli.global, tty_mode, diagnostic)
}

async fn dispatch_messaging(
    command: messaging::MessagingCommand,
    cli: &Cli,
    config: &ResolvedConfig,
    bytes: &[u8],
    tty_mode: TtyMode,
    now: u64,
    diagnostic: Option<String>,
) -> Result<OutputResult, RepoComError> {
    let state = if matches!(command, messaging::MessagingCommand::SetupCheck) {
        None
    } else if matches!(
        command,
        messaging::MessagingCommand::DraftApprove
            | messaging::MessagingCommand::DraftSecretOverride
    ) && tty_mode.is_non_tty()
    {
        // The handler owns the TTY gate. Keep automation from creating a
        // database before a non-interactive approval or override is rejected.
        None
    } else {
        Some(open_state(cli.state_path.as_deref())?)
    };
    let mut service = MessagingRuntime {
        state,
        config: config.clone(),
    };
    let mut ui =
        messaging::KeyboardOutboundUi::new(repo_com_terminal_outbound::StdinPrompt, tty_mode, now);
    let context = messaging::HandlerContext::from_args(&cli.global, tty_mode, now);
    let outcome = messaging::dispatch_json(&mut service, &mut ui, context, config, bytes).await;
    render_messaging_outcome(&outcome, &cli.global, tty_mode, diagnostic)
}

fn render_operations_outcome(
    outcome: &CommandOutcome<operations::OperationsOutput>,
    global: &GlobalArgs,
    tty_mode: TtyMode,
    diagnostic: Option<String>,
) -> Result<OutputResult, RepoComError> {
    if global.is_machine_mode() {
        let streams = operations::machine_output_streams(outcome, diagnostic)?;
        return Ok(OutputResult {
            exit_code: outcome.exit_code(),
            stdout: streams.stdout,
            stderr: streams.stderr,
        });
    }
    let options = OperationsRenderOptions::from_environment(global.color, tty_mode);
    Ok(OutputResult {
        exit_code: outcome.exit_code(),
        stdout: operations::render_human_outcome(outcome, options),
        stderr: diagnostic.unwrap_or_default(),
    })
}

fn render_messaging_outcome(
    outcome: &CommandOutcome<messaging::MessagingOutput>,
    global: &GlobalArgs,
    tty_mode: TtyMode,
    diagnostic: Option<String>,
) -> Result<OutputResult, RepoComError> {
    if global.is_machine_mode() {
        let streams = messaging::machine_output_streams(outcome, diagnostic)?;
        return Ok(OutputResult {
            exit_code: outcome.exit_code(),
            stdout: streams.stdout,
            stderr: streams.stderr,
        });
    }
    let options = OutboundRenderOptions::from_environment(global.color, tty_mode);
    Ok(OutputResult {
        exit_code: outcome.exit_code(),
        stdout: messaging::render_human_outcome(outcome, options),
        stderr: diagnostic.unwrap_or_default(),
    })
}

fn open_state(path: Option<&Path>) -> Result<StateStore, RepoComError> {
    path.map_or_else(StateStore::open, StateStore::open_path)
        .map_err(|error| error.to_repo_com_error())
}

/// A state-backed operations composition.  Each method delegates validation and
/// mutation to the existing domain owner; this type only supplies the owner.
struct OperationsRuntime {
    state: Option<StateStore>,
}

impl OperationsRuntime {
    fn state_ref(&self) -> Result<&StateStore, RepoComError> {
        self.state.as_ref().ok_or_else(|| {
            RepoComError::storage_integrity("the selected state service is not available")
        })
    }

    fn state_mut(&mut self) -> Result<&mut StateStore, RepoComError> {
        self.state.as_mut().ok_or_else(|| {
            RepoComError::storage_integrity("the selected state service is not available")
        })
    }
}

impl operations::ConfigService for OperationsRuntime {
    fn validate(
        &mut self,
        config: &ResolvedConfig,
        request: operations::ConfigValidationRequest,
    ) -> operations::OperationsResult<ConfigStatusView> {
        if let Some(path) = request.config_path.as_ref() {
            let requested = path.canonicalize().map_err(|_| {
                RepoComError::usage("config_path does not identify an existing configuration")
            })?;
            if requested != config.path {
                return Err(RepoComError::usage(
                    "config_path does not identify the explicitly resolved configuration",
                ));
            }
        }
        Ok(ConfigStatusView::from_resolved(config))
    }
}

impl operations::PolicyService for OperationsRuntime {
    fn status(
        &mut self,
        config: &ResolvedConfig,
        request: operations::PolicyStatusRequest,
    ) -> operations::OperationsResult<PolicyStatusView> {
        let status =
            repo_com_policy::status_from_state(self.state_ref()?, &config.config, &request.tuple)
                .map_err(map_policy_owned)?;
        Ok(PolicyStatusView::from_status(
            request.repository_id,
            &status,
        ))
    }

    fn activation_preview(
        &mut self,
        config: &ResolvedConfig,
        request: operations::PolicyActivationRequest,
    ) -> operations::OperationsResult<ActivationPreviewView> {
        let registry = PolicyRegistry::new(self.state_mut()?);
        let preview = registry
            .preview(
                &config.config,
                &request.tuple,
                request.activation_id.as_deref(),
            )
            .map_err(map_policy_owned)?;
        Ok(ActivationPreviewView::from(&preview))
    }

    fn activate(
        &mut self,
        config: &ResolvedConfig,
        request: operations::PolicyActivationRequest,
        _confirmation: repo_com_terminal_operations::ExactConfirmation,
        tty_mode: TtyMode,
    ) -> operations::OperationsResult<repo_com_policy::ActivationReceipt> {
        let mut registry = PolicyRegistry::new(self.state_mut()?);
        let confirmation = repo_com_policy::OperatorConfirmation::from_tty(tty_mode);
        let result = match request.activation_id {
            Some(activation_id) => registry.activate_with_id(
                &config.config,
                &request.tuple,
                activation_id,
                confirmation,
                request.activated_at,
            ),
            None => registry.activate(
                &config.config,
                &request.tuple,
                confirmation,
                request.activated_at,
            ),
        };
        result.map_err(map_policy_owned)
    }
}

impl operations::StateService for OperationsRuntime {
    fn verify(
        &mut self,
        request: operations::StateVerificationRequest,
    ) -> operations::OperationsResult<StateVerificationView> {
        Ok(operations::handlers::state::project_verification(
            &repo_com_lifecycle::StateVerifier::new().verify(&request.to_domain()),
        ))
    }

    fn inspect(
        &mut self,
        request: operations::LifecycleInspectionRequest,
    ) -> operations::OperationsResult<operations::LifecycleInspectionResult> {
        let domain_request = request.to_domain()?;
        let projection = LifecycleInspector::new(self.state_ref()?)
            .inspect(&domain_request)
            .map_err(map_lifecycle_owned)?;
        Ok(operations::LifecycleInspectionResult::from_projection(
            &projection,
        ))
    }
}

impl operations::AuditService for OperationsRuntime {
    fn query(
        &mut self,
        request: operations::AuditQueryRequest,
    ) -> operations::OperationsResult<AuditView> {
        operations::LocalAuditService::new(self.state_ref()?).query(request)
    }
}

impl operations::PurgeService for OperationsRuntime {
    fn plan(
        &mut self,
        request: operations::PurgePlanRequest,
    ) -> operations::OperationsResult<repo_com_purge::PurgePlan> {
        repo_com_purge::PurgePlanner::new()
            .plan(self.state_ref()?, &request.request)
            .map_err(map_purge_owned)
    }

    fn execute(
        &mut self,
        request: operations::PurgeExecutionRequest,
    ) -> operations::OperationsResult<repo_com_purge::PurgeExecution> {
        let state = self
            .state_ref()?
            .reopen()
            .map_err(|error| error.to_repo_com_error())?;
        PurgeExecutor::new(state)
            .execute_with_tty(
                &request.plan,
                &request.confirmation,
                request.tty_mode,
                request.executed_at,
            )
            .map_err(map_purge_owned)
    }
}

/// A messaging composition.  The handler remains responsible for protocol and
/// TTY routing; this adapter only supplies the existing domain ports.
struct MessagingRuntime {
    state: Option<StateStore>,
    config: ResolvedConfig,
}

impl MessagingRuntime {
    fn state_ref(&self) -> Result<&StateStore, RepoComError> {
        self.state.as_ref().ok_or_else(|| {
            RepoComError::storage_integrity("the selected state service is not available")
        })
    }

    fn state_mut(&mut self) -> Result<&mut StateStore, RepoComError> {
        self.state.as_mut().ok_or_else(|| {
            RepoComError::storage_integrity("the selected state service is not available")
        })
    }

    fn revalidate_reply_target(
        &self,
        model: &DraftModel,
        now_unix_seconds: u64,
    ) -> Result<(), RepoComError> {
        let Some(reference) = model.current_revision().reply_reference() else {
            return Ok(());
        };
        let request = ReplyTargetRequest::new(
            reference.repository_id(),
            reference.inbound_item_id(),
            now_unix_seconds,
        )
        .map_err(map_reply_target_error)?;
        let state = self
            .state_ref()?
            .reopen()
            .map_err(|error| error.to_repo_com_error())?;
        let inbox = InboxState::from_state(state);
        let validator = ReplyTargetValidator::new();
        let target = validator
            .validate(&inbox, &self.config, &request)
            .map_err(map_reply_target_error)?;
        validator
            .revalidate_before_eligibility(&inbox, &self.config, &target, model, now_unix_seconds)
            .map(|_| ())
            .map_err(map_reply_target_error)
    }

    fn ensure_repository(&mut self, timestamp: &str) -> Result<(), RepoComError> {
        let repository_id = self.config.config.repository_id.clone();
        let workspace_id = self.config.config.discord.workspace_id.clone();
        let config_hash = self.config.canonical_hash();
        let state = self.state_mut()?;
        if state
            .repository(&repository_id)
            .map_err(|error| error.to_repo_com_error())?
            .is_some()
        {
            return Ok(());
        }
        state
            .upsert_repository(&RepositoryInput::new(
                repository_id,
                workspace_id,
                config_hash,
                timestamp,
            ))
            .map_err(|error| error.to_repo_com_error())?;
        Ok(())
    }

    fn load_model(&self, identity: &messaging::DraftIdentity) -> Result<DraftModel, RepoComError> {
        let state = self.state_ref()?;
        let draft = state
            .draft(&identity.repository_id, &identity.draft_id)
            .map_err(|error| error.to_repo_com_error())?
            .ok_or_else(|| RepoComError::usage("the requested draft does not exist"))?;
        let records = state
            .repositories()
            .drafts()
            .revisions(&identity.repository_id, &identity.draft_id)
            .map_err(|error| error.to_repo_com_error())?;
        if records.is_empty()
            || u64::try_from(draft.current_revision).ok()
                != records
                    .last()
                    .and_then(|record| u64::try_from(record.revision).ok())
        {
            return Err(RepoComError::storage_integrity(
                "stored draft revision sequence is inconsistent",
            ));
        }
        if !records
            .iter()
            .any(|record| u64::try_from(record.revision).ok() == Some(identity.revision))
        {
            return Err(RepoComError::usage(
                "the requested draft revision does not exist",
            ));
        }
        let mut model: Option<DraftModel> = None;
        for record in records {
            let raw_body = raw_body_from_rendered(&record.body, &record.content_hash)?;
            let revision_number = u64::try_from(record.revision)
                .map_err(|_| RepoComError::storage_integrity("stored revision is out of range"))?;
            let metadata = match serde_json::from_str(&record.metadata_json) {
                Ok(metadata) => metadata,
                Err(_) => decode_revision_metadata(&record.metadata_json, revision_number)
                    .map(|persisted| persisted.metadata)?,
            };
            let persisted = decode_revision_metadata(&draft.metadata_json, revision_number)
                .or_else(|_| decode_revision_metadata(&record.metadata_json, revision_number))?;
            if persisted.metadata != metadata {
                return Err(RepoComError::storage_integrity(
                    "stored draft metadata does not match its revision",
                ));
            }
            let created_at = parse_timestamp_seconds(&record.created_at)?;
            let expires_at = record
                .expiry_at
                .as_deref()
                .map(parse_expiry_seconds)
                .transpose()?;
            let lifetime = expires_at
                .map(|expires| expires.saturating_sub(created_at))
                .unwrap_or(repo_com_draft_model::DEFAULT_EXPIRY_SECONDS);
            let mut request = DraftRequest::new(
                record.draft_id.clone(),
                record.destination_alias.clone(),
                raw_body,
                persisted.event_type,
                persisted.severity,
            )
            .map_err(|error| RepoComError::usage(error.to_string()))?
            .with_metadata(metadata)
            .with_expiry_seconds(lifetime);
            if let Some(reply_reference) = persisted.reply_reference {
                request = request.with_reply_reference(reply_reference);
            }
            let revision = if let Some(model) = model.as_mut() {
                model
                    .revise(request, &self.config, created_at)
                    .map_err(|error| RepoComError::usage(error.to_string()))?
                    .clone()
            } else {
                let created = DraftModel::create(request, &self.config, created_at)
                    .map_err(|error| RepoComError::usage(error.to_string()))?;
                let revision = created.current_revision().clone();
                model = Some(created);
                revision
            };
            let rendered = ContentRenderer::new()
                .render_revision(&revision, created_at)
                .map_err(|error| RepoComError::storage_integrity(error.to_string()))?;
            let body_matches =
                rendered.exact_text() == record.body || revision.exact_text() == record.body;
            if revision.content_hash() != record.content_hash
                || !body_matches
                || revision.destination_alias().as_str() != record.destination_alias
            {
                return Err(RepoComError::storage_integrity(
                    "stored draft revision does not match its deterministic model",
                ));
            }
        }
        model.ok_or_else(|| {
            RepoComError::storage_integrity("stored draft has no immutable revisions")
        })
    }

    fn persist_created(&mut self, model: &DraftModel, timestamp: &str) -> Result<(), RepoComError> {
        let revision = model.current_revision();
        let draft_metadata_json = encode_revision_metadata(model)?;
        let revision_metadata_json = serde_json::to_string(revision.metadata()).map_err(|_| {
            RepoComError::internal_failure("draft metadata could not be serialized")
        })?;
        let resolved_destination =
            serde_json::to_string(revision.resolved_destination()).map_err(|_| {
                RepoComError::internal_failure("draft destination could not be serialized")
            })?;
        let expiry_at = revision.expiry().expires_at_unix_seconds().to_string();
        let draft_input = DraftInput {
            repository_id: self.config.config.repository_id.clone(),
            draft_id: model.draft_id().as_str().to_owned(),
            event_type: revision.event_type().as_str().to_owned(),
            destination_alias: revision.destination_alias().as_str().to_owned(),
            status: "draft".to_owned(),
            expiry_at: Some(expiry_at.clone()),
            reply_to_inbound_item_id: revision
                .reply_reference()
                .map(|reference| reference.inbound_item_id().to_owned()),
            metadata_json: draft_metadata_json.clone(),
            created_at: timestamp.to_owned(),
            updated_at: timestamp.to_owned(),
        };
        let rendered = ContentRenderer::new()
            .render_revision(revision, revision.expiry().created_at_unix_seconds())
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let revision_input = DraftRevisionInput {
            repository_id: self.config.config.repository_id.clone(),
            draft_id: model.draft_id().as_str().to_owned(),
            revision: i64::try_from(revision.number())
                .map_err(|_| RepoComError::internal_failure("draft revision is out of range"))?,
            content_hash: revision.content_hash().to_owned(),
            body: rendered.exact_text().to_owned(),
            metadata_json: revision_metadata_json,
            destination_alias: revision.destination_alias().as_str().to_owned(),
            resolved_destination,
            expiry_at: Some(expiry_at),
            lifecycle_state: "draft".to_owned(),
            reply_to_inbound_item_id: revision
                .reply_reference()
                .map(|reference| reference.inbound_item_id().to_owned()),
            created_at: timestamp.to_owned(),
        };
        let state = self.state_mut()?;
        let transaction = state
            .begin_transaction()
            .map_err(|error| error.to_repo_com_error())?;
        transaction
            .repositories()
            .drafts()
            .create(&draft_input)
            .map_err(|error| error.to_repo_com_error())?;
        transaction
            .repositories()
            .drafts()
            .insert_revision(&revision_input)
            .map_err(|error| error.to_repo_com_error())?;
        transaction
            .commit()
            .map_err(|error| error.to_repo_com_error())
    }

    fn persist_revision(
        &mut self,
        model: &DraftModel,
        timestamp: &str,
    ) -> Result<(), RepoComError> {
        let revision = model.current_revision();
        let draft_metadata_json = encode_revision_metadata(model)?;
        let revision_metadata_json = serde_json::to_string(revision.metadata()).map_err(|_| {
            RepoComError::internal_failure("draft metadata could not be serialized")
        })?;
        let resolved_destination =
            serde_json::to_string(revision.resolved_destination()).map_err(|_| {
                RepoComError::internal_failure("draft destination could not be serialized")
            })?;
        let rendered = ContentRenderer::new()
            .render_revision(revision, revision.expiry().created_at_unix_seconds())
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let revision_input = DraftRevisionInput {
            repository_id: self.config.config.repository_id.clone(),
            draft_id: model.draft_id().as_str().to_owned(),
            revision: i64::try_from(revision.number())
                .map_err(|_| RepoComError::internal_failure("draft revision is out of range"))?,
            content_hash: revision.content_hash().to_owned(),
            body: rendered.exact_text().to_owned(),
            metadata_json: revision_metadata_json,
            destination_alias: revision.destination_alias().as_str().to_owned(),
            resolved_destination,
            expiry_at: Some(revision.expiry().expires_at_unix_seconds().to_string()),
            lifecycle_state: "draft".to_owned(),
            reply_to_inbound_item_id: revision
                .reply_reference()
                .map(|reference| reference.inbound_item_id().to_owned()),
            created_at: timestamp.to_owned(),
        };
        let repository_id = self.config.config.repository_id.clone();
        let draft_id = model.draft_id().as_str().to_owned();
        let event_type = revision.event_type().as_str().to_owned();
        let destination_alias = revision.destination_alias().as_str().to_owned();
        let expiry_at = revision.expiry().expires_at_unix_seconds().to_string();
        let metadata_for_update = draft_metadata_json;
        let state = self.state_mut()?;
        let transaction = state
            .begin_transaction()
            .map_err(|error| error.to_repo_com_error())?;
        transaction
            .execute(
                "UPDATE drafts SET event_type = ?3, destination_alias = ?4, expiry_at = ?5,
                metadata_json = ?6, updated_at = ?7
             WHERE repository_id = ?1 AND draft_id = ?2",
                rusqlite::params![
                    repository_id,
                    draft_id,
                    event_type,
                    destination_alias,
                    expiry_at,
                    metadata_for_update,
                    timestamp,
                ],
            )
            .map_err(|error| RepoComError::storage_integrity(error.to_string()))?;
        transaction
            .repositories()
            .drafts()
            .insert_revision(&revision_input)
            .map_err(|error| error.to_repo_com_error())?;
        transaction
            .commit()
            .map_err(|error| error.to_repo_com_error())
    }
}

fn persist_reply_metadata(state: &mut StateStore, model: &DraftModel) -> Result<(), RepoComError> {
    let revision = model.current_revision();
    let metadata_json = encode_revision_metadata(model)?;
    let repository_id = revision.repository_id().to_owned();
    let draft_id = model.draft_id().as_str().to_owned();
    let transaction = state
        .begin_transaction()
        .map_err(|error| error.to_repo_com_error())?;
    transaction
        .execute(
            "UPDATE drafts SET metadata_json = ?3
             WHERE repository_id = ?1 AND draft_id = ?2",
            rusqlite::params![repository_id, draft_id, metadata_json],
        )
        .map_err(|error| RepoComError::storage_integrity(error.to_string()))?;
    transaction
        .commit()
        .map_err(|error| error.to_repo_com_error())
}

impl messaging::DraftService for MessagingRuntime {
    async fn create(
        &mut self,
        request: messaging::CreateDraft,
    ) -> messaging::MessagingResult<messaging::DraftView> {
        self.ensure_repository(&request.created_at)?;
        let draft_request = with_optional_expiry(
            DraftRequest::new(
                request.draft_id.clone(),
                request.destination_alias.clone(),
                request.text.clone(),
                request.event_type.clone(),
                request.severity.clone(),
            )
            .map_err(|error| RepoComError::usage(error.to_string()))?
            .with_metadata(request.metadata.clone()),
            request.expires_in_seconds,
        );
        let model =
            DraftModel::create(draft_request, &self.config, request.created_at_unix_seconds)
                .map_err(|error| RepoComError::usage(error.to_string()))?;
        self.persist_created(&model, &request.created_at)?;
        let body = ContentRenderer::new()
            .render_revision(
                model.current_revision(),
                model.current_revision().expiry().created_at_unix_seconds(),
            )
            .map_err(|error| RepoComError::usage(error.to_string()))?
            .exact_text()
            .to_owned();
        Ok(messaging::DraftView::from_revision(
            model.draft_id().as_str(),
            model.current_revision(),
            body,
        ))
    }

    async fn show(
        &mut self,
        identity: messaging::DraftIdentity,
    ) -> messaging::MessagingResult<messaging::DraftView> {
        let model = self.load_model(&identity)?;
        let revision = model
            .revision(identity.revision)
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?;
        let record_body = self
            .state_ref()?
            .draft_revision(
                &identity.repository_id,
                &identity.draft_id,
                i64::try_from(identity.revision)
                    .map_err(|_| RepoComError::usage("revision is out of range"))?,
            )
            .map_err(|error| error.to_repo_com_error())?
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?
            .body;
        Ok(messaging::DraftView::from_revision(
            identity.draft_id,
            revision,
            record_body,
        ))
    }

    async fn update(
        &mut self,
        request: messaging::UpdateDraft,
    ) -> messaging::MessagingResult<messaging::DraftView> {
        let identity = request.identity.clone();
        let mut model = self.load_model(&identity)?;
        self.revalidate_reply_target(&model, request.created_at_unix_seconds)?;
        if model.current_revision().number() != identity.revision {
            return Err(RepoComError::usage(
                "draft update must replace the current immutable revision",
            ));
        }
        let mut draft_request = with_optional_expiry(
            DraftRequest::new(
                identity.draft_id.clone(),
                request.destination_alias.clone(),
                request.text.clone(),
                request.event_type.clone(),
                request.severity.clone(),
            )
            .map_err(|error| RepoComError::usage(error.to_string()))?
            .with_metadata(request.metadata.clone()),
            request.expires_in_seconds,
        );
        if let Some(reply_reference) = model.current_revision().reply_reference().cloned() {
            draft_request = draft_request.with_reply_reference(reply_reference);
        }
        let updated = model
            .revise(draft_request, &self.config, request.created_at_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?
            .clone();
        self.persist_revision(&model, &request.created_at)?;
        let body = self
            .state_ref()?
            .draft_revision(
                &identity.repository_id,
                &identity.draft_id,
                i64::try_from(updated.number())
                    .map_err(|_| RepoComError::usage("revision is out of range"))?,
            )?
            .map(|record| record.body)
            .unwrap_or_else(|| updated.exact_text().to_owned());
        Ok(messaging::DraftView::from_revision(
            identity.draft_id,
            &updated,
            body,
        ))
    }

    async fn preview(
        &mut self,
        identity: messaging::DraftIdentity,
        now_unix_seconds: u64,
    ) -> messaging::MessagingResult<OutboundPreview> {
        let model = self.load_model(&identity)?;
        self.revalidate_reply_target(&model, now_unix_seconds)?;
        let revision = model
            .revision(identity.revision)
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?;
        let rendered = ContentRenderer::new()
            .render_revision(revision, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let preview = rendered
            .preview(&model, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        Ok(OutboundPreview::from_draft_preview(&preview))
    }

    async fn approval_preview(
        &mut self,
        identity: messaging::DraftIdentity,
        now_unix_seconds: u64,
    ) -> messaging::MessagingResult<OutboundPreview> {
        let model = self.load_model(&identity)?;
        self.revalidate_reply_target(&model, now_unix_seconds)?;
        let revision = model
            .revision(identity.revision)
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?;
        let rendered = ContentRenderer::new()
            .render_revision(revision, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let draft_preview = rendered
            .preview(&model, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let state = self
            .state_ref()?
            .reopen()
            .map_err(|error| error.to_repo_com_error())?;
        let service = ApprovalService::new(state);
        let instant = approval_instant(now_unix_seconds)?;
        let preview = service
            .preview_at(&draft_preview, &self.config, &instant)
            .map_err(map_approval_error)?;
        Ok(OutboundPreview::from_approval_preview(&preview))
    }

    async fn approve(
        &mut self,
        identity: messaging::DraftIdentity,
        _confirmation: repo_com_terminal_outbound::ExactConfirmation,
        tty_mode: TtyMode,
        now_unix_seconds: u64,
    ) -> messaging::MessagingResult<repo_com_approval::ApprovalRecord> {
        self.ensure_repository(&format_rfc3339_utc(now_unix_seconds))?;
        let model = self.load_model(&identity)?;
        self.revalidate_reply_target(&model, now_unix_seconds)?;
        let revision = model
            .revision(identity.revision)
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?;
        let rendered = ContentRenderer::new()
            .render_revision(revision, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let draft_preview = rendered
            .preview(&model, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let state = self
            .state_ref()?
            .reopen()
            .map_err(|error| error.to_repo_com_error())?;
        let mut service = ApprovalService::new(state);
        let instant = approval_instant(now_unix_seconds)?;
        let domain_confirmation = OperatorConfirmation::from_preview(
            &service
                .preview_at(&draft_preview, &self.config, &instant)
                .map_err(map_approval_error)?,
            tty_mode,
        );
        service
            .approve(
                &draft_preview,
                &self.config,
                &domain_confirmation,
                &FixedApprovalClock(instant),
            )
            .map_err(map_approval_error)
    }

    async fn override_secret_finding(
        &mut self,
        identity: messaging::DraftIdentity,
        _confirmation: repo_com_terminal_outbound::ExactConfirmation,
        tty_mode: TtyMode,
        reason: OverrideReasonCode,
        now_unix_seconds: u64,
    ) -> messaging::MessagingResult<repo_com_approval::SecretOverrideRecord> {
        self.ensure_repository(&format_rfc3339_utc(now_unix_seconds))?;
        let model = self.load_model(&identity)?;
        self.revalidate_reply_target(&model, now_unix_seconds)?;
        let revision = model
            .revision(identity.revision)
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?;
        let rendered = ContentRenderer::new()
            .render_revision(revision, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let draft_preview = rendered
            .preview(&model, now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let state = self
            .state_ref()?
            .reopen()
            .map_err(|error| error.to_repo_com_error())?;
        let mut service = ApprovalService::new(state);
        let instant = approval_instant(now_unix_seconds)?;
        let preview = service
            .preview_at(&draft_preview, &self.config, &instant)
            .map_err(map_approval_error)?;
        let domain_confirmation = OperatorConfirmation::from_preview(&preview, tty_mode);
        service
            .override_secret_finding(
                &draft_preview,
                &self.config,
                reason,
                &domain_confirmation,
                &FixedApprovalClock(instant),
            )
            .map_err(map_approval_error)
    }
}

impl messaging::SendService for MessagingRuntime {
    async fn send(
        &mut self,
        request: messaging::SendRequest,
        config: &ResolvedConfig,
    ) -> messaging::MessagingResult<DeliveryView> {
        self.ensure_repository(&format_rfc3339_utc(request.now_unix_seconds))?;
        let identity = request.identity();
        let model = self.load_model(&identity)?;
        self.revalidate_reply_target(&model, request.now_unix_seconds)?;
        let revision = model
            .revision(request.revision)
            .ok_or_else(|| RepoComError::usage("the requested draft revision does not exist"))?;
        let rendered = ContentRenderer::new()
            .render_revision(revision, request.now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let draft_preview = rendered
            .preview(&model, request.now_unix_seconds)
            .map_err(|error| RepoComError::usage(error.to_string()))?;
        let scan = SecretScanner::new().scan_rendered(&rendered);
        let state = self
            .state_ref()?
            .reopen()
            .map_err(|error| error.to_repo_com_error())?;
        let approval_service = ApprovalService::new(state);
        let instant = approval_instant(request.now_unix_seconds)?;
        let approval_preview = approval_service
            .preview_at(&draft_preview, config, &instant)
            .map_err(map_approval_error)?;
        let approval_check = approval_service
            .check_current(&draft_preview, config, &FixedApprovalClock(instant.clone()))
            .map_err(map_approval_error)?;
        let tuple = repo_com_policy::PolicyTuple::new(
            revision.event_type().as_str().to_owned(),
            revision.destination_alias().as_str().to_owned(),
            revision.severity().as_str().to_owned(),
        );
        let policy_decision = evaluate_from_state(approval_service.state(), &config.config, &tuple)
            .map_err(map_policy_owned)?;
        let decision = EligibilityEvaluator::new().evaluate(&EligibilityInput {
            now_unix_seconds: request.now_unix_seconds,
            tty_mode: request.tty_mode,
            config,
            revision,
            approval_preview: &approval_preview,
            approval_check: &approval_check,
            policy_decision: &policy_decision,
            scan_result: &scan,
        });
        if !decision.is_eligible() {
            return Err(map_eligibility_blocker(
                decision
                    .blocker()
                    .unwrap_or(EligibilityBlocker::CurrentStateInvalid),
            ));
        }
        if let Some(reference) = model.current_revision().reply_reference() {
            let request = ReplyTargetRequest::new(
                reference.repository_id(),
                reference.inbound_item_id(),
                request.now_unix_seconds,
            )
            .map_err(map_reply_target_error)?;
            let state = self
                .state_ref()?
                .reopen()
                .map_err(|error| error.to_repo_com_error())?;
            let inbox = InboxState::from_state(state);
            let validator = ReplyTargetValidator::new();
            let target = validator
                .validate(&inbox, config, &request)
                .map_err(map_reply_target_error)?;
            validator
                .revalidate_for_delivery(
                    &inbox,
                    config,
                    &target,
                    &model,
                    &decision,
                    request.now_unix_seconds(),
                )
                .map_err(map_reply_target_error)?;
        }

        let coordinator_state = self
            .state
            .take()
            .ok_or_else(|| RepoComError::storage_integrity("delivery state is not available"))?;
        let mut coordinator = DeliveryCoordinator::new(coordinator_state);
        let claim = coordinator
            .claim_input(&repo_com_delivery::ClaimInput::new(
                &decision,
                config,
                &rendered,
                tuple,
                request.now_unix_seconds,
                format_rfc3339_utc(request.now_unix_seconds),
            ))
            .map_err(map_delivery_error);
        let claim = match claim {
            Ok(claim) => claim,
            Err(error) => {
                self.state = Some(coordinator.into_state());
                return Err(error);
            }
        };
        let recorded_attempt = claim.attempt.clone();
        let Some(permit) = claim.into_permit() else {
            self.state = Some(coordinator.into_state());
            return Ok(DeliveryView::from_attempt(&recorded_attempt));
        };
        let attempt = permit.attempt().clone();
        let request_message = match CreateMessageRequest::from_rendered(permit.rendered()) {
            Ok(request) => request,
            Err(error) => {
                self.state = Some(coordinator.into_state());
                return Err(RepoComError::usage(error.to_string()));
            }
        };
        let transport = match DiscordMessageClient::from_environment() {
            Ok(client) => client.send(request_message).await,
            Err(error) => Err(error),
        };
        let outcome = transport.as_ref().map_err(classify_transport).map_or_else(
            |error| error,
            |value| repo_com_delivery_retry::TransportOutcome::Accepted {
                message_id: value.message_id().to_owned(),
            },
        );
        let decision = repo_com_delivery_retry::RetryPolicy::new().classify(1, outcome);
        let updated = match decision {
            repo_com_delivery_retry::RetryDecision::Accepted { message_id } => coordinator
                .record_accepted(
                    TransitionRequest::new(
                        attempt.repository_id.clone(),
                        attempt.draft_id.clone(),
                        attempt.revision,
                        attempt.attempt_id.clone(),
                        DeliveryState::Accepted,
                        format_rfc3339_utc(request.now_unix_seconds),
                        "system",
                    )
                    .with_remote_message_id(message_id),
                )
                .map_err(map_delivery_error),
            repo_com_delivery_retry::RetryDecision::Failed { code } => coordinator
                .record_definitive_failed(
                    TransitionRequest::new(
                        attempt.repository_id.clone(),
                        attempt.draft_id.clone(),
                        attempt.revision,
                        attempt.attempt_id.clone(),
                        DeliveryState::Failed,
                        format_rfc3339_utc(request.now_unix_seconds),
                        "system",
                    )
                    .with_error_code(code),
                )
                .map_err(map_delivery_error),
            repo_com_delivery_retry::RetryDecision::Wait { .. } => coordinator
                .record_retry_wait(
                    TransitionRequest::new(
                        attempt.repository_id.clone(),
                        attempt.draft_id.clone(),
                        attempt.revision,
                        attempt.attempt_id.clone(),
                        DeliveryState::RetryWait,
                        format_rfc3339_utc(request.now_unix_seconds),
                        "system",
                    )
                    .with_error_code("retry-required"),
                )
                .map_err(map_delivery_error),
            repo_com_delivery_retry::RetryDecision::Unknown { reason } => coordinator
                .record_unknown(
                    TransitionRequest::new(
                        attempt.repository_id.clone(),
                        attempt.draft_id.clone(),
                        attempt.revision,
                        attempt.attempt_id.clone(),
                        DeliveryState::Unknown,
                        format_rfc3339_utc(request.now_unix_seconds),
                        "system",
                    )
                    .with_error_code(reason.code()),
                )
                .map_err(map_delivery_error),
            repo_com_delivery_retry::RetryDecision::Stop { code, .. } => coordinator
                .record_unknown(
                    TransitionRequest::new(
                        attempt.repository_id.clone(),
                        attempt.draft_id.clone(),
                        attempt.revision,
                        attempt.attempt_id.clone(),
                        DeliveryState::Unknown,
                        format_rfc3339_utc(request.now_unix_seconds),
                        "system",
                    )
                    .with_error_code(code),
                )
                .map_err(map_delivery_error),
        };
        self.state = Some(coordinator.into_state());
        updated.map(|value| DeliveryView::from_attempt(&value))
    }
}

impl messaging::SetupService for MessagingRuntime {
    async fn check_setup(
        &mut self,
        config: &ResolvedConfig,
    ) -> messaging::MessagingResult<repo_com_discord_client::SetupReport> {
        DiscordClient::from_environment()
            .map_err(|error| error.to_repo_com_error())?
            .check_setup(config)
            .await
            .map_err(|error| error.to_repo_com_error())
    }
}

impl messaging::InboxLifecycleService for MessagingRuntime {
    async fn apply(
        &mut self,
        request: messaging::InboundActionRequest,
    ) -> messaging::MessagingResult<messaging::InboundActionResult> {
        let state = self.state_mut()?;
        let result = match request.action {
            messaging::InboundAction::Acknowledge => state
                .acknowledge_inbound(
                    &request.repository_id,
                    &request.item_ids,
                    request.at.clone(),
                )
                .map(|_| ()),
            messaging::InboundAction::Archive => state
                .archive_inbound(
                    &request.repository_id,
                    &request.item_ids,
                    request.at.clone(),
                )
                .map(|_| ()),
        };
        result.map_err(|error| error.to_repo_com_error())?;
        Ok(messaging::InboundActionResult {
            repository_id: request.repository_id,
            action: request.action,
            item_ids: request.item_ids,
            at: request.at,
            remote_mutation: false,
        })
    }
}

impl messaging::InboxFetchService for MessagingRuntime {
    async fn fetch(
        &mut self,
        request: messaging::InboxFetchCommand,
        config: &ResolvedConfig,
    ) -> messaging::MessagingResult<messaging::InboxFetchResult> {
        let domain_request = request.to_domain()?;
        let fetcher = InboundFetcher::from_environment().map_err(map_fetch_error)?;
        let state = self
            .state
            .take()
            .ok_or_else(|| RepoComError::storage_integrity("inbound state is not available"))?;
        let mut inbox = InboxState::from_state(state);
        let result = fetcher
            .fetch_and_store(config, &domain_request, &[], &mut inbox)
            .await;
        self.state = Some(inbox.into_state());
        result
            .map(messaging::InboxFetchResult::from_stored)
            .map_err(map_fetch_error)
    }
}

impl messaging::ReplyService for MessagingRuntime {
    async fn create_reply(
        &mut self,
        request: messaging::ReplyCreateRequest,
        config: &ResolvedConfig,
    ) -> messaging::MessagingResult<messaging::ReplyDraftView> {
        self.ensure_repository(&request.created_at)?;
        let state = self
            .state
            .take()
            .ok_or_else(|| RepoComError::storage_integrity("reply state is not available"))?;
        let mut service = ReplyCommandService::from_state(state);
        let result = service.create_reply(config, request.to_domain());
        let mut state = service.into_state().into_state();
        if let Ok(created) = &result
            && let Err(error) = persist_reply_metadata(&mut state, created.draft())
        {
            self.state = Some(state);
            return Err(error);
        }
        self.state = Some(state);
        result
            .map(|created| messaging::ReplyDraftView::from_created(&created))
            .map_err(|error| msg_reply_error(&error))
    }
}

struct FixedApprovalClock(ApprovalInstant);

impl repo_com_approval::ApprovalClock for FixedApprovalClock {
    fn now(&self) -> ApprovalInstant {
        self.0.clone()
    }
}

fn approval_instant(seconds: u64) -> Result<ApprovalInstant, RepoComError> {
    ApprovalInstant::new(seconds, format_rfc3339_utc(seconds))
        .map_err(|error| RepoComError::usage(error.to_string()))
}

fn map_policy_owned(error: repo_com_policy::PolicyError) -> RepoComError {
    operations::handlers::policy::map_policy_error(&error)
}

fn map_lifecycle_owned(error: repo_com_lifecycle::LifecycleError) -> RepoComError {
    operations::handlers::state::map_lifecycle_error(&error)
}

fn map_purge_owned(error: repo_com_purge::PurgeError) -> RepoComError {
    operations::handlers::purge::map_purge_error(&error)
}

fn map_approval_error(error: repo_com_approval::ApprovalError) -> RepoComError {
    RepoComError::new(error.category(), error.to_string())
}

fn msg_reply_error(error: &repo_com_reply::ReplyServiceError) -> RepoComError {
    messaging::handlers::reply::map_reply_error(error)
}

fn map_reply_target_error(error: ReplyTargetError) -> RepoComError {
    match error {
        ReplyTargetError::InvalidRequest
        | ReplyTargetError::InvalidStoredIdentity
        | ReplyTargetError::InvalidStoredSnapshot
        | ReplyTargetError::MissingTarget
        | ReplyTargetError::CurrentSnapshotMissing
        | ReplyTargetError::NotReplyDraft => {
            RepoComError::usage("the reply target request is invalid")
        }
        ReplyTargetError::State(error) => error.to_repo_com_error(),
        ReplyTargetError::PersistedDraftChanged => {
            RepoComError::storage_integrity("the persisted reply draft is inconsistent")
        }
        ReplyTargetError::RepositoryMismatch
        | ReplyTargetError::WorkspaceMismatch
        | ReplyTargetError::AuthorizationChanged
        | ReplyTargetError::TargetDeleted
        | ReplyTargetError::TargetExpired
        | ReplyTargetError::ChannelNotConfigured
        | ReplyTargetError::AmbiguousInboundAlias
        | ReplyTargetError::AuthorizationDenied
        | ReplyTargetError::TargetChanged
        | ReplyTargetError::EligibilityBlocked(_)
        | ReplyTargetError::EligibilityFactsChanged => {
            RepoComError::policy_blocked("the reply target is not currently authorized")
        }
    }
}

fn map_fetch_error(error: FetchError) -> RepoComError {
    RepoComError::new(error.category(), error.to_string())
}

fn map_delivery_error(error: DeliveryError) -> RepoComError {
    match error {
        DeliveryError::InvalidInput { .. } | DeliveryError::StaleInput { .. } => {
            RepoComError::usage("delivery inputs are invalid or no longer current")
        }
        DeliveryError::NotEligible { blocker } => map_eligibility_blocker(blocker),
        DeliveryError::InvalidTransition { .. }
        | DeliveryError::RemoteMessageConflict
        | DeliveryError::AuditConflict => {
            RepoComError::remote_conflict("delivery state or evidence conflicts with the request")
        }
        DeliveryError::AttemptNotFound => {
            RepoComError::usage("the requested delivery attempt was not found")
        }
        DeliveryError::CorruptAttempt => {
            RepoComError::storage_integrity("the stored delivery attempt is inconsistent")
        }
        DeliveryError::State(error) => error.to_repo_com_error(),
        DeliveryError::Audit(_) => {
            RepoComError::storage_integrity("delivery audit evidence could not be committed")
        }
        DeliveryError::Serialization => {
            RepoComError::internal_failure("delivery evidence could not be serialized")
        }
    }
}

fn map_eligibility_blocker(blocker: EligibilityBlocker) -> RepoComError {
    match blocker {
        EligibilityBlocker::OperatorActionRequired => RepoComError::operator_action_required(
            "current exact approval or policy authority is required",
        ),
        EligibilityBlocker::AuthorityMissing
        | EligibilityBlocker::StalePolicyActivation
        | EligibilityBlocker::PolicyAmbiguous
        | EligibilityBlocker::PolicyNotActive
        | EligibilityBlocker::UnresolvedSecretFinding => {
            RepoComError::policy_blocked("the exact send eligibility gate is blocked")
        }
        EligibilityBlocker::DraftExpired
        | EligibilityBlocker::RevisionChanged
        | EligibilityBlocker::ConfigChanged
        | EligibilityBlocker::DestinationChanged
        | EligibilityBlocker::SecretScanChanged
        | EligibilityBlocker::PolicyBasisChanged
        | EligibilityBlocker::ApprovalStateChanged
        | EligibilityBlocker::ApprovalExpired
        | EligibilityBlocker::RepositoryScopeChanged
        | EligibilityBlocker::CurrentStateInvalid => {
            RepoComError::usage("the exact send inputs are no longer current")
        }
    }
}

fn classify_transport(
    error: &repo_com_discord_message::MessageError,
) -> repo_com_delivery_retry::TransportOutcome {
    repo_com_delivery_retry::TransportOutcome::from_message_error(error)
}

fn with_optional_expiry(request: DraftRequest, expires_in_seconds: Option<u64>) -> DraftRequest {
    expires_in_seconds.map_or(request.clone(), |seconds| {
        request.with_expiry_seconds(seconds)
    })
}

const REVISION_METADATA_REVISIONS_KEY: &str = "_repo_com_revisions";
const REVISION_METADATA_SEVERITY_KEY: &str = "_repo_com_severity";
const REVISION_METADATA_EVENT_TYPE_KEY: &str = "_repo_com_event_type";
const REVISION_METADATA_REPLY_REFERENCE_KEY: &str = "_repo_com_reply_reference";

struct PersistedRevisionMetadata {
    metadata: DraftMetadata,
    event_type: String,
    severity: String,
    reply_reference: Option<AuthorizedReplyReference>,
}

fn encode_revision_metadata(model: &DraftModel) -> Result<String, RepoComError> {
    let revisions = model
        .revisions()
        .iter()
        .map(|revision| {
            let metadata = serde_json::to_value(revision.metadata()).map_err(|_| {
                RepoComError::internal_failure("draft metadata could not be serialized")
            })?;
            let reply_reference = revision
                .reply_reference()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|_| {
                    RepoComError::internal_failure("reply reference could not be serialized")
                })?;
            Ok::<_, RepoComError>(json!({
                "revision": revision.number(),
                "metadata": metadata,
                "event_type": revision.event_type().as_str(),
                "severity": revision.severity().as_str(),
                "reply_reference": reply_reference,
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_string(&json!({ REVISION_METADATA_REVISIONS_KEY: revisions }))
        .map_err(|_| RepoComError::internal_failure("draft metadata could not be serialized"))
}

fn decode_revision_metadata(
    raw: &str,
    revision: u64,
) -> Result<PersistedRevisionMetadata, RepoComError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| RepoComError::storage_integrity("stored draft metadata is invalid"))?;
    if let Some(revisions) = value
        .get(REVISION_METADATA_REVISIONS_KEY)
        .and_then(Value::as_array)
    {
        let entry = revisions
            .iter()
            .find(|entry| entry.get("revision").and_then(Value::as_u64) == Some(revision))
            .ok_or_else(|| {
                RepoComError::storage_integrity("stored draft revision metadata is missing")
            })?;
        return decode_revision_entry(entry);
    }

    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| RepoComError::storage_integrity("stored draft metadata is not an object"))?;
    let severity = object
        .remove(REVISION_METADATA_SEVERITY_KEY)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| RepoComError::storage_integrity("stored draft severity is missing"))?;
    let event_type = object
        .remove(REVISION_METADATA_EVENT_TYPE_KEY)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| RepoComError::storage_integrity("stored draft event type is missing"))?;
    let reply_reference = object
        .remove(REVISION_METADATA_REPLY_REFERENCE_KEY)
        .filter(|value| !value.is_null())
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| RepoComError::storage_integrity("stored draft reply reference is invalid"))?;
    let metadata = serde_json::from_value(Value::Object(object))
        .map_err(|_| RepoComError::storage_integrity("stored draft metadata is invalid"))?;
    Ok(PersistedRevisionMetadata {
        metadata,
        event_type,
        severity,
        reply_reference,
    })
}

fn decode_revision_entry(entry: &Value) -> Result<PersistedRevisionMetadata, RepoComError> {
    let object = entry.as_object().ok_or_else(|| {
        RepoComError::storage_integrity("stored draft revision metadata is invalid")
    })?;
    let metadata = object
        .get("metadata")
        .cloned()
        .ok_or_else(|| RepoComError::storage_integrity("stored draft metadata is missing"))
        .and_then(|value| {
            serde_json::from_value(value)
                .map_err(|_| RepoComError::storage_integrity("stored draft metadata is invalid"))
        })?;
    let event_type = object
        .get("event_type")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| RepoComError::storage_integrity("stored draft event type is missing"))?;
    let severity = object
        .get("severity")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| RepoComError::storage_integrity("stored draft severity is missing"))?;
    let reply_reference = object
        .get("reply_reference")
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| RepoComError::storage_integrity("stored draft reply reference is invalid"))?;
    Ok(PersistedRevisionMetadata {
        metadata,
        event_type,
        severity,
        reply_reference,
    })
}

fn raw_body_from_rendered(body: &str, content_hash: &str) -> Result<String, RepoComError> {
    let footer = DeliveryNonce::new(content_hash)
        .map_err(|_| RepoComError::storage_integrity("stored draft content hash is invalid"))?
        .footer();
    Ok(body.strip_suffix(&footer).unwrap_or(body).to_owned())
}

fn parse_timestamp_seconds(value: &str) -> Result<u64, RepoComError> {
    let millis = rfc3339_to_unix_millis(value)
        .map_err(|_| RepoComError::storage_integrity("stored draft timestamp is invalid"))?;
    u64::try_from(millis / 1_000)
        .map_err(|_| RepoComError::storage_integrity("stored draft timestamp is out of range"))
}

fn parse_expiry_seconds(value: &str) -> Result<u64, RepoComError> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Ok(seconds);
    }
    let millis = rfc3339_to_unix_millis(value)
        .map_err(|_| RepoComError::storage_integrity("stored draft expiry is invalid"))?;
    u64::try_from(millis / 1_000)
        .map_err(|_| RepoComError::storage_integrity("stored draft expiry is out of range"))
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn format_rfc3339_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468;
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
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn machine_error_json(error: &RepoComError) -> Result<String, RepoComError> {
    CommandOutcome::<Value>::failure(error.clone())
        .to_json()
        .map_err(|_| RepoComError::internal_failure("error outcome could not be serialized"))
}

fn emit_error(
    error: RepoComError,
    route: Route,
    machine: bool,
    diagnostics: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let exit_code = error.exit_code();
    let diagnostic =
        diagnostics.then(|| format!("repo-com diagnostics: command={}", route.canonical()));
    if machine {
        let json = machine_error_json(&error).unwrap_or_else(|_| {
            "{\"protocol_version\":1,\"status\":\"error\",\"data\":null,\"error\":{\"code\":\"internal-failure\",\"message\":\"serialization failed\"}}".to_owned()
        });
        let _ = writeln!(stdout, "{json}");
        if let Some(diagnostic) = diagnostic {
            let _ = writeln!(stderr, "{diagnostic}");
        }
    } else {
        let text = if route.is_operations() {
            let options = OperationsRenderOptions::plain_text();
            OperationsRenderer::new(options).render(&OperationsView::Error(
                LocalErrorView::from_repo_com_error(&error),
            ))
        } else {
            let options = OutboundRenderOptions::plain_text();
            OutboundRenderer::new(options).render(&OutboundView::Error(
                repo_com_terminal_outbound::ErrorView::from(error),
            ))
        };
        let _ = write!(stdout, "{text}");
        if let Some(diagnostic) = diagnostic {
            let _ = writeln!(stderr, "{diagnostic}");
        }
    }
    exit_code
}

fn write_text(stdout: &mut dyn Write, text: &str, stderr: &mut dyn Write, code: i32) -> i32 {
    if write!(stdout, "{text}").is_err() {
        let _ = writeln!(stderr, "repo-com: stdout could not be written");
        return 1;
    }
    code
}
