# AGENTS.md Maintenance Section

> Load when drafting or editing an AGENTS.md section that connects repository work to the document map.

Adapt this template to the confirmed map path, which defaults to root-level `docmap.jsonl`, and applicable scope. Preserve existing headings and prose; add one delimited section rather than replacing the file.

```markdown
## Documentation Map Maintenance

- Before changing code or behavior, check the relevant entries in `<MAP_PATH>` and read the mapped source documents for the affected area.
- When several mapped documents may be affected, check `critical` and `high` priority entries first, then review `normal` and `low` entries as relevant.
- After changing code or behavior, update every affected mapped document or explain why no documentation change is needed.
- When a document is added, removed, renamed, or changes from source to generated output, update `<MAP_PATH>` in the same change.
- Keep paths in `<MAP_PATH>` repository-relative and do not add generated or vendored documents unless explicitly approved.
- Use the map's document groups, authority/purpose, and `update_when` fields to identify the source-of-truth documents that need review; do not update every document mechanically.
```

Replace `<MAP_PATH>` with the actual path. If the file already has a documentation rule, consolidate only when the result preserves its scope and meaning. For nested instructions, mention only documents relevant to that directory and avoid claiming root-wide authority.
