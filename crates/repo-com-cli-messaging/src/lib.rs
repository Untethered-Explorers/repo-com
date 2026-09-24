#![forbid(unsafe_code)]
#![doc = "Thin, validated messaging command handlers for repo-com."]

pub mod handlers;
pub mod input;

#[cfg(test)]
#[path = "../tests/cli_messaging_contract.rs"]
mod cli_messaging_contract;

use repo_com_config::ResolvedConfig;
use repo_com_foundation::{
    ColorChoice, CommandOutcome, DiagnosticsChoice, GlobalArgs, OutputStreams, RepoComError,
    TtyMode,
};
use repo_com_terminal_outbound::{
    ApprovalStatusView, DeliveryView, OutboundPreview, OutboundRenderer, OutboundView,
    RenderOptions, SecretFindingStatusView, TerminalWidth, block_lines, field_lines, lines_fit,
};
use serde::{Deserialize, Serialize};

pub use handlers::draft::{
    CreateDraft, DraftIdentity, DraftService, DraftView, KeyboardOutboundUi, OutboundUi,
    UpdateDraft, approve, create, override_secret_finding, preview, show, update,
};
pub use handlers::inbox::{
    FetchCommitView, FetchContinuationView, InboundAction, InboundActionRequest,
    InboundActionResult, InboundTrust, InboxFetchCommand, InboxFetchResult, InboxFetchService,
    InboxLifecycleService, ProbeContinuationView, apply, fetch,
};
pub use handlers::reply::{
    ReplyCreateRequest, ReplyDraftView, ReplyService, create as create_reply,
};
pub use handlers::send::{
    SendRequest, SendService, SetupCheckView, SetupService, send, setup_check,
};
pub use input::*;

/// Result type used by all messaging ports and handlers.
pub type MessagingResult<T> = Result<T, RepoComError>;

/// Explicit process context passed from the final executable to handlers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandlerContext {
    /// Whether stdin and stdout are both interactive.
    pub tty_mode: TtyMode,
    /// Injected current Unix time for domain decisions.
    pub now_unix_seconds: u64,
    /// Explicit color choice.
    pub color: ColorChoice,
    /// Explicit diagnostics choice.
    pub diagnostics: DiagnosticsChoice,
}

impl HandlerContext {
    /// Creates a context with the safe presentation defaults.
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

    /// Returns the renderer options for an explicit terminal width.
    #[must_use]
    pub const fn render_options(self, width: usize) -> RenderOptions {
        RenderOptions {
            color: self.color,
            width: TerminalWidth::new(width),
            tty_mode: self.tty_mode,
        }
    }
}

/// A complete typed command result suitable for a protocol-version-1 data
/// field or a human presentation adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "kebab-case")]
pub enum MessagingOutput {
    /// Draft create/show/update result.
    Draft(DraftView),
    /// Exact outbound preview.
    Preview(OutboundPreview),
    /// Exact approval result.
    Approval(ApprovalStatusView),
    /// Exact redacted safety override result.
    SecretOverride(SecretFindingStatusView),
    /// Delivery result from the delivery owner.
    Delivery(DeliveryView),
    /// Read-only setup result.
    Setup(SetupCheckView),
    /// Bounded inbound fetch result.
    Fetch(InboxFetchResult),
    /// Local inbound lifecycle result.
    Lifecycle(InboundActionResult),
    /// Reply-draft creation result.
    #[serde(rename = "reply-draft")]
    Reply(ReplyDraftView),
}

