# Feature: Release Readiness

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** [Privacy and Lifecycle Operations](privacy-and-lifecycle-operations.md)  
**Status:** Canonical v1 plan

This feature composes the focused libraries into the final `repo-com` executable, proves the complete mocked workflow, measures local performance, creates cross-platform CI and release infrastructure, publishes operator and security documentation, and places all human rubric scores and sign-off in dependent human-review tasks.

### In Scope

- Human terminal presentation and keyboard prompts.
- Machine command handlers and stable exit mapping.
- Thin final binary with semantic version and global options.
- Token-free end-to-end mocked workflow.
- Warm-command performance budget.
- Linux, macOS, and Windows CI and versioned release artifacts.
- Dependency/license/advisory/secret gates, checksums, and CycloneDX SBOM.
- Configuration, Discord setup, operator, security, and threat-model documentation.
- Human UX, security/privacy, live Discord, and final release reviews.

### Out of Scope

- Claiming human approval or live compatibility from automated tests.
- Embedding a Discord token, using a shared test credential, or running live tests in normal CI.
- Self-update, background services, hosted telemetry, or a package published to a public registry.
- Treating a human review rehearsal as sign-off.

---

## 2. Interfaces and Preconditions

| Interface | Owner Task | Evidence |
|---|---|---|
| Outbound terminal renderer and prompts | REL-UI-OUT-1 | Outbound snapshot and keyboard-flow contract |
| Operations terminal renderer and prompts | REL-UI-OPS-1 | Inbound/lifecycle snapshot and keyboard-flow contract |
| Operator command handlers | REL-OPS-CMD-1 | Operator handler contract with TTY and non-TTY cases |
| Messaging command handlers | REL-MSG-CMD-1 | Draft/send/inbox/reply contract with explicit identifiers |
| `repo-com` executable | REL-APP-1 | Routing, help, version, and exit contract |
| Performance harness | REL-PERF-1 | 100-run p50/p95 report and threshold check |
| Continuous-integration policy | REL-CI-1 | Cross-platform checks, advisories, licenses, secrets, workflow lint |
| Release packaging policy | REL-PACK-1 | Versioned artifacts, static SQLite, checksums, license evidence, SBOM |
| Documentation validator | REL-DOC-1 | Required topic and prohibited-claim checks |
| End-to-end workflow | REL-E2E-1 | Isolated config, state, wiremock Discord, full command journey |

Human review files are written only by the corresponding `human-review` tasks. Implementation tasks produce evidence but cannot populate a score, attestation, or release decision.

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| REL-FR-01 | requirement | Must | REL-APP-1 |
| REL-FR-02 | requirement | Must | REL-OPS-CMD-1, REL-MSG-CMD-1, REL-APP-1 |
| REL-FR-03 | requirement | Must | REL-UI-OUT-1, REL-UI-OPS-1, REL-OPS-CMD-1, REL-MSG-CMD-1, REL-APP-1 |
| REL-FR-04 | requirement | Must | REL-E2E-1 |
| REL-FR-05 | requirement | Must | REL-PERF-1 |
| REL-FR-06 | requirement | Must | REL-CI-1 |
| REL-FR-07 | requirement | Must | REL-PACK-1 |
| REL-FR-08 | requirement | Must | REL-DOC-1 |
| REL-FR-09 | requirement | Must | REL-LIVE-HR-1 |
| REL-CON-01 | constraint | Must | REL-E2E-1, REL-CI-1, REL-PACK-1 |
| REL-CON-02 | constraint | Must | REL-CI-1, REL-PACK-1, REL-DOC-1 |
| REL-CON-03 | constraint | Must | REL-APP-1, REL-PACK-1 |
| REL-CON-04 | constraint | Must | REL-UX-HR-1, REL-SEC-HR-1, REL-LIVE-HR-1, REL-SIGN-HR-1 |
| REL-CON-05 | constraint | Must | REL-CI-1, REL-PACK-1 |
| REL-CON-06 | constraint | Must | REL-DOC-1, REL-SEC-HR-1 |

```forge-requirement
{"id":"REL-FR-01","kind":"requirement","text":"Compose all domain crates into one installed binary named repo-com, expose a semantic version through --version, and resolve the final executable without hidden repository or destination state."}
```

```forge-requirement
{"id":"REL-FR-02","kind":"requirement","text":"Expose config, policy, draft, send, inbox, reply, audit, state, and purge command groups with explicit identifiers, stdin JSON where structured input is needed, and no command that implies a default destination."}
```

```forge-requirement
{"id":"REL-FR-03","kind":"requirement","text":"Render every command in labeled human or protocol JSON mode, map domain outcomes to stable exit categories, prohibit prompts in non-TTY mode, and preserve complete security and approval information at 80 columns without color."}
```

```forge-requirement
{"id":"REL-FR-04","kind":"requirement","text":"Prove the complete create, approve or activate, send, fetch, reply-draft, acknowledge, audit, and purge journey with isolated temp config/state and a wiremock Discord server without a real token or network."}
```

```forge-requirement
{"id":"REL-FR-05","kind":"requirement","text":"Measure 100 warm invocations of each documented no-network command class on the CI reference runner and fail when p95 exceeds 500 ms, excluding process start, network, and first-run dependency compilation."}
```

```forge-requirement
{"id":"REL-FR-06","kind":"requirement","text":"Run formatting, clippy, fail-on-no-tests nextest, performance, dependency advisory and license checks, secret scanning, workflow linting, and binary smoke tests on Linux, macOS, and Windows."}
```

