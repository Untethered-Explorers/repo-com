# ADR-003: Versioned JSON protocol on stdout, diagnostics on stderr

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

Repository skills call `repo-com` non-interactively and need a stable,
machine-readable contract. Operators also need human-readable output. Mixing
prompts, diagnostics, and data on the same stream makes automation unreliable.

## Decision

Define protocol version 1 as exactly one JSON object on stdout for machine mode,
with `protocol_version`, `status`, `data`, and `error` fields. Keep diagnostics,
prompts, and panic output off stdout (diagnostics go to stderr, opt-in only).
Map errors to stable process categories — usage/schema, approval or operator
action required, policy blocked, authentication, permission, remote conflict,
unknown delivery, storage integrity, connectivity/rate limit, and internal
failure — so callers can branch deterministically. Human mode uses labeled
sections instead of JSON.

## Alternatives Considered

- **Human text only** — rejected: not safely machine-parseable.
- **Multiple JSON lines / streaming** — rejected: one object per invocation is
  simpler and prevents partial-parse ambiguity.
- **Diagnostics on stdout** — rejected: would corrupt the protocol contract.

## Consequences

- Benefits: a stable skill-facing contract, deterministic exit categories, and
  clean stream separation.
- Costs and risks: every handler must respect the envelope and category mapping;
  `FOUND-CON-02` requires machine stdout to stay valid JSON even for
  operational errors.

## Implementation References

- [cli-foundation.md](../features/cli-foundation.md) `FOUND-FR-03` through
  `FOUND-FR-05`, `FOUND-CON-02`
- [release-readiness.md](../features/release-readiness.md) `REL-FR-02`,
  `REL-FR-03`
- Planned outputs: `crates/repo-com-foundation/src/protocol.rs`,
  `src/error.rs`, `crates/repo-com-cli/src/main.rs`
- Owning tasks: `PLAT-1`, `REL-APP-1`
