# View and Stream Matrix

> Load when: enumerating terminal views, typed labels, snapshot cases, prompt behavior, or stdout and stderr boundaries.

## Outbound Views

Include deterministic cases for:

- draft and exact rendered preview;
- resolved destination, expiry, and revision;
- policy status and exact approval;
- secret finding and redacted override state;
- proven failed, retry wait, accepted, unknown, unresolved, and reconciled absence;
- eligibility rejection and expired or stale authority;
- operational and integrity errors with next actions.

Every view must state whether a result is a local decision, a mocked observation, or a last-fetched remote state. Accepted must never become read or replied.

## Operations Views

Include deterministic cases for:

- configuration and validation status;
- policy status and exact activation;
- local state integrity verification;
- bounded audit and lifecycle inspection;
- untrusted inbound item and provenance;
- acknowledgement and archive status;
- retention status;
- purge plan counts and hash;
- confirmed purge result, rollback, and replan requirement;
- locked, corrupt, unsupported, missing, and operational errors.

Do not render purge planning as execution or last-fetched remote state as current truth.

## Label Requirements

Use explicit text labels for:

- `Repository`
- `Object` or `Revision`
- `Destination`
- `Expires`
- `Approval` or `Policy`
- `Safety`
- `Observed at`
- `Outcome`
- `Next action`

Exact label casing may follow the owning UI contract, but the semantic fields must not disappear. At 80 columns, wrap or repeat identity; do not truncate it.

## Stream Matrix

| Surface | stdout | stderr or interactive stream |
|---|---|---|
| Human mode | complete linear labeled output | diagnostics only when useful |
| Machine mode | exactly one protocol version 1 JSON object | diagnostics; never prompts |
| Prompt mode | no machine protocol object | complete preview and keyboard interaction |
| Operational failure | one valid JSON object in machine mode | redacted diagnostic without secret or raw content |

## Snapshot Matrix

For each required view, cover:

- width 80;
- explicit non-color mode;
- `NO_COLOR` mode;
- no ANSI assertion when disabled;
- long hashes, revisions, findings, and next actions;
- text-only state and focus;
- inbound untrusted provenance;
- local versus last-fetched remote state;
- deterministic ordering.

Prompt tests additionally cover cancel, default, invalid input, expiry, and changed plan hash or object identity. Non-TTY tests assert that the prompt dependency is never invoked.

## Evidence Boundary

Snapshots, keyboard tests, and protocol fixtures prove deterministic implementation behavior. They do not prove human usability, screen-reader quality, live Discord compatibility, or final release approval.
