# Administrator Guide

> **Status:** workspace `0.1.0`, pre-release. The repository currently provides Rust libraries and contract tests, not an installable CLI, Discord client, or packaged release. This guide covers the current local development and state operations without presenting planned product commands as available.

## Responsibilities and architecture

A repository maintainer or embedding application is responsible for:

- keeping the Rust toolchain and workspace dependencies pinned;
- providing a secret-free, schema-version-1 `.repo-com.toml`;
- registering repository scopes before writing state;
- protecting the local SQLite state and its parent directory;
- passing an explicit TTY decision for any authority-creating policy activation; and
- preserving state during upgrades and backups.

The current workspace is a Cargo workspace with four library packages. The state crate owns one local SQLite connection boundary; separate processes or threads can open additional connections against the same path. SQLite WAL, foreign-key enforcement, and a bounded busy timeout provide the current concurrency boundary. No server, daemon, telemetry service, or Discord network client exists in the current workspace.

See [ADR-001](adr/ADR-001-rust-workspace-and-patched-sqlite.md), [ADR-002](adr/ADR-002-repository-scoped-local-state.md), and [ADR-005](adr/ADR-005-operator-activated-exact-policy.md) for the implemented foundations and remaining release work.

## Prerequisites

For local development and validation:

- a Rust installation managed by `rustup` or an equivalent toolchain manager;
- Rust `1.98.1`, as pinned in [`rust-toolchain.toml`](../rust-toolchain.toml);
- Cargo and the dependencies recorded in [`Cargo.lock`](../Cargo.lock);
- a writable OS user-data directory; and
- `cargo-nextest` for the declared test command.

The release boundary requires linked SQLite `3.53.4` or newer. Development opens and contract tests can use the locally available SQLite engine; release-oriented opens use `StateStore::open_for_release` and fail before creating or mutating state if the runtime is older.

A Discord bot, token, workspace, channel, or network connection is not required to build or test the current four-crate workspace. Those are prerequisites for the future product surface, not current commands.

## Installation

There is no published install artifact, installer, or `repo-com` executable. For a local checkout:

