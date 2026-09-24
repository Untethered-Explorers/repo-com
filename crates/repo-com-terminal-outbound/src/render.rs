//! Pure outbound terminal rendering.
//!
//! Rendering is intentionally separate from command execution.  Every view is
//! linear, labeled, and complete at the effective terminal width.  Color is an
//! optional supplement only; all state, approval, safety, destination, and
//! action meaning remains in text labels.

use std::env;

use repo_com_foundation::{ColorChoice, CommandOutcome, TtyMode};
use serde_json::Error as JsonError;

use crate::preview::{
    ApprovalState, ApprovalView, DeliveryOutcome, DeliveryView, DestinationView, ErrorView,
    MetadataView, OutboundPreview, PolicyState, PolicyView, Provenance, SafetyState, SafetyView,
};
use crate::width::{
    MIN_TERMINAL_WIDTH, TerminalWidth, block_lines, display_width, field_lines, strip_ansi,
};

/// Presentation options for human output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderOptions {
    /// Explicit color request.
    pub color: ColorChoice,
    /// Effective output width, never below 80 columns.
    pub width: TerminalWidth,
    /// Explicit stream mode supplied by the command layer.
    pub tty_mode: TtyMode,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::plain_text()
    }
}

impl RenderOptions {
    /// Creates the safe plain-text 80-column default.
    #[must_use]
    pub const fn plain_text() -> Self {
        Self {
            color: ColorChoice::Never,
            width: TerminalWidth::eighty(),
            tty_mode: TtyMode::NonTty,
        }
    }

    /// Creates a non-color 80-column option.
    #[must_use]
    pub const fn no_color() -> Self {
        Self::plain_text()
    }

    /// Creates options with a caller-selected width, clamped to the contract
    /// minimum of 80 columns.
    #[must_use]
    pub const fn with_width(width: usize) -> Self {
        Self {
            color: ColorChoice::Never,
            width: TerminalWidth::new(width),
            tty_mode: TtyMode::NonTty,
        }
    }

    /// Returns options with an explicit color request and TTY mode.
    #[must_use]
    pub const fn with_color(color: ColorChoice, tty_mode: TtyMode) -> Self {
        Self {
            color,
            width: TerminalWidth::eighty(),
            tty_mode,
        }
    }

    /// Resolves color without reading the process environment.  Supplying
    /// `no_color_present` models the presence-only `NO_COLOR` convention and
    /// keeps this decision deterministic for callers and tests.
    #[must_use]
    pub const fn resolve_color(
        color: ColorChoice,
        tty_mode: TtyMode,
        no_color_present: bool,
    ) -> ColorChoice {
        if no_color_present || tty_mode.is_non_tty() {
            ColorChoice::Never
        } else {
            color
        }
    }

    /// Resolves an explicit color request while honoring the `NO_COLOR`
    /// environment convention.  A non-TTY stream is also rendered without ANSI.
    /// The environment is read only by this explicit constructor; ordinary
    /// renderers remain deterministic and environment-independent.
    #[must_use]
    pub fn from_environment(color: ColorChoice, tty_mode: TtyMode) -> Self {
        let color = Self::resolve_color(color, tty_mode, env::var_os("NO_COLOR").is_some());
        Self {
            color,
            width: TerminalWidth::eighty(),
            tty_mode,
        }
    }

    /// Returns a copy with a different width.
    #[must_use]
    pub const fn width(self, width: usize) -> Self {
        Self {
            width: TerminalWidth::new(width),
            ..self
        }
    }

    /// Returns whether ANSI styling is permitted for this invocation.
    #[must_use]
    pub const fn ansi_enabled(self) -> bool {
        matches!(self.color, ColorChoice::Always) && self.tty_mode.is_tty()
    }

    /// Returns the effective width.
    #[must_use]
    pub const fn columns(self) -> usize {
        self.width.columns()
    }
}

/// A reusable, stateless outbound renderer configuration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutboundRenderer {
    options: RenderOptions,
}

impl OutboundRenderer {
    /// Creates a renderer with explicit output options.
    #[must_use]
    pub const fn new(options: RenderOptions) -> Self {
        Self { options }
    }

    /// Creates the plain-text 80-column renderer.
    #[must_use]
    pub const fn plain_text() -> Self {
        Self::new(RenderOptions::plain_text())
    }

