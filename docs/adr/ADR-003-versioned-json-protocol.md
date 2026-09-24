# ADR-003: Versioned JSON protocol on stdout, diagnostics on stderr

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers

## Context

Repository skills need a deterministic machine-readable boundary, while
operators need human-readable output. Mixing data, prompts, and diagnostics on
one stream would make automation unsafe and fragile.

## Decision

Define protocol version 1 as exactly one JSON object with four fields:
`protocol_version`, `status`, `data`, and `error`. A success object contains
data and no error; an error object contains a typed error and no data. Keep
optional diagnostics in a separate stderr value. Map errors to stable categories
and deterministic exit codes: usage/schema, operator action required, policy
blocked, authentication, permission, remote conflict, unknown delivery, storage
integrity, connectivity/rate limit, and internal failure.

The foundation crate implements the typed envelope, stable categories, JSON
serialization, validation invariants, and separated `OutputStreams`. The final
executable and human renderer remain future work, so the current library does
not itself write to stdout or stderr.

## Alternatives Considered

- **Human text only** — rejected because repository skills need a stable parser.
- **Multiple JSON lines or streaming** — rejected because one object per
  invocation is easier to validate and prevents partial-parse ambiguity.
- **Diagnostics or prompts on stdout** — rejected because they would corrupt
  the protocol contract.

## Consequences

- Benefits: a stable skill-facing value model, deterministic error branches,
  and explicit stream separation.
- Costs and risks: every future handler must preserve the envelope and category
  mapping; adding output fields requires a protocol-version review.
- A current library consumer must write `OutputStreams` itself; no process-level
  behavior can be inferred from serialization alone.

## Implementation References

- [`crates/repo-com-foundation/src/protocol.rs`](../../crates/repo-com-foundation/src/protocol.rs)
- [`crates/repo-com-foundation/src/error.rs`](../../crates/repo-com-foundation/src/error.rs)
- [`crates/repo-com-foundation/src/args.rs`](../../crates/repo-com-foundation/src/args.rs)
- [`crates/repo-com-foundation/tests/foundation_contract.rs`](../../crates/repo-com-foundation/tests/foundation_contract.rs)
- [Library Consumer Guide](../user-guide.md)
- Planned CLI and terminal output: [`release-readiness.md`](../features/release-readiness.md)
