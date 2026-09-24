# Library Consumer Guide

> **Status:** workspace `0.1.0`, pre-release. This guide documents the currently implemented Rust library surface. There is no installable `repo-com` command or operator UI yet. The library contracts below are available to an embedding Rust application; the planned command workflow is kept separate.

## Overview

`repo-com` is a local, repository-scoped foundation for agent-originated Discord communication. The current workspace provides focused contracts for:

- protocol and error values;
- strict repository configuration and deterministic hashes;
- repository-scoped SQLite state and audit evidence;
- exact operator-activated policy;
- immutable drafts, deterministic content rendering, and pre-send safety scanning;
- exact-revision approval and send eligibility;
- dedicated-bot Discord REST v10 setup and message operations;
- local delivery claims, bounded retry, and read-only unknown reconciliation;
- bounded untrusted inbound retrieval and validated reply drafts; and
- local content and metadata retention sweeps.

These crates are not composed into a final executable. A consumer must own the
call sequence, transaction boundaries, clocks, explicit TTY confirmation, and
network decisions between contracts.

### Current audiences

- **Library consumer:** a Rust developer embedding one or more current crates.
- **Embedding application maintainer:** the developer responsible for composing
  state, approval, transport, and local lifecycle operations.
- **Operator:** the person who supplies an explicit TTY confirmation for an
  authority-creating action. The libraries do not detect or prompt on a terminal
  themselves.

There is no current end-user command workflow. Teammates, channel operators, and
repository skills cannot invoke a shipped `repo-com` binary in this release
boundary.

## Before you begin

There is no installation artifact. From a local checkout, verify the workspace
with:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The current full nextest run selected **195 tests across 19 binaries: 195 passed
and 2 skipped**. These are component and mocked-contract checks, not a live
Discord or end-to-end product validation.

A consumer that invokes the production Discord adapters also needs network
access and a raw dedicated bot token in `REPO_COM_DISCORD_TOKEN`. Do not put a
token in `.repo-com.toml`, source code, fixtures, logs, or documentation.

## Core library workflow

The following sequence is a composition guide, not a single library method. The
caller must preserve the exact identifiers, hashes, timestamps, and transaction
boundaries between steps.

### 1. Resolve configuration

Use `ConfigResolver`, `resolve_path`, or `resolve_current` to load a
schema-version-1 `.repo-com.toml`. Discovery searches from the supplied current
directory through ancestors, stops at the supplied repository root, and rejects
zero or multiple candidates. An explicit path must remain inside that root.

```rust
use std::path::Path;

let resolved = repo_com_config::resolve_path(Path::new(
    "examples/repo-com.example.toml",
))?;
let repository_id = resolved.config.repository_id.clone();
let workspace_id = resolved.config.discord.workspace_id.clone();
let config_hash = resolved.canonical_hash();
```

The smallest example is [`examples/repo-com.example.toml`](../examples/repo-com.example.toml).
The validator rejects unknown fields, secret-like keys, raw destination fields,
duplicate aliases, malformed IDs, wildcard policy values, and unsafe retention
values. Resolved aliases are the only normal workflow destinations.

### 2. Open and register local state

`StateStore::open()` uses the OS user-data path. `open_path()` is intended for
isolated tests or a caller-controlled deployment path; do not place operational
repository state in the repository tree. A file-backed store enables foreign
keys and WAL, applies the forward migration, verifies the schema, and creates
user-only storage on Unix.

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

Register the repository before writing repository-scoped state. All records use
the exact configured repository ID, so separate repositories can share one local
database without cross-scope reads.

### 3. Create and render a draft

`DraftModel::create` creates revision 1; `revise` appends a new immutable
revision. Revisions bind the exact text, metadata, event type, severity, resolved
destination, optional authorized reply reference, expiry, and canonical content
hash. Draft expiry defaults to 24 hours and is capped at seven days.

`ContentRenderer` normalizes text, resolves only configured mention aliases,
enforces the Discord character limit, and appends a deterministic delivery nonce.
`ContentRenderer::new()` has no bot identity; use `for_bot_user_id` when the
embedding application has a setup-proven bot ID and must reject a configured user
target that names that bot. Rendering is pure and performs no network I/O.

```rust
let draft = repo_com_draft_model::DraftModel::create(
    request,
    &resolved,
    created_at_unix_seconds,
)?;
let rendered = repo_com_draft_content::ContentRenderer::new()
    .render(draft.current_revision(), created_at_unix_seconds)?;
```

The rendered text is the exact text that a later transport request must use. Do
not edit it after approval or eligibility checks.

### 4. Scan, preview, and establish authority

`SecretScanner` scans rendered text and bounded metadata for high-confidence
credential patterns. A finding blocks eligibility unless an interactive
operator records a redacted override bound to the exact preview. The scanner
does not provide complete data-loss prevention and does not retain matched values.

