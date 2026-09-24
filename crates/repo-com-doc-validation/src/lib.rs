#![forbid(unsafe_code)]
#![doc = "Deterministic topic, claim-polarity, and secret-pattern checks for repo-com documentation."]

use std::fmt;
use std::fs;
use std::path::Path;

#[cfg(test)]
#[path = "../tests/documentation_contract.rs"]
mod documentation_contract;

/// The five documents owned by the release documentation task.
pub const DOCUMENT_PATHS: [&str; 5] = [
    "docs/configuration.md",
    "docs/discord-setup.md",
    "docs/operator-guide.md",
    "docs/security-model.md",
    "docs/threat-model.md",
];

/// Required topic markers, grouped by their owning document.
///
/// These are intentionally semantic markers rather than a prose snapshot. They
/// make a removed setup, recovery, privacy, or accessibility section fail the
/// contract without requiring a particular paragraph layout.
pub const REQUIRED_TOPICS: &[(&str, &[&str])] = &[
    (
        "docs/configuration.md",
        &[
            "schema_version = 1",
            "repository_id",
            "[discord]",
            "[destinations.",
            "[mentions.",
            "[inbound.",
            "[retention]",
            "[[auto_send]]",
            "destination alias",
            "mention alias",
            "inbound alias",
            "role:",
            "user:",
            "content_days",
            "metadata_days",
            "30 days",
            "365 days",
            "1 through 365",
            "30 through 3,650",
            "exact event type",
            "exact destination alias",
            "exact severity",
            "unknown fields",
            "secret-like fields",
            "raw destination",
            "unsafe schema",
            "WorkspaceReferenceIndex",
        ],
    ),
    (
        "docs/discord-setup.md",
        &[
            "dedicated Discord bot",
            "Developer Portal",
            "REST v10",
            "least privilege",
            "VIEW_CHANNEL",
            "SEND_MESSAGES",
            "READ_MESSAGE_HISTORY",
            "MENTION_ROLES",
            "channel check",
            "mention check",
            "workspace membership",
            "remediation",
            "ready",
            "REPO_COM_DISCORD_TOKEN",
            "rotate",
            "reset",
            "revoke",
            "read-only",
            "manual",
            "user token",
            "self-bot",
            "official Discord",
        ],
    ),
    (
        "docs/operator-guide.md",
        &[
            "Installation status",
            "rustup",
            "Rust 1.98.1",
            "no installed `repo-com` binary",
            "protocol_version",
            "protocol version 1",
            "exactly one JSON object",
            "stdout",
            "stderr",
            "--config",
            "--output",
            "--color",
            "--diagnostics",
            "config.validate",
            "policy.status",
            "policy.activate",
            "state.verify",
            "lifecycle.inspect",
            "audit.query",
            "purge.plan",
            "purge.execute",
            "draft.create",
            "draft.show",
            "draft.update",
            "draft.preview",
            "draft.approve",
            "draft.secret-override",
            "send",
            "setup-check",
            "inbox.fetch",
            "inbox.acknowledge",
            "inbox.archive",
            "reply.draft-create",
            "repository_id",
            "draft_id",
            "revision",
            "destination_alias",
            "cursor",
            "page_size",
            "limit",
            "include_retained_content",
            "event_type",
            "severity",
            "created_at",
            "expires_in_seconds",
            "bot_user_id",
            "item_ids",
            "scope",
            "cutoff",
            "config_hash",
            "executed_at",
            "plan_hash",
            "preview",
            "approve",
            "non-TTY",
            "TTY",
            "80 columns",
            "NO_COLOR",
            "unknown",
            "reconcil",
            "no automatic resend",
            "read receipt",
            "response analytics",
            "unsupported",
            "retention",
        ],
    ),
    (
        "docs/security-model.md",
        &[
            "Trust boundaries",
            "dedicated bot",
            "bot-only",
            "REPO_COM_DISCORD_TOKEN",
            "untrusted inbound",
            "exact-revision",
            "duplicate",
            "redact",
            "local state",
            "recovery",
            "user-only filesystem permissions",
            "no encryption at rest",
            "no telemetry",
            "local-account compromise",
            "backups",
            "filesystem snapshots",
        ],
    ),
    (
        "docs/threat-model.md",
        &[
            "Threat actors",
            "Trust assumptions",
            "Assets",
            "Abuse cases",
            "Mitigations",
            "Residual risk",
            "user-only filesystem permissions",
            "no encryption at rest",
            "no telemetry",
            "local-account compromise",
            "backups",
            "filesystem snapshots",
            "unsupported",
            "human review",
            "live Discord",
            "compliance",
        ],
    ),
];

