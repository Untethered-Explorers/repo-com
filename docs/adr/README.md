# Architecture Decision Records

This directory records durable architectural decisions for `repo-com`. The
records describe decisions, not every implementation detail. The [Product
Vision](../PRD.md) and [feature specifications](../features/) remain the
authoritative future requirements and task sources; the implementation status
below is based on the current source and contract tests.

> **Project status:** workspace version `0.1.0` with four implemented library
> packages. There is no tagged release or installable `repo-com` binary. An ADR
> can be **Accepted** as a decision while its implementation is partial or
> planned; the implementation column makes that distinction explicit.

## Index

| ADR | Decision | Decision status | Current implementation |
|---|---|---|---|
| [ADR-001](ADR-001-rust-workspace-and-patched-sqlite.md) | Rust 2024 workspace with a patched SQLite release floor | Accepted | Workspace and state release boundary implemented; packaging gate pending |
| [ADR-002](ADR-002-repository-scoped-local-state.md) | Repository-scoped local SQLite state under OS user-data | Accepted | State paths, schema, transactions, and immutability implemented |
| [ADR-003](ADR-003-versioned-json-protocol.md) | Versioned JSON protocol on stdout, diagnostics on stderr | Accepted | Foundation envelopes and stream values implemented; executable routing pending |
| [ADR-004](ADR-004-explicit-tty-exact-revision-approval.md) | Explicit TTY mode with exact-revision approval and fail-closed non-TTY | Accepted | Explicit TTY boundary implemented; approval and terminal workflow pending |
| [ADR-005](ADR-005-operator-activated-exact-policy.md) | Operator-activated exact-tuple auto-send policy | Accepted | Policy registry, hashes, activation, staleness, and deactivation implemented |
| [ADR-006](ADR-006-dedicated-discord-bot-rest-v10.md) | Dedicated Discord bot, REST v10, read-only setup | Accepted | Not implemented |
| [ADR-007](ADR-007-atomic-claim-and-nonce-reconciliation.md) | Atomic duplicate-safe claim and nonce reconciliation | Accepted | State persistence primitives implemented; network delivery and reconciliation pending |
| [ADR-008](ADR-008-untrusted-inbound-local-lifecycle.md) | Untrusted inbound data with local-only lifecycle | Accepted | Inbound persistence primitives implemented; fetch and reply transport pending |
| [ADR-009](ADR-009-bounded-retention-and-confirmed-purge.md) | Bounded retention, confirmed purge, unencrypted local state | Accepted | Configuration fields implemented; retention and purge services pending |
| [ADR-010](ADR-010-accessible-terminal-presentation.md) | Accessible terminal presentation | Accepted | Not implemented |
| [ADR-011](ADR-011-pre-send-secret-detection.md) | Pre-send secret detection with audited TTY override | Accepted | Configuration-key rejection implemented; message scanner pending |
| [ADR-012](ADR-012-provider-neutral-domain-discord-adapter.md) | Provider-neutral domain with a Discord-only v1 adapter | Accepted | Not implemented |

## Conventions

- **Decision status:** Proposed, Accepted, Superseded, or Deprecated.
- **Date:** the date the decision was recorded in this repository.
- **Implementation references:** source, tests, configuration, and task
  references. A planned output is not runtime evidence.
- **Current implementation:** explicitly identifies partial or absent product
  surfaces so an accepted decision is not mistaken for a shipped feature.