```forge-requirement
{"id":"REL-FR-07","kind":"requirement","text":"Publish versioned Linux, macOS, and Windows binaries, archives or installers, SHA-256 or stronger checksums, dependency license evidence, and a CycloneDX SBOM, with no self-updater."}
```

```forge-requirement
{"id":"REL-FR-08","kind":"requirement","text":"Document installation, configuration, bot creation and least-privilege grants, token rotation, operator commands, recovery from unknown delivery, retention and purge, security boundaries, threat model, and unsupported behavior without embedding a credential."}
```

```forge-requirement
{"id":"REL-FR-09","kind":"requirement","text":"Require a human-controlled disposable Discord workspace round trip in which a teammate notices a repo-com request, replies, the reply is fetched and correlated, and a validated reply draft is approved or policy-authorized without duplicate delivery."}
```

```forge-requirement
{"id":"REL-CON-01","kind":"constraint","text":"Normal automated tests must not use a real Discord token, workspace, or network; live acceptance is isolated to a dependent human-review task and must use a dedicated disposable bot."}
```

```forge-requirement
{"id":"REL-CON-02","kind":"constraint","text":"CI, release logs, fixtures, SBOMs, review files, and documentation must contain no bot token, authorization header, private key, or real team message content."}
```

```forge-requirement
{"id":"REL-CON-03","kind":"constraint","text":"Release artifacts must not self-update or execute setup mutations; installation and future upgrades remain explicit operator actions."}
```

```forge-requirement
{"id":"REL-CON-04","kind":"constraint","text":"Human rubric scores, live acceptance, native-language or stakeholder judgment, and final approval may appear only in dependent human-review tasks and their review files; implementation tasks may provide evidence but never approval."}
```

```forge-requirement
{"id":"REL-CON-05","kind":"constraint","text":"CI must fail on formatting, clippy, selected-test discovery, RustSec advisories, denied or unknown licenses, source or generated secret findings, invalid workflows, stale lockfile, unsupported SQLite runtime, or release-plan drift."}
```

