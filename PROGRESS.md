# Progress

**Estimated progress to 1.0: ~63 %**, weighted by milestone (see the README table).

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
| M6 | Layout mode | 🚧 |
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
| Layout objects: button actions (single step or Perform Script with parameter) | ✅ |
| Layout objects: formatting, object option bits | 🚧 |
| Scripts: catalog, folders, step list (type, enabled, options), long step lists | ✅ |
| Script step parameters: control flow, variables, fields, layouts, records, finds, sorts, dialogs (see M5) | ✅ |
| Script step parameters: other steps (windows, import/export, files…) | ⏳ |
| Value lists (custom values, from field) | ✅ |
| Themes: compiled styles, named styles, local object styles, fonts (`theme.rs`) | ✅ |
| Custom functions, accounts and privileges | ⏳ |
| Container data | ⏳ |

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
| Template scripts run in tests: Contacts and Tasks "Sort by Name", "Sort by Status", "Create Assignee Entry", "Cancel Add Assignee", "Go to Entry Field" | ✅ |
| Windows (New/Close/Select Window), Go to Related Record, Go to Portal Row, Select All, finds on related fields | ⏳ |
| Resumable runs (`Engine::start`, `Run::resume` → `Suspended(Dialog \| Pause)` / `Finished`): Show Custom Dialog, delete confirmations, Pause/Resume Script, modes entered with Pause wait for the user; no threads (wasm32) | ✅ |
| `perform` / `run_step` run to the end (dialogs through `Host::dialog`, button 1 by default): CLI and MCP unchanged | ✅ |
| `Get ( LastErrorLocation / LastErrorDetail / ScriptAnimationState / Trigger… )` (trigger values set by the host) | ✅ |
| Step text as in the Script Workspace (`script::text`), block indentation | ✅ |
| Script triggers: layouts decoded (`fmp12::triggers`, 8/8 OnLayoutEnter of the oracle files match the XML); object and file triggers, event ids other than 101 guessed | 🚧 |
| Calculation compiler (`fmp12::calc_compile`): source tokens (key 5) and prefix form (key 6); all 5,200 template calculations: key 6 derived from key 5 identical, key 5 round-trips through decompile | ✅ |
| Writing scripts (`fmp12::scripts_save::save_scripts`): step lists, parameter trees, new/renamed/deleted scripts; 224 template and oracle scripts (1,138 steps) re-encode byte for byte, 886 typed steps encode back from their typed parameters | ✅ tested by re-reading; 🔍 to verify in FileMaker |
| Script debugger, Pause/Resume duration storage, script folders created in the workspace, script order changes by dragging | ⏳ |
| Builds for `wasm32-unknown-unknown` | ✅ |
## M6 — Layout mode (`crates/app/src/design.rs`, `fmp12::save::save_layouts`)