/// One deterministic documentation validation finding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentationFinding {
    /// Repository-relative document path.
    pub path: String,
    /// Stable machine-readable finding category.
    pub code: &'static str,
    /// Safe explanation that does not include source content.
    pub message: String,
}

impl fmt::Display for DocumentationFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}: {}", self.path, self.code, self.message)
    }
}

impl std::error::Error for DocumentationFinding {}

/// Validates every owned document below a repository root.
pub fn validate_repository(root: &Path) -> Vec<DocumentationFinding> {
    let mut findings = Vec::new();
    for relative in DOCUMENT_PATHS {
        let path = root.join(relative);
        match fs::read_to_string(&path) {
            Ok(contents) => findings.extend(validate_document(relative, &contents)),
            Err(error) => findings.push(DocumentationFinding {
                path: relative.to_owned(),
                code: "missing-document",
                message: format!("could not read required document: {error}"),
            }),
        }
    }
    findings
}

/// Validates one document's required topics and claim/secret polarity.
pub fn validate_document(
    relative_path: impl Into<String>,
    contents: &str,
) -> Vec<DocumentationFinding> {
    let path = relative_path.into();
    let lower = contents.to_ascii_lowercase();
    let mut findings = Vec::new();

    for (document, markers) in REQUIRED_TOPICS {
        if *document != path {
            continue;
        }
        for marker in *markers {
            if !lower.contains(&marker.to_ascii_lowercase()) {
                findings.push(DocumentationFinding {
                    path: path.clone(),
                    code: "missing-topic",
                    message: format!("missing required topic marker: {marker}"),
                });
            }
        }
        break;
    }

    findings.extend(scan_forbidden_patterns(path, contents));
    findings
}

/// Scans text for credential-shaped values and prohibited positive claims.
///
/// The scanner intentionally reports locations only as a category and never
/// returns a matched value. Synthetic examples can be checked by constructing
/// their characters at runtime; no real credential needs to be committed.
pub fn scan_forbidden_patterns(
    relative_path: impl Into<String>,
    contents: &str,
) -> Vec<DocumentationFinding> {
    let path = relative_path.into();
    let mut findings = Vec::new();

    if contains_discord_token_shape(contents) {
        add_finding(
            &mut findings,
            &path,
            "discord-token-pattern",
            "a Discord-token-shaped value is present",
        );
    }

    let lower_contents = contents.to_ascii_lowercase();
    if lower_contents.contains("-----begin") && lower_contents.contains("private key-----") {
        add_finding(
            &mut findings,
            &path,
            "private-key-pattern",
            "private-key material is present",
        );
    }
    if contains_multiline_authorization_value(&lower_contents) {
        add_finding(
            &mut findings,
            &path,
            "authorization-value-pattern",
            "an authorization header contains a value",
        );
    }

    let lines: Vec<&str> = contents.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let lower_line = line.to_ascii_lowercase();
        let context_start = index.saturating_sub(1);
        let context_end = (index + 2).min(lines.len());
        let context = lines[context_start..context_end]
            .iter()
            .map(|value| value.trim())
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();

        check_assignment_value(&mut findings, &path, &lower_line);
        check_message_assignment(&mut findings, &path, &lower_line);
        check_authorization_value(&mut findings, &path, &lower_line);

        for marker in [
            "real team message",
            "actual team message",
            "production team message",
            "live team message",
            "real_message",
            "real-message",
            "team_message",
            "message-content-fixture",
            "real message content",
            "actual message content",
            "production message content",
            "team message content",
        ] {
            if context.contains(marker) && !has_negative_context(&context) {
                add_finding(
                    &mut findings,
                    &path,
                    "real-message-content-pattern",
                    "a real-message-content marker is present",
                );
                break;
            }
        }

        check_positive_claims(&mut findings, &path, &context);
    }

    findings
}

