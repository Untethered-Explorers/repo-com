# Changelog

All notable changes to this repository are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) categories: Added,
Changed, Deprecated, Removed, Fixed, and Security.

> **Release status:** `Cargo.toml`, all workspace package manifests, and
> `dist-workspace.toml` agree on version `0.1.0`. The source snapshot inspected
> for this entry is dated 2026-09-25. No Git tag, published archive, installer,
> or release date is recorded; the current release communication is therefore
> [Unreleased](docs/releases/unreleased.md), with a separately labeled
> [`0.1.0` source snapshot](docs/releases/0.1.0.md). The Product Vision's `1.0`
> is a planned target, not a released version.

## [Unreleased]

### Added

- Composed the final `repo-com` executable target with strict routing, semantic
  `--version`, explicit global options, human/protocol output selection, stable
  exit categories, and separated stdout/stderr behavior.
- Added the complete command surface for configuration, exact policy status and
  activation, immutable draft lifecycle, send, read-only Discord setup,
  bounded inbound fetch, local acknowledgement/archive, reply-draft creation,
  audit queries, state/lifecycle inspection, and purge planning/execution.
- Added accessible linear terminal renderers and keyboard prompt adapters with
  80-column wrapping, `NO_COLOR`, explicit cancellation, exact confirmation,
  expiry, and fail-closed non-TTY behavior.
- Added token-free WireMock contracts and a full mocked end-to-end journey,
  including durable local correlation, 100 concurrent send invocations, and
  post-dispatch unknown reconciliation branches.
- Added a repeatable token-free performance harness for no-network command
  classes, a cross-platform CI policy, and a tag-driven release-packaging policy
  for Linux, macOS, and Windows targets.
- Added release-policy checks for static SQLite `3.53.4` or newer, strong
  checksums, source archives, dependency license evidence, CycloneDX SBOMs,
  secret scanning, least-privilege workflow permissions, and no updater or
  setup mutation.

### Changed

- Composed previously focused domain crates into an executable workflow while
  keeping state and network boundaries owned by their respective services.
- Implemented repository-scoped retention, read-only lifecycle inspection, and
  deterministic local purge planning/execution with exact TTY confirmation.
- Refreshed the README, user guide, administrator guide, operator guide, ADR
  navigation, changelog, and release communication to distinguish the current
  executable from the unverified human/live/release phases.
- Recorded the current local validation boundary: the full nextest run reported
  325 passed tests across 35 binaries with 2 skipped tests, and the
  documentation contract reported 7 passed tests. These are automated results,
  not human acceptance or release approval.
- Recorded the local performance harness result: 100 samples for each of 8
  no-network command classes stayed below the 500 ms p95 threshold on the
  inspected Linux host. The host was not identified as the pinned CI reference
  runner, so this is local evidence only.

### Fixed

- Kept protocol command values, selected shell routes, and strict input objects
  aligned so malformed or mismatched automation fails with one typed result.
- Kept local delivery claims, delivery attempts, and audit evidence atomic
  before network I/O; repeated or concurrent sends receive the recorded local
  outcome rather than a second authorization to POST.
- Kept unknown delivery, point reconciliation, retention, and purge state
  transitions conservative, transactional where owned, and explicit about
  rollback, replanning, and untrusted input.

### Security

- Restricts production Discord authentication to a raw dedicated bot token in
  `REPO_COM_DISCORD_TOKEN`, with bot-only authorization, REST v10 pinning,
  redacted diagnostics, and no user-token or self-bot path.
- Treats inbound messages, mentions, edits, deletions, and attachment indicators
  as untrusted data that cannot approve, activate policy, override safety, or
  authorize a send.
- Keeps state repository-scoped and append-only for audit evidence, but
  explicitly discloses that v1 has no encryption at rest, relies on user-only
  filesystem permissions, and cannot protect against local-account compromise,
  backups, or filesystem snapshots.
- Sends no telemetry and performs no remote state or audit synchronization.
  Local acknowledgement, archive, retention, and purge never mutate Discord.
- Rejects positive claims of read receipts, response analytics, arbitrary
  destinations, user tokens, live-service compatibility, human approval,
  release sign-off, compliance certification, or automatic unknown-delivery
  resend.

## Future release work

A future tagged release still requires the pending human UX/security reviews,
live Discord acceptance in a disposable workspace, final release sign-off, and
verification of the actual cross-platform artifacts. The release workflow is
configured for explicit tag-driven artifact generation; this repository does
not publish automatically and has no updater. See the
[release index](docs/releases/README.md) and the
[`0.1.0` source snapshot notes](docs/releases/0.1.0.md) for the current evidence
boundary.
