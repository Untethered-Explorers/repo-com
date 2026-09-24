use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use repo_com_foundation::DiagnosticsChoice;
use repo_com_state::{AuditEventRecord, StateStore, StateTransaction};
use serde::Serialize;

use crate::{AuditError, AuditEvent, AuditResult, RedactedAuditEvent};

static GLOBAL_DIAGNOSTIC_LEVEL: AtomicU8 = AtomicU8::new(DiagnosticLevel::Off.as_u8());

/// Environment setting that explicitly enables structured diagnostics.
pub const DIAGNOSTICS_ENV: &str = "REPO_COM_DIAGNOSTICS";

/// Audit-specific compatibility environment setting.
pub const AUDIT_DIAGNOSTICS_ENV: &str = "REPO_COM_AUDIT_DIAGNOSTICS";

/// A destination that can accept one redacted audit event in its current
/// transaction or connection boundary.
///
/// The target receives an [`AuditEvent`], not the state crate's raw input
/// type, so callers cannot bypass this crate's redaction boundary through the
/// writer API.
pub trait AuditAppendTarget {
    /// Appends one event after validating and redacting it.
    fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord>;
}

impl AuditAppendTarget for StateStore {
    fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        let input = event.to_state_input()?;
        Ok(self.append_audit_event(&input)?)
    }
}

impl AuditAppendTarget for &mut StateStore {
    fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        let input = event.to_state_input()?;
        Ok((**self).append_audit_event(&input)?)
    }
}

impl<'conn> AuditAppendTarget for StateTransaction<'conn> {
    fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        let input = event.to_state_input()?;
        Ok(self.append_audit_event(&input)?)
    }
}

impl<'conn> AuditAppendTarget for &StateTransaction<'conn> {
    fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        let input = event.to_state_input()?;
        Ok((*self).append_audit_event(&input)?)
    }
}

impl<'conn> AuditAppendTarget for &mut StateTransaction<'conn> {
    fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        let input = event.to_state_input()?;
        Ok((**self).append_audit_event(&input)?)
    }
}

/// Transaction-aware append-only audit writer.
///
/// Constructing a writer does not perform I/O. A writer can target a mutable
/// [`StateStore`] for a standalone append or a borrowed
/// [`StateTransaction`] when the event must share a lifecycle mutation's
/// transaction. The type intentionally exposes no update or delete method.
pub struct AuditWriter<T = StateStore> {
    target: T,
    diagnostics: DiagnosticConfig,
}

impl<T> AuditWriter<T>
where
    T: AuditAppendTarget,
{
    /// Creates a writer with diagnostics disabled by default.
    pub fn new(target: T) -> Self {
        Self {
            target,
            diagnostics: DiagnosticConfig::default(),
        }
    }

    /// Creates a writer with an explicit diagnostic configuration.
    pub fn with_diagnostics(target: T, diagnostics: DiagnosticConfig) -> Self {
        Self {
            target,
            diagnostics,
        }
    }

    /// Creates a writer after explicitly reading the opt-in environment
    /// setting. Missing or disabled settings remain off.
    pub fn from_environment(target: T) -> AuditResult<Self> {
        Ok(Self::with_diagnostics(
            target,
            DiagnosticConfig::from_environment()?,
        ))
    }

    /// Appends one event and optionally emits a committed-store diagnostic.
    pub fn append(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        let record = self.append_without_diagnostics(event)?;
        if self.diagnostics.is_enabled() {
            let mut emitter = DiagnosticEmitter::new(self.diagnostics, io::stderr());
            // Diagnostics are not evidence and must not turn a committed
            // append into a caller-visible failure that could be retried.
            let _ = emitter.emit(DiagnosticLevel::Info, event);
        }
        Ok(record)
    }

    /// Alias for [`AuditWriter::append`].
    pub fn append_event(&mut self, event: &AuditEvent) -> AuditResult<AuditEventRecord> {
        self.append(event)
    }

    /// Appends one event to a separately supplied state transaction. The
    /// writer target is not used for this call, so the method is useful when a
    /// workflow already owns a transaction handle.
    pub fn append_in_transaction(
        &mut self,
        transaction: &StateTransaction<'_>,
        event: &AuditEvent,
    ) -> AuditResult<AuditEventRecord> {
        append_in_transaction(transaction, event)
    }

    /// Appends one event without emitting a diagnostic. This is the preferred
    /// method for a transaction target because the caller controls when the
    /// surrounding transaction commits or rolls back.
    pub fn append_without_diagnostics(
        &mut self,
        event: &AuditEvent,
    ) -> AuditResult<AuditEventRecord> {
        self.target.append_event(event)
    }

    /// Returns the current diagnostic configuration.
    #[must_use]
    pub const fn diagnostics(&self) -> DiagnosticConfig {
        self.diagnostics
    }

    /// Returns the underlying target.
    #[must_use]
    pub const fn target(&self) -> &T {
        &self.target
    }

    /// Returns the underlying target mutably.
    pub fn target_mut(&mut self) -> &mut T {
        &mut self.target
    }

    /// Consumes the writer and returns its target.
    pub fn into_target(self) -> T {
        self.target
    }
}