fn add_finding(
    findings: &mut Vec<DocumentationFinding>,
    path: &str,
    code: &'static str,
    message: &str,
) {
    if !findings
        .iter()
        .any(|finding| finding.path == path && finding.code == code && finding.message == message)
    {
        findings.push(DocumentationFinding {
            path: path.to_owned(),
            code,
            message: message.to_owned(),
        });
    }
}

fn check_assignment_value(findings: &mut Vec<DocumentationFinding>, path: &str, lower_line: &str) {
    for key in ["token", "password", "secret", "private_key", "private key"] {
        let mut offset = 0;
        while let Some(relative) = lower_line[offset..].find(key) {
            let value_start = offset + relative + key.len();
            let remainder = lower_line[value_start..].trim_start();
            let Some(value) = remainder
                .strip_prefix('=')
                .or_else(|| remainder.strip_prefix(':'))
            else {
                offset = value_start;
                continue;
            };
            let candidate = value
                .trim_matches(|character: char| {
                    character.is_whitespace() || matches!(character, '"' | '\'' | '`' | ')' | ']')
                })
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if looks_like_secret_value(candidate) {
                add_finding(
                    findings,
                    path,
                    "secret-assignment-pattern",
                    "a secret-like assignment contains a value",
                );
            }
            offset = value_start;
        }
    }
}

fn check_message_assignment(
    findings: &mut Vec<DocumentationFinding>,
    path: &str,
    lower_line: &str,
) {
    for key in [
        "real_message",
        "real-message",
        "team_message",
        "production_message",
        "message_content",
    ] {
        let Some(position) = lower_line.find(key) else {
            continue;
        };
        let remainder = lower_line[position + key.len()..].trim_start();
        let Some(value) = remainder
            .strip_prefix('=')
            .or_else(|| remainder.strip_prefix(':'))
        else {
            continue;
        };
        let candidate = value
            .trim()
            .trim_matches(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '\'' | '`' | ')' | ']')
            })
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if !candidate.is_empty()
            && !candidate.to_ascii_lowercase().contains("synthetic")
            && !candidate.to_ascii_lowercase().contains("example")
            && !candidate.to_ascii_lowercase().contains("redacted")
            && !has_negative_context(lower_line)
        {
            add_finding(
                findings,
                path,
                "real-message-content-pattern",
                "a real-message-content assignment contains a value",
            );
        }
    }
}

fn contains_multiline_authorization_value(lower_contents: &str) -> bool {
    let normalized = lower_contents
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for marker in ["authorization:", "authorization="] {
        let Some(position) = normalized.find(marker) else {
            continue;
        };
        let remainder = &normalized[position + marker.len()..];
        let candidate = remainder.split_whitespace().next().unwrap_or_default();
        if looks_like_secret_value(candidate) {
            return true;
        }
        for scheme in ["bearer ", "bot ", "basic "] {
            if let Some(value_start) = remainder.find(scheme) {
                let value = remainder[value_start + scheme.len()..]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default();
                if looks_like_secret_value(value) {
                    return true;
                }
            }
        }
    }
    false
}

fn check_authorization_value(
    findings: &mut Vec<DocumentationFinding>,
    path: &str,
    lower_line: &str,
) {
    if has_negative_context(lower_line) {
        return;
    }

    if let Some(value_start) = lower_line.find("authorization") {
        let remainder = lower_line[value_start + "authorization".len()..].trim_start();
        if let Some(value) = remainder
            .strip_prefix(':')
            .or_else(|| remainder.strip_prefix('='))
        {
            let candidate = value.trim().trim_matches('"').trim_matches('\'');
            if looks_like_secret_value(candidate) {
                add_finding(
                    findings,
                    path,
                    "authorization-value-pattern",
                    "an authorization header contains a value",
                );
            }
        }
    }

    for scheme in ["bearer ", "bot ", "basic "] {
        if let Some(value_start) = lower_line.find(scheme) {
            let candidate = lower_line[value_start + scheme.len()..]
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if looks_like_secret_value(candidate) {
                add_finding(
                    findings,
                    path,
                    "authorization-value-pattern",
                    "an authorization scheme contains a value",
                );
            }
        }
    }
}

