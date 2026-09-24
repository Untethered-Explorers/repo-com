---
name: doc-map
description: "Discover repository documentation, let the user select documents to maintain, create or update a document reference map, and add AGENTS.md instructions for maintain docs, update documentation, and check documentation requests."
---

# Skill: Document Reference Map

Use this skill when a user asks to maintain docs, update documentation, or check documentation. It inventories repository documentation, records only the documents the user approves, and connects that map to repository instructions without overwriting unrelated guidance.

Use root-level `docmap.jsonl` by default because it is portable, reviewable, and diff-friendly. Offer root-level `docmap.sqlite` when the user needs indexed queries or the repository has too many records for comfortable line-based review; use another approved location only when the user requests it. Keep one document record per line, plus optional `map_metadata` and `checklist` records for map-wide guidance. Every selected document record includes a user-overridable priority: `critical`, `high`, `normal`, or `low`.

Load `references/map-schema.md` when the user confirms a map should be created or updated. Load `references/document-selection.md` only for large or ambiguous inventories, and load `references/agents-instructions.md` only when drafting the `AGENTS.md` section. All references are direct children of this skill's `references/` directory and do not load other references.

## Process

### Step 1: Discover documentation

Inspect the repository root and nested directories for documentation files and documentation-bearing configuration. Respect `.gitignore`; exclude dependency, build, cache, vendored, and generated directories unless the user explicitly includes them. Include Markdown, MDX, text, reStructuredText, AsciiDoc, and project-specific documentation formats when their content is maintained by humans.

Use `git ls-files` when the repository is tracked, then supplement it with clearly relevant untracked files only if the user asks for them. Normalize paths relative to the repository root and deduplicate case-insensitively before presenting the inventory. If the repository is not Git-based, use a filesystem scan with the same exclusions and state that Git tracking could not be used.

Assign a suggested priority before presenting the inventory: `critical` for applicable `AGENTS.md` files and root policy or release-control documents; `high` for files under `docs/` and conventional project documents such as `README`, `CHANGELOG`, `CONTRIBUTING`, `CODE_OF_CONDUCT`, `SECURITY`, `LICENSE`, `GOVERNANCE`, and `SUPPORT`; `normal` for other human-maintained documentation; and `low` for archived or informational material. Match conventional names case-insensitively and recognize common suffixes such as `.md`, `.mdx`, and `.txt`.

**Output:** An inventory table with an index, path, format, likely audience or topic, suggested priority, classification, and generated-file warning.

### Step 2: Select the maintained set interactively

Present the inventory before writing anything and walk through it interactively. For each candidate, ask for one of `accept`, `accept with priority`, `exclude`, or `inspect`; support batch responses such as `accept all high`, `accept docs/**`, `exclude generated`, and `set README.md critical`. After each batch, show the remaining unreviewed count and the current accepted set. Do not continue to storage selection until every candidate is accepted, excluded, or explicitly deferred. Load `references/document-selection.md` when the inventory is large, ambiguous, or contains generated, duplicated, nested, or monorepo documentation.

Do not infer that every discovered document belongs in the map. If a document has a generated header, is under a build/vendor directory, or duplicates another source, mark it as excluded or derived and ask for confirmation. If the user selects a broad directory, expand it into exact files and restate the count before proceeding. A suggested priority is not approval: preserve it only after the user accepts the document, and allow the user to override it. Keep the approved set stable by storing repository-relative paths.

**Output:** An explicit approved set with final priorities, document groups, authority/purpose, update triggers, related paths, and an exclusion or deferral list.

Before choosing storage, summarize the final accepted, excluded, and deferred paths and ask the user to confirm that selection. Do not create or modify a map while any candidate remains deferred; a user may revise priorities or selections at this checkpoint.

### Step 3: Choose map storage

Recommend root-level `docmap.jsonl`. Use root-level `docmap.sqlite` only if the user requests SQLite or indexed filtering is materially useful. Load `references/map-schema.md` before creating or modifying either format.

If a map already exists, identify its format and location. Preserve the existing format unless the user explicitly approves migration. If multiple maps exist, stop and ask which is authoritative rather than merging silently.

When this skill exists in both a local development directory and a packaged collection directory, identify the documented source of truth and keep the other copy synchronized in the same change. Do not assume that one copy is generated or overwrite a differing copy without checking repository guidance.

**Output:** One confirmed map path and format.

