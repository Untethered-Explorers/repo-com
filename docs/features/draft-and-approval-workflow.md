# Feature: Draft and Approval Workflow

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** [Repository Configuration and State](repository-configuration-and-state.md)  
**Status:** Canonical v1 plan

This feature turns structured skill input into one immutable, channel-ready draft revision; presents an exact preview; binds human approval or an activated policy to that revision; and blocks unsafe or stale content before delivery.

### In Scope

- One-destination drafts with text and bounded metadata.
- Immutable revision history and deterministic content hashes.
- Human and machine preview data.
- Channel text and allowlisted mention rendering.
- Lightweight pre-send credential detection and audited TTY override.
- Exact-revision human approval and delivery eligibility.

### Out of Scope

- Discord HTTP requests, delivery claims, retries, or reconciliation.
- Broadcasts, arbitrary destinations, direct messages, attachments, embeds, templates, or generated copy.
- Allowing a non-TTY skill invocation to approve, activate policy, or override a safety finding.

---

## 2. Interfaces and Preconditions

| Interface | Input | Output | Owner Task |
|---|---|---|---|
| `DraftModel` | Validated draft request | Draft plus immutable revision | DRAFT-MODEL-1 |
| `DraftPreview` | Draft revision, config, policy state | Exact rendered text, metadata, expiry, and decision basis | DRAFT-MODEL-1, DRAFT-CONTENT-1 |
| `ContentRenderer` | Revision, resolved destination | Discord text and parsed mention tokens | DRAFT-CONTENT-1 |
| `SecretScanner` | Rendered text and metadata | Findings with stable reason codes | DRAFT-SECRET-1 |
| `ApprovalService` | TTY confirmation and revision hash | Time-bounded exact-revision approval | DRAFT-APPROVAL-1 |
| `EligibilityEvaluator` | Current config, revision, approval, policy, scan | Eligible or typed blocked decision with revalidation inputs | DRAFT-ELIG-1 |

A revision hash is SHA-256 over canonical JSON containing the body, normalized metadata, destination alias, resolved channel and mentions, expiry, and repository ID. Hashing alone does not authorize sending; the delivery coordinator must revalidate all inputs atomically before its claim.

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| DRAFT-FR-01 | requirement | Must | DRAFT-MODEL-1 |
| DRAFT-FR-02 | requirement | Must | DRAFT-MODEL-1 |
| DRAFT-FR-03 | requirement | Must | DRAFT-MODEL-1, DRAFT-CONTENT-1 |
| DRAFT-FR-04 | requirement | Must | DRAFT-CONTENT-1 |
| DRAFT-FR-05 | requirement | Must | DRAFT-SECRET-1, DRAFT-APPROVAL-1 |
| APPROVAL-FR-01 | requirement | Must | DRAFT-APPROVAL-1 |
| APPROVAL-FR-02 | requirement | Must | DRAFT-APPROVAL-1, DRAFT-ELIG-1 |
| APPROVAL-FR-03 | requirement | Must | DRAFT-APPROVAL-1, DRAFT-ELIG-1 |
| ELIG-FR-01 | requirement | Must | DRAFT-ELIG-1 |
| ELIG-FR-02 | requirement | Must | DRAFT-ELIG-1 |
| ELIG-FR-03 | requirement | Must | DRAFT-ELIG-1 |
| DRAFT-CON-01 | constraint | Must | DRAFT-MODEL-1, DRAFT-APPROVAL-1 |
| DRAFT-CON-02 | constraint | Must | DRAFT-MODEL-1, DRAFT-CONTENT-1 |
| CONTENT-CON-01 | constraint | Must | DRAFT-CONTENT-1 |
| SAFETY-CON-01 | constraint | Must | DRAFT-SECRET-1, DRAFT-APPROVAL-1 |
| APPROVAL-CON-01 | constraint | Must | DRAFT-APPROVAL-1, DRAFT-ELIG-1 |

```forge-requirement
{"id":"DRAFT-FR-01","kind":"requirement","text":"Create a draft with one configured destination alias, non-empty channel text, event type, severity, optional repository label, branch, and commit metadata, and an optional inbound-item reply reference; reject broadcasts, multiple destinations, unvalidated reply references, and unknown metadata."}
```

```forge-requirement
{"id":"DRAFT-FR-02","kind":"requirement","text":"Store every draft body, metadata set, destination alias, resolved destination, and expiry as an immutable revision with a monotonically increasing revision number and deterministic content hash."}
```

```forge-requirement
{"id":"DRAFT-FR-03","kind":"requirement","text":"Preview one revision with the resolved alias and Discord identifiers, exact final text, metadata, expiry, current approval or policy basis, secret-scan status, and a typed send decision without mutating the draft or Discord."}
```

