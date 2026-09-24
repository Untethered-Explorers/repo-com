# repo-com Product Vision

## 1. Overview

**Product Name:** repo-com  
**Summary:** A single-user, local command-line transport and approval workflow for agent-originated team communications through Discord.  
**Target Platform:** Globally installed `repo-com` binaries for Linux, macOS, and Windows, invoked by repository skills and operators in interactive or non-TTY shells.  
**Key Constraints:** Local-first durable state, no shared service, no arbitrary destinations, no user-token impersonation, explicit approval by default, and no silent handling of uncertain delivery.

The authoritative executable requirements and tasks are decomposed into the feature documents listed in section 14. `docs/IDEA.md` is historical source material and is not an execution source.

> **Implementation boundary:** this document is the canonical future product
> contract, not a statement that the complete workflow is currently shipped.
> The current workspace implements the foundation, configuration, state, and
> policy libraries described in the [Library Consumer Guide](user-guide.md).
> Feature checklists remain requirements and plans unless current source and
> tests confirm implementation.

---

## 2. Version History

| Version | Date | Author | Changes |
|---|---|---|---|
| 1.0 | 2026-09-24 | Forge headless authoring | Initial canonical vision and decomposed v1 feature set |

---

## 3. Goals and Non-Goals

### 3.1 Goals

- Let a repository skill create a focused request for a teammate using a stable machine protocol.
- Let an operator inspect and approve the exact draft revision before a normal send.
- Deliver one message to one configured Discord destination with duplicate protection and explicit unknown outcomes.
- Retrieve replies and mentions on demand, treat them as untrusted data, and support validated threaded replies.
- Keep a recoverable local audit trail whose retention is operator-controlled.

### 3.2 Non-Goals

- Email delivery, a shared multi-user service, team accounts, or centralized state.
- A daemon, Gateway connection, live inbox UI, assignment workflow, or collaboration dashboard.
- Broadcasts, arbitrary direct messages, attachments, embeds, reactions, files, scheduled sends, arbitrary templates, AI-generated copy, or full-history search.
- Automatic Discord application creation, server-permission mutation, self-update, or export/import of local state.
- Read receipts, response analytics, or claims that a teammate has viewed a message.

---

## 4. User Stories / Personas

### 4.1 Personas

| Persona | Description | Key Needs |
|---|---|---|
| Repository skill | An AI-assisted workflow running inside the operator's repository | Structured input/output, explicit destinations, durable identifiers, recoverable failures |
| Operator | The software owner who controls credentials, approvals, and retention | Exact preview, keyboard-usable prompts, non-duplicative delivery, local audit and purge controls |
| Teammate | A Discord recipient who does not use repo-com | A concise actionable message, an understandable channel, and a normal Discord reply |

### 4.2 Canonical User Stories

