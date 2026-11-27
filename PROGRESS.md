# Progress

Legend: ✅ done and tested · 🚧 in progress · ⏳ not started

## Milestones

| # | Milestone | State |
|---|-----------|-------|
| M0 | Bootstrap: workspace, CI, docs | ✅ |
| M1 | Read `.fmp12`: schema, data, layouts, scripts | 🚧 |
| M2 | Calculation engine | 🚧 |
| M3 | Data engine: CRUD, find, sort, relationships, calc and summary fields | ⏳ |
| M4 | GUI: Browse / Find / List / Table views | ⏳ |
| M5 | Script engine and Script Workspace | ⏳ |
| M6 | Layout mode | ⏳ |
| M7 | Write `.fmp12` files that FileMaker Pro opens | 🚧 |
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
| Calculation token streams → source text (253 functions, 140 `Get()` flags, operators, literals, field refs, Let) | ✅ |
| Calculations in scripts and layouts (contexts other than a table occurrence) | 🚧 |
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

Oracle: all 214 template fields and 32 graph objects, 30 scripts, 85 script steps 24 value lists, 24 layouts, 504 layout objects and 31/31 field calculations (decompiled text) match FileMaker's XML export.

Corpus: all 66 FileMaker 26 starter templates (6 solutions × 11 languages) parse without error.

## M2 — Calculation engine

Reference: [docs/reference/functions.txt](docs/reference/functions.txt) (234 functions) and
[docs/reference/get_functions.txt](docs/reference/get_functions.txt) (140 `Get()` flags).

Crate: `crates/calc`. Run `cargo test -p calc registry_coverage -- --nocapture` for the live
numbers and the list of missing functions.

| Item | Done / total |
|------|-------------:|
| Functions | 142 / 234 |
| `Get()` flags answered without a host | 10 / 139 |

The 234 reference lines include `Status` twice. The 139 `Get()` flags exclude the
`Get ( flag )` header line. A host can answer any flag through `Context::get`. The 10
built-in flags are CurrentDate, CurrentTime, CurrentTimestamp, CurrentHostTimestamp
(UTC, because the engine has no time-zone data), CurrentTimeUTCMilliseconds,
CurrentTimeUTCMicroseconds, UUID, CalculationRepetitionNumber, SystemPlatform and
ApplicationArchitecture.

| Feature | State |
|---------|-------|
| Lexer, parser with error positions and codes, AST, round-trip pretty-printer | ✅ |
| Operators, precedence, coercions, exact decimal arithmetic (400 digits) | ✅ |
| If / Case / Choose / Let / While / Evaluate / EvaluationError / IsValid / SetPrecision | ✅ |
| Text, value-list and word functions | ✅ |
| Number, financial and math functions (decimal Exp/Ln/Sqrt/Pi; trig through f64) | ✅ |
| Date, time and timestamp functions (en_US formats) | ✅ |
| JSON (Set/Get/Delete/ListKeys/ListValues/Format/MakeArray/GetElementType) | ✅ |
| Base64, hex, URL encoding, CryptDigest, CryptAuthCode, TextEncode/Decode (UTF-8) | ✅ |
| Aggregates over lists, repetitions and related records (through `Context`) | ✅ |
| Styled text (TextColor, TextStyleAdd…): plain text passes through, styles dropped | 🚧 |
| Schema, design, layout, SQL, AI, sensor and encryption functions | ⏳ |
| Custom functions, file-locale date/number formats | ⏳ |

## M7 — Writing `.fmp12`

| Feature | State |
|---------|-------|
| Chunk encoder (all opcodes) and leaf packing; round-trips through our reader | ✅ |
| Same-size in-place edits accepted by FileMaker | ✅ (verified) |
| Index (separator) sectors that FileMaker accepts | 🚧 rebuilt indexes are rejected; separator rules under study |
