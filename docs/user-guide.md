# User Guide

> **Status:** the current source snapshot exposes a composed `repo-com`
> executable at workspace version `0.1.0`. The snapshot is dated 2026-09-25;
> there is no Git tag, published release date, or public installer in this
> checkout. This guide describes implemented commands and known limitations,
> not a human or live-service approval.

## Overview

Use `repo-com` when a repository workflow needs to send one focused Discord
message through a named destination, or when it needs to inspect and continue a
local conversation. The command is intentionally explicit: the caller supplies
the repository, draft, revision, destination alias, and any cursor, object, or
time boundary. There is no default destination.

A typical path is:

```text
validate configuration
  -> read-only Discord setup check
  -> create and preview an immutable draft
  -> approve the exact revision or activate one exact policy tuple
  -> send and interpret the local delivery outcome
  -> fetch untrusted replies or mentions
  -> create a reply draft and acknowledge/archive local items
  -> inspect audit evidence or perform a confirmed local purge
```

The current executable is source-buildable:

```bash
cargo build --release --locked --package command_routing_contract --bin repo-com
./target/release/repo-com --version
```

The version command prints `0.1.0`. On Windows, use
`target\release\repo-com.exe`. The release workflow defines an explicit archive
installation policy, but this source snapshot does not contain a published
installer or archive.

## Before you begin

### Configure one repository

Create a secret-free `.repo-com.toml` at the repository root, using
[`examples/repo-com.example.toml`](../examples/repo-com.example.toml) as a
shape reference. The repository root is the nearest ancestor containing `.git`.
Configuration discovery searches from the current directory through that root
for exactly one `.repo-com.toml`; an explicit `--config` path must remain
inside the root.

The file contains schema `1`, one workspace, named destination aliases, named
mention aliases, enabled inbound aliases, retention periods, and exact
`auto_send` tuples. It never contains the Discord token. See the
[configuration contract](configuration.md) for field-level rules and rejected
unknown or secret-like fields.

### Provide the Discord token only when needed

Local configuration, state, audit, and purge operations do not need a network
credential. `setup-check`, `send`, and `inbox.fetch` use the production Discord
adapter and read a raw dedicated bot token from:

```text
REPO_COM_DISCORD_TOKEN='<raw dedicated bot token>'
```

The angle-bracketed text is a placeholder. The token must be supplied through
the process environment, not a command argument, TOML file, source file, log,
fixture, or support report. User tokens, self-bots, bearer user authentication,
and arbitrary endpoints are unsupported. Follow [Discord setup](discord-setup.md)
for the dedicated bot and manual permissions.

## First successful workflow

The examples below use the synthetic `acme/widgets` repository and a fixed
future timestamp so the protocol pair is internally consistent. Replace those
values with your own identifiers and current time before use.

### 1. Validate configuration

The command reads one strict JSON envelope from standard input:

```bash
repo-com --config .repo-com.toml --output json config validate <<'JSON'
{"protocol_version":1,"command":"config.validate","input":{"repository_id":"acme/widgets"}}
JSON
```

A successful result reports the resolved configuration hash, schema version,
aliases, and exact auto-send count. It is local validation only: it does not
grant a Discord permission or prove current workspace access.

### 2. Check Discord setup

After creating a dedicated bot and manually granting least privilege, run the
read-only setup check:

```bash
repo-com --config .repo-com.toml --output json setup-check <<'JSON'
{"protocol_version":1,"command":"setup-check","input":{"repository_id":"acme/widgets"}}
JSON
```

Review bot identity, workspace membership, channel checks, mention checks, and
structured remediation. The command does not invite a bot, change permissions,
or mutate a Discord message. A `ready` report is a point-in-time result. It does
not prove that a teammate saw a later message or that a release is approved.

### 3. Create a draft

Use a named destination alias. The first state-mutating draft command registers
the repository in local state; `config.validate` alone does not do that.

