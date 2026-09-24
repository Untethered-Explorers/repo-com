# Administrator Guide

> **Status:** workspace `0.1.0`, pre-release. This guide covers the current local Rust library workspace and its state/configuration operations. There is no installable CLI, packaged release, purge command, terminal UI, or live Discord acceptance in this release boundary.

## Responsibilities and architecture

A repository maintainer or embedding application is responsible for:

- keeping the Rust toolchain and dependency lockfile pinned;
- providing a secret-free schema-version-1 `.repo-com.toml`;
- registering repository scopes before writing state;
- protecting the local SQLite database, its parent directory, WAL sidecars, and
  backups;
- supplying an explicit TTY confirmation for policy activation, approval, or a
  future purge action;
- composing focused crates without bypassing their transaction, revalidation, or
  untrusted-input boundaries; and
- preserving state during upgrades and diagnostics.

The workspace currently contains 19 library crates. Foundation, configuration,
state, policy, audit, draft, approval, Discord, delivery, inbound, reply, and
retention contracts are independently testable, but there is no final application
binary or daemon that composes them. SQLite WAL, foreign-key enforcement, and a
bounded busy timeout are the current state concurrency boundary.

## Prerequisites

For local development and validation:

- a Rust installation managed by `rustup` or an equivalent toolchain manager;
- Rust `1.98.1`, as pinned in [`rust-toolchain.toml`](../rust-toolchain.toml);
- Cargo and the dependencies recorded in [`Cargo.lock`](../Cargo.lock);
- `cargo-nextest` for the full test command; and
- a writable OS user-data directory for file-backed state.

The release boundary requires linked SQLite `3.53.4` or newer. Development and
contract-test opens can use the locally available SQLite runtime;
`StateStore::open_for_release` and `open_path_for_release` fail before creating or
mutating state when the runtime is older.

A Discord bot, token, workspace, channel, or network connection is not required
to build or run the non-network tests. A network-capable embedding application
must provide a raw dedicated bot token through `REPO_COM_DISCORD_TOKEN` before
constructing a production Discord adapter.

## Installation

There is no published install artifact, installer, or `repo-com` executable. For
a local checkout, use the repository as a Rust library workspace:

```bash
cargo metadata --no-deps --format-version 1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The current full nextest run selected **195 tests across 19 binaries: 195 passed
and 2 skipped**. These are automated library and mocked-contract checks. A
release package must be built and verified separately once the final CLI,
packaging, release-policy, and human-review work is implemented.

## Configuration

The configuration crate discovers `.repo-com.toml` relative to a supplied
repository root. With no explicit path it searches the current directory and
ancestors up to the inclusive root, selecting exactly one candidate. An explicit
path is normalized and must remain inside that root. A missing, ambiguous, or
out-of-root candidate is an error.

The strict schema requires:

- `schema_version = 1`;
- a valid `repository_id`;
- one `[discord]` workspace ID;
- named destination, mention, and inbound alias tables;
- positive content and metadata retention day values; and
- exact `auto_send` tuples, without wildcards or duplicate matches.

Destination channels are named by aliases. Mention targets use only `role:<id>`
or `user:<id>`. Inbound aliases must refer to a configured destination. Unknown
fields, duplicate aliases, wildcard policy values, secret-like keys, raw
destination fields, malformed IDs, and known cross-workspace references are
rejected without retaining the offending scalar values.

The example [`examples/repo-com.example.toml`](../examples/repo-com.example.toml)
is synthetic and safe to review. The configuration format is intended to remain
secret-free. Discord credentials do not belong in this file.

The normalized configuration is serialized deterministically and hashed with
SHA-256. The current policy, approval, eligibility, and delivery contracts use
these hashes to invalidate stale authority.

## Identity, secrets, and TLS

The current Discord adapters accept credentials only from
`REPO_COM_DISCORD_TOKEN`. The value must be a raw three-part dedicated Discord
bot token. It is held in zeroizing owned storage and sent only in the `Bot`
authorization scheme. User tokens, self-bots, bearer user authentication,
unlisted API versions, and untrusted non-loopback test endpoints are rejected.

The production Discord client uses the fixed Discord origin and Rustls-backed
`reqwest`; test-only constructors accept an origin-only loopback WireMock
endpoint. Setup validation performs read-only `GET` requests to inspect bot
identity, workspace membership, channel visibility, `VIEW_CHANNEL`,
`SEND_MESSAGES`, `READ_MESSAGE_HISTORY`, and resolved mention access. It does not
create applications, mutate permissions, edit messages, delete messages, or
change remote state.

The message adapter performs one `POST` create-message attempt and classifies
authentication, permission, validation, conflict, rate-limit, server, and
ambiguous post-dispatch outcomes. The inbound adapter performs bounded read-only
`GET` requests. No current command or UI exposes these operations to an
operator.

Do not add a token, authorization header, bot credential, private URL, real
message content, or private key to `.repo-com.toml`, examples, fixtures, logs, or
documentation. Token rotation is an operator responsibility; the adapters return
safe remediation but do not rotate credentials automatically.

## Storage and backups

### Location

The default database path is:

```text
<dirs::data_local_dir()>/repo-com/state.sqlite3
```

`database_path_in(root)` provides the same `repo-com/state.sqlite3` layout below
an explicit test or deployment root. The default operational path is outside the
repository; an explicit `open_path` call is caller-controlled and must not place
live state in the repository.

### Protection

On Unix, the state crate creates the parent directory with mode `0700` and the
database with mode `0600`, and it refuses a symbolic-link database path. On
Windows, the implementation relies on the inherited user-profile ACL boundary;
it does not rewrite Windows ACLs through portable permission bits.

The database is not encrypted by this implementation. Local account access,
backups, filesystem snapshots, and other copies can expose retained content.
Protect copies with equivalent access controls and avoid uploading them to
shared or remote storage.

### Schema and migrations

The current schema version is `1`. Migration 1 creates repository,
draft/revision, approval, policy, delivery-attempt, inbound, acknowledgement,
archive, reply-link, and append-only audit tables. Database triggers protect
immutable draft revisions, first inbound snapshots, and audit events.

Migrations are forward-only. The state store rejects a newer schema, preserves
corrupt or migration-conflicting bytes, and never automatically deletes or
recreates a database. SQLite `user_version`, required tables, triggers, and
foreign keys are verified after opening.

SQLite WAL mode creates `-wal` and `-shm` sidecar files while a database is
active. Do not treat the main file alone as a complete live backup. Coordinate a
safe copy with the embedding application and SQLite's backup/checkpoint semantics;
this repository does not provide a backup command.

## Operations and health checks

The current local validation gates are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
```

The `StateStore` API exposes read-only checks for `schema_version()`,
`foreign_keys_enabled()`, `journal_mode()`, `quick_check()`, linked runtime
information through `sqlite_runtime()` and `assert_sqlite_runtime()`, and
user-only database inspection. Mutating convenience methods use explicit
transactions and roll back on typed failure.

Audit queries are read-only, repository-scoped, bounded to 100 rows per page,
and expose a stable continuation cursor. Redaction is reapplied when evidence is
read. There is no monitoring daemon, crash upload, analytics, remote audit
synchronization, or Discord health-check command in the current workspace.

### Network validation boundary

The Discord setup, message, and inbound contracts have token-free WireMock
coverage for REST v10 paths, bot authentication, permissions, rate-limit
metadata, message validation, and inbound filtering. That evidence proves the
mocked HTTP contract only. It does not prove a live Discord workspace, current
Discord policy, production TLS path, or human operator acceptance.

If an embedding application invokes a production adapter, treat the network
result as an external side effect. Keep unknown delivery blocked, use the
read-only reconciliation predicates, and do not treat a local duplicate-safe
claim as transport-level exactly-once delivery.

### Retention and lifecycle

The retention crate defaults to 30 days for content and 365 days for metadata.
Validated overrides allow content from 1–365 days and metadata from 30–3,650
days, with metadata retention never shorter than content retention. Sweeps are
repository-scoped and transactional: expired content becomes
`[content-expired]`, later metadata rows may be removed, and count-only audit
summaries are written.

A failed sweep blocks a new state mutation. The current workspace does not
implement a confirmed purge plan/execution API, a purge command, or a read-only
lifecycle inspection command. Do not describe those planned surfaces as
available.

## Upgrades and rollback

There is no packaged upgrade or rollback procedure in version `0.1.0`. When
changing the workspace:

1. Preserve the database and its active WAL sidecars using a safe SQLite-aware
   backup procedure.
2. Run formatting, clippy, and the full current test suite.
3. Verify the migration and linked SQLite version before opening operational
   state.