| ID | Priority | Definition |
|---|---|---|
| RC-US-01 | Must | [RC-US-01](#rc-us-01) |
| RC-US-02 | Must | [RC-US-02](#rc-us-02) |
| RC-US-03 | Must | [RC-US-03](#rc-us-03) |
| RC-US-04 | Must | [RC-US-04](#rc-us-04) |

```forge-requirement
{"id":"RC-US-01","kind":"story","text":"As a repository skill, I want to create a structured request for a configured team destination so that important work can reach a human without direct credential access."}
```

```forge-requirement
{"id":"RC-US-02","kind":"story","text":"As the operator, I want to preview and approve the exact text, metadata, and destination revision so that an agent cannot send unintended content."}
```

```forge-requirement
{"id":"RC-US-03","kind":"story","text":"As the operator, I want duplicate-safe delivery outcomes and a local audit trail so that retries are safe and past transitions remain attributable."}
```

```forge-requirement
{"id":"RC-US-04","kind":"story","text":"As a repository skill, I want on-demand replies and mentions with local acknowledgement so that I can continue a conversation without a background service."}
```

---

## 5. Research Findings

Technology currency was checked on 2026-09-24. Versions below are planning baselines; implementation must refresh advisories and patch versions before release without changing major versions silently.

| Area | Finding | Planning decision |
|---|---|---|
| Language | Rust 1.98.1 is the current stable release and supports the 2024 edition, strong CLI ergonomics, deterministic builds, and cross-platform static binaries. | Use Rust 1.98.1 and Cargo workspaces. |
| CLI and protocol | clap 4.6.7, serde 1.0.229, serde_json 1.0.151, and toml 1.1.6 are current. | Use a typed command tree plus versioned JSON on stdin/stdout. |
| Async HTTP | tokio 1.53.1 and reqwest 0.13.5 are current; reqwest supports rustls-based TLS. | Use a bounded Tokio runtime and rustls-backed reqwest client. |
| Local state | rusqlite 0.40.2 and rusqlite_migration 2.6.0 are current. The current rusqlite bundled source is SQLite 3.53.2, while upstream SQLite 3.53.4 fixes a WAL-reset corruption bug. | Require linked SQLite 3.53.4 or newer; do not ship the older bundled source. |
| Platform paths and secrets | dirs 7.0.0, secrecy 0.10.3, and zeroize 1.9.0 are current. | Resolve OS user-data paths and keep tokens in redacted, zeroizing wrappers. |
| Discord | Discord REST API v10 remains available; rate limits must be read from response headers and `Retry-After`, not hard-coded. Discord advises idempotent behavior because requests and events are not strongly consistent. | Pin `/api/v10`, use a dedicated bot, classify outcomes conservatively, and reconcile an ambiguous send before resend. |
| Accessibility | WCAG 2.2 AA is the current W3C Recommendation baseline; terminal output is plain text interpreted by the user's terminal and assistive technology. | Require linear labels, keyboard operation, no color-only meaning, no prompts in non-TTY mode, and a no-ANSI fallback. |
| Secrets | OWASP guidance says secrets should not be logged and detection must balance recall with false positives. | Add pre-send lightweight detection, an explicit exact-revision TTY override, and CI secret scanning. |

Primary research sources: Rust release notes, crates.io package pages, Discord API and rate-limit documentation, SQLite release history, W3C WCAG 2.2, and OWASP Secrets Management guidance.

---

## 6. Concept

### 6.1 Core Loop

```text
Skill creates revision
  -> operator or policy decides eligibility
  -> destination and approval are revalidated
  -> delivery is claimed atomically
  -> Discord send is attempted
  -> accepted / failed / unknown is recorded
  -> unknown is reconciled before any resend
  -> later fetch returns replies and mentions
  -> skill creates a validated reply draft
  -> operator or activated policy handles that reply like any other draft
```

### 6.2 Completion Criteria

A v1 workflow is complete when a real teammate can notice a test request in a configured Discord channel, reply to it, and have that reply retrieved and correlated locally while all automated tests, security checks, cross-platform builds, and dependent human reviews pass.

---

## 7. Technical Architecture

### 7.1 Technology Stack

| Component | Technology | Verified baseline | Purpose |
|---|---|---|---|
| Language and build | Rust 2024 edition, Cargo | Rust 1.98.1 | Cross-platform native CLI and reproducible dependency locking |
| Command parsing | clap | 4.6.7 | Typed subcommands and non-TTY-safe arguments |
| Serialization | serde, serde_json, toml | 1.0.229, 1.0.151, 1.1.6 | Configuration and versioned machine protocol |
| Async runtime | tokio | 1.53.1 | Bounded Discord I/O and retry timing |
| HTTP and TLS | reqwest with rustls | 0.13.5 | Discord REST v10 calls without OpenSSL runtime dependency |
| Local database | rusqlite, rusqlite_migration | 0.40.2, 2.6.0 | Transactional repository-scoped state |
| SQLite runtime | SQLite, statically linked in release artifacts | 3.53.4 minimum | Current patched local database engine |
| User-data paths | dirs | 7.0.0 | Linux, macOS, and Windows application-data locations |
| Interactive prompts | dialoguer | 0.12.0 | Keyboard-usable TTY confirmations |
| Secret handling | secrecy, zeroize | 0.10.3, 1.9.0 | Redacted token wrapper and best-effort memory clearing |
| Diagnostics | tracing, tracing-subscriber | 0.1.44, 0.3.23 | Opt-in structured diagnostics with redaction |
| Test tooling | cargo-nextest, assert_cmd, predicates, insta, wiremock, tempfile | 0.9.146, 2.2.2, 3.1.4, 1.48.0, 0.6.5, 3.27.0 | Fail-on-zero-tests selection, CLI tests, snapshots, HTTP mocks, and isolated state |
| Security tooling | cargo-deny, cargo-audit | 0.20.2, 0.22.2 | Dependency policy and RustSec checks |
| Workflow lint | actionlint | 1.7.12 | Static GitHub Actions workflow validation |
| SBOM generation | cargo-cyclonedx | 0.5.9 | CycloneDX software bill of materials for release artifacts |
| Release packaging | cargo-dist | 0.32.0 | Installers, archives, checksums, and multi-platform release artifacts |

No selected major technology is deprecated or end-of-life as of the verification date.

### 7.2 Project Structure

```text
Cargo.toml
rust-toolchain.toml
.config/nextest.toml
dist-workspace.toml
crates/
  repo-com-foundation/
  repo-com-config/
  repo-com-state/
  repo-com-policy/
  repo-com-audit/
  repo-com-audit-query/
  repo-com-draft-model/
  repo-com-draft-content/
  repo-com-draft-safety/
  repo-com-approval/
  repo-com-send-eligibility/
  repo-com-discord-client/
  repo-com-discord-message/
  repo-com-delivery/
  repo-com-delivery-retry/
  repo-com-inbox-fetch/
  repo-com-inbox-state/
  repo-com-reply/
  repo-com-retention/
  repo-com-purge/
  repo-com-lifecycle/
  repo-com-terminal-outbound/
  repo-com-terminal-operations/
  repo-com-cli-operations/
  repo-com-cli-messaging/
  repo-com-cli/
  repo-com-performance/
  repo-com-ci-policy/
  repo-com-release-policy/
  repo-com-doc-validation/
.github/workflows/
  ci.yml
  release.yml
```

Focused library crates are intentional: they make independently testable behaviors and retry boundaries explicit while the final `repo-com-cli` crate composes them into one installed binary.

### 7.3 Key Interfaces

| Interface | Contract |
|---|---|
| `CommandOutcome` | One JSON envelope containing protocol version, status, data, and a stable error code; diagnostics never contaminate stdout. |
| `ConfigResolver` | Finds an explicit or repository-local config, parses schema version 1, rejects unknown fields and secrets, and resolves aliases without widening policy. |
| `StateStore` | Opens a repository-keyed SQLite database, applies forward migrations, enables foreign keys/WAL/busy timeout, and exposes transactional repositories. |
| `PolicyRegistry` | Activates an exact canonical configuration hash and exact event/destination/severity tuple through an interactive operator action. |
| `DraftRepository` | Stores immutable revisions and their content hashes, resolved destination snapshot, policy decision, expiry, and lifecycle state. |
| `EligibilityEvaluator` | Revalidates the current config, alias, approval or activated policy, secret scan, revision hash, and expiry immediately before a send claim. |
| `DiscordClient` | Authenticates only with a bot token, pins REST v10, performs read-only setup checks, and exposes typed HTTP outcomes. |
| `DeliveryCoordinator` | Atomically claims a send, enforces duplicate protection, records each attempt, and returns the recorded outcome to concurrent callers. |
| `DeliveryRecovery` | Applies bounded retry rules and reconciles a nonce-bearing bot message before permitting resend. |
| `InboundFetcher` | Reads only enabled inbound aliases from an explicit cursor or time boundary, follows bounded pagination, and emits untrusted envelopes. |
| `InboxRepository` | Preserves the first remote snapshot, current remote state, edit/delete transitions, acknowledgement, archival, and reply linkage. |
| `RetentionManager` | Plans and executes repository-scoped content and metadata expiry or explicit purge without mutating Discord. |

---

## 8. Functional Requirements

### 8.1 Shared Product Requirements

| ID | Priority | Definition |
|---|---|---|
| RC-FR-01 | Must | [RC-FR-01](#rc-fr-01) |
| RC-FR-02 | Must | [RC-FR-02](#rc-fr-02) |
| RC-FR-03 | Must | [RC-FR-03](#rc-fr-03) |
| RC-FR-04 | Must | [RC-FR-04](#rc-fr-04) |
| RC-FR-05 | Must | [RC-FR-05](#rc-fr-05) |
| RC-FR-06 | Must | [RC-FR-06](#rc-fr-06) |

```forge-requirement
{"id":"RC-FR-01","kind":"requirement","text":"Provide one globally installed, versioned repo-com command on PATH for supported Linux, macOS, and Windows systems."}
```

```forge-requirement
{"id":"RC-FR-02","kind":"requirement","text":"Support interactive TTY operation for operator decisions and deterministic non-TTY operation for repository skills without prompting or ambient terminal state."}
```

```forge-requirement
{"id":"RC-FR-03","kind":"requirement","text":"Support one configured Discord workspace per repository and one destination per outbound draft; broadcasts and fan-out require separate drafts."}
```

```forge-requirement
{"id":"RC-FR-04","kind":"requirement","text":"Keep draft, delivery, and inbox domain interfaces provider-neutral while shipping only a Discord adapter in v1."}
```

```forge-requirement
{"id":"RC-FR-05","kind":"requirement","text":"Record every meaningful local lifecycle transition with timestamps and stable draft, revision, delivery, or inbound identifiers so the workflow can be recovered and inspected."}
```

```forge-requirement
{"id":"RC-FR-06","kind":"requirement","text":"Treat a teammate noticing an agent-originated request and replying to it as the primary success outcome; do not claim read receipts or response analytics."}
```

---

## 9. Non-Functional Requirements

| ID | Priority | Definition |
|---|---|---|
| RC-NFR-01 | Must | [RC-NFR-01](#rc-nfr-01) |
| RC-NFR-02 | Must | [RC-NFR-02](#rc-nfr-02) |
| RC-NFR-03 | Should | [RC-NFR-03](#rc-nfr-03) |
| RC-NFR-04 | Must | [RC-NFR-04](#rc-nfr-04) |
| RC-NFR-05 | Must | [RC-NFR-05](#rc-nfr-05) |

```forge-requirement
{"id":"RC-NFR-01","kind":"constraint","text":"A warm local command that performs no network I/O must complete within 500 ms at p95 on the documented CI reference runner."}
```

```forge-requirement
{"id":"RC-NFR-02","kind":"constraint","text":"A crash or process termination must not create a second remote send for the same draft revision, and an ambiguous send must remain blocked until reconciliation resolves it."}
```

```forge-requirement
{"id":"RC-NFR-03","kind":"constraint","text":"Pin the toolchain and lock dependencies; release builds must statically link a patched SQLite version at least 3.53.4 and publish dependency and license evidence."}
```

```forge-requirement
{"id":"RC-NFR-04","kind":"constraint","text":"Pin every Discord request to REST API v10 and obey dynamic route, global, and shared rate-limit responses without hard-coded reset assumptions."}
```

```forge-requirement
{"id":"RC-NFR-05","kind":"constraint","text":"Limit each Discord request to three total transport attempts; retry only outcomes proven not to create a message, and cap any Discord-directed wait at 30 seconds per attempt."}
```

---

## 10. Security and Privacy

| ID | Priority | Definition |
|---|---|---|
| RC-SEC-01 | Must | [RC-SEC-01](#rc-sec-01) |
| RC-SEC-02 | Must | [RC-SEC-02](#rc-sec-02) |
| RC-SEC-03 | Must | [RC-SEC-03](#rc-sec-03) |
| RC-SEC-04 | Must | [RC-SEC-04](#rc-sec-04) |
| RC-SEC-05 | Must | [RC-SEC-05](#rc-sec-05) |
| RC-SEC-06 | Must | [RC-SEC-06](#rc-sec-06) |
| RC-SEC-07 | Must | [RC-SEC-07](#rc-sec-07) |
| RC-SEC-08 | Must | [RC-SEC-08](#rc-sec-08) |
| RC-SEC-09 | Must | [RC-SEC-09](#rc-sec-09) |
| RC-PRIV-01 | Must | [RC-PRIV-01](#rc-priv-01) |
| RC-PRIV-02 | Must | [RC-PRIV-02](#rc-priv-02) |

```forge-requirement
{"id":"RC-SEC-01","kind":"constraint","text":"Accept the Discord bot token only from the REPO_COM_DISCORD_TOKEN environment variable, keep it out of repository configuration and local state, redact it from errors and diagnostics, and zeroize owned copies on drop."}
```

```forge-requirement
{"id":"RC-SEC-02","kind":"constraint","text":"Use only a dedicated Discord bot identity; never accept a user token, self-bot credential, or mechanism that impersonates the operator."}
```

```forge-requirement
{"id":"RC-SEC-03","kind":"constraint","text":"Keep committed TOML configuration limited to non-secret workspace, alias, inbound, retention, and exact auto-send policy data; reject secret-like keys and unknown or unsafe schema versions."}
```

```forge-requirement
{"id":"RC-SEC-04","kind":"constraint","text":"Require a separate interactive operator action to activate an auto-send policy, and invalidate activation when the canonical configuration hash or exact policy tuple changes."}
```

```forge-requirement
{"id":"RC-SEC-05","kind":"constraint","text":"Bind human approval to one immutable draft revision hash containing text, metadata, destination alias, resolved destination, and expiry; any change invalidates approval."}
```

```forge-requirement
{"id":"RC-SEC-06","kind":"constraint","text":"Revalidate destination aliases at approval and immediately before every send attempt; normal skill-facing commands reject raw Discord destinations."}
```

```forge-requirement
{"id":"RC-SEC-07","kind":"constraint","text":"Treat every inbound message, mention, edit, and deletion as untrusted data that cannot grant permission, approve a draft, or trigger a send."}
```

```forge-requirement
{"id":"RC-SEC-08","kind":"constraint","text":"Keep sent messages immutable; corrections use a new draft, and inbound acknowledgement or archival is local-only with no Discord reaction, edit, or delete."}
```

```forge-requirement
{"id":"RC-SEC-09","kind":"constraint","text":"Detect common credential patterns before send, require an explicit exact-revision TTY override for a false positive, prohibit non-TTY override, and audit every override without recording the secret value."}
```

```forge-requirement
{"id":"RC-PRIV-01","kind":"constraint","text":"Store drafts, inbound content, cursors, acknowledgements, and delivery history only in a user-level SQLite database keyed by repository; never upload or synchronize that database."}
```

```forge-requirement
{"id":"RC-PRIV-02","kind":"constraint","text":"Retain message content for 30 days and non-content audit or delivery metadata for one year by default, allow per-repository overrides, provide explicit purge, and send no product telemetry."}
```

---

## 11. Accessibility

| ID | Priority | Definition |
|---|---|---|
| RC-ACC-01 | Must | [RC-ACC-01](#rc-acc-01) |
| RC-ACC-02 | Must | [RC-ACC-02](#rc-acc-02) |
| RC-ACC-03 | Must | [RC-ACC-03](#rc-acc-03) |

```forge-requirement
{"id":"RC-ACC-01","kind":"constraint","text":"Design every interactive command for keyboard operation, linear screen-reader reading, visible focus or selection, and no color-only meaning, following applicable WCAG 2.2 AA principles for terminal software."}
```

```forge-requirement
{"id":"RC-ACC-02","kind":"constraint","text":"Never prompt in non-TTY mode, honor NO_COLOR and non-color output requests, and provide a complete plain-text rendering at an 80-column minimum without truncating security or approval information."}
```

```forge-requirement
{"id":"RC-ACC-03","kind":"constraint","text":"Announce state and errors in text, including destination, revision, policy or approval basis, and next action; ANSI styling may supplement but never replace those labels."}
```

---

## 12. User Interface / Interaction Design

`repo-com` is a terminal interface with two rendering modes:

- Human mode uses short labeled sections and explicit confirmations. Destructive or permission-widening actions always show the exact affected object and require keyboard confirmation.
- JSON mode writes exactly one protocol object to stdout, keeps diagnostics on stderr, and returns stable process exit categories.
- TTY detection is explicit. A command that requires operator confirmation returns a typed approval-required or operator-action-required outcome in non-TTY mode.
- Preview shows resolved destination, exact outbound text, metadata, expiry, approval or policy basis, and any pre-send safety finding without mutating Discord.
- Lists and errors remain understandable when ANSI color is disabled or output is piped.

---

## 13. System States / Lifecycle

```text
Draft: draft -> approved | policy_eligible | blocked | expired
Delivery: unclaimed -> claimed -> accepted | failed | unknown
Unknown: unknown -> reconciled_accepted | reconciled_absent | unresolved
Inbound: unseen -> stored -> acknowledged | archived
Conversation: delivered -> replied
```

Only forward-safe transitions are legal. A sent draft revision is immutable; a content edit creates a new revision. A delivery cannot move from accepted or unknown to a new attempt without the duplicate and reconciliation gates. Acknowledgement and archival do not change remote Discord state.

---

## 14. Features and Delivery Order

| # | Feature | File | Depends On |
|---|---|---|---|
| 1 | CLI Foundation | [cli-foundation.md](features/cli-foundation.md) | None |
| 2 | Repository Configuration and State | [repository-configuration-and-state.md](features/repository-configuration-and-state.md) | CLI Foundation |
| 3 | Draft and Approval Workflow | [draft-and-approval-workflow.md](features/draft-and-approval-workflow.md) | Repository Configuration and State |
| 4 | Discord Delivery and Reconciliation | [discord-delivery-and-reconciliation.md](features/discord-delivery-and-reconciliation.md) | Draft and Approval Workflow |
| 5 | Inbound Retrieval and Reply | [inbound-retrieval-and-reply.md](features/inbound-retrieval-and-reply.md) | Discord Delivery and Reconciliation |
| 6 | Privacy and Lifecycle Operations | [privacy-and-lifecycle-operations.md](features/privacy-and-lifecycle-operations.md) | Inbound Retrieval and Reply |
| 7 | Release Readiness | [release-readiness.md](features/release-readiness.md) | Privacy and Lifecycle Operations |

Each feature document is the sole owner of its detailed requirements and executable `forge-task` contracts. This vision contains no second task catalogue.

---

## 15. Testing Strategy

| Level | Scope | Approach |
|---|---|---|
| Unit and property | Parsing, hashing, state transitions, policy matching, secret detection, retry classification | Focused crate tests with table-driven and boundary cases |
| Storage integration | Migrations, transactions, concurrent claims, corruption handling, retention | Temporary SQLite databases and multi-connection tests |
| CLI integration | JSON envelopes, exit codes, TTY/non-TTY behavior, exact command output | `assert_cmd`, `predicates`, and `cargo-nextest` with fail-on-no-tests filters |
| Discord contract | Authentication, permissions, message creation, rate limits, pagination, ambiguous responses | `wiremock` fixtures pinned to REST v10; no live token in normal CI |
| End-to-end | Create, approve or activate policy, send, fetch, reply, acknowledge, audit | One mocked Discord server and isolated temp state/config |
| Security and privacy | Secret leakage, raw destination rejection, approval invalidation, log redaction, dependency policy | Negative tests, cargo-deny, cargo-audit, and repository secret scanning |
| Cross-platform | Compile and CLI contract tests | Linux, macOS, and Windows CI matrix with platform-specific path tests |
| Human | Terminal UX/accessibility, security/privacy rubric, real Discord round trip, release sign-off | Dependent `human-review` tasks only |

---

## 16. Analytics / Success Metrics

No telemetry is collected. Success is evaluated from local evidence and controlled acceptance reviews.

| Metric | Target | Measurement Method |
|---|---|---|
| Primary workflow | One real teammate reply correlated to the original sent draft | Live Discord human-review evidence |
| Duplicate prevention | Zero duplicate remote sends in 100 concurrent same-revision invocations | Delivery concurrency contract and report |
| Unknown recovery | Zero resends while an outcome remains unresolved or unreconciled | Retry/reconciliation contract |
| Operator safety | Zero secret values in config, state metadata, diagnostics, or audit output | Security/privacy automated checks and human rubric |
| Cross-platform delivery | All supported release targets pass build and CLI contract tests | CI matrix evidence |
| Terminal usability | All required flows completable by keyboard with no color dependency | Human UX/accessibility rubric |

---

## 17. Acceptance Criteria

The project is not approvable until the active task contracts and their named checks pass, followed by the dependent human-review tasks.

1. Every requirement in this vision and every feature document is covered by at least one active task contract.
2. `validate-prd` passes without `--allow-legacy`, with no canonical or DAG errors.
3. The release CLI contract suite covers human and JSON modes, TTY and non-TTY behavior, and stable exit categories.
4. Delivery concurrency and ambiguous-outcome tests demonstrate no unsafe duplicate path.
5. Security/privacy checks demonstrate no token persistence, no telemetry, bounded retention, and redacted diagnostics.
6. Linux, macOS, and Windows release builds and CLI tests pass with a patched SQLite runtime.
7. Human terminal UX/accessibility review, security/privacy review, real Discord round trip, and final release sign-off are recorded in their designated review files.

---

## 18. Dependencies and Risks

### 18.1 Dependencies

| Dependency | Type | Risk if Unavailable | Mitigation |
|---|---|---|---|
| Discord REST API v10 and a dedicated bot | External service | Send/fetch unavailable | Read-only setup diagnostics, typed terminal errors, no alternate provider in v1 |
| Discord administrator | Human/operator | Missing permissions block delivery | Guided non-mutating setup and explicit remediation text |
| Rust toolchain and Cargo registry | Build | No reproducible build | Pinned toolchain, lockfile, cached CI dependencies, checksums |
| SQLite 3.53.4 or newer | Runtime library | State reliability risk | Static release linkage, runtime version assertion, patched-version release gate |
| Supported terminal | Human interface | Output may be unreadable | Plain-text mode, 80-column checks, no-ANSI fallback, human review |

### 18.2 Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Discord accepted a message but the client lost the response | Medium | High duplicate risk | Deterministic nonce footer, atomic claim, explicit unknown state, reconciliation before resend |
| Repository configuration is edited by a skill | Medium | High permission expansion | Local activation bound to canonical config and policy hash; interactive activation only |
| Local state contains sensitive team communication | Medium | High privacy impact | User-only permissions, no sync/telemetry, bounded retention, explicit purge, documented residual risk |
| Secret detector produces false negatives or positives | High | Medium | Lightweight defense only, explicit audited TTY override, never claim complete DLP |
| API or dependency changes | Medium | Medium | Pinned API version, current-version research, advisory and license gates |
| Terminal interaction is unclear | Medium | Medium approval friction | Prototype-driven human UX rubric before release |
| Cross-platform path or packaging differences | Medium | Medium usability | OS-native path abstraction, CI matrix, release smoke tests |

---

## 19. Future Considerations

| Item | Description | Potential Version |
|---|---|---|
| Email adapter | Provider-specific authentication, addressing, threading, and inbound semantics after the shared model is proven | v2 |
| Encrypted local state | Evaluate authenticated encryption and recovery UX if threat modeling requires it | v2 |
| OS keychain integration | Optional token storage without changing environment-based operation | v2 |
| Additional Discord capabilities | Attachments, reactions, embeds, scheduling, or richer templates only after safety review | v2+ |
| Historical backfill | Explicit bounded operation separate from normal cursor fetch | v2 |
| Additional notification providers | Implement the provider-neutral transport interface | v2+ |

---

## 20. Open Questions

| # | Question | Default Assumption |
|---|---|---|
| 1 | Which implementation language and packaging ecosystem should be used? | Use Rust 1.98.1, Cargo workspaces, and cargo-dist 0.32.0. |
| 2 | What command and protocol names should v1 expose? | Use `repo-com` with `config`, `policy`, `draft`, `send`, `inbox`, `reply`, `audit`, `state`, and `purge` groups; stdin/stdout JSON uses protocol version 1. |
| 3 | What exact TOML shape should v1 use? | Use `.repo-com.toml` schema version 1 with repository/workspace identity, destination and inbound aliases, exact auto-send tuples, and retention. |
| 4 | Which Discord permissions are minimally required? | Request View Channel, Send Messages, and Read Message History; mention permissions are validated against the resolved role/user and never mutated automatically. |
| 5 | How should approval and auto-send activation work without a background service? | Human approval is an interactive TTY action; auto-send activation is a separate interactive action bound to a config hash. |
| 6 | How should ambiguous sends be reconciled? | Render a deterministic delivery nonce in channel-ready text, search recent bot messages for exact nonce and content, and keep unresolved outcomes blocked. |
| 7 | Which SQLite linkage should ship? | Statically link SQLite 3.53.4 or newer in release artifacts; the current rusqlite bundled 3.53.2 source is not acceptable. |
| 8 | What terminal interaction is clearest? | Start with labeled, keyboard-operable prompts and plain-text fallback, then require human rubric approval before release. |
| 9 | Does v1 need encrypted local state or an OS keychain? | No; use user-only filesystem permissions, document residual risk, and revisit after threat-model review. |
| 10 | How is real Discord success demonstrated? | A dependent human-review task performs a disposable test workspace round trip and records the result; normal automated tests remain token-free. |

---

## 21. Glossary

| Term | Definition |
|---|---|
| Agent-originated request | A draft created by a repository skill for human attention or a decision |
| Alias | A committed, non-secret name for a configured Discord channel or mention set |
| Canonical hash | A deterministic hash of normalized configuration or exact draft revision content |
| Draft revision | An immutable snapshot of text, metadata, destination, and expiry |
| Eligibility | The decision that a revision may be claimed for delivery because of exact approval or an activated policy |
| Inbound item | A fetched Discord reply or mention treated as untrusted remote data |
| Known outcome | An accepted or definitively failed delivery that cannot be retried as a new send |
| Reconciliation | A read-only search that determines whether an unknown outbound message exists remotely |
| Unknown outcome | An ambiguous delivery result that must not be resent until resolved |
| v1 | The first Discord-only release defined by this vision |