`ApprovalService::preview` builds a complete current preview. `approve` requires
an `OperatorConfirmation` bound to the preview hash and explicit
`TtyMode::Tty`; replaying the same approval is idempotent and does not extend the
original expiry. The approval lifetime is the earlier of 15 minutes and the draft
expiry.

`PolicyRegistry` is the separate exact-policy path. It matches only the exact
event type, destination alias, and severity. Activation requires an explicit TTY
confirmation and stores canonical configuration and tuple hashes. A changed hash
makes the activation stale; ambiguous active rows deny the policy decision.
Policy eligibility is not final send eligibility.

### 5. Evaluate eligibility and claim locally

`EligibilityEvaluator` and `evaluate` revalidate the current configuration,
immutable revision, destination, expiry, approval or policy authority, and scan
result. An eligible result carries the hashes that the delivery coordinator must
repeat at its atomic claim boundary. The evaluator performs no claim and no
network I/O.

`DeliveryCoordinator::claim` commits the local claim, current-state checks, and
matching audit evidence before returning a single-use permit for a new transport
call. Concurrent or repeated callers receive the recorded attempt rather than a
second permit. This is local duplicate protection, not transport-level exactly
once delivery.

### 6. Use the Discord adapter and recovery contracts

`DiscordMessageClient::from_environment` reads the dedicated bot token from
`REPO_COM_DISCORD_TOKEN`, pins REST v10, validates the resolved destination and
allowlisted mentions, and performs exactly one HTTP message attempt. It does not
sleep or retry. Classify its result as accepted, definitive failure, pre-dispatch
failure, rate limit, or ambiguous post-dispatch outcome.

`RetryPolicy` and `RetryRunner` provide a bounded policy with at most three total
transport attempts, safe pre-dispatch waits, and server-directed Discord waits
capped at 30 seconds. `DeliveryRecovery::record_decision` records a completed
classification in the local delivery state machine. The current libraries do not
provide a complete restart-safe coordinator that persists every intermediate
attempt and recovery decision; callers must retain those boundaries explicitly.

For an unknown result, use `RecoveryReconciler` for a read-only search. A safe
match requires the configured destination, bot author, request/content nonce,
and exact content. Absence requires the configured time window and multiple
successful reads. Conflicting, incomplete, or unresolved evidence must not
authorize a resend. Reconciliation does not mutate Discord.

### 7. Fetch and process inbound data

`DiscordInboundClient` and the inbound fetch contracts use only enabled inbound
aliases. A request requires exactly one validated last-event-ID cursor or RFC
3339 boundary. The fetch is bounded by ten pages, 1,000 raw messages, and 100
point checks, and returns continuation metadata when more data exists.

The filter retains human replies to accepted same-repository deliveries or direct
mentions of the configured bot. Returned envelopes are explicitly untrusted;
attachment indicators are metadata, not downloaded bytes. The local state service
preserves the first snapshot, records current edit/delete state, and exposes
idempotent local acknowledgement and archive operations.

The current adapter exposes page storage/cursor handling and point reconciliation
as separate phases. A consumer must not treat a committed page as proof that all
later point checks completed in one product transaction.

### 8. Create a reply draft and query evidence

`ReplyCommandService::create_reply` validates a stored inbound target, creates
one immutable linked reply draft, and records the local link. It does not send a
message and does not create approval, policy, safety, or delivery authority.
A reply marked replied requires a linked accepted delivery and matching durable
audit evidence.

Use `AuditQuery` for bounded, read-only, repository-scoped evidence. Queries
reapply redaction, support stable chronological pagination, and expose a
continuation cursor. They do not export state, contact Discord, or provide
read-receipt analytics.

### 9. Run retention before new mutations

`RetentionSweeper` computes deterministic UTC cutoffs from a validated policy.
Defaults retain content for 30 days and metadata for 365 days. Configured content
retention is 1–365 days, metadata retention is 30–3,650 days, and metadata
retention cannot be shorter than content retention.

`sweep` and `run_before_mutation` are repository-scoped and transactional. Expired
content is replaced with `[content-expired]` while identifiers, hashes,
timestamps, links, and audit evidence remain. Later metadata expiry removes
non-content rows. A failed sweep blocks the new mutation. The crate has no
Discord, telemetry, scheduler, backup, or remote-mutation capability.

Purge planning/execution and read-only lifecycle inspection are not implemented
in the current workspace.

## Current feature reference

