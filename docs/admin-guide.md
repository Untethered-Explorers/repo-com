# repo-com Administrator Guide

> **Status: pre-release specification (2026-09-24).** `repo-com` is specified
> but **not implemented**. There is no installable binary, release artifact,
> CI workflow, or packaged database. This guide documents the canonical v1
> operating contract from [`docs/PRD.md`](PRD.md) and
> [`docs/features/`](features/); treat every command and path as **planned**.
> Exact flags and file paths may change before release. See
> [Unreleased release notes](releases/unreleased.md).

## Responsibilities and Architecture

An administrator (often the same person as the operator) is responsible for:

- Creating a **dedicated Discord bot** and granting least-privilege permissions.
- Supplying the bot token through the environment, never through configuration.
- Committing a valid, non-secret `.repo-com.toml` per repository.
- Protecting the local SQLite state file and its containing directory.
- Planning and executing retention and purge operations.

Architecture in one line: one globally installed `repo-com` binary per machine
composes focused Rust crates and stores repository-scoped state in a single
user-level SQLite database; it talks only to Discord REST API v10 as a bot. See
[ADR-001](adr/ADR-001-rust-workspace-and-patched-sqlite.md),
[ADR-002](adr/ADR-002-repository-scoped-local-state.md), and
[ADR-006](adr/ADR-006-dedicated-discord-bot-rest-v10.md).

## Prerequisites

- **Planned platform support:** Linux, macOS, and Windows (x86_64 and platform
  targets defined by `REL-PACK-1`, not yet published).
- **Discord:** a workspace where the administrator can create a bot and grant
  channel permissions.
- **Runtime database:** SQLite **3.53.4 or newer**, statically linked in release
  artifacts with a runtime version assertion. An older bundled or system SQLite
  is not acceptable.
- **No daemon:** there is no server, gateway, or background service to run.

## Installation

No install path exists yet. The planned release publishes versioned binaries,
archives or installers, SHA-256-or-stronger checksums, dependency-license
evidence, and a CycloneDX SBOM, with **no self-updater**. Installation will be an
explicit operator action:

1. Download the artifact for the platform from the release.
2. Verify the SHA-256 checksum against the published value.
3. Place the `repo-com` binary on `PATH`.
4. Confirm `repo-com --version` prints the package semantic version.

Do not treat a development build as a release artifact; release builds must
assert the patched SQLite version at compile and run time.

## Configuration

