#![forbid(unsafe_code)]
#![doc = "Machine-checkable policy for token-free, versioned repo-com release packaging."]

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "../tests/release_policy_contract.rs"]
mod release_policy_contract;

/// Repository-relative path of the distribution plan owned by this task.
pub const DIST_WORKSPACE_PATH: &str = "dist-workspace.toml";
/// Repository-relative path of the release workflow owned by this task.
pub const RELEASE_WORKFLOW_PATH: &str = ".github/workflows/release.yml";
/// Repository-relative path of the committed lockfile.
pub const LOCKFILE_PATH: &str = "Cargo.lock";
/// Repository-relative path of the workspace manifest.
pub const WORKSPACE_MANIFEST_PATH: &str = "Cargo.toml";
/// Repository-relative path of the final executable manifest.
pub const FINAL_BINARY_MANIFEST_PATH: &str = "crates/repo-com-cli/Cargo.toml";
/// Repository-relative path of the state runtime boundary.
pub const SQLITE_SOURCE_PATH: &str = "crates/repo-com-state/src/store.rs";

/// The final Cargo package and installed executable names.
pub const FINAL_PACKAGE_NAME: &str = "command_routing_contract";
/// The globally installed executable name.
pub const FINAL_BINARY_NAME: &str = "repo-com";
/// The semantic package version used by the initial release plan.
pub const PACKAGE_VERSION: &str = "0.1.0";
/// The pinned cargo-dist release-planning tool.
pub const CARGO_DIST_VERSION: &str = "0.32.0";
/// The pinned CycloneDX SBOM generator.
pub const CARGO_CYCLONEDX_VERSION: &str = "0.5.9";
/// The pinned CI policy dependency checker reused by release evidence.
pub const CARGO_DENY_VERSION: &str = "0.20.2";
/// The pinned CI RustSec checker reused by release evidence.
pub const CARGO_AUDIT_VERSION: &str = "0.22.2";
/// The pinned workflow validator.
pub const ACTIONLINT_VERSION: &str = "1.7.12";
/// The pinned generated-evidence secret scanner.
pub const GITLEAKS_VERSION: &str = "8.30.1";
/// The minimum SQLite version accepted by a release artifact.
pub const MINIMUM_SQLITE_VERSION: &str = "3.53.4";

/// Native release targets required by the packaging plan.
pub const REQUIRED_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

/// Stable names for every required release gate.
pub const REQUIRED_GATES: &[&str] = &[
    "ci-policy",
    "performance-budget",
    "release-policy",
    "release-plan",
    "actionlint",
    "cargo-dist",
    "cargo-cyclonedx",
    "license-evidence",
    "strong-checksums",
    "source-archive",
    "static-sqlite",
    "secret-scan",
];

/// A safe, redacted release-policy finding. Findings never include an input value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyFinding {
    /// Repository-relative path that produced the finding.
    pub path: String,
    /// Stable machine-readable finding category.
    pub code: &'static str,
    /// Safe explanation without source contents, credentials, or message text.
    pub message: String,
}

impl fmt::Display for PolicyFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}: {}", self.path, self.code, self.message)
    }
}

impl std::error::Error for PolicyFinding {}

/// Validates all repository files whose contents are part of release policy.
pub fn validate_repository(root: &Path) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    validate_file(
        root,
        DIST_WORKSPACE_PATH,
        validate_release_plan,
        &mut findings,
    );
    validate_file(
        root,
        RELEASE_WORKFLOW_PATH,
        validate_release_workflow,
        &mut findings,
    );
    validate_file(root, LOCKFILE_PATH, validate_lockfile, &mut findings);
    validate_file(
        root,
        WORKSPACE_MANIFEST_PATH,
        validate_workspace_manifest,
        &mut findings,
    );
    validate_file(
        root,
        FINAL_BINARY_MANIFEST_PATH,
        validate_final_binary_manifest,
        &mut findings,
    );
    validate_file(
        root,
        SQLITE_SOURCE_PATH,
        validate_sqlite_source,
        &mut findings,
    );
    scan_known_generated_surfaces(root, &mut findings);
    findings
}

