#![forbid(unsafe_code)]
#![doc = "Machine-checkable policy for the token-free repo-com continuous-integration workflow."]

use std::fmt;
use std::fs;
use std::path::Path;

#[cfg(test)]
#[path = "../tests/ci_policy_contract.rs"]
mod ci_policy_contract;

/// Repository-relative path of the workflow owned by this task.
pub const CI_WORKFLOW_PATH: &str = ".github/workflows/ci.yml";
/// Repository-relative path of the dependency policy owned by this task.
pub const DENY_POLICY_PATH: &str = "deny.toml";
/// Repository-relative path of the dependency update policy owned by this task.
pub const DEPENDABOT_PATH: &str = ".github/dependabot.yml";
/// Repository-relative path of the committed lockfile.
pub const LOCKFILE_PATH: &str = "Cargo.lock";
/// Repository-relative path of the pinned Rust toolchain file.
pub const TOOLCHAIN_PATH: &str = "rust-toolchain.toml";

/// The Rust toolchain used by every CI job.
pub const RUST_TOOLCHAIN_VERSION: &str = "1.98.1";
/// The fail-on-zero-tests runner version used by every CI job.
pub const NEXTEST_VERSION: &str = "0.9.146";
/// The cargo-deny version used for advisories, bans, licenses, and sources.
pub const CARGO_DENY_VERSION: &str = "0.20.2";
/// The cargo-audit version used for RustSec checks.
pub const CARGO_AUDIT_VERSION: &str = "0.22.2";
/// The actionlint version used for workflow validation.
pub const ACTIONLINT_VERSION: &str = "1.7.12";
/// The gitleaks version used for source and generated-file scanning.
pub const GITLEAKS_VERSION: &str = "8.30.1";

/// The minimum SQLite release version required by the runtime gate.
pub const MINIMUM_SQLITE_VERSION: &str = "3.53.4";

/// Stable names for every required CI check.
pub const REQUIRED_CHECKS: &[&str] = &[
    "format",
    "clippy",
    "nextest-fail-on-zero-tests",
    "performance",
    "cargo-deny",
    "cargo-audit",
    "secret-scan",
    "actionlint",
    "sqlite-runtime-assertion",
    "binary-smoke",
];

/// The operating-system families that must be present in the CI matrix.
pub const REQUIRED_PLATFORM_FAMILIES: &[&str] = &["ubuntu", "macos", "windows"];

/// A safe, redacted policy finding. Findings never include the offending value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyFinding {
    /// Repository-relative path that produced the finding.
    pub path: String,
    /// Stable machine-readable finding category.
    pub code: &'static str,
    /// Safe explanation without source contents or secret values.
    pub message: String,
}

impl fmt::Display for PolicyFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}: {}", self.path, self.code, self.message)
    }
}

impl std::error::Error for PolicyFinding {}

/// Validates all repository files whose contents are part of the CI policy.
pub fn validate_repository(root: &Path) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    validate_file(root, CI_WORKFLOW_PATH, validate_ci_workflow, &mut findings);
    validate_file(root, DENY_POLICY_PATH, validate_deny_policy, &mut findings);
    validate_file(
        root,
        DEPENDABOT_PATH,
        validate_dependabot_policy,
        &mut findings,
    );
    validate_file(root, LOCKFILE_PATH, validate_lockfile, &mut findings);
    validate_file(root, TOOLCHAIN_PATH, validate_toolchain, &mut findings);
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
            code: "missing-policy-file",
            message: format!("required policy file could not be read: {error}"),
        }),
    }
}

