# Configuration contract

This document owns the repository configuration schema, discovery rules, aliases,
retention periods, and the boundary between a declared auto-send tuple and an
operator-activated policy. The implemented model is in
`crates/repo-com-config`; the canonical requirements are `REPO-FR-01` through
`REPO-FR-04` in `docs/features/repository-configuration-and-state.md` and the
product constraints in `docs/PRD.md#10`.

## Scope and safety boundary

The configuration file is committed, non-secret workflow data. It may name a
workspace, channels, mention aliases, inbound aliases, retention periods, and
exact auto-send tuples. It must not contain a Discord token, password, private
key, cookie, authorization value, or other secret-like field. The Discord token
is accepted only through the environment variable documented in
[`discord-setup.md`](discord-setup.md); it is not a configuration field and is
not stored in local state.

Configuration parsing and validation do not activate policy, approve a draft,
contact Discord, or mutate state. A configuration result is a local validation
result, not evidence that a remote workspace is ready.

## File discovery

The supported file name is `.repo-com.toml`. A caller may provide an explicit
normalized path through the `--config PATH` global option, or the resolver may
search the current directory and its ancestors up to the repository root for
exactly one `.repo-com.toml` file. A missing candidate, multiple candidates, a
path outside the repository root, or an unsafe filesystem path fails with a
typed configuration error. No configuration, destination, or repository is
silently selected by a default.

The validator reports safe path-aware error categories. It does not echo the
file contents, a token, an authorization value, or a real message body in an
error or diagnostic.

## Schema version 1

The only accepted schema is `schema_version = 1`. A future or unknown version is
rejected without automatic migration. Unknown fields are rejected at every
configuration level rather than ignored.

A synthetic, secret-free shape is:

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

The numeric identifiers above are synthetic documentation placeholders. Replace
them with the identifiers for the operator's disposable workspace; do not copy
production team content into a configuration or fixture.

### Fields and aliases

| Field | Meaning and boundary |
|---|---|
| `repository_id` | Stable, explicit repository scope used by local state and every handler. It is not inferred from a directory name. |
| `discord.workspace_id` | The one configured Discord workspace. A destination or inbound alias may not refer to another workspace. |
| `destinations.<alias>` | A repository-local **destination alias** for exactly one channel. Normal skill-facing commands accept the alias, not a raw Discord channel identifier. |
| `destinations.<alias>.allowed_mentions` | Repository-local mention aliases that the renderer may resolve for this destination. An empty list is valid; an unlisted alias is not. |
| `mentions.<alias>` | A named **mention alias** whose target is exactly `role:<identifier>` or `user:<identifier>`. The local parser requires a portable non-empty identifier; the Discord setup check later verifies the remote target and its workspace. Raw role, user, channel, and bot mention forms are rejected. |
| `inbound.<alias>` | An **inbound alias** that inherits the channel of the matching destination alias in the same workspace. `enabled` controls whether retrieval is allowed. |
| `retention` | Local content and metadata periods; see the limits below. |
| `auto_send` | Non-activated exact declarations; see the policy section. |

Aliases are repository-local and deterministic. Duplicate aliases, missing
mention targets, unknown destination references, cross-workspace references,
malformed Discord identifiers, and invalid mention prefixes fail validation.
A raw destination field is not an alternative spelling for an alias.

The local configuration crate does not call Discord. A cross-workspace
reference is rejected when the caller supplies the optional
`WorkspaceReferenceIndex` built from separately observed setup data; without
that index, remote workspace membership and channel visibility remain
`setup-check` responsibilities. A local `config.validate` result must not be
read as a live cross-workspace proof.

## Retention configuration

The implemented defaults are:

Both `content_days` and `metadata_days` are required keys in the v1 TOML
model. The defaults below apply when a caller constructs the library
`RetentionConfig`/`RetentionPolicy`; omitting a TOML key is not an implicit
configuration default.
- `content_days = 30`: draft and inbound message content remains readable for
  30 days by default;
- `metadata_days = 365`: non-content delivery and audit metadata remains for
  one year (365 days) by default.

A per-repository content override is **1 through 365** days. A metadata
override is **30 through 3,650** days, and metadata retention must be at least
as long as content retention. The configuration parser itself rejects zero
values; the retention policy owner enforces the full 1-through-365 and
30-through-3,650 bounds and the metadata-at-least-content rule when it builds
`RetentionPolicy`. A non-mutating `config.validate` result alone is therefore
not proof that a retention sweep can run: the retention service must validate
the policy before a state mutation. Retention is local and is separate from
the purge scopes described in the operator guide.

At content expiry, the retention owner removes or irreversibly replaces draft
and inbound text with the retained marker `[content-expired]` while preserving
non-content identifiers, hashes, timestamps, lifecycle state, and audit
evidence. A failed sweep blocks a new state mutation with a storage-integrity
result; it does not silently extend the cutoff.

## Exact auto-send declarations

Each `[[auto_send]]` entry is an exact three-part tuple:

1. `event_type` — the exact event type;
2. `destination` — the exact destination alias, resolved to one configured channel; and
3. `severity` — the exact severity.

Matching is equality-only. A wildcard, prefix, broader severity, or raw
channel is ineligible. A declaration in this file is not an activation and
cannot authorize a send by itself. Activation is a separate interactive TTY
action bound to the canonical configuration hash, the exact tuple hash, and
the activation time. A changed relevant configuration or tuple makes the old
activation stale. See `policy.status` and `policy.activate` in the operator
guide.

## Rejected fields and migration behavior

The following are rejected, not silently ignored:

- unknown fields and unknown/future schema versions;
- secret-like fields, including token, password, private key, cookie, or
  authorization-value fields;
- raw destination, raw channel, and unlisted mention fields;
- duplicate aliases, and known cross-workspace references when a workspace reference index is supplied;
- invalid event types, severities, timestamps, zero retention periods, and
  policy tuples; full retention bounds and ordering are checked by the
  retention policy owner; and
- unsafe paths, unsafe schema versions, or more than one discovered configuration.

There is no automatic migration, import, export, or keychain integration in
this version. A changed file must be reviewed and validated as a new local
configuration; the resulting hash is the basis for any later approval or
policy decision.

## Operator checklist

1. Place a secret-free `.repo-com.toml` at the repository root, or pass an
   explicit in-repository path.
2. Use aliases in workflow input; do not introduce raw Discord destinations.
3. Run the structured `config.validate` operation described in the operator
   guide and inspect its safe error and hash fields.
4. Re-run validation after any change. Do not infer that a valid file means
   Discord permissions or a live workspace have been verified.

## Sources and limitations

Authoritative sources are `docs/features/repository-configuration-and-state.md`,
`docs/PRD.md#10. Security and Privacy`, and the implemented
`repo-com-config` model/resolver/validator. This document does not claim
live Discord compatibility, human approval, release approval, or compliance
certification. It describes the local configuration contract only.