```bash
repo-com --config .repo-com.toml --output json draft create <<'JSON'
{"protocol_version":1,"command":"draft.create","input":{"repository_id":"acme/widgets","draft_id":"draft-1","destination_alias":"release","text":"synthetic build failure example","event_type":"build_failed","severity":"high","created_at":"2099-01-01T00:00:00Z","created_at_unix_seconds":4070908800,"expires_in_seconds":3600,"metadata":{}}}
JSON
```

Record the returned `draft_id` and positive `revision`. Draft revisions are
immutable. Use `draft.update` to create a new revision; it cannot rewrite an
already accepted remote message.

### 4. Preview the exact revision

```bash
repo-com --config .repo-com.toml --output json draft preview <<'JSON'
{"protocol_version":1,"command":"draft.preview","input":{"repository_id":"acme/widgets","draft_id":"draft-1","revision":1}}
JSON
```

Read the complete labeled or JSON preview: destination, exact rendered text,
metadata, event type, severity, expiry, revision hash, and next action. The
renderer adds a deterministic delivery nonce to the final Discord text. Do not
edit that text after approval or eligibility checks.

If the secret scanner reports a finding, send remains blocked by default. A
real-TTY `draft.secret-override` can be used only after reviewing the exact
preview, and the prompt expects `override <preview-hash>`. The override is
redacted and audited; it is not a general bypass or a complete DLP control.

### 5A. Approve the exact revision

`draft.approve` is an authority-creating action. It requires an interactive
TTY, shows the complete approval preview, and expects the exact response:

```text
approve <preview-hash>
```

`N`, `no`, `Esc`, or Enter cancels. A short `yes` is not a substitute for the
exact hash response. The final executable reads structured JSON from standard
input before it reaches the prompt; use a real terminal/PTY-capable invocation
and finish the structured-input read before entering the confirmation. Do not
pipe a TTY-only command. Non-TTY callers receive `operator-action-required` and
cannot create approval.

The command is:

```bash
repo-com --config .repo-com.toml draft approve
```

Supply the `draft.approve` envelope with `repository_id`, `draft_id`, and
positive `revision`, then answer the prompt with the exact preview hash. The
domain revalidates the preview, configuration, revision, expiry, and current
state before recording approval.

### 5B. Or activate one exact policy

For a deliberately automated path, inspect the exact tuple first:

```bash
repo-com --config .repo-com.toml --output json policy status <<'JSON'
{"protocol_version":1,"command":"policy.status","input":{"repository_id":"acme/widgets","event_type":"build_failed","destination_alias":"release","severity":"high"}}
JSON
```

Activation is separate from the `auto_send` declaration and requires an
interactive TTY. The prompt asks for the exact values shown in its preview:

```text
activate <repository> <activation-id> <config-hash> <tuple-hash> <event-type>/<destination-alias>/<severity>
```

Use `policy.activate` with the exact `repository_id`, `event_type`,
`destination_alias`, `severity`, `activated_at`, and optional
`activation_id`. A wildcard, raw channel, stale hash, ambiguous activation, or
missing repository state blocks the decision. There is no current shell command
for policy deactivation; inspect status and leave stale or ambiguous state
blocked, or use a separately reviewed embedding owner.

### 6. Send one exact revision

With a current approval or exact active policy, send the same immutable
revision:

```bash
repo-com --config .repo-com.toml --output json send <<'JSON'
{"protocol_version":1,"command":"send","input":{"repository_id":"acme/widgets","draft_id":"draft-1","revision":1}}
JSON
```

The command revalidates the current configuration, revision, expiry,
destination and mention allowlist, authority, and secret scan. The local claim
and audit transition commit before the one-attempt HTTP request. The result may
be:

- `accepted`: a validated remote message identifier was returned;
- `failed`: a definitive rejection or non-retryable failure was recorded;
- `retry-wait`: a bounded next attempt is recorded, but the current executable
  does not run that retry automatically;
- `unknown`: dispatch may have reached Discord and the result is not known;
- `reconciled-accepted` or `reconciled-absent`: a read-only reconciliation
  decision was supplied by the domain owner; or
- `unresolved`: evidence is conflicting or insufficient.