/// Validates the CI workflow's required gates and security boundaries.
pub fn validate_ci_workflow(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    let active = active_text(contents);
    let lower = active.to_ascii_lowercase();

    if active.trim().is_empty() {
        add_finding(
            &mut findings,
            CI_WORKFLOW_PATH,
            "empty-workflow",
            "the CI workflow is empty",
        );
        return findings;
    }

    require_markers(
        &mut findings,
        &lower,
        "workflow-trigger",
        &["push:", "pull_request:"],
    );
    if lower.contains("pull_request_target") || lower.contains("workflow_run") {
        add_finding(
            &mut findings,
            CI_WORKFLOW_PATH,
            "untrusted-trigger",
            "CI must not execute a privileged or follow-on workflow trigger",
        );
    }

    require_markers(
        &mut findings,
        &lower,
        "least-privilege-permissions",
        &[
            "permissions:",
            "contents: read",
            "persist-credentials: false",
        ],
    );
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
                CI_WORKFLOW_PATH,
                "overprivileged-permissions",
                "CI requests a permission beyond read-only source access",
            );
            break;
        }
    }

    require_markers(
        &mut findings,
        &lower,
        "concurrency-control",
        &["concurrency:", "group:", "cancel-in-progress: true"],
    );

    let has_matrix = lower.contains("matrix:") && lower.contains("runs-on: ${{ matrix.os }}");
    if !has_matrix {
        add_finding(
            &mut findings,
            CI_WORKFLOW_PATH,
            "missing-platform-matrix",
            "CI must run jobs from a platform matrix",
        );
    }
    for family in REQUIRED_PLATFORM_FAMILIES {
        let marker = format!("{family}-");
        if !lower.contains(&marker) {
            add_finding(
                &mut findings,
                CI_WORKFLOW_PATH,
                "missing-platform",
                "the CI matrix is missing a required operating-system family",
            );
        }
    }

    validate_action_pins(&active, &mut findings);
    validate_required_checks(&lower, &mut findings);
    validate_tool_pins(&lower, &mut findings);
    validate_untrusted_secret_boundary(&lower, &mut findings);
    validate_publication_boundary(&lower, &mut findings);

    for finding in scan_forbidden_secrets(CI_WORKFLOW_PATH, &active) {
        findings.push(finding);
    }
    findings
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
                CI_WORKFLOW_PATH,
                "unpinned-action",
                "every external action must use an immutable commit reference",
            );
        }
    }
    if !found_action {
        add_finding(
            findings,
            CI_WORKFLOW_PATH,
            "missing-action",
            "CI must check out source with a pinned action",
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

fn validate_required_checks(lower: &str, findings: &mut Vec<PolicyFinding>) {
    let required: &[(&str, &[&str])] = &[
        ("format", &["cargo fmt --all -- --check"]),
        (
            "clippy",
            &["cargo clippy --workspace --all-targets --all-features -- -d warnings"],
        ),
        (
            "nextest-fail-on-zero-tests",
            &["cargo nextest run --workspace --no-tests fail"],
        ),
        (
            "performance",
            &["performance_contract", "repo-com-performance"],
        ),
        (
            "cargo-deny",
            &["cargo deny check advisories bans licenses sources"],
        ),
        ("cargo-audit", &["cargo audit --file cargo.lock"]),
        (
            "secret-scan",
            &[
                "gitleaks dir .",
                "git ls-files -co --exclude-standard -z",
                "repo-com-evidence",
            ],
        ),
        ("actionlint", &["actionlint .github/workflows/ci.yml"]),
        (
            "sqlite-runtime-assertion",
            &[
                "3.53.4",
                "assert_sqlite_runtime",
                "sqlite runtime",
                "cargo test --locked --release",
                "ldd",
                "otool",
                "objdump",
                "dumpbin",
            ],
        ),
        (
            "binary-smoke",
            &[
                "cargo build --locked --package command_routing_contract --bin repo-com",
                "--version",
            ],
        ),
    ];

    for (name, markers) in required {
        if markers.iter().any(|marker| !lower.contains(marker)) {
            add_finding(
                findings,
                CI_WORKFLOW_PATH,
                "missing-required-check",
                format!("the CI workflow is missing or renamed required check: {name}"),
            );
        }
    }
    if !lower.contains("cargo metadata --locked") || !lower.contains("cargo.lock") {
        add_finding(
            findings,
            CI_WORKFLOW_PATH,
            "missing-lockfile-gate",
            "CI must verify the committed lockfile with Cargo's locked mode",
        );
    }
    if lower.contains("cargo update") || lower.contains("cargo generate-lockfile") {
        add_finding(
            findings,
            CI_WORKFLOW_PATH,
            "lockfile-mutation",
            "CI must not update or regenerate the committed lockfile",
        );
    }
}

fn validate_tool_pins(lower: &str, findings: &mut Vec<PolicyFinding>) {
    for (label, version) in [
        ("Rust", RUST_TOOLCHAIN_VERSION),
        ("cargo-nextest", NEXTEST_VERSION),
        ("cargo-deny", CARGO_DENY_VERSION),
        ("cargo-audit", CARGO_AUDIT_VERSION),
        ("actionlint", ACTIONLINT_VERSION),
        ("gitleaks", GITLEAKS_VERSION),
    ] {
        if !lower.contains(&version.to_ascii_lowercase()) {
            add_finding(
                findings,
                CI_WORKFLOW_PATH,
                "unpinned-tool",
                format!("required CI tool is not pinned to its declared version: {label}"),
            );
        }
    }
}

fn validate_untrusted_secret_boundary(lower: &str, findings: &mut Vec<PolicyFinding>) {
    for forbidden in [
        "secrets.",
        "${{ secrets",
        "github.token",
        "github_token",
        "repository_dispatch",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                findings,
                CI_WORKFLOW_PATH,
                "untrusted-secret-use",
                "CI must not reference a secret or privileged token in any trigger",
            );
            break;
        }
    }
}

fn validate_publication_boundary(lower: &str, findings: &mut Vec<PolicyFinding>) {
    for forbidden in [
        "cargo publish",
        "gh release",
        "upload-artifact",
        "actions/upload-artifact",
        "softprops/action-gh-release",
        "docker push",
        "git push",
        "git tag",
        "npm publish",
        "self-update",
        "self_update",
        "auto-update",
        "updater",
        "setup mutation",
        "setup-mutation",
        "mutate discord",
        "docs/artifacts/",
        "docs/engine-run.log",
    ] {
        if lower.contains(forbidden) {
            add_finding(
                findings,
                CI_WORKFLOW_PATH,
                "publication-or-mutation",
                "normal CI must not publish artifacts or perform setup/updater/Discord mutation",
            );
            break;
        }
    }
}