```forge-requirement
{"id":"DRAFT-FR-04","kind":"requirement","text":"Create one Discord text message with a deterministic delivery-nonce footer, only named mentions resolved from the destination allowlist, and a validated message_reference when the draft is a reply; reject any rendered message over Discord's 2,000-character limit."}
```

```forge-requirement
{"id":"DRAFT-FR-05","kind":"requirement","text":"Scan the final rendered text and metadata for high-confidence Discord tokens, authorization values, private-key markers, and common credential URL or assignment patterns before a send can become eligible."}
```

```forge-requirement
{"id":"APPROVAL-FR-01","kind":"requirement","text":"Allow an operator to approve only after seeing the complete preview in an interactive TTY, and bind the recorded approval to the exact current revision hash and repository ID."}
```

```forge-requirement
{"id":"APPROVAL-FR-02","kind":"requirement","text":"Expire approval at the earlier of draft expiry or 15 minutes after approval, and invalidate it immediately when the revision hash, repository config hash, destination resolution, or policy basis changes."}
```

```forge-requirement
{"id":"APPROVAL-FR-03","kind":"requirement","text":"Keep an accepted remote message immutable; represent a correction as a new draft or a validated threaded reply and expose no edit or delete operation for outbound content."}
```

```forge-requirement
{"id":"ELIG-FR-01","kind":"requirement","text":"Mark a revision eligible only when it has unexpired exact-revision human approval or an unexpired exact activated policy and has no unresolved secret-scan finding."}
```

```forge-requirement
{"id":"ELIG-FR-02","kind":"requirement","text":"Revalidate current config hash, repository ID, expiry, revision hash, destination resolution, approval or activation, and secret scan at eligibility evaluation and again inside the delivery claim immediately before network I/O."}
```

```forge-requirement
{"id":"ELIG-FR-03","kind":"requirement","text":"Permit a non-TTY send invocation only when an existing exact human approval or activated policy already satisfies ELIG-FR-01; a non-TTY invocation may not create approval, activate policy, or override a finding."}
```

```forge-requirement
{"id":"DRAFT-CON-01","kind":"constraint","text":"Default draft expiry is 24 hours, callers may choose at most 7 days, and expired revisions cannot be approved, made eligible, or sent."}
```

```forge-requirement
{"id":"DRAFT-CON-02","kind":"constraint","text":"A revision may contain text and bounded metadata only; never attach repository code, files, diffs, embeds, attachments, or automatically generated message copy."}
```

```forge-requirement
{"id":"CONTENT-CON-01","kind":"constraint","text":"Normal draft input accepts aliases rather than raw Discord channel, role, or user identifiers and every mention emitted by the renderer must belong to that destination allowlist."}
```

```forge-requirement
{"id":"SAFETY-CON-01","kind":"constraint","text":"A secret-scan finding blocks send by default; only an interactive TTY operator may override the exact revision after reviewing the preview, the override must be audited without the matched value, and non-TTY override is forbidden."}
```

