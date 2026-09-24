# Feature: Discord Delivery and Reconciliation

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** [Draft and Approval Workflow](draft-and-approval-workflow.md)  
**Status:** Canonical v1 plan

This feature provides read-only Discord setup validation, one text-message adapter, and the durable delivery state machine that combines authorization, revision checks, atomic claiming, duplicate protection, bounded retry, and ambiguous-outcome reconciliation.

### In Scope

- Dedicated bot authentication from `REPO_COM_DISCORD_TOKEN`.
- REST API v10 setup, permission, channel, and message-send checks.
- One text-and-allowlisted-mentions message adapter.
- Typed rate-limit and remote error classification.
- Atomic same-revision claim and recorded concurrent outcomes.
- Accepted, failed, and unknown state transitions.
- Conservative reconciliation and permission-reducing retry behavior.

### Out of Scope

- Gateway connections, a daemon, arbitrary history, DMs, attachments, embeds, reactions, or remote edits/deletes.
- User-token authentication or automatic Discord application or permission mutation.
- Automatic resend of an unresolved unknown outcome.
- Read receipts or claims based on delivery state.

---

## 2. Interfaces and Preconditions

### Discord Permission Set

The setup check verifies the bot identity, configured workspace membership, channel visibility, `VIEW_CHANNEL`, `SEND_MESSAGES`, and `READ_MESSAGE_HISTORY`. It reports mention capability separately because it depends on the resolved role or user and guild rules. It never grants permissions.

### Delivery State Machine

```text
unclaimed --atomic claim--> claimed
claimed --accepted response--> accepted
claimed --definitive rejection--> failed
claimed --pre-dispatch failure or 429--> retry_wait --> claimed
claimed --ambiguous response--> unknown
unknown --exact remote match--> accepted
unknown --conservative absence window--> reconciled_absent
unknown --conflict or insufficient evidence--> unresolved
```

Only `unclaimed` may be claimed. Only `retry_wait` may issue the next safe transport attempt. `accepted`, `failed`, `unknown`, and `reconciled_absent` never issue a POST without the explicit eligibility and claim rules that apply to their state.

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| DISC-FR-01 | requirement | Must | DISC-CLIENT-1 |
| DISC-FR-02 | requirement | Must | DISC-CLIENT-1 |
| DISC-FR-03 | requirement | Must | DISC-MSG-1 |
| DISC-FR-04 | requirement | Must | DISC-MSG-1 |
| DISC-FR-05 | requirement | Must | DISC-MSG-1 |
| DEL-FR-01 | requirement | Must | DISC-DELIVERY-1 |
| DEL-FR-02 | requirement | Must | DISC-DELIVERY-1 |
| DEL-FR-03 | requirement | Must | DISC-DELIVERY-1 |
| DEL-FR-04 | requirement | Must | DISC-DELIVERY-2 |
| DEL-FR-05 | requirement | Must | DISC-DELIVERY-2 |
| DEL-FR-06 | requirement | Must | DISC-DELIVERY-2 |
| DEL-FR-07 | requirement | Must | DISC-DELIVERY-1 |
| DISC-CON-01 | constraint | Must | DISC-CLIENT-1, DISC-MSG-1 |
| DISC-CON-02 | constraint | Must | DISC-CLIENT-1 |
| DEL-CON-01 | constraint | Must | DISC-DELIVERY-1 |
| DEL-CON-02 | constraint | Must | DISC-DELIVERY-1, DISC-DELIVERY-2 |
| DEL-CON-03 | constraint | Must | DISC-MSG-1, DISC-DELIVERY-2 |
| DEL-CON-04 | constraint | Must | DISC-DELIVERY-1, DISC-DELIVERY-2 |

```forge-requirement
{"id":"DISC-FR-01","kind":"requirement","text":"Provide a guided, read-only setup check that validates the configured bot identity, workspace membership, destination and inbound channels, required channel permissions, and resolved mention access without creating an application or changing Discord permissions."}
```

```forge-requirement
{"id":"DISC-FR-02","kind":"requirement","text":"Read the bot token only from REPO_COM_DISCORD_TOKEN, send it only in the Discord Bot authorization scheme, redact it from errors and diagnostics, and fail with a rotation instruction when authentication fails."}
```

