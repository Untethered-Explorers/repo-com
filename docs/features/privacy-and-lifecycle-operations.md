# Feature: Privacy and Lifecycle Operations

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** [Inbound Retrieval and Reply](inbound-retrieval-and-reply.md)  
**Status:** Canonical v1 plan

This feature enforces local retention without a daemon, provides explicit dry-run and confirmed purge operations, and exposes bounded read-only lifecycle inspection and database verification. It makes the privacy tradeoffs of v1 visible without claiming protections the product does not provide.

### In Scope

- Per-repository content and metadata retention.
- Opportunistic retention sweeps at process start and explicit sweep commands.
- Separate content expiry, metadata expiry, and full local purge.
- Dry-run counts and TTY confirmation for destructive local operations.
- Read-only state, draft, delivery, inbox, and audit inspection.
- SQLite integrity and migration verification without automatic repair.

### Out of Scope

- Encryption at rest, OS keychain integration, remote backup, export, or synchronization.
- A background scheduler, daemon, or automatic Discord deletion.
- Automatic repair or deletion of a corrupt database.
- Product telemetry, remote compliance certification, or legal-retention guarantees.

---

## 2. Interfaces and Preconditions

| Interface | Input | Output | Owner Task |
|---|---|---|---|
| `RetentionPolicy` | Validated repository config and current UTC time | Content and metadata cutoffs | PRIV-RET-1 |
| `RetentionSweeper` | Policy plus transactional state repositories | Counts, removed IDs, and audit summary | PRIV-RET-1 |
| `PurgePlanner` | Repository, scope, cutoff, and dry-run flag | Exact table/category counts without mutation | PRIV-RET-2 |
| `PurgeExecutor` | Confirmed plan and TTY operator action | Transactional local deletion and audit record | PRIV-RET-2 |
| `LifecycleInspector` | Repository, object ID, and bounded page | Read-only lifecycle projection | PRIV-LIFE-1 |
| `StateVerifier` | Database path and expected schema | Integrity, foreign-key, migration, and permission report | PRIV-LIFE-1 |

Retention runs before a state-mutating operation and on explicit invocation. A failed sweep blocks a new send or inbox mutation rather than silently extending retention.

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| RET-FR-01 | requirement | Must | PRIV-RET-1 |
| RET-FR-02 | requirement | Must | PRIV-RET-1 |
| RET-FR-03 | requirement | Must | PRIV-RET-1 |
| RET-FR-04 | requirement | Must | PRIV-RET-2 |
| RET-FR-05 | requirement | Must | PRIV-RET-2 |
| LIFE-FR-01 | requirement | Must | PRIV-LIFE-1 |
| LIFE-FR-02 | requirement | Must | PRIV-LIFE-1 |
| RET-CON-01 | constraint | Must | PRIV-RET-1 |
| RET-CON-02 | constraint | Must | PRIV-RET-1, PRIV-RET-2 |
| RET-CON-03 | constraint | Must | PRIV-RET-1, PRIV-RET-2 |
| LIFE-CON-01 | constraint | Must | PRIV-LIFE-1 |
| PRIV-CON-01 | constraint | Must | PRIV-RET-1, PRIV-LIFE-1 |
| PRIV-CON-02 | constraint | Must | PRIV-RET-1, PRIV-RET-2 |

```forge-requirement
{"id":"RET-FR-01","kind":"requirement","text":"Retain draft and inbound message content for 30 days by default and non-content delivery or audit metadata for one year by default, with validated per-repository overrides."}
```

```forge-requirement
{"id":"RET-FR-02","kind":"requirement","text":"Run an opportunistic retention sweep before each state-mutating invocation and on explicit request; if retention fails, block the new mutation with a typed storage-integrity error rather than continuing past the cutoff."}
```

```forge-requirement
{"id":"RET-FR-03","kind":"requirement","text":"At content expiry remove or irreversibly replace draft and inbound text with a retained content-expired marker while preserving non-content IDs, hashes, timestamps, lifecycle state, and audit evidence."}
```

```forge-requirement
{"id":"RET-FR-04","kind":"requirement","text":"Provide a non-mutating purge plan that reports exact repository-scoped counts for content, metadata, or all local state before an operator executes it."}
```

