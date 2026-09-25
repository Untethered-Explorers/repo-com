# Operator guide

This guide owns the exact `repo-com` installation boundary, command tree,
protocol-version-1 input/output contract, operator confirmations, delivery
outcomes, recovery rules, accessibility behavior, and unsupported behavior. The
implemented sources are `crates/repo-com-cli`, the two command-handler crates,
the terminal renderer/prompt crates, and the state and domain services they
compose.

## Installation status

There is **no installed `repo-com` binary** in the published release boundary of
this source snapshot. The repository does contain the final executable target,
so an operator can build it explicitly:

```bash
rustup toolchain install 1.98.1
cargo build --release --locked --package command_routing_contract --bin repo-com
./target/release/repo-com --version
```

The command prints the workspace/package version `0.1.0`. The source snapshot
was inspected on 2026-09-25; no Git tag, published archive, installer, or
release date is recorded. The tag-driven release workflow is packaging policy,
not evidence that an artifact has been published or that a release is approved.
The required toolchain is Rust 1.98.1.

Use human output for commands that request a TTY confirmation. Use
`--output json` for non-interactive commands whose structured result is being
consumed by automation. The current process reads its JSON input from standard
input before dispatch; a TTY confirmation therefore needs a real terminal or a
PTY-capable wrapper, not a shell pipe that makes stdin non-interactive.

## Protocol version 1 and streams

Every command except `--help` and `--version` reads one strict protocol-version-1
outer object from standard input. The only accepted value is
`protocol_version: 1`:

```json
{
  "protocol_version": 1,
  "command": "config.validate",
  "input": {
    "repository_id": "acme/widgets"
  }
}
```

The outer object and each command input reject unknown fields. The selected
shell route and the JSON `command` must agree. There is no default repository,
destination, revision, cursor, time boundary, inbound item, or purge scope.

In machine mode, `stdout` contains **exactly one JSON object**. Diagnostics are
optional values on `stderr`; prompts must not be placed in machine stdout.
Every outcome contains the four protocol fields `protocol_version`, `status`,
`data`, and `error`:

```json
{"protocol_version":1,"status":"success","data":{},"error":null}
```

```json
{"protocol_version":1,"status":"error","data":null,"error":{"code":"usage-schema","message":"safe detail"}}
```

Human mode emits linear labeled text. The output contract supports a minimum of
80 columns, wraps long hashes and values, and never relies on color alone.
`NO_COLOR` and `--color never` produce plain text.

## Global options and exits

| Option | Accepted values | Default | Boundary |
|---|---|---|---|
| `--config PATH` | one explicit path | discovered `.repo-com.toml` | Normalized path must stay inside the detected repository root |
| `--output FORMAT` | `human`, `json` | `human` | `json` selects exactly one protocol object |
| `--color COLOR` | `auto`, `always`, `never` | `auto` | `never` and `NO_COLOR` select plain text |
| `--diagnostics MODE` | `off`, `on` | `off` | Opt-in diagnostics use the diagnostic stream |
| `--state PATH` | one explicit SQLite path | OS user-data path | Used by stateful commands; `state.verify` still requires `database_path` in JSON |
| `--tty` | flag | detected | Requests TTY mode but cannot manufacture a TTY |
| `--non-tty` | flag | detected | Explicitly selects non-interactive mode |
| `--version` | flag | — | Prints only the semantic package version |

TTY is the safer default for automation: if either relevant stream is
non-interactive, an authority-creating prompt fails with
`operator-action-required`. A prompt is never created in non-TTY mode.

Stable error categories and process exits are:

| Error code | Exit |
|---|---:|
| `usage-schema` | 2 |
| `operator-action-required` | 3 |
| `policy-blocked` | 4 |
| `authentication` | 5 |
| `permission` | 6 |
| `remote-conflict` | 7 |
| `unknown-delivery` | 8 |
| `storage-integrity` | 9 |
| `connectivity-rate-limit` | 10 |
| `internal-failure` | 1 |

