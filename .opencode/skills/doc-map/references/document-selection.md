# Document Selection And Priority

> Load when the inventory is large, ambiguous, generated, duplicated, nested, or spread across a monorepo.

Classify and prioritize each candidate before asking for approval:

| Suggested priority | Default candidates | Meaning |
|---|---|---|
| `critical` | Applicable `AGENTS.md`; root policy, release, or compliance guidance | Check first because missing updates can invalidate repository work or operating rules. |
| `high` | Any file under `docs/`; root or package `README`, `CHANGELOG`, `CONTRIBUTING` or `CONTRIBUTION`, `CODE_OF_CONDUCT`, `SECURITY`, `LICENSE`, `GOVERNANCE`, or `SUPPORT` | Usually user- or contributor-facing and commonly affected by changes. |
| `normal` | Other active human-maintained documentation | Include when relevant, but do not assume every document needs review for every change. |
| `low` | Archived, historical, example-only, or informational material | Track only when the user wants long-tail documentation maintained. |

Priority is case-insensitive and is a starting recommendation, not consent to map the file.

| Classification | Map by default? | Decision rule |
|---|---:|---|
| Authoritative source | Yes, after approval | Human-edited document that defines behavior, usage, policy, or architecture. |
| Derived output | No | Generated from another document or build step; map the source instead. |
| Archived material | No | Historical content not expected to change with current code. Include only on request. |
| Duplicate or mirror | No | Same content appears elsewhere; identify the canonical path. |
| Scoped instruction | Usually yes | `AGENTS.md` or equivalent that controls work in a directory; record its scope. |
| Untracked candidate | Ask | Include only when the user confirms untracked files are in scope. |

Use this interactive response protocol:

1. Show candidates in indexed groups, starting with `critical`, then `high`, `normal`, and `low`.
2. Ask for `accept`, `accept with priority`, `exclude`, `inspect`, or `defer` for the current candidate or group.
3. Accept batch selectors such as `all high`, `docs/**`, and `README*`; expand each selector to exact paths before recording it.
4. Show the updated accepted, excluded, and deferred lists after every batch.
5. Before leaving selection, require an explicit decision on every deferred candidate and confirm the final count by priority.

Examples:

```text
accept all high
set docs/adr/0001.md critical
exclude **/generated/**
inspect CHANGELOG.md
```

For each approved file, capture:

- Repository-relative path.
- Format and scope or audience.
- Document group, such as repository guidance, product/user-facing, requirements/authoring, execution/orchestration, architecture/research, or release/change history.
- Authority and purpose: what the document is authoritative for and who relies on it.
- Whether it is authoritative or derived.
- Whether it is generated; generated artifacts should normally be excluded from the maintained set and mapped only as derived references when useful.
- Update triggers: concrete code, behavior, workflow, release, or policy changes that require an update.
- Final priority: `critical`, `high`, `normal`, or `low`.
- Related code or directories, if known.
- Optional canonical links to related documents.

Use these default groups when they fit, but allow repository-specific group names:

- `repository-guidance`
- `product-user-facing`
- `requirements-authoring`
- `execution-orchestration`
- `architecture-research`
- `release-change-history`
- `generated-artifacts`

When a repository has generated outputs, identify the source document, template, skill, or compiler that produces each output. Do not treat generated output as the editable source of truth.

In a monorepo, group candidates by package or service and identify the nearest `AGENTS.md` before asking for approval. Do not treat a root README as authoritative for every package without confirmation.
