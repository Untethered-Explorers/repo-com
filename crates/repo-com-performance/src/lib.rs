#![forbid(unsafe_code)]
#![doc = "Token-free, isolated warm-command performance evidence for the composed repo-com binary."]

use std::env;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The number of measured warm process invocations required for every class.
pub const SAMPLE_COUNT: usize = 100;

/// The number of untimed warm-up invocations performed before each class.
pub const WARMUP_SAMPLE_COUNT: usize = 1;

/// The inclusive p95 budget from `REL-FR-05` and `RC-NFR-01`.
pub const P95_THRESHOLD: Duration = Duration::from_millis(500);

/// The canonical local command classes selected by this harness.
///
/// The list intentionally contains only repeatable local read/preview/planning
/// commands. Network-capable commands (`send`, `setup-check`, and
/// `inbox.fetch`) and interactive-only commands are not silently turned into
/// performance samples.
pub const DOCUMENTED_NO_NETWORK_COMMANDS: &[&str] = &[
    "config.validate",
    "policy.status",
    "state.verify",
    "lifecycle.inspect",
    "audit.query",
    "purge.plan",
    "draft.show",
    "draft.preview",
];

const REPOSITORY_ID: &str = "acme/widgets";
const DRAFT_ID: &str = "performance-fixture-draft";
const INBOUND_ALIAS: &str = "release";
const CONFIG_FILE_NAME: &str = ".repo-com.toml";
const STATE_FILE_NAME: &str = "performance-state.sqlite3";
const CONFIG_TEXT: &str = r#"schema_version = 1
repository_id = "acme/widgets"
auto_send = []

[discord]
workspace_id = "100000000000000001"

[destinations.release]
channel_id = "200000000000000001"
allowed_mentions = []

[mentions]

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365
"#;

/// A focused error type that never includes child output or environment values.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HarnessError(String);

impl HarnessError {
    /// Creates a safe, non-secret harness error.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HarnessError {}

/// Result type used by the harness.
pub type Result<T> = std::result::Result<T, HarnessError>;

/// One measured process invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessObservation {
    /// Complete child-process wall duration, including spawn and output drain.
    pub duration: Duration,
    /// Captured process exit code, or `None` when terminated by a signal.
    pub exit_code: Option<i32>,
}

/// A command specification used for one isolated fixture invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    /// Stable class name used in the evidence report.
    pub class: String,
    /// Canonical command name, retained separately from shell arguments.
    pub command: String,
    /// Arguments passed to the final binary after global options.
    pub args: Vec<String>,
    /// Protocol-version-1 JSON written to stdin.
    pub stdin: String,
    /// Must be false for every measured command in this harness.
    pub network: bool,
}

impl CommandSpec {
    fn new(class: &str, command: &str, args: Vec<String>, stdin: Value) -> Self {
        Self {
            class: class.to_owned(),
            command: command.to_owned(),
            args,
            stdin: serde_json::to_string(&stdin).unwrap_or_else(|_| "{}".to_owned()),
            network: false,
        }
    }
}

/// A sorted sample distribution with an exact nanosecond p95 and report-friendly
/// millisecond fields. Milliseconds are rounded up so a sub-millisecond excess
/// cannot be hidden in human-readable evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Distribution {
    /// Number of measured samples used for the distribution.
    pub sample_count: usize,
    /// p50 rounded up to milliseconds.
    pub p50_ms: u64,
    /// p95 rounded up to milliseconds.
    pub p95_ms: u64,
    /// Maximum rounded up to milliseconds.
    pub max_ms: u64,
    /// Exact p50 nanoseconds.
    pub p50_ns: u64,
    /// Exact p95 nanoseconds.
    pub p95_ns: u64,
    /// Exact maximum nanoseconds.
    pub max_ns: u64,
}