A successful command exits 0. These categories are process contracts, not
claims that a remote action took effect.

## Canonical command surface

The dotted names are canonical protocol values. The shell parser also accepts
the nested, kebab, and snake spellings implemented by the command tree; the
canonical JSON command should be used by automation.

### Operations commands

| Command | Explicit input |
|---|---|
| `config.validate` | `repository_id`; optional `config_path` |
| `policy.status` | `repository_id`, `event_type`, `destination_alias`, `severity` |
| `policy.activate` | `repository_id`, `event_type`, `destination_alias`, `severity`, `activated_at`; optional `activation_id` |
| `state.verify` | `repository_id`, `database_path`; optional `expected_migration` |
| `lifecycle.inspect` | `repository_id`, `object_type`, one `page_size` or `limit`; conditional `object_id`, `revision`; optional `after`, `include_retained_content` |
| `audit.query` | `repository_id`, one `page_size` or `limit` from 1 through 100; optional time/object filters and repository-scoped `cursor` |
| `purge.plan` | `repository_id`, `scope` (`content`, `metadata`, or `all`), exactly one cutoff representation; optional `expected_config_hash` |
| `purge.execute` | `repository_id`, `scope`, exactly one cutoff representation, `config_hash`, `plan_hash`, `executed_at` |

`lifecycle.inspect` object types are `repository`, `draft`, `draft_revision`,
`delivery_attempt`, `inbound_item`, `acknowledgement`, `archive`, `reply_link`,
and `audit_transition`. `repository` and `audit_transition` do not take an
object ID. `draft_revision` requires a positive `revision`; other object
families require the applicable `object_id`. `page_size` and `limit` are
mutually exclusive. `include_retained_content` is explicit opt-in.

A purge cutoff is either the single canonical `cutoff` RFC 3339 value or the
agreeing `cutoff_unix_seconds` and `cutoff_utc` pair. Purge hashes are SHA-256
hex values. The current command tree has no retention-sweep command; retention
is a domain service described below.

### Messaging commands

| Command | Explicit input |
|---|---|
| `draft.create` | `repository_id`, `draft_id`, `destination_alias`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional `metadata`, `expires_in_seconds` |
| `draft.show` | `repository_id`, `draft_id`, positive `revision` |
| `draft.update` | `draft.show` identity plus new destination, text, event type, severity, timestamps, metadata, and optional expiry |
| `draft.preview` | `repository_id`, `draft_id`, positive `revision` |
| `draft.approve` | `repository_id`, `draft_id`, positive `revision` |
| `draft.secret-override` | `repository_id`, `draft_id`, positive `revision` |
| `send` | `repository_id`, `draft_id`, positive `revision` |
| `setup-check` | `repository_id` |
| `inbox.fetch` | `repository_id`, enabled `alias`, `bot_user_id`, exactly one `cursor` or `time`; optional `retrieved_at` |
| `inbox.acknowledge` | `repository_id`, nonempty `item_ids`, `at` |
| `inbox.archive` | `repository_id`, nonempty `item_ids`, `at` |
| `reply.draft-create` | `repository_id`, `inbound_item_id`, `draft_id`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional metadata and expiry |

Draft metadata accepts only the bounded fields `repository_label`, `branch`,
and `commit`. `destination_alias` is a configured alias, never a raw Discord
channel. `inbox.fetch` requires exactly one cursor or RFC 3339 time boundary;
`bot_user_id` is untrusted filter context, not proof of the caller's identity.
All timestamps must be valid and, when a Unix pair is supplied, must identify
the same instant.

## Operator procedures

### 1. Validate the resolved configuration

1. Work from the repository root or pass an explicit in-repository `--config`.
2. Run `config.validate` with the exact `repository_id`.
3. Read the returned schema version, configuration hash, aliases, and exact
   auto-send count.