4. Keep the previous source or binary available for diagnosis, but do not assume
   an older implementation can open a newer schema.
5. Re-run link, secret, and documentation checks for any operator-facing change.

The release artifact, checksum, SBOM, cross-platform matrix, and migration
compatibility policy remain future release work.

## Troubleshooting

| Symptom | Likely cause | Next action |
|---|---|---|
| `config-not-found` | No configuration in the bounded repository search | Add a valid `.repo-com.toml` or pass a path inside the root |
| `multiple-config-candidates` | More than one configuration in the ancestor range | Remove the extra file or use an explicit path |
| `unsupported-schema-version` | Configuration is not version 1 | Update the document using the [example](../examples/repo-com.example.toml) |
| `secret-field` or `raw-destination-field` | A forbidden key or raw destination was supplied | Remove the key and use a named alias; do not paste the value into a report |
| `operator-action-required` or `TtyRequired` | An authority-creating action lacks explicit TTY confirmation | Supply confirmation only after the complete exact preview is shown; never self-approve in automation |
| `policy-blocked` | Policy is not exact, current, or uniquely activated | Inspect current hashes and status, then perform a fresh operator activation if appropriate |
| `authentication-failed` | Discord rejected the dedicated bot token | Rotate the token in the operator environment and rerun setup; never use a user token |
| `permission-denied` | The bot lacks a required Discord permission or resource | Review the read-only setup remediation and grant only the required access manually |
| `connectivity-rate-limit` | A request failed, was rate-limited, or returned an ambiguous result | Honor dynamic rate-limit metadata; reconcile unknown delivery before any resend |
| `storage-integrity` | SQLite is busy, corrupt, unsupported, or migration failed | Preserve the database and sidecars, inspect the typed error, and do not delete the database |
| `UnsupportedSqliteRuntime` | Linked SQLite is below `3.53.4` for a release open | Use an approved release runtime or remain in development mode for contract tests |
| Retention sweep failure | A local retention transaction could not commit | Preserve state, resolve the typed storage error, and do not continue the blocked mutation |

## Security and privacy checklist

- [ ] Keep `.repo-com.toml` free of tokens, passwords, private keys, authorization
  values, and credential-bearing URLs.
- [ ] Provide a raw dedicated bot token only through `REPO_COM_DISCORD_TOKEN`.
- [ ] Keep the state database and WAL sidecars outside the repository and out of
  shared storage.
- [ ] On Unix, verify owner-only directory and file permissions; on Windows,
  verify the inherited user-profile ACL.
- [ ] Back up unencrypted state only to locations with equivalent access control.
- [ ] Treat inbound text, mentions, edits, and deletions as untrusted data.
- [ ] Never interpret policy or eligibility as proof that a remote send occurred.
- [ ] Keep unknown delivery blocked until read-only reconciliation resolves it.
- [ ] Use the high-level redacted audit writer for lifecycle evidence; do not pass
  untrusted message content through lower-level state metadata APIs.
- [ ] Remember that no current implementation provides encryption at rest,
  automatic repair, purge, terminal UI, or human release sign-off.
- [ ] Do not upload state, audit records, or configuration to telemetry or
  synchronization services.

## Recovery and support

- **Stale or ambiguous policy:** inspect the current configuration and tuple
  hashes. A stale activation grants no eligibility; resolve ambiguity under
  operator control before activating again.
- **Changed draft or destination:** create a new preview and approval. Approval
  is intentionally invalidated by any relevant revision, configuration, alias,
  expiry, or scan change.
- **Unknown delivery:** retain the exact destination, bot author, nonce, and
  content evidence. Perform read-only reconciliation; do not resend an unresolved
  attempt.
- **Inbound page or cursor failure:** preserve the previous local state and avoid
  treating a page commit as proof that later point reconciliation completed.
- **Corrupt or unsupported state:** preserve the database and sidecars, inspect
  the typed state error, and avoid deletion or recreation.
- **Configuration mistake:** use the safe error code and field path; do not
  include original scalar values in logs or issue reports.
- **Discord setup failure:** review the structured remediation for bot identity,
  workspace membership, channel visibility, permissions, and mention access.
  Setup remains read-only and does not mutate the workspace.

## Further documentation

- [Library Consumer Guide](user-guide.md)
- [Architecture Decision Records](adr/README.md)
- [Changelog](../CHANGELOG.md)
- [Unreleased release notes](releases/unreleased.md)