/// Validates the cargo-dist workspace plan and its packaging safety boundary.
pub fn validate_release_plan(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    let active = active_text(contents);
    let lower = active.to_ascii_lowercase();

    if active.trim().is_empty() {
        add_finding(
            &mut findings,
            DIST_WORKSPACE_PATH,
            "empty-release-plan",
            "the release plan is empty",
        );
        return findings;
    }

    if !contents.contains(MINIMUM_SQLITE_VERSION) {
        add_finding(
            &mut findings,
            DIST_WORKSPACE_PATH,
            "missing-sqlite-release-floor",
            "the release plan does not record the patched SQLite release floor",
        );
    }

    for marker in [
        "[workspace]",
        "members = [\"cargo:.\"]",
        "[dist]",
        "dist = true",
        "cargo-dist-version = \"0.32.0\"",
        "version = \"0.1.0\"",
        "packages = [\"command_routing_contract\"]",
        "installers = []",
        "source-tarball = true",
        "cargo-cyclonedx = true",
        "msvc-crt-static = true",
        "install-updater = false",
        "always-use-latest-updater = false",
        "ci = []",
        "hosting = []",
        "precise-builds = true",
        "fail-fast = true",
        "dependency-licenses.txt",
    ] {
        if !lower.contains(&marker.to_ascii_lowercase()) {
            add_finding(
                &mut findings,
                DIST_WORKSPACE_PATH,
                "release-plan-drift",
                format!("the release plan is missing or changed required field: {marker}"),
            );
        }
    }

    for target in REQUIRED_TARGETS {
        if !lower.contains(&format!("\"{target}\"")) {
            add_finding(
                &mut findings,
                DIST_WORKSPACE_PATH,
                "missing-release-target",
                format!("the release plan is missing required target: {target}"),
            );
        }
    }

    if !lower.contains("installers = []") {
        add_finding(
            &mut findings,
            DIST_WORKSPACE_PATH,
            "implicit-installation",
            "the release plan must leave installation as an explicit archive operation",
        );
    }

    if !has_strong_checksum(&active) {
        add_finding(
            &mut findings,
            DIST_WORKSPACE_PATH,
            "weak-checksum",
            "the release plan must use a SHA-256 or stronger checksum algorithm",
        );
    }

    for forbidden in [
        "install-updater = true",
        "always-use-latest-updater = true",
        "updater = true",
        "selfupdate",
        "self-update",
        "auto-update",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                &mut findings,
                DIST_WORKSPACE_PATH,
                "updater-enabled",
                "the release plan must not enable any updater",
            );
            break;
        }
    }

    if lower.contains("md5") || lower.contains("sha1") || lower.contains("checksum = false") {
        add_finding(
            &mut findings,
            DIST_WORKSPACE_PATH,
            "weak-checksum",
            "the release plan contains a checksum algorithm weaker than SHA-256",
        );
    }

    if !semantic_version(assignment_value(&active, "version").unwrap_or_default()) {
        add_finding(
            &mut findings,
            DIST_WORKSPACE_PATH,
            "invalid-semantic-version",
            "the release plan version must be a semantic major.minor.patch value",
        );
    }

    for finding in scan_forbidden_secrets(DIST_WORKSPACE_PATH, contents) {
        findings.push(finding);
    }
    findings
}