/// Appends one event to a caller-owned state transaction without creating a
/// second transaction. The state mutation and this insert therefore share the
/// caller's commit or rollback boundary.
pub fn append_in_transaction(
    transaction: &StateTransaction<'_>,
    event: &AuditEvent,
) -> AuditResult<AuditEventRecord> {
    let mut writer = AuditWriter::new(transaction);
    writer.append_without_diagnostics(event)
}

/// Alias with an explicit name for workflow call sites.
pub fn append_to_transaction(
    transaction: &StateTransaction<'_>,
    event: &AuditEvent,
) -> AuditResult<AuditEventRecord> {
    append_in_transaction(transaction, event)
}

/// Structured diagnostic severity. No level bypasses redaction.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum DiagnosticLevel {
    /// Emit nothing.
    #[default]
    Off = 0,
    /// Emit error-level diagnostics.
    Error = 1,
    /// Emit warning-level diagnostics and above.
    Warn = 2,
    /// Emit informational diagnostics and above.
    Info = 3,
    /// Emit debug diagnostics and above.
    Debug = 4,
    /// Emit the highest supported diagnostic detail.
    Trace = 5,
}

impl DiagnosticLevel {
    const fn as_u8(self) -> u8 {
        self as u8
    }

    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Off),
            1 => Some(Self::Error),
            2 => Some(Self::Warn),
            3 => Some(Self::Info),
            4 => Some(Self::Debug),
            5 => Some(Self::Trace),
            _ => None,
        }
    }

    /// Returns the stable diagnostic spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    /// Returns whether this level can emit output.
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Returns whether a requested level is allowed by this threshold.
    #[must_use]
    pub fn allows(self, requested: Self) -> bool {
        self.is_enabled() && requested.is_enabled() && requested <= self
    }
}

impl std::fmt::Display for DiagnosticLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Explicit diagnostic configuration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticConfig {
    /// Maximum level that may be emitted.
    pub level: DiagnosticLevel,
}

impl DiagnosticConfig {
    /// Creates a configuration for one level.
    #[must_use]
    pub const fn new(level: DiagnosticLevel) -> Self {
        Self { level }
    }

    /// Creates the safe default configuration.
    #[must_use]
    pub const fn disabled() -> Self {
        Self::new(DiagnosticLevel::Off)
    }

    /// Creates an enabled configuration at the supplied level.
    #[must_use]
    pub const fn enabled(level: DiagnosticLevel) -> Self {
        Self::new(level)
    }

    /// Creates a configuration from an explicit operator flag.
    #[must_use]
    pub const fn from_operator_flag(enabled: bool, level: DiagnosticLevel) -> Self {
        if enabled {
            Self::new(level)
        } else {
            Self::disabled()
        }
    }

    /// Converts the foundation flag while keeping the audit level explicit.
    #[must_use]
    pub const fn from_foundation_choice(choice: DiagnosticsChoice, level: DiagnosticLevel) -> Self {
        Self::from_operator_flag(choice.enabled(), level)
    }

    /// Reads the explicit environment setting. Absent, false, and off values
    /// all resolve to the disabled default; malformed values fail closed.
    pub fn from_environment() -> AuditResult<Self> {
        let raw = match std::env::var(DIAGNOSTICS_ENV) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => match std::env::var(AUDIT_DIAGNOSTICS_ENV) {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(_) => return Err(AuditError::InvalidDiagnosticConfiguration),
            },
            Err(_) => return Err(AuditError::InvalidDiagnosticConfiguration),
        };
        raw.map_or(Ok(Self::disabled()), |value| {
            Ok(Self::new(parse_diagnostic_level(&value)?))
        })
    }

    /// Returns whether output is enabled.
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        self.level.is_enabled()
    }
}

