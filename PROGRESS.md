# Progress

**Estimated progress to 1.0: ~92 %**, weighted by milestone (see the README table).

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

Coverage audit against FileMaker Pro 26's own inventories ([docs/COVERAGE.md](docs/COVERAGE.md),
counts asserted by tests), implemented / partial / missing:

| Functions | `Get()` flags | Script steps | Triggers | Layout objects | Control styles | Error codes raised |
|----------:|--------------:|-------------:|---------:|---------------:|---------------:|-------------------:|
| 197 / 37 / 0 of 234 | 53 / 8 / 78 of 139 | 128 / 23 / 0 (+71 desktop error) of 222 | 9 / 0 / 17 of 26 | 15 / 4 / 0 of 19 | 3 / 3 / 1 of 7 | 58 of 300 |

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
| Script step parameters: other steps (windows, portal rows, related records, editing, files, misc: FORMAT.md §8) | 🚧 most decoded from steps pasted into FileMaker 26; Find Matching Records' choice, data files, Go to List of Records ⏳ |
| Value lists (custom values, from field: second field, related only, sort by second field) | ✅ |
| Themes: compiled styles, named styles, local object styles, fonts (`theme.rs`) | ✅ |
| Accounts (name, active, privilege set, authentication type, must change password, password data), privilege sets (records per table with limiting calculations, fields, layouts, scripts, value lists, other privileges, menus, print, export), extended privileges, and their MD5 digests (FORMAT.md §15) | ✅ every digest of the 66 templates recomputed |
| Custom functions (`fmp12::custom_functions`, catalog `[16]`): name, parameters, calculation, availability, folders; calls in calculations (`1B`/`1C` tokens, FORMAT.md §16) | ✅ read and written (create, edit, delete through `schema_edit`); files BaseCraft wrote open in FileMaker Pro 26 and FileMaker Server's consistency check passes; Manage Custom Functions and Save as XML list them; calculation fields and Evaluate give the same results in FileMaker and BaseCraft |
| Container data: embedded files and pictures (store `[T].[31]`, streams FNAM, PNGf, JPEG, GIFf, FILE, SIZE, DPI_), references by path; GetContainerAttribute, GetAsText, Base64Encode | ✅ (from containers inserted in FileMaker; `all` matches) |
| Container data stored externally: field options, `REMO` reference and data stream, base directories `[38]`, open storage read from and written to `<base directory>/<Table>/<Field>/` (`name_1.ext` for a second file of the same name), as FileMaker Pro 26 does (FORMAT.md §13) | ✅ open storage: FileMaker reads the files BaseCraft wrote (MD5, size, picture size, path); BaseCraft reads FileMaker's |
| Secure external storage | ⏳ blocked: layout decoded, but FileMaker encrypts the files and their folder names (AES, per-file `ENCR` stream) with a key not in reach: not read; new files in a secure field are stored embedded (FileMaker reads them) |
| Thumbnails | ⏳ FileMaker 26 makes them only for hosted files; none are stored in local files (checked with permanent thumbnails on) |
| Custom open-storage folder calculations, base directories other than the first | ⏳ |

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
| `Get()` flags answered without a host | 11 / 139 |
| `Get()` flags answered by the data engine without a script or window | 15 more |

The 234 reference lines include `Status` twice. The 139 `Get()` flags exclude the
`Get ( flag )` header line. A host can answer any flag through `Context::get`. The 11
built-in flags are UserName, CurrentDate, CurrentTime, CurrentTimestamp, CurrentHostTimestamp
(UTC, because the engine has no time-zone data), CurrentTimeUTCMilliseconds,
CurrentTimeUTCMicroseconds, UUID, CalculationRepetitionNumber, SystemPlatform and
ApplicationArchitecture. The engine answers FoundCount, RecordNumber, TotalRecordCount,
RecordID, SortState and LayoutCount from the found set it is given (every record,
unsorted, without one), and outside a script LastError, ErrorCaptureState,
RecordOpenState, RecordOpenCount, TransactionOpenState and WindowMode (0),
AllowAbortState (1), ScriptName, ScriptParameter, ScriptResult, LastErrorDetail and
LastErrorLocation (empty); a script answers them from its own state first.

