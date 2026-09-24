# Task and Test Map

> Load when: selecting a repo-com implementation package, test target, dependency shape, or documented exception.

Use the owning feature's `forge-task` as authority. This map is a navigation aid, not a replacement for that contract.

## Common Rust Shape

- Rust: 1.98.1
- Edition: 2024
- Workspace members: `crates/*`
- Workspace dependencies: central `Cargo.toml` table
- Contract test pattern: `tests/<name>.rs`
- Required selection: `cargo nextest run --no-tests fail -E 'binary_id(<exact-test-target>)'`

## Task Matrix

| Task | Package or existing target | Test target |
|---|---|---|
| `PLAT-1` | `repo-com-foundation` | `foundation_contract` |
| `REPO-CFG-1` | `repo-com-config` | `config_contract` |
| `REPO-STATE-1` | `repo-com-state` | `state_contract` |
| `REPO-POLICY-1` | `repo-com-policy` | `policy_contract` |
| `REPO-AUDIT-1` | `repo-com-audit` | `audit_contract` |
| `REPO-AUDIT-2` | `repo-com-audit-query` | `audit_query_contract` |
| `DRAFT-MODEL-1` | `repo-com-draft-model` | `draft_model_contract` |
| `DRAFT-CONTENT-1` | `repo-com-draft-content` | `draft_content_contract` |
| `DRAFT-SECRET-1` | `repo-com-draft-safety` | `draft_safety_contract` |
| `DRAFT-APPROVAL-1` | `repo-com-approval` | `approval_contract` |
| `DRAFT-ELIG-1` | `repo-com-send-eligibility` | `send_eligibility_contract` |
| `DISC-CLIENT-1` | `repo-com-discord-client` | `discord_client_contract` |
| `DISC-MSG-1` | `repo-com-discord-message` | `discord_message_contract` |
| `DISC-DELIVERY-1` | `repo-com-delivery` | `delivery_contract` |
| `DISC-DELIVERY-2` | `repo-com-delivery-retry` | `delivery_retry_contract` |
| `IN-STATE-1` | `repo-com-inbox-state` | `inbox_state_contract` |
| `IN-FETCH-1` | `repo-com-inbox-fetch` | `inbox_fetch_contract` |
| `IN-REPLY-1` | `repo-com-reply` | `reply_contract` |
| `PRIV-RET-1` | `repo-com-retention` | `retention_contract` |
| `PRIV-RET-2` | `repo-com-purge` | `purge_contract` |
| `PRIV-LIFE-1` | `repo-com-lifecycle` | `lifecycle_contract` |
| `REL-UI-OUT-1` | `repo-com-terminal-outbound` | `terminal_outbound_contract` |
| `REL-UI-OPS-1` | `repo-com-terminal-operations` | `terminal_operations_contract` |
| `REL-OPS-CMD-1` | `repo-com-cli-operations` | `cli_operations_contract` |
| `REL-MSG-CMD-1` | `repo-com-cli-messaging` | `cli_messaging_contract` |
| `REL-DOC-1` | `repo-com-doc-validation` plus five documents | `documentation_contract` |
| `REL-APP-1` | `repo-com-cli` binary | `command_routing_contract` |
| `REL-PERF-1` | `repo-com-performance` library and binary | `performance_contract` |
| `REL-CI-1` | `repo-com-ci-policy` plus CI outputs | `ci_policy_contract` |
| `REL-PACK-1` | `repo-com-release-policy` plus packaging outputs | `release_policy_contract` |
| `REL-E2E-1` | Existing `repo-com-cli` test tree | `e2e_workflow` |

## Shape Exceptions

- `REL-APP-1` remains a thin installed executable; domain logic belongs to existing crates.
- `REL-DOC-1`, `REL-CI-1`, and `REL-PACK-1` include non-Rust files beyond the package and contract test.
- `REL-PERF-1` owns both a library and `main.rs` for the warm-command harness.
- `REL-E2E-1` does not declare a new package manifest; it adds `tests/e2e_workflow.rs`, support code, and fixtures to `repo-com-cli`.
- Human-review tasks have no agent validation commands and no implementation outputs.

## Task-Specific Extra Checks

Append these only to the tasks that declare them:

```bash
actionlint .github/workflows/ci.yml
actionlint .github/workflows/release.yml
```

Do not create or run the omitted `run-rust-task-checks` package. The task contracts remain the authoritative command catalogue.