    /// Renders one view without performing I/O.
    #[must_use]
    pub fn render(&self, view: &OutboundView) -> String {
        render_view(view, self.options)
    }

    /// Serializes one view as a protocol-version-1 success envelope.
    pub fn render_machine(&self, view: &OutboundView) -> Result<String, JsonError> {
        render_machine(view)
    }
}

/// A policy status view with enough identity context for a linear read.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct PolicyStatusView {
    /// Repository identity.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Resolved destination.
    pub destination: DestinationView,
    /// Policy basis.
    pub policy: PolicyView,
    /// Approval basis, when supplied.
    pub approval: ApprovalView,
    /// Safety basis, when supplied.
    pub safety: SafetyView,
    /// Provenance label.
    pub provenance: Provenance,
}

impl PolicyStatusView {
    /// Creates a policy status view with no approval or safety decision.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        destination: DestinationView,
        policy: PolicyView,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            destination,
            policy,
            approval: ApprovalView::default(),
            safety: SafetyView::default(),
            provenance: Provenance::LocalDecision,
        }
    }
}

/// An approval status view with exact hash bindings.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct ApprovalStatusView {
    /// Repository identity.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Resolved destination.
    pub destination: DestinationView,
    /// Approval basis.
    pub approval: ApprovalView,
    /// Policy basis, when supplied.
    pub policy: PolicyView,
    /// Safety basis, when supplied.
    pub safety: SafetyView,
    /// Hash of the complete exact preview.
    pub preview_hash: Option<String>,
    /// Provenance label.
    pub provenance: Provenance,
}

impl ApprovalStatusView {
    /// Creates an approval status view.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        destination: DestinationView,
        approval: ApprovalView,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            destination,
            approval,
            policy: PolicyView::default(),
            safety: SafetyView::default(),
            preview_hash: None,
            provenance: Provenance::LocalDecision,
        }
    }
}

/// A redacted secret finding and override view.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct SecretFindingStatusView {
    /// Repository identity.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Resolved destination.
    pub destination: DestinationView,
    /// Safety and override basis.
    pub safety: SafetyView,
    /// Approval basis, when supplied.
    pub approval: ApprovalView,
    /// Policy basis, when supplied.
    pub policy: PolicyView,
    /// Hash of the complete exact preview.
    pub preview_hash: Option<String>,
    /// Hash of exact outbound text, when supplied.
    pub exact_text_hash: Option<String>,
    /// Exclusive expiry boundary.
    pub expires_at_unix_seconds: u64,
    /// Provenance label.
    pub provenance: Provenance,
}

impl SecretFindingStatusView {
    /// Creates a secret finding view.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        destination: DestinationView,
        safety: SafetyView,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            destination,
            safety,
            approval: ApprovalView::default(),
            policy: PolicyView::default(),
            preview_hash: None,
            exact_text_hash: None,
            expires_at_unix_seconds: 0,
            provenance: Provenance::LocalDecision,
        }
    }
}

/// Every outbound presentation view owned by this crate.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "view", rename_all = "kebab-case")]
pub enum OutboundView {
    /// Exact draft preview.
    Preview(OutboundPreview),
    /// Exact policy status.
    Policy(PolicyStatusView),
    /// Exact approval status.
    Approval(ApprovalStatusView),
    /// Redacted safety finding and override status.
    SecretFinding(SecretFindingStatusView),
    /// Delivery, retry, reconciliation, or eligibility result.
    Delivery(DeliveryView),
    /// A local operational or integrity error.
    Error(ErrorView),
}

impl OutboundView {
    /// Creates a preview view.
    #[must_use]
    pub fn preview(preview: OutboundPreview) -> Self {
        Self::Preview(preview)
    }

    /// Creates a policy status view.
    #[must_use]
    pub fn policy(view: PolicyStatusView) -> Self {
        Self::Policy(view)
    }

    /// Creates an approval status view.
    #[must_use]
    pub fn approval(view: ApprovalStatusView) -> Self {
        Self::Approval(view)
    }

    /// Creates a secret finding view.
    #[must_use]
    pub fn secret_finding(view: SecretFindingStatusView) -> Self {
        Self::SecretFinding(view)
    }

    /// Creates a delivery view.
    #[must_use]
    pub fn delivery(view: DeliveryView) -> Self {
        Self::Delivery(view)
    }