4. Correct the safe field path on failure. Do not bypass validation with a raw
   destination.

A valid configuration is local evidence only. It does not contact Discord,
grant a permission, or prove a live workspace.

### 2. Run the read-only Discord setup check

Set the raw dedicated bot token only in `REPO_COM_DISCORD_TOKEN`, then run
`setup-check`. The report includes bot identity, workspace membership,
alias-based channel checks, mention checks, required/granted/missing
permissions, issues, and safe remediation. The check uses REST v10 GETs and
never changes Discord configuration.

The channel checks cover `VIEW_CHANNEL`, `SEND_MESSAGES`, and, for enabled
inbound aliases, `READ_MESSAGE_HISTORY`. Role mention checks also consider
`MENTION_ROLES` and mentionability. Apply missing grants manually, then rerun
the check. Follow [`discord-setup.md`](discord-setup.md) for bot creation,
least privilege, and token rotation.

### 3. Create and preview a draft

1. Create one revision with `draft.create` and a named destination.
2. Use `draft.show` to inspect the returned exact revision.
3. Use `draft.preview` for the same repository, draft, and revision.
4. Read the complete exact text, metadata, destination, expiry, event type,
   severity, and next action.
5. Do not send until the preview and safety decision are current.

Revisions are immutable. `draft.update` must replace the current revision and
creates a new one. Draft expiry defaults to 24 hours and is capped at seven
days. The pre-send secret scanner checks rendered text and bounded metadata. A
finding blocks eligibility unless an exact TTY `draft.secret-override` is
recorded after reviewing the complete preview. Its prompt expects
`override <preview-hash>`. The override is redacted and audited; it is not a
general bypass or complete data-loss prevention.

### 4. Establish exact authority

For normal per-draft approval, run `draft.approve` in a real TTY. The prompt
renders the complete preview and expects:

```text
approve <preview-hash>
```

A short `yes` does not satisfy the exact-hash grammar. The approval is bound to
the exact repository, revision, text, metadata, destination, configuration,
expiry, and safety facts. It expires at the earlier of draft expiry and 15
minutes after approval. A changed fact requires a new preview and decision.

For a deliberately automated path, run `policy.status` first. An `auto_send`
entry in TOML is only a declaration; it is not an activation. `policy.activate`
requires an interactive TTY and the exact response shown by its preview:

```text
activate <repository> <activation-id> <config-hash> <tuple-hash> <event-type>/<destination-alias>/<severity>
```

Activation is equality-only. No wildcard, prefix, broader severity, raw
channel, or implicit default is eligible. A changed configuration/tuple makes
the activation stale; ambiguous active rows deny the decision. Non-TTY mode
cannot create approval, activate policy, or record a secret override.

### 5. Send and interpret the outcome

`send` accepts only `repository_id`, `draft_id`, and positive `revision`. The
binary revalidates current configuration, destination resolution, mention
allowlist, revision hash, expiry, approval or exact policy basis, and secret
scan. The local claim, attempt, and audit transition commit before one HTTP
message attempt.

The local persisted delivery states are:

- `unclaimed`;
- `claimed`;
- `accepted`;
- `failed`;
- `retry_wait`;
- `unknown`;
- `reconciled_accepted`;
- `reconciled_absent`; and
- `unresolved`.

Human delivery labels include `accepted`, `failed`, `retry-wait`, `unknown`,
`reconciled-accepted`, `reconciled-absent`, and `unresolved`, plus explicit
eligibility, expiry, stale-authority, and error views. The current executable
records a bounded retry wait but does not automatically perform the next
attempt. The retry service contract limits transport attempts to three and caps
Discord-directed waits at 30 seconds, but no retry command is exposed here.

`accepted` means the transport returned a validated message identifier. It does
not mean a teammate read the message. The product has no read receipt and no
response analytics. It does not edit or delete an accepted remote message.

### 6. Recover an unknown delivery without duplication

