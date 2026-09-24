# Feature: Repository Configuration and State

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** [CLI Foundation](cli-foundation.md)  
**Status:** Canonical v1 plan

This feature establishes strict committed configuration, explicit local policy activation, the repository-keyed SQLite store, append-only audit writes, and local audit queries. These interfaces are prerequisites for every send and inbound workflow.

> **Current implementation:** `REPO-CFG-1`, `REPO-STATE-1`, and `REPO-POLICY-1` are implemented as tested Rust libraries. The dedicated audit query surface, Discord setup, delivery, inbound transport, retention, and purge integrations remain future work; this document continues to own the planned requirements and task contract.

### In Scope

- `.repo-com.toml` schema version 1 and repository-root discovery.
- Non-secret destination, inbound, retention, and exact auto-send definitions.
- Interactive policy activation bound to canonical config and tuple hashes.
- OS-native user-data paths and a forward-migrated SQLite schema.
- Redacted diagnostics and append-only lifecycle audit events.
- Local audit filtering without telemetry or remote synchronization.

### Out of Scope

- Storing a Discord token or any secret in config or state.
- Activating policy from a skill or non-TTY command.
- Export/import, encryption, keychain integration, remote state, or read receipts.

---

## 2. Interfaces and Preconditions

### Configuration Shape

```toml
schema_version = 1
repository_id = "acme/widgets"

[discord]
workspace_id = "123456789012345678"

[destinations.release]
channel_id = "234567890123456789"
allowed_mentions = ["oncall"]

[mentions.oncall]
target = "role:345678901234567890"

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365

[[auto_send]]
event_type = "build_failed"
destination = "release"
severity = "high"
```

Unknown keys, duplicate aliases, unsafe schema versions, cross-workspace references, invalid mention prefixes, or secret-like fields fail validation. Alias names are repository-local and are the only destinations accepted by normal skill commands.

### Store Boundary

The initial migration creates repository, draft/revision, approval, policy activation, delivery attempt, inbound cursor/item, acknowledgement/archive/reply-link, and append-only audit tables. The runtime enables foreign keys, WAL, and a bounded busy timeout. Database corruption is reported without deleting or recreating user data.

| Interface | Owner Task | Prerequisite |
|---|---|---|
| `ConfigResolver` | REPO-CFG-1 | PLAT-1 |
| `StateStore` and schema repositories | REPO-STATE-1 | PLAT-1 |
| `PolicyRegistry` | REPO-POLICY-1 | REPO-CFG-1, REPO-STATE-1 |
| `AuditWriter` | REPO-AUDIT-1 | REPO-STATE-1 |
| `AuditQuery` | REPO-AUDIT-2 | REPO-AUDIT-1 |

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| REPO-FR-01 | requirement | Must | REPO-CFG-1 |
| REPO-FR-02 | requirement | Must | REPO-CFG-1 |
| REPO-FR-03 | requirement | Must | REPO-CFG-1 |
| REPO-FR-04 | requirement | Must | REPO-CFG-1 |
| POLICY-FR-01 | requirement | Must | REPO-POLICY-1 |
| POLICY-FR-02 | requirement | Must | REPO-POLICY-1 |
| POLICY-FR-03 | requirement | Must | REPO-POLICY-1 |
| STATE-FR-01 | requirement | Must | REPO-STATE-1 |
| STATE-FR-02 | requirement | Must | REPO-STATE-1 |
| STATE-FR-03 | requirement | Must | REPO-STATE-1 |
| AUDIT-FR-01 | requirement | Must | REPO-AUDIT-1 |
| AUDIT-FR-02 | requirement | Must | REPO-AUDIT-1 |
| AUDIT-FR-03 | requirement | Must | REPO-AUDIT-2 |
| REPO-CON-01 | constraint | Must | REPO-CFG-1 |
| STATE-CON-01 | constraint | Must | REPO-STATE-1 |
| STATE-CON-02 | constraint | Must | REPO-STATE-1, REL-INFRA-1 |
| POLICY-CON-01 | constraint | Must | REPO-POLICY-1 |
| AUDIT-CON-01 | constraint | Must | REPO-AUDIT-1, REPO-AUDIT-2 |

```forge-requirement
{"id":"REPO-FR-01","kind":"requirement","text":"Resolve configuration from an explicit normalized path or by searching the current directory and ancestors up to the repository root for exactly one .repo-com.toml file; stop with a typed error when zero or multiple candidates apply."}
```