| Area | Current library behavior | Not currently available |
|---|---|---|
| Foundation | `GlobalArgs`, `TtyMode`, `CommandOutcome`, `ErrorCategory`, `OutputStreams` | Process routing, command parsing, terminal rendering |
| Configuration | Strict schema 1, bounded discovery, aliases, canonical SHA-256 hash | Remote workspace validation through a shipped command |
| State | Repository-scoped SQLite, migration 1, transactions, immutable evidence, retention hooks | Backup, repair, purge, or final lifecycle command layer |
| Policy | Exact tuple matching, TTY-confirmed activation, status, staleness, deactivation | Automatic policy activation or final send composition |
| Drafts and approval | Immutable revisions, rendered previews, nonce, safety scan, exact approval, eligibility | CLI draft creation, prompts, or a complete human workflow |
| Discord | Dedicated-bot setup checks, one-attempt message adapter, inbound reads, typed errors and rate limits | A user-facing setup/send/fetch command and live acceptance |
| Delivery | Local duplicate-safe claim, state transitions, retry classification, read-only reconciliation | Restart-safe complete coordinator and automatic end-to-end retry handling |
| Inbound and reply | Bounded untrusted fetch, local lifecycle, validated reply draft/link | Final inbox UI, remote mutation, or automatic reply send |
| Audit and retention | Redacted append/query APIs and transactional retention sweeps | Telemetry, remote synchronization, purge, and lifecycle inspection |

## Safety, privacy, and accessibility

- **Credentials:** `REPO_COM_DISCORD_TOKEN` is the only Discord token input for
  the current adapters. It must be a raw dedicated bot token; configuration,
  source, logs, and examples remain secret-free.
- **Network boundary:** a production client can make HTTP requests when the
  embedding application calls it. Read-only setup and inbound contracts do not
  mutate Discord; message delivery does one POST and never retries internally.
- **Approval boundary:** approval is bound to the exact revision, text, metadata,
  destination, policy basis, configuration hash, and scan result. Any change
  invalidates it.
- **Inbound trust:** remote messages, mentions, edits, and deletions are
  untrusted data. They cannot approve, activate policy, override safety, or
  authorize a send.
- **Local state:** the database is outside the repository by default, uses
  user-only filesystem protection on Unix, and is not encrypted at rest. Backups
  and snapshots need equivalent access controls.
- **Privacy:** the current workspace has no telemetry or remote state
  synchronization. Retention sweeps preserve non-content evidence while
  replacing expired content.
- **TTY and accessibility:** libraries accept an explicit TTY mode and never
  probe or prompt themselves. There is no current terminal renderer, color mode,
  keyboard prompt, or 80-column product surface to validate. The accessibility
  contract remains a planned release requirement.

## Troubleshooting

| Symptom | Likely cause | Corrective action |
|---|---|---|
| `config-not-found` | No `.repo-com.toml` exists in the bounded search range | Place a valid file in the repository or pass an explicit path inside the root |
| `multiple-config-candidates` | More than one configuration exists in the ancestor range | Remove the extra file or select one explicit path |
| `unsupported-schema-version` | The document is not schema version 1 | Update the file using the example |
| `secret-field` or `raw-destination-field` | A forbidden key or raw destination was supplied | Move secrets to the environment boundary and use named aliases |
| `operator-action-required` or `TtyRequired` | An authority-creating action was requested without explicit TTY confirmation | Supply `TtyMode::Tty` only after the caller has shown the complete preview; automation must not self-approve |
| `policy-blocked` | The tuple is absent, stale, deactivated, or ambiguous | Inspect current hashes and status, correct the configuration, or deactivate the conflicting record before any new activation |
| Approval or eligibility blocker | The preview, revision, destination, expiry, scan, or policy basis changed | Build a new preview and approval; do not reuse an old confirmation |
| `delivery-blocked` | Eligibility or a claim precheck no longer matches the exact facts | Rebuild the current preview and claim; never bypass the revalidation boundary |
| `authentication-failed` or `permission-denied` | Discord rejected the bot credential or required access | Rotate the dedicated token or grant the least-privilege Discord permissions reported by setup; never use a user token |
| `connectivity-rate-limit` | A bounded request failed or Discord returned a dynamic limit | Honor the returned delay; do not guess a reset time or retry an ambiguous send |
| `storage-integrity` | SQLite, migration, lock, retention, or transaction failure | Preserve the database and sidecars, inspect the typed error, and do not delete or recreate state automatically |
| Unknown delivery | Dispatch may have reached Discord but the response was lost | Keep the attempt blocked and perform read-only reconciliation; do not resend from the unknown state |

There is no current executable error display. Library errors expose typed
categories, safe messages, and caller-owned remediation data.

## Planned product workflow

The intended command-level workflow—draft, preview, approve or activate policy,
send, fetch, reply, acknowledge, audit, retain, and purge—remains specified in
the [Product Vision](PRD.md) and [feature documents](features/). Those documents
are requirements and roadmap material, not installation or runtime instructions
for version `0.1.0`.

## Further help

- [Administrator Guide](admin-guide.md)
- [Architecture Decision Records](adr/README.md)
- [Changelog](../CHANGELOG.md)
- [Unreleased release notes](releases/unreleased.md)
- [Configuration example](../examples/repo-com.example.toml)
