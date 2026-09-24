use crate::{
    ColorChoice, CommandOutcome, DiagnosticsChoice, ErrorCategory, GlobalArgs, OutcomeStatus,
    OutputFormat, OutputStreams, PROTOCOL_VERSION, RepoComError, TtyMode,
};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize)]
struct SuccessPayload {
    draft_id: String,
    revision: u32,
}

fn assert_single_json_object(serialized: &str) -> Value {
    assert_eq!(
        serialized.lines().count(),
        1,
        "protocol output has extra lines"
    );
    let value: Value = serde_json::from_str(serialized).expect("protocol output must be JSON");
    let object = value
        .as_object()
        .expect("protocol output must be an object");
    assert_eq!(object.len(), 4, "protocol envelope must have four fields");
    assert!(object.contains_key("protocol_version"));
    assert!(object.contains_key("status"));
    assert!(object.contains_key("data"));
    assert!(object.contains_key("error"));
    value
}

#[test]
fn success_serializes_exactly_one_protocol_v1_object() {
    let outcome = CommandOutcome::success(SuccessPayload {
        draft_id: "draft-123".to_owned(),
        revision: 4,
    });

    let serialized = outcome.to_json().expect("success should serialize");
    let value = assert_single_json_object(&serialized);

    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(value["status"], "success");
    assert_eq!(value["data"]["draft_id"], "draft-123");
    assert_eq!(value["data"]["revision"], 4);
    assert!(value["error"].is_null());
    assert!(!serialized.contains("diagnostic"));
    assert!(!serialized.contains("prompt"));
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn every_stable_error_category_is_a_valid_failure_envelope() {
    let expected = [
        (ErrorCategory::UsageOrSchema, "usage-schema", 2),
        (
            ErrorCategory::OperatorActionRequired,
            "operator-action-required",
            3,
        ),
        (ErrorCategory::PolicyBlocked, "policy-blocked", 4),
        (ErrorCategory::Authentication, "authentication", 5),
        (ErrorCategory::Permission, "permission", 6),
        (ErrorCategory::RemoteConflict, "remote-conflict", 7),
        (ErrorCategory::UnknownDelivery, "unknown-delivery", 8),
        (ErrorCategory::StorageIntegrity, "storage-integrity", 9),
        (
            ErrorCategory::ConnectivityRateLimit,
            "connectivity-rate-limit",
            10,
        ),
        (ErrorCategory::InternalFailure, "internal-failure", 1),
    ];

    assert_eq!(expected.len(), ErrorCategory::ALL.len());

    for (category, code, exit_code) in expected {
        let outcome = CommandOutcome::<()>::failure(RepoComError::new(category, "safe detail"));
        let serialized = outcome.to_json().expect("failure should serialize");
        let value = assert_single_json_object(&serialized);

        assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(value["status"], "error");
        assert!(value["data"].is_null());
        assert_eq!(value["error"]["code"], code);
        assert_eq!(value["error"]["message"], "safe detail");
        assert_eq!(outcome.exit_code(), exit_code);
        assert_eq!(ErrorCategory::from_code(code), Some(category));
    }
}

#[test]
fn non_tty_fails_closed_and_streams_never_mix_diagnostics() {
    let non_tty = TtyMode::from_stream_states(false, true);
    assert_eq!(non_tty, TtyMode::NonTty);
    assert!(!non_tty.can_prompt());
    assert!(!non_tty.allows_prompt());

    let error = non_tty
        .require_prompt_allowed()
        .expect_err("non-TTY prompt requests must fail closed");
    assert_eq!(error.category(), ErrorCategory::OperatorActionRequired);
    assert_eq!(error.exit_code(), 3);

    let outcome = CommandOutcome::success("ok");
    let streams: OutputStreams = outcome
        .output_streams(Some("diagnostic: stderr only".to_owned()))
        .expect("stream values should serialize");
    let value = assert_single_json_object(streams.stdout());

    assert_eq!(value["data"], "ok");
    assert_eq!(streams.stderr(), "diagnostic: stderr only");
    assert!(!streams.stdout().contains("diagnostic"));
    assert!(!streams.stdout().contains("prompt"));
    assert!(!streams.stdout().contains('\n'));

    assert_eq!(TtyMode::from_stream_states(true, true), TtyMode::Tty);
    assert!(TtyMode::Tty.can_prompt());
    assert!(TtyMode::Tty.require_prompt_allowed().is_ok());
}

#[test]
fn global_arguments_are_typed_and_default_to_human_output() {
    let defaults = GlobalArgs::default();
    assert!(!defaults.is_machine_mode());
    assert!(!defaults.diagnostics_enabled());
    assert_eq!(defaults.output_format, OutputFormat::Human);

    let machine = GlobalArgs {
        config_path: Some("config/repo-com.toml".into()),
        output_format: OutputFormat::Json,
        color: ColorChoice::Never,
        diagnostics: DiagnosticsChoice::On,
    };
    assert!(machine.is_machine_mode());
    assert!(machine.diagnostics_enabled());
    assert_eq!(
        machine.config_path.as_deref(),
        Some(Path::new("config/repo-com.toml"))
    );
}

fn workspace_file(relative: &str) -> String {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("foundation crate must be nested under crates/");
    fs::read_to_string(workspace_root.join(relative))
        .unwrap_or_else(|error| panic!("missing workspace file {relative}: {error}"))
}

#[test]
fn toolchain_workspace_and_lockfile_are_reproducible() {
    let toolchain = workspace_file("rust-toolchain.toml");
    assert!(toolchain.contains("channel = \"1.98.1\""));
    assert!(toolchain.contains("components = [\"rustfmt\", \"clippy\"]"));

    let root_manifest = workspace_file("Cargo.toml");
    assert!(root_manifest.contains("members = [\"crates/*\"]"));
    assert!(root_manifest.contains("resolver = \"3\""));
    assert!(root_manifest.contains("edition = \"2024\""));
    assert!(root_manifest.contains("rust-version = \"1.98.1\""));
    assert!(root_manifest.contains("[workspace.dependencies]"));
    assert!(root_manifest.contains("clap = { version = \"=4.6.7\""));
    assert!(root_manifest.contains("serde = { version = \"=1.0.229\""));
    assert!(root_manifest.contains("serde_json = \"=1.0.151\""));

    let nextest_config = workspace_file(".config/nextest.toml");
    assert!(nextest_config.contains("nextest-version = \"0.9.146\""));
    assert!(nextest_config.contains("--no-tests fail"));
    assert!(nextest_config.contains("binary_id(foundation_contract)"));

    let crate_manifest = workspace_file("crates/repo-com-foundation/Cargo.toml");
    assert!(crate_manifest.contains("name = \"foundation_contract\""));
    assert!(crate_manifest.contains("name = \"repo_com_foundation\""));
    assert!(crate_manifest.contains("edition.workspace = true"));
    assert!(crate_manifest.contains("rust-version.workspace = true"));
    assert!(crate_manifest.contains("serde.workspace = true"));
    assert!(crate_manifest.contains("serde_json.workspace = true"));

    let lockfile = workspace_file("Cargo.lock");
    assert!(lockfile.contains("name = \"foundation_contract\""));
    assert!(lockfile.contains("name = \"clap\""));
    assert!(lockfile.contains("version = \"4.6.7\""));
    assert!(lockfile.contains("name = \"serde\""));
    assert!(lockfile.contains("version = \"1.0.229\""));
    assert!(lockfile.contains("name = \"serde_json\""));
    assert!(lockfile.contains("version = \"1.0.151\""));
}

#[test]
fn protocol_constructors_reject_inconsistent_parts() {
    let invalid_success =
        CommandOutcome::<u8>::try_new(PROTOCOL_VERSION, OutcomeStatus::Success, None, None);
    assert!(invalid_success.is_err());

    let invalid_error = CommandOutcome::<u8>::try_new(
        PROTOCOL_VERSION,
        OutcomeStatus::Error,
        Some(1),
        Some(RepoComError::internal_failure("failure")),
    );
    assert!(invalid_error.is_err());

    let wrong_version = CommandOutcome::<u8>::try_new(2, OutcomeStatus::Success, Some(1), None);
    assert!(wrong_version.is_err());
}
