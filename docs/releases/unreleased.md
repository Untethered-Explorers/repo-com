# repo-com (Unreleased)

**Release date:** not yet released

> Draft release notes for the eventual v1.0.0. No release has been cut and no
> installable artifact exists. These notes describe the intended release; they
> are **not** a record of completed work and contain no validation evidence.

## Summary

`repo-com` is a single-user, local command-line transport and approval workflow
that lets a repository skill request a teammate's attention or decision through
Discord, and lets the operator preview, approve, deliver, retrieve, and audit the
exchange. It is Discord-only in v1. The primary success signal is a real
teammate noticing an agent-originated request and replying to it.

## Highlights

- One globally installed, versioned `repo-com` binary for Linux, macOS, and
  Windows.
- Strict, secret-free `.repo-com.toml` (schema version 1) with local destination
  and mention aliases.
- Immutable draft revisions with exact preview and exact-revision human approval.
- Narrow, operator-activated exact-tuple auto-send policy.
- Duplicate-safe delivery with an atomic claim and read-only reconciliation of
  ambiguous outcomes.
- On-demand, bounded retrieval of replies and mentions, treated as untrusted
  data, with local acknowledgement and archival.
- Validated threaded reply drafts that pass the same safety gates.
- Bounded retention (30 days content / 365 days metadata) and operator-confirmed
  local purge.
- Versioned JSON protocol (version 1) for non-TTY skill callers.
- Accessible, keyboard-operable, 80-column, `NO_COLOR`-aware terminal output.

## Compatibility and Prerequisites

- **Platforms (planned):** Linux, macOS, and Windows release targets.
- **Runtime database:** SQLite 3.53.4 or newer, statically linked in release
  artifacts.
- **Discord:** a dedicated least-privilege bot with `VIEW_CHANNEL`,
  `SEND_MESSAGES`, and `READ_MESSAGE_HISTORY`.
- **Local:** a user-level SQLite database; no server or daemon.

## Installation or Upgrade

No installation or upgrade path is available because no artifact has been
published. The planned path is: download the platform artifact, verify its
checksum, place the binary on `PATH`, confirm `repo-com --version`, set
`REPO_COM_DISCORD_TOKEN`, add `.repo-com.toml`, and run the read-only setup
check. There is no self-updater; upgrades are explicit, and database migrations
are forward-only (back up the state file first).

## Known Limitations

- The entire v1 contract is **unimplemented** as of this draft. No automated,
  mocked, cross-platform, or live Discord validation has been produced.
- v1 is Discord-only; email and other providers are planned for v2.
- No encryption at rest or OS-keychain integration; local state is protected by
  user-only filesystem permissions and is exposed to local account access,
  backups, and filesystem snapshots.
- No telemetry, read receipts, response analytics, or full-history search.
- Automatic sending requires a separately activated exact policy; all other
  messages require interactive approval.
- No attachments, embeds, reactions, scheduled sends, or arbitrary destinations.
- Live Discord acceptance is a dependent human review and has not been performed.

## Validation

**None.** Planned validation includes unit and property tests, storage and CLI
integration tests, token-free WireMock Discord contracts pinned to REST v10,
a mocked end-to-end workflow, cross-platform CI, advisory/license/secret gates,
a warm-command performance budget, and human UX, security/privacy, live Discord,
and release sign-off reviews. None of these has run because no implementation
exists.

## Security and Privacy

- Bot token only from `REPO_COM_DISCORD_TOKEN`, redacted and zeroized.
- Dedicated bot identity only; no user-token impersonation.
- No secrets in configuration or state; pre-send credential detection with an
  audited, TTY-only exact-revision override (defense in depth, not complete DLP).
- Inbound messages are untrusted and cannot grant permission or trigger sends.
- Local audit trail with append-only evidence; retention is bounded and purge is
  operator-confirmed and local-only.
- No telemetry; residual risk of unencrypted local state is explicitly disclosed.

## Documentation

- [Product Vision](../PRD.md) and [feature specifications](../features/)
- [User Guide](../user-guide.md)
- [Administrator Guide](../admin-guide.md)
- [Architecture Decision Records](../adr/README.md)
- [Changelog](../../CHANGELOG.md)
