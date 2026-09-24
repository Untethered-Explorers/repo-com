# repo-com — Agent Instructions

## Documentation Map Maintenance

- The document reference map lives at `docmap.jsonl` and covers the repo-com
  project documentation suite (README, changelog, user guide, administrator
  guide, ADRs, and release notes). It does not cover `docs/PRD.md`,
  `docs/features/*`, or generated `.opencode/` agents and skills.
- Before changing code or behavior, check the relevant entries in `docmap.jsonl`
  and read the mapped source documents for the affected area.
- When several mapped documents may be affected, check `critical` and `high`
  priority entries first, then review `normal` and `low` entries as relevant.
- After changing code or behavior, update every affected mapped document or
  explain why no documentation change is needed.
- When a document is added, removed, renamed, or changes from source to
  generated output, update `docmap.jsonl` in the same change.
- Keep paths in `docmap.jsonl` repository-relative and do not add generated or
  vendored documents unless explicitly approved.
- Use the map's document groups, authority/purpose, and `update_when` fields to
  identify the source-of-truth documents that need review; do not update every
  document mechanically.
