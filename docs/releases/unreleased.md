# repo-com (Unreleased)

**Workspace version:** `0.1.0`
**Release date:** not released

The `1.0` version recorded in the Product Vision is a planned product target,
not an authoritative release version. The current release source is the Cargo
workspace manifest.

## Summary

`repo-com` currently provides a tested Rust library foundation for a future
single-user Discord communication workflow. The implemented crates cover
versioned outcomes and safety categories, strict repository configuration,
repository-scoped SQLite state, and exact operator-activated policy matching.
There is no installable CLI, Discord client, or complete end-to-end transport in
this release candidate.

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
- 42 passing contract tests across four current test binaries.

## Compatibility and prerequisites

- Rust `1.98.1` and Cargo are required for local development.
- Release-oriented state opens require linked SQLite `3.53.4` or newer.
- Development and contract-test opens can use the locally available SQLite
  runtime and should not be treated as release-package evidence.
- A Discord bot, token, workspace, network connection, and installed package
  are not required for the current library tests.

## Installation or upgrade

There is no published `repo-com` artifact or installation path. From a local
checkout, use:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

There is no self-updater. Future schema changes are forward-only; preserve the
state database before any future upgrade and do not assume an older binary can
open a newer schema.

## Known limitations

- No `repo-com` executable, CLI command tree, terminal renderer, or installed
  package exists yet.
- No Discord REST v10 client, bot authentication, setup check, delivery,
  reconciliation, inbound fetch, or reply transport exists yet.
- Draft-content generation, secret scanning, exact approval, and final send
  eligibility are not implemented.
- Retention, purge, lifecycle inspection commands, cross-platform CI,
  packaging, checksums, SBOM generation, and release automation are not
  implemented.
- Local state is not encrypted at rest. Unix file modes protect the current
  implementation, while Windows relies on inherited user-profile ACLs; backups,
  snapshots, and local-account compromise remain disclosure risks.
- No live Discord compatibility, human UX/security acceptance, or release
  sign-off has been performed.

## Validation

The following checks passed for the current workspace:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The nextest run completed with **42 passed, 0 skipped** across
`foundation_contract`, `config_contract`, `state_contract`, and
`policy_contract`. These are automated library checks; they do not prove
operator usability, live Discord behavior, packaging readiness, or release
approval.

## Security and privacy

- Configuration examples contain synthetic IDs and no credentials.
- Secret-like configuration keys and raw destination fields fail closed, and
  typed errors do not echo offending values.
- State is repository-scoped, transactional, and append-only for audit evidence.
- The current crates do not read `REPO_COM_DISCORD_TOKEN`, call Discord, send
  messages, fetch remote data, or provide telemetry.
- Policy activation requires an explicitly supplied TTY confirmation; the
  library never infers operator approval from a non-TTY caller.
- State is not encrypted at rest. Copies must receive equivalent local access
  controls.

## Documentation

- [Library Consumer Guide](../user-guide.md)
- [Administrator Guide](../admin-guide.md)
- [Architecture Decision Records](../adr/README.md)
- [Product Vision](../PRD.md) and [feature specifications](../features/)
- [Changelog](../../CHANGELOG.md)