```forge-requirement
{"id":"DISC-FR-03","kind":"requirement","text":"Create exactly one Discord text message through the versioned channel endpoint with the deterministic draft nonce and an allowed_mentions policy containing only configured role and user IDs."}
```

```forge-requirement
{"id":"DISC-FR-04","kind":"requirement","text":"Apply proactive route and global rate-limit headers and obey HTTP 429 Retry-After or retry_after values for both user and shared scopes without hard-coded bucket timing."}
```

```forge-requirement
{"id":"DISC-FR-05","kind":"requirement","text":"Classify validation, authentication, permission, not-found, conflict, rate-limit, server, pre-dispatch network, and post-dispatch ambiguous outcomes into stable typed errors without logging response bodies that may contain content or credentials."}
```

```forge-requirement
{"id":"DEL-FR-01","kind":"requirement","text":"Claim a send in one local database transaction only when the exact draft revision is unclaimed, and enforce a uniqueness boundary so concurrent or repeated invocations cannot both proceed to network I/O."}
```

```forge-requirement
{"id":"DEL-FR-02","kind":"requirement","text":"Inside the claim transaction, revalidate current repository and config hashes, draft revision hash and expiry, resolved alias and mention allowlist, exact approval or activated policy, and secret-scan decision before recording the claim."}
```

```forge-requirement
{"id":"DEL-FR-03","kind":"requirement","text":"Record every delivery attempt with attempt number, request nonce, start and completion time, redacted error code, Discord message ID when known, and accepted, failed, retry_wait, or unknown state, and append the matching audit event transactionally."}
```

```forge-requirement
{"id":"DEL-FR-04","kind":"requirement","text":"Represent a timeout, reset, or server response after dispatch may have reached Discord as unknown and attach the deterministic nonce and exact intended content needed for read-only reconciliation."}
```

```forge-requirement
{"id":"DEL-FR-05","kind":"requirement","text":"Reconcile an unknown outcome by reading only the configured destination and matching the bot author, deterministic nonce, and exact content; one match becomes accepted, no match remains unknown during the observation window, and multiple or conflicting matches remain unresolved."}
```

```forge-requirement
{"id":"DEL-FR-06","kind":"requirement","text":"Retry at most three total transport attempts and only for a proven pre-dispatch failure or HTTP 429; use bounded jitter for pre-dispatch failures and the server-directed delay capped at 30 seconds for 429, while every post-dispatch ambiguity becomes unknown."}
```

```forge-requirement
{"id":"DEL-FR-07","kind":"requirement","text":"Return the existing accepted, failed, retry_wait, unknown, or reconciled outcome to duplicate and concurrent callers for the same draft revision rather than creating another attempt."}
```

```forge-requirement
{"id":"DISC-CON-01","kind":"constraint","text":"Pin every Discord route to REST API v10, use a dedicated bot token only, and reject user tokens, Bearer user authentication, self-bots, and unpinned API versions."}
```

```forge-requirement
{"id":"DISC-CON-02","kind":"constraint","text":"Setup and validation are read-only and must not create applications, join additional workspaces, change roles, overwrite channels, or mutate permissions."}
```

```forge-requirement
{"id":"DEL-CON-01","kind":"constraint","text":"Authorization, current revision and destination revalidation, duplicate claim, attempt creation, and audit state must commit atomically before any network call; no intermediate state may authorize a second send."}
```

```forge-requirement
{"id":"DEL-CON-02","kind":"constraint","text":"An unknown or unresolved delivery must never be automatically resent; only exact reconciliation or a later explicit operator-authorized new attempt may change that state."}
```

```forge-requirement
{"id":"DEL-CON-03","kind":"constraint","text":"Do not hard-code Discord rate limits, retry all errors, or exceed three total transport attempts; each request must have bounded connect, total, and response time."}
```

