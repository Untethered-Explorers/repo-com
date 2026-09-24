# Document Map Schemas

> Load when creating or modifying a JSONL or SQLite document reference map.

## JSONL

Write one object per line. Use `record_type: "document"` for document records. Required document fields are `record_type`, `path`, `format`, `status`, `source_of_truth`, `priority`, `group`, `authority`, `purpose`, `update_when`, and `generated`. Use repository-relative POSIX paths. Optional `map_metadata` and `checklist` records store map-wide guidance without pretending that guidance is a document path.

Recommended record:

```json
{"record_type":"document","path":"docs/architecture.md","format":"markdown","status":"active","source_of_truth":true,"priority":"high","group":"architecture-research","authority":"High-level repository architecture","purpose":"Explains component boundaries, data flow, and system structure.","update_when":["component boundary changes","data flow changes","system structure changes"],"generated":false,"scope":"repository","audience":"contributors","related_paths":[]}
{"record_type":"map_metadata","map_version":1,"title":"Documentation Map","instructions":["Read the relevant source-of-truth documents before changing behavior.","Update affected documentation in the same change.","Preserve historical records when they accurately describe past behavior."]}
{"record_type":"checklist","name":"reference-check","items":["implementation and tests","relevant skill or package README","user-facing guide or top-level README","changelog or release notes","related ADRs and deep dives","internal links and referenced paths","examples and environment-variable names"]}
```

Allowed `status` values are `active`, `derived`, `archived`, and `missing`. Allowed `priority` values are `critical`, `high`, `normal`, and `low`. `generated` must be a boolean. Preserve additional user fields when updating. Do not duplicate a document path or use absolute paths. A generated record should normally use `status: "derived"` and identify its source in `related_paths` or `notes`.

## SQLite

Use this minimal schema unless the repository has an existing approved schema:

```sql
CREATE TABLE IF NOT EXISTS documents (
  path TEXT PRIMARY KEY,
  format TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('active', 'derived', 'archived', 'missing')),
  source_of_truth INTEGER NOT NULL CHECK (source_of_truth IN (0, 1)),
  priority TEXT NOT NULL CHECK (priority IN ('critical', 'high', 'normal', 'low')),
  record_group TEXT NOT NULL,
  authority TEXT NOT NULL,
  purpose TEXT NOT NULL,
  update_when TEXT NOT NULL,
  generated INTEGER NOT NULL CHECK (generated IN (0, 1)),
  scope TEXT,
  audience TEXT,
  update_triggers TEXT,
  related_paths TEXT,
  updated_at TEXT NOT NULL
);
```

Store arrays as JSON text and timestamps as UTC ISO 8601. Before completing an update, run:

```sql
SELECT path, COUNT(*) FROM documents GROUP BY path HAVING COUNT(*) > 1;
SELECT path FROM documents WHERE path LIKE '/%' OR path LIKE '%\\%';
```

The first query must return no rows. The second detects absolute or Windows-style paths that violate the repository-relative convention.

SQLite implementations may store `map_metadata` and `checklist` records in separate tables, or keep them as JSON in a map metadata table. Do not force map-wide instructions into the `documents` table.
