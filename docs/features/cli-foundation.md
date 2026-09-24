# Feature: CLI Foundation

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** None  
**Status:** Canonical v1 plan

This feature establishes the reproducible Rust workspace, typed global arguments, versioned machine protocol, stable error categories, and test-selection convention used by every later feature. It deliberately does not implement repository configuration, storage, Discord, or feature-specific commands.

> **Current implementation:** `PLAT-1` is implemented in the four-package workspace. The final executable, process output routing, and human terminal renderer remain future work; this document continues to own the planned requirements and task contract.

### In Scope

- Rust 2024 workspace and pinned toolchain.
- Shared argument, protocol, and error value types.
- JSON and human-output boundaries at the library level.
- TTY/non-TTY decision primitives.
- Test runner configuration that fails when a selected test set is empty.

### Out of Scope

- Provider calls and feature-specific command handlers.
- SQLite files or TOML parsing.
- A composed `repo-com` binary; final command routing is owned by Release Readiness.

---

## 2. Interfaces and Preconditions

| Interface | Consumed By | Boundary |
|---|---|---|
| `GlobalArgs` | All later command crates and the final CLI | Config path, output format, color choice, diagnostics choice |
| `CommandOutcome<T>` | All command handlers and tests | One protocol object with stable status and error code |
| `TtyMode` | Approval, activation, and override services | Explicit enum; no hidden terminal probing inside domain code |
| `RepoComError` | All workflows | Stable category mapped to deterministic process exit codes |

The workspace uses `members = ["crates/*"]` so focused implementation crates can be added without a shared mutable module registry. Central workspace dependency versions are declared once and crates use workspace dependencies.

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| FOUND-FR-01 | requirement | Must | REL-APP-1 |
| FOUND-FR-02 | requirement | Must | REL-APP-1 |
| FOUND-FR-03 | requirement | Must | PLAT-1 |
| FOUND-FR-04 | requirement | Must | PLAT-1, REL-APP-1 |
| FOUND-FR-05 | requirement | Must | PLAT-1 |
| FOUND-FR-06 | requirement | Must | PLAT-1 |
| FOUND-CON-01 | constraint | Must | PLAT-1, REL-APP-1 |
| FOUND-CON-02 | constraint | Must | PLAT-1, REL-APP-1 |

```forge-requirement
{"id":"FOUND-FR-01","kind":"requirement","text":"Install one binary named repo-com on PATH and report its semantic version through repo-com --version."}
```

```forge-requirement
{"id":"FOUND-FR-02","kind":"requirement","text":"Route configuration, policy, draft, send, inbox, reply, audit, state, and purge commands through one typed command tree with no hidden default destination."}
```

```forge-requirement
{"id":"FOUND-FR-03","kind":"requirement","text":"Define protocol version 1 as exactly one JSON object on stdout for machine mode, with protocol_version, status, data, and error fields; keep diagnostics and prompts off stdout."}
```

```forge-requirement
{"id":"FOUND-FR-04","kind":"requirement","text":"Expose an explicit TTY versus non-TTY mode and prohibit interactive prompts whenever stdin or stdout is non-interactive unless the command is an operator-only action that fails closed in automation."}
```

```forge-requirement
{"id":"FOUND-FR-05","kind":"requirement","text":"Map errors to stable process categories: usage/schema, approval or operator action required, policy blocked, authentication, permission, remote conflict, unknown delivery, storage integrity, connectivity/rate limit, and internal failure."}
```

```forge-requirement
{"id":"FOUND-FR-06","kind":"requirement","text":"Provide a cargo-nextest convention using --no-tests fail and exact test-binary filters so every task can prove its intended tests were discovered and executed."}
```

```forge-requirement
{"id":"FOUND-CON-01","kind":"constraint","text":"Pin Rust 1.98.1 and the Rust 2024 edition, commit Cargo.lock, use one central workspace dependency table, and do not add a new major dependency without an explicit planning review."}
```

