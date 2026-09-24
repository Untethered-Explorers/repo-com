# repo-com

Private repo work communications interface: a single-user, local command-line
transport and approval workflow that lets a repository skill request a
teammate's attention or decision through Discord, and lets the operator preview,
approve, deliver, retrieve, and audit the exchange.

> **Project status: pre-release specification.** As of 2026-09-24, `repo-com` is
> fully specified but **not implemented**. There is no source code, binary, tag,
> or release. The canonical [Product Vision](docs/PRD.md) and
> [feature documents](docs/features/) define the v1 plan; the guidance in this
> repository describes planned behavior and must not be read as working software.

## Documentation

| Document | Purpose |
|---|---|
| [Product Vision (PRD)](docs/PRD.md) | Canonical requirements, architecture, security, and lifecycle |
| [Feature specifications](docs/features/) | Per-feature requirements and executable `forge-task` contracts |
| [User Guide](docs/user-guide.md) | Task-oriented operator and skill workflows |
| [Administrator Guide](docs/admin-guide.md) | Prerequisites, configuration, secrets, operations, hardening |
| [Architecture Decision Records](docs/adr/README.md) | Durable v1 architectural decisions and rationale |
| [Changelog](CHANGELOG.md) | Repository change history |
| [Release notes](docs/releases/) | Versioned release notes (currently draft only) |
| [Project idea (historical)](docs/IDEA.md) | Original source material; not an execution source |

## Scope at a glance

- **In v1:** one Discord workspace per repository; immutable previewed drafts;
  exact-revision human approval or narrow operator-activated exact policy;
  duplicate-safe delivery with ambiguous-outcome reconciliation; on-demand
  bounded reply and mention retrieval with local acknowledgement; bounded
  retention and confirmed local purge; versioned JSON protocol.
- **Out of v1:** email and other providers, a daemon or gateway, a live inbox UI,
  broadcasts and arbitrary DMs, attachments/embeds/reactions, scheduled sends,
  encryption at rest, telemetry, read receipts, and self-updating binaries.

See [PRD §3](docs/PRD.md#3-goals-and-non-goals) for the full goals and non-goals.

## Repository layout

- `docs/PRD.md` — product vision and shared requirements.
- `docs/features/` — seven canonical feature documents (the sole owners of
  detailed requirements and tasks).
- `docs/adr/` — architecture decision records.
- `docs/releases/` — release notes index and drafts.
- `docs/IDEA.md` — historical idea; not an execution source.
- `.opencode/agents/` — generated project agent team.
- `.opencode/skills/` — generated reusable project skills.

Planned implementation crates (`crates/repo-com-*`) and CI/release workflows do
not exist yet; their intended names appear in [PRD §7.2](docs/PRD.md#72-project-structure).

## Release status

No tagged release exists. See [CHANGELOG.md](CHANGELOG.md) and the
[draft release notes](docs/releases/unreleased.md) for the current state.
