# ADR-001: Rust 2024 workspace with statically linked patched SQLite

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

`repo-com` must ship as one globally installed binary for Linux, macOS, and
Windows, build deterministically, and rely on a local SQLite database whose
integrity is safety-critical. The bundled SQLite source available through the
planned `rusqlite` version is 3.53.2, while upstream 3.53.4 fixes a WAL-reset
corruption bug.

## Decision

Use Rust 1.98.1 and the 2024 edition with a Cargo workspace
(`members = ["crates/*"]`), one central workspace dependency table, and a
committed `Cargo.lock`. Use `rusqlite` with `rusqlite_migration`, and require
release artifacts to statically link SQLite **3.53.4 or newer** with a runtime
version assertion.

## Alternatives Considered

- **Go or Node/TypeScript CLI** — rejected: no existing scaffold constrains the
  choice, and Rust gives deterministic static binaries and strong typing for the
  safety-critical state machine.
- **System-provided SQLite** — rejected: cannot guarantee the patched version on
  operator machines.
- **OpenSSL-backed TLS** — rejected in favor of rustls to avoid a runtime
  OpenSSL dependency.

## Consequences

- Benefits: reproducible builds, cross-platform static artifacts, a patched
  database engine, and focused crates that make retry and safety boundaries
  independently testable.
- Costs and risks: a pinned toolchain and central dependency table add upgrade
  friction; release builds must assert the SQLite version at compile and run
  time, and `REL-PACK-1` must reject an older bundled or system runtime.

## Implementation References

- [PRD §5 Research Findings](../PRD.md#5-research-findings) and
  [§7.1 Technology Stack](../PRD.md#7-technical-architecture)
- [cli-foundation.md](../features/cli-foundation.md) `FOUND-CON-01`
- [repository-configuration-and-state.md](../features/repository-configuration-and-state.md)
  `STATE-CON-02`
- Planned outputs: `Cargo.toml`, `rust-toolchain.toml`, `.config/nextest.toml`,
  `crates/repo-com-foundation/`, `crates/repo-com-state/`
- Owning tasks: `PLAT-1`, `REPO-STATE-1`, `REL-PACK-1`