```forge-requirement
{"id":"DEL-CON-04","kind":"constraint","text":"Outbound messages are immutable in repo-com; expose no Discord edit or delete API and record reconciliation without mutating the remote message."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| DISC-CLIENT-1 | Read-only bot setup validation | discord-engineer | Foundation and config | `repo-com-discord-client`, `discord_client_contract` | DISC-FR-01/02 and Discord constraints | Sending, delivery state |
| DISC-MSG-1 | One typed REST message operation and error/rate classification | discord-engineer | Client and rendered draft | `repo-com-discord-message`, `discord_message_contract` | DISC-FR-03 through 05, retry constraint | Claiming, retries across attempts |
| DISC-DELIVERY-1 | Atomic duplicate-safe delivery state machine | delivery-engineer | Eligibility, state, audit, message | `repo-com-delivery`, `delivery_contract` | DEL-FR-01 through 03 and 07, atomic constraints | Retry timing and reconciliation |
| DISC-DELIVERY-2 | Bounded safe retry and unknown reconciliation | delivery-engineer | Delivery state and read client | `repo-com-delivery-retry`, `delivery_retry_contract` | DEL-FR-04 through 06 and delivery constraints | New draft content, inbound replies |

---

## Phase 1: Discord Adapter and Delivery Safety

```forge-task
{
  "id": "DISC-CLIENT-1",
  "title": "Implement read-only Discord setup validation",
  "description": "Implement the focused repo-com-discord-client crate for dedicated bot authentication from REPO_COM_DISCORD_TOKEN and read-only REST v10 setup diagnostics. Validate bot identity, configured workspace membership, destination and inbound channel visibility, VIEW_CHANNEL, SEND_MESSAGES, READ_MESSAGE_HISTORY, and resolved mention access; return structured remediation without changing Discord. Redact tokens and response bodies, reject user-token schemes, and do not implement message creation, Gateway, or application or permission mutation.",
  "ownerAgent": "discord-engineer",
  "dependencies": ["PLAT-1", "REPO-CFG-1"],
  "expectedOutputs": [
    "crates/repo-com-discord-client/Cargo.toml",
    "crates/repo-com-discord-client/src/lib.rs",
    "crates/repo-com-discord-client/src/auth.rs",
    "crates/repo-com-discord-client/src/client.rs",
    "crates/repo-com-discord-client/src/setup.rs",
    "crates/repo-com-discord-client/tests/discord_client_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(discord_client_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DISC-FR-01",
      "docs/features/discord-delivery-and-reconciliation.md#DISC-FR-02"
    ],
    "acceptanceCriteria": [
      "Client tests use wiremock to verify every request path is under /api/v10 and uses the Bot authorization scheme",
      "Setup tests cover bot identity, missing guild, missing channel, each missing required permission, mention denial, and complete success",
      "Authentication failure returns a token-rotation remediation and no token or raw response body in Debug, Display, or diagnostics",
      "Request logs prove setup issues only GET or HEAD requests and no application, role, channel, or permission mutation endpoint"
    ],
    "constraints": [
      "Use a dedicated bot identity and read-only setup operations"
    ],
    "constraintRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DISC-CON-01",
      "docs/features/discord-delivery-and-reconciliation.md#DISC-CON-02",
      "docs/PRD.md#RC-SEC-01",
      "docs/PRD.md#RC-SEC-02"
    ],
    "references": [
      "docs/PRD.md#5. Research Findings",
      "docs/features/discord-delivery-and-reconciliation.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DISC-MSG-1",
  "title": "Implement one Discord message operation",
  "description": "Implement the focused repo-com-discord-message crate for exactly one create-message request using rendered draft text, deterministic nonce, an allowlisted allowed_mentions policy, and an optional already-validated message_reference. Pin REST v10, enforce request limits, parse dynamic rate-limit headers and 429 Retry-After, and classify validation, authentication, permission, not-found, conflict, rate-limit, server, pre-dispatch, and ambiguous outcomes without logging content or credentials. Do not claim delivery, retry across attempts, reconcile, or expose rich-message behavior.",
  "ownerAgent": "discord-engineer",
  "dependencies": ["DISC-CLIENT-1", "DRAFT-CONTENT-1"],
  "expectedOutputs": [
    "crates/repo-com-discord-message/Cargo.toml",
    "crates/repo-com-discord-message/src/lib.rs",
    "crates/repo-com-discord-message/src/request.rs",
    "crates/repo-com-discord-message/src/rate_limit.rs",
    "crates/repo-com-discord-message/src/error.rs",
    "crates/repo-com-discord-message/tests/discord_message_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(discord_message_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DISC-FR-03",
      "docs/features/discord-delivery-and-reconciliation.md#DISC-FR-04",
      "docs/features/discord-delivery-and-reconciliation.md#DISC-FR-05"
    ],
    "acceptanceCriteria": [
      "Message tests assert one POST to /api/v10/channels/{channel_id}/messages with exact text, nonce footer, only allowlisted role and user IDs, and a message_reference only for a validated reply draft",
      "Request tests reject attachments, embeds, raw channel IDs, unlisted mentions, oversized content, and any API version other than v10",
      "Rate-limit tests cover route and global headers plus user and shared 429 scopes and prove the client uses server-provided reset timing",
      "Error tests cover 400, 401, 403, 404, 409, 429, 5xx, connect failure, pre-dispatch timeout, and post-dispatch timeout without leaking body content or token",
      "Request and error types distinguish proven-not-sent from ambiguous outcomes for the delivery recovery layer"
    ],
    "constraints": [
      "A message operation performs at most one HTTP send attempt"
    ],
    "constraintRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DISC-CON-01",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-CON-03",
      "docs/PRD.md#RC-NFR-04",
      "docs/PRD.md#RC-NFR-05"
    ],
    "references": [
      "docs/PRD.md#5. Research Findings",
      "docs/features/discord-delivery-and-reconciliation.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DISC-DELIVERY-1",
  "title": "Implement atomic duplicate-safe delivery claims",
  "description": "Implement the focused repo-com-delivery crate as the durable state machine around one draft revision. In a single SQLite transaction, reload current config, revision, expiry, resolved destination, exact approval or activated policy, and secret decision; create one uniquely constrained attempt and audit event; then commit before network I/O. Concurrent and repeated callers receive the recorded state without a second POST. Record accepted message ID, definitive failure, retry_wait, or unknown and audit each transition. Do not implement retry timing, reconciliation, or a bypass around the claim.",
  "ownerAgent": "delivery-engineer",
  "dependencies": ["DISC-MSG-1", "DRAFT-ELIG-1", "REPO-STATE-1", "REPO-AUDIT-1"],
  "expectedOutputs": [
    "crates/repo-com-delivery/Cargo.toml",
    "crates/repo-com-delivery/src/lib.rs",
    "crates/repo-com-delivery/src/model.rs",
    "crates/repo-com-delivery/src/claim.rs",
    "crates/repo-com-delivery/src/transition.rs",
    "crates/repo-com-delivery/tests/delivery_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(delivery_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-01",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-02",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-03",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-07"
    ],
    "acceptanceCriteria": [
      "Claim tests prove stale config, revision, expiry, destination, approval, activation, or scan input rolls back with no attempt or network authorization",
      "A 100-way concurrent test for one revision proves exactly one claim commits and all callers observe the same recorded outcome",
      "Transition tests cover accepted, definitive failed, retry_wait, and unknown with required attempt metadata and transactional audit events",
      "Crash-boundary tests stop after claim and after remote response but before local completion and prove the next caller does not POST again",
      "State tests reject outbound edit, delete, second accepted message, and transition from a terminal state to a new claim"
    ],
    "constraints": [
      "Keep claim, authorization, duplicate protection, attempt creation, and audit atomic"
    ],
    "constraintRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DEL-CON-01",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-CON-04",
      "docs/PRD.md#RC-NFR-02",
      "docs/PRD.md#RC-SEC-05",
      "docs/PRD.md#RC-SEC-06"
    ],
    "references": [
      "docs/PRD.md#RC-US-03",
      "docs/PRD.md#13. System States / Lifecycle",
      "docs/features/discord-delivery-and-reconciliation.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DISC-DELIVERY-2",
  "title": "Implement bounded retry and unknown reconciliation",
  "description": "Implement the focused repo-com-delivery-retry crate on top of the delivery state machine. Permit at most three total transport attempts and retry only proven pre-dispatch failures or HTTP 429, using bounded jitter or the server delay capped at 30 seconds. Classify any post-dispatch timeout, reset, or 5xx as unknown. Reconcile unknown by reading only the configured destination and matching bot author, deterministic nonce, and exact content; require a five-minute observation window and three successful reads before reconciled_absent, while conflicts remain unresolved. Do not automatically resend unknown or expose a remote mutation.",
  "ownerAgent": "delivery-engineer",
  "dependencies": ["DISC-DELIVERY-1", "DISC-CLIENT-1"],
  "expectedOutputs": [
    "crates/repo-com-delivery-retry/Cargo.toml",
    "crates/repo-com-delivery-retry/src/lib.rs",
    "crates/repo-com-delivery-retry/src/policy.rs",
    "crates/repo-com-delivery-retry/src/reconcile.rs",
    "crates/repo-com-delivery-retry/tests/delivery_retry_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(delivery_retry_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-04",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-05",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-FR-06"
    ],
    "acceptanceCriteria": [
      "Retry tests prove no fourth attempt and no retry for 400, 401, 403, 404, 409, or ambiguous post-dispatch outcomes",
      "Clock-controlled tests prove pre-dispatch jitter bounds, 429 server-directed delay, and the 30-second per-attempt cap without wall-clock sleeps",
      "Unknown tests prove timeout, connection reset after dispatch, and 5xx responses enter unknown and never automatic resend",
      "Reconciliation tests match only the configured destination, bot author, exact nonce, and exact content and distinguish one match, no match, multiple matches, edited match, and deleted match",
      "Absence tests require five minutes and three successful reads before reconciled_absent; conflicts and insufficient evidence remain unresolved",
      "Recovery tests prove no reconciliation call mutates Discord"
    ],
    "constraints": [
      "Unknown and unresolved outcomes are permission-blocking, not retry suggestions"
    ],
    "constraintRefs": [
      "docs/features/discord-delivery-and-reconciliation.md#DEL-CON-02",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-CON-03",
      "docs/features/discord-delivery-and-reconciliation.md#DEL-CON-04",
      "docs/PRD.md#RC-NFR-02",
      "docs/PRD.md#RC-NFR-04",
      "docs/PRD.md#RC-NFR-05"
    ],
    "references": [
      "docs/PRD.md#13. System States / Lifecycle",
      "docs/features/discord-delivery-and-reconciliation.md#2. Interfaces and Preconditions"
    ]
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Tasks |
|---|---|---|
| [DISC-FR-01](discord-delivery-and-reconciliation.md#DISC-FR-01) | requirement | DISC-CLIENT-1 |
| [DISC-FR-02](discord-delivery-and-reconciliation.md#DISC-FR-02) | requirement | DISC-CLIENT-1 |
| [DISC-FR-03](discord-delivery-and-reconciliation.md#DISC-FR-03) | requirement | DISC-MSG-1 |
| [DISC-FR-04](discord-delivery-and-reconciliation.md#DISC-FR-04) | requirement | DISC-MSG-1 |
| [DISC-FR-05](discord-delivery-and-reconciliation.md#DISC-FR-05) | requirement | DISC-MSG-1 |
| [DEL-FR-01](discord-delivery-and-reconciliation.md#DEL-FR-01) | requirement | DISC-DELIVERY-1 |
| [DEL-FR-02](discord-delivery-and-reconciliation.md#DEL-FR-02) | requirement | DISC-DELIVERY-1 |
| [DEL-FR-03](discord-delivery-and-reconciliation.md#DEL-FR-03) | requirement | DISC-DELIVERY-1 |
| [DEL-FR-04](discord-delivery-and-reconciliation.md#DEL-FR-04) | requirement | DISC-DELIVERY-2 |
| [DEL-FR-05](discord-delivery-and-reconciliation.md#DEL-FR-05) | requirement | DISC-DELIVERY-2 |
| [DEL-FR-06](discord-delivery-and-reconciliation.md#DEL-FR-06) | requirement | DISC-DELIVERY-2 |
| [DEL-FR-07](discord-delivery-and-reconciliation.md#DEL-FR-07) | requirement | DISC-DELIVERY-1 |
| [DISC-CON-01](discord-delivery-and-reconciliation.md#DISC-CON-01) | constraint | DISC-CLIENT-1, DISC-MSG-1 |
| [DISC-CON-02](discord-delivery-and-reconciliation.md#DISC-CON-02) | constraint | DISC-CLIENT-1 |
| [DEL-CON-01](discord-delivery-and-reconciliation.md#DEL-CON-01) | constraint | DISC-DELIVERY-1 |
| [DEL-CON-02](discord-delivery-and-reconciliation.md#DEL-CON-02) | constraint | DISC-DELIVERY-1, DISC-DELIVERY-2 |
| [DEL-CON-03](discord-delivery-and-reconciliation.md#DEL-CON-03) | constraint | DISC-MSG-1, DISC-DELIVERY-2 |
| [DEL-CON-04](discord-delivery-and-reconciliation.md#DEL-CON-04) | constraint | DISC-DELIVERY-1, DISC-DELIVERY-2 |
