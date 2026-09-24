use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

fn binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join(format!("repo-com{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str], input: &str, cwd: &Path) -> Output {
    let mut command = Command::new(binary());
    command.args(args).current_dir(cwd);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("repo-com binary starts");
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(input.as_bytes())
            .expect("structured input writes");
    }
    child
        .wait_with_output()
        .expect("repo-com process completes")
}

fn temporary_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "repo-com-command-routing-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(path.join(".git")).expect("temporary repository root");
    path
}

fn write_config(root: &Path) -> PathBuf {
    let path = root.join(".repo-com.toml");
    fs::write(
        &path,
        r#"schema_version = 1
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
"#,
    )
    .expect("configuration writes");
    path
}

fn envelope(command: &str, input: Value) -> String {
    serde_json::to_string(&json!({
        "protocol_version": 1,
        "command": command,
        "input": input,
    }))
    .expect("envelope serializes")
}

#[test]
fn version_prints_only_the_semantic_package_version() {
    let output = run(&["--version"], "", Path::new("."));
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn help_exposes_every_command_group_and_global_option() {
    let output = run(&["--help"], "", Path::new("."));
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "config",
        "policy",
        "draft",
        "send",
        "inbox",
        "reply",
        "audit",
        "state",
        "purge",
        "--config",
        "--output",
        "--color",
        "--diagnostics",
    ] {
        assert!(help.contains(expected), "help omitted {expected}: {help}");
    }
    assert!(output.stderr.is_empty());
}

#[test]
fn every_group_has_a_routed_help_path_and_missing_ids_fail_closed() {
    for (group, action) in [
        ("config", "validate"),
        ("policy", "status"),
        ("policy", "activate"),
        ("draft", "create"),
        ("draft", "show"),
        ("draft", "update"),
        ("draft", "preview"),
        ("draft", "approve"),
        ("draft", "secret-override"),
        ("send", "dispatch"),
        ("setup", "check"),
        ("inbox", "fetch"),
        ("inbox", "acknowledge"),
        ("inbox", "archive"),
        ("reply", "draft-create"),
        ("audit", "query"),
        ("state", "verify"),
        ("state", "inspect"),
        ("purge", "plan"),
        ("purge", "execute"),
    ] {
        let help = run(&[group, action, "--help"], "", Path::new("."));
        assert!(help.status.success(), "{group} {action} help failed");
    }
    let setup_help = run(&["setup-check", "--help"], "", Path::new("."));
    assert!(setup_help.status.success());
    let missing_group_action = run(&["--output", "json", "draft"], "", Path::new("."));
    assert_eq!(missing_group_action.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&missing_group_action.stdout)
        .expect("one JSON object for missing group action");
    assert_eq!(value["error"]["code"], "usage-schema");

    let output = run(
        &["--output", "json", "draft", "show"],
        &envelope("draft.show", json!({})),
        Path::new("."),
    );
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).expect("one JSON object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "usage-schema");
    assert!(output.stderr.is_empty());
}