### Step 4: Create or update the map

Compare the approved set to the existing map, if any. Preserve user-maintained metadata, refresh paths and clearly stale facts, and flag missing or renamed documents for confirmation instead of deleting them automatically. For a new map, write one record per approved document with a stable repository-relative path, group, authority/purpose, update triggers, related paths, source-of-truth status, and generated status. Add a `map_metadata` record for repository-wide usage instructions and a `checklist` record for cross-document reference checks when the user approves them.

Show the proposed diff or record summary and obtain confirmation before writing. For JSONL, validate that each line is an independent object and that paths are unique. For SQLite, create the schema and run duplicate-path and stale-path checks before treating the update as complete. If any approved path is missing, pause and ask whether to mark it `missing`, replace it with a renamed path, or remove it from the approved set; never resolve that choice silently.

**Output:** A confirmed, parseable map containing exactly the approved maintained documents.

### Step 5: Update repository instructions

Find the nearest applicable `AGENTS.md` files, including root and scoped nested instructions. If none exists, propose a root `AGENTS.md`; if one exists, append a clearly delimited document-maintenance section rather than rewriting existing prose. Load `references/agents-instructions.md` when drafting or editing this section.

The instructions must point to the actual map path, require checking relevant mapped documents before and after changes, and say how to update the map when documentation is added, removed, or renamed. Explain that `critical` and `high` documents should be checked first when several documents may be affected. Show the exact proposed diff and obtain confirmation before writing. If the user declines, leave the map intact and report that instructions were not changed. If the nearest scoped file conflicts with root guidance, preserve the scoped rule and ask whether the map section belongs in the root, scoped file, or both.

**Output:** An updated or newly proposed `AGENTS.md` with a map-specific maintenance rule.

## Gotchas

- **Git discovery can omit intended files.** `git ls-files` excludes untracked documentation; report that limitation and ask whether untracked files should be included rather than silently treating the inventory as complete.
- **Generated documentation can look authoritative.** Files in build, vendor, cache, or generated directories may contain valid Markdown but should not be mapped as editable sources unless the user explicitly approves them.
- **Duplicate maps create conflicting instructions.** If more than one JSONL or SQLite map is present, do not merge or choose by modification time; ask the user to designate one authoritative map.
- **A successful write can still produce a wrong map.** Validate repository-relative paths, uniqueness, parseability, and membership against the approved set before editing `AGENTS.md`.
- **AGENTS.md scope matters.** A nested `AGENTS.md` can override or narrow root guidance; update the nearest applicable file and avoid inserting root-only instructions into a scoped directory.
- **Priority is only a recommendation until accepted.** Do not silently map a `docs/` file or conventional document just because it received a `high` suggestion; record it only after the interactive selection step.
- **Conventional names have aliases and spelling variants.** Match `CONTRIBUTING`, `CONTRIBUTION`, `CODE_OF_CONDUCT`, `SECURITY`, `CHANGELOG`, and case variants, but show the exact path and let the user override the priority.

## Validation

After each write, verify:

- [ ] The map exists at the confirmed path and uses the confirmed format.
- [ ] Every JSONL line parses as one object, or SQLite opens and contains the expected table/schema.
- [ ] Mapped paths are unique, repository-relative, and present unless explicitly marked stale or missing.
- [ ] Every document record has group, authority, purpose, update triggers, generated status, priority, and source-of-truth metadata.
- [ ] Map-wide instructions and the reference-check checklist are represented as `map_metadata` or `checklist` records when approved.
- [ ] The mapped set matches the user-approved set; excluded or unconfirmed files are not silently included.
- [ ] The applicable `AGENTS.md` contains the actual map path and instructions to check and maintain it.
- [ ] `git diff --check` exits successfully and the diff contains no unrelated changes.

For a JSONL map, run `jq -c . docmap.jsonl`; if `jq` is unavailable, run `python3 -c 'import json, pathlib; [json.loads(line) for line in pathlib.Path("docmap.jsonl").read_text().splitlines()]'`. If parsing fails, fix or remove the offending line before changing `AGENTS.md`. For SQLite, run `sqlite3 docmap.sqlite '.schema'` and the duplicate-path query from `references/map-schema.md`; if `sqlite3` is unavailable, use the repository's available SQLite tooling or stop before claiming validation passed. These commands are checks, not instructions to overwrite files.
