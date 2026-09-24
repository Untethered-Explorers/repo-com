# repo-com User Guide

> **Status: pre-release specification (2026-09-24).** `repo-com` is specified
> but **not implemented**. There is no installable binary, no tagged release, and
> none of the commands below has been executed. This guide documents the
> canonical v1 contract from [`docs/PRD.md`](PRD.md) and
> [`docs/features/`](features/). Treat every command as **planned behavior**.
> Exact subcommand names, flags, and JSON fields are **not yet frozen**; they
> will be defined by task `REL-APP-1` and documented in the future
> `docs/operator-guide.md`. See [Unreleased release notes](releases/unreleased.md).

## Overview

`repo-com` is a single-user, local command-line transport and approval layer
that lets a repository skill ask a teammate a focused question through Discord,
then retrieve and reply to the answer on demand. It is not a general-purpose
chat client: it keeps explicit destinations, exact-review approval, durable
local state, and duplicate-safe delivery. See the
[Product Vision](PRD.md) and the [ADRs](adr/README.md) for rationale.

Audiences:

- **Repository skill** — a programmatic, non-TTY caller that creates drafts and
  fetches inbound replies using the versioned JSON protocol.
- **Operator** — the person who owns credentials, approves exact revisions, and
  controls retention and purge.
- **Teammate** — a Discord recipient who does not use `repo-com` and simply
  replies in channel.

## Install and first use

There is **no install path yet**. The planned release publishes versioned
Linux, macOS, and Windows binaries with checksums and a CycloneDX SBOM. Until a
release exists, no install, first-run, or `--version` output can be shown. The
planned first-run sequence is:

1. Create a dedicated Discord bot and set `REPO_COM_DISCORD_TOKEN` in the
   environment (see the [Administrator Guide](admin-guide.md)).