```forge-requirement
{"id":"REPO-FR-02","kind":"requirement","text":"Parse repository configuration schema version 1 with strict unknown-field rejection and support repository identity, one Discord workspace ID, destination aliases, named mention aliases, inbound aliases, retention, and exact auto-send entries."}
```

```forge-requirement
{"id":"REPO-FR-03","kind":"requirement","text":"Resolve destination aliases to one channel and an allowlist of named mention aliases whose targets are role: or user: identifiers, and resolve inbound aliases only to configured channels in the same workspace."}
```

```forge-requirement
{"id":"REPO-FR-04","kind":"requirement","text":"Provide config validation that reports precise path-aware errors in human or protocol JSON form without logging file contents or secret-like values."}
```

```forge-requirement
{"id":"POLICY-FR-01","kind":"requirement","text":"Match automatic sending only when one configured policy exactly equals the draft event type, destination alias, and severity; a wildcard, prefix, or broader severity match is ineligible."}
```

```forge-requirement
{"id":"POLICY-FR-02","kind":"requirement","text":"Activate an exact policy only through a TTY operator confirmation that records the canonical configuration hash, policy tuple hash, and activation time in user-level state."}
```

```forge-requirement
{"id":"POLICY-FR-03","kind":"requirement","text":"Expose policy status and deactivation, automatically report an activation stale after any relevant config or tuple hash change, and prevent a skill from activating or widening policy in non-TTY mode."}
```

```forge-requirement
{"id":"STATE-FR-01","kind":"requirement","text":"Store one user-level SQLite database in the OS application-data location, create it with user-only filesystem permissions, and namespace every record by the configured repository ID."}
```

```forge-requirement
{"id":"STATE-FR-02","kind":"requirement","text":"Provide forward-only migrations and transactional repositories for repository identity, drafts and immutable revisions, approvals, policy activations, delivery attempts, inbound items and cursors, acknowledgement/archive/reply links, and append-only audit events."}
```

```forge-requirement
{"id":"STATE-FR-03","kind":"requirement","text":"Enable foreign keys, WAL, and a bounded busy timeout; handle concurrent readers/writers deterministically and report corruption, unsupported schema, lock timeout, or migration failure without deleting or recreating the database."}
```

```forge-requirement
{"id":"AUDIT-FR-01","kind":"requirement","text":"Append a structured audit event for each meaningful local transition with repository ID, object type and ID, transition, UTC timestamp, actor kind, outcome, and redacted metadata in the same transaction as the state change when one exists."}
```

```forge-requirement
{"id":"AUDIT-FR-02","kind":"requirement","text":"Enable diagnostics only by explicit operator flag or environment setting and redact message content, token values, authorization headers, and secret-like values by default even in the highest supported diagnostic level."}
```

```forge-requirement
{"id":"AUDIT-FR-03","kind":"requirement","text":"Query local audit events by repository, time range, object type, object ID, and transition with bounded pagination; inspection never claims a remote read receipt or mutates Discord."}
```

```forge-requirement
{"id":"REPO-CON-01","kind":"constraint","text":"Repository configuration must contain no token, password, private key, cookie, authorization value, or secret-like field; unknown or future schema versions and unknown keys fail closed without automatic migration."}
```

```forge-requirement
{"id":"STATE-CON-01","kind":"constraint","text":"Local state must never be uploaded, synchronized, or exposed through a network service and must not use repository-relative storage for operational data."}
```

```forge-requirement
{"id":"STATE-CON-02","kind":"constraint","text":"Release builds must assert SQLite version 3.53.4 or newer at runtime and compile; development may use a newer local SQLite but must never silently test a release artifact against an older engine."}
```

```forge-requirement
{"id":"POLICY-CON-01","kind":"constraint","text":"Policy activation and deactivation commands that expand permission require an interactive TTY; non-TTY execution may inspect or deactivate but may not activate."}
```