fn looks_like_secret_value(value: &str) -> bool {
    if value.len() < 12 {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if [
        "redacted",
        "placeholder",
        "example",
        "synthetic",
        "your",
        "token",
        "secret",
        "value",
        "raw dedicated bot",
        "authorization",
        "scheme",
        "header",
        "credential",
        "user",
        "<",
        ">",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return false;
    }
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_-./+=:".contains(character))
}

fn has_negative_context(lower_line: &str) -> bool {
    [
        "not ",
        "no ",
        "never",
        "cannot",
        "can't",
        "without",
        "unsupported",
        "prohibited",
        "forbidden",
        "must not",
        "does not",
        "doesn't",
        "is not",
        "aren't",
        "rather than",
        "reject",
        "rejected",
        "fail closed",
        "not a claim",
        "not a guarantee",
    ]
    .iter()
    .any(|marker| lower_line.contains(marker))
}

fn check_positive_claims(findings: &mut Vec<DocumentationFinding>, path: &str, lower_line: &str) {
    if has_negative_context(lower_line) {
        return;
    }

    let claims: &[(&'static str, &[&str])] = &[
        (
            "read-receipt-claim",
            &[
                "provides read receipts",
                "read receipts are available",
                "read receipt is available",
                "proves a teammate read",
                "confirms a teammate read",
            ],
        ),
        (
            "response-analytics-claim",
            &[
                "provides response analytics",
                "response analytics",
                "response analytics are available",
                "tracks responses",
            ],
        ),
        (
            "arbitrary-destination-claim",
            &[
                "allows arbitrary destinations",
                "supports arbitrary destinations",
            ],
        ),
        (
            "user-token-claim",
            &[
                "supports user tokens",
                "accepts user tokens",
                "uses user-token authentication",
            ],
        ),
        (
            "live-compatibility-claim",
            &[
                "is compatible with live discord",
                "live discord compatibility is proven",
                "live compatibility is confirmed",
                "live test succeeded",
                "live test success",
                "live discord success is demonstrated",
            ],
        ),
        (
            "human-approval-claim",
            &[
                "has human approval",
                "human approval is complete",
                "release is approved",
                "release approval is complete",
                "final sign-off is recorded",
                "human sign-off is recorded",
                "sign-off is complete",
                "final approval is granted",
                "release is signed off",
                "compliance certified",
                "certified compliant",
            ],
        ),
        (
            "telemetry-claim",
            &[
                "collects telemetry",
                "telemetry is collected",
                "telemetry is available",
                "sends telemetry",
            ],
        ),
        (
            "encryption-claim",
            &[
                "encrypts local state",
                "local state is encrypted",
                "encrypts data at rest",
                "provides encryption at rest",
            ],
        ),
        (
            "automatic-resend-claim",
            &[
                "automatically resends",
                "automatic resend is performed",
                "unknown outcomes are automatically resent",
                "unknown delivery is resent",
                "unresolved delivery is resent",
            ],
        ),
        (
            "remote-truth-claim",
            &["knows current remote truth", "proves current remote state"],
        ),
        (
            "completion-claim",
            &[
                "send completed",
                "purge completed",
                "approval completed",
                "live send succeeded",
            ],
        ),
    ];

    for (code, phrases) in claims {
        if phrases.iter().any(|phrase| lower_line.contains(phrase)) {
            add_finding(
                findings,
                path,
                code,
                "a positive claim outside the supported evidence boundary is present",
            );
        }
    }
}

fn contains_discord_token_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !is_token_character(bytes[start]) || (start > 0 && is_token_character(bytes[start - 1]))
        {
            continue;
        }
        let mut cursor = start;
        let mut lengths = [0usize; 3];
        for (segment, length) in lengths.iter_mut().enumerate() {
            while cursor < bytes.len() && is_token_character(bytes[cursor]) {
                cursor += 1;
                *length += 1;
            }
            if segment < 2 {
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
