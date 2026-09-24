# repo-com

A local, repository-scoped safety and state foundation for agent-originated Discord workflows.

> **Status:** workspace version `0.1.0`, pre-release. The repository currently contains four tested Rust library crates. It does not contain an installable `repo-com` binary, a tagged release, a Discord network client, or the complete end-to-end product workflow.

## What exists now

The implemented surface is intentionally small and evidence-led:

| Package | Library | Implemented responsibility |
|---|---|---|
| `foundation_contract` | `repo_com_foundation` | Protocol-version-1 outcome envelopes, stable error categories, typed global arguments, output stream separation, and explicit TTY/non-TTY decisions. |
| `config_contract` | `repo_com_config` | Strict schema-version-1 TOML parsing, bounded repository-root discovery, alias resolution, safe errors, and normalized SHA-256 configuration hashes. |
| `state_contract` | `repo_com_state` | Repository-scoped SQLite state, forward-only migration version 1, WAL/foreign-key/busy-timeout setup, immutable evidence boundaries, and user-only file handling on Unix. |
| `policy_contract` | `repo_com_policy` | Exact event/destination/severity matching, TTY-confirmed activation, stale-hash invalidation, ambiguous-policy denial, and permission-reducing deactivation. |

The current workspace validation runs **42 tests across four binaries**. The tests cover protocol invariants, configuration rejection and discovery, SQLite migrations and concurrency, repository isolation, immutable evidence, and policy activation behavior.

## Current library workflow

There is no CLI yet. Library consumers can currently:

1. Parse and resolve a secret-free `.repo-com.toml` file.
2. Open the local state store and register a repository scope.
3. Preview and activate one exact policy tuple through an explicitly supplied TTY confirmation.
4. Inspect policy status, evaluate the current hashes, and deactivate without widening authority.
5. Build protocol-version-1 success or error values for a future command layer.

See the [Library Consumer Guide](docs/user-guide.md) for a complete Rust workflow and the [Administrator Guide](docs/admin-guide.md) for state, configuration, security, and recovery details.

## Not yet available

The following product surfaces remain planned and must not be presented as installed behavior:

- a globally installed `repo-com` executable or command tree;
- Discord REST v10 client, bot authentication, setup checks, delivery, or reconciliation;
- draft-content creation, secret scanning, exact approval, and send eligibility;
- bounded inbound fetch, threaded reply, retention/purge, and terminal renderers;
- cross-platform CI, packaged release artifacts, live Discord validation, or human release sign-off.

The intended product contract remains in the [Product Vision](docs/PRD.md) and [feature specifications](docs/features/). Those documents are requirements and roadmap material, not proof of current runtime behavior.

## Configuration

The smallest current configuration example is [`examples/repo-com.example.toml`](examples/repo-com.example.toml). It is schema version 1 and contains only synthetic IDs and aliases. The configuration crate does not accept secret-like fields and does not consume a Discord token.

The future product contract reserves `REPO_COM_DISCORD_TOKEN` for a dedicated bot, but no current crate reads it and no Discord request is made by the current workspace.

## State location

The state crate resolves the operational database below the OS user-data directory:

```text
<user-data-root>/repo-com/state.sqlite3
```

The exact root is supplied by `dirs::data_local_dir()`. On Unix, the implementation creates the state directory with mode `0700` and the database with mode `0600`; on Windows it relies on the inherited user-profile ACL boundary. The default state path is outside the repository; explicit `open_path` calls are caller-controlled and must not be used for operational repository-local state.

## Build and validation

Use the pinned toolchain from [`rust-toolchain.toml`](rust-toolchain.toml):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The release boundary requires SQLite `3.53.4` or newer. Development and contract-test opens can use the local engine; release-oriented opens use `StateStore::open_for_release` and fail before creating or mutating state when the linked runtime is older.

## Documentation

| Document | Purpose |
|---|---|
| [Library Consumer Guide](docs/user-guide.md) | Current library workflow, configuration, policy operations, outcomes, and troubleshooting |
| [Administrator Guide](docs/admin-guide.md) | Local checkout setup, state paths, permissions, migrations, backups, security, and recovery |
| [Architecture Decision Records](docs/adr/README.md) | Durable decisions and their implementation status |
| [Changelog](CHANGELOG.md) | Repository history, including the current library implementation |
| [Unreleased release notes](docs/releases/unreleased.md) | Current pre-release status and validation boundary |
| [Product Vision](docs/PRD.md) | Canonical future product requirements |
| [Feature specifications](docs/features/) | Detailed future feature requirements and task contracts |

## Repository layout

```text
Cargo.toml                  Rust workspace and dependency policy
Cargo.lock                  Locked dependency graph
rust-toolchain.toml         Rust 1.98.1 toolchain pin
.config/nextest.toml        Test-selection policy
examples/                   Secret-free configuration example
crates/
  repo-com-foundation/      Protocol, error, argument, and TTY contracts
  repo-com-config/          Configuration model, parser, discovery, validation
  repo-com-state/           SQLite paths, migrations, repositories, and state
  repo-com-policy/          Exact policy matching and activation
docs/                       Product requirements, guides, ADRs, and release notes
```

## Project status

The authoritative workspace version is `0.1.0`; the `1.0` entry in the Product Vision is a planned product target, not a released version. There are no Git tags or packaged releases. The current implementation is a library foundation, not a complete Discord transport. The [Unreleased release notes](docs/releases/unreleased.md) record what was validated and what remains planned.