```forge-requirement
{"id":"AUDIT-CON-01","kind":"constraint","text":"Audit records are append-only local evidence; a later remote edit or deletion is represented as a new event and never overwrites the original local snapshot."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| REPO-CFG-1 | Strict config discovery, parse, and validation | configuration-engineer | PLAT-1 | `repo-com-config` and `config_contract` | REPO-FR-01 through REPO-FR-04, REPO-CON-01 | Secrets, policy activation, CLI |
| REPO-STATE-1 | Repository-keyed migrated transactional store | persistence-engineer | PLAT-1 | `repo-com-state`, initial migration, `state_contract` | STATE-FR-01 through STATE-FR-03 and state constraints | Business workflows, retention execution |
| REPO-POLICY-1 | Exact operator-activated policy registry | policy-engineer | Config and state interfaces | `repo-com-policy`, `policy_contract` | POLICY-FR-01 through POLICY-FR-03, POLICY-CON-01 | Draft approval, Discord |
| REPO-AUDIT-1 | Transactional redacted audit writer | audit-engineer | State store | `repo-com-audit`, `audit_contract` | AUDIT-FR-01, AUDIT-FR-02 | Query UI, telemetry |
| REPO-AUDIT-2 | Bounded local audit query service | audit-engineer | Audit writer | `repo-com-audit-query`, `audit_query_contract` | AUDIT-FR-03, AUDIT-CON-01 | Remote history, read receipts |

---

## Phase 1: Configuration, State, and Audit Foundations

```forge-task
{
  "id": "REPO-CFG-1",
  "title": "Implement strict repository configuration",
  "description": "Implement strict schema-version-1 repository configuration discovery, parsing, destination and named-mention alias resolution, and path-aware diagnostics in the focused repo-com-config crate. Search only from the working directory through the detected repository root, reject unknown fields, unsafe versions, duplicate aliases, missing mention targets, secret-like keys, and cross-workspace references, and provide a non-secret example configuration. Do not activate policy, access Discord, or persist operational state.",
  "ownerAgent": "configuration-engineer",
  "dependencies": ["PLAT-1"],
  "expectedOutputs": [
    "crates/repo-com-config/Cargo.toml",
    "crates/repo-com-config/src/lib.rs",
    "crates/repo-com-config/src/model.rs",
    "crates/repo-com-config/src/resolve.rs",
    "crates/repo-com-config/src/validation.rs",
    "crates/repo-com-config/tests/config_contract.rs",
    "examples/repo-com.example.toml"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(config_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/repository-configuration-and-state.md#REPO-FR-01",
      "docs/features/repository-configuration-and-state.md#REPO-FR-02",
      "docs/features/repository-configuration-and-state.md#REPO-FR-03",
      "docs/features/repository-configuration-and-state.md#REPO-FR-04",
      "docs/PRD.md#RC-FR-03"
    ],
    "acceptanceCriteria": [
      "Configuration tests cover explicit-path and ancestor discovery, zero/multiple-candidate failures, and no search above the repository root",
      "Configuration tests cover valid schema version 1, unknown keys, unsafe versions, duplicate aliases, missing or invalid named mention targets, and cross-workspace references",
      "Configuration tests prove secret-like fields and raw destination fields are rejected without echoing their values",
      "The example configuration parses successfully and contains no credential value"
    ],
    "constraints": [
      "Configuration parsing must be side-effect free"
    ],
    "constraintRefs": [
      "docs/features/repository-configuration-and-state.md#REPO-CON-01",
      "docs/PRD.md#RC-SEC-03"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/features/repository-configuration-and-state.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "REPO-STATE-1",
  "title": "Create the repository-scoped SQLite store",
  "description": "Create the focused repo-com-state crate with OS user-data path resolution, user-only database creation, forward-only migrations, and transactional repositories for every v1 state family. The initial schema must cover repository identity, draft revisions, approvals, policy activation, delivery attempts, inbound snapshots and cursors, acknowledgement/archive/reply links, and append-only audit events. Enable foreign keys, WAL, and bounded busy timeout, and fail without destructive recreation on corruption, lock timeout, or unsupported schema. Do not implement draft policy, network calls, retention execution, or audit queries.",
  "ownerAgent": "persistence-engineer",
  "dependencies": ["PLAT-1"],
  "expectedOutputs": [
    "crates/repo-com-state/Cargo.toml",
    "crates/repo-com-state/src/lib.rs",
    "crates/repo-com-state/src/paths.rs",
    "crates/repo-com-state/src/migrations.rs",
    "crates/repo-com-state/src/store.rs",
    "crates/repo-com-state/migrations/0001_initial.sql",
    "crates/repo-com-state/tests/state_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(state_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/repository-configuration-and-state.md#STATE-FR-01",
      "docs/features/repository-configuration-and-state.md#STATE-FR-02",
      "docs/features/repository-configuration-and-state.md#STATE-FR-03"
    ],
    "acceptanceCriteria": [
      "State tests verify OS-specific user-data path selection and user-only permissions or the documented Windows ACL behavior",
      "Migration tests start from an empty database, apply schema version 1 exactly once, and reopen without destructive changes",
      "Repository scoping tests prove records from one repository ID cannot be read or mutated through another repository ID",
      "Concurrency tests cover WAL readers, bounded busy timeout, and conflicting transactions without data loss",
      "Corruption and unsupported-schema tests preserve the original database bytes and return typed integrity errors"
    ],
    "constraints": [
      "Operational state must not be stored beneath the repository"
    ],
    "constraintRefs": [
      "docs/features/repository-configuration-and-state.md#STATE-CON-01",
      "docs/features/repository-configuration-and-state.md#STATE-CON-02",
      "docs/PRD.md#RC-NFR-03"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/features/repository-configuration-and-state.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "REPO-POLICY-1",
  "title": "Implement exact operator policy activation",
  "description": "Implement the focused repo-com-policy crate for exact event type, destination alias, and severity matching plus an operator-controlled activation registry. Canonicalize and hash the complete relevant configuration and policy tuple, record activation time in user state, mark stale on any relevant hash change, and require interactive TTY confirmation for activation. Permit status inspection and permission-reducing deactivation in automation. Do not evaluate draft content or call Discord.",
  "ownerAgent": "policy-engineer",
  "dependencies": ["REPO-CFG-1", "REPO-STATE-1"],
  "expectedOutputs": [
    "crates/repo-com-policy/Cargo.toml",
    "crates/repo-com-policy/src/lib.rs",
    "crates/repo-com-policy/src/hash.rs",
    "crates/repo-com-policy/src/activation.rs",
    "crates/repo-com-policy/src/evaluate.rs",
    "crates/repo-com-policy/tests/policy_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(policy_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/repository-configuration-and-state.md#POLICY-FR-01",
      "docs/features/repository-configuration-and-state.md#POLICY-FR-02",
      "docs/features/repository-configuration-and-state.md#POLICY-FR-03"
    ],
    "acceptanceCriteria": [
      "Policy tests prove only an exact event type, destination alias, and severity tuple matches and reject wildcard or broader-severity matches",
      "Activation tests prove a TTY-confirmed policy is eligible only while both canonical config hash and tuple hash match",
      "Activation tests prove a changed destination, mention set, retention value, or policy tuple makes prior activation stale",
      "Non-TTY tests prove activation fails closed while status and deactivation remain available"
    ],
    "constraints": [
      "Policy matching must be deterministic across platforms and normalized config ordering"
    ],
    "constraintRefs": [
      "docs/features/repository-configuration-and-state.md#POLICY-CON-01",
      "docs/PRD.md#RC-SEC-04"
    ],
    "references": [
      "docs/features/repository-configuration-and-state.md#2. Interfaces and Preconditions",
      "docs/PRD.md#10. Security and Privacy"
    ]
  }
}
```

```forge-task
{
  "id": "REPO-AUDIT-1",
  "title": "Write redacted append-only audit events",
  "description": "Implement the focused repo-com-audit crate as a transactional append-only audit writer plus opt-in diagnostic initialization. Record the defined transition envelope without message content or secret values, support the same transaction as a state mutation, and redact content, token values, authorization headers, and secret-like values at every diagnostic level. Do not implement query pagination, remote history, or product telemetry.",
  "ownerAgent": "audit-engineer",
  "dependencies": ["REPO-STATE-1"],
  "expectedOutputs": [
    "crates/repo-com-audit/Cargo.toml",
    "crates/repo-com-audit/src/lib.rs",
    "crates/repo-com-audit/src/event.rs",
    "crates/repo-com-audit/src/redact.rs",
    "crates/repo-com-audit/src/writer.rs",
    "crates/repo-com-audit/tests/audit_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(audit_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/repository-configuration-and-state.md#AUDIT-FR-01",
      "docs/features/repository-configuration-and-state.md#AUDIT-FR-02",
      "docs/PRD.md#RC-FR-05"
    ],
    "acceptanceCriteria": [
      "Audit tests prove a state change and its event commit or roll back together",
      "Redaction tests cover Discord bot tokens, authorization headers, message text, private-key markers, and secret-like field names at every enabled diagnostic level",
      "Append-only tests prove existing events cannot be updated or deleted through the writer API",
      "Diagnostics remain disabled by default and emit no output unless explicitly enabled"
    ],
    "constraints": [
      "Audit records must not contain raw message content"
    ],
    "constraintRefs": [
      "docs/features/repository-configuration-and-state.md#AUDIT-CON-01",
      "docs/PRD.md#RC-SEC-01"
    ],
    "references": [
      "docs/PRD.md#10. Security and Privacy",
      "docs/features/repository-configuration-and-state.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "REPO-AUDIT-2",
  "title": "Query local audit evidence",
  "description": "Implement the focused repo-com-audit-query crate for bounded, repository-scoped audit lookup by time, object type, object ID, and transition. Return only redacted local evidence with stable pagination and explicit truncation metadata, and expose no remote-history or read-receipt operation. Do not add a live UI, export format, or mutation path.",
  "ownerAgent": "audit-engineer",
  "dependencies": ["REPO-AUDIT-1"],
  "expectedOutputs": [
    "crates/repo-com-audit-query/Cargo.toml",
    "crates/repo-com-audit-query/src/lib.rs",
    "crates/repo-com-audit-query/src/filter.rs",
    "crates/repo-com-audit-query/src/query.rs",
    "crates/repo-com-audit-query/tests/audit_query_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(audit_query_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/repository-configuration-and-state.md#AUDIT-FR-03",
      "docs/PRD.md#RC-FR-06"
    ],
    "acceptanceCriteria": [
      "Query tests combine and separately apply repository, time, object type, object ID, and transition filters",
      "Pagination tests prove stable ordering, bounded page size, and explicit next-page or truncation metadata",
      "Repository isolation tests prove no event from another repository ID is returned",
      "Query output contains no raw message content and has no Discord mutation operation"
    ],
    "constraints": [
      "Audit inspection remains local-only"
    ],
    "constraintRefs": [
      "docs/features/repository-configuration-and-state.md#AUDIT-CON-01"
    ],
    "references": [
      "docs/PRD.md#7. Technical Architecture",
      "docs/PRD.md#16. Analytics / Success Metrics"
    ]
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Tasks |
|---|---|---|
| [REPO-FR-01](repository-configuration-and-state.md#REPO-FR-01) | requirement | REPO-CFG-1 |
| [REPO-FR-02](repository-configuration-and-state.md#REPO-FR-02) | requirement | REPO-CFG-1 |
| [REPO-FR-03](repository-configuration-and-state.md#REPO-FR-03) | requirement | REPO-CFG-1 |
| [REPO-FR-04](repository-configuration-and-state.md#REPO-FR-04) | requirement | REPO-CFG-1 |
| [POLICY-FR-01](repository-configuration-and-state.md#POLICY-FR-01) | requirement | REPO-POLICY-1 |
| [POLICY-FR-02](repository-configuration-and-state.md#POLICY-FR-02) | requirement | REPO-POLICY-1 |
| [POLICY-FR-03](repository-configuration-and-state.md#POLICY-FR-03) | requirement | REPO-POLICY-1 |
| [STATE-FR-01](repository-configuration-and-state.md#STATE-FR-01) | requirement | REPO-STATE-1 |
| [STATE-FR-02](repository-configuration-and-state.md#STATE-FR-02) | requirement | REPO-STATE-1 |
| [STATE-FR-03](repository-configuration-and-state.md#STATE-FR-03) | requirement | REPO-STATE-1 |
| [AUDIT-FR-01](repository-configuration-and-state.md#AUDIT-FR-01) | requirement | REPO-AUDIT-1 |
| [AUDIT-FR-02](repository-configuration-and-state.md#AUDIT-FR-02) | requirement | REPO-AUDIT-1 |
| [AUDIT-FR-03](repository-configuration-and-state.md#AUDIT-FR-03) | requirement | REPO-AUDIT-2 |
| [REPO-CON-01](repository-configuration-and-state.md#REPO-CON-01) | constraint | REPO-CFG-1 |
| [STATE-CON-01](repository-configuration-and-state.md#STATE-CON-01) | constraint | REPO-STATE-1 |
| [STATE-CON-02](repository-configuration-and-state.md#STATE-CON-02) | constraint | REPO-STATE-1, REL-INFRA-1 |
| [POLICY-CON-01](repository-configuration-and-state.md#POLICY-CON-01) | constraint | REPO-POLICY-1 |
| [AUDIT-CON-01](repository-configuration-and-state.md#AUDIT-CON-01) | constraint | REPO-AUDIT-1, REPO-AUDIT-2 |