    /// Creates an error view.
    #[must_use]
    pub fn error(view: ErrorView) -> Self {
        Self::Error(view)
    }

    /// Returns the stable view name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Preview(_) => "preview",
            Self::Policy(_) => "policy",
            Self::Approval(_) => "approval",
            Self::SecretFinding(_) => "secret-finding",
            Self::Delivery(_) => "delivery",
            Self::Error(_) => "error",
        }
    }

    /// Renders this view as complete labeled human text.
    #[must_use]
    pub fn render(&self, options: RenderOptions) -> String {
        render_view(self, options)
    }

    /// Serializes the typed view as one deterministic JSON value.
    ///
    /// The command layer remains responsible for wrapping this value in the
    /// repository's protocol-version-1 command envelope and for keeping
    /// diagnostics off machine stdout.
    pub fn to_json(&self) -> Result<String, JsonError> {
        serde_json::to_string(self)
    }

    /// Serializes this view inside one protocol-version-1 success envelope.
    pub fn to_protocol_json(&self) -> Result<String, JsonError> {
        match self {
            Self::Error(error) => render_machine_failure(error),
            _ => CommandOutcome::success(self.clone()).to_json(),
        }
    }
}

/// Renders one outbound view with the supplied options.
#[must_use]
pub fn render_view(view: &OutboundView, options: RenderOptions) -> String {
    let mut output = Output::new(options);
    match view {
        OutboundView::Preview(preview) => render_preview_into(preview, &mut output),
        OutboundView::Policy(view) => render_policy_into(view, &mut output),
        OutboundView::Approval(view) => render_approval_into(view, &mut output),
        OutboundView::SecretFinding(view) => render_secret_finding_into(view, &mut output),
        OutboundView::Delivery(view) => render_delivery_into(view, &mut output),
        OutboundView::Error(view) => render_error_into(view, &mut output),
    }
    output.finish()
}

/// Serializes a successful outbound view as exactly one protocol-version-1
/// command envelope.  The command handler remains responsible for choosing
/// human versus machine mode and for writing the returned string to stdout.
pub fn render_machine(view: &OutboundView) -> Result<String, JsonError> {
    match view {
        OutboundView::Error(error) => render_machine_failure(error),
        _ => view.to_protocol_json(),
    }
}

/// Serializes a redacted local error as exactly one protocol-version-1 failure
/// envelope with a stable foundation category.
pub fn render_machine_failure(error: &ErrorView) -> Result<String, JsonError> {
    CommandOutcome::<OutboundView>::failure(error.to_repo_com_error()).to_json()
}

/// Renders a preview view directly.
#[must_use]
pub fn render_preview(preview: &OutboundPreview, options: RenderOptions) -> String {
    render_view(&OutboundView::Preview(preview.clone()), options)
}

/// Renders a policy status view directly.
#[must_use]
pub fn render_policy(view: &PolicyStatusView, options: RenderOptions) -> String {
    render_view(&OutboundView::Policy(view.clone()), options)
}

/// Renders an approval status view directly.
#[must_use]
pub fn render_approval(view: &ApprovalStatusView, options: RenderOptions) -> String {
    render_view(&OutboundView::Approval(view.clone()), options)
}

/// Renders a redacted secret finding view directly.
#[must_use]
pub fn render_secret_finding(view: &SecretFindingStatusView, options: RenderOptions) -> String {
    render_view(&OutboundView::SecretFinding(view.clone()), options)
}

/// Renders a delivery view directly.
#[must_use]
pub fn render_delivery(view: &DeliveryView, options: RenderOptions) -> String {
    render_view(&OutboundView::Delivery(view.clone()), options)
}

/// Renders a local error view directly.
#[must_use]
pub fn render_error(view: &ErrorView, options: RenderOptions) -> String {
    render_view(&OutboundView::Error(view.clone()), options)
}

/// Returns the number of columns in the longest visible output line.
#[must_use]
pub fn rendered_width(value: &str) -> usize {
    value.lines().map(display_width).max().unwrap_or(0)
}

/// Returns whether rendered output contains no ANSI escape sequences.
#[must_use]
pub fn is_ansi_free(value: &str) -> bool {
    strip_ansi(value) == value
}