```forge-requirement
{"id":"RET-FR-05","kind":"requirement","text":"Execute a purge only after an interactive TTY confirmation bound to the current repository, scope, cutoff, plan hash, and configuration hash, and append a redacted audit record of removed counts."}
```

```forge-requirement
{"id":"LIFE-FR-01","kind":"requirement","text":"Inspect repository, draft revision, delivery attempt, inbound item, acknowledgement, archive, reply link, and audit transitions through bounded read-only queries without interpreting content or claiming remote state beyond the last recorded fetch."}
```

```forge-requirement
{"id":"LIFE-FR-02","kind":"requirement","text":"Verify SQLite quick_check, foreign keys, expected migration version, repository scope, and filesystem permissions and return a structured report without modifying or recreating the database."}
```

```forge-requirement
{"id":"RET-CON-01","kind":"constraint","text":"Content retention defaults to 30 days, metadata retention defaults to 365 days, content override is 1 through 365 days, metadata override is 30 through 3,650 days, and metadata retention must be at least content retention."}
```

```forge-requirement
{"id":"RET-CON-02","kind":"constraint","text":"Automatic retention may remove content or metadata only; full purge, alternate cutoffs, and cross-category deletion require an explicit operator-confirmed plan."}
```

```forge-requirement
{"id":"RET-CON-03","kind":"constraint","text":"Purge and retention operate only on the selected local repository and never edit, delete, react to, or otherwise mutate a Discord message."}
```

```forge-requirement
{"id":"LIFE-CON-01","kind":"constraint","text":"Lifecycle inspection and state verification are read-only, bounded, paginated where applicable, and never attempt an implicit migration, repair, backup upload, or remote fetch."}
```

```forge-requirement
{"id":"PRIV-CON-01","kind":"constraint","text":"v1 state is protected by user-only filesystem permissions rather than encryption at rest; documentation and human security review must state that local account access, backups, and filesystem snapshots can still read retained content."}
```

