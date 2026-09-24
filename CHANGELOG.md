# Changelog

All notable changes to this repository are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) categories: Added,
Changed, Deprecated, Removed, Fixed, and Security.

> **Release status:** the workspace version is `0.1.0`, but there is no Git tag,
> packaged artifact, or installable `repo-com` binary. The current release
> communication is [Unreleased](docs/releases/unreleased.md).

## [Unreleased]

### Added

- Rust 2024 Cargo workspace with the `foundation_contract`,
  `config_contract`, `state_contract`, and `policy_contract` library packages.
- Protocol-version-1 success and error envelopes, stable error categories and
  exit-code mapping, separated output streams, typed global arguments, and
  explicit TTY/non-TTY prompt decisions.
- Strict schema-version-1 `.repo-com.toml` parsing, bounded repository-root
  discovery, alias resolution, secret-key rejection, raw-destination rejection,
  and deterministic SHA-256 configuration hashes.
- Repository-scoped SQLite state with forward-only migration version 1, WAL,
  foreign keys, bounded busy timeout, user-only file handling on Unix, immutable
  evidence triggers, and transactional repository APIs.
- Exact event/destination/severity policy matching with TTY-confirmed
  activation, stale-hash invalidation, ambiguity denial, and permission-
  reducing deactivation.
- Four contract-test suites covering the current foundation, configuration,
  state, and policy behavior.

### Changed

- Refreshed the README, user guide, administrator guide, ADRs, and release
  notes to distinguish the implemented library surface from the planned
  Discord/CLI product.
- Documented the current state path, configuration rules, local validation
  commands, and explicit gaps in installation and transport support.
- Recorded the successful local formatting, lint, and 42-test validation run.
- Added the repository root ignore rules for Rust build output and transient
  workflow-engine files so they do not become future source-control changes.

### Security

- Configuration errors are typed and redacted; secret-like values are not echoed.
- State is repository-scoped and protected by platform user-only file controls,
  but it is not encrypted at rest by the current implementation.
- The current workspace performs no Discord or other network side effects and
  has no token consumer, telemetry, or remote audit synchronization.

## Future release

When a tagged release is created, replace the `[Unreleased]` heading with the
authoritative version and date, then add a versioned file under
[`docs/releases/`](docs/releases/). Do not describe the current library
foundation as a complete Discord transport.
