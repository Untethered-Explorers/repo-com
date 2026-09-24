# Architecture Decision Records

This directory records the durable architectural decisions for `repo-com` v1.
Each record states a decision that shapes the system, not every implementation
detail. The decisions are derived from the canonical
[Product Vision](../PRD.md) and the [feature documents](../features/), which
remain the authoritative requirement and task sources.

> **Project status:** `repo-com` is a canonical v1 **plan**. No source code,
> crate, binary, or release exists as of 2026-09-24. ADRs are marked
> **Accepted** because they are approved planning decisions; the
> "Implementation References" sections point to the feature contracts and the
> planned crate paths that will satisfy each decision.

## Index

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](ADR-001-rust-workspace-and-patched-sqlite.md) | Rust 2024 workspace with statically linked patched SQLite | Accepted |
| [ADR-002](ADR-002-repository-scoped-local-state.md) | Repository-scoped local SQLite state under OS user-data | Accepted |
| [ADR-003](ADR-003-versioned-json-protocol.md) | Versioned JSON protocol on stdout, diagnostics on stderr | Accepted |
| [ADR-004](ADR-004-explicit-tty-exact-revision-approval.md) | Explicit TTY mode with exact-revision approval and fail-closed non-TTY | Accepted |
| [ADR-005](ADR-005-operator-activated-exact-policy.md) | Operator-activated exact-tuple auto-send policy | Accepted |
| [ADR-006](ADR-006-dedicated-discord-bot-rest-v10.md) | Dedicated Discord bot, REST v10, read-only setup | Accepted |
| [ADR-007](ADR-007-atomic-claim-and-nonce-reconciliation.md) | Atomic duplicate-safe claim and nonce reconciliation | Accepted |
| [ADR-008](ADR-008-untrusted-inbound-local-lifecycle.md) | Untrusted inbound data with local-only lifecycle | Accepted |
| [ADR-009](ADR-009-bounded-retention-and-confirmed-purge.md) | Bounded retention, confirmed purge, unencrypted local state | Accepted |
| [ADR-010](ADR-010-accessible-terminal-presentation.md) | Accessible terminal presentation | Accepted |
| [ADR-011](ADR-011-pre-send-secret-detection.md) | Pre-send secret detection with audited TTY override | Accepted |
| [ADR-012](ADR-012-provider-neutral-domain-discord-adapter.md) | Provider-neutral domain with a Discord-only v1 adapter | Accepted |

## Conventions

- **Status:** Proposed, Accepted, Superseded, or Deprecated.
- **Date:** the date the decision was recorded in this repository.
- **Implementation References:** links to the owning requirements, feature
  contracts, and planned outputs. These are planning references, not runtime
  evidence.