```forge-requirement
{"id":"REL-CON-06","kind":"constraint","text":"Security and privacy documentation must state that v1 relies on user-only filesystem permissions, performs no telemetry, does not encrypt local state, and cannot protect against local account compromise, backups, or filesystem snapshots."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| REL-UI-OUT-1 | Accessible outbound rendering and prompts | cli-ux-engineer | Outbound result types | `repo-com-terminal-outbound`, `terminal_outbound_contract` | REL-FR-03 and terminal constraints | Command execution, inbound/lifecycle views |
| REL-UI-OPS-1 | Accessible operations rendering and prompts | cli-ux-engineer | Inbound and lifecycle result types | `repo-com-terminal-operations`, `terminal_operations_contract` | REL-FR-03, trust labels, terminal constraints | Command execution, outbound views |
| REL-OPS-CMD-1 | Validated operator and lifecycle handlers | cli-engineer | Operations UI and lifecycle services | `repo-com-cli-operations`, `cli_operations_contract` | REL-FR-02/03 and foundation protocol | Messaging handlers, final binary |
| REL-MSG-CMD-1 | Validated messaging handlers | cli-engineer | Outbound UI and messaging services | `repo-com-cli-messaging`, `cli_messaging_contract` | REL-FR-02/03 and messaging safety | Operator handlers, final binary |
| REL-DOC-1 | Validated operator and security documentation | technical-writer | Canonical contracts | operator/security docs, `documentation_contract` | REL-FR-08 and privacy disclosure | Fabricated live evidence |
| REL-APP-1 | Thin installed `repo-com` binary | cli-engineer | Both UI and handler crates | `repo-com-cli`, `command_routing_contract` | REL-FR-01 through 03 | Feature logic, packaging |
| REL-PERF-1 | Repeatable local performance budget | quality-engineer | Final binary | `repo-com-performance`, `performance_contract` | REL-FR-05 and RC-NFR-01 | Live network latency |
| REL-CI-1 | Cross-platform CI policy | release-engineer | Performance harness | CI workflow, advisory/license/secret policy, `ci_policy_contract` | REL-FR-06 and CI constraints | Release artifacts, human approval |
| REL-PACK-1 | Versioned multi-platform packaging | release-engineer | CI and final binary | release workflow, dist policy, `release_policy_contract` | REL-FR-07 and release constraints | Publication, human approval |
| REL-E2E-1 | Full mocked product journey | quality-engineer | Final binary | E2E test, support module, exact fixtures | REL-FR-04 and primary success guardrails | Real Discord |
| REL-UX-HR-1 | Human terminal UX/accessibility rubric | Human | UI, E2E, docs | `docs/reviews/terminal-ux.json` | REL-FR-03 and accessibility constraints | Autonomous score |
| REL-SEC-HR-1 | Human security/privacy rubric | Human | E2E, CI, packaging, docs | `docs/reviews/security-privacy.json` | REL-FR-07/08 and security/privacy constraints | Implementation fixes |
| REL-LIVE-HR-1 | Real Discord round-trip acceptance | Human | UX, security, CI, packaging, docs | `docs/reviews/discord-live-acceptance.json` | REL-FR-09 and primary success | Shared production bot |
| REL-SIGN-HR-1 | Final release decision | Human | All evidence and reviews | `docs/reviews/release-signoff.json` | Vision acceptance criteria and release constraints | Agent approval |

---

## Phase 1: Presentation, Handler, and Documentation Boundaries

```forge-task
{
  "id": "REL-UI-OUT-1",
  "title": "Implement accessible outbound terminal presentation",
  "description": "Implement the focused repo-com-terminal-outbound crate for draft preview, policy status, exact approval, secret-finding override, and accepted, failed, retry-wait, unknown, or reconciled send outcomes. Render complete labeled text at an 80-column minimum, honor NO_COLOR, never encode meaning by color alone, and keep approval and override unavailable in non-TTY mode. Produce deterministic snapshots and keyboard-flow tests, but do not render inbound or lifecycle views, execute commands, call Discord, or self-approve human review.",
  "ownerAgent": "cli-ux-engineer",
  "dependencies": ["PLAT-1", "REPO-POLICY-1", "DRAFT-APPROVAL-1", "DISC-DELIVERY-2"],
  "expectedOutputs": [
    "crates/repo-com-terminal-outbound/Cargo.toml",
    "crates/repo-com-terminal-outbound/src/lib.rs",
    "crates/repo-com-terminal-outbound/src/render.rs",
    "crates/repo-com-terminal-outbound/src/preview.rs",
    "crates/repo-com-terminal-outbound/src/prompt.rs",
    "crates/repo-com-terminal-outbound/src/width.rs",
    "crates/repo-com-terminal-outbound/tests/terminal_outbound_contract.rs",
    "crates/repo-com-terminal-outbound/tests/snapshots/terminal_outbound_contract.snap"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(terminal_outbound_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-03"
    ],
    "acceptanceCriteria": [
      "Snapshot tests cover every outbound view and outcome at 80 columns with no ANSI when color is disabled",
      "Prompt tests cover cancel, default, invalid input, expiry, and complete exact-preview confirmation for approval and secret override",
      "Linear output tests prove destination, revision, approval or policy basis, safety state, delivery outcome, and next action are text labels rather than color or position alone",
      "Keyboard tests prove all outbound actions are reachable without a mouse and no prompt is invoked in non-TTY mode"
    ],
    "constraints": [
      "Outbound presentation is deterministic and cannot execute domain actions itself"
    ],
    "constraintRefs": [
      "docs/PRD.md#RC-ACC-01",
      "docs/PRD.md#RC-ACC-02",
      "docs/PRD.md#RC-ACC-03",
      "docs/features/release-readiness.md#REL-CON-04"
    ],
    "references": [
      "docs/PRD.md#12. User Interface / Interaction Design",
      "docs/features/release-readiness.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "REL-UI-OPS-1",
  "title": "Implement accessible operations terminal presentation",
  "description": "Implement the focused repo-com-terminal-operations crate for config and policy status, state verification, bounded audit and lifecycle inspection, inbound untrusted items, acknowledgement, archive, retention status, purge plan, confirmed purge execution, and local errors. Render complete labeled text at an 80-column minimum, honor NO_COLOR, distinguish untrusted content and local from remote state, and keep activation and purge confirmation unavailable in non-TTY mode. Produce deterministic snapshots and keyboard-flow tests, but do not render outbound messaging, execute commands, call Discord, or self-approve human review.",
  "ownerAgent": "cli-ux-engineer",
  "dependencies": ["PLAT-1", "IN-STATE-1", "REPO-AUDIT-2", "PRIV-RET-1", "PRIV-LIFE-1"],
  "expectedOutputs": [
    "crates/repo-com-terminal-operations/Cargo.toml",
    "crates/repo-com-terminal-operations/src/lib.rs",
    "crates/repo-com-terminal-operations/src/render.rs",
    "crates/repo-com-terminal-operations/src/purge.rs",
    "crates/repo-com-terminal-operations/src/prompt.rs",
    "crates/repo-com-terminal-operations/src/width.rs",
    "crates/repo-com-terminal-operations/tests/terminal_operations_contract.rs",
    "crates/repo-com-terminal-operations/tests/snapshots/terminal_operations_contract.snap"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(terminal_operations_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-03"
    ],
    "acceptanceCriteria": [
      "Snapshot tests cover every operations, inbound, audit, retention, purge, and local-state view at 80 columns with no ANSI when color is disabled",
      "Prompt tests cover cancel, default, invalid input, plan-hash change, and complete exact-scope confirmation for activation and purge execution",
      "Linear output tests label inbound content as untrusted, distinguish last-fetched remote state from local state, and never present delivery as read",
      "Keyboard tests prove all operations actions are reachable without a mouse and no prompt is invoked in non-TTY mode"
    ],
    "constraints": [
      "Operations presentation is deterministic and cannot execute domain actions itself"
    ],
    "constraintRefs": [
      "docs/PRD.md#RC-ACC-01",
      "docs/PRD.md#RC-ACC-02",
      "docs/PRD.md#RC-ACC-03",
      "docs/PRD.md#RC-SEC-07",
      "docs/features/release-readiness.md#REL-CON-04"
    ],
    "references": [
      "docs/PRD.md#12. User Interface / Interaction Design",
      "docs/features/release-readiness.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "REL-OPS-CMD-1",
  "title": "Implement operator and lifecycle command handlers",
  "description": "Implement the focused repo-com-cli-operations crate with validated handlers for config, policy, state verification, audit inspection, purge planning, and confirmed purge execution. Parse structured stdin JSON at protocol version 1, enforce explicit repository and object identifiers, map domain outcomes to stable exit categories, route activation and purge confirmation through the operations UI adapter, and keep diagnostics off machine stdout. Do not implement draft, send, inbox, or reply commands, the final executable, releases, live services, or human review evidence.",
  "ownerAgent": "cli-engineer",
  "dependencies": ["PLAT-1", "REPO-CFG-1", "REPO-POLICY-1", "PRIV-LIFE-1", "REL-UI-OPS-1"],
  "expectedOutputs": [
    "crates/repo-com-cli-operations/Cargo.toml",
    "crates/repo-com-cli-operations/src/lib.rs",
    "crates/repo-com-cli-operations/src/input.rs",
    "crates/repo-com-cli-operations/src/handlers/config.rs",
    "crates/repo-com-cli-operations/src/handlers/policy.rs",
    "crates/repo-com-cli-operations/src/handlers/state.rs",
    "crates/repo-com-cli-operations/src/handlers/audit.rs",
    "crates/repo-com-cli-operations/src/handlers/purge.rs",
    "crates/repo-com-cli-operations/tests/cli_operations_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(cli_operations_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-02",
      "docs/features/release-readiness.md#REL-FR-03"
    ],
    "acceptanceCriteria": [
      "Handler tests cover config validation, policy status, activation, state verification, audit query, purge plan, and purge execution with explicit repository identifiers",
      "Structured-input tests cover protocol version 1, unknown-field rejection, bounded pagination, and one valid JSON stdout object for every failure",
      "TTY tests prove activation and purge execution require the operations UI and return operator-action-required in automation",
      "Dispatch tests prove state verification is read-only, purge plan is non-mutating, and services retain their own authorization and transaction rules"
    ],
    "constraints": [
      "Operator handlers validate and route but do not reimplement lifecycle safety logic"
    ],
    "constraintRefs": [
      "docs/features/cli-foundation.md#FOUND-CON-02",
      "docs/features/privacy-and-lifecycle-operations.md#LIFE-CON-01"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/PRD.md#12. User Interface / Interaction Design"
    ]
  }
}
```

```forge-task
{
  "id": "REL-MSG-CMD-1",
  "title": "Implement messaging command handlers",
  "description": "Implement the focused repo-com-cli-messaging crate with validated handlers for draft create, show, update, preview, approval, send, setup check, inbox fetch, item acknowledgement or archive, and reply-draft creation. Parse structured stdin JSON at protocol version 1, enforce explicit draft, revision, destination, cursor or time, and inbound identifiers, map domain outcomes to stable exit categories, and route approval and override through the outbound UI adapter. Do not implement config, policy, audit, state, purge, the final executable, releases, live services, or human review evidence.",
  "ownerAgent": "cli-engineer",
  "dependencies": ["PLAT-1", "DRAFT-ELIG-1", "DISC-CLIENT-1", "DISC-DELIVERY-2", "IN-FETCH-1", "IN-REPLY-1", "REL-UI-OUT-1"],
  "expectedOutputs": [
    "crates/repo-com-cli-messaging/Cargo.toml",
    "crates/repo-com-cli-messaging/src/lib.rs",
    "crates/repo-com-cli-messaging/src/input.rs",
    "crates/repo-com-cli-messaging/src/handlers/draft.rs",
    "crates/repo-com-cli-messaging/src/handlers/send.rs",
    "crates/repo-com-cli-messaging/src/handlers/inbox.rs",
    "crates/repo-com-cli-messaging/src/handlers/reply.rs",
    "crates/repo-com-cli-messaging/tests/cli_messaging_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(cli_messaging_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-02",
      "docs/features/release-readiness.md#REL-FR-03"
    ],
    "acceptanceCriteria": [
      "Handler tests cover every messaging command with protocol version 1, unknown-field rejection, and one valid JSON stdout object for failures",
      "Boundary tests require exact draft and revision identifiers, reject a fetch without exactly one cursor or time source, and expose no default destination",
      "TTY tests prove approval and secret override require the outbound UI and cannot be created in non-TTY mode",
      "Dispatch tests prove send, fetch, and reply services retain eligibility, delivery idempotency, inbound trust, and target authorization rules"
    ],
    "constraints": [
      "Messaging handlers validate and route but do not reimplement draft or delivery safety logic"
    ],
    "constraintRefs": [
      "docs/features/cli-foundation.md#FOUND-CON-02",
      "docs/PRD.md#RC-SEC-05",
      "docs/PRD.md#RC-SEC-07"
    ],
    "references": [
      "docs/PRD.md#6. Concept",
      "docs/PRD.md#7. Technical Architecture",
      "docs/PRD.md#12. User Interface / Interaction Design"
    ]
  }
}
```

```forge-task
{
  "id": "REL-DOC-1",
  "title": "Publish validated operator and security documentation",
  "description": "Create the focused repo-com-doc-validation crate and author configuration, Discord setup, operator guide, security model, and threat model documents. Cover installation, exact command and JSON protocol, dedicated bot creation, least-privilege grants, token rotation, preview and approval, narrow policy activation, unknown-delivery recovery, inbound reply, audit, retention, purge, accessibility, unsupported behavior, and residual local-state risk. Validate required topics and prohibit credentials, unsupported claims, telemetry claims, or human sign-off. Do not fabricate live test or compliance evidence.",
  "ownerAgent": "technical-writer",
  "dependencies": ["REL-UI-OUT-1", "REL-UI-OPS-1", "REL-OPS-CMD-1", "REL-MSG-CMD-1"],
  "expectedOutputs": [
    "crates/repo-com-doc-validation/Cargo.toml",
    "crates/repo-com-doc-validation/src/lib.rs",
    "crates/repo-com-doc-validation/tests/documentation_contract.rs",
    "docs/configuration.md",
    "docs/discord-setup.md",
    "docs/operator-guide.md",
    "docs/security-model.md",
    "docs/threat-model.md"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(documentation_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-08"
    ],
    "acceptanceCriteria": [
      "Documentation tests require every listed setup, operation, recovery, retention, accessibility, unsupported behavior, and residual-risk topic in its owning document",
      "Documentation tests reject token, private-key, authorization-value, and real-message-content patterns",
      "Security and threat-model tests explicitly state no encryption at rest, user-only filesystem permissions, no telemetry, and exposure to local account, backup, and snapshot compromise",
      "Operator documentation matches both command-handler crates and protocol version without inventing flags, defaults, or completion claims"
    ],
    "constraints": [
      "Documentation is evidence for review, not human approval"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-02",
      "docs/features/release-readiness.md#REL-CON-06",
      "docs/features/release-readiness.md#REL-CON-04"
    ],
    "references": [
      "docs/PRD.md#10. Security and Privacy",
      "docs/PRD.md#17. Acceptance Criteria",
      "docs/PRD.md#20. Open Questions"
    ]
  }
}
```

## Phase 2: Thin Executable

```forge-task
{
  "id": "REL-APP-1",
  "title": "Compose the final repo-com binary",
  "description": "Compose the final repo-com-cli crate as a thin executable around the operator, messaging, outbound UI, and operations UI crates. Define the complete command tree and global options, map the package version to --version, propagate stable exit codes, and keep panic output, prompts, diagnostics, and protocol data on their required streams. Do not add domain logic, default destinations, self-update, background work, or human review claims.",
  "ownerAgent": "cli-engineer",
  "dependencies": ["REL-UI-OUT-1", "REL-UI-OPS-1", "REL-OPS-CMD-1", "REL-MSG-CMD-1"],
  "expectedOutputs": [
    "crates/repo-com-cli/Cargo.toml",
    "crates/repo-com-cli/src/main.rs",
    "crates/repo-com-cli/src/app.rs",
    "crates/repo-com-cli/tests/command_routing_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(command_routing_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-01",
      "docs/features/release-readiness.md#REL-FR-02",
      "docs/features/release-readiness.md#REL-FR-03",
      "docs/features/cli-foundation.md#FOUND-FR-01",
      "docs/features/cli-foundation.md#FOUND-FR-02",
      "docs/features/cli-foundation.md#FOUND-FR-04",
      "docs/PRD.md#RC-FR-01",
      "docs/PRD.md#RC-FR-02"
    ],
    "acceptanceCriteria": [
      "Process tests prove repo-com --version prints only the package semantic version and exits successfully",
      "Routing tests cover every command group, help output, missing identifier, and no hidden default destination",
      "Stream tests prove JSON mode writes one object to stdout, diagnostics to stderr, and prompts never to stdout",
      "Exit tests map each domain category to its documented stable process code and preserve the original typed cause for machine output"
    ],
    "constraints": [
      "The final binary contains no self-updater or hidden background process"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-03"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/PRD.md#12. User Interface / Interaction Design"
    ]
  }
}
```

## Phase 3: Performance, Infrastructure, and End-to-End Evidence

```forge-task
{
  "id": "REL-PERF-1",
  "title": "Measure the local command performance budget",
  "description": "Implement the focused repo-com-performance harness for 100 complete warm invocations of each documented no-network command class against the final binary on the pinned CI reference runner. Measure end-to-end process wall time while excluding first compilation and network activity; record machine, OS, toolchain, sample count, p50, p95, and maximum, and fail when any class exceeds 500 ms at p95. Use a fixed temporary repository and state, and do not benchmark Discord latency or claim results on unrecorded hardware.",
  "ownerAgent": "quality-engineer",
  "dependencies": ["REL-APP-1"],
  "expectedOutputs": [
    "crates/repo-com-performance/Cargo.toml",
    "crates/repo-com-performance/src/lib.rs",
    "crates/repo-com-performance/src/main.rs",
    "crates/repo-com-performance/tests/performance_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(performance_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-05"
    ],
    "acceptanceCriteria": [
      "Performance tests execute exactly 100 warm samples for each required no-network command class and record environment plus p50, p95, and maximum",
      "The contract fails when injected p95 exceeds 500 ms and passes at the threshold",
      "Test setup uses isolated config and state and performs no Discord or other external network request",
      "The report includes complete process wall time and separately identifies first compilation and network exclusions"
    ],
    "constraints": [
      "Performance evidence is generated by the harness rather than asserted in prose"
    ],
    "constraintRefs": [
      "docs/PRD.md#RC-NFR-01"
    ],
    "references": [
      "docs/PRD.md#15. Testing Strategy",
      "docs/PRD.md#16. Analytics / Success Metrics"
    ]
  }
}
```

```forge-task
{
  "id": "REL-CI-1",
  "title": "Create cross-platform CI policy",
  "description": "Create the focused repo-com-ci-policy crate plus pinned cargo-deny 0.20.2, cargo-audit 0.22.2, actionlint 1.7.12, dependency update, and continuous-integration configuration. Run formatting, clippy, fail-on-no-tests nextest, performance, RustSec, license, secret, workflow, SQLite-version, and binary smoke checks on Linux, macOS, and Windows with least-privilege workflow permissions and concurrency control. Do not define release artifacts, publish, use a real Discord secret, run a live service, or claim human approval.",
  "ownerAgent": "release-engineer",
  "dependencies": ["REL-PERF-1", "REL-APP-1"],
  "expectedOutputs": [
    "crates/repo-com-ci-policy/Cargo.toml",
    "crates/repo-com-ci-policy/src/lib.rs",
    "crates/repo-com-ci-policy/tests/ci_policy_contract.rs",
    "deny.toml",
    ".github/dependabot.yml",
    ".github/workflows/ci.yml"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(ci_policy_contract)'",
    "actionlint .github/workflows/ci.yml"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-06"
    ],
    "acceptanceCriteria": [
      "Policy tests require Linux, macOS, and Windows jobs with pinned tools, least-privilege permissions, concurrency control, and no untrusted pull-request secret use",
      "CI contract requires fmt, clippy, fail-on-no-tests nextest, performance, cargo-deny, cargo-audit, secret scan, actionlint, SQLite runtime assertion, and binary smoke checks",
      "Actionlint validation succeeds for CI and missing or renamed required checks fail the contract",
      "No workflow, fixture, generated metadata, or log path contains a real Discord credential or team message"
    ],
    "constraints": [
      "Normal CI remains token-free and does not publish release artifacts"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-01",
      "docs/features/release-readiness.md#REL-CON-02",
      "docs/features/release-readiness.md#REL-CON-05",
      "docs/features/repository-configuration-and-state.md#STATE-CON-02",
      "docs/PRD.md#RC-NFR-03"
    ],
    "references": [
      "docs/PRD.md#5. Research Findings",
      "docs/PRD.md#15. Testing Strategy",
      "docs/PRD.md#18. Dependencies and Risks"
    ]
  }
}
```

```forge-task
{
  "id": "REL-PACK-1",
  "title": "Create versioned release packaging",
  "description": "Create the focused repo-com-release-policy crate and a tag-driven release workflow using cargo-dist 0.32.0 and cargo-cyclonedx 0.5.9. Build versioned Linux, macOS, and Windows binaries and archives or installers with SQLite 3.53.4 or newer statically linked, SHA-256 or stronger checksums, license evidence, source archive, and CycloneDX SBOM. Keep installation explicit, disable every updater, and require the CI policy, performance budget, release-plan validation, and actionlint. Do not publish during implementation, embed a token, or claim human approval.",
  "ownerAgent": "release-engineer",
  "dependencies": ["REL-CI-1", "REL-PERF-1", "REL-APP-1"],
  "expectedOutputs": [
    "crates/repo-com-release-policy/Cargo.toml",
    "crates/repo-com-release-policy/src/lib.rs",
    "crates/repo-com-release-policy/tests/release_policy_contract.rs",
    "dist-workspace.toml",
    ".github/workflows/release.yml"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(release_policy_contract)'",
    "actionlint .github/workflows/release.yml"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-07"
    ],
    "acceptanceCriteria": [
      "Release-plan tests cover required Linux, macOS, and Windows targets, semantic version metadata, source archive, strong checksums, license evidence, and CycloneDX SBOM",
      "Release contract statically links and asserts SQLite 3.53.4 or newer and fails for an older bundled or system runtime",
      "Release workflow is tag-driven, invokes the validated CI evidence, uses least-privilege permissions, and contains no updater or setup mutation",
      "Actionlint validation succeeds and generated release-plan drift fails the contract",
      "No release configuration, generated manifest, checksum input, or SBOM contains a Discord credential or team message"
    ],
    "constraints": [
      "Release publication is tag-driven and cannot be performed by this task"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-01",
      "docs/features/release-readiness.md#REL-CON-02",
      "docs/features/release-readiness.md#REL-CON-03",
      "docs/features/release-readiness.md#REL-CON-05",
      "docs/features/repository-configuration-and-state.md#STATE-CON-02",
      "docs/PRD.md#RC-NFR-03"
    ],
    "references": [
      "docs/PRD.md#5. Research Findings",
      "docs/PRD.md#15. Testing Strategy",
      "docs/PRD.md#18. Dependencies and Risks"
    ]
  }
}
```

```forge-task
{
  "id": "REL-E2E-1",
  "title": "Prove the complete mocked communication workflow",
  "description": "Create the final binary's end-to-end contract, support module, and exact non-secret fixtures for a full isolated journey: validate config, activate exact policy or approve, create and preview, send through wiremock, fetch a human reply and mention, create a validated reply draft, acknowledge locally, inspect audit, and execute a purge plan. Also cover concurrent duplicate send and unknown reconciliation branches. Use temp user-data paths and no real credential or network; do not claim live acceptance or human approval.",
  "ownerAgent": "quality-engineer",
  "dependencies": ["REL-APP-1"],
  "expectedOutputs": [
    "crates/repo-com-cli/tests/e2e_workflow.rs",
    "crates/repo-com-cli/tests/support/mod.rs",
    "crates/repo-com-cli/tests/fixtures/valid-config.toml",
    "crates/repo-com-cli/tests/fixtures/accepted-message.json",
    "crates/repo-com-cli/tests/fixtures/human-reply.json"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(e2e_workflow)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-04",
      "docs/PRD.md#RC-FR-06"
    ],
    "acceptanceCriteria": [
      "The happy-path test executes every named command against isolated state and wiremock and verifies durable IDs, correlation, acknowledgement, audit, and purge-plan results",
      "A 100-invocation concurrent branch proves one Discord create request and one accepted outcome for one revision",
      "Timeout and matching-message branches prove unknown reconciliation and duplicate prevention without a second unsafe POST",
      "Fixture and captured-output scans prove no real token, authorization value, or team message content",
      "The test performs no external network request and fails when wiremock is bypassed"
    ],
    "constraints": [
      "End-to-end evidence is mocked and does not satisfy live acceptance"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-01",
      "docs/features/release-readiness.md#REL-CON-02",
      "docs/PRD.md#RC-NFR-02"
    ],
    "references": [
      "docs/PRD.md#RC-US-01",
      "docs/PRD.md#6. Concept",
      "docs/PRD.md#15. Testing Strategy",
      "docs/PRD.md#17. Acceptance Criteria"
    ]
  }
}
```

## Phase 4: Human UX and Security Rubrics

```forge-task
{
  "id": "REL-UX-HR-1",
  "title": "Review terminal UX and accessibility",
  "description": "Perform the human terminal usability and accessibility review after the implementation, documentation, performance, and mocked end-to-end evidence exists. Use the final binary at 80 columns with color disabled and with a screen reader or linear terminal reader; complete configuration, approval, policy, send outcome, unknown recovery, fetch, reply, acknowledgement, and purge-plan flows by keyboard. Score discoverability, exact preview comprehension, error recovery, linear reading, focus or selection visibility, non-color meaning, and non-TTY safety from 0 through 4 in docs/reviews/terminal-ux.json, record evidence and defects, and do not approve implementation or live compatibility.",
  "dependencies": ["REL-UI-OUT-1", "REL-UI-OPS-1", "REL-APP-1", "REL-PERF-1", "REL-DOC-1", "REL-E2E-1"],
  "expectedOutputs": [],
  "validationCommands": [],
  "contract": {
    "version": 2,
    "kind": "human-review",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-03"
    ],
    "acceptanceCriteria": [
      "A human reviewer records environment, tested commands, observed evidence, defect references, and a 0-4 score for every named rubric dimension",
      "Any critical failure in exact preview, keyboard operation, non-TTY safety, or security information visibility is recorded as a release blocker",
      "The review distinguishes automated implementation evidence from human judgment and does not claim live Discord acceptance"
    ],
    "constraints": [
      "Only a human reviewer may populate the rubric"
    ],
    "constraintRefs": [
      "docs/PRD.md#RC-ACC-01",
      "docs/PRD.md#RC-ACC-02",
      "docs/PRD.md#RC-ACC-03",
      "docs/features/release-readiness.md#REL-CON-04"
    ],
    "references": [
      "docs/PRD.md#RC-US-02",
      "docs/PRD.md#11. Accessibility",
      "docs/PRD.md#12. User Interface / Interaction Design"
    ],
    "reviewFile": "docs/reviews/terminal-ux.json"
  }
}
```

```forge-task
{
  "id": "REL-SEC-HR-1",
  "title": "Review security and privacy evidence",
  "description": "Perform the human security and privacy rubric after automated dependency, secret, release, E2E, and documentation evidence exists. Inspect the threat model, permission matrix, token flow, config and state boundaries, approval and policy activation, duplicate protection, unknown recovery, log redaction, retention and purge, dependency policy, artifact evidence, and residual local-account risk. Score threat coverage, least privilege, secret handling, untrusted-input isolation, duplicate safety, privacy enforcement, recovery safety, and residual-risk disclosure from 0 through 4 in docs/reviews/security-privacy.json, record evidence and blockers, and do not sign final release approval.",
  "dependencies": ["REL-CI-1", "REL-PACK-1", "REL-DOC-1", "REL-E2E-1"],
  "expectedOutputs": [],
  "validationCommands": [],
  "contract": {
    "version": 2,
    "kind": "human-review",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-07",
      "docs/features/release-readiness.md#REL-FR-08"
    ],
    "acceptanceCriteria": [
      "A human reviewer records scope, evidence reviewed, defect references, and a 0-4 score for every named rubric dimension",
      "The review verifies bot-only least privilege, no secret persistence, exact approval or policy activation, inbound isolation, and no unsafe unknown resend",
      "The review verifies retention, purge, no telemetry, artifact checksums and SBOM, and explicit unencrypted-state residual risk",
      "Any critical security or privacy failure is recorded as a release blocker without autonomous remediation or approval"
    ],
    "constraints": [
      "Only a human reviewer may populate the security and privacy rubric"
    ],
    "constraintRefs": [
      "docs/PRD.md#RC-SEC-01",
      "docs/PRD.md#RC-SEC-07",
      "docs/PRD.md#RC-PRIV-01",
      "docs/PRD.md#RC-PRIV-02",
      "docs/features/release-readiness.md#REL-CON-04",
      "docs/features/release-readiness.md#REL-CON-06"
    ],
    "references": [
      "docs/PRD.md#10. Security and Privacy",
      "docs/features/release-readiness.md#3. Canonical Requirements"
    ],
    "reviewFile": "docs/reviews/security-privacy.json"
  }
}
```

## Phase 5: Human Live Discord Acceptance

```forge-task
{
  "id": "REL-LIVE-HR-1",
  "title": "Perform the real Discord round trip",
  "description": "Perform the human-controlled live acceptance in a disposable Discord workspace using a dedicated least-privilege bot and no production team content. Validate setup, create and preview a unique test request, use explicit approval or an exact activated policy, confirm one send, have a teammate reply, fetch and correlate that reply, create and deliver a validated reply draft, acknowledge locally, inspect audit and purge-plan behavior, and exercise the documented unknown-outcome recovery without creating a duplicate. Record environment, timestamps, stable IDs with secret-safe redaction, observed outcomes, defects, and pass or fail in docs/reviews/discord-live-acceptance.json. Rotate or revoke the test token after the session and do not write implementation code.",
  "dependencies": ["REL-UX-HR-1", "REL-SEC-HR-1", "REL-CI-1", "REL-PACK-1", "REL-DOC-1"],
  "expectedOutputs": [],
  "validationCommands": [],
  "contract": {
    "version": 2,
    "kind": "human-review",
    "requirements": [],
    "requirementRefs": [
      "docs/features/release-readiness.md#REL-FR-09",
      "docs/PRD.md#RC-FR-06"
    ],
    "acceptanceCriteria": [
      "A human reviewer records disposable workspace, bot permission evidence, test timing, stable redacted IDs, teammate participation, and token revocation or rotation confirmation",
      "The live record proves one outbound message, one fetched human reply with correlation, one accepted validated reply draft, local acknowledgement, and usable audit evidence",
      "The live record includes the unknown-recovery observation and proves no duplicate remote message or unsafe automatic resend",
      "Any permission, delivery, correlation, recovery, privacy, or usability defect is recorded as a release blocker"
    ],
    "constraints": [
      "Live acceptance uses a dedicated disposable bot and never records the token"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-01",
      "docs/features/release-readiness.md#REL-CON-02",
      "docs/features/release-readiness.md#REL-CON-04"
    ],
    "references": [
      "docs/PRD.md#6. Concept",
      "docs/PRD.md#17. Acceptance Criteria",
      "docs/features/release-readiness.md#3. Canonical Requirements"
    ],
    "reviewFile": "docs/reviews/discord-live-acceptance.json"
  }
}
```

## Phase 6: Human Release Sign-Off

```forge-task
{
  "id": "REL-SIGN-HR-1",
  "title": "Record final release sign-off",
  "description": "Perform the final human release decision only after automated validation, performance, cross-platform CI, documentation, UX rubric, security/privacy rubric, and live Discord acceptance are available. Verify that every referenced evidence item exists, every blocker is resolved or explicitly rejected, the SQLite and dependency gates passed, no secret or unsupported claim is present, and the primary teammate-reply outcome was demonstrated. Record reviewers, evidence hashes, decision, conditions, and date in docs/reviews/release-signoff.json. Do not modify code, rerun human judgments, or approve on behalf of any absent reviewer.",
  "dependencies": ["REL-UX-HR-1", "REL-SEC-HR-1", "REL-LIVE-HR-1", "REL-CI-1", "REL-PACK-1", "REL-E2E-1", "REL-PERF-1"],
  "expectedOutputs": [],
  "validationCommands": [],
  "contract": {
    "version": 2,
    "kind": "human-review",
    "requirements": [],
    "requirementRefs": [
      "docs/PRD.md#RC-FR-06"
    ],
    "acceptanceCriteria": [
      "Named human reviewers record an explicit approve, approve with conditions, or reject decision and no absent reviewer is represented",
      "The sign-off links evidence hashes for automated validation, performance, cross-platform artifacts, UX rubric, security rubric, and live acceptance",
      "Any unresolved critical blocker or missing required evidence results in reject rather than implicit approval",
      "The sign-off confirms the release is not self-updating and makes no unsupported privacy, compliance, read-receipt, or Discord compatibility claim"
    ],
    "constraints": [
      "Final approval is human-only and follows all dependent reviews"
    ],
    "constraintRefs": [
      "docs/features/release-readiness.md#REL-CON-03",
      "docs/features/release-readiness.md#REL-CON-04",
      "docs/PRD.md#RC-NFR-02",
      "docs/PRD.md#RC-NFR-03"
    ],
    "references": [
      "docs/PRD.md#17. Acceptance Criteria",
      "docs/PRD.md#18. Dependencies and Risks"
    ],
    "reviewFile": "docs/reviews/release-signoff.json"
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Tasks |
|---|---|---|
| [REL-FR-01](release-readiness.md#REL-FR-01) | requirement | REL-APP-1 |
| [REL-FR-02](release-readiness.md#REL-FR-02) | requirement | REL-OPS-CMD-1, REL-MSG-CMD-1, REL-APP-1 |
| [REL-FR-03](release-readiness.md#REL-FR-03) | requirement | REL-UI-OUT-1, REL-UI-OPS-1, REL-OPS-CMD-1, REL-MSG-CMD-1, REL-APP-1 |
| [REL-FR-04](release-readiness.md#REL-FR-04) | requirement | REL-E2E-1 |
| [REL-FR-05](release-readiness.md#REL-FR-05) | requirement | REL-PERF-1 |
| [REL-FR-06](release-readiness.md#REL-FR-06) | requirement | REL-CI-1 |
| [REL-FR-07](release-readiness.md#REL-FR-07) | requirement | REL-PACK-1 |
| [REL-FR-08](release-readiness.md#REL-FR-08) | requirement | REL-DOC-1 |
| [REL-FR-09](release-readiness.md#REL-FR-09) | requirement | REL-LIVE-HR-1 |
| [REL-CON-01](release-readiness.md#REL-CON-01) | constraint | REL-E2E-1, REL-CI-1, REL-PACK-1 |
| [REL-CON-02](release-readiness.md#REL-CON-02) | constraint | REL-CI-1, REL-PACK-1, REL-DOC-1 |
| [REL-CON-03](release-readiness.md#REL-CON-03) | constraint | REL-APP-1, REL-PACK-1 |
| [REL-CON-04](release-readiness.md#REL-CON-04) | constraint | REL-UX-HR-1, REL-SEC-HR-1, REL-LIVE-HR-1, REL-SIGN-HR-1 |
| [REL-CON-05](release-readiness.md#REL-CON-05) | constraint | REL-CI-1, REL-PACK-1 |
| [REL-CON-06](release-readiness.md#REL-CON-06) | constraint | REL-DOC-1, REL-SEC-HR-1 |
