# Progress

**Estimated progress to 1.0: ~43 %**, weighted by milestone (see the README table).

Legend: ✅ done and tested · 🚧 in progress · ⏳ not started

## Milestones

| # | Milestone | State |
|---|-----------|-------|
| M0 | Bootstrap: workspace, CI, docs | ✅ |
| M1 | Read `.fmp12`: schema, data, layouts, scripts | 🚧 |
| M2 | Calculation engine | 🚧 |
| M3 | Data engine: CRUD, find, sort, relationships, calc and summary fields | 🚧 |
| M4 | GUI: Browse / Find / List / Table views | 🚧 |
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
| Record storage: values as text in the file's locale, long/non-Latin-1 sub-path form, record-id sets, last id, field value indexes (FORMAT.md §13) | ✅ (from records typed in FileMaker) |
| File locale (`[2]` key 13): separators, date order, AM/PM | ✅ 11 languages |
| Repeating fields | ⏳ |
| Field types and options (type, data type, storage, index, auto-enter, validation) | ✅ |
| Calculation token streams → source text (253 functions, 140 `Get()` flags, operators, literals, field refs, Let) | ✅ |
| Calculations in scripts and layouts (contexts other than a table occurrence) | 🚧 |
| Table occurrences (base table, position, colour) and relationships (TOs, join fields, cascade create) | ✅ |
| Relationship options: join operators other than `=`, cascade delete, sorting | 🚧 |
| Layouts: name, table occurrence, width, parts (kind, position, height, style) | ✅ |
| Layout objects: kind, id, bounds, style name, field reference, static text | ✅ |
| Layout object nesting (tab controls, panels, portals, button bars): 501/504 match XML | ✅ |
| Portal definition (table occurrence, rows) | ✅ |
| Layout objects: formatting, button actions, object option bits | 🚧 |
| Scripts: catalog, folders, step list (type, enabled, options) | ✅ |
| Script step parameters | 🚧 |
| Value lists (custom values, from field) | ✅ |
| Themes: compiled styles, named styles, local object styles, fonts (`theme.rs`) | ✅ |
| Custom functions, accounts and privileges | ⏳ |
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
| Functions | 234 / 234 |
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
| JSON (Set/Get/Delete/ListKeys/ListValues/Format/MakeArray/GetElementType, Parse/ParsedState) | ✅ |
| JSONValueExists/ValueCount/AnyValuesExist/RemoveValues/UniqueValues (undocumented; semantics inferred from the signatures) | 🚧 |
| Base64, hex, URL encoding, CryptDigest, CryptAuthCode, TextEncode/Decode (UTF-8) | ✅ |
| Aggregates over lists, repetitions and related records (through `Context`) | ✅ |
| Styled text (TextColor, TextStyleAdd…): plain text passes through, styles dropped; GetAsCSS/GetAsSVG of plain text | 🚧 |
| Design functions (FieldNames, FieldType, RelationInfo, GetTableDDL, LayoutObjectNames…) on `Context::schema` | ✅ |
| Record functions (GetNthRecord, Lookup, LookupNext, GetSummary, Self, Status, GetRecordIDsFromFoundSet) through `Context` | ✅ |
| ExecuteSQL / ExecuteSQLe: single-table SELECT, WHERE, ORDER BY, DISTINCT, COUNT(*), OFFSET/FETCH, `?` arguments, over `Context::rows` | 🚧 no joins, GROUP BY or system tables |
| Japanese: kana width and script, KanjiNumeral, NumToJText, YearName, MonthNameJ (formats partly unverified) | 🚧 |
| CryptGenerateSignature / CryptVerifySignature (RSA PKCS #1 v1.5, plain and DES-EDE3/AES-encrypted PEM keys) | ✅ matches OpenSSL |
| CryptEncrypt / CryptDecrypt (+Base64): PBKDF2 + AES-128-GCM with SHA-256 digest, own byte layout | 🚧 not FileMaker-compatible |
| Embeddings (CosineSimilarity, Add/Subtract/NormalizeEmbedding, GetEmbeddingAsText/File), container size/attributes, path conversion | ✅ |
| Location, sensors, Live Text, AI models, persistent data, layout-object attributes: `Context::host_function`, "not available" (error 3) by default | ✅ |
| Custom functions, file-locale date/number formats | ⏳ |

## M7 — Writing `.fmp12`

| Feature | State |
|---------|-------|
| Chunk encoder (all opcodes) and leaf packing; round-trips through our reader | ✅ |
| Same-size in-place edits accepted by FileMaker | ✅ (verified) |
| Leaves keep FileMaker's exact chunk bytes (key spelling, `80` node markers, empty nodes, trailing pops) | ✅ all 6,824 template leaves re-pack byte for byte |
| Index (separator) sectors: encoding with pop flags, exact first-key separators | ✅ all 66 template roots re-encode byte for byte |
| Full rewrite (`fmdump <file> rewrite <out>`) opens in FileMaker Pro 26 | ✅ 10 templates verified; XML export identical for Contacts, Tasks, Assets (DDR_INFO order and ids aside) |
| Multi-level index trees (level ≥ 2) | 🚧 implemented, no template is deep enough to verify |
| Writing records: add, edit, delete (`fmp12::save::save_records`, `engine::Database::save`) | ✅ verified: FileMaker opens the file and shows the right count, values and related records |
| Bookkeeping: record-id set, last record id, change counter, modification count kept | ✅ |
| Field indexes after edits: dropped and marked for rebuild (FileMaker rebuilds them on demand) | ✅ verified (portal, find, unique key) |
| Writing exact index keys (FileMaker collation tables) | ⏳ |
| Files saved by OpenMaker, edited and saved again by FileMaker, read back | ✅ verified |
| Auto-enter values (serials, UUIDs, stamps) on records created by OpenMaker | ⏳ |
| Repeating fields, container data | ⏳ |

## M3 — Data engine (`crates/engine`)
| Feature | State |
|---------|-------|
| Load tables, fields, records, graph and layouts from `.fmp12` | ✅ |
| Calculated fields: decompiled from the file and evaluated with `calc` | ✅ |
| New record, edit, in memory | ✅ |
| Find requests: words, `=`, `==`, `*`, `@`, `#`, `<` `>` `<=` `>=`, ranges, phrases, omit | ✅ |
| Sort ascending/descending by several fields | ✅ |
| Related records through the relationship graph (multi-hop, `=` joins); related fields and aggregates in calculations | ✅ |
| Portals in the GUI, sorted relationships, other join operators | ⏳ |
| Design functions (TableNames, ValueListItems…), ExecuteSQL, GetNthRecord answered from the database | ✅ |
| Auto-enter on new records (creation/modification stamps, serials, auto-enter calculations), written in the file's date/time formats; verified in FileMaker | ✅ |
| Summary fields, validation enforcement | ⏳ |
| Storage formats of dates/times/numbers in records: text in the file's locale (`typed_in`) | ✅ |
| Stored calculations re-evaluated for new and edited records on save | ✅ |
## M4 — GUI (`crates/app`)
| Menu bar, status toolbar, layout bar (English/French) | ✅ |
| Layout drawn at stored coordinates: parts, fields, text, buttons | ✅ |
| Browse mode: navigate, edit fields, new/delete record, show all | ✅ |
| Find mode with multiple requests and omit requests; Omit Record, Show Omitted Only; sort by one field | ✅ |
| Calculated fields displayed live | ✅ |
| Theme styles decoded (`fmp12::theme`: colours, borders, fonts, padding, alignment) | ✅ |
| Theme styles applied when drawing layouts: backgrounds, borders, radius, text colour, size, alignment, padding | ✅ |
| Button labels (calculations evaluated live), merge fields `<<Field>>` in text | ✅ |
| Fonts (family, weight), icons, states (hover, focus, pressed) | ⏳ |
| Form, List (body repeated per record) and Table views | ✅ |
| Tab controls (clickable tabs, front panel) and portals (rows of related records, editable) | ✅ |
| Related fields outside portals (first related record) | ✅ |
| Pop-up menus and drop-down lists from value lists (custom and field-based) | ✅ |
| Checkboxes, radio buttons, calendars | ⏳ |
| Layout mode, Preview mode | ⏳ |
| Web version (WebAssembly, GitHub Pages): open or drop a file, everything runs locally | ✅ |
| Save from the app: Ctrl+S / Save a Copy As (desktop), download (web) | ✅ |

## Automation (`crates/cli`)

| Feature | State |
|---------|-------|
| `openmaker-cli`: info, records (find/omit/sort/limit), eval, new, set, delete, JSON output | ✅ |
| Session mode (JSON-lines on stdin/stdout) with save | ✅ |
| Run scripts from the CLI | ⏳ (after M5) |
| MCP server for AI agents (`openmaker-cli mcp`): open, describe, find, evaluate, create/update/delete, save | ✅ |