/// Validates the tag-driven release workflow and all of its evidence boundaries.
pub fn validate_release_workflow(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    let active = active_text(contents);
    let lower = active.to_ascii_lowercase();

    if active.trim().is_empty() {
        add_finding(
            &mut findings,
            RELEASE_WORKFLOW_PATH,
            "empty-release-workflow",
            "the release workflow is empty",
        );
        return findings;
    }

    for marker in [
        "on:",
        "push:",
        "tags:",
        "[0-9]+.[0-9]+.[0-9]+*",
        "permissions:",
        "contents: read",
        "concurrency:",
        "cancel-in-progress: false",
        "cargo fmt --all -- --check",
        "cargo clippy --workspace --all-targets --all-features -- -d warnings",
        "binary_id(ci_policy_contract)",
        "binary_id(performance_contract)",
        "binary_id(release_policy_contract)",
        "actionlint .github/workflows/release.yml",
        "cargo install --locked cargo-dist --version",
        "cargo install --locked cargo-cyclonedx --version",
        "cargo install --locked cargo-deny --version",
        "cargo install --locked cargo-audit --version",
        "go install github.com/rhysd/actionlint/cmd/actionlint@",
        "go install github.com/zricethezav/gitleaks/v8@",
        "CARGO_DIST_VERSION: \"0.32.0\"",
        "CARGO_CYCLONEDX_VERSION: \"0.5.9\"",
        "CARGO_DENY_VERSION: \"0.20.2\"",
        "CARGO_AUDIT_VERSION: \"0.22.2\"",
        "ACTIONLINT_VERSION: \"1.7.12\"",
        "GITLEAKS_VERSION: \"8.30.1\"",
        "dist plan",
        "--output-format=json",
        "--no-local-paths",
        "dist build",
        "--artifacts=local",
        "--artifacts=global",
        "cargo cyclonedx",
        "--format json",
        "--target all",
        "cargo deny check licenses",
        "sha256sum",
        "shasum -a 256",
        "source.tar",
        "dist-manifest.json",
        "git diff --exit-code -- dist-workspace.toml",
        "dependency-licenses.txt",
        "bom",
        "gitleaks dir",
        "INSTALLATION_MODE: \"explicit-archive\"",
        "SQLITE3_STATIC: \"1\"",
        "SQLITE3_LIB_DIR:",
        "SQLITE3_INCLUDE_DIR:",
        "assert_sqlite_runtime",
        "REQUIRED_SQLITE_VERSION",
        "repo-com-release.sqlite3",
        "draft create",
        "ldd",
        "otool",
        "objdump",
        "dumpbin",
    ] {
        if !lower.contains(&marker.to_ascii_lowercase()) {
            add_finding(
                &mut findings,
                RELEASE_WORKFLOW_PATH,
                "missing-release-gate",
                format!("the release workflow is missing or renamed required gate: {marker}"),
            );
        }
    }

    for marker in [
        "evidence:",
        "performance:",
        "plan:",
        "build-local-artifacts:",
        "build-global-artifacts:",
        "artifact-safety:",
    ] {
        if !lower.contains(marker) {
            add_finding(
                &mut findings,
                RELEASE_WORKFLOW_PATH,
                "missing-evidence-job",
                format!("the release workflow is missing required job: {marker}"),
            );
        }
    }

    if !has_dependency_chain(&lower, "plan:", &["evidence", "performance"])
        || !has_dependency_chain(&lower, "build-local-artifacts:", &["plan"])
        || !has_dependency_chain(
            &lower,
            "build-global-artifacts:",
            &["build-local-artifacts"],
        )
        || !has_dependency_chain(&lower, "artifact-safety:", &["build-global-artifacts"])
    {
        add_finding(
            &mut findings,
            RELEASE_WORKFLOW_PATH,
            "missing-evidence-dependency",
            "release jobs must depend on validated CI, performance, plan, and artifact evidence",
        );
    }

    for forbidden in [
        "pull_request:",
        "pull_request_target:",
        "workflow_run:",
        "repository_dispatch:",
        "schedule:",
        "workflow_dispatch:",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                &mut findings,
                RELEASE_WORKFLOW_PATH,
                "non-tag-release-trigger",
                "release execution must be tag-driven only",
            );
            break;
        }
    }

    for forbidden in [
        "secrets.",
        "${{ secrets",
        "github.token",
        "github_token",
        "authorization:",
        "authorization=",
        "private key",
        "gh release",
        "softprops/action-gh-release",
        "cargo publish",
        "npm publish",
        "git push",
        "git tag",
        "dist host",
        "--steps=release",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                &mut findings,
                RELEASE_WORKFLOW_PATH,
                "publication-or-secret-risk",
                "the release workflow must not publish, embed credentials, or invoke a privileged host step",
            );
            break;
        }
    }

    for forbidden in [
        "selfupdate",
        "self-update",
        "auto-update",
        "updater = true",
        "install-updater = true",
        "repo-com-update",
        "setup --apply",
        "./setup",
        "setup mutation",
        "setup-mutation",
        "mutate discord",
        "discord mutation",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                &mut findings,
                RELEASE_WORKFLOW_PATH,
                "updater-or-mutation-risk",
                "the release workflow must not contain updater or setup mutation behavior",
            );
            break;
        }
    }

    for forbidden in [
        "write-all",
        "read-all",
        "contents: write",
        "id-token: write",
        "packages: write",
        "pull-requests: write",
        "security-events: write",
        "actions: write",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                &mut findings,
                RELEASE_WORKFLOW_PATH,
                "overprivileged-release-permissions",
                "release jobs must retain least-privilege permissions",
            );
            break;
        }
    }

    validate_action_pins(&active, &mut findings);

    for finding in scan_forbidden_secrets(RELEASE_WORKFLOW_PATH, contents) {
        findings.push(finding);
    }
    findings
}