If the outcome is `unknown`, stop the normal send loop. There is **no automatic
resend**.

1. Preserve the exact configured destination, bot author, deterministic content
   nonce, exact intended content, draft/revision, and attempt identifier.
2. Use the read-only delivery-recovery contract to search only that destination
   and require all exact predicates: configured bot author, nonce, and content.
3. Treat one exact match as reconciled acceptance. A complete conservative
   absence window with three successful reads can become reconciled absence.
4. Keep incomplete, conflicting, or unresolved evidence unresolved. An
   incomplete read is not proof of absence.
5. Require a fresh, explicit operator-authorized decision before any new
   transport attempt.

The current command tree does not expose a `reconcile` command. Do not invent a
CLI flag or treat a repeated `send` as a safe recovery operation. A local
`unknown` record remains durable local evidence even if a later domain-level
read finds a matching message.

### 7. Fetch inbound data as untrusted input

`inbox.fetch` uses only enabled inbound aliases and one explicit `cursor` or
`time` boundary. The implementation is bounded to 10 pages, 1,000 raw messages,
and 100 point checks. It returns continuation metadata and commits the page and
cursor locally before bounded point reconciliation.

The filter can retain human replies to accepted local deliveries or direct
mentions of the configured bot, and it excludes bot/webhook authors and
repo-com's own messages. The current final-binary composition supplies an empty
accepted-delivery list to the fetcher, so a non-mention reply may be omitted by
the CLI path; this is a known implementation gap, not a reply-coverage
guarantee.

Every envelope is explicitly `untrusted`. Text, mentions, edits, deletions, and
attachment indicators cannot approve, activate policy, override safety, alter a
destination, or trigger a send. The first snapshot, current snapshot or deleted
marker, and local transitions are separate. A stored snapshot is not current
remote state.

`inbox.acknowledge` and `inbox.archive` are idempotent local actions. They do
not react, edit, delete, assign, or otherwise mutate Discord. `reply.draft-create`
validates a retained inbound target and creates a linked immutable draft; it
does not send directly and must pass the normal draft lifecycle afterward.

### 8. Inspect audit, state, and lifecycle

`audit.query` is a bounded, repository-scoped, redacted, read-only query. It
requires one `page_size` or `limit` from 1 through 100 and may filter by time,
object type, object ID, transition, and continuation cursor. It does not export
state, or contact Discord. It does not provide response analytics.

`state.verify` opens an existing `database_path` read-only and checks quick
check, foreign keys, migration, repository scope, and permissions without
repair, migration, or deletion. `state inspect`/`lifecycle inspect` returns
bounded local projections for the supported object families. Use
`include_retained_content` only when the operator explicitly needs retained
content; the result remains local data, not current remote state.

### 9. Apply retention and plan a local purge

The retention policy defaults to 30 days for content and 365 days for metadata.
Content overrides are 1 through 365 days; metadata overrides are 30 through
3,650 days, and metadata retention cannot be shorter than content retention.
The domain sweeper is transactional, replaces expired content with
`[content-expired]`, preserves non-content evidence, and blocks a new mutation
when its sweep fails.

The current final binary has no `retention.sweep` command. An embedding
application must invoke the retention service before mutations if it relies on
opportunistic retention. The explicit `purge.plan` command is a separate,
non-mutating local preview; it does not run a retention sweep.

For a purge:

1. Choose `scope` equal to `content`, `metadata`, or `all` and one canonical
   cutoff representation.
2. Review counts, table counts, state fingerprint, `config_hash`, `plan_hash`,
   and `execution_performed: false`.
3. If the plan is current, run `purge.execute` in a real TTY with the same
   fields and `executed_at`.
4. Enter the exact prompt response:

   ```text
   purge <repository> <scope> <cutoff-unix> <config-hash> <plan-hash>
   ```

5. If any fact changed, generate a new plan. Read the count-only result and
   local audit event ID.