#[test]
fn machine_mode_separates_one_json_object_from_opt_in_diagnostics() {
    let root = temporary_root();
    let config = write_config(&root);
    let state = root.join("state.sqlite3");
    let output = run(
        &[
            "--config",
            config.to_str().expect("UTF-8 config path"),
            "--state",
            state.to_str().expect("UTF-8 state path"),
            "--output",
            "json",
            "--diagnostics",
            "on",
            "config",
            "validate",
        ],
        &envelope(
            "config.validate",
            json!({ "repository_id": "acme/widgets" }),
        ),
        &root,
    );
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("exactly one JSON object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["status"], "success");
    assert_eq!(value["data"]["view"], "config");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("diagnostics"));
    assert!(!stderr.contains("acme/widgets"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parser_errors_keep_diagnostics_off_machine_stdout() {
    let output = run(
        &["--output", "json", "--diagnostics", "on", "not-a-command"],
        "",
        Path::new("."),
    );
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).expect("one JSON object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["error"]["code"], "usage-schema");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("diagnostics"));
    assert!(stderr.contains("diagnostics"));
}

#[test]
fn draft_create_and_show_round_trip_preserves_the_exact_revision() {
    let root = temporary_root();
    let config = write_config(&root);
    let state = root.join("state.sqlite3");
    let common = [
        "--config",
        config.to_str().expect("UTF-8 config path"),
        "--state",
        state.to_str().expect("UTF-8 state path"),
        "--output",
        "json",
    ];
    let created = run(
        &[
            common[0], common[1], common[2], common[3], common[4], common[5], "draft", "create",
        ],
        &envelope(
            "draft.create",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-round-trip",
                "destination_alias": "release",
                "text": "build failed",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "2026-01-01T00:00:00Z",
                "created_at_unix_seconds": 1767225600,
            }),
        ),
        &root,
    );
    assert!(created.status.success());
    let created_value: Value =
        serde_json::from_slice(&created.stdout).expect("one create JSON object");
    let created_hash = created_value["data"]["revision_hash"]
        .as_str()
        .expect("create revision hash");

    let shown = run(
        &[
            common[0], common[1], common[2], common[3], common[4], common[5], "draft", "show",
        ],
        &envelope(
            "draft.show",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-round-trip",
                "revision": 1,
            }),
        ),
        &root,
    );
    assert!(shown.status.success());
    let shown_value: Value = serde_json::from_slice(&shown.stdout).expect("one show JSON object");
    assert_eq!(shown_value["data"]["revision_hash"], created_hash);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn draft_update_preserves_each_revision_metadata_snapshot() {
    let root = temporary_root();
    let config = write_config(&root);
    let state = root.join("state.sqlite3");
    let common = [
        "--config",
        config.to_str().expect("UTF-8 config path"),
        "--state",
        state.to_str().expect("UTF-8 state path"),
        "--output",
        "json",
    ];
    let created = run(
        &[
            common[0], common[1], common[2], common[3], common[4], common[5], "draft", "create",
        ],
        &envelope(
            "draft.create",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-metadata",
                "destination_alias": "release",
                "text": "old body",
                "event_type": "build_failed",
                "severity": "high",
                "metadata": {
                    "repository_label": "repo",
                    "branch": "main",
                    "commit": "old"
                },
                "created_at": "2026-01-01T00:00:00Z",
                "created_at_unix_seconds": 1767225600,
            }),
        ),
        &root,
    );
    assert!(created.status.success());

    let updated = run(
        &[
            common[0], common[1], common[2], common[3], common[4], common[5], "draft", "update",
        ],
        &envelope(
            "draft.update",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-metadata",
                "revision": 1,
                "destination_alias": "release",
                "text": "new body",
                "event_type": "build_fixed",
                "severity": "normal",
                "metadata": {
                    "repository_label": "repo",
                    "branch": "release",
                    "commit": "new"
                },
                "created_at": "2026-01-02T00:00:00Z",
                "created_at_unix_seconds": 1767312000,
            }),
        ),
        &root,
    );
    assert!(updated.status.success());
    let updated_value: Value =
        serde_json::from_slice(&updated.stdout).expect("one update JSON object");
    assert_eq!(updated_value["data"]["revision"], 2);

    let old = run(
        &[
            common[0], common[1], common[2], common[3], common[4], common[5], "draft", "show",
        ],
        &envelope(
            "draft.show",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-metadata",
                "revision": 1,
            }),
        ),
        &root,
    );
    assert!(old.status.success());
    let old_value: Value = serde_json::from_slice(&old.stdout).expect("one old revision object");
    assert_eq!(old_value["data"]["event_type"], "build_failed");
    assert_eq!(old_value["data"]["severity"], "high");
    assert_eq!(old_value["data"]["metadata"]["branch"], "main");
    assert_eq!(old_value["data"]["metadata"]["commit"], "old");

    let new = run(
        &[
            common[0], common[1], common[2], common[3], common[4], common[5], "draft", "show",
        ],
        &envelope(
            "draft.show",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-metadata",
                "revision": 2,
            }),
        ),
        &root,
    );
    assert!(new.status.success());
    let new_value: Value = serde_json::from_slice(&new.stdout).expect("one new revision object");
    assert_eq!(new_value["data"]["event_type"], "build_fixed");
    assert_eq!(new_value["data"]["severity"], "normal");
    assert_eq!(new_value["data"]["metadata"]["branch"], "release");
    assert_eq!(new_value["data"]["metadata"]["commit"], "new");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn non_tty_operator_action_returns_operator_action_required_without_a_prompt() {
    let root = temporary_root();
    let config = write_config(&root);
    let state = root.join("state.sqlite3");
    let output = run(
        &[
            "--config",
            config.to_str().expect("UTF-8 config path"),
            "--state",
            state.to_str().expect("UTF-8 state path"),
            "--output",
            "json",
            "draft",
            "approve",
        ],
        &envelope(
            "draft.approve",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-1",
                "revision": 1,
            }),
        ),
        &root,
    );
    assert_eq!(output.status.code(), Some(3));
    let value: Value = serde_json::from_slice(&output.stdout).expect("one protocol object");
    assert_eq!(value["error"]["code"], "operator-action-required");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("keyboard"));
    assert!(!stdout.contains("Y/yes"));
    assert!(output.stderr.is_empty());
    assert!(
        !state.exists(),
        "non-TTY approval must not create local state"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn protocol_command_must_match_the_selected_route_and_unknown_fields_fail() {
    let mismatched = run(
        &["--output", "json", "draft", "show"],
        &envelope(
            "draft.preview",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-1",
                "revision": 1,
            }),
        ),
        Path::new("."),
    );
    assert_eq!(mismatched.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&mismatched.stdout).expect("one JSON object");
    assert_eq!(value["error"]["code"], "usage-schema");

    let unknown = run(
        &["--output", "json", "draft", "show"],
        &envelope(
            "draft.show",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-1",
                "revision": 1,
                "destination_alias": "release",
            }),
        ),
        Path::new("."),
    );
    assert_eq!(unknown.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&unknown.stdout).expect("one JSON object");
    assert_eq!(value["error"]["code"], "usage-schema");

    let unknown_envelope = json!({
        "protocol_version": 1,
        "command": "draft.show",
        "input": {
            "repository_id": "acme/widgets",
            "draft_id": "draft-1",
            "revision": 1,
        },
        "extra": true,
    })
    .to_string();
    let unknown_envelope = run(
        &["--output", "json", "draft", "show"],
        &unknown_envelope,
        Path::new("."),
    );
    assert_eq!(unknown_envelope.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&unknown_envelope.stdout).expect("one JSON object");
    assert_eq!(value["error"]["code"], "usage-schema");
}

#[test]
fn no_destination_flag_or_default_destination_is_added_by_the_router() {
    let output = run(
        &["--output", "json", "draft", "create"],
        &envelope(
            "draft.create",
            json!({
                "repository_id": "acme/widgets",
                "draft_id": "draft-1",
                "text": "body",
                "event_type": "build_failed",
                "severity": "high",
                "created_at": "1970-01-01T00:16:40Z",
                "created_at_unix_seconds": 1000,
            }),
        ),
        Path::new("."),
    );
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).expect("one JSON object");
    assert_eq!(value["error"]["code"], "usage-schema");
    let help = run(&["draft", "create", "--help"], "", Path::new("."));
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(!help.contains("--destination"));
    assert!(!help.contains("--channel"));
}