/// Verifies that the lockfile contains the release-policy and final-binary packages.
pub fn validate_lockfile(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    for marker in [
        "version = 4",
        "name = \"release_policy_contract\"",
        "name = \"command_routing_contract\"",
        "name = \"libsqlite3-sys\"",
    ] {
        if !contents.contains(marker) {
            add_finding(
                &mut findings,
                LOCKFILE_PATH,
                "stale-release-lockfile",
                "the committed lockfile is missing a required release package or format",
            );
        }
    }
    findings
}

/// Verifies that the workspace does not silently enable the older bundled SQLite source.
pub fn validate_workspace_manifest(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    for marker in [
        "resolver = \"3\"",
        "version = \"0.1.0\"",
        "rusqlite = { version = \"=0.40.2\", default-features = false",
    ] {
        if !contents.contains(marker) {
            add_finding(
                &mut findings,
                WORKSPACE_MANIFEST_PATH,
                "workspace-release-drift",
                "the workspace manifest is missing a required release-build setting",
            );
        }
    }
    if contents.lines().any(|line| {
        line.contains("rusqlite") && line.contains("features") && line.contains("\"bundled\"")
    }) {
        add_finding(
            &mut findings,
            WORKSPACE_MANIFEST_PATH,
            "bundled-sqlite-source",
            "the release workspace must not enable the known-old bundled SQLite source",
        );
    }
    findings
}

/// Verifies that the final executable package is the only package selected by dist.
pub fn validate_final_binary_manifest(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    for marker in [
        "name = \"command_routing_contract\"",
        "name = \"repo-com\"",
        "path = \"src/main.rs\"",
    ] {
        if !contents.contains(marker) {
            add_finding(
                &mut findings,
                FINAL_BINARY_MANIFEST_PATH,
                "final-binary-drift",
                "the final executable manifest is missing a required release binary marker",
            );
        }
    }
    findings
}

/// Verifies the state-layer runtime assertion used by release builds.
pub fn validate_sqlite_source(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    for marker in [
        "REQUIRED_SQLITE_VERSION: &str = \"3.53.4\"",
        "pub fn assert_sqlite_runtime()",
        "rusqlite::version_number()",
        "pub fn open_path_for_release",
    ] {
        if !contents.contains(marker) {
            add_finding(
                &mut findings,
                SQLITE_SOURCE_PATH,
                "missing-sqlite-runtime-assertion",
                "the state runtime boundary does not expose the required release assertion",
            );
        }
    }
    findings
}

/// Returns true only for a SQLite version at or above the release floor.
#[must_use]
pub fn sqlite_version_satisfies_release_floor(version: &str) -> bool {
    let mut components = version.trim().split('.');
    let major = components.next().and_then(|part| part.parse::<u32>().ok());
    let minor = components.next().and_then(|part| part.parse::<u32>().ok());
    let patch = components
        .next()
        .map(|part| part.split(['-', '+']).next().unwrap_or(part))
        .and_then(|part| part.parse::<u32>().ok());
    let Some((major, minor, patch)) = major.zip(minor).zip(patch).map(|((a, b), c)| (a, b, c))
    else {
        return false;
    };
    (major, minor, patch) >= (3, 53, 4)
}

/// Validates a release runtime observation. `linkage` must describe static linkage.
pub fn validate_sqlite_release_observation(version: &str, linkage: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    if !sqlite_version_satisfies_release_floor(version) {
        add_finding(
            &mut findings,
            SQLITE_SOURCE_PATH,
            "unsupported-sqlite-runtime",
            "the observed SQLite runtime is older than the release floor",
        );
    }
    let normalized = linkage.trim().to_ascii_lowercase();
    if normalized != "static" && !normalized.contains("static sqlite") {
        add_finding(
            &mut findings,
            SQLITE_SOURCE_PATH,
            "dynamic-sqlite-linkage",
            "the release observation does not prove static SQLite linkage",
        );
    }
    if normalized.contains("dynamic") || normalized.contains("system") {
        add_finding(
            &mut findings,
            SQLITE_SOURCE_PATH,
            "dynamic-sqlite-linkage",
            "the release observation reports dynamic or system SQLite linkage",
        );
    }
    findings
}