A purge is local-only. It never edits, deletes, or otherwise mutates a Discord
message, and it cannot revoke a copy in a backup or filesystem snapshot.

## Accessibility and terminal behavior

The terminal contract is designed for keyboard operation and linear
screen-reader reading:

- output is labeled by field and states destination, revision, authority,
  safety, outcome, provenance, and next action in text;
- the effective presentation supports 80 columns and wraps rather than
  truncating hashes, security values, or approval information;
- `NO_COLOR` and `--color never` are plain-text modes; color is never the only
  meaning;
- prompts expose keyboard actions, cancellation, invalid-input recovery, and
  exact confirmation; and
- non-TTY mode does not prompt, and machine stdout remains separate from
  diagnostics on stderr.

The contract tests and snapshots verify these mechanical properties. They do
not constitute a human accessibility or usability review.

## Troubleshooting

| Symptom | Likely cause | Corrective action |
|---|---|---|
| `config-not-found` | No configuration in the bounded search | Add `.repo-com.toml` or pass an in-root `--config` |
| `multiple-config-candidates` | Multiple ancestor candidates | Remove the extra file or select one explicit path |
| `unsupported-schema-version` | Schema is not version 1 | Update the document from the example |
| `secret-field` / `raw-destination-field` | Forbidden configuration shape | Remove the field and use named aliases |
| `operator-action-required` / `TtyRequired` | Authority action without a TTY | Use a real interactive terminal; never self-approve in automation |
| `policy-blocked` | Missing, stale, ambiguous, or changed exact authority | Inspect status, correct the tuple, and make a fresh decision |
| `authentication` | Dedicated bot credential rejected | Rotate/revoke the bot token and retry setup |
| `permission` | Discord access is insufficient | Apply the reported manual grant only |
| `connectivity-rate-limit` | Dynamic limit or bounded request failure | Honor the returned delay; do not guess a reset time |
| `unknown-delivery` | Dispatch may have reached Discord | Keep blocked and perform read-only reconciliation; no automatic resend |
| `storage-integrity` | SQLite, lock, migration, permission, or transaction failure | Preserve the database and sidecars; do not recreate state |
| `expired` / `stale-authority` | Draft or authority is no longer current | Create a new revision or obtain fresh exact authority |
| Purge plan changed | Configuration or local state changed | Generate a fresh plan and confirmation |

## Unsupported behavior and evidence boundary

This version does not provide:

- no read receipts, no proof of teammate attention, and no response analytics;
- arbitrary destinations, broadcasts, fan-out, direct-message authorization, or
  user-token/self-bot authentication;
- a daemon, Gateway monitor, background scheduler, arbitrary history search, or
  remote message edit/delete;
- automatic Discord permission changes or automatic resend of an unknown
  delivery;
- a `reconcile` command, policy-deactivation command, or retention-sweep command
  in the current command tree;
- encryption at rest, an OS keychain, encrypted backups, or a guarantee against
  local-account compromise, backups, or filesystem snapshots; or
- a claim of live Discord compatibility, human approval, compliance
  certification, or release sign-off from automated tests.

A local accepted result is not a read receipt. A stored inbound snapshot is not
current remote truth. A valid setup report is point-in-time local evidence. The
security and threat model documents contain the detailed residual-risk register.

## Sources

Primary sources are `docs/features/release-readiness.md` (`REL-FR-02`,
`REL-FR-03`, and `REL-FR-08`), `docs/features/cli-foundation.md`, the implemented
CLI handler/input modules, the final process router, the terminal renderer and
prompt contracts, the state/lifecycle/retention/purge services, and the Discord
adapter modules. The [configuration contract](configuration.md),
[Discord setup](discord-setup.md), [security model](security-model.md), and
[threat model](threat-model.md) own their specialist topics.

This document is implementation guidance for review. It does not authorize a
human decision, a live acceptance result, or a release decision.