```forge-requirement
{"id":"FOUND-CON-02","kind":"constraint","text":"Keep machine-mode stdout valid JSON even for operational errors; reserve stderr for opt-in diagnostics and never mix prompts into protocol output."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| PLAT-1 | Reproducible workspace compiles a tested protocol/error/TTY foundation | rust-foundation-engineer | Rust 1.98.1 available | Workspace manifests, foundation library, `foundation_contract` nextest target | FOUND-FR-03 through FOUND-FR-06 and both constraints | Binary routing, storage, Discord |

---

## Phase 1: Reproducible Protocol Foundation

```forge-task
{
  "id": "PLAT-1",
  "title": "Establish the Rust workspace and command contract",
  "description": "Establish a reproducible Rust 2024 workspace and a typed foundation for global arguments, protocol version 1 output, stable error categories, explicit TTY mode, and fail-on-zero-tests selection. Pin Rust 1.98.1, centralize direct dependency versions, commit Cargo.lock, and create only the focused repo-com-foundation library plus its contract test. Machine stdout must remain one JSON object and diagnostics or prompts must use stderr. Do not create the final binary, feature commands, storage, configuration, or network clients.",
  "ownerAgent": "rust-foundation-engineer",
  "dependencies": [],
  "expectedOutputs": [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".config/nextest.toml",
    "crates/repo-com-foundation/Cargo.toml",
    "crates/repo-com-foundation/src/lib.rs",
    "crates/repo-com-foundation/src/args.rs",
    "crates/repo-com-foundation/src/protocol.rs",
    "crates/repo-com-foundation/src/error.rs",
    "crates/repo-com-foundation/tests/foundation_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(foundation_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/cli-foundation.md#FOUND-FR-03",
      "docs/features/cli-foundation.md#FOUND-FR-04",
      "docs/features/cli-foundation.md#FOUND-FR-05",
      "docs/features/cli-foundation.md#FOUND-FR-06",
      "docs/PRD.md#RC-FR-04"
    ],
    "acceptanceCriteria": [
      "Foundation contract tests prove protocol version 1 serializes exactly one valid JSON envelope for success and every stable error category",
      "Foundation contract tests prove non-TTY mode cannot request an interactive prompt and diagnostics never appear in the machine stdout value",
      "Foundation contract tests prove the selected nextest binary is discovered; the command is configured to fail when no test matches",
      "Toolchain and lockfile checks prove Rust 1.98.1, edition 2024, and locked direct dependencies are in force"
    ],
    "constraints": [
      "Keep the foundation library free of repository-specific behavior and external I/O"
    ],
    "constraintRefs": [
      "docs/features/cli-foundation.md#FOUND-CON-01",
      "docs/features/cli-foundation.md#FOUND-CON-02",
      "docs/features/repository-configuration-and-state.md#AUDIT-CON-01"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/PRD.md#12. User Interface / Interaction Design"
    ]
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Task |
|---|---|---|
| [FOUND-FR-01](cli-foundation.md#FOUND-FR-01) | requirement | REL-APP-1 |
| [FOUND-FR-02](cli-foundation.md#FOUND-FR-02) | requirement | REL-APP-1 |
| [FOUND-FR-03](cli-foundation.md#FOUND-FR-03) | requirement | PLAT-1 |
| [FOUND-FR-04](cli-foundation.md#FOUND-FR-04) | requirement | PLAT-1, REL-APP-1 |
| [FOUND-FR-05](cli-foundation.md#FOUND-FR-05) | requirement | PLAT-1 |
| [FOUND-FR-06](cli-foundation.md#FOUND-FR-06) | requirement | PLAT-1 |
| [FOUND-CON-01](cli-foundation.md#FOUND-CON-01) | constraint | PLAT-1, REL-APP-1 |
| [FOUND-CON-02](cli-foundation.md#FOUND-CON-02) | constraint | PLAT-1, REL-APP-1 |
