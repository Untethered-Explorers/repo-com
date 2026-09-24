# Changelog

All notable changes to this repository are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
categories: Added, Changed, Deprecated, Removed, Fixed, and Security.

> **Release status:** `repo-com` has **no tagged release and no implemented
> binary** as of 2026-09-24. The entries below track authored specifications,
> generated agent/skill definitions, and documentation. They are not claims of
> working software behavior. See [docs/releases/unreleased.md](docs/releases/unreleased.md).

## [Unreleased]

### Added

- Product vision and shared requirements in `docs/PRD.md`, decomposed into seven
  canonical feature documents under `docs/features/`.
- Executable `forge-task` contracts for every v1 feature, each naming an owning
  agent, dependencies, expected outputs, and validation commands.
- Generated project agent team under `.opencode/agents/` and reusable project
  skills under `.opencode/skills/`, recorded in `docs/SKILL-CANDIDATES.json`
  and reviewed in `docs/skill-review/`.
- Architectural decision records under `docs/adr/`.
- Pre-release user and administrator guides, `docs/user-guide.md` and
  `docs/admin-guide.md`.
- Release notes index and draft notes under `docs/releases/`.

### Changed

- Expanded `README.md` from a one-line description into a documentation
  navigation hub.

### Not yet implemented

- No `Cargo.toml`, Rust workspace, or `repo-com` source crates exist.
- No `repo-com` binary is installable on any platform.
- No continuous-integration or release workflow is present.
- No automated, mocked end-to-end, cross-platform, or live Discord evidence has
  been produced.

<!--
When the first release is tagged, replace [Unreleased] with the released
version and date, for example:

## [0.1.0] - YYYY-MM-DD
### Added
...
-->
