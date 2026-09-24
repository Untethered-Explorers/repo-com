<div align="center">

# repo-com

**A single-user, local transport and approval workflow for agent-originated team communications through Discord.**

[![Status: pre-release](https://img.shields.io/badge/status-pre--release-orange)](#project-status)
[![Planned toolchain: Rust 1.98.1](https://img.shields.io/badge/toolchain-Rust%201.98.1-000000)](#architecture)
[![Providers: Discord only](https://img.shields.io/badge/providers-Discord%20only-5865F2)](#scope)

</div>

`repo-com` is a command-line application that lets a repository skill ask a
teammate a focused question through Discord — and lets the operator preview,
approve, deliver, retrieve, and audit the exchange. It is a safety layer for
agent-originated requests, not a general-purpose chat client.

> [!WARNING]
> **Pre-release specification.** `repo-com` is fully specified but **not yet
> implemented**. As of 2026-09-24 there is no source code, binary, tag, or
> release, and none of the commands or configuration below has been executed.
> This repository describes planned v1 behavior. Start with the
> [Product Vision](docs/PRD.md) and the [feature specifications](docs/features/).

## Features

- **Explicit destinations** — one configured channel alias per draft, with
  allowlisted role and user mentions. No broadcasts, DMs, or raw IDs.
- **Immutable, previewed drafts** — each revision is content-hashed, shown
  exactly, and never mutated after approval.
- **Exact-revision approval** — human consent is bound to one revision and a
  15-minute window. A skill cannot approve or widen its own permissions.
- **Narrow auto-send** — only an operator-activated exact event/type/destination
  policy may send without per-message approval.
- **Duplicate-safe delivery** — an atomic local claim before network I/O, a
  deterministic nonce, and read-only reconciliation of ambiguous outcomes.
- **Untrusted inbound** — on-demand, bounded retrieval of replies and mentions
  that can never approve a draft or trigger a send.
- **Local-first privacy** — one user-level SQLite store, bounded retention
  (30 days content / 365 days metadata), confirmed purge, and no telemetry.
- **Accessible terminal UI** — keyboard-operable, labeled, 80-column, and
  `NO_COLOR`-aware, with a versioned JSON protocol for automation.

## How it works

1. A skill creates a draft targeting one configured destination alias.
2. The operator previews the exact resolved text, metadata, and decision basis.
3. The operator approves the revision, or an activated exact policy matches it.
4. `repo-com` atomically claims the send, delivers it, and records the outcome —
   `accepted`, `failed`, `retry_wait`, or `unknown`.
5. A later invocation fetches replies and mentions, stored as untrusted items.
6. The skill drafts a validated threaded reply; it passes the same gates.
7. Items are acknowledged or archived locally, and the audit trail is inspected.

```text
draft → preview → approve / policy → atomic claim → send
      → accepted | failed | unknown → reconcile before any resend
      → fetch reply → validated reply draft → acknowledge locally
```

## Scope

| In v1 | Out of v1 |
|---|---|
| One Discord workspace per repository | Email and other providers |
| Immutable drafts and exact approval | A daemon, gateway, or live inbox UI |
| Operator-activated exact auto-send policy | Broadcasts, arbitrary DMs, scheduling |
| Duplicate-safe delivery with reconciliation | Attachments, embeds, reactions, files |
| On-demand bounded reply/mention retrieval | Full-history search and read receipts |
| Bounded retention and confirmed local purge | Encryption at rest and OS keychain |
| Versioned JSON protocol and accessible TUI | Telemetry and self-updating binaries |

See [PRD §3](docs/PRD.md#3-goals-and-non-goals) for complete goals and non-goals.

## Configuration

Repository behavior is described by a committed, secret-free `.repo-com.toml`
(schema version 1). The bot token is supplied only through the
`REPO_COM_DISCORD_TOKEN` environment variable.

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

## Architecture

Planned as a pinned Rust 2024 workspace composed of focused `repo-com-*` crates
for configuration, state, policy, drafting, approval, Discord, delivery, inbox,
reply, retention, terminal, and release policy. Release artifacts statically
link a patched SQLite (3.53.4+) and target Linux, macOS, and Windows.

The core design decisions — repository-scoped local state, a versioned JSON
protocol, explicit TTY approval, atomic delivery claims, untrusted inbound data,
and bounded retention — are recorded in the [Architecture Decision Records](docs/adr/README.md).

## Documentation

| Document | Purpose |
|---|---|
| [Product Vision (PRD)](docs/PRD.md) | Canonical requirements, architecture, security, and lifecycle |
| [Feature specifications](docs/features/) | Per-feature requirements and executable `forge-task` contracts |
| [User Guide](docs/user-guide.md) | Task-oriented operator and skill workflows |
| [Administrator Guide](docs/admin-guide.md) | Prerequisites, configuration, secrets, operations, hardening |
| [Architecture Decision Records](docs/adr/README.md) | Durable v1 architectural decisions and rationale |
| [Release notes](docs/releases/) | Versioned release notes (currently draft only) |
| [Change history](CHANGELOG.md) | Repository and, later, product changes |
| [Project idea (historical)](docs/IDEA.md) | Original source material; not an execution source |

## Repository layout

```text
docs/
  PRD.md                 Canonical product vision and shared requirements
  features/              Seven feature specs (sole owners of detailed tasks)
  adr/                   Architecture decision records
  releases/              Release notes index and drafts
  user-guide.md          Pre-release user guide
  admin-guide.md         Pre-release administrator guide
  IDEA.md                Historical idea (not an execution source)
.opencode/
  agents/                Generated project agent team
  skills/                Generated reusable project skills
```

## Project status

No tagged release exists. Planned implementation crates (`crates/repo-com-*`)
and CI/release workflows do not exist yet; their intended names appear in
[PRD §7.2](docs/PRD.md#72-project-structure). The current state is tracked in the
[changelog](CHANGELOG.md) and the [draft release notes](docs/releases/unreleased.md).