impl MessagingOutput {
    /// Returns a stable view label.
    #[must_use]
    pub const fn view_name(&self) -> &'static str {
        match self {
            Self::Draft(_) => "draft",
            Self::Preview(_) => "preview",
            Self::Approval(_) => "approval",
            Self::SecretOverride(_) => "secret-override",
            Self::Delivery(_) => "delivery",
            Self::Setup(_) => "setup",
            Self::Fetch(_) => "fetch",
            Self::Lifecycle(_) => "lifecycle",
            Self::Reply(_) => "reply-draft",
        }
    }

    /// Renders labeled human output without contacting a domain service.
    #[must_use]
    pub fn render_human(&self, options: RenderOptions) -> String {
        match self {
            Self::Draft(view) => OutboundRenderer::new(options)
                .render(&OutboundView::Preview(view.to_outbound_preview())),
            Self::Preview(view) => {
                OutboundRenderer::new(options).render(&OutboundView::Preview(view.clone()))
            }
            Self::Approval(view) => {
                OutboundRenderer::new(options).render(&OutboundView::Approval(view.clone()))
            }
            Self::SecretOverride(view) => {
                OutboundRenderer::new(options).render(&OutboundView::SecretFinding(view.clone()))
            }
            Self::Delivery(view) => {
                OutboundRenderer::new(options).render(&OutboundView::Delivery(view.clone()))
            }
            Self::Setup(report) => render_setup(report, options),
            Self::Fetch(result) => render_fetch(result, options),
            Self::Lifecycle(result) => render_lifecycle(result, options),
            Self::Reply(result) => render_reply(result, options),
        }
    }

    /// Serializes this output as a protocol-version-1 success envelope.
    pub fn to_protocol_json(&self) -> Result<String, serde_json::Error> {
        CommandOutcome::success(self.clone()).to_json()
    }
}

/// Aggregate domain port used by the thin dispatcher.
///
/// A final executable can implement the focused sub-port traits by composing
/// the existing state, approval, eligibility, delivery, Discord, inbound, and
/// reply services.  The handler layer never receives network clients directly.
pub trait MessagingService:
    DraftService
    + SendService
    + SetupService
    + InboxFetchService
    + InboxLifecycleService
    + ReplyService
    + Send
{
}

impl<T> MessagingService for T where
    T: DraftService
        + SendService
        + SetupService
        + InboxFetchService
        + InboxLifecycleService
        + ReplyService
        + Send
{
}

/// Dispatches one parsed command and always returns a protocol outcome.
///
/// All operational failures are converted to `RepoComError` before the
/// protocol envelope is constructed, so a caller can write one JSON object
/// even when a domain service rejects a request.
pub async fn dispatch<S, U>(
    service: &mut S,
    ui: &mut U,
    context: HandlerContext,
    config: &ResolvedConfig,
    input: MessagingInput,
) -> CommandOutcome<MessagingOutput>
where
    S: MessagingService + ?Sized,
    U: OutboundUi + ?Sized,
{
    let result = dispatch_result(service, ui, context, config, input).await;
    CommandOutcome::from_result(result)
}

/// Parses and dispatches bytes, preserving one protocol object on every
/// malformed-input or domain failure.
pub async fn dispatch_json<S, U>(
    service: &mut S,
    ui: &mut U,
    context: HandlerContext,
    config: &ResolvedConfig,
    bytes: &[u8],
) -> CommandOutcome<MessagingOutput>
where
    S: MessagingService + ?Sized,
    U: OutboundUi + ?Sized,
{
    match input::parse(bytes) {
        Ok(parsed) => dispatch(service, ui, context, config, parsed).await,
        Err(error) => CommandOutcome::failure(error),
    }
}

