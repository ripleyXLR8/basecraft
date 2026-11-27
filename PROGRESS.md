# Progress

Legend: ✅ done and tested · 🚧 in progress · ⏳ not started

## Milestones

| # | Milestone | State |
|---|-----------|-------|
| M0 | Bootstrap: workspace, CI, docs | ✅ |
| M1 | Read `.fmp12`: schema, data, layouts, scripts | 🚧 |
| M2 | Calculation engine | ⏳ |
| M3 | Data engine: CRUD, find, sort, relationships, calc and summary fields | ⏳ |
| M4 | GUI: Browse / Find / List / Table views | ⏳ |
| M5 | Script engine and Script Workspace | ⏳ |
| M6 | Layout mode | ⏳ |
| M7 | Write `.fmp12` files that FileMaker Pro opens | ⏳ |
| M8 | Import/export, Preview/print/PDF, charts, security, Data Viewer | ⏳ |
| M9 | Out of scope for now: Server hosting, WebDirect, ODBC, plug-ins, AI steps | — |

## M1 — Reading `.fmp12`

| Feature | State |
|---------|-------|
| Header, magic, creator string | ✅ |
| Sector chain, B-tree leaf walk | ✅ |
| Byte-code decoder (all known opcodes) | ✅ |
| SCSU text decoding | ✅ |
| Table names | ✅ |
| Field names | ✅ |
| Record values (plain and long) | ✅ |
| Repeating fields | ⏳ |
| Field types and options (type, data type, storage, index, auto-enter, validation) | ✅ |
| Compiled calculations (auto-enter, validation, calc fields) | 🚧 |
| Table occurrences (base table, position, colour) and relationships (TOs, join fields, cascade create) | ✅ |
| Relationship options: join operators other than `=`, cascade delete, sorting | 🚧 |
| Layouts: name, table occurrence, width, parts (kind, position, height, style) | ✅ |
| Layout objects: kind, id, bounds, style name, field reference, static text | ✅ |
| Layout objects: nesting (tabs, portals, button bars), local CSS, formatting, actions | 🚧 |
| Scripts: catalog, folders, step list (type, enabled, options) | ✅ |
| Script step parameters | 🚧 |
| Value lists (custom values, from field) | ✅ |
| Custom functions, accounts and privileges, themes | ⏳ |
| Container data | ⏳ |

Oracle: all 214 template fields and 32 graph objects, 30 scripts, 85 script steps 24 value lists, 24 layouts and 504 layout objects match FileMaker's XML export.

Corpus: all 66 FileMaker 26 starter templates (6 solutions × 11 languages) parse without error.

## M2 — Calculation engine

Reference: [docs/reference/functions.txt](docs/reference/functions.txt) (234 functions) and
[docs/reference/get_functions.txt](docs/reference/get_functions.txt) (140 `Get()` flags).

| Item | Done / total |
|------|-------------:|
| Functions | 0 / 234 |
| `Get()` flags | 0 / 140 |