/// The internal line-oriented output builder keeps rendering pure and makes it
/// straightforward to assert the 80-column contract in tests.
struct Output {
    options: RenderOptions,
    lines: Vec<String>,
}

impl Output {
    fn new(options: RenderOptions) -> Self {
        Self {
            options,
            lines: Vec::new(),
        }
    }

    fn finish(self) -> String {
        let mut result = self.lines.join("\n");
        result.push('\n');
        result
    }

    fn push_line(&mut self, line: impl Into<String>) {
        let line = line.into();
        debug_assert!(display_width(&line) <= self.options.columns());
        self.lines.push(line);
    }

    fn push_lines(&mut self, lines: impl IntoIterator<Item = String>) {
        for line in lines {
            self.push_line(line);
        }
    }

    fn heading(&mut self, text: &str) {
        let line = if self.options.ansi_enabled() {
            format!("\u{1b}[1m{text}\u{1b}[0m")
        } else {
            text.to_owned()
        };
        self.push_line(line);
    }

    fn field(&mut self, label: &str, value: impl AsRef<str>) {
        let value = value.as_ref();
        let value = if value.is_empty() { "(none)" } else { value };
        self.push_lines(field_lines(label, value, self.options.columns()));
    }

    fn optional(&mut self, label: &str, value: Option<&str>) {
        self.field(label, value.unwrap_or("(none)"));
    }

    fn block(&mut self, label: &str, value: &str) {
        self.push_lines(block_lines(label, value, self.options.columns()));
    }

    fn provenance(&mut self, provenance: Provenance) {
        self.field("Provenance", provenance.as_str());
    }
}

fn render_preview_into(preview: &OutboundPreview, output: &mut Output) {
    output.heading("Outbound preview");
    render_identity(
        output,
        &preview.repository_id,
        &preview.draft_id,
        preview.revision,
        &preview.revision_hash,
        &preview.destination,
        preview.expires_at_unix_seconds,
    );
    output.field(
        "Created",
        format!("Unix {} seconds", preview.created_at_unix_seconds),
    );
    output.optional("Preview hash", Some(preview.preview_hash.as_str()));
    output.block("Exact text", &preview.exact_text);
    output.field("Exact text hash", &preview.exact_text_hash);
    render_metadata(output, &preview.metadata);
    output.field("Event type", &preview.event_type);
    output.field("Severity", &preview.severity);
    if let Some(reference) = &preview.reply_reference {
        output.block("Reply reference", &reply_reference_lines(reference));
    } else {
        output.optional("Reply reference", None);
    }
    render_approval_fields(output, &preview.approval);
    render_policy_fields(output, &preview.policy);
    render_safety_fields(output, &preview.safety);
    output.field("Outcome", "preview-ready");
    output.provenance(preview.provenance);
    output.field(
        "Next action",
        "Review the exact preview, then use a current approval or activated policy; no send occurred.",
    );
}

fn render_policy_into(view: &PolicyStatusView, output: &mut Output) {
    output.heading("Policy status");
    render_identity(
        output,
        &view.repository_id,
        &view.draft_id,
        view.revision,
        "(not supplied)",
        &view.destination,
        0,
    );
    render_policy_fields(output, &view.policy);
    render_approval_fields(output, &view.approval);
    render_safety_fields(output, &view.safety);
    output.field("Outcome", view.policy.state.as_str());
    output.provenance(view.provenance);
    output.field("Next action", policy_next_action(view.policy.state));
}

fn render_approval_into(view: &ApprovalStatusView, output: &mut Output) {
    output.heading("Approval status");
    render_identity(
        output,
        &view.repository_id,
        &view.draft_id,
        view.revision,
        view.approval
            .revision_hash
            .as_deref()
            .unwrap_or("(not supplied)"),
        &view.destination,
        view.approval.expires_at_unix_seconds.unwrap_or(0),
    );
    render_approval_fields(output, &view.approval);
    output.optional("Preview hash", view.preview_hash.as_deref());
    output.field(
        "Exact text hash",
        optional_text(view.approval.exact_text_hash.as_deref()),
    );
    render_policy_fields(output, &view.policy);
    render_safety_fields(output, &view.safety);
    output.field("Outcome", approval_outcome(view.approval.state));
    output.provenance(view.provenance);
    output.field("Next action", approval_next_action(view.approval.state));
}

