use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    ACTIONLINT_VERSION, CARGO_AUDIT_VERSION, CARGO_CYCLONEDX_VERSION, CARGO_DENY_VERSION,
    CARGO_DIST_VERSION, DIST_WORKSPACE_PATH, FINAL_BINARY_NAME, FINAL_PACKAGE_NAME,
    GITLEAKS_VERSION, MINIMUM_SQLITE_VERSION, RELEASE_WORKFLOW_PATH, REQUIRED_GATES,
    REQUIRED_TARGETS, scan_forbidden_secrets, sqlite_version_satisfies_release_floor,
    validate_final_binary_manifest, validate_lockfile, validate_release_plan,
    validate_release_workflow, validate_repository, validate_sqlite_release_observation,
    validate_sqlite_source, validate_workspace_manifest,
};

#[test]
fn owned_release_policy_files_satisfy_the_machine_contract() {
    let findings = validate_repository(&repository_root());
    assert!(
        findings.is_empty(),
        "release policy findings:\n{}",
        findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );

    let plan = read_owned(DIST_WORKSPACE_PATH);
    let workflow = read_owned(RELEASE_WORKFLOW_PATH);
    for target in REQUIRED_TARGETS {
        assert!(plan.contains(target), "missing release target: {target}");
    }
    for version in [
        CARGO_DIST_VERSION,
        CARGO_CYCLONEDX_VERSION,
        CARGO_DENY_VERSION,
        CARGO_AUDIT_VERSION,
        ACTIONLINT_VERSION,
        GITLEAKS_VERSION,
        MINIMUM_SQLITE_VERSION,
    ] {
        assert!(
            workflow.contains(version),
            "workflow does not pin {version}"
        );
    }
    for gate in REQUIRED_GATES {
        assert!(!gate.is_empty());
    }
    assert!(plan.contains(FINAL_PACKAGE_NAME));
    assert!(workflow.contains(FINAL_BINARY_NAME));
}

#[test]
fn generated_release_plan_drift_fails_closed() {
    let plan = read_owned(DIST_WORKSPACE_PATH);
    for (old, new) in [
        (
            "cargo-dist-version = \"0.32.0\"",
            "cargo-dist-version = \"0.31.0\"",
        ),
        ("source-tarball = true", "source-tarball = false"),
        ("checksum = \"sha256\"", "checksum = \"md5\""),
        ("cargo-cyclonedx = true", "cargo-cyclonedx = false"),
        ("dependency-licenses.txt", "missing-license-evidence.txt"),
        ("3.53.4", "3.53.2"),
        ("install-updater = false", "install-updater = true"),
        ("x86_64-pc-windows-msvc", "x86_64-unknown-freebsd"),
    ] {
        let drifted = plan.replace(old, new);
        let findings = validate_release_plan(&drifted);
        assert!(
            !findings.is_empty(),
            "release-plan drift was accepted: {old} -> {new}"
        );
    }
}

#[test]
fn required_targets_version_and_installer_boundaries_are_explicit() {
    let plan = read_owned(DIST_WORKSPACE_PATH);
    let wrong_target = plan.replace("x86_64-pc-windows-msvc", "x86_64-unknown-freebsd");
    assert!(
        validate_release_plan(&wrong_target)
            .iter()
            .any(|finding| finding.code == "missing-release-target")
    );

    let weak_checksum = plan.replace("checksum = \"sha256\"", "checksum = \"false\"");
    assert!(
        validate_release_plan(&weak_checksum)
            .iter()
            .any(|finding| finding.code == "weak-checksum")
    );

    let implicit_installer = plan.replace("installers = []", "installers = [\"shell\"]");
    assert!(
        validate_release_plan(&implicit_installer)
            .iter()
            .any(|finding| finding.code == "implicit-installation")
    );

    let invalid_version = plan.replace("version = \"0.1.0\"", "version = \"release\"");
    assert!(
        validate_release_plan(&invalid_version)
            .iter()
            .any(|finding| finding.code == "invalid-semantic-version")
    );

    let publication_config = plan.replace("ci = []", "ci = [\"github\"]");
    assert!(
        validate_release_plan(&publication_config)
            .iter()
            .any(|finding| finding.code == "release-plan-drift")
    );
}