`accepted` is not a read receipt. The product does not provide response
analytics or proof that a teammate noticed a message. The final executable
does not expose a separate reconciliation command; see
[Unknown-delivery recovery](#unknown-delivery-recovery).

## Inbound replies and mentions

### Fetch a bounded page

`inbox.fetch` requires an enabled named inbound alias and exactly one boundary:
either a cursor or an RFC 3339 time value. It also requires the dedicated bot
user ID used as untrusted mention context.

```bash
repo-com --config .repo-com.toml --output json inbox fetch <<'JSON'
{"protocol_version":1,"command":"inbox.fetch","input":{"repository_id":"acme/widgets","alias":"release","time":"2098-01-01T00:00:00Z","bot_user_id":"123456789012345678"}}
JSON
```

The fetch is bounded to 10 pages, 1,000 raw messages, and 100 point checks.
Continuation metadata is explicit. The returned envelopes are marked
`untrusted`; attachment data is represented by indicators, not downloaded bytes.
A stored first snapshot, a later current snapshot, and edit/delete transitions
are separate local facts. A stored snapshot is not current remote truth.

The library filter can retain human replies to accepted local deliveries or
direct mentions of the configured bot. The current final-binary composition
passes an empty accepted-delivery correlation list to the fetcher, so a reply
without a direct bot mention may not be retained by the CLI path. Treat that as
a current implementation gap, not as a guarantee of reply coverage.

### Acknowledge or archive locally

```bash
repo-com --config .repo-com.toml --output json inbox acknowledge <<'JSON'
{"protocol_version":1,"command":"inbox.acknowledge","input":{"repository_id":"acme/widgets","item_ids":["300000000000000002"],"at":"2099-01-01T00:00:01Z"}}
JSON
```

Use `inbox.archive` with the same explicit fields for archival. Both actions
are idempotent local transitions. They do not react, edit, delete, assign, or
otherwise mutate Discord.

### Create a reply draft

`reply.draft-create` validates a stored inbound item and creates a new
immutable linked draft. It never sends directly and never bypasses preview,
approval, policy, safety, or delivery checks:

```bash
repo-com --config .repo-com.toml --output json reply draft-create <<'JSON'
{"protocol_version":1,"command":"reply.draft-create","input":{"repository_id":"acme/widgets","inbound_item_id":"300000000000000002","draft_id":"reply-1","text":"synthetic reply draft example","event_type":"build_failed","severity":"high","created_at":"2099-01-01T00:00:02Z","created_at_unix_seconds":4070908802,"expires_in_seconds":3600}}
JSON
```

Review and send the returned reply through the normal draft lifecycle.

## Unknown-delivery recovery

An `unknown` outcome must remain blocked. **There is no automatic resend.**

1. Preserve the exact configured destination, bot author, deterministic
   content nonce, intended text, draft/revision, and attempt identifier from
   local output or audit evidence.
2. Perform read-only reconciliation through the delivery-recovery domain
   contract. Search only the configured destination and require the configured
   bot author, exact nonce, and exact content. A complete, conservative absence
   window with multiple successful reads is a different result from an
   incomplete read.
3. Keep conflicting, incomplete, or unresolved evidence unresolved. Do not infer
   current remote state from a local snapshot.
4. A later new attempt requires a fresh, explicit operator-authorized decision
   after reconciliation. The current command tree has no `reconcile` or retry
   command, so do not invent one or treat `send` as a safe retry.

The domain implementation distinguishes a local `unknown` record from a later
reconciled result. That distinction is durable local evidence, not a claim
that a teammate read or replied to the outbound message.

## Local inspection, retention, and purge

### Audit evidence

`audit.query` is read-only, repository-scoped, redacted, and bounded. Supply
exactly one `page_size` or `limit` from 1 through 100. Optional filters include
`occurred_from`, `occurred_before`, `object_type`, `object_id`, `transition`,
and the returned repository-scoped `cursor`.

```bash
repo-com --config .repo-com.toml --output json audit query <<'JSON'
{"protocol_version":1,"command":"audit.query","input":{"repository_id":"acme/widgets","page_size":50}}
JSON
```

Audit results do not export state, contact Discord, or provide read receipts or
response analytics.

### State verification and lifecycle inspection

`state.verify` is read-only and requires an existing `database_path` in its
JSON input. It checks quick-check status, foreign keys, migration version,
repository scope, and filesystem permissions without creating, repairing, or
migrating the database.

`state inspect` (also exposed as `lifecycle inspect`) requires an explicit
`object_type`, one `page_size` or `limit` from 1 through 100, and the applicable
`object_id`/`revision`. Supported object families are `repository`, `draft`,
`draft_revision`, `delivery_attempt`, `inbound_item`, `acknowledgement`,
`archive`, `reply_link`, and `audit_transition`. `include_retained_content` is
an explicit opt-in. A retained-content inspection still returns local data, not
current remote state.

### Retention

Configured retention defaults are 30 days for draft/inbound content and 365
days for non-content metadata. Valid content overrides are 1 through 365 days;
metadata overrides are 30 through 3,650 days, and metadata retention cannot be
shorter than content retention. The retention service replaces expired content
with `[content-expired]` and can remove later metadata.

The current final executable has no `retention.sweep` command and does not claim
that an unexposed service runs automatically. Use an embedding application or a
separately reviewed local process for retention sweeps. Retention is local and
does not delete Discord messages.

### Confirmed local purge

1. Run `purge.plan` with `scope` equal to `content`, `metadata`, or `all`, and
   exactly one canonical cutoff representation (`cutoff`, or the agreeing
   `cutoff_unix_seconds` plus `cutoff_utc`). The plan is non-mutating and
   includes counts, a state fingerprint, `config_hash`, and `plan_hash`.
2. Review `execution_performed: false` and the complete scope.
3. Run `purge.execute` in a real TTY with the same scope, cutoff, hashes, and
   `executed_at`. The prompt expects:

   ```text
   purge <repository> <scope> <cutoff-unix> <config-hash> <plan-hash>
   ```

4. If any fact changes, discard the plan and make a new one. A purge never
   edits, deletes, or otherwise mutates a Discord message.

The purge is a local transaction. Keep a copy of local state until the result
and count-only audit event have been reviewed; a local purge cannot revoke
copies in a backup or filesystem snapshot.

## Protocol and command reference

### Streams and global options

Machine mode uses one strict outer object:

```json
{"protocol_version":1,"command":"<canonical-command>","input":{}}
```

Every outcome has `protocol_version`, `status`, `data`, and `error`. JSON mode
writes exactly one object to `stdout`; diagnostics are opt-in stderr values.
Human mode is labeled text.

| Option | Values | Default | Meaning |
|---|---|---|---|
| `--config PATH` | one path | discovered `.repo-com.toml` | Explicit in-repository configuration path |
| `--output FORMAT` | `human`, `json` | `human` | Human text or one protocol object |
| `--color COLOR` | `auto`, `always`, `never` | `auto` | Color is supplemental; `NO_COLOR` is honored |
| `--diagnostics MODE` | `off`, `on` | `off` | Opt-in diagnostic stream |
| `--state PATH` | one path | OS user-data path | Explicit local SQLite path for stateful commands |
| `--version` | flag | — | Print only the package semantic version |
| `--tty` / `--non-tty` | flags | detected | Explicit stream decision; non-TTY fails closed for authority actions |

`--state` does not replace the required `database_path` field of
`state.verify`; that command deliberately verifies the explicit existing path.

### Canonical commands and input fields

| Command | Required explicit fields |
|---|---|
| `config.validate` | `repository_id`; optional `config_path` |
| `policy.status` | `repository_id`, `event_type`, `destination_alias`, `severity` |
| `policy.activate` | `repository_id`, `event_type`, `destination_alias`, `severity`, `activated_at`; optional `activation_id` |
| `state.verify` | `repository_id`, `database_path`; optional `expected_migration` |
| `lifecycle.inspect` | `repository_id`, `object_type`, one `page_size` or `limit`; conditional `object_id`/`revision`; optional `after`, `include_retained_content` |
| `audit.query` | `repository_id`, one `page_size` or `limit`; optional time/object filters and `cursor` |
| `purge.plan` | `repository_id`, `scope`, one cutoff representation; optional `expected_config_hash` |
| `purge.execute` | `repository_id`, `scope`, one cutoff representation, `config_hash`, `plan_hash`, `executed_at` |
| `draft.create` | `repository_id`, `draft_id`, `destination_alias`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional `metadata`, `expires_in_seconds` |
| `draft.show` | `repository_id`, `draft_id`, positive `revision` |
| `draft.update` | `draft.show` identity plus new destination/text/event/severity/timestamps; optional metadata and expiry |
| `draft.preview` | `repository_id`, `draft_id`, positive `revision` |
| `draft.approve` | `repository_id`, `draft_id`, positive `revision` |
| `draft.secret-override` | `repository_id`, `draft_id`, positive `revision` |
| `send` | `repository_id`, `draft_id`, positive `revision` |
| `setup-check` | `repository_id` |
| `inbox.fetch` | `repository_id`, `alias`, `bot_user_id`, exactly one `cursor` or `time`; optional `retrieved_at` |
| `inbox.acknowledge` / `inbox.archive` | `repository_id`, nonempty `item_ids`, `at` |
| `reply.draft-create` | `repository_id`, `inbound_item_id`, `draft_id`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional metadata and expiry |

Unknown envelope and input fields fail closed. The selected shell route must
match the JSON `command`; no hidden repository, revision, destination, cursor,
or object default is supplied.

## Accessibility and safe terminal use

Human output is linear and labeled for screen readers and keyboard use. The
renderer supports an 80-column minimum, wraps rather than truncating security
or approval facts, and carries state, destination, revision, authority,
safety, outcome, and next action in text. `NO_COLOR` and `--color never` produce
plain text; meaning never depends on color or position. Prompts are keyboard
reachable and do not run in non-TTY mode. Machine stdout and stderr remain
separate.

The contract tests check these mechanical properties. They do not replace a
human accessibility or usability review.

## Troubleshooting

| Symptom | Likely cause | Safe next action |
|---|---|---|
| `config-not-found` | No `.repo-com.toml` in the bounded search | Create one at the repository root or pass an in-root `--config` path |
| `multiple-config-candidates` | More than one ancestor configuration | Remove the extra file or select one explicit path |
| `unsupported-schema-version` | Configuration is not schema 1 | Compare with the example and update the document |
| `secret-field` / `raw-destination-field` | Forbidden configuration shape | Move the value out of configuration and use named aliases |
| `operator-action-required` | Approval, policy, secret override, or purge requested without a TTY | Run the human prompt in a real terminal; never self-approve in automation |
| `policy-blocked` | Missing, stale, ambiguous, or changed exact authority | Inspect status, correct the tuple/configuration, and create a fresh decision |
| `authentication` | Dedicated bot credential rejected | Rotate `REPO_COM_DISCORD_TOKEN` in the Developer Portal and retry setup |
| `permission` | Discord access is insufficient | Apply only the reported manual channel permission and rerun setup |
| `unknown-delivery` | Dispatch may have taken effect | Keep it blocked and perform read-only reconciliation; do not resend |
| `connectivity-rate-limit` | Bounded request or dynamic Discord limit | Honor server-provided delay metadata; do not guess a reset time |
| `storage-integrity` | SQLite, migration, lock, permission, or transaction problem | Preserve the database and WAL sidecars; do not delete or recreate state |
| `usage-schema` | Invalid JSON, command mismatch, missing identifier, or invalid bound | Correct the explicit envelope/input and retry |
| `expired` / `stale-authority` | Draft or approval is no longer current | Create a new revision or obtain fresh exact authority |

## Further help

- [Administrator Guide](admin-guide.md)
- [Operator guide](operator-guide.md)
- [Configuration contract](configuration.md)
- [Discord setup](discord-setup.md)
- [Security model](security-model.md)
- [Threat model](threat-model.md)
- [Changelog](../CHANGELOG.md)
- [`0.1.0` source snapshot release notes](releases/0.1.0.md)
- [Unreleased release notes](releases/unreleased.md)

Automated documentation checks support review; they do not constitute human
approval, live Discord acceptance, compliance certification, or release
sign-off.