```bash
cargo metadata --no-deps --format-version 1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The current nextest run executes 42 tests across four binaries. A release package must be built and verified separately once the remaining CLI, adapter, packaging, and release-policy crates are implemented.

## Configuration

The configuration crate discovers `.repo-com.toml` relative to a supplied repository root. With no explicit path it searches the current directory and ancestors up to the inclusive root, selecting exactly one candidate. An explicit path is canonicalized and must remain inside that root. A missing or ambiguous candidate is an error.

The strict schema requires:

- `schema_version = 1`;
- a valid `repository_id`;
- one `[discord]` workspace ID;
- named destination, mention, and inbound alias tables; and
- positive content and metadata retention day values.

Destination channels are named by aliases. Mention targets use only `role:<id>` or `user:<id>`. Inbound aliases must refer to a configured destination. Unknown fields, duplicate aliases, wildcard policy values, secret-like keys, raw destination fields, malformed IDs, and known cross-workspace references are rejected without retaining the offending scalar values.

The example [`examples/repo-com.example.toml`](../examples/repo-com.example.toml) is synthetic and safe to review. The configuration format is intended to remain secret-free. The future product's `REPO_COM_DISCORD_TOKEN` is not read by any current crate.

The normalized configuration is serialized deterministically and hashed with SHA-256. The hash is used by the current policy crate to invalidate an activation after relevant configuration changes.

## Identity, secrets, and TLS

The current workspace has no Discord client and therefore has no token or TLS implementation to operate. Do not add a token, authorization header, bot credential, or private URL to `.repo-com.toml`, examples, fixtures, logs, or documentation.

The future Discord contract reserves `REPO_COM_DISCORD_TOKEN` for a dedicated bot identity. That variable, its bot-only authentication scheme, and its Discord permission requirements are documented as planned product requirements in the [Product Vision](PRD.md) and [ADR-006](adr/ADR-006-dedicated-discord-bot-rest-v10.md); they are not current runtime instructions.

The current configuration validator checks key names for secret-like terms and does not echo the associated values in typed errors. This is a defense against accidental configuration mistakes, not complete data-loss prevention.

## Storage and backups

### Location

The default database path is:

```text
<dirs::data_local_dir()>/repo-com/state.sqlite3
```

`database_path_in(root)` provides the same `repo-com/state.sqlite3` layout below an explicit test or deployment root. The default operational path is outside the repository; an explicit `open_path` call is caller-controlled and must not place live state in the repository.

### Protection

On Unix, the state crate creates the parent directory with mode `0700` and the database with mode `0600`, and it refuses a symbolic-link database path. On Windows, the implementation documents the inherited Known Folder ACL as the boundary; it does not claim to rewrite Windows ACLs through portable permission bits.

The database is not encrypted by this implementation. Local account access, backups, filesystem snapshots, and other copies of the database can expose retained content. Protect copies with equivalent access controls and avoid uploading them to shared or remote storage.

### Schema and migrations

The current schema version is `1`. Migration 1 creates repository, draft/revision, approval, policy, delivery-attempt, inbound, acknowledgement, archive, reply-link, and append-only audit tables. Database triggers protect immutable draft revisions, first inbound snapshots, and audit events.

Migrations are forward-only. The state store rejects a newer schema, preserves corrupt or migration-conflicting bytes, and never automatically deletes or recreates a database. SQLite `user_version`, required tables, triggers, and foreign keys are verified after opening.

SQLite WAL mode creates `-wal` and `-shm` sidecar files while a database is active. Do not treat the main file alone as a complete live backup. Coordinate a safe copy with the embedding application and SQLite's backup/checkpoint semantics; this repository does not provide a backup command.

## Health checks and monitoring

The current local validation gates are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The `StateStore` API also exposes read-only checks for:

- `schema_version()`;
- `foreign_keys_enabled()`;
- `journal_mode()`;
- `quick_check()`;
- the linked runtime and release requirement through `sqlite_runtime()` and `assert_sqlite_runtime()`; and
- user-only file protection through `inspect_user_only_database()`.

There is no monitoring daemon, crash upload, analytics, remote audit synchronization, or Discord health check in the current workspace. The state store's convenience mutations use explicit transactions and roll back on typed failures.

## Upgrades and rollback

There is no packaged upgrade or rollback procedure in version `0.1.0`. When changing the workspace:

1. Back up the database while the embedding application is in a safe state.
2. Run formatting, lint, and all current contract tests.
3. Verify the migration and linked SQLite version before opening operational state.
4. Keep the previous source or binary available for diagnosis, but do not assume an older implementation can open a newer schema.

The release artifact, checksum, SBOM, cross-platform matrix, and migration compatibility policy remain future release work.

## Troubleshooting

| Symptom | Likely cause | Next action |
|---|---|---|
| `config-not-found` | No configuration in the bounded repository search | Add a valid `.repo-com.toml` or pass a path inside the repository root. |
| `multiple-config-candidates` | More than one configuration in the ancestor range | Remove the extra file or use an explicit path. |
| `unsupported-schema-version` | Configuration is not version 1 | Update the document using the [example](../examples/repo-com.example.toml). |
| `secret-field` or `raw-destination-field` | A forbidden key or raw destination field was supplied | Remove the key and use a named alias; do not paste the value into an error report. |
| `operator-action-required` | A caller requested authority creation without a TTY | Supply an explicit interactive confirmation in the embedding application; do not bypass it in automation. |
| `policy-blocked` | Policy is not exact, current, or uniquely activated | Inspect the current config and tuple hashes, then perform a fresh operator activation if appropriate. |
| `storage-integrity` | SQLite is busy, corrupt, unsupported, or migration failed | Preserve the bytes, inspect the typed error and path, and do not delete the database. |
| `UnsupportedSqliteRuntime` | Linked SQLite is below `3.53.4` for a release open | Use an approved release runtime or remain in development mode for contract tests. |
| Permission failure | User-data parent or database cannot be secured | Check ownership, parent permissions, and symbolic-link status; do not weaken access controls. |

## Security and privacy checklist

- [ ] Keep `.repo-com.toml` free of tokens, passwords, private keys, authorization values, and credential-bearing URLs.
- [ ] Treat the current workspace as offline: it contains no Discord client or token consumer.
- [ ] Keep the state database and sidecars outside the repository and out of shared storage.
- [ ] On Unix, verify owner-only directory and file permissions; on Windows, verify the inherited user-profile ACL.
- [ ] Back up unencrypted state only to locations with equivalent access control.
- [ ] Never upload state, audit records, or configuration to telemetry or synchronization services.
- [ ] Treat policy activation as authority creation; require a real operator confirmation and do not infer approval from automation.
- [ ] Keep future approvals, policy changes, retention, and purge revalidation requirements separate from the current policy gate.
- [ ] Remember that no current implementation provides encryption at rest, automatic repair, or a human release sign-off.

## Recovery and support

- **Unknown or stale policy:** inspect the activation snapshot and current hashes. A stale activation does not grant eligibility; deactivate it if needed and activate only after review.
- **Ambiguous policy:** do not choose one row automatically. Resolve the duplicate or malformed active records under operator control.
- **Corrupt or unsupported state:** preserve the database and sidecars, inspect the typed state error, and avoid deletion or recreation.
- **Configuration mistake:** use the safe error code and field path; do not include the original scalar values in logs or issue reports.
- **Future Discord setup:** the bot, token, permissions, and read-only setup behavior are planned surfaces documented in [ADR-006](adr/ADR-006-dedicated-discord-bot-rest-v10.md), not current executable behavior.

## Further documentation

- [Library Consumer Guide](user-guide.md)
- [Architecture Decision Records](adr/README.md)
- [Changelog](../CHANGELOG.md)
- [Unreleased release notes](releases/unreleased.md)
