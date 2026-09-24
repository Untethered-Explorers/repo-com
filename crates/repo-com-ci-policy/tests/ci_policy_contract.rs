use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    ACTIONLINT_VERSION, CARGO_AUDIT_VERSION, CARGO_DENY_VERSION, CI_WORKFLOW_PATH,
    GITLEAKS_VERSION, MINIMUM_SQLITE_VERSION, NEXTEST_VERSION, REQUIRED_CHECKS,
    REQUIRED_PLATFORM_FAMILIES, RUST_TOOLCHAIN_VERSION, scan_forbidden_secrets,
    sqlite_version_satisfies_release_floor, validate_ci_workflow, validate_deny_policy,
    validate_dependabot_policy, validate_lockfile, validate_repository, validate_toolchain,
};

#[test]
fn owned_ci_policy_files_satisfy_the_machine_contract() {
    let findings = validate_repository(&repository_root());
    assert!(
        findings.is_empty(),
        "CI policy findings:\n{}",
        findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );

    let workflow = read_owned(CI_WORKFLOW_PATH);
    for family in REQUIRED_PLATFORM_FAMILIES {
        assert!(
            workflow.contains(family),
            "missing platform family: {family}"
        );
    }
    for check in REQUIRED_CHECKS {
        assert!(!check.is_empty());
    }
    for version in [
        RUST_TOOLCHAIN_VERSION,
        NEXTEST_VERSION,
        CARGO_DENY_VERSION,
        CARGO_AUDIT_VERSION,
        ACTIONLINT_VERSION,
        GITLEAKS_VERSION,
    ] {
        assert!(
            workflow.contains(version),
            "workflow does not pin {version}"
        );
    }
    assert!(workflow.contains(MINIMUM_SQLITE_VERSION));
}

#[test]
fn missing_or_renamed_required_checks_fail_closed() {
    let workflow = read_owned(CI_WORKFLOW_PATH);
    let renamed = workflow.replace(
        "cargo fmt --all -- --check",
        "cargo fmt --all --check-with-a-different-name",
    );
    let findings = validate_ci_workflow(&renamed);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "missing-required-check"),
        "renamed format check was not rejected: {findings:?}"
    );

    let removed = workflow.replace("cargo audit --file Cargo.lock", "true");
    let findings = validate_ci_workflow(&removed);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "missing-required-check"),
        "removed audit check was not rejected: {findings:?}"
    );
}

#[test]
fn wrong_platform_and_missing_sqlite_floor_fail_closed() {
    let workflow = read_owned(CI_WORKFLOW_PATH);
    let wrong_platform = workflow.replace("windows-2022", "freebsd-14");
    let findings = validate_ci_workflow(&wrong_platform);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "missing-platform"),
        "wrong platform was not rejected: {findings:?}"
    );

    let old_sqlite = workflow.replace(MINIMUM_SQLITE_VERSION, "3.53.2");
    let findings = validate_ci_workflow(&old_sqlite);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "missing-required-check"),
        "old SQLite policy was not rejected: {findings:?}"
    );

    assert!(sqlite_version_satisfies_release_floor("3.53.4"));
    assert!(sqlite_version_satisfies_release_floor("3.53.5"));
    assert!(sqlite_version_satisfies_release_floor("4.0.0"));
    assert!(!sqlite_version_satisfies_release_floor("3.53.3"));
    assert!(!sqlite_version_satisfies_release_floor("3.52.99"));
    assert!(!sqlite_version_satisfies_release_floor("3.53"));
    assert!(!sqlite_version_satisfies_release_floor("not-a-version"));
}

#[test]
fn secret_shapes_and_real_message_content_are_rejected_without_committing_values() {
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
        authorization,
        bot_authorization,
        private_key,
        real_message.to_owned(),
    ] {
        let findings = scan_forbidden_secrets("synthetic.txt", &value);
        assert!(!findings.is_empty(), "secret-shaped fixture was accepted");
    }
}

#[test]
fn publication_updater_and_setup_mutations_are_rejected() {
    let workflow = read_owned(CI_WORKFLOW_PATH);
    for addition in [
        "      - name: Publish\n        run: cargo publish",
        "      - name: Self update\n        run: ./updater",
        "      - name: Setup mutation\n        run: ./setup --apply",
        "      - name: Release\n        run: gh release create v0.1.0",
    ] {
        let mutated = format!("{workflow}\n{addition}\n");
        let findings = validate_ci_workflow(&mutated);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "publication-or-mutation"),
            "prohibited operation was accepted: {addition}\n{findings:?}"
        );
    }
}

#[test]
fn secret_and_privilege_boundaries_reject_untrusted_workflow_changes() {
    let workflow = read_owned(CI_WORKFLOW_PATH);
    let with_secret = workflow.replace(
        "permissions:\n  contents: read",
        "permissions:\n  contents: read\n  env:\n    TOKEN: ${{ secrets.DISCORD_TOKEN }}",
    );
    let findings = validate_ci_workflow(&with_secret);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "untrusted-secret-use"),
        "secret reference was not rejected: {findings:?}"
    );

    let privileged_trigger = workflow.replace("  pull_request:", "  pull_request_target:");
    let findings = validate_ci_workflow(&privileged_trigger);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "untrusted-trigger"),
        "privileged trigger was not rejected: {findings:?}"
    );
}

#[test]
fn dependency_update_and_lockfile_policies_fail_closed_on_drift() {
    let deny = read_owned("deny.toml");
    let mut permissive = deny.clone();
    permissive = permissive.replace(
        "unknown-registry = \"deny\"",
        "unknown-registry = \"allow\"",
    );
    let findings = validate_deny_policy(&permissive);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "missing-dependency-policy"),
        "permissive source policy was not rejected: {findings:?}"
    );

    let dependabot = read_owned(".github/dependabot.yml");
    let missing_actions = dependabot.replace(
        "package-ecosystem: github-actions",
        "package-ecosystem: npm",
    );
    let findings = validate_dependabot_policy(&missing_actions);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "missing-dependabot-coverage"),
        "missing GitHub Actions updates were not rejected: {findings:?}"
    );

    let lockfile = read_owned("Cargo.lock");
    assert!(validate_lockfile(&lockfile).is_empty());
    let stale = lockfile.replace(
        "name = \"ci_policy_contract\"",
        "name = \"stale_policy_contract\"",
    );
    assert!(
        validate_lockfile(&stale)
            .iter()
            .any(|finding| finding.code == "stale-lockfile")
    );

    let toolchain = read_owned("rust-toolchain.toml");
    assert!(validate_toolchain(&toolchain).is_empty());
    let drift = toolchain.replace("1.98.1", "stable");
    assert!(
        validate_toolchain(&drift)
            .iter()
            .any(|finding| finding.code == "unpinned-toolchain")
    );
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("CI policy crate is two levels below repository root")
        .to_path_buf()
}

fn read_owned(relative: &str) -> String {
    let root = repository_root();
    fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("missing owned CI policy file {relative}: {error}"))
}