fn render_secret_finding_into(view: &SecretFindingStatusView, output: &mut Output) {
    output.heading("Secret finding");
    render_identity(
        output,
        &view.repository_id,
        &view.draft_id,
        view.revision,
        "(not supplied)",
        &view.destination,
        view.expires_at_unix_seconds,
    );
    output.optional("Preview hash", view.preview_hash.as_deref());
    output.optional("Exact text hash", view.exact_text_hash.as_deref());
    render_approval_fields(output, &view.approval);
    render_policy_fields(output, &view.policy);
    render_safety_fields(output, &view.safety);
    output.field("Outcome", view.safety.state.as_str());
    output.provenance(view.provenance);
    output.field(
        "Next action",
        match view.safety.state {
            SafetyState::Clear => "No override is required; continue with the current approval or policy gate.",
            SafetyState::OverrideRecorded => "Keep the exact redacted override bound to this preview; do not reuse it for another revision.",
            SafetyState::Expired => "Create a new immutable revision; the expired finding cannot be overridden.",
            _ => "Review every redacted finding and, if appropriate, request an exact TTY override.",
        },
    );
}

fn render_delivery_into(view: &DeliveryView, output: &mut Output) {
    output.heading("Delivery outcome");
    render_identity(
        output,
        &view.repository_id,
        &view.draft_id,
        view.revision,
        &view.revision_hash,
        &view.destination,
        view.expires_at_unix_seconds,
    );
    output.optional("Attempt ID", view.attempt_id.as_deref());
    output.optional(
        "Attempt",
        view.attempt_number
            .map(|number| format!("attempt {number}"))
            .as_deref(),
    );
    if let Some(exact_text) = &view.exact_text {
        output.block("Exact text", exact_text);
    }
    output.optional("Exact text hash", view.exact_text_hash.as_deref());
    output.optional("Request nonce", view.request_nonce.as_deref());
    output.optional("Content nonce", view.content_nonce.as_deref());
    render_metadata(output, &view.metadata);
    render_approval_fields(output, &view.approval);
    render_policy_fields(output, &view.policy);
    render_safety_fields(output, &view.safety);
    if let Some(category) = view.outcome.error_category() {
        output.field("Stable exit category", category.code());
    }
    render_delivery_outcome(output, &view.outcome);
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_error_into(view: &ErrorView, output: &mut Output) {
    output.heading("Outbound error");
    output.optional("Repository", view.repository_id.as_deref());
    output.optional("Object", view.draft_id.as_deref());
    output.optional(
        "Revision",
        view.revision.map(|value| value.to_string()).as_deref(),
    );
    if let Some(destination) = &view.destination {
        render_destination(output, destination);
    } else {
        output.optional("Destination", None);
    }
    output.field("Error category", &view.category);
    output.block("Error detail", &view.detail);
    output.field("Outcome", "error");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_identity(
    output: &mut Output,
    repository_id: &str,
    draft_id: &str,
    revision: u64,
    revision_hash: &str,
    destination: &DestinationView,
    expires_at_unix_seconds: u64,
) {
    output.field("Repository", repository_id);
    output.field("Object", format!("draft {draft_id}"));
    output.field("Revision", revision.to_string());
    output.field("Revision hash", revision_hash);
    render_destination(output, destination);
    if expires_at_unix_seconds == 0 {
        output.optional("Expires", None);
    } else {
        output.field("Expires", format!("Unix {expires_at_unix_seconds} seconds"));
    }
}

fn render_destination(output: &mut Output, destination: &DestinationView) {
    output.field("Destination", destination.summary());
    output.field("Destination workspace", &destination.workspace_id);
    output.field("Destination channel", &destination.channel_id);
    output.optional(
        "Allowed mentions",
        (!destination.allowed_mentions.is_empty())
            .then(|| destination.allowed_mentions.join(", "))
            .as_deref(),
    );
}

fn render_metadata(output: &mut Output, metadata: &MetadataView) {
    output.heading("Metadata");
    output.optional("Repository label", metadata.repository_label.as_deref());
    output.optional("Branch", metadata.branch.as_deref());
    output.optional("Commit", metadata.commit.as_deref());
}

fn render_approval_fields(output: &mut Output, approval: &ApprovalView) {
    output.heading("Approval");
    output.field("Approval state", approval.state.as_str());
    output.optional("Approval ID", approval.approval_id.as_deref());
    output.optional("Approval hash", approval.approval_hash.as_deref());
    output.optional("Approval preview hash", approval.preview_hash.as_deref());
    output.optional("Approval revision hash", approval.revision_hash.as_deref());
    output.optional(
        "Approval exact text hash",
        approval.exact_text_hash.as_deref(),
    );
    output.optional("Approval metadata hash", approval.metadata_hash.as_deref());
    output.optional("Approval config hash", approval.config_hash.as_deref());
    output.optional(
        "Approval destination hash",
        approval.destination_hash.as_deref(),
    );
    output.optional(
        "Approval policy basis hash",
        approval.policy_basis_hash.as_deref(),
    );
    output.optional("Approval safety hash", approval.scan_hash.as_deref());
    output.optional(
        "Approval draft expiry",
        approval
            .draft_expires_at_unix_seconds
            .map(|value| format!("Unix {value} seconds"))
            .as_deref(),
    );
    output.optional(
        "Approval expiry",
        approval
            .expires_at_unix_seconds
            .map(|value| format!("Unix {value} seconds"))
            .as_deref(),
    );
    output.optional("Approval override hash", approval.override_hash.as_deref());
    output.optional("Approval actor", approval.actor_kind.as_deref());
    output.optional("Approval reason", approval.reason.as_deref());
}

fn render_policy_fields(output: &mut Output, policy: &PolicyView) {
    output.heading("Policy");
    output.field("Policy state", policy.state.as_str());
    output.optional(
        "Policy tuple",
        policy.tuple.as_ref().map(ToString::to_string).as_deref(),
    );
    output.optional("Policy config hash", policy.config_hash.as_deref());
    output.optional("Policy tuple hash", policy.tuple_hash.as_deref());
    output.optional("Policy activation ID", policy.activation_id.as_deref());
    output.optional(
        "Policy recorded config hash",
        policy.recorded_config_hash.as_deref(),
    );
    output.optional(
        "Policy recorded tuple hash",
        policy.recorded_tuple_hash.as_deref(),
    );
    output.optional(
        "Policy current config hash",
        policy.current_config_hash.as_deref(),
    );
    output.optional(
        "Policy current tuple hash",
        policy.current_tuple_hash.as_deref(),
    );
    output.optional("Policy activated at", policy.activated_at.as_deref());
    output.optional("Policy deactivated at", policy.deactivated_at.as_deref());
    output.optional("Policy stale reason", policy.stale_reason.as_deref());
    output.optional("Policy basis", policy.basis.as_deref());
    if !policy.activations.is_empty() {
        output.heading("Policy activations");
        for (index, activation) in policy.activations.iter().enumerate() {
            output.field(
                &format!("Policy activation {}", index + 1),
                format!(
                    "{} ({}), active={}",
                    activation.activation_id, activation.tuple, activation.active
                ),
            );
            output.field(
                &format!("Policy activation {} recorded config hash", index + 1),
                &activation.recorded_config_hash,
            );
            output.field(
                &format!("Policy activation {} recorded tuple hash", index + 1),
                &activation.recorded_tuple_hash,
            );
            output.field(
                &format!("Policy activation {} current config hash", index + 1),
                &activation.current_config_hash,
            );
            output.field(
                &format!("Policy activation {} current tuple hash", index + 1),
                &activation.current_tuple_hash,
            );
            output.optional(
                &format!("Policy activation {} stale reason", index + 1),
                activation.stale_reason.as_deref(),
            );
        }
    }
}

fn render_safety_fields(output: &mut Output, safety: &SafetyView) {
    output.heading("Safety");
    output.field("Safety state", safety.state.as_str());
    output.optional("Safety scan hash", safety.scan_hash.as_deref());
    if safety.findings.is_empty() {
        output.field("Safety finding", "none");
    } else {
        for (index, finding) in safety.findings.iter().enumerate() {
            output.field(
                &format!("Safety finding {}", index + 1),
                format!("{} at {}", finding.reason_code, finding.location),
            );
            output.field(
                &format!("Safety finding {} source", index + 1),
                &finding.source,
            );
            output.optional(
                &format!("Safety finding {} metadata field", index + 1),
                finding.metadata_field.as_deref(),
            );
            output.field(
                &format!("Safety finding {} byte range", index + 1),
                format!("{}..{}", finding.start, finding.end),
            );
        }
    }
    output.optional("Safety override hash", safety.override_hash.as_deref());
    output.optional("Safety override reason", safety.override_reason.as_deref());
}

fn render_delivery_outcome(output: &mut Output, outcome: &DeliveryOutcome) {
    output.heading("Delivery result");
    output.field("Outcome", outcome.as_str());
    match outcome {
        DeliveryOutcome::Unclaimed | DeliveryOutcome::Claimed => {}
        DeliveryOutcome::Accepted { message_id } => {
            output.field("Remote message ID", message_id);
            output.field(
                "Remote meaning",
                "message accepted; no recipient-attention or response claim",
            );
        }
        DeliveryOutcome::Failed { code } => {
            output.field("Failure code", code);
        }
        DeliveryOutcome::RetryWait {
            next_attempt,
            delay_seconds,
            delay_kind,
        } => {
            output.field("Next attempt", next_attempt.to_string());
            output.optional(
                "Wait seconds",
                delay_seconds.map(|value| value.to_string()).as_deref(),
            );
            output.field("Wait kind", delay_kind);
        }
        DeliveryOutcome::Unknown { reason } => {
            output.field("Unknown reason", reason);
            output.field("Resend", "blocked until exact reconciliation");
        }
        DeliveryOutcome::ReconciliationUnknown {
            reason,
            successful_reads,
            observation_started_at_unix_seconds,
            last_successful_read_at_unix_seconds,
        } => {
            output.field("Unknown reason", reason);
            output.field(
                "Successful reconciliation reads",
                successful_reads.to_string(),
            );
            output.field(
                "Observation started",
                format!("Unix {observation_started_at_unix_seconds} seconds"),
            );
            output.optional(
                "Last successful read",
                last_successful_read_at_unix_seconds
                    .map(|value| format!("Unix {value} seconds"))
                    .as_deref(),
            );
            output.field("Resend", "blocked until exact reconciliation");
        }
        DeliveryOutcome::ReconciledAccepted {
            message_id,
            successful_reads,
            observed_at_unix_seconds,
        } => {
            output.field("Reconciled message ID", message_id);
            output.optional(
                "Successful reconciliation reads",
                successful_reads.map(|value| value.to_string()).as_deref(),
            );
            output.optional(
                "Observed at",
                observed_at_unix_seconds
                    .map(|value| format!("Unix {value} seconds"))
                    .as_deref(),
            );
            output.field(
                "Remote meaning",
                "exact message observed; no recipient-attention or response claim",
            );
        }
        DeliveryOutcome::ReconciledAbsent {
            successful_reads,
            observation_started_at_unix_seconds,
            observed_at_unix_seconds,
        } => {
            output.optional(
                "Successful reconciliation reads",
                successful_reads.map(|value| value.to_string()).as_deref(),
            );
            output.optional(
                "Observation started",
                observation_started_at_unix_seconds
                    .map(|value| format!("Unix {value} seconds"))
                    .as_deref(),
            );
            output.optional(
                "Observed at",
                observed_at_unix_seconds
                    .map(|value| format!("Unix {value} seconds"))
                    .as_deref(),
            );
        }
        DeliveryOutcome::Unresolved {
            reason,
            successful_reads,
        } => {
            output.field("Unresolved reason", reason);
            output.optional(
                "Successful reconciliation reads",
                successful_reads.map(|value| value.to_string()).as_deref(),
            );
        }
        DeliveryOutcome::EligibilityRejected { blocker } => {
            output.field("Eligibility blocker", blocker);
        }
        DeliveryOutcome::Expired => {
            output.field("Expiry", "draft or approval boundary reached");
        }
        DeliveryOutcome::StaleAuthority { reason } => {
            output.field("Stale authority", reason);
        }
        DeliveryOutcome::Error { category, detail } => {
            output.field("Error category", category);
            output.block("Error detail", detail);
        }
    }
}

fn reply_reference_lines(reference: &crate::preview::ReplyReferenceView) -> String {
    format!(
        "repository={} workspace={} channel={} inbound_item={} message={} authorization={} snapshot_hash={}",
        reference.repository_id,
        reference.workspace_id,
        reference.channel_id,
        reference.inbound_item_id,
        reference.message_id,
        reference.authorization_reference,
        reference.validated_snapshot_hash
    )
}

fn policy_next_action(state: PolicyState) -> &'static str {
    match state {
        PolicyState::Active => {
            "Carry the exact policy basis into the current send decision; do not widen the tuple."
        }
        PolicyState::Stale => {
            "Reactivate only the exact current configuration and tuple through the operator boundary."
        }
        PolicyState::Deactivated => {
            "Use a new explicit operator decision or human approval for this revision."
        }
        PolicyState::Ambiguous => {
            "Keep the send blocked and remove the ambiguous local activation state."
        }
        PolicyState::NotActivated => {
            "Activate the exact policy tuple interactively, or use human approval."
        }
        PolicyState::NotConfigured => "Configure an exact policy tuple or use human approval.",
        PolicyState::NotEvaluated => "Evaluate the exact policy tuple before any send claim.",
    }
}

fn approval_outcome(state: ApprovalState) -> &'static str {
    match state {
        ApprovalState::Valid => "approved",
        ApprovalState::Expired => "approval-expired",
        ApprovalState::Stale => "approval-stale",
        ApprovalState::Revoked => "approval-revoked",
        ApprovalState::OverrideRequired => "approval-override-required",
        ApprovalState::Missing => "approval-missing",
        ApprovalState::Invalid => "approval-invalid",
        ApprovalState::NotEvaluated => "approval-not-evaluated",
    }
}