2. Add a committed `.repo-com.toml` (see [Configuration](#configuration)).
3. Run the read-only Discord setup check to validate identity, workspace
   membership, channels, permissions, and mention access.
4. Create a draft, preview it, approve the exact revision, and send.

## Core workflow

The planned happy path, mirroring [PRD §6.1](PRD.md#61-core-loop):

1. **Create a draft.** A skill or operator submits one draft targeting one
   configured destination alias with text and bounded metadata (event type,
   severity, optional repository label, branch, commit, optional inbound reply
   reference). Broadcasts and multiple destinations are rejected.
2. **Preview.** The exact resolved destination, final text, metadata, expiry,
   approval or policy basis, secret-scan status, and a typed send decision are
   shown without mutating state or Discord.
3. **Approve or match policy.** In an interactive TTY, an operator approves the
   exact revision, or a previously activated exact policy makes it eligible.
   Approval is bound to the revision hash and expires at the earlier of draft
   expiry or 15 minutes.
4. **Send.** Delivery is claimed atomically, the message is sent through the
   dedicated bot, and the outcome is recorded as `accepted`, `failed`,
   `retry_wait`, or `unknown`.
5. **Fetch replies.** A later invocation reads enabled inbound channels from an
   explicit cursor or time boundary and stores replies and mentions as untrusted
   items.
6. **Reply.** The skill creates a validated threaded reply draft, which then
   passes the same approval, policy, safety, and delivery gates as any draft.
7. **Acknowledge or archive** inbound items locally, and inspect the audit trail.

## Command reference

The planned v1 command groups (exact syntax not frozen) are:

| Group | Purpose | Notable operations |
|---|---|---|
| `config` | Validate committed repository configuration | validate, show resolved aliases |
| `policy` | Exact auto-send policy status and activation | status, activate (TTY only), deactivate |
| `draft` | Immutable draft revisions | create, show, update, preview |
| `send` | Approval and delivery | approve (TTY only), send |
| `inbox` | Inbound retrieval and local lifecycle | fetch, show, acknowledge, archive |
| `reply` | Create threaded reply drafts from stored items | create |
| `audit` | Bounded local audit queries | query |
| `state` | Read-only database and lifecycle verification | verify, inspect |
| `purge` | Retention and purge planning/execution | plan, execute (TTY only) |

Every command that requires operator confirmation returns an
operator-action-required outcome instead of prompting in a non-TTY shell.
Machine mode emits exactly one JSON protocol object on stdout; diagnostics go to
stderr. See [ADR-003](adr/ADR-003-versioned-json-protocol.md).

### Machine protocol (protocol version 1)

Planned envelope fields: `protocol_version`, `status`, `data`, and `error`.
Stable error categories map to process exit codes: usage/schema, approval or
operator action required, policy blocked, authentication, permission, remote
conflict, unknown delivery, storage integrity, connectivity/rate limit, and
internal failure.

### Delivery outcomes

| Outcome | Meaning |
|---|---|
| `accepted` | Discord acknowledged the message; immutable remote message exists |
| `failed` | Definitive rejection; no message created |
| `retry_wait` | Proven pre-dispatch failure or HTTP 429; a bounded retry may follow |
| `unknown` | Ambiguous post-dispatch result; blocked until reconciled, never auto-resent |
| `accepted` (via reconciliation) | An `unknown` outcome was matched to an existing bot message |
| `reconciled_absent` | An `unknown` outcome was proven absent within a five-minute, three-read window |
| `unresolved` | Conflicting or insufficient evidence; remains blocked |

`repo-com` guarantees **at most one remote message per immutable draft
revision** via a local atomic claim before network I/O. This is duplicate-safe
delivery with an idempotent local effect, **not** a transport-level exactly-once
guarantee from Discord.

## Configuration

A committed, non-secret `.repo-com.toml` (schema version 1) is discovered by
searching the current directory and ancestors up to the repository root. Unknown
keys, duplicate aliases, unsafe schema versions, cross-workspace references,
invalid mention prefixes, and secret-like fields fail validation. The smallest
valid example:

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

The bot token is **never** placed in this file; it is supplied only through the
`REPO_COM_DISCORD_TOKEN` environment variable. Normal skill-facing commands
accept aliases, never raw Discord channel, role, or user IDs.

## Safety and data handling

- **Exact preview and approval.** Approval binds to one immutable revision; any
  change to text, metadata, destination, repository, or expiry invalidates it.
- **Sent messages are immutable.** Corrections and follow-ups create a new draft
  or a validated threaded reply. There is no outbound edit or delete.
- **Auto-send is narrow.** Only an exact event-type + destination + severity
  tuple that an operator activated can send without per-message approval.
- **Inbound is untrusted.** Replies and mentions cannot approve a draft,
  activate policy, or trigger a send.
- **Retention and purge are local-only.** Defaults are 30 days for content and
  365 days for metadata. Purge requires a non-mutating plan and TTY confirmation,
  and never deletes Discord messages.
- **No telemetry, no encryption at rest.** State is protected by user-only
  filesystem permissions only. Local account access, backups, and filesystem
  snapshots can still read retained content.

## Troubleshooting

Because no binary exists, there are no verified user-visible errors yet. The
planned behaviors to expect:

| Planned symptom | Likely cause | Planned next action |
|---|---|---|
| Configuration rejected | Unknown key, unsafe schema version, duplicate alias, or secret-like field | Fix `.repo-com.toml`; errors are path-aware and do not echo secrets |
| Operator action required | Approval, policy activation, or override attempted in non-TTY mode | Re-run interactively in a TTY |
| Authentication failure | Missing, invalid, or revoked `REPO_COM_DISCORD_TOKEN` | Rotate the bot token and set the environment variable |
| Permission / not-found | Missing `VIEW_CHANNEL`, `SEND_MESSAGES`, or `READ_MESSAGE_HISTORY`, wrong channel/workspace | Run the read-only setup check and grant the documented permissions |
| Policy blocked | No exact approval and no matching activated policy | Approve the revision interactively or activate the exact policy |
| Unknown delivery | Post-dispatch timeout, reset, or 5xx | Reconcile read-only; do not resend until resolved |
| Storage integrity error | Failed retention sweep, corruption, or unsupported schema | Inspect with `state verify`; the database is never auto-deleted |

## Further help

- [README](../README.md) — documentation index
- [Product Vision](PRD.md) and [feature specifications](features/)
- [Architecture Decision Records](adr/README.md)
- [Administrator Guide](admin-guide.md)
- [Changelog](../CHANGELOG.md) and [release notes](releases/)