impl Distribution {
    /// Computes nearest-rank p50/p95/max from a non-empty sample set.
    pub fn from_samples(samples: &[Duration]) -> Result<Self> {
        if samples.is_empty() {
            return Err(HarnessError::new(
                "at least one performance sample is required",
            ));
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let p50 = sorted[percentile_index(sorted.len(), 50)];
        let p95 = sorted[percentile_index(sorted.len(), 95)];
        let max = *sorted.last().expect("non-empty samples have a maximum");
        Ok(Self {
            sample_count: sorted.len(),
            p50_ms: duration_ms_ceil(p50),
            p95_ms: duration_ms_ceil(p95),
            max_ms: duration_ms_ceil(max),
            p50_ns: duration_nanos(p50),
            p95_ns: duration_nanos(p95),
            max_ns: duration_nanos(max),
        })
    }

    /// Returns the exact p95 duration.
    #[must_use]
    pub fn p95_duration(&self) -> Duration {
        Duration::from_nanos(self.p95_ns)
    }

    /// Tests the inclusive performance budget.
    #[must_use]
    pub fn passes(&self, threshold: Duration) -> bool {
        self.p95_duration() <= threshold
    }
}

fn percentile_index(len: usize, percentile: usize) -> usize {
    // ceil(percentile / 100 * len) - 1, with integer arithmetic and a safe
    // upper bound. The samples are non-empty by the caller.
    let rank = (len.saturating_mul(percentile).saturating_add(99)) / 100;
    rank.saturating_sub(1).min(len - 1)
}

fn duration_nanos(duration: Duration) -> u64 {
    duration.as_nanos().try_into().unwrap_or(u64::MAX)
}

fn duration_ms_ceil(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos().saturating_add(999_999) / 1_000_000).unwrap_or(u64::MAX)
}

/// Machine-readable environment captured outside the measured interval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    /// Explicit CI reference-runner label, or `unrecorded` when not supplied.
    pub runner: String,
    /// Host/machine label supplied by the runner, or a safe hostname fallback.
    pub machine: String,
    /// Operating system and platform family.
    pub os: String,
    /// Rust target architecture.
    pub arch: String,
    /// Actual `rustc --version` output when available.
    pub toolchain: String,
    /// Actual `cargo --version` output when available.
    pub cargo: String,
    /// Available logical CPU count, when the platform exposes it.
    pub cpu_threads: Option<usize>,
    /// True only when the runner was explicitly identified by the harness.
    pub reference_runner_verified: bool,
}

/// Explicit measurement exclusions and scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementExclusions {
    /// Dependency compilation/build work is completed before sampling.
    pub first_compilation_excluded: bool,
    /// The allowlisted command set has no network-capable operation.
    pub network_io_excluded: bool,
    /// Complete process wall time includes spawn and child startup.
    pub process_start_included_in_wall_time: bool,
    /// The harness never substitutes an internal domain timer for process time.
    pub internal_function_timer_used: bool,
    /// Untimed warm-up invocations are excluded from the measured sample count.
    pub warmup_invocations_excluded: bool,
    /// Human-readable scope statement for release evidence consumers.
    pub scope: String,
}

impl Default for MeasurementExclusions {
    fn default() -> Self {
        Self {
            first_compilation_excluded: true,
            network_io_excluded: true,
            process_start_included_in_wall_time: true,
            internal_function_timer_used: false,
            warmup_invocations_excluded: true,
            scope: "complete child-process wall time from immediately before spawn through output drain; first compilation and network-capable commands are excluded".to_owned(),
        }
    }
}

/// Network boundary evidence for the measured allowlist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkEvidence {
    /// Number of measured specs marked as network-capable.
    pub network_capable_commands_selected: u64,
    /// Harness-level count of external requests made by the measured classes.
    pub external_request_count: u64,
    /// Whether a Discord token was present in the child environment.
    pub discord_token_present: bool,
    /// Whether a platform network-syscall probe was available for this run.
    pub network_syscall_probe_supported: bool,
    /// Boundary used to make the zero-request result explicit.
    pub proof: String,
}

impl Default for NetworkEvidence {
    fn default() -> Self {
        Self {
            network_capable_commands_selected: 0,
            external_request_count: 0,
            discord_token_present: false,
            network_syscall_probe_supported: false,
            proof: "token-free allowlist of local commands; each child runs with Discord credentials removed and no network-capable route selected".to_owned(),
        }
    }
}

/// Result for one documented command class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassReport {
    /// Stable class name.
    pub class: String,
    /// Canonical protocol command.
    pub command: String,
    /// Exactly the configured number of measured warm samples.
    pub sample_count: usize,
    /// Untimed invocations performed before measurement.
    pub warmup_sample_count: usize,
    /// Complete process count including warm-up invocations.
    pub complete_process_count: usize,
    /// Measured distribution.
    pub distribution: Distribution,
    /// Inclusive threshold used for this class.
    pub threshold_ms: u64,
    /// Whether the measured p95 is within the budget.
    pub passed: bool,
    /// Per-class network boundary evidence.
    pub network: NetworkEvidence,
    /// All measured durations in milliseconds, rounded up.
    pub samples_ms: Vec<u64>,
}