Repository configuration is a committed, reviewable `.repo-com.toml` (schema
version 1). It defines the workspace, destination and mention aliases, inbound
aliases, retention, and exact auto-send policy entries — and **never** a secret.
See the [user guide configuration section](user-guide.md#configuration) for a
minimal example.

Resolution rules:

- Prefer an explicit normalized path; otherwise search the current directory and
  ancestors up to the repository root for exactly one `.repo-com.toml`.
- Fail with a typed error on zero or multiple candidates.
- Reject unknown keys, duplicate aliases, unsafe schema versions, cross-workspace
  references, invalid mention prefixes, and secret-like fields.
- Report path-aware errors without logging file contents or secret-like values.

A skill may propose configuration changes, but activating a new permission
requires a separate interactive operator action (see
[ADR-005](adr/ADR-005-operator-activated-exact-policy.md)).

## Identity, Secrets, and TLS

- **Identity:** a dedicated Discord **bot** only. User tokens, Bearer user
  authentication, and self-bots are rejected.
- **Token source:** `REPO_COM_DISCORD_TOKEN` environment variable only. The
  token is never written to config or state, is redacted from errors and
  diagnostics, and owned copies are zeroized on drop.
- **Authorization scheme:** the Discord Bot scheme, sent only to
  `https://discord.com/api/v10`.
- **TLS:** the planned HTTP client uses rustls, avoiding an OpenSSL runtime
  dependency.
- **Least privilege:** request `VIEW_CHANNEL`, `SEND_MESSAGES`, and
  `READ_MESSAGE_HISTORY`. Mention capability is validated against the resolved
  role or user and guild rules, never mutated automatically.
- **Rotation:** on authentication failure, the product returns a token-rotation
  instruction. Rotate the token in the Discord developer portal and update the
  environment variable; no local state needs migration.

## Storage and Backups

- **Location:** one user-level SQLite database in the OS application-data
  location (resolved with `dirs`). Operational state is **never** stored beneath
  the repository.
- **Protection:** created with user-only filesystem permissions; foreign keys,
  WAL, and a bounded busy timeout are enabled.
- **Schema:** forward-only migrations. Corruption, unsupported schema, lock
  timeout, or migration failure is reported with a typed error; the database is
  never deleted or recreated automatically.
- **Backups:** there is no built-in backup, export, or synchronization. If you
  copy the database, protect the copy with equivalent permissions and remember
  it contains unencrypted retained content.
- **Retention defaults:** content 30 days; non-content delivery/audit metadata
  365 days; both overridable per repository within validated bounds.

## Health Checks and Monitoring

- **Setup check:** a guided, read-only command validates bot identity, workspace
  membership, channel visibility, required channel permissions, and resolved
  mention access. It makes no mutations.
- **State verification:** a read-only command reports SQLite `quick_check`,
  foreign keys, expected migration version, repository scope, and filesystem
  permissions without modifying or recreating the database.
- **Audit query:** bounded, repository-scoped local audit lookup. There is no
  remote history or read receipt.
- **No monitoring service, no telemetry.** There is no crash upload, analytics,
  or remote audit synchronization to configure.

## Upgrades and Rollback

Planned behavior:

- Upgrades are explicit: install a new binary over the old one. There is no
  self-updater.
- Migrations are forward-only. Before upgrading, back up the state database
  under protection equivalent to the original.
- Preserve the previous binary to roll back the executable if needed; note that
  a forward schema migration cannot be automatically reverted by reinstalling an
  older binary. Test upgrades on a disposable workspace first.

## Troubleshooting

| Planned symptom | Likely cause | Next action |
|---|---|---|
| `--version` fails or no binary found | No release/install performed | Confirm the release artifact and `PATH` entry |
| Authentication failure | Missing, invalid, or revoked token | Rotate the token; set `REPO_COM_DISCORD_TOKEN` |
| Permission or not-found errors | Missing channel permissions or wrong channel/workspace | Run the read-only setup check; grant the documented permissions |
| Configuration rejected | Unknown key, unsafe schema version, duplicate alias, secret-like field | Correct `.repo-com.toml`; validation never echoes secret values |
| `operator-action-required` | Activation/approval/purge attempted in a non-TTY shell | Re-run interactively in a TTY |
| `unknown` delivery | Post-dispatch timeout, reset, or 5xx | Reconcile read-only; never resend until resolved |
| `storage-integrity` error | Failed retention sweep, corruption, unsupported schema, lock timeout | Inspect with `state verify`; database bytes are preserved |
| Repeated duplicate-send concern | Concurrent invocations of one revision | Confirm the atomic claim returned the recorded outcome |

## Security and Privacy Checklist

- [ ] The token exists only in `REPO_COM_DISCORD_TOKEN`, never in config, state,
      logs, CI, fixtures, SBOMs, or documentation.
- [ ] Only a dedicated bot identity is used; no user tokens or self-bots.
- [ ] `/api/v10` is pinned; rate limits are read from response headers.
- [ ] Setup and validation are read-only; no application, role, channel, or
      permission mutation occurs.
- [ ] `.repo-com.toml` contains no secrets and is safe to commit and review.
- [ ] State is protected by user-only filesystem permissions and never uploaded
      or synchronized.
- [ ] Retention defaults are in place and purge requires a plan plus TTY
      confirmation.
- [ ] There is no telemetry, crash upload, or remote audit synchronization.
- [ ] The residual risk is understood: **unencrypted** local state is readable by
      anyone with local account access, and by backups and filesystem snapshots.

## Recovery and Support

- **Unknown delivery:** follow the read-only reconciliation path in
  [ADR-007](adr/ADR-007-atomic-claim-and-nonce-reconciliation.md). Do not resend
  while an outcome is unknown or unresolved.
- **Corrupt or unsupported state:** inspect with read-only state verification. Do
  not delete or recreate the database; preserve the bytes for diagnosis.
- **Incident containment:** rotate the bot token and revoke the Discord
  application's access; review the local audit trail.
- **Documentation:** the product's own operator and security documentation is a
  planned deliverable (`REL-DOC-1`): `docs/operator-guide.md`,
  `docs/configuration.md`, `docs/discord-setup.md`, `docs/security-model.md`, and
  `docs/threat-model.md`. Until then, this guide, the [user guide](user-guide.md),
  the [Product Vision](PRD.md), and the [ADRs](adr/README.md) are the canonical
  references.