async fn dispatch_result<S, U>(
    service: &mut S,
    ui: &mut U,
    context: HandlerContext,
    config: &ResolvedConfig,
    input: MessagingInput,
) -> MessagingResult<MessagingOutput>
where
    S: MessagingService + ?Sized,
    U: OutboundUi + ?Sized,
{
    match input {
        MessagingInput::DraftCreate(value) => {
            let output = create(service, CreateDraft::try_from(value)?).await?;
            Ok(MessagingOutput::Draft(output))
        }
        MessagingInput::DraftShow(value) => {
            let output = show(service, DraftIdentity::from(value)).await?;
            Ok(MessagingOutput::Draft(output))
        }
        MessagingInput::DraftUpdate(value) => {
            let output = update(service, UpdateDraft::try_from(value)?).await?;
            Ok(MessagingOutput::Draft(output))
        }
        MessagingInput::DraftPreview(value) => {
            let output = preview(
                service,
                DraftIdentity::from(value),
                context.now_unix_seconds,
            )
            .await?;
            Ok(MessagingOutput::Preview(output))
        }
        MessagingInput::DraftApprove(value) => {
            let output = approve(
                service,
                ui,
                DraftIdentity::from(value),
                context.tty_mode,
                context.now_unix_seconds,
            )
            .await?;
            Ok(MessagingOutput::Approval(output))
        }
        MessagingInput::DraftSecretOverride(value) => {
            let output = override_secret_finding(
                service,
                ui,
                DraftIdentity::from(value),
                context.tty_mode,
                context.now_unix_seconds,
            )
            .await?;
            Ok(MessagingOutput::SecretOverride(output))
        }
        MessagingInput::Send(value) => {
            let request =
                SendRequest::from_input(value, context.tty_mode, context.now_unix_seconds);
            let output = send(service, request, config).await?;
            Ok(MessagingOutput::Delivery(output))
        }
        MessagingInput::SetupCheck(value) => {
            let output = setup_check(service, &value, config).await?;
            Ok(MessagingOutput::Setup(output))
        }
        MessagingInput::InboxFetch(value) => {
            let request = InboxFetchCommand::from(value);
            let output = fetch(service, request, config).await?;
            Ok(MessagingOutput::Fetch(output))
        }
        MessagingInput::InboxAcknowledge(value) => {
            let output = apply(service, InboundActionRequest::acknowledge(value)).await?;
            Ok(MessagingOutput::Lifecycle(output))
        }
        MessagingInput::InboxArchive(value) => {
            let output = apply(service, InboundActionRequest::archive(value)).await?;
            Ok(MessagingOutput::Lifecycle(output))
        }
        MessagingInput::ReplyDraftCreate(value) => {
            let request = ReplyCreateRequest::try_from(value)?;
            let output = create_reply(service, request, config).await?;
            Ok(MessagingOutput::Reply(output))
        }
    }
}

/// Returns separated machine output and optional diagnostics.
pub fn machine_output_streams(
    outcome: &CommandOutcome<MessagingOutput>,
    diagnostics: Option<String>,
) -> Result<OutputStreams, RepoComError> {
    outcome
        .output_streams(diagnostics)
        .map_err(|_| RepoComError::internal_failure("messaging outcome could not be serialized"))
}

/// Renders a successful or failed human outcome while keeping protocol output
/// available separately through [`machine_output_streams`].
#[must_use]
pub fn render_human_outcome(
    outcome: &CommandOutcome<MessagingOutput>,
    options: RenderOptions,
) -> String {
    match outcome.status() {
        repo_com_foundation::OutcomeStatus::Success => outcome
            .data()
            .map_or_else(String::new, |data| data.render_human(options)),
        repo_com_foundation::OutcomeStatus::Error => {
            let mut lines = vec!["Messaging error".to_owned()];
            if let Some(error) = outcome.error() {
                lines.extend(field_lines(
                    "Error category",
                    error.code.code(),
                    options.columns(),
                ));
                lines.extend(block_lines(
                    "Error detail",
                    &error.message,
                    options.columns(),
                ));
            }
            lines.extend(field_lines(
                "Next action",
                "correct the command and retry safely",
                options.columns(),
            ));
            format!("{}\n", lines.join("\n"))
        }
    }
}

/// Validates a rendered human result for the minimum terminal contract.
#[must_use]
pub fn human_output_fits(value: &str, width: usize) -> bool {
    lines_fit(value, TerminalWidth::new(width).columns())
}