/// Configuration for a harness run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessConfig {
    /// Number of measured samples per class.
    pub sample_count: usize,
    /// Number of untimed warm-up invocations per class.
    pub warmup_sample_count: usize,
    /// Inclusive p95 threshold.
    pub threshold: Duration,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            sample_count: SAMPLE_COUNT,
            warmup_sample_count: WARMUP_SAMPLE_COUNT,
            threshold: P95_THRESHOLD,
        }
    }
}

impl HarnessConfig {
    fn validate(&self) -> Result<()> {
        if self.sample_count == 0 {
            return Err(HarnessError::new("sample_count must be greater than zero"));
        }
        Ok(())
    }
}

/// A complete, machine-readable performance report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceReport {
    /// Stable evidence schema.
    pub schema_version: u8,
    /// Owning task identifier.
    pub task_id: String,
    /// Name of the final binary measured, without a user-specific path.
    pub final_binary_name: String,
    /// Environment captured before measurement.
    pub environment: EnvironmentReport,
    /// Global threshold and sample policy.
    pub sample_count_per_class: usize,
    pub warmup_sample_count_per_class: usize,
    pub threshold_ms: u64,
    /// Explicit scope/exclusion evidence.
    pub exclusions: MeasurementExclusions,
    /// Network boundary evidence.
    pub network: NetworkEvidence,
    /// Per-class evidence in deterministic command order.
    pub classes: Vec<ClassReport>,
    /// Aggregate result.
    pub passed: bool,
}

impl PerformanceReport {
    /// Returns the exact p95 for a named class, if present.
    #[must_use]
    pub fn class(&self, name: &str) -> Option<&ClassReport> {
        self.classes.iter().find(|class| class.class == name)
    }
}

/// An isolated temporary repository and state fixture.
#[derive(Debug)]
pub struct Fixture {
    root: PathBuf,
    config_path: PathBuf,
    state_path: PathBuf,
}

