# Architecture Decision Records

This directory records durable architectural decisions for `repo-com`. The
records describe decisions, not every implementation detail. The [Product
Vision](../PRD.md) and [feature specifications](../features/) remain the
authoritative future requirements and task sources; the implementation column
below is maintained from the current source and contract tests.

> **Project status:** workspace version `0.1.0` with 31 packages and a
> composed `repo-com` executable target. There is no Git tag, published
> artifact, installer, or release date in this checkout. Human UX/security
> review, live Discord acceptance, and final release sign-off are pending.
> Individual ADR records retain their decision dates; their historical wording
> is not a substitute for the current status in this index.

## Index

| ADR | Decision | Decision status | Current implementation |
|---|---|---|---|
| [ADR-001](ADR-001-rust-workspace-and-patched-sqlite.md) | Rust 2024 workspace with a patched SQLite release floor | Accepted | Workspace, toolchain, state boundary, release policy, and static-SQLite checks are implemented; no published artifact is recorded |
| [ADR-002](ADR-002-repository-scoped-local-state.md) | Repository-scoped local SQLite state under OS user-data | Accepted | State paths, schema, transactions, immutability, lifecycle, and purge are implemented |
| [ADR-003](ADR-003-versioned-json-protocol.md) | Versioned JSON protocol on stdout, diagnostics on stderr | Accepted | Foundation envelopes, final routing, machine streams, and human renderers are implemented |
| [ADR-004](ADR-004-explicit-tty-exact-revision-approval.md) | Explicit TTY mode with exact-revision approval and fail-closed non-TTY | Accepted | Exact approval, secret override, policy, purge prompts, and non-TTY contracts are implemented; human UX review is pending |
| [ADR-005](ADR-005-operator-activated-exact-policy.md) | Operator-activated exact-tuple auto-send policy | Accepted | Policy registry, exact activation, staleness, ambiguity denial, and status are implemented; no shell deactivation command is exposed |
| [ADR-006](ADR-006-dedicated-discord-bot-rest-v10.md) | Dedicated Discord bot, REST v10, read-only setup | Accepted | Dedicated-bot setup, message, and inbound adapters plus token-free WireMock contracts are implemented; live acceptance is pending |
| [ADR-007](ADR-007-atomic-claim-and-nonce-reconciliation.md) | Atomic duplicate-safe claim and nonce reconciliation | Accepted | Local claims, duplicate-safe transport composition, retry classification, and read-only reconciliation contracts are implemented; no CLI reconciliation command is exposed |
| [ADR-008](ADR-008-untrusted-inbound-local-lifecycle.md) | Untrusted inbound data with local-only lifecycle | Accepted | Fetch, snapshots, point reconciliation, local markers, and reply-draft services are implemented; the final CLI has a known accepted-delivery correlation gap |
| [ADR-009](ADR-009-bounded-retention-and-confirmed-purge.md) | Bounded retention, confirmed purge, unencrypted local state | Accepted | Retention, confirmed local purge, lifecycle inspection, and privacy disclosures are implemented; no retention-sweep CLI is exposed |
| [ADR-010](ADR-010-accessible-terminal-presentation.md) | Accessible terminal presentation | Accepted | Labeled 80-column renderers, keyboard prompts, no-color behavior, and non-TTY gates are implemented with automated contracts; human review is pending |
| [ADR-011](ADR-011-pre-send-secret-detection.md) | Pre-send secret detection with audited TTY override | Accepted | Rendered-text/metadata scanning, exact TTY override, and redacted audit behavior are implemented |
| [ADR-012](ADR-012-provider-neutral-domain-discord-adapter.md) | Provider-neutral domain with a Discord-only v1 adapter | Accepted | Draft/delivery/inbound domain boundaries and the Discord adapter split are implemented; v1 remains Discord-only |

## Conventions

- **Decision status:** Proposed, Accepted, Superseded, or Deprecated.
- **Date:** the date the decision was recorded in this repository.
- **Implementation references:** source, tests, configuration, and task
  references. A planned output is not runtime evidence.
- **Current implementation:** explicitly identifies partial, absent, or
  externally unverified product surfaces so an accepted decision is not
  mistaken for a shipped or approved capability.