Oracle: **2,022 expressions** evaluated by FileMaker Pro 26.0.1 (Windows, English Tasks
template, en_US formats) in [crates/calc/tests/oracle.tsv](crates/calc/tests/oracle.tsv),
produced by `tools/oracle/calc_oracle.ps1`; `cargo test -p calc --test oracle` checks
them all (`-- --nocapture` lists the rows that still differ). 1,997 match; 25 are listed
as known differences (see below), 7 of them platform-dependent. Before this round:
1,934 of 1,987, 53 known.

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
| Errors FileMaker absorbs (`Evaluator::sticky`, `calc::evaluate_with_code`): a value comes back and EvaluationError still reports the error. Aggregates skip a failed argument (`Sum ( 1 ; 1/0 )` = 1, error 15), List shows it as `?`, IsEmpty of an error is 0, IsValid is 0, Evaluate of text that does not parse is `?`, an unknown TextEncode/TextDecode encoding gives `?` with error 4 | ✅ oracle |
| Dates and times shown as read (`Value::Shown`): GetAsDate/GetAsTimestamp keep a `.` or `-` separator, GetAsTime shows AM/PM text and timestamps in the 12-hour form (`1 PM` without minutes typed); arithmetic and comparisons see the plain value, fields store it | ✅ oracle |
| Parse errors of an empty last parameter (1202 past the maximum, accepted by Case and other functions without one), `x::` (106) | ✅ oracle |
| ExecuteSQL / ExecuteSQLe: single-table SELECT, WHERE, ORDER BY, DISTINCT, COUNT(*), OFFSET/FETCH, `?` arguments, over `Context::rows` | 🚧 no joins, GROUP BY or system tables |
| Japanese: kana width and script, KanjiNumeral, NumToJText, YearName, MonthNameJ, TrimAll types | ✅ oracle |
| CryptGenerateSignature / CryptVerifySignature (RSA PKCS #1 v1.5, plain and DES-EDE3/AES-encrypted PEM keys) | ✅ matches OpenSSL |
| CryptEncrypt / CryptDecrypt (+Base64): PBKDF2 + AES-128-GCM with SHA-256 digest, own byte layout | 🚧 not FileMaker-compatible |
| Embeddings (CosineSimilarity, Add/Subtract/NormalizeEmbedding, GetEmbeddingAsText/File), container size/attributes, path conversion | ✅ |
| Location, sensors, Live Text, AI models, persistent data, layout-object attributes: `Context::host_function`, "not available" (error 3) by default | ✅ |
| Custom functions: parameters, recursion, `Get` inside, wrong parameter count (1201, 1202), the 50,000-call limit of one calculation ("?", not caught by EvaluationError; an Evaluate counts its own) | ✅ matches FileMaker Pro 26 (`custom_functions_match_filemaker`, `custom_function_call_limit`) |
| File-locale date/number formats | ⏳ |

Known differences (rows after `# known differences` in oracle.tsv):

* Platform-dependent (7): trigonometry runs on f64 and differs in the last shown digit for about 1 argument in 5 (`Sin ( Pi / 6 )`; no conversion of the decimal argument to a double, nearest, truncated or cut to 16–18 digits, matches every Sin, Cos and Tan row, so FileMaker's libm differs from Rust's); ConvertToFileMakerPath / ConvertFromFileMakerPath as FileMaker on Windows does them (`file:/C:/…`, error 5).
* Guard digits beyond the 16 shown, visible only through `* 10^20` probes, differ for transcendental functions (FileMaker keeps about 20 significant digits with its own rounding: `Ln ( 2 )` ends …0940, `Degrees` divides by a 19-digit Pi); `Exp` of large arguments (FileMaker's `Exp ( 100 )` is off from the exact value; `Exp ( 1000 )` exceeds our 400 digits).
* Styled text (no style runs: GetAsCSS/GetAsSVG of TextColor, TextStyleAdd), lone UTF-16 surrogates from `Char` (Rust strings cannot hold them), Shift_JIS TextEncode (no encoding tables without a new dependency), `JSONMakeArray ( "" ; ¶ ; JSONString )` (FileMaker reports error -1).

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
| Windows: New / Select / Close / Move-Resize / Adjust / Arrange Window, Set Window Title, Freeze / Refresh, Scroll, Show/Hide Toolbars, Menubar, Text Ruler, View As, Set Zoom Level; `Get ( WindowName / WindowWidth / WindowZoomLevel / LayoutViewState / StatusAreaState )` | ✅ checked against FileMaker 26 (FORMAT.md §8) |
| Navigation: Go to Object (field objects become active, 116 when missing), Go to Portal Row, Delete Portal Row, Go to Field, Go to Next / Previous Field, Go to Related Record (layout, only related, match all, new window) | ✅ |
| Editing: Cut, Copy, Paste, Clear, Select All, Set Selection, Undo/Redo; `Get ( ActiveFieldName / ActiveSelectionStart / ActiveSelectionSize / ActivePortalRowNumber )` | ✅ |
| Fields: Set Next Serial Value, Insert from Index (the app's picker), Insert from Last Visited, Export Field Contents; Insert from URL disabled (no HTTP client) | ✅ |
| Records: Open Record, Revert Record, Copy Record, Copy All Records, Truncate Table, Import / Export Records (the app's dialogs; stored import/export options ⏳), Open / Commit / Revert Transaction | ✅ |
| Found sets: Sort with stored order and with dialog, Sort Records by Field, Unsort, Find Matching Records, Constrain / Extend Found Set, Modify Last Find, Omit Multiple, Show Omitted Only, Perform Quick Find, Go to Record | ✅ |
| Control: Perform Script by name, Install OnTimer Script (the app runs it every interval while no script runs), Set Error Logging, Set Layout Object Animation, Halt Script, Allow User Abort | ✅ |
| Files: New / Open / Close File and Exit Application (after the script), Save a Copy as, Print (to PDF), Save Records as PDF / Excel (next to the file for a relative `file:` path, otherwise a save dialog); Set Multi-User does nothing; Recover File refused (error 3) | ✅ |
| Misc: Beep, Open URL and Send Mail (`mailto:`) behind a confirmation, handed to the OS opener (http, https, mailto, ftp only); Flush Cache, Get Folder Path; Speak, Dial Phone, Execute SQL, Set Web Viewer return FileMaker's error | ✅ |
| Data files (`script::datafiles`): Get File Exists / Size, Create / Open / Close Data File, Write to Data File (variable or field; UTF-16, UTF-8, bytes; append line feed), Read from Data File (count, or the whole file; UTF-16 counts characters; bytes give a container named after the file), Get / Set Data File Position, Delete File, Rename File; file ids from 1; FileMaker's errors (100 missing file or id, 300 already open, 10 read past the end, 14 negative position). Paths: FileMaker's (`file:` relative to the file's folder, `filewin:`, `filemac:`, `/C:/…`, path lists in variables, first line that exists), through the host (`script::files::Disk`); a write outside the file's folder asks first (app: "Write a file" confirmation) | ✅ checked against FileMaker 26 (`tools/oracle/datafiles`) |
| PDF documents: Create / Open / Append / Close / Cancel PDF (pages of PDF files or containers joined by `print::merge`), Print PDF (the app saves it); FileMaker's errors (5 nothing to close, 829 nothing open, 833 already open, 100 missing file) | ✅ checked against FileMaker 26; PDFs with compressed object streams cannot be joined ⏳ |
| Go to List of Records (record ids in their order, layout; 101 some ids missing, 401 none), Configure Persistent Data (name required, 5), Insert PDF / Insert Audio/Video (the host's file picker; 3 with no field), Insert Object (obsolete in FileMaker 26: 4), Configure Local Notification / AI Account with no parameters (5) | ✅ checked against FileMaker 26 |
| Steps that cannot apply (AI, server, plug-ins, iOS, design dialogs) return FileMaker's desktop error (`script::UNSUPPORTED`, table below) | ✅ |
| Finds on related fields | ⏳ |
| Resumable runs (`Engine::start`, `Run::resume` → `Suspended(Dialog \| Pause)` / `Finished`): Show Custom Dialog, delete confirmations, Pause/Resume Script, modes entered with Pause wait for the user; no threads (wasm32) | ✅ |
| `perform` / `run_step` run to the end (dialogs through `Host::dialog`, button 1 by default): CLI and MCP unchanged | ✅ |
| `Get ( LastErrorLocation / LastErrorDetail / ScriptAnimationState / Trigger… )` (trigger values set by the host) | ✅ |
| Step text as in the Script Workspace (`script::text`), block indentation | ✅ |
| French step names and step text (`script::text::step_name_fr`, `step_text_in`): the 222 step names, palette categories and step labels as FileMaker 26's French UI shows them (`Définir rubrique [Tasks::Description; "abc"]`), read from its Script Workspace through UI Automation; calculations keep English function names | ✅ |
| Script triggers (`fmp12::triggers`): layout, object and file triggers, 26 event ids, modes (Browse 1, Find 2, Preview 4) and parameters decoded from triggers set in FileMaker 26; 25 triggers of the oracle files match the XML (event, script, modes, parameter) | ✅ read; writing ⏳ |
| Calculation compiler (`fmp12::calc_compile`, one compiler for fields and scripts): source tokens (key 5) and prefix form (key 6); all 5,214 template calculations, comments included: key 6 derived from key 5 identical, decompiled text compiles back to both keys | ✅ |
| Writing scripts (`fmp12::scripts_save::save_scripts`): step lists and parameter trees of existing scripts; 224 template and oracle scripts re-encode byte for byte, 886 typed steps encode back from their typed parameters | ✅ verified in FileMaker (`tools/oracle/script_check.ps1`): edited, added and disabled steps, If/Else, Set Variable, Go to Layout, Perform Script, Show Custom Dialog, Let/Case/Choose/List/`$v[2]` show as written and evaluate |
| New, deleted and renamed scripts | ✅ verified in FileMaker: `[17].[1]` key 8 is a plain MD5 integrity digest of the script catalog (FORMAT.md §8), recomputed on save; it reproduces all 66 templates and 395 sample files. A Tasks copy with scripts added, renamed and deleted, and a `basecraft-cli new-file` file with the same, open without warning, list the new names and run the new scripts |
| Script Debugger (Tools menu, `app/src/debugger.rs`): stops at the first step while open and at breakpoints set in the Script Workspace's gutter (kept for the session); Step, Step Into, Step Out, Run, Halt Script; current step highlighted, call stack, last error; local variables in the Data Viewer's Current tab. Built on `Run`: `Suspend::Breakpoint`, `Run::set_stepping` | ✅ |
| Pause/Resume duration storage, script folders created in the workspace, script order changes by dragging | ⏳ |
| Builds for `wasm32-unknown-unknown` | ✅ |

### Step coverage

`cargo test -p script step_coverage -- --nocapture` runs a new step of
every id (FileMaker's default parameters) and prints these lists. Before
M5 the engine ran 50 of the 222 steps; the other 172 returned error 3.
Every step now runs; none is left to `Host::other`.

| | Steps |
|---|---|
| **Run (151)** | 1 Perform Script, 4 Go to Next Field, 5 Go to Previous Field, 6 Go to Layout, 7 New Record/Request, 8 Duplicate Record/Request, 9 Delete Record/Request, 10 Delete All Records, 11 Insert from Index, 12 Insert from Last Visited, 13 Insert Current Date, 14 Insert Current Time, 16 Go to Record/Request/Page, 17 Go to Field, 18 Check Selection, 19 Check Record, 20 Check Found Set, 21 Unsort Records, 22 Enter Find Mode, 23 Show All Records, 24 Modify Last Find, 25 Omit Record, 26 Omit Multiple Records, 27 Show Omitted Only, 28 Perform Find, 29 Show/Hide Toolbars, 30 View As, 31 Adjust Window, 33 Open File, 34 Close File, 35 Import Records, 36 Export Records, 37 Save a Copy as, 39 Sort Records, 40 Relookup Field Contents, 41 Enter Preview Mode, 42 Print Setup, 43 Print, 44 Exit Application, 45 Undo/Redo, 46 Cut, 47 Copy, 48 Paste, 49 Clear, 50 Select All, 51 Revert Record/Request, 55 Enter Browse Mode, 56 Insert Picture, 60 Insert Current User Name, 61 Insert Text, 62 Pause/Resume Script, 63 Send Mail, 68 If, 69 Else, 70 End If, 71 Loop, 72 Exit Loop If, 73 End Loop, 74 Go to Related Record, 75 Commit Records/Requests, 76 Set Field, 77 Insert Calculated Result, 79 Freeze Window, 80 Refresh Window, 81 Scroll Window, 82 New File, 83 Change Password, 84 Set Multi-User, 85 Allow User Abort, 86 Set Error Capture, 87 Show Custom Dialog, 89 # (comment), 90 Halt Script, 91 Replace Field Contents, 92 Show/Hide Text Ruler, 93 Beep, 94 Set Use System Formats, 97 Set Zoom Level, 98 Copy All Records/Requests, 99 Go to Portal Row, 101 Copy Record/Request, 102 Flush Cache to Disk, 103 Exit Script, 104 Delete Portal Row, 111 Open URL, 116 Set Next Serial Value, 119 Move/Resize Window, 120 Arrange All Windows, 121 Close Window, 122 New Window, 123 Select Window, 124 Set Window Title, 125 Else If, 126 Constrain Found Set, 127 Extend Found Set, 128 Perform Find/Replace, 130 Set Selection, 131 Insert File, 132 Export Field Contents, 133 Open Record/Request, 134 Add Account, 135 Delete Account, 136 Reset Account Password, 137 Enable Account, 138 Re-Login, 141 Set Variable, 143 Save Records as Excel, 144 Save Records as PDF, 145 Go to Object, 146 Set Web Viewer, 147 Set Field By Name, 148 Install OnTimer Script, 150 Perform Quick Find, 154 Sort Records by Field, 155 Find Matching Records, 158 Insert PDF, 159 Insert Audio/Video, 166 Show/Hide Menubar, 167 Refresh Object, 168 Set Layout Object Animation, 169 Close Popover, 174 Enable Touch Keyboard, 180 Refresh Portal, 181 Get Folder Path, 182 Truncate Table, 188 Get File Exists, 189 Get File Size, 190 Create Data File, 191 Open Data File, 192 Write to Data File, 193 Read from Data File, 194 Get Data File Position, 195 Set Data File Position, 196 Close Data File, 197 Delete File, 199 Rename File, 200 Set Error Logging, 205 Open Transaction, 206 Commit Transaction, 207 Revert Transaction, 209 Set Dictionary, 223 Set Revert Transaction on Error, 228 Go to List of Records, 237 Flush Web Viewer Cookies, 238 Configure Persistent Data, 242 Print PDF, 243 Create PDF, 244 Append PDF, 245 Close PDF, 246 Open PDF, 247 Cancel PDF |
| **FileMaker's desktop error by design (71)**: AI, server, plug-ins, iOS / macOS only, FileMaker's own design dialogs, obsolete steps; error 3 unless noted | 3 Save a Copy as XML, 32 Open Help, 38 Open Manage Database, 57 Send Event, 59 Insert QuickTime, 64 Send DDE Execute, 65 Dial Phone, 66 Speak, 67 Perform AppleScript, 78 Insert Object (4), 88 Open Script Workspace, 95 Recover File (refused), 96 Save a Copy as Add-on Package, 105 Open Settings, 106 Correct Word, 107 Spelling Options, 108 Select Dictionaries, 109 Edit User Dictionary, 112 Open Manage Value Lists, 113 Open Sharing, 114 Open File Options, 115 Allow Formatting Bar, 117 Execute SQL (5), 118 Open Hosts, 129 Open Find/Replace, 139 Convert File, 140 Open Manage Data Sources, 142 Install Menu Set, 149 Open Edit Saved Finds, 151 Open Manage Layouts, 152 Save Records as Snapshot Link, 156 Open Manage Containers, 157 Install Plug-In File, 160 Insert from URL, 161 Insert from Device, 164 Perform Script on Server, 165 Open Manage Themes, 172 Open Upload to Host, 175 Perform JavaScript in Web Viewer, 176 Set Allowed Orientations (4), 177 AVPlayer Play, 178 AVPlayer Set Playback State, 179 AVPlayer Set Options, 183 Open Favorites, 184 Open Starter Solution, 185 Configure Region Monitor Script, 187 Configure Local Notification (5 with no parameters), 201 Configure NFC Reading, 202 Configure Machine Learning Model, 203 Execute FileMaker Data API, 208 Set Session Identifier, 210 Perform Script on Server with Callback, 211 Trigger Claris Connect Flow, 212 Configure AI Account (5 with no parameters), 213 Fine-Tune Model, 214 Perform SQL Query by Natural Language, 215 Insert Embedding, 216 Insert Embedding in Found Set, 217 Set AI Call Logging, 218 Perform Semantic Find, 219 Perform RAG Action, 220 Generate Response from Model, 221 Perform Find by Natural Language, 222 Configure Regression Model, 224 Execute URL, 225 Save Records as JSONL, 226 Configure Prompt Template, 227 Configure RAG Account, 239 Validate Scripts, 240 Insert Image Captions in Found Set, 241 Insert Image Caption |
| **Not run yet (0)** | — |

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
| Inspector › Data: field, control style (edit box, drop-down list, pop-up menu, checkbox set, radio button set, drop-down calendar, concealed edit box), value list; portal occurrence, first row, rows | ✅ |
| Inspector › Data: data formatting (object node key 13, `fmp12::formats`): number (as entered, general, decimal, currency, percent, Boolean; fixed decimals, thousands, system separators, currency symbol leading or trailing, negative style and colour, hide zero), date (predefined formats, custom elements and separators, day and month styles), time (custom hours/minutes/seconds, separator, styles, AM/PM), container format number | ✅ offsets and meanings checked against FileMaker's rendering |
| Inspector › Data: placeholder text, "Hide object when" (and in Find mode); Position: tooltip; sliding "only objects directly above" and "also resize enclosing part" | ✅ option bits decoded in FileMaker's Inspector |
| Format › Conditional Formatting…: formula or "Value is" tests (FileMaker's 20 operators, written with the `Self` formula FileMaker stores), text colour, fill, bold, italic, underline | ✅ |
| Insert › Popover Button, Slide Control, Button Bar, Web Viewer… (tools too); Web Viewer Setup; Arrange › Set Tab Order… | ✅ round trip; FileMaker check in `layout_check.ps1` |
| Rulers, grid (show, snap to grid), guides dragged from the rulers (snap within 5 pt; dragged back onto the ruler to remove) | ✅ (guides are kept for the session; FileMaker stores its own guides in a place not decoded) |
| Save to `.fmp12`: object list, container child lists (field 6), per-object nodes (text, name, button/tab labels, UUIDs), parts (added, removed, reordered) and their object lists, layout name, table occurrence, width, last object id, spatial index and tab order | ✅ verified in FileMaker |
| Protobuf encoder (`pb::Msg::encode`, `pb::MsgBuf`): all 418 template layouts and 5,544 objects re-encode byte for byte | ✅ |
| Layout object nesting read from the containers' child lists: 633/633 objects match the XML | ✅ |
| Chart tool; script triggers on objects; icons of buttons; popover and slide control options (position, title bar, dots, swipe bits 🔸) | ⏳ |

## M7 — Writing `.fmp12`

| Feature | State |
|---------|-------|
| Chunk encoder (all opcodes) and leaf packing; round-trips through our reader | ✅ |
| Same-size in-place edits accepted by FileMaker | ✅ (verified) |
| Leaves keep FileMaker's exact chunk bytes (key spelling, `80` node markers, empty nodes, trailing pops) | ✅ all 6,824 template leaves re-pack byte for byte |
| Index (separator) sectors: encoding with pop flags, exact first-key separators | ✅ all 66 template roots re-encode byte for byte |
| Full rewrite (`fmdump <file> rewrite <out>`) opens in FileMaker Pro 26 | ✅ 10 templates verified; XML export identical for Contacts, Tasks, Assets (DDR_INFO order and ids aside) |
| Sectors keep room for their closing pops (FileMaker Server's consistency check, error 805, flagged full sectors without them: 14 of 66 template rewrites before the fix) | ✅ every leaf of 686 corpus rewrites ends as FileMaker's do; FMDeveloperTool re-check pending (test VM down) |
| Scripts of 128 steps or more (step records `03 02 80 xx`, step count `02 80 xx`, as FileMaker 26 writes them) | ✅ byte for byte with a 140-step script FileMaker built; FileMaker Pro 26's XML export of one BaseCraft wrote shows all 140 steps' parameters |
| Multi-level index trees (level ≥ 2) | 🚧 implemented, no template is deep enough to verify |
| Writing records: add, edit, delete (`fmp12::save::save_records`, `engine::Database::save`) | ✅ verified: FileMaker opens the file and shows the right count, values and related records |
| Bookkeeping: record-id set, last record id, change counter, modification count kept | ✅ |
| Field indexes after edits: dropped and marked for rebuild (FileMaker rebuilds them on demand) | ✅ verified (portal, find, unique key) |
| Writing exact index keys (FileMaker collation tables) | ⏳ |
| Files saved by BaseCraft, edited and saved again by FileMaker, read back | ✅ verified |
| Auto-enter values (serials, UUIDs, stamps) on records created by BaseCraft | ⏳ |
| Repeating fields and container data (`Database::save`, `containers::save_containers`: new files, shared copies, use counts) | ✅ verified in FileMaker: it reads the repetitions and pictures BaseCraft wrote (GetContainerAttribute MD5), adds its own records, repetitions and files after them, and BaseCraft reads those back |
| Files marked cleanly closed (page 0 flag 0x800 = `AC`, file sequence number) | ✅ verified: FileMaker opens and closes them unchanged (an unclean file gets checked and rewritten) |
| Calculation compiler: source text → token stream (key 5) and compiled form (key 6) FileMaker evaluates (`fmp12::calc::compile`) | ✅ 1,890 FileMaker-compiled calculations and all 5,214 template calculations (fields, scripts, layouts, comments included) byte for byte; one compiler for schema and scripts |
| Scripts (`fmp12::scripts_save`): edited step lists and parameters | ✅ verified in FileMaker: opens without warning, Save a Copy as XML shows the steps as written, the calculations evaluate |
| New, deleted, renamed scripts | ✅ verified in FileMaker: `[17].[1]` key 8 (MD5 over the catalog members' names in id order, FORMAT.md §8) written by `save_scripts`; non-Latin names in the long form |
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
| Found sets in calculations (`engine::FoundSet`, `Scope`): summary fields over the window's found set in its sort order (`Database::value_in`), GetSummary over it (by the break field only when sorted by it, empty otherwise), GetNthRecord, GetRecordIDsFromFoundSet, `Get ( FoundCount / RecordNumber / SortState )`; a related summary field summarizes the related records; a Total of no values is empty. Scripts pass their window's found set and sort (`script::Window::sort`), the CLI's `eval` and `records` the one of `--find`/`--sort` | ✅ GetSummary sorted/unsorted, empty groups, GetNthRecord and RecordNumber checked in FileMaker 26 (`get_summary_matches_filemaker`) |
| `Self` (`Scope::self_value`): validation calculations (the field's value), auto-enter calculations (the field's current value), layout objects through `Database::eval_with_self` | ✅ |
| Auto-enter calculations evaluate again when a field they use changes (`set_values`): over the value with "Do not replace existing value" unticked, else only into an empty field | ✅ (one pass; a field is "used" when its name appears in the formula) |
| Validation enforced on commit, in the app (dialog, "Accept anyway" when allowed) and in scripts (record stays open, `Get ( LastError )`; Commit Records' "skip data entry validation" and imports keep "always" fields, `Invalid::always`), first failing rule per field in the Field Options order: strict data type (502 numeric, 500 four-digit year date, 501 time of day), not empty (509), unique (504), existing (505), member of value list (506), in range (503, typed by the field: numbers, dates, times, text), validated by calculation (507, `Self` is the value, not run on an empty value), maximum characters (511); custom messages (a calculation) replace FileMaker's text | ✅ |
| A script's commit (`Database::validate_script`) checks only not empty (also a field the script never set), unique, existing and calculations, and reports the first failing field by id: value lists, ranges and maximum characters are data-entry checks, even "validate always" | ✅ checked in FileMaker 26 (`tools/oracle/validation_check.ps1`); 500/501/502 and the app's data-entry path from FileMaker's documentation |
| Storage formats of dates/times/numbers in records: text in the file's locale (`typed_in`) | ✅ |
| Stored calculations re-evaluated for new and edited records on save | ✅ |
| Schema changes in memory (`Database::add_table`, `add_field`, `set_field`, `add_occurrence`, `add_relationship`, `add_value_list`, `rename_*`, `set_*`, `delete_*`) with unsaved records kept; `save` writes records and moves the next serial value | ✅ |

Engine API for the app (not wired in `crates/app` yet):

* `engine::FoundSet { table, records, sort }`: the window's found set (record indexes in
  display order) and the fields it is sorted by (empty when unsorted). The app keeps the
  sort keys only as a bool per table today; it needs the field ids.
* `Database::value_in(table, record, field, rep, Some(found))`: what a field shows in
  that window (summary fields: running totals, subtotaled fractions; calculations with
  GetSummary, `Get ( RecordNumber )`…). `value`/`value_rep` stay "every record, unsorted".
* `Database::eval_with_self(tokens, table, to, record, &value, Some(found))`: a layout
  object's calculation with `Self` = the object's value: the stored formula of a
  conditional format (instead of evaluating "Value is…" from its operator), "hide object
  when", tooltips.
* `Database::eval_tokens_scoped(tokens, table, to, record, &Scope { found, self_value },
  env)` and `eval_source_in(src, …, &scope)`: the general forms (Data Viewer: pass the
  window's found set so `Get ( RecordNumber )` and GetSummary answer).
* `script::Window::sort`: fill it with the sort field ids when building the script
  window, as `found` is filled; Sort steps set it.
* `engine::Rule` has four new variants (Date, Time, Range, Calculation; the app's
  validation dialog got minimal texts) and `Invalid::always`. `Database::validate` is
  the data-entry check (the app's commit); scripts use `validate_script`.
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
| Security (`fmp12::security`, `engine::access`): login with FileMaker's salted password hash (`--account`/`--password` in the CLI and MCP, a login dialog in the app, "Log in using" honoured, "must change password" asks for a new one); privileges enforced (records with limiting calculations, read-only and hidden fields, hidden layouts, scripts, print, export; errors 9, 200, 201); `Get ( AccountName )`, `Get ( AccountPrivilegeSetName )`, `Get ( AccountExtendedPrivileges )`; script steps Add Account, Delete Account, Reset Account Password, Enable Account, Change Password, Re-Login (`script::accounts`); writing accounts and privilege sets with full access (File › Manage › Security, `accounts`/`privileges` commands, MCP tools) | ✅ verified in FileMaker 26: files BaseCraft wrote open without warning and take its passwords (wrong password and inactive account refused with FileMaker's alert, a no-access layout shows « Accès refusé »); hashes FileMaker wrote for test passwords verify; the account steps store and fail as in FileMaker (`tools/oracle/security_check.ps1`) |
| Extended privileges written as FileMaker reads them (key 02 `01 01` turns a privilege's set list on, FORMAT.md §15) | ✅ FMDeveloperTool's Save as XML lists the sets BaseCraft grants `fmrest`/`fmodata`; FileMaker Server writes the same bytes when it adds `fmapp`; the Data API logs in the account of such a set |
| External-server and OAuth logins, "Run script with full access privileges", Re-Login/Change Password dialogs from scripts, encryption at rest | ⏳ |

## M4 — GUI (`crates/app`)
| Menu bar, status toolbar, layout bar (English/French) | ✅ |
| Windows (`windows.rs`): Window › New Window (named "File - 2" as FileMaker), each window its own layout, mode, found set, sort and requests; an OS window each (egui viewports; `egui::Window` in the browser and tests); Window menu with the open windows (front checked), Minimize, Tile Horizontally/Vertically, Cascade; File › Close (Ctrl+W) closes the front window, the last one closes the file; New/Select/Close Window script steps act on the same windows (the engine's background windows); clicking a window brings it to the front | ✅ compared with FileMaker 26 (Window menu, "Contacts - 2") |
| Several files open at once (File › Open, Open Recent, the command line, drag and drop onto a window), each in its own OS window; the Window menu lists them all | ✅ |
| Status toolbar as FileMaker 26's (`toolbar.rs`): book (◀ ▶), record number field (Enter), slider, found / total (Sorted/Unsorted), Show All, New Record, Delete Record, Find with saved finds ▾, Sort (opens Sort Records), Share, Quick Find box; Find mode: requests, New/Delete Request, Perform Find ▾ (Constrain / Extend Found Set), Cancel Find, Saved Finds; layout bar: layout pop-up, View As, Preview, Edit Layout; in Find mode Matching records Include/Omit and Insert › Operators (FileMaker's 14 operators, at the cursor); View › Customize Status Toolbar (buttons, labels); the Records menu is Requests in Find mode | ✅ compared with FileMaker 26 screenshots (French) |
| Quick Find over the layout's fields (every word starts a word of a field); "no records match" keeps the found set as FileMaker | ✅ fields whose "Include field for Quick Find" is off (field object option `0x8000`) are skipped, also by Perform Quick Find |
| Table view (`views.rs`): click a header to sort (again: descending, ▲/▼), drag a column edge to resize, header menu (sort, unsort, hide field, add field, total/count row), current record highlighted, values as the window sees them (`value_in`, summaries over the found set); List view: current record band and bar, click selects; arrows and Ctrl+Up/Down move between records | ✅ |
| Undo/Redo of field edits (Ctrl+Z/Ctrl+Y outside a field; egui's own undo inside one); Insert › Current Date (Ctrl+-), Current Time (Ctrl+;), Current User Name (Ctrl+Shift+N), From Index… (Ctrl+I), From Last Visited Record (Ctrl+'), at the cursor of the field last edited | ✅ menus checked against FileMaker 26's |
| Edit › Preferences… (`prefs.rs`): user name (`Get ( UserName )`), interface language (system / English / French, live), recent files count, light/dark/system theme, drag and drop, layout units (points, inches, centimetres, used by the Inspector); kept in `%APPDATA%\BaseCraft\preferences.txt` (or `BASECRAFT_CONFIG`); File › Open Recent | ✅ |
| Shortcuts as FileMaker 26 on Windows: Ctrl+B/F/L/U modes, Ctrl+S Sort (BaseCraft's File › Save has no shortcut), Ctrl+Alt+S toolbar, F3/Shift+F3 zoom, Ctrl+N/D/E/J/T/R…, Ctrl+O, Ctrl+W, Ctrl+Q, Enter performs a find | ✅ |
| Edit boxes with "Auto-complete using previously entered values" (option `0x800`) list matching values of other records while typing | ✅ (mouse only; drop-down lists do not auto-complete from their value list yet) |
| Spelling, Show/Hide Window, styled toolbar icons as FileMaker's | ⏳ |
| Layout drawn at stored coordinates: parts, fields, text, buttons | ✅ |
| Browse mode: navigate, edit fields, new/delete record, show all | ✅ |
| Records menu as in FileMaker 26: New, Duplicate (Ctrl+D), Delete Record… (Ctrl+E) and Delete All Records… with FileMaker's confirmations (related records deleted by cascade), Show All (Ctrl+J), Show Omitted Only, Omit Record (Ctrl+T), Omit Multiple… (Ctrl+Shift+T), Modify Last Find (Ctrl+R), Replace Field Contents… (Ctrl+=) | ✅ |
| Replace Field Contents (`engine::replace`): current contents, serial numbers (prefix and leading zeros kept, "Update serial number in Entry Options"), calculated result; auto-enter options on or off | ✅ |
| Edit › Find/Replace (Ctrl+Shift+F, modeless): Find Next, Replace & Find, Replace, Replace All; direction forward / backward / all; match case, whole word; all records or current record; all fields or current field | ✅ compared with FileMaker 26's dialog |
| Records menu (`app/src/browse.rs`): Sort Records… (field list; ascending, descending or custom order by value list; keep records in sorted order), Unsort Records, Go to Record…, Saved Finds (Save Current Find, kept for the session), Revert Record… | ✅ |
| View menu: View as Form / List / Table, Zoom In / Out (ponytail: the whole window zooms), Status Toolbar | ✅ |
| Relookup Field Contents in the Records menu | ⏳ |
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
| Checkbox sets and radio button sets (one per value-list value, in columns; Browse and Find modes; undo, OnObjectModify), drop-down calendar (month popup, icon bit 0x400, the day written in the file's date format), concealed edit box (bullets) | ✅ `ui_tests::control_styles_in_browse_and_find`; no "Other…" item, "Allow editing" ignored |
| Data formats (numbers, dates, times, timestamps) shown as FileMaker shows them, in Browse, Preview and PDF (`print::format`) | ✅ |
| Conditional formatting, "Hide object when", placeholder text, tooltips | ✅ |
| Popovers (open and close), slide controls (dots, swipe), button bars (active segment, segment actions), web viewers (address in a box, "Open in Browser…" after a confirmation; no browser engine) | ✅ |
| Tab and Shift+Tab follow the layout's tab order | ✅ |
| Layout mode (see M6) | 🚧 |
| Preview mode | ⏳ |
| Web version (WebAssembly, GitHub Pages): open or drop a file, everything runs locally | ✅ |
| Save from the app: Ctrl+S / Save a Copy As (desktop), download (web) | ✅ |
| File › Open Remote… (`remote_ui.rs`, `crates/remote`): a file hosted by FileMaker Server, through the Data API; its layouts' fields and records in Browse, Find, sort, List and Table views; committed edits, new and deleted records sent with their modId (conflicts reported); Perform Script on Server…, Refresh from Server; "Hosted on <host>"; self-signed certificates only by explicit opt-in. Desktop only | ✅ against FileMaker Server 26 (VM) |
| Hosted files: login and loading on a thread (spinner, records loaded, Cancel); related fields (read-only) and portals from `portalData` (a related table each); value lists from the layout's metadata as pop-up menus / drop-down lists; containers downloaded when a record is shown, uploaded (`…/containers/{field}/{rep}`) when a file is put in one | ✅ against a mock Data API (`crates/remote/tests/mock.rs`, `remote_ui` tests); the FileMaker Server VM did not answer during M4 |
| Hosted files: portal rows written back, server-side find for files over 50,000 records, live refresh, Layout mode and schema (refused: not possible through the Data API) | ⏳ |

## Automation (`crates/cli`)

| Feature | State |
|---------|-------|
| `basecraft-cli`: info, records (find/omit/sort/limit), eval, new, set, delete, JSON output | ✅ |
| Session mode (JSON-lines on stdin/stdout) with save | ✅ |
| Run scripts from the CLI and MCP (`run-script`, `run_script`, `list_scripts`), dialogs answered automatically | ✅ |
| MCP server for AI agents (`basecraft-cli mcp`): open, describe, find, evaluate, create/update/delete, save | ✅ |
| Hosted files through FileMaker Server's Data API: `--remote https://host/Database --account A --password-env VAR` for `info`, `records`, `find`, `new`, `set` (`--mod-id`), `delete`, `run-script`; MCP `remote_command` | ✅ against FileMaker Server 26 (VM) |
| Manage Database from the CLI: `schema add-table`, `add-field`, `set-options`, `add-occurrence`, `add-relationship`, `set-relationship`, `add-valuelist`, `set-valuelist`, `rename`, `delete`, `new-file`; MCP `new_file`, `create_table`, `create_field` (lookups, validation, summaries), `create_occurrence`, `create_relationship`, `create_valuelist` | ✅ |
| Custom functions from the CLI (`functions [add|set|delete]`) and MCP (`list_custom_functions`, `set_custom_function`, `delete_custom_function`); File › Manage › Custom Functions… in the app (`custom_functions.rs`: list by folder, new, duplicate, delete, name, parameters, calculation, availability) | ✅ |
| Container files from the CLI: `new`/`set` with `Field=@path` (open external storage written next to the saved file) | ✅ |
| Stored calculation fields that use no field of their table (constants, custom functions, `Get`): dependency keys FF00 = FF01 = 00 | ✅ found while checking custom functions: without them FileMaker never computed such a field |

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
| Script Debugger window and the Data Viewer's Current tab (UI tests drive Step, Run, a breakpoint and Halt) | ✅ |

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
