# doc-map

Create and maintain a repository documentation map by discovering candidate documents, letting the user select the maintained set interactively, assigning priorities, and connecting the result to `AGENTS.md` instructions.

## When To Use It

Use this skill for requests such as:

- "Maintain the repository docs."
- "Update the documentation map."
- "Check which documentation needs updating."
- "Create a docmap for this repository."

The skill defaults to a root-level `docmap.jsonl`. It supports root-level `docmap.sqlite` when indexed queries are more useful than a reviewable JSONL file.

## Installation

Copy the complete skill directory into the skill location used by your assistant:

```bash
cp -r collection/skills/doc-map .agents/skills/
```

Keep the directory name `doc-map` unchanged. The package is self-contained and includes its `SKILL.md` and `references/` files.

## Workflow

The skill:

1. Discovers tracked documentation and optionally asks whether untracked files should be included.
2. Assigns suggested priorities, including higher defaults for `docs/` and conventional files such as `README`, `CHANGELOG`, `CONTRIBUTING`, `SECURITY`, and `CODE_OF_CONDUCT`.
3. Walks through interactive document selection with individual and batch commands such as `accept all high`, `exclude generated`, and `set README.md critical`.
4. Shows the proposed map before writing it.
5. Confirms the complete accepted, excluded, and deferred selection before choosing storage.
6. Creates or updates the map only after explicit confirmation.
7. Proposes a scoped `AGENTS.md` maintenance section and requires separate confirmation before writing it.

Suggested priorities are recommendations, not automatic inclusion:

| Priority | Typical documents |
|----------|-------------------|
| `critical` | `AGENTS.md`, root policy, compliance, or release-control guidance |
| `high` | `docs/` content and conventional project documents |
| `normal` | Other active human-maintained documentation |
| `low` | Archived, historical, example-only, or informational material |

## JSONL Records

Document records include:

- Repository-relative `path` and `format`
- `status`, `source_of_truth`, and `generated`
- User-confirmed `priority`
- Document `group`, `authority`, and `purpose`
- `update_when` triggers
- Scope, audience, and related paths

The map may also include `map_metadata` and `checklist` records for repository-wide documentation rules and reference checks. See [`references/map-schema.md`](references/map-schema.md) for the complete JSONL and SQLite guidance.

## Supporting References

- [`references/document-selection.md`](references/document-selection.md): priority defaults, classifications, and interactive selection protocol.
- [`references/map-schema.md`](references/map-schema.md): JSONL records and the optional SQLite schema.
- [`references/agents-instructions.md`](references/agents-instructions.md): template for connecting the map to `AGENTS.md`.

## Validation

For a JSONL map, validate every line with `jq`:

```bash
jq -c . docmap.jsonl
```

If `jq` is unavailable, use Python:

```bash
python3 -c 'import json, pathlib; [json.loads(line) for line in pathlib.Path("docmap.jsonl").read_text().splitlines()]'
```

Also confirm that mapped paths are unique, repository-relative, and present unless explicitly marked `missing`, then run:

```bash
git diff --check
```

The skill does not silently include generated, vendored, archived, duplicate, or excluded documents.

When local and packaged copies of this skill both exist, follow the repository's documented source-of-truth rule and keep the copies synchronized when changing the workflow.