/// Scans a release surface for credential and real-message shapes without returning values.
pub fn scan_forbidden_secrets(
    relative_path: impl Into<String>,
    contents: &str,
) -> Vec<PolicyFinding> {
    let path = relative_path.into();
    let lower = contents.to_ascii_lowercase();
    let mut findings = Vec::new();

    if contains_discord_token_shape(contents) {
        add_finding(
            &mut findings,
            &path,
            "discord-token-pattern",
            "a Discord-token-shaped value is present",
        );
    }
    if lower.contains("-----begin") && lower.contains("private key-----") {
        add_finding(
            &mut findings,
            &path,
            "private-key-pattern",
            "private-key material is present",
        );
    }
    if contains_authorization_value(&lower) {
        add_finding(
            &mut findings,
            &path,
            "authorization-value-pattern",
            "an authorization header contains a value",
        );
    }
    if contains_real_message_value(&lower) {
        add_finding(
            &mut findings,
            &path,
            "real-message-pattern",
            "real team message content is present",
        );
    }
    findings
}

fn validate_file(
    root: &Path,
    relative: &str,
    validator: fn(&str) -> Vec<PolicyFinding>,
    findings: &mut Vec<PolicyFinding>,
) {
    match fs::read_to_string(root.join(relative)) {
        Ok(contents) => findings.extend(validator(&contents)),
        Err(error) => findings.push(PolicyFinding {
            path: relative.to_owned(),
            code: "missing-release-policy-file",
            message: format!("required release policy file could not be read: {error}"),
        }),
    }
}

fn active_text(contents: &str) -> String {
    contents
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_strong_checksum(contents: &str) -> bool {
    let value = assignment_value(contents, "checksum").unwrap_or_default();
    [
        "sha256", "sha512", "sha3-256", "sha3-512", "blake2s", "blake2b",
    ]
    .iter()
    .any(|algorithm| value.eq_ignore_ascii_case(algorithm))
}

fn assignment_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let trimmed = line.trim();
        let (name, value) = trimmed.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case(key) {
            return None;
        }
        Some(
            value
                .split('#')
                .next()
                .unwrap_or(value)
                .trim()
                .trim_matches('"')
                .trim_matches('\''),
        )
    })
}

fn semantic_version(value: &str) -> bool {
    let mut components = value.split('.');
    let major = components.next().and_then(|part| part.parse::<u32>().ok());
    let minor = components.next().and_then(|part| part.parse::<u32>().ok());
    let patch = components
        .next()
        .map(|part| part.split(['-', '+']).next().unwrap_or(part))
        .and_then(|part| part.parse::<u32>().ok());
    major.is_some() && minor.is_some() && patch.is_some()
}

fn has_dependency_chain(lower: &str, job: &str, needs: &[&str]) -> bool {
    let Some(start) = lower.find(job) else {
        return false;
    };
    let remainder = &lower[start + job.len()..];
    let mut end = remainder.len();
    for (index, _) in remainder.match_indices("\n  ") {
        let next = remainder.as_bytes().get(index + 3).copied();
        if next.is_some_and(|byte| !byte.is_ascii_whitespace()) {
            end = index;
            break;
        }
    }
    let section = &remainder[..end];
    let Some((_, dependency_text)) = section.split_once("needs:") else {
        return false;
    };
    let mut dependencies = Vec::new();
    for line in dependency_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(dependency) = trimmed.strip_prefix("- ") {
            dependencies.push(dependency.trim());
            continue;
        }
        if dependencies.is_empty() && !trimmed.starts_with('-') {
            dependencies.push(trimmed.split_whitespace().next().unwrap_or_default());
        }
        break;
    }
    needs
        .iter()
        .all(|need| dependencies.iter().any(|dependency| dependency == need))
}

fn validate_action_pins(active: &str, findings: &mut Vec<PolicyFinding>) {
    let mut found_action = false;
    for line in active.lines() {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("uses:") else {
            continue;
        };
        found_action = true;
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !is_immutable_action_reference(value) {
            add_finding(
                findings,
                RELEASE_WORKFLOW_PATH,
                "unpinned-release-action",
                "every external release action must use an immutable commit reference",
            );
        }
    }
    if !found_action {
        add_finding(
            findings,
            RELEASE_WORKFLOW_PATH,
            "missing-release-action",
            "the release workflow must check out source with a pinned action",
        );
    }
}