```forge-requirement
{"id":"PRIV-CON-02","kind":"constraint","text":"Send no product telemetry, analytics, crash upload, or remote audit synchronization, and keep retention and purge execution entirely local."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| PRIV-RET-1 | Validated cutoff and transactional automatic retention | privacy-engineer | Config, state, audit | `repo-com-retention`, `retention_contract` | RET-FR-01 through 03 and retention/privacy constraints | Explicit destructive purge, inspection |
| PRIV-RET-2 | Hashed dry-run and confirmed local purge | security-engineer | Retention and state | `repo-com-purge`, `purge_contract` | RET-FR-04/05, RET-CON-02/03 | Remote deletion, repair |
| PRIV-LIFE-1 | Read-only lifecycle and state verification | operations-engineer | All state repositories | `repo-com-lifecycle`, `lifecycle_contract` | LIFE-FR-01/02, LIFE-CON-01, privacy disclosure | Mutation, command routing |

---

## Phase 1: Automatic Retention

```forge-task
{
  "id": "PRIV-RET-1",
  "title": "Enforce content and metadata retention",
  "description": "Implement the focused repo-com-retention crate with validated per-repository cutoffs, clock injection, and a transactional sweep that runs before state mutation or explicit invocation. Remove or replace draft and inbound text at content expiry while preserving non-content metadata, and remove expired metadata only at its later cutoff. A failed sweep must return a blocking storage-integrity result. Do not implement full purge, lifecycle inspection, remote deletion, or background scheduling.",
  "ownerAgent": "privacy-engineer",
  "dependencies": ["REPO-CFG-1", "REPO-STATE-1", "REPO-AUDIT-1", "IN-STATE-1", "DISC-DELIVERY-1"],
  "expectedOutputs": [
    "crates/repo-com-retention/Cargo.toml",
    "crates/repo-com-retention/src/lib.rs",
    "crates/repo-com-retention/src/policy.rs",
    "crates/repo-com-retention/src/sweep.rs",
    "crates/repo-com-retention/tests/retention_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(retention_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/privacy-and-lifecycle-operations.md#RET-FR-01",
      "docs/features/privacy-and-lifecycle-operations.md#RET-FR-02",
      "docs/features/privacy-and-lifecycle-operations.md#RET-FR-03"
    ],
    "acceptanceCriteria": [
      "Clock-controlled tests cover the 30-day and 365-day defaults and exact content and metadata cutoff boundaries",
      "Override tests cover allowed minimum and maximum values and reject metadata retention shorter than content retention",
      "Sweep tests prove expired draft and inbound text is removed or replaced while non-content IDs, hashes, timestamps, links, and audit evidence remain",
      "Failure-injection tests prove a partial or failed sweep rolls back and blocks the attempted new state mutation",
      "Repository isolation tests prove one repository's retention never changes another repository's rows",
      "Sweep tests prove no Discord request or remote mutation is possible"
    ],
    "constraints": [
      "Automatic retention changes only local content and metadata"
    ],
    "constraintRefs": [
      "docs/features/privacy-and-lifecycle-operations.md#RET-CON-01",
      "docs/features/privacy-and-lifecycle-operations.md#RET-CON-02",
      "docs/features/privacy-and-lifecycle-operations.md#RET-CON-03",
      "docs/features/privacy-and-lifecycle-operations.md#PRIV-CON-01",
      "docs/features/privacy-and-lifecycle-operations.md#PRIV-CON-02"
    ],
    "references": [
      "docs/PRD.md#10. Security and Privacy",
      "docs/features/privacy-and-lifecycle-operations.md#2. Interfaces and Preconditions"
    ]
  }
}
```

## Phase 2: Purge and Read-Only Lifecycle

```forge-task
{
  "id": "PRIV-RET-2",
  "title": "Implement hashed dry-run and confirmed purge",
  "description": "Implement the focused repo-com-purge crate with repository-scoped content, metadata, and all-state plans. A plan must return exact category and row counts plus a deterministic plan hash without mutation. Execution must require an interactive TTY confirmation matching repository, scope, cutoff, configuration hash, and plan hash, then delete locally in bounded transactions and append a redacted count-only audit event. Do not permit non-TTY execution, remote deletion, automatic alternate cutoff, or database recreation.",
  "ownerAgent": "security-engineer",
  "dependencies": ["PRIV-RET-1", "REPO-CFG-1", "REPO-STATE-1", "REPO-AUDIT-1"],
  "expectedOutputs": [
    "crates/repo-com-purge/Cargo.toml",
    "crates/repo-com-purge/src/lib.rs",
    "crates/repo-com-purge/src/plan.rs",
    "crates/repo-com-purge/src/execute.rs",
    "crates/repo-com-purge/tests/purge_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(purge_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/privacy-and-lifecycle-operations.md#RET-FR-04",
      "docs/features/privacy-and-lifecycle-operations.md#RET-FR-05"
    ],
    "acceptanceCriteria": [
      "Plan tests cover content, metadata, and all scopes with exact deterministic counts and a stable hash while proving no row changes",
      "Confirmation tests reject non-TTY execution and mismatched repository, scope, cutoff, config hash, or plan hash",
      "Execution tests prove only the confirmed repository and category are deleted and a count-only audit event remains available",
      "Failure-injection tests prove partial purge rolls back and a new plan is required before retry",
      "API tests expose no Discord client or remote delete operation"
    ],
    "constraints": [
      "Destructive local purge requires an explicit confirmed plan"
    ],
    "constraintRefs": [
      "docs/features/privacy-and-lifecycle-operations.md#RET-CON-02",
      "docs/features/privacy-and-lifecycle-operations.md#RET-CON-03",
      "docs/features/privacy-and-lifecycle-operations.md#PRIV-CON-02",
      "docs/PRD.md#RC-PRIV-02"
    ],
    "references": [
      "docs/PRD.md#10. Security and Privacy",
      "docs/features/privacy-and-lifecycle-operations.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "PRIV-LIFE-1",
  "title": "Expose read-only lifecycle and state verification",
  "description": "Implement the focused repo-com-lifecycle crate for bounded repository-scoped inspection of drafts, delivery attempts, inbound items, acknowledgement, archive, reply links, and audit transitions, plus a state verifier for SQLite quick_check, foreign keys, expected migration, repository scope, and filesystem permissions. Return content only when the caller explicitly requests a retained item and redact diagnostics. Do not interpret inbound content, fetch Discord, migrate, repair, back up, or mutate state.",
  "ownerAgent": "operations-engineer",
  "dependencies": ["PRIV-RET-1", "REPO-AUDIT-2", "IN-STATE-1", "DISC-DELIVERY-2", "IN-REPLY-1"],
  "expectedOutputs": [
    "crates/repo-com-lifecycle/Cargo.toml",
    "crates/repo-com-lifecycle/src/lib.rs",
    "crates/repo-com-lifecycle/src/inspect.rs",
    "crates/repo-com-lifecycle/src/verify.rs",
    "crates/repo-com-lifecycle/tests/lifecycle_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(lifecycle_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/privacy-and-lifecycle-operations.md#LIFE-FR-01",
      "docs/features/privacy-and-lifecycle-operations.md#LIFE-FR-02"
    ],
    "acceptanceCriteria": [
      "Inspection tests cover every listed object type, stable bounded pagination, retained-content opt-in, and cross-repository denial",
      "Projection tests distinguish local state from last-fetched remote state and never label accepted delivery as read or replied",
      "Verifier tests cover healthy quick_check, foreign-key failure, unexpected migration, wrong repository scope, and unsafe filesystem permissions",
      "Failure tests prove verifier and inspector preserve database bytes and expose no migration, repair, backup, remote fetch, or mutation path",
      "Diagnostic tests prove retained content and secret-like values are redacted"
    ],
    "constraints": [
      "Lifecycle inspection is bounded and read-only"
    ],
    "constraintRefs": [
      "docs/features/privacy-and-lifecycle-operations.md#LIFE-CON-01",
      "docs/features/privacy-and-lifecycle-operations.md#PRIV-CON-01",
      "docs/features/privacy-and-lifecycle-operations.md#PRIV-CON-02"
    ],
    "references": [
      "docs/PRD.md#12. User Interface / Interaction Design",
      "docs/features/privacy-and-lifecycle-operations.md#2. Interfaces and Preconditions"
    ]
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Tasks |
|---|---|---|
| [RET-FR-01](privacy-and-lifecycle-operations.md#RET-FR-01) | requirement | PRIV-RET-1 |
| [RET-FR-02](privacy-and-lifecycle-operations.md#RET-FR-02) | requirement | PRIV-RET-1 |
| [RET-FR-03](privacy-and-lifecycle-operations.md#RET-FR-03) | requirement | PRIV-RET-1 |
| [RET-FR-04](privacy-and-lifecycle-operations.md#RET-FR-04) | requirement | PRIV-RET-2 |
| [RET-FR-05](privacy-and-lifecycle-operations.md#RET-FR-05) | requirement | PRIV-RET-2 |
| [LIFE-FR-01](privacy-and-lifecycle-operations.md#LIFE-FR-01) | requirement | PRIV-LIFE-1 |
| [LIFE-FR-02](privacy-and-lifecycle-operations.md#LIFE-FR-02) | requirement | PRIV-LIFE-1 |
| [RET-CON-01](privacy-and-lifecycle-operations.md#RET-CON-01) | constraint | PRIV-RET-1 |
| [RET-CON-02](privacy-and-lifecycle-operations.md#RET-CON-02) | constraint | PRIV-RET-1, PRIV-RET-2 |
| [RET-CON-03](privacy-and-lifecycle-operations.md#RET-CON-03) | constraint | PRIV-RET-1, PRIV-RET-2 |
| [LIFE-CON-01](privacy-and-lifecycle-operations.md#LIFE-CON-01) | constraint | PRIV-LIFE-1 |
| [PRIV-CON-01](privacy-and-lifecycle-operations.md#PRIV-CON-01) | constraint | PRIV-RET-1, PRIV-LIFE-1 |
| [PRIV-CON-02](privacy-and-lifecycle-operations.md#PRIV-CON-02) | constraint | PRIV-RET-1, PRIV-RET-2 |
