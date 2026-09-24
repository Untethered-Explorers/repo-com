# repo-com

A local, repository-scoped safety and state foundation for agent-originated Discord workflows.

> **Status:** workspace version `0.1.0`, pre-release. The repository currently contains 19 focused Rust library crates. It has no Git tag, packaged artifact, installable `repo-com` binary, final command tree, terminal UI, or live Discord acceptance. Discord REST adapters and token-free WireMock contracts exist, but they are not composed into an end-to-end product.

## What exists now

The implemented surface is intentionally split into small library contracts. Every package is a library; there is no `src/main.rs` or `[[bin]]` target in the current workspace.

| Area | Packages | Current responsibility |
|---|---|---|
| Protocol | `foundation_contract` (`repo_com_foundation`) | Protocol-version-1 outcomes, stable error categories, typed global arguments, explicit TTY decisions, and separated output values |
| Configuration, state, and policy | `config_contract`, `state_contract`, `policy_contract` | Strict repository configuration, repository-scoped SQLite state, exact policy matching, TTY-confirmed activation, and stale-hash invalidation |
| Audit | `audit_contract`, `audit_query_contract` | Transactional redacted append-only evidence and bounded repository-scoped local queries |
| Drafts and approval | `draft_model_contract`, `draft_content_contract`, `draft_safety_contract`, `approval_contract`, `send_eligibility_contract` | Immutable revisions, deterministic Discord text and nonce rendering, credential-pattern scanning, exact-revision approval, and fail-closed eligibility |
| Discord | `discord_client_contract`, `discord_message_contract` | Dedicated-bot REST v10 setup checks and one-attempt text-message operations with typed rate-limit and uncertainty outcomes |
| Delivery | `delivery_contract`, `delivery_retry_contract` | Atomic duplicate-safe local claims, delivery state transitions, bounded retry policy, and read-only unknown-delivery reconciliation |
| Inbound and reply | `inbox_state_contract`, `inbox_fetch_contract`, `reply_contract` | Bounded untrusted inbound retrieval, local acknowledgement/archive state, and validated threaded reply drafts |
| Retention | `retention_contract` | Repository-scoped content and metadata retention sweeps with deterministic cutoffs and blocking failure behavior |

A library consumer can now:

1. Resolve a secret-free `.repo-com.toml` file and its named aliases.
2. Open the local SQLite store and register a repository scope.
3. Create immutable draft revisions, render channel-ready text, scan for credential patterns, and build a complete preview.
4. Record exact TTY-bound approval or activate a narrow exact policy, then evaluate current send eligibility.
5. Claim a local delivery attempt, invoke the one-attempt Discord adapter, classify the result, and use the separate retry and read-only reconciliation contracts.
6. Fetch bounded inbound data as untrusted input, store local lifecycle evidence, and create a validated reply draft without implicitly sending it.
7. Query local audit evidence and run repository-scoped retention sweeps.

These contracts are not a single composed workflow. The embedding application owns the orchestration, clocks, transactions, terminal confirmation, and network sequencing between crates.

## What is not yet available

- An installed `repo-com` executable, command tree, human terminal renderer, or keyboard prompt flow.
- A complete mocked end-to-end journey, cross-platform CI, packaged installers, checksums, SBOM, performance evidence, or human release sign-off.
- Confirmed purge execution, read-only lifecycle inspection, and final operator command composition.
- A restart-safe production coordinator connecting draft, approval, eligibility, claim, Discord transport, retry, reconciliation, inbound state, and retention.
- Live Discord compatibility evidence. The HTTP contracts use token-free WireMock fixtures and do not prove behavior against a live workspace.

The [Product Vision](docs/PRD.md) and [feature specifications](docs/features/) remain requirements and roadmap material. They are not proof of current runtime behavior.

## Configuration and Discord credentials

The smallest current configuration example is [`examples/repo-com.example.toml`](examples/repo-com.example.toml). It uses schema version `1`, synthetic workspace/channel/mention IDs, inbound aliases, retention, and an exact auto-send tuple. The configuration model rejects unknown and secret-like fields, raw destination fields, malformed IDs, duplicate aliases, and unsafe policy values.

Discord credentials are not configuration fields. The Discord adapters accept only a raw dedicated bot token from `REPO_COM_DISCORD_TOKEN`, use `Bot` authorization, pin REST API v10, and zeroize owned token storage. The production client uses Discord's fixed origin; test-only constructors accept an origin-only loopback WireMock endpoint. User tokens, self-bots, unlisted endpoints, and unlisted mention targets fail closed.

A caller that constructs the production adapters can perform network I/O. The current workspace does not provide a command that invokes them automatically, and the automated tests do not use a live token or workspace.

## State and privacy

The default state path is:

```text
<dirs::data_local_dir()>/repo-com/state.sqlite3
```

The state schema is version `1`, repository-scoped, forward-only, and uses WAL, foreign keys, a bounded busy timeout, immutable evidence triggers, and user-only filesystem protection on Unix. On Windows, the inherited user-profile ACL is the boundary. The state database is not encrypted at rest.

Retention defaults to 30 days for draft and inbound content and 365 days for non-content metadata. The retention crate performs transactional sweeps and replaces expired content with `[content-expired]`; purge and lifecycle inspection are not implemented. The workspace collects no telemetry and does not synchronize local state or audit evidence.

## Build and validation

Use the pinned toolchain from [`rust-toolchain.toml`](rust-toolchain.toml):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
git diff --check
```

The current full nextest run selected **195 tests across 19 binaries: 195 passed and 2 skipped**. Formatting and clippy also passed on the same workspace. These are automated component and contract checks; they do not prove live Discord behavior, operator usability, packaging readiness, or human approval.

## Documentation

| Document | Purpose |
|---|---|
| [Library Consumer Guide](docs/user-guide.md) | Current library workflow, APIs, configuration, states, recovery, and troubleshooting |
| [Administrator Guide](docs/admin-guide.md) | Local checkout, state operations, configuration, Discord validation, security, backups, and upgrades |
| [Architecture Decision Records](docs/adr/README.md) | Durable decisions and their implementation status |
| [Changelog](CHANGELOG.md) | Repository history and current validation boundary |
| [Unreleased release notes](docs/releases/unreleased.md) | Current pre-release status and limitations |
| [Product Vision](docs/PRD.md) | Canonical future product requirements |
| [Feature specifications](docs/features/) | Detailed future requirements and task contracts |

## Repository layout

```text
Cargo.toml                  Rust workspace and dependency policy
Cargo.lock                  Locked dependency graph
rust-toolchain.toml         Rust 1.98.1 toolchain pin
.config/nextest.toml        Test-selection policy
examples/                   Secret-free configuration example
crates/                     19 focused Rust library packages
docs/                       Product requirements, guides, ADRs, and release notes
```

## Project status

The authoritative workspace version is `0.1.0`. The `1.0` entry in the Product Vision is a planned product target, not a released version. There are no Git tags or packaged releases. The current implementation is a tested set of library contracts, not a complete installed Discord transport.