fn is_immutable_action_reference(value: &str) -> bool {
    let Some((_, reference)) = value.rsplit_once('@') else {
        return false;
    };
    reference.len() == 40
        && reference
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn scan_known_generated_surfaces(root: &Path, findings: &mut Vec<PolicyFinding>) {
    let mut paths = vec![
        root.join("dist-manifest.json"),
        root.join("plan-dist-manifest.json"),
        root.join("release-plan.json"),
        root.join("dependency-licenses.txt"),
        root.join("bom.json"),
        root.join("bom.cdx.json"),
    ];
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if name.starts_with("bom") || name == "release-plan.json" {
                paths.push(path);
            }
        }
    }
    let generated_root = root.join("target/distrib");
    collect_generated_files(&generated_root, 0, &mut paths);
    for path in paths {
        let Some(relative) = relative_path(root, &path) else {
            continue;
        };
        if !is_text_surface(&path) {
            continue;
        }
        if let Ok(contents) = fs::read_to_string(&path) {
            findings.extend(scan_forbidden_secrets(relative, &contents));
        }
    }
}

fn collect_generated_files(root: &Path, depth: usize, paths: &mut Vec<PathBuf>) {
    if depth > 3 || !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_generated_files(&path, depth + 1, paths);
        } else {
            paths.push(path);
        }
    }
}

fn is_text_surface(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        ".json",
        ".xml",
        ".txt",
        ".sha256",
        ".sha512",
        ".sha3-256",
        ".sha3-512",
        ".blake2s",
        ".blake2b",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
}

fn add_finding(
    findings: &mut Vec<PolicyFinding>,
    path: &str,
    code: &'static str,
    message: impl Into<String>,
) {
    let finding = PolicyFinding {
        path: path.to_owned(),
        code,
        message: message.into(),
    };
    if !findings.contains(&finding) {
        findings.push(finding);
    }
}

fn contains_discord_token_shape(contents: &str) -> bool {
    let bytes = contents.as_bytes();
    for start in 0..bytes.len() {
        if !is_token_character(bytes[start]) || (start > 0 && is_token_character(bytes[start - 1]))
        {
            continue;
        }
        let mut cursor = start;
        let mut lengths = [0usize; 3];
        for (index, length) in lengths.iter_mut().enumerate() {
            while cursor < bytes.len() && is_token_character(bytes[cursor]) {
                cursor += 1;
                *length += 1;
            }
            if index < 2 {
                if cursor >= bytes.len() || bytes[cursor] != b'.' {
                    break;
                }
                cursor += 1;
            }
        }
        if cursor <= bytes.len()
            && lengths[0] >= 8
            && lengths[1] >= 3
            && lengths[2] >= 8
            && (cursor == bytes.len() || !is_token_character(bytes[cursor]))
        {
            return true;
        }
    }
    false
}

fn is_token_character(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-' | b'=')
}

fn contains_authorization_value(lower: &str) -> bool {
    for marker in ["authorization:", "authorization="] {
        let Some(position) = lower.find(marker) else {
            continue;
        };
        let remainder = lower[position + marker.len()..].trim();
        let value = remainder
            .trim_matches('"')
            .trim_matches('\'')
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if looks_like_secret_value(value) {
            return true;
        }
        for scheme in ["bearer ", "bot ", "basic "] {
            if let Some(value_start) = remainder.find(scheme) {
                let scheme_value = remainder[value_start + scheme.len()..]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default();
                if looks_like_secret_value(scheme_value) {
                    return true;
                }
            }
        }
    }
    false
}

fn contains_real_message_value(lower: &str) -> bool {
    for marker in [
        "real team message:",
        "production team message:",
        "live team message:",
        "actual team message:",
    ] {
        if let Some(position) = lower.find(marker) {
            let value = lower[position + marker.len()..].trim();
            if !value.is_empty() && !has_negative_context(value) {
                return true;
            }
        }
    }
    false
}

fn looks_like_secret_value(value: &str) -> bool {
    if value.len() < 12
        || [
            "redacted",
            "placeholder",
            "example",
            "synthetic",
            "token",
            "secret",
            "value",
            "header",
            "scheme",
            "credential",
            "<",
            ">",
        ]
        .iter()
        .any(|marker| value.contains(marker))
    {
        return false;
    }
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_-./+=:".contains(character))
}

fn has_negative_context(value: &str) -> bool {
    [
        "synthetic",
        "redacted",
        "placeholder",
        "example",
        "not ",
        "no ",
        "never",
    ]
    .iter()
    .any(|marker| value.contains(marker))
}