fn render_setup(view: &SetupCheckView, options: RenderOptions) -> String {
    let report = &view.report;
    let mut lines = vec!["Discord setup check".to_owned()];
    lines.extend(field_lines(
        "Repository",
        &view.repository_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Configuration hash",
        &view.config_hash,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Repository workspace",
        &report.workspace_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Bot user",
        &report.bot.user_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Dedicated bot",
        if report.bot.dedicated_bot {
            "yes"
        } else {
            "no"
        },
        options.columns(),
    ));
    lines.extend(field_lines(
        "Workspace membership",
        match report.workspace_membership {
            repo_com_discord_client::WorkspaceMembership::Member => "member",
            repo_com_discord_client::WorkspaceMembership::Missing => "missing",
            repo_com_discord_client::WorkspaceMembership::Unchecked => "unchecked",
        },
        options.columns(),
    ));
    lines.extend(field_lines(
        "Ready",
        if report.ready() { "yes" } else { "no" },
        options.columns(),
    ));
    for (index, channel) in report.channels.iter().enumerate() {
        let label = format!("Channel check {}", index + 1);
        lines.extend(field_lines(
            &format!("{label} destinations"),
            &channel.destination_aliases.join(", "),
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} inbound aliases"),
            &channel.inbound_aliases.join(", "),
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} required permissions"),
            &format!("{:?}", channel.required_permissions),
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} missing permissions"),
            &format!("{:?}", channel.missing_permissions),
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} ready"),
            if channel.ready { "yes" } else { "no" },
            options.columns(),
        ));
    }
    for (index, mention) in report.mentions.iter().enumerate() {
        let label = format!("Mention check {}", index + 1);
        lines.extend(field_lines(
            &format!("{label} destination"),
            &mention.destination_alias,
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} alias"),
            &mention.mention_alias,
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} target kind"),
            &format!("{:?}", mention.target_kind),
            options.columns(),
        ));
        lines.extend(field_lines(
            &format!("{label} ready"),
            if mention.ready { "yes" } else { "no" },
            options.columns(),
        ));
    }
    lines.push(format!("Issues: {}", report.issues.len()));
    for (index, issue) in report.issues.iter().enumerate() {
        let label = format!("Issue {}", index + 1);
        lines.extend(field_lines(
            &label,
            &format!("{:?}", issue.kind),
            options.columns(),
        ));
        lines.extend(block_lines(
            &format!("{label} remediation"),
            &issue.remediation.instruction,
            options.columns(),
        ));
    }
    lines.push("Next action: review the labeled remediation; setup is read-only.".to_owned());
    format!("{}\n", lines.join("\n"))
}

fn render_fetch(result: &InboxFetchResult, options: RenderOptions) -> String {
    let mut lines = vec!["Inbound fetch".to_owned()];
    lines.extend(field_lines(
        "Repository",
        &result.repository_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Inbound alias",
        &result.alias,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Resolved channel",
        &result.channel_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Trust",
        "untrusted remote data",
        options.columns(),
    ));
    lines.extend(field_lines(
        "Items retained",
        &result.items.len().to_string(),
        options.columns(),
    ));
    lines.extend(field_lines(
        "Raw messages",
        &result.raw_messages.to_string(),
        options.columns(),
    ));
    lines.extend(field_lines(
        "Pages fetched",
        &result.pages_fetched.to_string(),
        options.columns(),
    ));
    lines.extend(field_lines(
        "Authoritative cursor",
        &result.authoritative_cursor,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Continuation",
        &result.continuation.reason,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Local commit",
        &format!(
            "{} item(s), {} transition(s)",
            result.commit.stored_items, result.commit.stored_transitions
        ),
        options.columns(),
    ));
    lines
        .push("Remote mutation: none; fetch performs bounded reads and a local commit.".to_owned());
    format!("{}\n", lines.join("\n"))
}

fn render_lifecycle(result: &InboundActionResult, options: RenderOptions) -> String {
    let mut lines = vec!["Inbound local action".to_owned()];
    lines.extend(field_lines(
        "Repository",
        &result.repository_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Action",
        match result.action {
            InboundAction::Acknowledge => "acknowledge",
            InboundAction::Archive => "archive",
        },
        options.columns(),
    ));
    lines.extend(field_lines(
        "Item count",
        &result.item_ids.len().to_string(),
        options.columns(),
    ));
    lines.extend(field_lines("Recorded at", &result.at, options.columns()));
    lines.push("Remote mutation: none; the local marker is idempotent.".to_owned());
    format!("{}\n", lines.join("\n"))
}

fn render_reply(result: &ReplyDraftView, options: RenderOptions) -> String {
    let mut lines = vec!["Reply draft created".to_owned()];
    lines.extend(field_lines(
        "Repository",
        &result.repository_id,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Inbound item",
        &result.inbound_item_id,
        options.columns(),
    ));
    lines.extend(field_lines("Draft", &result.draft_id, options.columns()));
    lines.extend(field_lines(
        "Revision",
        &result.revision.to_string(),
        options.columns(),
    ));
    lines.extend(field_lines(
        "Revision hash",
        &result.revision_hash,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Destination",
        &result.destination_alias,
        options.columns(),
    ));
    lines.extend(field_lines(
        "Destination channel",
        &result.resolved_destination.channel_id,
        options.columns(),
    ));
    lines.push("Outcome: draft-only; no Discord message was sent.".to_owned());
    lines.push(
        "Next action: review, approve, and send through the normal draft lifecycle.".to_owned(),
    );
    format!("{}\n", lines.join("\n"))
}
