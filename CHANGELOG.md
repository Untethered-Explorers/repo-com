# Changelog

All notable changes to this repository are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) categories: Added,
Changed, Deprecated, Removed, Fixed, and Security.

> **Release status:** the workspace version is `0.1.0`, but there is no Git tag,
> packaged artifact, or installable `repo-com` binary. The current release
> communication is [Unreleased](docs/releases/unreleased.md).

## [Unreleased]

### Added

- Rust 2024 Cargo workspace with 19 focused library packages covering protocol,
  configuration, repository-scoped state, exact policy, audit, drafts, safety,
  approval, eligibility, Discord, delivery, inbound, reply, and retention
  contracts.
- Protocol-version-1 success and error envelopes, stable error categories and
  exit-code mapping, separated output values, typed global arguments, and
  explicit TTY/non-TTY decisions.
- Strict schema-version-1 `.repo-com.toml` parsing, bounded repository-root
  discovery, alias resolution, secret-key rejection, raw-destination rejection,
  and deterministic SHA-256 configuration hashes.
- Repository-scoped SQLite state with forward-only migration version 1, WAL,
  foreign keys, bounded busy timeout, user-only file handling on Unix, immutable
  evidence triggers, and transactional repository APIs.
- Exact event/destination/severity policy matching with TTY-confirmed activation,
  stale-hash invalidation, ambiguity denial, and permission-reducing
  deactivation.
- Redacted append-only audit writes and bounded, repository-scoped local audit
  queries.
- Immutable draft revisions, deterministic channel-ready rendering with
  allowlisted mentions and a revision-derived delivery nonce, credential-pattern
  scanning, exact-revision TTY approval, and fail-closed send eligibility.
- Dedicated-bot Discord REST v10 setup validation, one-attempt text-message
  operations, typed rate-limit and uncertainty outcomes, and token-free
  WireMock contract fixtures.
- Atomic local delivery claims, delivery state transitions, bounded retry
  classification, and read-only unknown-delivery reconciliation primitives.
- Bounded untrusted inbound fetch, repository-scoped inbound lifecycle state,
  local acknowledgement and archive operations, and validated threaded reply
  draft creation.
- Repository-scoped content and metadata retention sweeps with deterministic
  cutoffs, `[content-expired]` replacement, count-only audit summaries, and
  blocking failure behavior.

### Changed

- Refreshed the README, library consumer guide, administrator guide, and
  Unreleased release notes to describe the current library contracts separately
  from the planned CLI and product workflow.
- Recorded fresh local validation: formatting, clippy, and a full nextest run
  covering 195 tests across 19 binaries, with 195 passed and 2 skipped.
- Documented the current state path, schema, retention behavior, environment-only
  Discord credential boundary, WireMock validation boundary, and explicit gaps in
  final composition and release readiness.

### Security

- Configuration errors and audit diagnostics remain redacted; secret-like values
  are not echoed and Discord bot tokens are accepted only from
  `REPO_COM_DISCORD_TOKEN`.
- The Discord adapters use zeroizing owned token storage, `Bot` authorization,
  fixed REST v10 routes, and dedicated-bot identity checks; the current tests do
  not use a live token or workspace.
- State is repository-scoped, transactional, and append-only for audit evidence,
  but it is not encrypted at rest. User-only filesystem permissions do not
  protect against local-account compromise, backups, or filesystem snapshots.
- The current product surface has no telemetry, remote state synchronization, or
  remote audit synchronization. Inbound data remains untrusted and local
  acknowledgement/archive operations do not mutate Discord.

## Future release

When a tagged release is created, replace the `[Unreleased]` heading with the
authoritative version and date, then add a versioned file under
[`docs/releases/`](docs/releases/). Do not describe the current library contracts
as a complete installed Discord product, and do not claim live-service or human
sign-off evidence that has not been recorded.