fn approval_next_action(state: ApprovalState) -> &'static str {
    match state {
        ApprovalState::Valid => "Carry the exact approval hashes into the current send decision.",
        ApprovalState::Expired => {
            "Create a new revision or obtain a new exact approval; do not reuse expired authority."
        }
        ApprovalState::Stale | ApprovalState::Revoked => {
            "Rebuild current approval evidence before attempting delivery."
        }
        ApprovalState::OverrideRequired => {
            "Review the redacted finding and request an exact TTY override if appropriate."
        }
        ApprovalState::Missing | ApprovalState::NotEvaluated => {
            "Present the complete exact preview and request interactive approval or policy authority."
        }
        ApprovalState::Invalid => "Resolve the redacted approval error before attempting delivery.",
    }
}

fn optional_text(value: Option<&str>) -> &str {
    value.unwrap_or("(none)")
}

/// A small compile-time assertion that the public minimum remains visible in
/// the renderer API.
const _: () = assert!(MIN_TERMINAL_WIDTH == 80);

#[cfg(test)]
mod tests {
    use super::{RenderOptions, is_ansi_free, render_error, render_preview};
    use crate::preview::{DestinationView, ErrorView, MetadataView, OutboundPreview};

    fn preview() -> OutboundPreview {
        OutboundPreview {
            repository_id: "acme/widgets".to_owned(),
            draft_id: "draft-1".to_owned(),
            revision: 7,
            revision_hash: "a".repeat(64),
            destination: DestinationView::new("release", "100", "200", Vec::new()),
            exact_text: "build failed\nplease investigate".to_owned(),
            exact_text_hash: "b".repeat(64),
            metadata: MetadataView {
                repository_label: Some("widgets".to_owned()),
                branch: Some("main".to_owned()),
                commit: Some("abc123".to_owned()),
            },
            event_type: "build_failed".to_owned(),
            severity: "high".to_owned(),
            reply_reference: None,
            created_at_unix_seconds: 1_000,
            expires_at_unix_seconds: 2_000,
            approval: Default::default(),
            policy: Default::default(),
            safety: Default::default(),
            preview_hash: "c".repeat(64),
            provenance: crate::preview::Provenance::LocalDecision,
        }
    }

    #[test]
    fn plain_preview_has_no_ansi_and_fits_contract_width() {
        let rendered = render_preview(&preview(), RenderOptions::plain_text());
        assert!(is_ansi_free(&rendered));
        assert!(rendered.lines().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn error_view_has_text_next_action() {
        let rendered = render_error(
            &ErrorView::new("storage-integrity", "redacted detail", "repair local state"),
            RenderOptions::plain_text(),
        );
        assert!(rendered.contains("Error category: storage-integrity"));
        assert!(rendered.contains("Next action: repair local state"));
    }
}