#[test]
fn old_or_dynamic_sqlite_release_observations_fail_closed() {
    assert!(validate_sqlite_release_observation("3.53.4", "static").is_empty());
    assert!(validate_sqlite_release_observation("3.54.0", "static sqlite").is_empty());
    assert!(
        validate_sqlite_release_observation("3.53.3", "static")
            .iter()
            .any(|finding| finding.code == "unsupported-sqlite-runtime")
    );
    assert!(
        validate_sqlite_release_observation("3.53.4", "dynamic system sqlite")
            .iter()
            .any(|finding| finding.code == "dynamic-sqlite-linkage")
    );
    assert!(sqlite_version_satisfies_release_floor("3.53.4"));
    assert!(sqlite_version_satisfies_release_floor("3.53.5"));
    assert!(sqlite_version_satisfies_release_floor("4.0.0"));
    assert!(!sqlite_version_satisfies_release_floor("3.53.2"));
    assert!(!sqlite_version_satisfies_release_floor("3.53"));
    assert!(!sqlite_version_satisfies_release_floor("not-a-version"));

    let source = read_owned("crates/repo-com-state/src/store.rs");
    assert!(validate_sqlite_source(&source).is_empty());
}

#[test]
fn workflow_requires_validated_evidence_and_rejects_unsafe_changes() {
    let workflow = read_owned(RELEASE_WORKFLOW_PATH);
    let removed_dependency = workflow.replace("      - evidence\n", "");
    assert!(
        validate_release_workflow(&removed_dependency)
            .iter()
            .any(|finding| finding.code == "missing-evidence-dependency")
    );

    let unpinned_action = workflow.replace(
        "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683",
        "actions/checkout@v4",
    );
    assert!(
        validate_release_workflow(&unpinned_action)
            .iter()
            .any(|finding| finding.code == "unpinned-release-action")
    );

    let overprivileged = workflow.replacen("contents: read", "contents: write", 1);
    assert!(
        validate_release_workflow(&overprivileged)
            .iter()
            .any(|finding| finding.code == "overprivileged-release-permissions")
    );

    for addition in [
        "      - name: Unsafe publication\n        run: gh release create v0.1.0",
        "      - name: Updater\n        run: ./repo-com-update",
        "      - name: Secret\n        run: echo \"${{ secrets.RELEASE_TOKEN }}\"",
        "      - name: Mutation\n        run: ./setup --apply",
    ] {
        let mutated = format!("{workflow}\n{addition}\n");
        let findings = validate_release_workflow(&mutated);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "publication-or-secret-risk"
                    || finding.code == "updater-or-mutation-risk"),
            "unsafe workflow addition was accepted: {addition}\n{findings:?}"
        );
    }
}

#[test]
fn secret_shapes_and_real_message_content_are_rejected_without_values() {
    let token = [
        "A".repeat(24),
        format!("b{}", "=".repeat(4)),
        "c".repeat(27),
    ]
    .join(".");
    let authorization = format!("Authorization: {}", "x".repeat(24));
    let bot_authorization = format!("Authorization: Bot {}", "y".repeat(24));
    let private_key = format!("-----BEGIN {}-----", "PRIVATE KEY");
    let real_message = "real team message: payload";

    for value in [
        token,
        authorization.clone(),
        bot_authorization,
        private_key,
        real_message.to_owned(),
    ] {
        let findings = scan_forbidden_secrets("synthetic.txt", &value);
        assert!(!findings.is_empty(), "secret-shaped fixture was accepted");
    }

    for surface in ["dist-manifest.json", "sha256.sum", "bom.json"] {
        let findings = scan_forbidden_secrets(surface, &authorization);
        assert!(
            !findings.is_empty(),
            "generated surface scan missed {surface}"
        );
    }
}

#[test]
fn workspace_final_binary_and_lockfile_drift_fail_closed() {
    let manifest = read_owned("Cargo.toml");
    assert!(validate_workspace_manifest(&manifest).is_empty());
    let bundled = manifest.replace(
        "default-features = false",
        "default-features = false, features = [\"bundled\"]",
    );
    assert!(
        validate_workspace_manifest(&bundled)
            .iter()
            .any(|finding| finding.code == "bundled-sqlite-source")
    );

    let final_manifest = read_owned("crates/repo-com-cli/Cargo.toml");
    assert!(validate_final_binary_manifest(&final_manifest).is_empty());
    let renamed = final_manifest.replace("name = \"repo-com\"", "name = \"other\"");
    assert!(
        validate_final_binary_manifest(&renamed)
            .iter()
            .any(|finding| finding.code == "final-binary-drift")
    );

    let lockfile = read_owned("Cargo.lock");
    assert!(validate_lockfile(&lockfile).is_empty());
    let stale = lockfile.replace(
        "name = \"release_policy_contract\"",
        "name = \"stale_release_policy_contract\"",
    );
    assert!(
        validate_lockfile(&stale)
            .iter()
            .any(|finding| finding.code == "stale-release-lockfile")
    );
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("release policy crate is two levels below repository root")
        .to_path_buf()
}

fn read_owned(relative: &str) -> String {
    let path = repository_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing owned release policy file {relative}: {error}"))
}
