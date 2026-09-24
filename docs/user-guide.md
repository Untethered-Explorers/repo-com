# Library Consumer Guide

> **Status:** workspace `0.1.0`, pre-release. This guide documents the currently implemented Rust library surface. `repo-com` does not currently ship a CLI or perform Discord requests; planned command behavior is kept separate below.

## Overview

The current workspace provides four libraries for building the safety and state foundation of a repository communication workflow:

- protocol, error, stream, and explicit TTY contracts;
- strict repository-local configuration discovery and validation;
- repository-scoped transactional SQLite state; and
- exact operator-activated policy matching.

The canonical future product is a single-user CLI for agent-originated Discord communication. The [Product Vision](PRD.md) and [feature specifications](features/) describe that planned product, but they are not current installation or runtime instructions.

### Current audiences

- **Library consumer:** a Rust developer embedding one of the current crates.
- **Repository maintainer:** a developer reviewing configuration, state, and policy invariants.
- **Operator:** the future human who will approve or activate actions. The current policy library accepts an explicit TTY confirmation supplied by its caller; it does not prompt or detect a terminal itself.

There is no current end-user command workflow. A teammate-facing Discord workflow and installed command surface are not available yet.

## Build and verify

The repository pins Rust `1.98.1` in [`rust-toolchain.toml`](../rust-toolchain.toml). From the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The current nextest run executes 42 tests across `foundation_contract`, `config_contract`, `state_contract`, and `policy_contract`.

## Core library workflow

The following sequence uses synthetic configuration from [`examples/repo-com.example.toml`](../examples/repo-com.example.toml). Paths and timestamps are supplied by the embedding application; the libraries do not read a Discord token or contact Discord.

### 1. Parse and resolve configuration

Use `ConfigResolver` or the `parse_config`/`resolve_path` functions. Discovery searches from the current directory through ancestors, stops at the nearest `.git` marker, and rejects zero or multiple `.repo-com.toml` candidates.

```rust
use std::path::Path;

let resolved = repo_com_config::resolve_path(Path::new(
    "examples/repo-com.example.toml",
))?;

let repository_id = resolved.config.repository_id.clone();
let workspace_id = resolved.config.discord.workspace_id.clone();
let config_hash = resolved.canonical_hash();
```

When repository-bounded discovery is used, an explicit path is normalized and must remain inside the supplied repository root. The resolver never creates a configuration file.

### 2. Open and register local state

`StateStore::open()` uses the OS user-data path. `open_path()` is useful for an explicit application-controlled path and for isolated tests. A file-backed store creates the parent and database with the platform's user-only protection model, enables foreign keys and WAL, applies the forward migration, and verifies the resulting schema.

```rust
use repo_com_state::{RepositoryInput, StateStore};

let mut store = StateStore::open()?;
store.upsert_repository(&RepositoryInput::new(
    repository_id,
    workspace_id,
    config_hash,
    "2026-01-01T00:00:00Z",
))?;
```

Register the repository before writing repository-scoped state. All state records are keyed by the configured repository ID, so two repositories may safely use the same local database without cross-scope reads.

### 3. Preview and activate one exact policy

A policy is only eligible when its `event_type`, destination alias, and severity exactly equal the configured tuple. Wildcards, prefixes, broader severity labels, and duplicate configuration entries are rejected; ambiguous active rows deny evaluation rather than selecting one. Configuration and tuple hashes are canonical SHA-256 values.

```rust
use repo_com_foundation::TtyMode;
use repo_com_policy::{
    OperatorConfirmation, PolicyRegistry, PolicyTuple,
};

let mut registry = PolicyRegistry::new(&mut store);
let tuple = PolicyTuple::from_entry(&resolved.config.auto_send[0]);
let preview = registry.preview(&resolved.config, &tuple, None)?;

let receipt = registry.activate(
    &resolved.config,
    &tuple,
    OperatorConfirmation::confirmed(TtyMode::Tty)?,
    "2026-01-01T00:00:01Z",
)?;

assert_eq!(preview.config_hash, receipt.config_hash);
assert_eq!(preview.tuple_hash, receipt.tuple_hash);
```

`PolicyRegistry` never probes stdin or stdout. The caller must collect and pass the operator confirmation. A non-TTY caller receives `PolicyError::TtyRequired` before activation is attempted.

### 4. Inspect, evaluate, and deactivate

Status and evaluation are read-only and do not require a TTY. A matching activation is eligible only when its stored configuration and tuple hashes still match the current values.

```rust
let status = registry.status(&resolved.config, &tuple)?;
let decision = registry.evaluate(&resolved.config, &tuple)?;

let _policy_gate_satisfied = decision.is_eligible();

let deactivated = registry.deactivate(
    &resolved.config.repository_id,
    &receipt.activation_id,
    "2026-01-01T00:00:02Z",
)?;
assert!(!deactivated.active);
```

Deactivation is permission-reducing and does not require a TTY. A changed configuration, destination, mention list, retention value, or policy tuple makes an old activation stale. Multiple current matching activations produce an ambiguous result rather than expanding authority.

### 5. Prepare a future protocol result

The foundation crate creates protocol-version-1 values; it does not write to process streams. A future executable must own stdout and stderr and must keep the streams separate.

```rust
use repo_com_foundation::CommandOutcome;

let outcome = CommandOutcome::success("configured");
let streams = outcome.output_streams(None)?;
assert_eq!(streams.stdout(), r#"{"protocol_version":1,"status":"success","data":"configured","error":null}"#);
assert_eq!(streams.stderr(), "");
```

