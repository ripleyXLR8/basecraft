# Progress

**Estimated progress to 1.0: ~84 %**, weighted by milestone (see the README table).

Legend: ✅ done and tested · 🚧 in progress · ⏳ not started

## Milestones

| # | Milestone | State |
|---|-----------|-------|
| M0 | Bootstrap: workspace, CI, docs | ✅ |
| M1 | Read `.fmp12`: schema, data, layouts, scripts | 🚧 |
| M2 | Calculation engine | 🚧 |
| M3 | Data engine: CRUD, find, sort, relationships, calc and summary fields | 🚧 |
| M4 | GUI: Browse / Find / List / Table views | 🚧 |
| M5 | Script engine and Script Workspace | 🚧 |
| M6 | Layout mode | 🚧 (most of FileMaker's Layout mode; data formats, web viewers, charts, button bars to come) |
| M7 | Write `.fmp12` files that FileMaker Pro opens | 🚧 |
| M8 | Import/export, Preview/print/PDF, charts, security, Data Viewer | 🚧 |
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
| Repeating fields: repetition keys (`07 02`, `80 03 03`), long values, `value_rep`, `Field[n]`, GetRepetition, Last, Extend, repeating calculations | ✅ (from records set in FileMaker) |
| Field types and options (type, data type, storage, index, auto-enter, validation) | ✅ |
| Calculation token streams → source text (253 functions, 140 `Get()` flags, 29 design and location functions, operators, literals, field refs, Let, comments) | ✅ |
| Calculations in scripts and layouts (contexts other than a table occurrence) | 🚧 |
| Table occurrences (base table, position, colour) and relationships (TOs, join fields, cascade create) | ✅ |
| Relationship options: all join operators, cascade create and delete, sorting of either side | ✅ (decoded from FileMaker 26) |
| Layouts: name, table occurrence, width, parts (kind, position, height, style) | ✅ |
| Layout objects: kind, id, bounds, style name, field reference, static text | ✅ |
| Layout object nesting (tab controls, panels, portals, button bars): 501/504 match XML | ✅ |
| Portal definition (table occurrence, rows) | ✅ |
| Layout objects: button actions (single step or Perform Script with parameter) | ✅ |
| Layout objects: option bits (anchors, lock, sliding, hide when printing), child lists, portal, tab control and button fields | ✅ |
| Layout objects: field data formats (node key 13) | ⏳ |
| Scripts: catalog, folders, step list (type, enabled, options), long step lists | ✅ |
| Script step parameters: control flow, variables, fields, layouts, records, finds, sorts, dialogs (see M5) | ✅ |
| Script step parameters: other steps (windows, import/export, files…) | ⏳ |
| Value lists (custom values, from field: second field, related only, sort by second field) | ✅ |
| Themes: compiled styles, named styles, local object styles, fonts (`theme.rs`) | ✅ |
| Custom functions, accounts and privileges | ⏳ |
| Container data: embedded files and pictures (store `[T].[31]`, streams FNAM, PNGf, JPEG, GIFf, FILE, SIZE, DPI_), references by path; GetContainerAttribute, GetAsText, Base64Encode | ✅ (from containers inserted in FileMaker; `all` matches) |
| Container data stored externally (secure or open storage), thumbnails | ⏳ |

Oracle: all 214 template fields and 32 graph objects, 30 scripts, 85 script steps 24 value lists, 24 layouts, 504 layout objects and 31/31 field calculations (decompiled text) match FileMaker's XML export. Step parameters: 203 steps (319 parameter values) and 108 button actions match, templates plus a 57-step script pasted into a copy of Tasks.

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

Oracle: **1,987 expressions** evaluated by FileMaker Pro 26.0.1 (Windows, English Tasks
template, en_US formats) in [crates/calc/tests/oracle.tsv](crates/calc/tests/oracle.tsv),
produced by `tools/oracle/calc_oracle.ps1`; `cargo test -p calc --test oracle` checks
them all. 1,934 match; 53 are listed as known differences (see below).

| Feature | State |
|---------|-------|
| Lexer, parser with error positions and FileMaker's error codes (1204, 1208, 1212...), exponent literals (`1e3`) | ✅ oracle |
| Operators, precedence (unary minus before `^`, `not` before comparisons), coercions (text vs number compares as text) | ✅ oracle |
| Decimal arithmetic: exact + - *, division and math functions with 2 guard digits beyond the 16 shown (`1/3*3` = 1) | ✅ oracle |
| Number display: no leading zero (`.5`), `1.0E+9` / `1.5E-10` notation, 16 decimals | ✅ oracle |
| Text to number (exponents, `(5)` negative, full-width digits) | ✅ oracle |
| If / Case / Choose / Let / While / Evaluate / EvaluationError / IsValid / IsValidExpression / SetPrecision | ✅ oracle |
| Text, value-list and word functions (non-overlapping Position/PatternCount, Proper, Trim widths, LF/CR LF value breaks, word rules, accent-aware sorting) | ✅ oracle |
| Number, financial and math functions (errors 14/15/1218, Factorial/Combination domains, Mod by 0) | ✅ oracle |
| Date, time and timestamp functions (en_US): 12-hour timestamps, 6-digit time fractions, two-digit year window, errors 500/501 | ✅ oracle |
| JSON (Set/Get/Delete/ListKeys/ListValues/Format/MakeArray/GetElementType, Parse/ParsedState): JsonCpp number text and error messages | ✅ oracle |
| JSONValueExists/ValueCount/AnyValuesExist/RemoveValues/UniqueValues (undocumented): array values, case-insensitive text match | ✅ oracle |
| Design function formats (RelationInfo, GetTableDDL, FieldType, GetFieldsOnLayout; empty text for a missing file/field) | ✅ checked against FileMaker by hand (template schema not committed) |
| Base64 (CRLF after every line), hex, URL encoding, CryptDigest, CryptAuthCode, TextEncode/Decode (UTF-8, ISO-8859-1, windows-1252) | ✅ oracle |
| Aggregates over lists, repetitions and related records (through `Context`) | ✅ |
| Styled text (TextColor, TextStyleAdd…): plain text passes through, styles dropped; GetAsCSS/GetAsSVG of plain text (✅ oracle) | 🚧 |
| Design functions (FieldNames, FieldType, RelationInfo, GetTableDDL, LayoutObjectNames…) on `Context::schema` | ✅ |
| Record functions (GetNthRecord, Lookup, LookupNext, GetSummary, Self, Status, GetRecordIDsFromFoundSet) through `Context` | ✅ |
| ExecuteSQL / ExecuteSQLe: single-table SELECT, WHERE, ORDER BY, DISTINCT, COUNT(*), OFFSET/FETCH, `?` arguments, over `Context::rows` | 🚧 no joins, GROUP BY or system tables |
| Japanese: kana width and script, KanjiNumeral, NumToJText, YearName, MonthNameJ, TrimAll types | ✅ oracle |
| CryptGenerateSignature / CryptVerifySignature (RSA PKCS #1 v1.5, plain and DES-EDE3/AES-encrypted PEM keys) | ✅ matches OpenSSL |
| CryptEncrypt / CryptDecrypt (+Base64): PBKDF2 + AES-128-GCM with SHA-256 digest, own byte layout | 🚧 not FileMaker-compatible |
| Embeddings (CosineSimilarity, Add/Subtract/NormalizeEmbedding, GetEmbeddingAsText/File), container size/attributes, path conversion | ✅ |
| Location, sensors, Live Text, AI models, persistent data, layout-object attributes: `Context::host_function`, "not available" (error 3) by default | ✅ |
| Custom functions, file-locale date/number formats | ⏳ |

Known differences (rows after `# known differences` in oracle.tsv):

* Times keep no "12-hour" flag: FileMaker shows `GetAsTime ( "1:30 PM" )` as `1:30 PM`, we show `13:30:00`; dates from text keep their separator in FileMaker (`1.15.2024`).
* Trigonometry differs in the last shown digit for about 1 argument in 5 (`Sin ( Pi / 6 )`); `Exp` of large arguments (FileMaker's `Exp ( 100 )` is off from the exact value; `Exp ( 1000 )` exceeds our 400 digits).
* Guard digits beyond the 16 shown, visible only through `* 10^20` probes, differ for transcendental functions.
* Styled text (no style runs), lone UTF-16 surrogates from `Char`, JSON duplicate keys, Shift_JIS/UTF-16 TextEncode, Windows-specific path conversion, and a few error codes that come with a value (`Count ( 1 ; 1/0 )`).

## M5 — Script engine (`crates/script`)

Step catalogue: [docs/reference/script_steps.txt](docs/reference/script_steps.txt),
222 step ids (1–247) named by FileMaker Pro 26's XML export (reverse oracle,
`tools/oracle/step_ids.py`).

| Feature | State |
|---------|-------|
| Step catalogue (id → name) | ✅ 222 steps |
| Typed step parameters (`fmp12::scripts::StepParams`), raw tree for the rest | ✅ |
| Control flow: If / Else If / Else / End If, Loop / Exit Loop If / End Loop, Halt Script | ✅ |
| `$local` (per call) and `$$global` variables, repetitions | ✅ |
| Perform Script with parameter, Exit Script with result, nested calls; `Get ( ScriptParameter / ScriptResult / ScriptName )` | ✅ |
| `Get ( LastError )`, Set Error Capture (host shows errors when off), Allow User Abort | ✅ |
| Set Field (current or first related record), Insert Text (field or variable), Commit, Revert Record | ✅ |
| New / Duplicate / Delete Record, Delete All Records, Go to Record (first, last, previous, next with exit after last, by number) | ✅ |
| Enter Browse / Find / Preview Mode, find requests built by Set Field in Find mode, Perform Find (stored requests, omit requests) | ✅ |
| Show All Records, Show Omitted Only, Omit Record, Omit Multiple Records | ✅ |
| Sort Records (stored keys, ascending, descending, custom value-list order), Unsort Records | ✅ |
| Go to Layout (layout, original, by name, by number); one found set per window | ✅ |
| Show Custom Dialog (title, message, 3 buttons, input field 1), Go to Field, Go to Object, Refresh/Freeze Window, Beep through the `Host` trait | ✅ |
| Button actions (`Engine::run_step`) | ✅ |
| Perform Find/Replace (all four operations, direction, current record, match case, whole word), Replace Field Contents (current contents, serial numbers with Entry Options, calculation, auto-enter): parameters decoded from steps made in FileMaker 26 | ✅ |
| Template scripts run in tests: Contacts and Tasks "Sort by Name", "Sort by Status", "Create Assignee Entry", "Cancel Add Assignee", "Go to Entry Field" | ✅ |
| Windows (New/Close/Select Window), Go to Related Record, Go to Portal Row, Select All, finds on related fields | ⏳ |
| Resumable runs (`Engine::start`, `Run::resume` → `Suspended(Dialog \| Pause)` / `Finished`): Show Custom Dialog, delete confirmations, Pause/Resume Script, modes entered with Pause wait for the user; no threads (wasm32) | ✅ |
| `perform` / `run_step` run to the end (dialogs through `Host::dialog`, button 1 by default): CLI and MCP unchanged | ✅ |
| `Get ( LastErrorLocation / LastErrorDetail / ScriptAnimationState / Trigger… )` (trigger values set by the host) | ✅ |
| Step text as in the Script Workspace (`script::text`), block indentation | ✅ |
| French step names and step text (`script::text::step_name_fr`, `step_text_in`): the 222 step names, palette categories and step labels as FileMaker 26's French UI shows them (`Définir rubrique [Tasks::Description; "abc"]`), read from its Script Workspace through UI Automation; calculations keep English function names | ✅ |
| Script triggers (`fmp12::triggers`): layout, object and file triggers, 26 event ids, modes (Browse 1, Find 2, Preview 4) and parameters decoded from triggers set in FileMaker 26; 25 triggers of the oracle files match the XML (event, script, modes, parameter) | ✅ read; writing ⏳ |
| Calculation compiler (`fmp12::calc_compile`, one compiler for fields and scripts): source tokens (key 5) and prefix form (key 6); all 5,214 template calculations, comments included: key 6 derived from key 5 identical, decompiled text compiles back to both keys | ✅ |
| Writing scripts (`fmp12::scripts_save::save_scripts`): step lists and parameter trees of existing scripts; 224 template and oracle scripts re-encode byte for byte, 886 typed steps encode back from their typed parameters | ✅ verified in FileMaker (`tools/oracle/script_check.ps1`): edited, added and disabled steps, If/Else, Set Variable, Go to Layout, Perform Script, Show Custom Dialog, Let/Case/Choose/List/`$v[2]` show as written and evaluate |
| New, deleted and renamed scripts | ✅ verified in FileMaker: `[17].[1]` key 8 is a plain MD5 integrity digest of the script catalog (FORMAT.md §8), recomputed on save; it reproduces all 66 templates and 395 sample files. A Tasks copy with scripts added, renamed and deleted, and a `basecraft-cli new-file` file with the same, open without warning, list the new names and run the new scripts |
| Script debugger, Pause/Resume duration storage, script folders created in the workspace, script order changes by dragging | ⏳ |
| Builds for `wasm32-unknown-unknown` | ✅ |
## M6 — Layout mode (`crates/app/src/design.rs`, `fmp12::save::save_layouts`)

| Feature | State |
|---------|-------|
| Enter/leave: Edit Layout button, View › Layout Mode, Ctrl+L; "Save changes to layout…?" (Save / Don't Save / Cancel) on exit or layout switch | ✅ |
| Canvas: rulers, grid, part labels in the left margin (English/French names), objects drawn with their theme look, field names in fields | ✅ |
| Selection: click, Shift-click, marquee; handles; move with the mouse or the arrow keys (Shift = 10 pt); resize from the handles; Delete | ✅ |
| Undo/redo within the session (Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z, Edit menu) for every layout edit | ✅ |
| Edit menu: Cut, Copy, Paste (also across layouts), Duplicate (Ctrl+D), Delete, Select All | ✅ |
| Arrange menu: Bring to Front / Forward, Send Backward / to Back; Align (edges, centres); Distribute (horizontally, vertically); Resize To (smallest/largest width, height); Group / Ungroup (group objects, kind 8); Lock / Unlock | ✅ |
| Objects dropped into a portal or the front tab panel go inside it; dragged out, back to the layout | ✅ |
| Parts: resize by dragging a boundary or in the Inspector; Insert › Part… (header, footer, title header/footer, leading/trailing grand summary, leading/trailing sub-summary by a field) at its place in FileMaker's part order; select a part label and Delete (its objects go too) | ✅ verified in FileMaker |
| Layout bar: layout pop-up, New Layout/Report, Layout Setup (name, table occurrence), Exit Layout | ✅ |
| Layouts menu: New Layout/Report… (name, occurrence), Duplicate Layout, Delete Layout…, Layout Setup…; written to the file at once (`schema_edit::Editor::new_layout`, `duplicate_layout`, `delete_layout`, `rename_layout` with its collation-keyed name index) | ✅ verified in FileMaker |
| Tools and Insert menu: selection, text (edited in place), line, rectangle, oval, field (Specify Field, optional label), button (Go to Layout or Perform Script with a label; double-click to edit), portal (related occurrence, rows, fields placed in the first row), tab control (one panel per tab name) | ✅ verified in FileMaker |
| Inspector › Position: name, left/top/right/bottom/width/height in points, autosizing anchors, lock, slide left, slide up, hide when printing (object option bits, decoded in FileMaker) | ✅ |
| Inspector › Styles: theme, named styles of the object's type, clear local changes | ✅ |
| Inspector › Appearance: fill, line (style none/solid/dashed/dotted/double, colour, width), corner radius; text font (font catalog), size, bold, italic, underline, strikethrough, colour, alignment (left, centre, right, justify) — written as local `.self` properties FileMaker reads (XML `LocalCSS`) | ✅ verified in FileMaker |
| Inspector › Data: field, control style (edit box, drop-down list, pop-up menu, checkbox set, radio button set, drop-down calendar), value list; portal occurrence, first row, rows | ✅ |
| Inspector › Data: number and date display formats (object node key 13) | ⏳ kept as they are, not decoded |
| Rulers, grid (show, snap to grid), guides dragged from the rulers (snap within 5 pt; dragged back onto the ruler to remove) | ✅ (guides are kept for the session; FileMaker stores its own guides in a place not decoded) |
| Save to `.fmp12`: object list, container child lists (field 6), per-object nodes (text, name, button/tab labels, UUIDs), parts (added, removed, reordered) and their object lists, layout name, table occurrence, width, last object id, spatial index and tab order | ✅ verified in FileMaker |
| Protobuf encoder (`pb::Msg::encode`, `pb::MsgBuf`): all 418 template layouts and 5,544 objects re-encode byte for byte | ✅ |
| Layout object nesting read from the containers' child lists: 633/633 objects match the XML | ✅ |
| Web viewer, chart, popover, button bar and slide control tools; tab order editing; conditional formatting; script triggers on objects; sliding "only objects directly above" | ⏳ |

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
| Files saved by BaseCraft, edited and saved again by FileMaker, read back | ✅ verified |
| Auto-enter values (serials, UUIDs, stamps) on records created by BaseCraft | ⏳ |
| Repeating fields, container data | ⏳ |
| Calculation compiler: source text → token stream (key 5) and compiled form (key 6) FileMaker evaluates (`fmp12::calc::compile`) | ✅ 1,890 FileMaker-compiled calculations and all 5,214 template calculations (fields, scripts, layouts, comments included) byte for byte; one compiler for schema and scripts |
| Scripts (`fmp12::scripts_save`): edited step lists and parameters | ✅ verified in FileMaker: opens without warning, Save a Copy as XML shows the steps as written, the calculations evaluate |
| New, deleted, renamed scripts | ✅ verified in FileMaker: `[17].[1]` key 8 (MD5 over the catalog members' names in id order, FORMAT.md §8) written by `save_scripts`; non-Latin names in the long form. The accounts/privileges catalogs (`[23]`) carry their own digests and are not written by BaseCraft |
| Name indexes: collation keys of names (423 characters; `fmp12::collate`) | ✅ all 4,097 template name-index keys |
| Schema editing (`fmp12::schema_edit`): add, rename, delete tables (with occurrence and layout) and fields; every data type; calculated and summary fields; auto-enter (serial, stamps, constant, calculation), validation (not empty, unique, existing, range, calculation), storage (global, repetitions, indexing) | ✅ verified in FileMaker: opens without warning, Manage Database and Save a Copy as XML show the tables, fields and options, records created in FileMaker get serials, stamps, constants and calculated values |
| New empty file (`schema_edit::new_file`, from `assets/blank.fmp12` with BaseCraft's own theme) | ✅ verified in FileMaker |
| Table occurrences: add (several per table, any name, placed on the graph), rename, delete | ✅ verified in FileMaker |
| Relationships: add, edit, delete; one or more predicates with `=`, `≠`, `<`, `≤`, `>`, `≥`, `×`; create and delete related records on either side; sorted related records | ✅ verified in FileMaker: Manage Database shows the graph, XML shows every option, records created in FileMaker see their related records |
| Value lists: create, edit, delete; custom values, values from a field (second field, show only the second, sort by the second, related only) | ✅ verified in FileMaker (Manage Value Lists, `ValueListItems`) |
| Lookups (relationship, source field, no match: nothing / next lower / next higher / constant, copy empty values) | ✅ verified in FileMaker: lookups fire on creation and on Set Field |
| Validation: member of value list, maximum characters, custom messages; serial increment ≠ 1; summary options (running total and count, restart per sort group, weighted average, fraction of total subtotaled) | ✅ verified in FileMaker |

## M3 — Data engine (`crates/engine`)
| Feature | State |
|---------|-------|
| Load tables, fields, records, graph and layouts from `.fmp12` | ✅ |
| Calculated fields: decompiled from the file and evaluated with `calc` | ✅ |
| New record, edit, in memory | ✅ |
| Find requests: words, `=`, `==`, `*`, `@`, `#`, `<` `>` `<=` `>=`, ranges, phrases, omit | ✅ |
| Sort ascending/descending by several fields | ✅ |
| Related records through the relationship graph (multi-hop, every join operator, relationship sort order); related fields and aggregates in calculations | ✅ |
| Lookups applied when a match field changes (`Database::set_values`, the one path for every caller): new records, import, fields left in the app (form and table views), Set Field, Set Field By Name, Insert Text / Calculated Result / Current Date, Time, User Name, Paste, Replace Field Contents; Relookup Field Contents (`Database::relookup`) over the found set; next lower/higher value | ✅ (parameter storage of the steps other than Set Field and Insert Text inferred, not seen in a file) |
| Cascade delete of related records (`Database::delete_record`); serial increments | ✅ |
| Value lists from fields: second field sort, related values only (`value_list_values_from`) | ✅ |
| Running total and count (restarting for each sorted group), weighted average, fraction of total subtotaled by a field, standard deviation by population (`summary_at` for a record, `summarize_sorted` for a group): in records, `GetSummary`, Preview/print sub-summaries and grand summaries, exports, the CLI's `records` | ✅ checked in FileMaker's Table view (`tools/oracle/summary_demo.sh`): every value unsorted and sorted, `GetSummary` by the break field; List is operation 9, by population is option 0x01 |
| Design functions (TableNames, ValueListItems…), ExecuteSQL, GetNthRecord answered from the database | ✅ |
| Auto-enter on new records (creation/modification stamps, serials, auto-enter calculations), written in the file's date/time formats; verified in FileMaker | ✅ |
| Summary fields (Total, Average, Count, Min, Max, StDev, Fraction of Total, List) over a found set or break groups (`engine::summary`) | ✅ |
| Validation enforced on commit, in the app (dialog, "Accept anyway" when allowed) and in scripts (record stays open, `Get ( LastError )`: 502 number, 504 unique, 505 existing, 506 value list, 509 not empty, 511 maximum characters; Commit Records' "skip data entry validation" keeps "always" fields): not empty, unique, existing, strict number, member of value list, maximum characters; custom messages (a calculation) replace FileMaker's text | ✅ (error codes from FileMaker's documentation, not checked in FileMaker) |
| Validation by range and by calculation | ⏳ |
| Storage formats of dates/times/numbers in records: text in the file's locale (`typed_in`) | ✅ |
| Stored calculations re-evaluated for new and edited records on save | ✅ |
| Schema changes in memory (`Database::add_table`, `add_field`, `set_field`, `add_occurrence`, `add_relationship`, `add_value_list`, `rename_*`, `set_*`, `delete_*`) with unsaved records kept; `save` writes records and moves the next serial value | ✅ |
## M8 — Import/export, Preview and PDF

| Feature | State |
|---------|-------|
| Export Records (`engine::export`): found set in sort order, chosen fields, related fields, formatted or as stored | ✅ |
| Comma-separated text: every value quoted, CR LF between records, returns as VT (0x0B), repetitions and related records joined with GS (0x1D) | ✅ |
| Tab-separated text (tabs in values become spaces), Merge (`.mer`, field-name row) | ✅ |
| FMPXMLRESULT XML (ERRORCODE, PRODUCT, DATABASE, METADATA, RESULTSET; one DATA per related record) | ✅ |
| Excel `.xlsx` (hand-written OOXML, numbers and Excel dates typed, header row) and JSON | ✅ |
| HTML Table (`.htm`): ISO-8859-1, CR LF, field-name row, one row per record, returns and empty values as `<BR>` | ✅ byte for byte with a FileMaker 26 export |
| Grouped export ("Group by" sort fields): one row per group with its first record's values, summary fields' grand total on the first row only | ✅ as FileMaker 26 exports; the automatic "Summary by field" subtotal columns are not added yet |
| Layout data formats, repeating fields, container data, DBF and FileMaker Pro export formats | ⏳ |
| File › Export Records… dialog (Specify Field Order for Export: current layout and related occurrences, Move / Move All / Clear All, double-click, reorder, Group by with "(Unsorted)", Apply current layout's data formatting, type: tab, CSV, Merge, HTML, XML, Excel) | ✅ compared with FileMaker 26's dialog |
| Import Records (`engine::import`): CSV, tab, Merge, `.xlsx`; mapping by name or order; add, update matching (match fields, add remaining), replace found set | ✅ |
| Import: auto-enter on or off, validation reported, records failing always validation skipped | ✅ |
| Import from a FileMaker file (`engine::import::fmp12_source`: every table, values converted to the target file's formats) | ✅ |
| File › Import Records › File… dialog (Import Field Mapping: source record browser, Import / `=` / Don't import per field, target by matching names or creation order, Add / Update / Replace with "add remaining", auto-enter on or off, Import Summary; the imported records become the found set) | ✅ compared with FileMaker 26's dialog |
| Import from XML, folders, ODBC; character sets other than UTF-8; field delimiter choice | ⏳ |
| Summary field definitions (field key 9) and sub-summary break fields (part field 6) decoded | ✅ (operation numbers and options checked in FileMaker's Table view) |
| Preview pagination (`crates/print`): header/footer per page, title header/footer on page 1, body per record, leading/trailing sub-summaries by sorted break field, grand summaries, letter/A4, landscape, margins | ✅ |
| PDF output: hand-written PDF 1.4, Helvetica (WinAnsi), theme backgrounds, borders, text colours, sizes, bold, alignment, padding; fields, text with merge fields and `{{PageNumber}}`, button labels, portals | ✅ |
| Part options (page break before/after, allow break across pages), sliding, layout number/date formats, images, Unicode fonts (non-Latin scripts) | ⏳ |
| GUI Preview mode | ⏳ |
| CLI `export`, `import`, `print-pdf`; MCP `export_records`, `import_records`, `print_pdf` | ✅ |
| Builds for `wasm32-unknown-unknown` (`engine`, `print`) | ✅ |
| Charts (`fmp12::charts`, FORMAT.md §10): object kind 13 (`CHRT`), title, axis titles, X data, Y series and names, bubble size, type (column, bar, line, area, pie, positive/negative, stacked column, stacked bar, bubble, scatter), data source (found set, current record delimited values, related records), legend, data points, colour scheme | ✅ 14 charts made in FileMaker 26 decoded (`tests/charts.rs`) |
| Charts drawn in Browse, Preview (`app::chart`) and PDF (`print::chart`, one layout for both): all 10 types, title, legend, ticks, value labels, Earth and Bright palettes | ✅ compared with FileMaker screenshots |
| Chart fonts, custom series colours, gridlines, legend position, axis ranges and formats, other colour schemes, "summary groups when sorted" | ⏳ |
| Security (accounts, privileges) | ⏳ |

## M4 — GUI (`crates/app`)
| Menu bar, status toolbar, layout bar (English/French) | ✅ |
| Layout drawn at stored coordinates: parts, fields, text, buttons | ✅ |
| Browse mode: navigate, edit fields, new/delete record, show all | ✅ |
| Records menu as in FileMaker 26: New, Duplicate (Ctrl+D), Delete Record… (Ctrl+E) and Delete All Records… with FileMaker's confirmations (related records deleted by cascade), Show All (Ctrl+J), Show Omitted Only, Omit Record (Ctrl+T), Omit Multiple… (Ctrl+Shift+T), Modify Last Find (Ctrl+R), Replace Field Contents… (Ctrl+=) | ✅ |
| Replace Field Contents (`engine::replace`): current contents, serial numbers (prefix and leading zeros kept, "Update serial number in Entry Options"), calculated result; auto-enter options on or off | ✅ |
| Edit › Find/Replace (Ctrl+Shift+F, modeless): Find Next, Replace & Find, Replace, Replace All; direction forward / backward / all; match case, whole word; all records or current record; all fields or current field | ✅ compared with FileMaker 26's dialog |
| Go to Record, Saved Finds, Sort Records dialog, Unsort, Relookup and Revert Record in the Records menu | ⏳ |
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
| Layout mode (see M6) | 🚧 |
| Preview mode | ⏳ |
| Web version (WebAssembly, GitHub Pages): open or drop a file, everything runs locally | ✅ |
| Save from the app: Ctrl+S / Save a Copy As (desktop), download (web) | ✅ |

## Automation (`crates/cli`)

| Feature | State |
|---------|-------|
| `basecraft-cli`: info, records (find/omit/sort/limit), eval, new, set, delete, JSON output | ✅ |
| Session mode (JSON-lines on stdin/stdout) with save | ✅ |
| Run scripts from the CLI and MCP (`run-script`, `run_script`, `list_scripts`), dialogs answered automatically | ✅ |
| MCP server for AI agents (`basecraft-cli mcp`): open, describe, find, evaluate, create/update/delete, save | ✅ |
| Manage Database from the CLI: `schema add-table`, `add-field`, `set-options`, `add-occurrence`, `add-relationship`, `set-relationship`, `add-valuelist`, `set-valuelist`, `rename`, `delete`, `new-file`; MCP `new_file`, `create_table`, `create_field` (lookups, validation, summaries), `create_occurrence`, `create_relationship`, `create_valuelist` | ✅ |

### Scripts in the GUI

| Feature | State |
|---------|-------|
| Scripts menu runs the file's scripts | ✅ |
| Buttons run their action (single step or Perform Script) | ✅ |
| Custom dialogs: modal with title, message, input fields and buttons (button 1 on the right, Enter); the script waits | ✅ |
| Pause/Resume Script: "Script paused" with Continue / Cancel in the status area; timed pauses continue by themselves | ✅ |
| Script triggers run: OnLayoutEnter/Exit, OnRecordLoad, OnModeEnter, OnRecordCommit (user actions and layout changes by scripts), OnObjectEnter/Modify/Exit on fields, OnFirstWindowOpen; queued, one at a time, no trigger loops | ✅ (Contacts sorts by name when opened) |
| Script Workspace (Scripts › Script Workspace…, Ctrl+Shift+S): script list with folders, step text with indentation and greyed disabled steps, palette by category with search, add / delete / duplicate / disable / move (drag or buttons), new / rename / delete script, Run | ✅ |
| Workspace step editors: Set Field, Set Variable, If / Else If / Exit Loop If, Go to Layout, Go to Record, Perform Script, Show Custom Dialog, Sort, Perform Find, Enter Find Mode, Comment, Commit, Insert Text, Go to Field/Object, Pause/Resume, Refresh Window; calculation editor with fields, functions and Get flags | ✅ |
| Data Viewer (Tools menu): watch expressions evaluated in the window's context (Get functions, $$ globals) | ✅ |
| Script Workspace and Data Viewer in their own OS windows (egui immediate viewports) on desktop, with title, icon, size remembered between openings, Ctrl+Shift+S / menu bring them to the front, closing the window closes them, Ctrl+S in the workspace saves its scripts; in-page windows in the browser and the UI tests | ✅ |
| UI driven headless through `egui_kittest` (`crates/app/src/ui_tests.rs`): workspace edit / run / save, dialog and pause, Data Viewer, Preview, open trigger, Layout mode (canvas clicks and drags, Inspector, Arrange, Insert and Layouts menus); `BASECRAFT_SHOTS=<dir>` also renders PNGs | ✅ |
| Script debugger | ⏳ |

Layout writing check (2026-11-27): a file whose "Contact List" layout BaseCraft
edited (object moved, text changed, object deleted, body taller, new text,
rectangle and button) opens in FileMaker Pro 26 without a warning. FileMaker's
Layout mode shows every edit.

Layout mode check (2026-11-27, `tools/oracle/layout_check.ps1`,
`Test-FmLayout`, on the file `BASECRAFT_LAYOUT_DEMO=<out> cargo test -p
basecraft layout_demo_file` builds from the Contacts template through the
designer): FileMaker Pro 26 opens it without a warning; the layout pop-up
shows the new layout and the duplicated, renamed one, not the deleted one;
Browse and Layout mode show the added parts (leading and trailing grand
summary, footer), fields with labels, a pop-up menu, a portal on a related
occurrence with two fields, a tab control with two tabs and a field on the
first, a button with its label running a script, underlined and bold
italic red centred text, a rounded locked rectangle, an oval and a line in
a group; Save a Copy as XML lists them with their kinds and options.

Schema writing check (2026-11-27, `tools/oracle/schema_demo.sh` +
`Test-FmSchema` in `schema_check.ps1`): a file made by `basecraft-cli new-file`
and 26 `schema` commands (a table with 17 fields of every kind and option,
a field set, renamed and deleted, a table added, renamed and deleted) opens in
FileMaker Pro 26 without a warning. Manage Database lists both tables and all
17 fields with their types and options (serial, stamps, constant, auto-enter
calculations, range and calculated validation, global, repetitions, index,
stored and unstored calculations, Total and Count summaries); Save a Copy as
XML shows the same schema with the calculation texts; two records created in
FileMaker on the new table's layout get serials 1 and 2, stamps, the constant,
a UUID, `Code` = `FR-1` and `Label` = `France #1 first` / `France #2 many`.
A copy of the Tasks template with a table added (a calculation referring to
`Tasks::Task`), a Tasks field renamed and a field deleted also opens without a
warning, and its XML export shows the new table, its layout and the renamed field.

Relationship writing check (2026-11-27, `tools/oracle/graph_demo.sh`,
`template_demo.sh` + `Test-FmGraph` in `schema_check.ps1`): a file made
by `basecraft-cli new-file` and 37 `schema` commands (Clients and
Invoices, two more occurrences named differently from their tables,
three relationships using all seven operators with create, delete and
sorts on either side, two lookups, four value lists, value-list,
size and message validation, a serial by 5, four summary options; an
occurrence, a relationship and a value list added, edited and deleted)
opens in FileMaker Pro 26 without a warning. Manage Database's
Relationships tab draws the graph, Manage Value Lists lists the four
lists with their values, and Save a Copy as XML shows every option as
written. Records created in FileMaker see the related records
(`Count ( Invoices::Id )` = 2, `List ( Invoices::Amount )`, `Clients::Name`),
lookups copy the client's name and city when the record is created and
when a script sets the match field (and the no-match constant), the
serial goes 1, 6, 11, 16 and `ValueListItems` returns the custom and
field values. The same commands on a copy of the Tasks template (a Notes
table related to Tasks, a lookup of the task's name, a `×` relationship,
two value lists, a renamed field) give the same results.

## Distribution

| Feature | State |
|---------|-------|
| Release workflow (tag `vX.Y.Z`): Windows x64/ARM64 `.msi` + portable `.zip`, Linux x86_64/aarch64 `.AppImage` + `.deb` + `.tar.gz`, macOS universal `.dmg` (unsigned), `SHA256SUMS.txt` | ✅ dry run green on all 5 targets |
| Original app icon, window and web favicon | ✅ |
| Workspace lints (no `unsafe`, no `unwrap`/`expect`/`panic` outside tests), `cargo deny` in CI | ✅ |
| First public release | ⏳ waiting for the owner's go |
| ArtCraft launcher community listing | ⏳ draft request to be reviewed by the owner |