```forge-requirement
{"id":"APPROVAL-CON-01","kind":"constraint","text":"Approval is permission to evaluate and claim one exact revision, not a reusable bypass; the delivery coordinator must consume it idempotently and a concurrent or repeated call must not create a second remote send."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| DRAFT-MODEL-1 | Immutable draft revision and preview model | messaging-engineer | Config and state interfaces | `repo-com-draft-model`, `draft_model_contract` | DRAFT-FR-01 through DRAFT-FR-03, expiry/content constraints | Rendering, approval, Discord |
| DRAFT-CONTENT-1 | Deterministic allowlisted Discord text | messaging-engineer | Draft model | `repo-com-draft-content`, `draft_content_contract` | DRAFT-FR-03, DRAFT-FR-04, content constraints | Secret detection, delivery |
| DRAFT-SECRET-1 | Stable pre-send credential findings | security-engineer | Rendered draft interface | `repo-com-draft-safety`, `draft_safety_contract` | DRAFT-FR-05, SAFETY-CON-01 | Override, approval |
| DRAFT-APPROVAL-1 | TTY exact-revision approval lifecycle | approval-engineer | Model, policy, state, safety | `repo-com-approval`, `approval_contract` | DRAFT-FR-05, APPROVAL-FR-01 through 03, safety/approval constraints | Discord send |
| DRAFT-ELIG-1 | Fail-closed current-state send eligibility | approval-engineer | Approval and policy interfaces | `repo-com-send-eligibility`, `send_eligibility_contract` | APPROVAL-FR-02/03 and ELIG-FR-01 through 03 | Network send and retries |

---

## Phase 1: Draft, Safety, and Approval Boundaries

```forge-task
{
  "id": "DRAFT-MODEL-1",
  "title": "Model immutable draft revisions and previews",
  "description": "Implement the focused repo-com-draft-model crate for validated one-destination draft creation, immutable revision storage, canonical SHA-256 revision hashes, expiry, and side-effect-free preview data. Enforce exact text, event-type, severity, optional repository metadata, and an optional already-validated inbound reply reference; reject broadcasts, unvalidated or arbitrary reply IDs, unknown fields, and edits to an existing revision. Resolve aliases through the current config interface and expose the approval, policy, and safety bases as unresolved preview facts. Do not render Discord text, approve, scan secrets, or call Discord.",
  "ownerAgent": "messaging-engineer",
  "dependencies": ["REPO-CFG-1", "REPO-STATE-1"],
  "expectedOutputs": [
    "crates/repo-com-draft-model/Cargo.toml",
    "crates/repo-com-draft-model/src/lib.rs",
    "crates/repo-com-draft-model/src/model.rs",
    "crates/repo-com-draft-model/src/canonical.rs",
    "crates/repo-com-draft-model/src/preview.rs",
    "crates/repo-com-draft-model/tests/draft_model_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(draft_model_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-01",
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-02",
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-03"
    ],
    "acceptanceCriteria": [
      "Model tests cover valid draft boundaries, an authorized opaque inbound reply reference, arbitrary or unvalidated reply IDs, unknown metadata, multiple destinations, empty text, invalid event type, invalid severity, and expiry above seven days",
      "Revision tests prove any body, metadata, alias, resolved-destination, repository, or expiry change creates a new revision and never mutates the prior snapshot",
      "Canonical hash tests are deterministic across metadata ordering and change when any approval-bound field changes",
      "Preview tests prove exact current fields and unresolved approval, policy, and safety bases are returned without a state or network mutation"
    ],
    "constraints": [
      "Draft revisions contain no file or code attachment"
    ],
    "constraintRefs": [
      "docs/features/draft-and-approval-workflow.md#DRAFT-CON-01",
      "docs/features/draft-and-approval-workflow.md#DRAFT-CON-02",
      "docs/PRD.md#RC-SEC-06"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/features/draft-and-approval-workflow.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DRAFT-CONTENT-1",
  "title": "Render deterministic channel-ready text",
  "description": "Implement the focused repo-com-draft-content crate to render one immutable draft revision as Discord text, append a deterministic delivery-nonce footer, resolve only configured mention aliases, and carry an optional already-validated message_reference as request metadata rather than body text. Normalize line endings and Unicode without changing semantic content, reject unsupported or raw mention forms, and enforce the final 2,000-character Discord limit. Do not add attachments, embeds, generated copy, secret scanning, or network I/O.",
  "ownerAgent": "messaging-engineer",
  "dependencies": ["DRAFT-MODEL-1"],
  "expectedOutputs": [
    "crates/repo-com-draft-content/Cargo.toml",
    "crates/repo-com-draft-content/src/lib.rs",
    "crates/repo-com-draft-content/src/normalize.rs",
    "crates/repo-com-draft-content/src/mentions.rs",
    "crates/repo-com-draft-content/src/render.rs",
    "crates/repo-com-draft-content/tests/draft_content_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(draft_content_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-03",
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-04"
    ],
    "acceptanceCriteria": [
      "Renderer tests prove identical revision input yields byte-identical text and nonce footer on repeated runs",
      "Mention tests prove configured named role and user aliases resolve and every unlisted, raw, malformed, or user-bot target is rejected",
      "Normalization tests cover CRLF, Unicode, trailing whitespace, and already-rendered nonce protection without silent semantic expansion",
      "Boundary tests reject a final Discord message over 2,000 characters and the 2,000-character maximum succeeds"
    ],
    "constraints": [
      "Rendering supports one message and text plus allowlisted mentions only"
    ],
    "constraintRefs": [
      "docs/features/draft-and-approval-workflow.md#DRAFT-CON-02",
      "docs/features/draft-and-approval-workflow.md#CONTENT-CON-01",
      "docs/PRD.md#RC-SEC-08"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/features/draft-and-approval-workflow.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DRAFT-SECRET-1",
  "title": "Detect pre-send credential patterns",
  "description": "Implement the focused repo-com-draft-safety crate to scan final rendered text and metadata for high-confidence Discord tokens, authorization values, private-key markers, credential-bearing URLs, and common secret assignments. Return stable reason codes and redacted match locations without returning the matched secret, keep the scanner side-effect free, and expose an explicit result object for TTY override evaluation. Do not claim complete data-loss prevention, persist findings, or perform an override.",
  "ownerAgent": "security-engineer",
  "dependencies": ["DRAFT-CONTENT-1"],
  "expectedOutputs": [
    "crates/repo-com-draft-safety/Cargo.toml",
    "crates/repo-com-draft-safety/src/lib.rs",
    "crates/repo-com-draft-safety/src/patterns.rs",
    "crates/repo-com-draft-safety/src/scanner.rs",
    "crates/repo-com-draft-safety/tests/draft_safety_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(draft_safety_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-05"
    ],
    "acceptanceCriteria": [
      "Scanner tests cover Discord bot token, Authorization header, private-key marker, credential URL, and secret-assignment fixtures with stable reason codes",
      "Finding output contains only redacted location and reason metadata, never the matched value",
      "Clean repository text, branch names, commit hashes, and ordinary URLs do not trigger the high-confidence rules",
      "Scanner tests prove repeated scans of the same input are deterministic and perform no persistence or network action"
    ],
    "constraints": [
      "Treat secret detection as defense in depth, not complete DLP"
    ],
    "constraintRefs": [
      "docs/features/draft-and-approval-workflow.md#SAFETY-CON-01",
      "docs/PRD.md#RC-SEC-09"
    ],
    "references": [
      "docs/PRD.md#5. Research Findings",
      "docs/features/draft-and-approval-workflow.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DRAFT-APPROVAL-1",
  "title": "Bind human approval to one draft revision",
  "description": "Implement the focused repo-com-approval crate for TTY-only human approval, exact revision and config binding, time-bounded validity, secret-finding override, and idempotent audit events. Approval expires at the earlier of draft expiry or 15 minutes and becomes invalid on any revision, repository, config, or destination change. A TTY override is allowed only for the reviewed exact revision; non-TTY approval and override must fail closed. Do not send Discord requests or treat approval as a reusable send bypass.",
  "ownerAgent": "approval-engineer",
  "dependencies": ["DRAFT-MODEL-1", "DRAFT-SECRET-1", "REPO-POLICY-1", "REPO-STATE-1"],
  "expectedOutputs": [
    "crates/repo-com-approval/Cargo.toml",
    "crates/repo-com-approval/src/lib.rs",
    "crates/repo-com-approval/src/service.rs",
    "crates/repo-com-approval/src/override.rs",
    "crates/repo-com-approval/tests/approval_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(approval_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/draft-and-approval-workflow.md#DRAFT-FR-05",
      "docs/features/draft-and-approval-workflow.md#APPROVAL-FR-01",
      "docs/features/draft-and-approval-workflow.md#APPROVAL-FR-02",
      "docs/features/draft-and-approval-workflow.md#APPROVAL-FR-03"
    ],
    "acceptanceCriteria": [
      "Approval tests prove TTY confirmation records the exact repository, revision, config, and destination hashes and is idempotent for the same confirmation",
      "Invalidation tests change body, metadata, alias, resolved destination, repository config, and expiry and prove approval becomes invalid",
      "Time tests use an injected clock to cover the 15-minute approval limit, draft expiry, and earlier-expiry precedence",
      "Override tests require TTY and exact current revision, record only a redacted reason code, and reject non-TTY override",
      "No approval API exposes outbound edit or delete"
    ],
    "constraints": [
      "Approval authorizes one exact revision and never bypasses final delivery revalidation"
    ],
    "constraintRefs": [
      "docs/features/draft-and-approval-workflow.md#SAFETY-CON-01",
      "docs/features/draft-and-approval-workflow.md#APPROVAL-CON-01",
      "docs/features/draft-and-approval-workflow.md#DRAFT-CON-01",
      "docs/PRD.md#RC-SEC-05",
      "docs/PRD.md#RC-SEC-08"
    ],
    "references": [
      "docs/PRD.md#12. User Interface / Interaction Design",
      "docs/features/draft-and-approval-workflow.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "DRAFT-ELIG-1",
  "title": "Evaluate current send eligibility",
  "description": "Implement the focused repo-com-send-eligibility crate as a pure, fail-closed decision over current repository config, immutable draft revision, resolved destination, expiry, exact human approval, activated policy, and secret-scan result. Return a typed decision containing the basis and all hashes the delivery coordinator must revalidate atomically. Permit non-TTY send only for a pre-existing exact approval or activated policy; do not create approval, activate policy, override findings, claim delivery, or call Discord.",
  "ownerAgent": "approval-engineer",
  "dependencies": ["DRAFT-APPROVAL-1", "DRAFT-CONTENT-1", "DRAFT-SECRET-1", "REPO-POLICY-1"],
  "expectedOutputs": [
    "crates/repo-com-send-eligibility/Cargo.toml",
    "crates/repo-com-send-eligibility/src/lib.rs",
    "crates/repo-com-send-eligibility/src/decision.rs",
    "crates/repo-com-send-eligibility/src/evaluator.rs",
    "crates/repo-com-send-eligibility/tests/send_eligibility_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(send_eligibility_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/draft-and-approval-workflow.md#APPROVAL-FR-02",
      "docs/features/draft-and-approval-workflow.md#APPROVAL-FR-03",
      "docs/features/draft-and-approval-workflow.md#ELIG-FR-01",
      "docs/features/draft-and-approval-workflow.md#ELIG-FR-02",
      "docs/features/draft-and-approval-workflow.md#ELIG-FR-03"
    ],
    "acceptanceCriteria": [
      "Eligibility tests cover valid exact approval, valid exact activated policy, both absent, expired approval, stale activation, expired draft, changed config, changed destination, and unresolved scan finding",
      "Decision tests prove every eligible result carries repository, revision, config, destination, approval or policy, and scan hashes for final revalidation",
      "Non-TTY tests prove an existing valid approval or policy can be evaluated but no approval, activation, or override is created",
      "Decision tests prove an outbound correction requires a new draft or reply and offers no edit or delete path"
    ],
    "constraints": [
      "Eligibility is a decision, not a durable send claim"
    ],
    "constraintRefs": [
      "docs/features/draft-and-approval-workflow.md#APPROVAL-CON-01",
      "docs/PRD.md#RC-SEC-04",
      "docs/PRD.md#RC-SEC-05",
      "docs/PRD.md#RC-SEC-06"
    ],
    "references": [
      "docs/PRD.md#13. System States / Lifecycle",
      "docs/features/draft-and-approval-workflow.md#2. Interfaces and Preconditions"
    ]
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Tasks |
|---|---|---|
| [DRAFT-FR-01](draft-and-approval-workflow.md#DRAFT-FR-01) | requirement | DRAFT-MODEL-1 |
| [DRAFT-FR-02](draft-and-approval-workflow.md#DRAFT-FR-02) | requirement | DRAFT-MODEL-1 |
| [DRAFT-FR-03](draft-and-approval-workflow.md#DRAFT-FR-03) | requirement | DRAFT-MODEL-1, DRAFT-CONTENT-1 |
| [DRAFT-FR-04](draft-and-approval-workflow.md#DRAFT-FR-04) | requirement | DRAFT-CONTENT-1 |
| [DRAFT-FR-05](draft-and-approval-workflow.md#DRAFT-FR-05) | requirement | DRAFT-SECRET-1, DRAFT-APPROVAL-1 |
| [APPROVAL-FR-01](draft-and-approval-workflow.md#APPROVAL-FR-01) | requirement | DRAFT-APPROVAL-1 |
| [APPROVAL-FR-02](draft-and-approval-workflow.md#APPROVAL-FR-02) | requirement | DRAFT-APPROVAL-1, DRAFT-ELIG-1 |
| [APPROVAL-FR-03](draft-and-approval-workflow.md#APPROVAL-FR-03) | requirement | DRAFT-APPROVAL-1, DRAFT-ELIG-1 |
| [ELIG-FR-01](draft-and-approval-workflow.md#ELIG-FR-01) | requirement | DRAFT-ELIG-1 |
| [ELIG-FR-02](draft-and-approval-workflow.md#ELIG-FR-02) | requirement | DRAFT-ELIG-1 |
| [ELIG-FR-03](draft-and-approval-workflow.md#ELIG-FR-03) | requirement | DRAFT-ELIG-1 |
| [DRAFT-CON-01](draft-and-approval-workflow.md#DRAFT-CON-01) | constraint | DRAFT-MODEL-1, DRAFT-APPROVAL-1 |
| [DRAFT-CON-02](draft-and-approval-workflow.md#DRAFT-CON-02) | constraint | DRAFT-MODEL-1, DRAFT-CONTENT-1 |
| [CONTENT-CON-01](draft-and-approval-workflow.md#CONTENT-CON-01) | constraint | DRAFT-CONTENT-1 |
| [SAFETY-CON-01](draft-and-approval-workflow.md#SAFETY-CON-01) | constraint | DRAFT-SECRET-1, DRAFT-APPROVAL-1 |
| [APPROVAL-CON-01](draft-and-approval-workflow.md#APPROVAL-CON-01) | constraint | DRAFT-APPROVAL-1, DRAFT-ELIG-1 |
