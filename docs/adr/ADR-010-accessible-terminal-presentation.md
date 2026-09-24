# ADR-010: Accessible terminal presentation

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

`repo-com` is a terminal interface used for security-sensitive decisions such as
approval, policy activation, and purge confirmation. Assistive technology and
piped output must be able to read the same information as an interactive
terminal, and non-interactive automation must never be prompted.

## Decision

Design every interactive command for keyboard operation, linear screen-reader
reading, visible focus or selection, and no color-only meaning, following
applicable WCAG 2.2 AA principles for terminal software. Honor `NO_COLOR` and
non-color output requests, render a complete plain-text presentation at an
**80-column minimum** without truncating security or approval information, and
never prompt in non-TTY mode. Announce state and errors in text — destination,
revision, policy or approval basis, and next action — with ANSI styling only as a
supplement. Keep JSON mode to exactly one protocol object on stdout (ADR-003) and
produce deterministic snapshots for review.

## Alternatives Considered

- **Color-rich TUI framework** — rejected: adds a rendering dependency and risks
  color-only meaning; plain labeled text is sufficient and more testable.
- **Mouse-first interaction** — rejected: not keyboard- or assistive-technology
  friendly.
- **No formal accessibility contract** — rejected: this is a security surface
  where comprehensibility is required.

## Consequences

- Benefits: keyboard-only and screen-reader flows, deterministic testable output,
  and safe behavior when piped.
- Costs and risks: layout must be validated at 80 columns; a human UX rubric
  (`REL-UX-HR-1`) must confirm discoverability and non-TTY safety before release.

## Implementation References

- [PRD §11 Accessibility](../PRD.md#11-accessibility) `RC-ACC-01` through
  `RC-ACC-03`; [PRD §12 User Interface / Interaction Design](../PRD.md)
  (section 12)
- [release-readiness.md](../features/release-readiness.md) `REL-FR-03`
- Planned outputs: `crates/repo-com-terminal-outbound/`,
  `crates/repo-com-terminal-operations/`
- Owning tasks: `REL-UI-OUT-1`, `REL-UI-OPS-1`, `REL-UX-HR-1`