impl Fixture {
    /// Creates a token-free fixture outside the repository checkout.
    pub fn create() -> Result<Self> {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "repo-com-performance-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        let fixture = Self {
            config_path: root.join(CONFIG_FILE_NAME),
            state_path: root.join(STATE_FILE_NAME),
            root,
        };
        if let Err(error) = fixture.initialize() {
            let _ = fs::remove_dir_all(&fixture.root);
            return Err(HarnessError::new(format!(
                "temporary performance fixture could not be created: {error}"
            )));
        }
        Ok(fixture)
    }

    fn initialize(&self) -> std::io::Result<()> {
        fs::create_dir_all(self.root.join(".git"))?;
        restrict_permissions(&self.root)?;
        fs::write(&self.config_path, CONFIG_TEXT)?;
        Ok(())
    }

    /// Returns the temporary repository root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the isolated config path.
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Returns the isolated state path.
    #[must_use]
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// Builds the deterministic, repeatable local command allowlist.
    #[must_use]
    pub fn command_specs(&self) -> Vec<CommandSpec> {
        vec![
            self.spec(
                "config.validate",
                &["config", "validate"],
                json!({ "repository_id": REPOSITORY_ID }),
            ),
            self.spec(
                "policy.status",
                &["policy", "status"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "event_type": "build_failed",
                    "destination_alias": INBOUND_ALIAS,
                    "severity": "high",
                }),
            ),
            self.spec(
                "state.verify",
                &["state", "verify"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "database_path": self.state_path.to_string_lossy(),
                }),
            ),
            self.spec(
                "lifecycle.inspect",
                &["lifecycle", "inspect"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "object_type": "repository",
                    "page_size": 10,
                }),
            ),
            self.spec(
                "audit.query",
                &["audit", "query"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "page_size": 10,
                }),
            ),
            self.spec(
                "purge.plan",
                &["purge", "plan"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "scope": "content",
                    "cutoff": "2026-01-01T00:00:00Z",
                }),
            ),
            self.spec(
                "draft.show",
                &["draft", "show"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "draft_id": DRAFT_ID,
                    "revision": 1,
                }),
            ),
            self.spec(
                "draft.preview",
                &["draft", "preview"],
                json!({
                    "repository_id": REPOSITORY_ID,
                    "draft_id": DRAFT_ID,
                    "revision": 1,
                }),
            ),
        ]
    }

    fn spec(&self, class: &str, route: &[&str], input: Value) -> CommandSpec {
        let mut args = self.base_args();
        args.extend(route.iter().map(|value| (*value).to_owned()));
        CommandSpec::new(
            class,
            DOCUMENTED_NO_NETWORK_COMMANDS
                .iter()
                .find(|candidate| **candidate == class)
                .copied()
                .unwrap_or(class),
            args,
            envelope(
                DOCUMENTED_NO_NETWORK_COMMANDS
                    .iter()
                    .find(|candidate| **candidate == class)
                    .copied()
                    .unwrap_or(class),
                input,
            ),
        )
    }

    fn base_args(&self) -> Vec<String> {
        vec![
            "--config".to_owned(),
            self.config_path.to_string_lossy().into_owned(),
            "--state".to_owned(),
            self.state_path.to_string_lossy().into_owned(),
            "--output".to_owned(),
            "json".to_owned(),
            "--non-tty".to_owned(),
        ]
    }

    /// Creates one local draft before measurement, establishing the fixed state
    /// used by the read-only classes. This setup invocation is never sampled.
    pub fn prepare(&self, binary: &Path) -> Result<()> {
        let mut args = self.base_args();
        args.extend(["draft".to_owned(), "create".to_owned()]);
        let spec = CommandSpec::new(
            "fixture.draft.create",
            "draft.create",
            args,
            envelope(
                "draft.create",
                json!({
                    "repository_id": REPOSITORY_ID,
                    "draft_id": DRAFT_ID,
                    "destination_alias": INBOUND_ALIAS,
                    "text": "synthetic performance fixture",
                    "event_type": "build_failed",
                    "severity": "high",
                    "created_at": "2099-01-01T00:00:00Z",
                    "created_at_unix_seconds": 4_070_908_800_u64,
                }),
            ),
        );
        let _ = execute_process(binary, &self.root, &spec)?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn envelope(command: &str, input: Value) -> Value {
    json!({
        "protocol_version": 1,
        "command": command,
        "input": input,
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Captures the current runner without making an unrecorded hardware claim.
#[must_use]
pub fn capture_environment() -> EnvironmentReport {
    let explicit_runner = [
        "REPO_COM_PERF_REFERENCE_RUNNER",
        "REPO_COM_REFERENCE_RUNNER",
    ]
    .into_iter()
    .find_map(|name| env::var(name).ok().filter(|value| !value.trim().is_empty()));
    let runner = explicit_runner
        .clone()
        .or_else(|| {
            env::var("GITHUB_RUNNER_NAME")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unrecorded".to_owned());
    let machine = env::var("REPO_COM_PERF_MACHINE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("HOSTNAME").ok().filter(|value| !value.is_empty()))
        .or_else(|| {
            env::var("COMPUTERNAME")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .or_else(machine_fallback)
        .unwrap_or_else(|| "unrecorded".to_owned());
    EnvironmentReport {
        runner,
        machine,
        os: format!("{} ({})", env::consts::OS, env::consts::FAMILY),
        arch: env::consts::ARCH.to_owned(),
        toolchain: command_version("rustc", &["--version"])
            .unwrap_or_else(|| "unavailable".to_owned()),
        cargo: command_version("cargo", &["--version"]).unwrap_or_else(|| "unavailable".to_owned()),
        cpu_threads: std::thread::available_parallelism().ok().map(usize::from),
        reference_runner_verified: explicit_runner.is_some(),
    }
}

#[cfg(unix)]
fn machine_fallback() -> Option<String> {
    fs::read_to_string("/etc/hostname")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(not(unix))]
fn machine_fallback() -> Option<String> {
    None
}

fn command_version(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let first_line = value.lines().next()?.trim();
    if first_line.is_empty() {
        None
    } else {
        Some(first_line.to_owned())
    }
}

/// Locates the final `repo-com` binary, building it outside the measured
/// interval when the workspace target is not already present.
pub fn ensure_final_binary() -> Result<PathBuf> {
    if let Some(value) = env::var_os("REPO_COM_FINAL_BINARY") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(HarnessError::new(
            "REPO_COM_FINAL_BINARY does not name an existing file",
        ));
    }
    if let Some(value) = env::var_os("CARGO_BIN_EXE_repo-com") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
    }

    let root = repository_root()?;
    build_final_binary(&root)?;
    let target = target_directory(&root);
    let path = target
        .join("debug")
        .join(format!("repo-com{}", env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(HarnessError::new(
            "the final repo-com binary was not produced by the workspace build",
        ))
    }
}

fn repository_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| HarnessError::new("performance crate manifest has no repository parent"))
}

fn target_directory(root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR").map_or_else(
        || root.join("target"),
        |value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    )
}

fn build_final_binary(root: &Path) -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "--quiet",
            "--package",
            "command_routing_contract",
            "--bin",
            "repo-com",
        ])
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .map_err(|_| HarnessError::new("cargo could not start to prepare the final binary"))?;
    if !output.status.success() {
        return Err(HarnessError::new(
            "workspace build for the final repo-com binary failed before measurement",
        ));
    }
    Ok(())
}

fn execute_process(binary: &Path, root: &Path, spec: &CommandSpec) -> Result<ProcessObservation> {
    let mut command = Command::new(binary);
    command
        .args(&spec.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("REPO_COM_DISCORD_TOKEN")
        .env("REPO_COM_PERF_NO_NETWORK", "1");
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|_| HarnessError::new("final repo-com process could not start"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(spec.stdin.as_bytes())
            .map_err(|_| HarnessError::new("structured performance input could not be written"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|_| HarnessError::new("final repo-com process could not complete"))?;
    let duration = started.elapsed();
    validate_process_output(spec, &output)?;
    Ok(ProcessObservation {
        duration,
        exit_code: output.status.code(),
    })
}

fn validate_process_output(spec: &CommandSpec, output: &Output) -> Result<()> {
    if !output.status.success() {
        return Err(HarnessError::new(format!(
            "{} returned a non-success process status",
            spec.class
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        HarnessError::new(format!(
            "{} did not return one JSON protocol object",
            spec.class
        ))
    })?;
    if value.get("status").and_then(Value::as_str) != Some("success") {
        return Err(HarnessError::new(format!(
            "{} returned a non-success protocol outcome",
            spec.class
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkProbe {
    external_request_count: u64,
    supported: bool,
    method: &'static str,
}

fn fallback_network_probe() -> NetworkProbe {
    NetworkProbe {
        external_request_count: 0,
        supported: false,
        method: "allowlist-and-token-removal",
    }
}

#[cfg(target_os = "linux")]
fn probe_network(binary: &Path, root: &Path, spec: &CommandSpec) -> Result<NetworkProbe> {
    let trace_path = root.join(format!(
        ".repo-com-network-probe-{}-{}.trace",
        std::process::id(),
        spec.class.replace('.', "_")
    ));
    let mut command = Command::new("strace");
    command
        .args(["-f", "-e", "trace=network", "-o"])
        .arg(&trace_path)
        .arg("--")
        .arg(binary)
        .args(&spec.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("REPO_COM_DISCORD_TOKEN")
        .env("REPO_COM_PERF_NO_NETWORK", "1");
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(fallback_network_probe());
        }
        Err(_) => return Ok(fallback_network_probe()),
    };
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(spec.stdin.as_bytes()).is_err()
    {
        let _ = fs::remove_file(&trace_path);
        return Ok(fallback_network_probe());
    }
    let output = child.wait_with_output();
    let Ok(output) = output else {
        let _ = fs::remove_file(&trace_path);
        return Ok(fallback_network_probe());
    };
    if !output.status.success() {
        let _ = fs::remove_file(&trace_path);
        return Ok(fallback_network_probe());
    }
    let trace = fs::read_to_string(&trace_path);
    let _ = fs::remove_file(&trace_path);
    let Ok(trace) = trace else {
        return Ok(fallback_network_probe());
    };
    // The trace file is intentionally inspected only for address-family
    // markers. AF_UNIX activity is local IPC and is not an external request.
    let external_request_count = trace
        .lines()
        .filter(|line| line.contains("AF_INET") || line.contains("AF_INET6"))
        .count() as u64;
    Ok(NetworkProbe {
        external_request_count,
        supported: true,
        method: "strace-network-syscalls",
    })
}

#[cfg(not(target_os = "linux"))]
fn probe_network(_binary: &Path, _root: &Path, _spec: &CommandSpec) -> Result<NetworkProbe> {
    Ok(fallback_network_probe())
}

fn network_evidence(probe: NetworkProbe) -> NetworkEvidence {
    NetworkEvidence {
        external_request_count: probe.external_request_count,
        network_syscall_probe_supported: probe.supported,
        proof: format!(
            "token-free allowlist; Discord credentials removed; probe={}",
            probe.method
        ),
        ..NetworkEvidence::default()
    }
}

/// Runs the complete measured allowlist and generates structured evidence.
pub fn run_harness(
    binary: &Path,
    fixture: &Fixture,
    config: &HarnessConfig,
) -> Result<PerformanceReport> {
    config.validate()?;
    if !binary.is_file() {
        return Err(HarnessError::new(
            "final repo-com binary path is not a file",
        ));
    }
    let binary = binary
        .canonicalize()
        .map_err(|_| HarnessError::new("final repo-com binary path could not be resolved"))?;
    let binary = binary.as_path();
    fixture.prepare(binary)?;
    let environment = capture_environment();
    let mut classes = Vec::new();
    let mut aggregate_network = NetworkEvidence {
        network_syscall_probe_supported: true,
        ..NetworkEvidence::default()
    };
    for spec in fixture.command_specs() {
        if spec.network {
            return Err(HarnessError::new(format!(
                "network-capable command class entered the performance allowlist: {}",
                spec.class
            )));
        }
        let probe = probe_network(binary, fixture.root(), &spec)?;
        if probe.external_request_count != 0 {
            return Err(HarnessError::new(format!(
                "{} made an external network syscall during the no-network probe",
                spec.class
            )));
        }
        let network = network_evidence(probe);
        aggregate_network.external_request_count = aggregate_network
            .external_request_count
            .saturating_add(network.external_request_count);
        aggregate_network.network_syscall_probe_supported = aggregate_network
            .network_syscall_probe_supported
            && network.network_syscall_probe_supported;
        for _ in 0..config.warmup_sample_count {
            let _ = execute_process(binary, fixture.root(), &spec)?;
        }
        let mut durations = Vec::with_capacity(config.sample_count);
        let mut samples_ms = Vec::with_capacity(config.sample_count);
        for _ in 0..config.sample_count {
            let observation = execute_process(binary, fixture.root(), &spec)?;
            samples_ms.push(duration_ms_ceil(observation.duration));
            durations.push(observation.duration);
        }
        let distribution = Distribution::from_samples(&durations)?;
        classes.push(ClassReport {
            class: spec.class,
            command: spec.command,
            sample_count: config.sample_count,
            warmup_sample_count: config.warmup_sample_count,
            complete_process_count: config
                .sample_count
                .saturating_add(config.warmup_sample_count),
            passed: distribution.passes(config.threshold),
            threshold_ms: duration_ms_ceil(config.threshold),
            distribution,
            network,
            samples_ms,
        });
    }
    aggregate_network.proof = if aggregate_network.network_syscall_probe_supported {
        "per-class strace network-syscall probes completed before warm sampling; token-free allowlist and no external network syscalls observed".to_owned()
    } else {
        "token-free allowlist and credential removal applied; network-syscall probe unavailable on this platform".to_owned()
    };
    Ok(PerformanceReport {
        schema_version: 1,
        task_id: "REL-PERF-1".to_owned(),
        final_binary_name: binary
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repo-com")
            .to_owned(),
        environment,
        sample_count_per_class: config.sample_count,
        warmup_sample_count_per_class: config.warmup_sample_count,
        threshold_ms: duration_ms_ceil(config.threshold),
        exclusions: MeasurementExclusions::default(),
        network: aggregate_network,
        classes,
        passed: true,
    }
    .with_passed_result())
}

impl PerformanceReport {
    fn with_passed_result(mut self) -> Self {
        self.passed = self.classes.iter().all(|class| class.passed);
        self
    }
}

/// Serializes a report as stable, machine-readable JSON.
pub fn report_json(report: &PerformanceReport) -> Result<String> {
    serde_json::to_string_pretty(report)
        .map_err(|_| HarnessError::new("performance report could not be serialized"))
}

#[cfg(test)]
#[path = "../tests/performance_contract.rs"]
mod performance_contract;
