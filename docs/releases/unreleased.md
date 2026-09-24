# repo-com (Unreleased)

**Workspace version:** `0.1.0`
**Release date:** not released

The `1.0` version recorded in the Product Vision is a planned product target,
not an authoritative release version. The current release source is the Cargo
workspace manifest.

## Summary

`repo-com` currently provides 19 focused Rust library contracts for a future
single-user Discord communication workflow. The implemented surface covers
configuration and repository-scoped state, exact policy and approval gates,
draft rendering and safety checks, Discord REST v10 adapters, local delivery and
inbound state, audit evidence, and retention sweeps.

There is no installable `repo-com` binary, command tree, terminal renderer, or
complete end-to-end transport composition in this release boundary. Discord
adapters can perform network I/O when an embedding application constructs them;
the automated contracts use token-free WireMock fixtures and no live workspace.

## Highlights

- Rust 2024 workspace pinned to Rust `1.98.1` with a committed `Cargo.lock`.
- Protocol version 1 success/error values with four stable envelope fields and
  separated stdout/stderr values.
- Strict, secret-free schema-version-1 TOML configuration with bounded
  repository-root discovery, alias resolution, and deterministic SHA-256 hashes.
- Repository-scoped SQLite schema version 1 with forward-only migration, WAL,
  foreign keys, a bounded busy timeout, user-only file handling on Unix, and
  immutable audit/inbound/revision evidence.
- Exact event-type, destination-alias, and severity policy matching with
  TTY-confirmed activation, stale-hash invalidation, ambiguity denial, and
  permission-reducing deactivation.
- Immutable draft revisions, deterministic allowlisted-mention rendering,
  revision-derived delivery nonces, credential-pattern scanning, exact-revision
  approval, and fail-closed send eligibility.
- Dedicated-bot REST v10 setup validation, one-attempt text-message operations,
  typed rate-limit and ambiguity outcomes, atomic local delivery claims, bounded
  retry classification, and read-only reconciliation contracts.
- Bounded untrusted inbound retrieval, local acknowledgement/archive state,
  validated threaded reply drafts, redacted local audit queries, and transactional
  retention sweeps.
- Fresh local validation covered **195 tests across 19 binaries: 195 passed and
  2 skipped**.

## Compatibility and prerequisites

- Rust `1.98.1` and Cargo are required for local development.
- Release-oriented state opens require linked SQLite `3.53.4` or newer.
- Development and contract-test opens can use the locally available SQLite
  runtime and must not be treated as release-package evidence.
- `cargo-nextest` is required for the declared full test command.
- A Discord bot token and network access are required only when an embedding
  application invokes the production Discord adapters. The token is read from
  `REPO_COM_DISCORD_TOKEN` and must be a raw dedicated bot token.
- No installed `repo-com` package, Discord setup command, or live workspace is
  available in this release boundary.

## Installation or upgrade

There is no published `repo-com` artifact or installation path. From a local
checkout, use:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
git diff --check
```

There is no self-updater. State migrations are forward-only. Preserve the
database and its WAL sidecars before any future upgrade, and do not assume an
older binary can open a newer schema.

## Known limitations

- No `repo-com` executable, CLI command tree, terminal renderer, or installed
  package exists yet.
- No complete production composition connects configuration, draft creation,
  approval, eligibility, delivery, Discord transport, retry, reconciliation,
  inbound lifecycle, and retention into one recoverable journey.
- The Discord setup, message, and inbound adapters are component contracts. Their
  automated evidence uses token-free WireMock fixtures; no live Discord
  compatibility, human UX/security acceptance, or release sign-off has been
  performed.
- The current delivery and reconciliation APIs are not a restart-safe end-to-end
  coordinator. Callers must preserve the separate claim, attempt, and
  reconciliation evidence and must not resend an unknown result without the
  read-only reconciliation gate.
- The current inbound adapter exposes page storage/cursor handling and point
  reconciliation as separate phases; callers must not treat a stored page as
  proof that every later point check completed in one product transaction.
- Confirmed purge, read-only lifecycle inspection, command handlers, package
  installers, cross-platform CI, checksums, SBOM generation, performance
  evidence, and release automation are not implemented.
- Local state is not encrypted at rest. Unix file modes protect the current
  implementation, while Windows relies on inherited user-profile ACLs; backups,
  snapshots, and local-account compromise remain disclosure risks.
- The secret scanner is defense in depth, not complete data-loss prevention.
- No current implementation provides a human release decision or production
  support commitment.

## Validation

The following checks passed for the current workspace on 2026-09-24:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
git diff --check
```

The full nextest run selected **195 tests across 19 binaries: 195 passed and 2
skipped**. These are automated library and mocked-contract checks; they do not
prove live Discord behavior, end-to-end product composition, packaging
readiness, operator usability, or release approval.

## Security and privacy

- Configuration examples contain synthetic IDs and no credentials.
- Secret-like configuration keys and raw destination fields fail closed, and
  typed errors do not echo offending values.
- Discord bot credentials are accepted only through `REPO_COM_DISCORD_TOKEN`,
  owned in zeroizing storage, and used only for dedicated-bot authorization.
- Audit evidence is repository-scoped, transactional, append-only, and defensively
  redacted; local audit queries are bounded and read-only.
- Inbound messages, mentions, edits, and deletions are untrusted data. Local
  acknowledgement and archival do not mutate Discord.
- The current workspace has no telemetry, remote state synchronization, or remote
  audit synchronization.
- State is not encrypted at rest. Copies must receive equivalent local access
  controls.

## Documentation

- [Library Consumer Guide](../user-guide.md)
- [Administrator Guide](../admin-guide.md)
- [Architecture Decision Records](../adr/README.md)
- [Product Vision](../PRD.md) and [feature specifications](../features/)
- [Changelog](../../CHANGELOG.md)