| Feature | State |
|---------|-------|
| Enter/leave: Edit Layout button, View › Layout Mode, Ctrl+L; "Save changes to layout…?" (Save / Don't Save / Cancel) on exit or layout switch | ✅ |
| Canvas: rulers, grid, part labels in the left margin (English/French names), objects drawn with their theme look, field names in fields | ✅ |
| Selection: click, Shift-click, marquee; handles; move with the mouse or the arrow keys (Shift = 10 pt); resize from the handles; Delete | ✅ |
| Undo/redo within the session (Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z, Edit menu) | ✅ |
| Part resizing by dragging a part boundary (parts and objects below follow) | ✅ |
| Layout bar: layout pop-up, New Layout/Report, Layout Setup (name, table occurrence), Exit Layout | ✅ |
| Tools: selection, text (edited in place, double-click to edit again), field (Specify Field, optional label), rectangle, line, button (Go to Layout or Perform Script; double-click to edit) | ✅ |
| Inspector: Position (name, left/top/right/bottom/width/height, layout width), Appearance (named style, fill, border, text colour overrides), Data (field, control style, value list) | ✅ |
| Save to `.fmp12`: object list, per-object nodes (text, name, label, UUIDs), parts, layout name, table occurrence, width, last object id, spatial index and tab order | ✅ tested by re-reading; 🔍 to verify in FileMaker |
| Protobuf encoder (`pb::Msg::encode`, `pb::MsgBuf`): all 418 template layouts and 5,544 objects re-encode byte for byte | ✅ |
| New layouts written to the file (needs the layout catalog `[4].[1]` and its collation-keyed name index) | ⏳ session only |
| Name index `[4].[1].[1]` after a rename (collation keys) | ⏳ left as is |
| Other tools (oval, portal, tab control, web viewer…), alignment, grouping, arrange (z-order), fonts, part add/delete, tab order editing | ⏳ |

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
| Summary fields (Total, Average, Count, Min, Max, StDev, Fraction of Total, List) over a found set or break groups (`engine::summary`) | ✅ |
| Validation enforcement | ⏳ |
| Storage formats of dates/times/numbers in records: text in the file's locale (`typed_in`) | ✅ |
| Stored calculations re-evaluated for new and edited records on save | ✅ |
## M8 — Import/export, Preview and PDF

| Feature | State |
|---------|-------|
| Export Records (`engine::export`): found set in sort order, chosen fields, related fields, formatted or as stored | ✅ |
| Comma-separated text: every value quoted, CR LF between records, returns as VT (0x0B), repetitions and related records joined with GS (0x1D) | ✅ |
| Tab-separated text (tabs in values become spaces), Merge (`.mer`, field-name row) | ✅ |
| FMPXMLRESULT XML (ERRORCODE, PRODUCT, DATABASE, METADATA, RESULTSET; one DATA per related record) | ✅ |
| Excel `.xlsx` (hand-written OOXML, numbers and Excel dates typed, header row) and JSON | ✅ |
| Grouped export by sort fields (sub-summary rows), layout data formats, repeating fields, container data | ⏳ |
| Import Records (`engine::import`): CSV, tab, Merge, `.xlsx`; mapping by name or order; add, update matching (match fields, add remaining), replace found set | ✅ |
| Import: auto-enter on or off, validation reported, records failing always validation skipped | ✅ |
| Import from XML, other files, folders; character sets other than UTF-8 | ⏳ |
| Summary field definitions (field key 9) and sub-summary break fields (part field 6) decoded | ✅ (Total, Count seen in templates; other operation numbers inferred) |
| Preview pagination (`crates/print`): header/footer per page, title header/footer on page 1, body per record, leading/trailing sub-summaries by sorted break field, grand summaries, letter/A4, landscape, margins | ✅ |
| PDF output: hand-written PDF 1.4, Helvetica (WinAnsi), theme backgrounds, borders, text colours, sizes, bold, alignment, padding; fields, text with merge fields and `{{PageNumber}}`, button labels, portals | ✅ |
| Part options (page break before/after, allow break across pages), sliding, layout number/date formats, images, Unicode fonts (non-Latin scripts) | ⏳ |
| GUI Preview mode | ⏳ |
| CLI `export`, `import`, `print-pdf`; MCP `export_records`, `import_records`, `print_pdf` | ✅ |
| Builds for `wasm32-unknown-unknown` (`engine`, `print`) | ✅ |
| Charts, security (accounts, privileges), Data Viewer | ⏳ |

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
| Script debugger, Data Viewer | ⏳ |

Layout writing check (2026-11-27): a file whose "Contact List" layout BaseCraft
edited (object moved, text changed, object deleted, body taller, new text,
rectangle and button) opens in FileMaker Pro 26 without a warning. FileMaker's
Layout mode shows every edit.

## Distribution

| Feature | State |
|---------|-------|
| Release workflow (tag `vX.Y.Z`): Windows x64/ARM64 `.msi` + portable `.zip`, Linux x86_64/aarch64 `.AppImage` + `.deb` + `.tar.gz`, macOS universal `.dmg` (unsigned), `SHA256SUMS.txt` | ✅ dry run green on all 5 targets |
| Original app icon, window and web favicon | ✅ |
| Workspace lints (no `unsafe`, no `unwrap`/`expect`/`panic` outside tests), `cargo deny` in CI | ✅ |
| First public release | ⏳ waiting for the owner's go |
| ArtCraft launcher community listing | ⏳ draft request to be reviewed by the owner |