/// Validates the machine-readable cargo-deny baseline.
pub fn validate_deny_policy(contents: &str) -> Vec<PolicyFinding> {
    let lower = contents.to_ascii_lowercase();
    let mut findings = Vec::new();
    let required = [
        "[graph]",
        "[advisories]",
        "[bans]",
        "[sources]",
        "[licenses]",
        "unknown-registry = \"deny\"",
        "unknown-git = \"deny\"",
        "allow-registry = [\"https://github.com/rust-lang/crates.io-index\"]",
        "multiple-versions = \"warn\"",
        "wildcards = \"allow\"",
        "allow = [",
    ];
    for marker in required {
        if !lower.contains(&marker.to_ascii_lowercase()) {
            add_finding(
                &mut findings,
                DENY_POLICY_PATH,
                "missing-dependency-policy",
                "the dependency policy does not fail closed for a required check",
            );
        }
    }
    for license in [
        "mit",
        "apache-2.0",
        "isc",
        "unicode-3.0",
        "zlib",
        "bsd-2-clause",
        "bsd-3-clause",
    ] {
        if !lower.contains(license) {
            add_finding(
                &mut findings,
                DENY_POLICY_PATH,
                "missing-license-allow-list",
                "the dependency policy does not explicitly allow a required license family",
            );
        }
    }
    if lower.contains("allow = [\"*\"]") || lower.contains("confidence-threshold = 0.0") {
        add_finding(
            &mut findings,
            DENY_POLICY_PATH,
            "permissive-license-policy",
            "the dependency policy must not allow every license or infer with zero confidence",
        );
    }
    findings
}

/// Validates Dependabot coverage for Rust dependencies and GitHub Actions.
pub fn validate_dependabot_policy(contents: &str) -> Vec<PolicyFinding> {
    let lower = contents.to_ascii_lowercase();
    let mut findings = Vec::new();
    for marker in [
        "version: 2",
        "package-ecosystem: cargo",
        "package-ecosystem: github-actions",
        "directory: \"/\"",
        "interval: weekly",
    ] {
        if !lower.contains(marker) {
            add_finding(
                &mut findings,
                DEPENDABOT_PATH,
                "missing-dependabot-coverage",
                "dependency updates do not cover the required ecosystem and schedule",
            );
        }
    }
    if lower.contains("open-pull-requests-limit: 0") {
        add_finding(
            &mut findings,
            DEPENDABOT_PATH,
            "disabled-dependency-updates",
            "Dependabot must be allowed to open update pull requests",
        );
    }
    findings
}

/// Verifies that the committed lockfile includes the policy crate and is v4.
pub fn validate_lockfile(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    for marker in ["version = 4", "name = \"ci_policy_contract\""] {
        if !contents.contains(marker) {
            add_finding(
                &mut findings,
                LOCKFILE_PATH,
                "stale-lockfile",
                "the committed lockfile is missing the current policy package or format",
            );
        }
    }
    findings
}

/// Verifies that the repository toolchain is pinned to the declared baseline.
pub fn validate_toolchain(contents: &str) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    for marker in [
        "channel = \"1.98.1\"",
        "profile = \"minimal\"",
        "components = [\"rustfmt\", \"clippy\"]",
    ] {
        if !contents.contains(marker) {
            add_finding(
                &mut findings,
                TOOLCHAIN_PATH,
                "unpinned-toolchain",
                "the Rust toolchain file does not match the CI baseline",
            );
        }
    }
    findings
}

/// Returns true only for a SQLite version at or above `3.53.4`.
#[must_use]
pub fn sqlite_version_satisfies_release_floor(version: &str) -> bool {
    let mut components = version.split('.');
    let major = components.next().and_then(|part| part.parse::<u32>().ok());
    let minor = components.next().and_then(|part| part.parse::<u32>().ok());
    let patch = components.next().and_then(|part| part.parse::<u32>().ok());
    let Some((major, minor, patch)) = major.zip(minor).zip(patch).map(|((a, b), c)| (a, b, c))
    else {
        return false;
    };
    (major, minor, patch) >= (3, 53, 4)
}

/// Scans owned policy surfaces for credential and real-message shapes without
/// returning any matched value.
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

fn active_text(contents: &str) -> String {
    contents
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn require_markers(
    findings: &mut Vec<PolicyFinding>,
    lower: &str,
    code: &'static str,
    markers: &[&str],
) {
    if markers.iter().any(|marker| !lower.contains(marker)) {
        add_finding(
            findings,
            CI_WORKFLOW_PATH,
            code,
            "the CI workflow is missing a required security or scheduling marker",
        );
    }
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
            let Some(value_start) = remainder.find(scheme) else {
                continue;
            };
            let scheme_value = remainder[value_start + scheme.len()..]
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if looks_like_secret_value(scheme_value) {
                return true;
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
    if value.len() < 12 {
        return false;
    }
    if [
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
