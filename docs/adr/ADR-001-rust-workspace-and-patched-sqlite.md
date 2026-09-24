# ADR-001: Rust 2024 workspace with a patched SQLite release floor

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers

## Context

`repo-com` needs a deterministic Rust workspace for safety-critical protocol,
configuration, state, and policy boundaries. The state layer must not silently
run against an SQLite engine below the release floor. The current workspace
also needs a reproducible dependency graph and a clear boundary between
development validation and future release packaging.

## Decision

Use Rust `1.98.1` and edition 2024 with a Cargo workspace whose members are
`crates/*`, a central workspace dependency table, and a committed `Cargo.lock`.
The state layer uses `rusqlite` and `rusqlite_migration` without enabling the
bundled SQLite source. Release-oriented state opens must reject a linked SQLite
runtime below `3.53.4` before creating or mutating operational state.

The current repository implements the workspace, toolchain, lockfile, and
runtime gate. Static release packaging, compile-time linkage verification, and
the complete multi-platform release policy remain future work.

## Alternatives Considered

- **Go or Node/TypeScript CLI** — rejected because the safety and state
  contracts benefit from Rust's typed ownership and deterministic native
  builds.
- **Unpinned system SQLite** — rejected because an operator machine could run
  an older or unpatched engine.
- **Bundled older SQLite source** — rejected because the release floor is a
  safety requirement, not an optimization choice.
- **OpenSSL-backed TLS** — deferred; the current workspace has no HTTP client,
  and a future adapter must make its TLS dependency explicit.

## Consequences

- Benefits: a reproducible workspace, a visible release floor, and a tested
  runtime boundary.
- Costs and risks: toolchain and dependency upgrades require deliberate review;
  development opens can exercise the local engine and must not be mistaken for
  release evidence.
- Release packaging must separately prove the linked SQLite version, platform
  targets, checksums, licenses, and SBOM before publication.

## Implementation References

- [`Cargo.toml`](../../Cargo.toml)
- [`rust-toolchain.toml`](../../rust-toolchain.toml)
- [`Cargo.lock`](../../Cargo.lock)
- [`crates/repo-com-state/src/store.rs`](../../crates/repo-com-state/src/store.rs)
  (`REQUIRED_SQLITE_VERSION`, `assert_sqlite_runtime`, and
  `StateStore::open_for_release`)
- [`crates/repo-com-state/tests/state_contract.rs`](../../crates/repo-com-state/tests/state_contract.rs)
- [`crates/repo-com-foundation/tests/foundation_contract.rs`](../../crates/repo-com-foundation/tests/foundation_contract.rs)
- Planned release-policy work: `REL-PACK-1` and the release-readiness
  [feature specification](../features/release-readiness.md)