Machine output is exactly one JSON object with `protocol_version`, `status`, `data`, and `error`. `GlobalArgs::output_format` can be set to `OutputFormat::Json`; the foundation still does not write to process streams. Human output, prompts, and the final executable are not implemented yet.

The current typed error categories and their deterministic exit codes are:

| Category | Exit code |
|---|---:|
| `usage-schema` | 2 |
| `operator-action-required` | 3 |
| `policy-blocked` | 4 |
| `authentication` | 5 |
| `permission` | 6 |
| `remote-conflict` | 7 |
| `unknown-delivery` | 8 |
| `storage-integrity` | 9 |
| `connectivity-rate-limit` | 10 |
| `internal-failure` | 1 |

## Current feature reference

| Area | Current API or behavior | Not implemented |
|---|---|---|
| Foundation | `GlobalArgs`, `TtyMode`, `CommandOutcome`, `ErrorCategory`, `OutputStreams` | CLI parsing, process I/O, terminal rendering |
| Configuration | `parse_config`, `ConfigResolver`, `ResolvedConfig`, canonical SHA-256 hash | Discord workspace discovery and remote validation |
| State | `StateStore`, schema version 1, repository-scoped repositories, drafts, approvals, policy rows, delivery attempts, inbound records, audit rows, transactions | Remote delivery, inbound fetching, retention, purge, lifecycle commands |
| Policy | `PolicyTuple`, `PolicyRegistry::preview`, `activate`, `status`, `evaluate`, `deactivate` | Final send eligibility, approval, and Discord transport |
| Protocol | Protocol version 1 success/error envelopes and deterministic category exit codes | Executable output routing and handler wiring |

## Configuration

The current schema is strict and versioned. All of these top-level sections are required:

```toml
schema_version = 1
repository_id = "acme/widgets"

[discord]
workspace_id = "123456789012345678"

[destinations.release]
channel_id = "234567890123456789"
allowed_mentions = ["oncall"]

[mentions.oncall]
target = "role:345678901234567890"

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365

[[auto_send]]
event_type = "build_failed"
destination = "release"
severity = "high"
```

The validator:

- accepts only schema version `1`;
- rejects unknown fields, missing sections, duplicate aliases, duplicate exact policy tuples, wildcard syntax, malformed IDs, and zero retention values;
- accepts mention targets only as `role:<id>` or `user:<id>`;
- requires inbound aliases to name configured destination aliases;
- rejects secret-like keys without echoing their values; and
- can reject known cross-workspace references when a caller supplies a `WorkspaceReferenceIndex`.

The canonical hash sorts map and list order before serializing, so formatting and declaration order do not change the hash. The hash includes the complete normalized configuration, including destinations, mentions, inbound settings, retention, and auto-send entries.

## Safety and data handling

- **No secret in configuration:** secret-like field names are rejected, and error rendering does not retain parsed values.
- **No token is consumed yet:** `REPO_COM_DISCORD_TOKEN` is a future product variable, not an input read by the current crates.
- **Repository scope:** state records and policy lookups require the exact repository ID.
- **Immutable evidence:** the database triggers prevent updates or deletes to draft revisions, first inbound snapshots, and audit events.
- **Transaction boundary:** state convenience mutations use an explicit SQLite transaction and roll back on typed failure.
- **Local storage:** the state database is not encrypted by this implementation. On Unix it is protected with owner-only mode bits; on Windows the inherited user-profile ACL is the boundary.
- **No remote side effects:** the current libraries do not call Discord, send messages, fetch channels, or collect telemetry.

A future send layer must revalidate approval, destination, policy, revision, and safety state immediately before any network request. The current policy decision is not a send.

## Troubleshooting

| Symptom | Likely cause | Corrective action |
|---|---|---|
| `config-not-found` | No `.repo-com.toml` exists between the start directory and repository root | Place one valid file in the repository or pass an explicit path within the root. |
| `multiple-config-candidates` | More than one configuration exists in the bounded ancestor path | Remove or explicitly select the intended file. |
| `unsupported-schema-version` | The document is not schema version 1 | Update the configuration to the supported schema. |
| `secret-field` or `raw-destination-field` | A forbidden key or raw destination field was supplied | Move the value to the future operator-controlled environment boundary and use aliases in configuration. |
| `operator-action-required` / `TtyRequired` | An authority-creating action was requested without a TTY confirmation | Re-run through an interactive caller that supplies `TtyMode::Tty`; automation must not self-approve. |
| `policy-blocked` | The tuple is not configured, is stale, or is ambiguous | Inspect status, correct the exact tuple/configuration, or deactivate ambiguous rows before a fresh operator activation. |
| `storage-integrity` | Migration, SQLite, lock, or runtime-boundary failure | Preserve the database, inspect the typed state error, and do not delete or recreate it automatically. |
| JSON serialization failure | A caller supplied a payload that cannot be serialized | Use a serializable payload and keep diagnostics in the separate stderr value. |

There is no current executable error display; library errors expose typed categories, paths, and safe messages according to their crate APIs.

## Planned product workflow

The intended command-level workflow—draft, preview, approve or activate policy, send, fetch, reply, acknowledge, audit, retain, and purge—remains specified in the [Product Vision](PRD.md) and [feature documents](features/). Do not treat those command names or transport claims as available in version `0.1.0`.

## Further help

- [Administrator Guide](admin-guide.md)
- [Architecture Decision Records](adr/README.md)
- [Changelog](../CHANGELOG.md)
- [Unreleased release notes](releases/unreleased.md)
- [Configuration example](../examples/repo-com.example.toml)