/// Parses a diagnostic level without accepting aliases that could broaden
/// output unexpectedly.
pub fn parse_diagnostic_level(value: &str) -> AuditResult<DiagnosticLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "0" | "false" | "no" | "disabled" | "" => Ok(DiagnosticLevel::Off),
        "error" | "err" => Ok(DiagnosticLevel::Error),
        "warn" | "warning" => Ok(DiagnosticLevel::Warn),
        "info" | "on" | "true" | "1" => Ok(DiagnosticLevel::Info),
        "debug" => Ok(DiagnosticLevel::Debug),
        "trace" | "verbose" => Ok(DiagnosticLevel::Trace),
        _ => Err(AuditError::InvalidDiagnosticConfiguration),
    }
}

/// One redacted structured diagnostic line.
#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticRecord {
    /// Requested diagnostic level.
    pub level: String,
    /// Redacted event envelope and metadata.
    #[serde(flatten)]
    pub event: RedactedAuditEvent,
}

impl DiagnosticRecord {
    /// Creates a safe record for one event.
    #[must_use]
    pub fn new(level: DiagnosticLevel, event: &AuditEvent) -> Self {
        Self {
            level: level.as_str().to_owned(),
            event: event.redacted(),
        }
    }
}

/// Renders a diagnostic only when the requested level is enabled.
///
/// Redaction is performed before serialization at every level, including
/// [`DiagnosticLevel::Trace`]. A disabled configuration returns no output.
pub fn render_diagnostic(
    config: DiagnosticConfig,
    requested: DiagnosticLevel,
    event: &AuditEvent,
) -> AuditResult<Option<String>> {
    if !config.level.allows(requested) {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(&DiagnosticRecord::new(
        requested, event,
    ))?))
}

/// A writer that emits structured diagnostics to an injected stream.
pub struct DiagnosticEmitter<W> {
    config: DiagnosticConfig,
    writer: W,
}

impl<W> DiagnosticEmitter<W>
where
    W: Write,
{
    /// Creates an emitter with an explicit threshold.
    pub fn new(config: DiagnosticConfig, writer: W) -> Self {
        Self { config, writer }
    }

    /// Returns the configured threshold.
    #[must_use]
    pub const fn config(&self) -> DiagnosticConfig {
        self.config
    }

    /// Emits one redacted JSON line if the requested level is enabled.
    pub fn emit(&mut self, requested: DiagnosticLevel, event: &AuditEvent) -> AuditResult<bool> {
        let Some(line) = render_diagnostic(self.config, requested, event)? else {
            return Ok(false);
        };
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        Ok(true)
    }

    /// Alias for [`DiagnosticEmitter::emit`].
    pub fn emit_event(
        &mut self,
        requested: DiagnosticLevel,
        event: &AuditEvent,
    ) -> AuditResult<bool> {
        self.emit(requested, event)
    }

    /// Returns the injected writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

/// Initializes the process-local diagnostic threshold.
///
/// Initialization is explicit and starts at the disabled default. The writer
/// only targets the diagnostic stream; no protocol stdout path exists in this
/// crate.
pub fn init_diagnostics(config: DiagnosticConfig) -> DiagnosticLevel {
    let level = config.level;
    GLOBAL_DIAGNOSTIC_LEVEL.store(level.as_u8(), Ordering::Release);
    level
}

/// Initializes diagnostics from the explicit environment setting.
pub fn init_diagnostics_from_env() -> AuditResult<DiagnosticLevel> {
    let config = DiagnosticConfig::from_environment()?;
    Ok(init_diagnostics(config))
}

/// Initializes diagnostics from an explicit operator flag.
pub fn init_diagnostics_with_flag(enabled: bool, level: DiagnosticLevel) -> DiagnosticLevel {
    init_diagnostics(DiagnosticConfig::from_operator_flag(enabled, level))
}

/// Returns the process-local diagnostic threshold.
#[must_use]
pub fn current_diagnostic_config() -> DiagnosticConfig {
    let value = GLOBAL_DIAGNOSTIC_LEVEL.load(Ordering::Acquire);
    DiagnosticConfig::new(DiagnosticLevel::from_u8(value).unwrap_or(DiagnosticLevel::Off))
}

/// Returns whether process-local diagnostics are enabled.
#[must_use]
pub fn diagnostics_enabled() -> bool {
    current_diagnostic_config().is_enabled()
}

/// Disables process-local diagnostics and returns the prior level.
pub fn disable_diagnostics() -> DiagnosticLevel {
    let previous = current_diagnostic_config().level;
    init_diagnostics(DiagnosticConfig::disabled());
    previous
}

/// Emits one event to the process-local diagnostic threshold using stderr.
pub fn emit_diagnostic(requested: DiagnosticLevel, event: &AuditEvent) -> AuditResult<bool> {
    let mut emitter = DiagnosticEmitter::new(current_diagnostic_config(), io::stderr());
    emitter.emit(requested, event)
}
