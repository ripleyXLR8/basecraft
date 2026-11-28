# Coverage of FileMaker Pro 26

This audit compares the inventories of FileMaker Pro 26 under [docs/reference/](reference/)
(names and ids only) with what BaseCraft does today. It is an audit only: no gap below has
been filled. Every count is asserted by a test, so a change to the code or to a list fails a
test until this page and the test are updated together.

* **Implemented**: BaseCraft does what FileMaker does (checked against FileMaker where the
  test or the oracle says so).
* **Partial**: present, but stubbed, approximate, or ignoring some options.
* **Missing**: not done. Script steps that return FileMaker's own desktop error by design
  (AI, server, plug-ins, iOS) have their own column.

| Category | Reference | Total | Implemented | Partial | Missing | Test |
|----------|-----------|------:|------------:|--------:|--------:|------|
| Calculation functions | [functions.txt](reference/functions.txt) | 234 | 200 | 34 | 0 | `cargo test -p calc registry_coverage -- --nocapture` |
| `Get()` flags | [get_functions.txt](reference/get_functions.txt) | 139 | 97 | 8 | 34 | `cargo test -p script get_coverage -- --nocapture` |
| Script steps | [script_steps.txt](reference/script_steps.txt) | 222 | 130 | 28 | 0 (+64 desktop error) | `cargo test -p script step_coverage -- --nocapture` |
| Script triggers | [script_triggers.txt](reference/script_triggers.txt) | 26 | 22 | 0 | 4 | `cargo test -p basecraft trigger_coverage -- --nocapture` |
| Layout object kinds | [layout_objects.txt](reference/layout_objects.txt) | 19 | 15 | 4 | 0 | `cargo test -p basecraft layout_object_coverage -- --nocapture` |
| Field control styles | [control_styles.txt](reference/control_styles.txt) | 7 | 5 | 2 | 0 | `cargo test -p basecraft control_style_coverage -- --nocapture` |
| Error codes | [error_codes.txt](reference/error_codes.txt) | 300 | 65 raised | — | 235 never raised | `cargo test -p basecraft error_code_coverage -- --nocapture` |

Not inventoried: script step options (each step's parameters are covered step by step in
FORMAT.md §8 and PROGRESS.md M5), and File Options / Preferences (FileMaker 26 draws those
dialogs from compiled WPF resources, which have no plain list of settings).
Container storage (Field Options › Storage) is not a counted list either: embedded, open
external and secure external storage (with or without fewer folders, with or without
encryption at rest) are read and written as FileMaker writes them (FORMAT.md §13); only
open-storage folder calculations and base directories other than the first are not.

## How each list was made

* **Functions, `Get()` flags**: extracted from DBEngine.dll's strings in an earlier round
  (234 lines, `Status` twice, so 233 functions; 139 flags after the `Get ( flag )` header).
* **Script steps**: the reverse oracle (`tools/oracle/step_ids.py`): every step id of a
  template copy overwritten with 0…365 and the file exported as XML by FileMaker 26.
* **Script triggers**: the 26 trigger names in DBEngine.dll's string table (it also lists
  OnTimer and OnCallbackScript, which are not trigger events); ids from `fmp12::triggers`,
  decoded by setting each trigger in FileMaker 26. The test checks that both agree.
* **Layout object kinds, control styles**: the object and control style names in
  DBEngine.dll's string table; kinds and ids from FORMAT.md §10 (Concealed Edit Box: edit
  box flag 0x200000, found by setting bits in a copy and opening it in FileMaker 26).
* **Error codes**: the code column of Claris's public error code page, cross-checked
  against the English error messages DBEngine.dll carries (numbers only; the reserved
  range 5000–5499 left out).

The resource tables were read with REA's PE resource inspector (FMRSRC.dll) and a plain
string scan of DBEngine.dll, FMEngine.dll, ClientUI.dll and Support.dll; no code was
decompiled.

## Top gaps by likely user impact

1. **ExecuteSQL / ExecuteSQLe** have joins, GROUP BY, HAVING, the aggregates, arithmetic,
   CASE, CAST, the Claris SQL Reference's scalar functions, uncorrelated subqueries and the
   FileMaker_Tables / FileMaker_Fields / FileMaker_BaseTables / FileMaker_BaseTableFields
   system tables; still no correlated subqueries or EXISTS, ModCount is NULL, no
   FileMaker_ValueLists, and nothing was checked against FileMaker's own output.
2. **Open Transaction**'s Skip auto-enter options (`0x1000`), Skip data entry
   validation (`0x100`) and Override ESS locking conflicts (`0x200`) are decoded
   (FileMaker 26, FORMAT.md §8) but not applied.
3. **Checkbox sets and radio button sets** have no "Other…" item and ignore "Allow
   editing"; their columns are not checked against FileMaker's (PROGRESS.md M4). The
   flags are known (FileMaker 26, field options `0x1` "Other…", `0x8` "Allow editing",
   the latter offered for pop-up menus and drop-down lists only) but not drawn.
4. **`Get()` flags a local solution still misses**: SystemLocaleElements / FileLocaleElements, CustomMenuSetName
   (custom menu sets are not decoded), WindowStyle, WindowUUID, PersistentID (error 1225
   where FileMaker answers). The active-field flags see only fields a script entered, and
   RecordModificationCount is written back on save (a record changed without a commit counts one). The
   other missing flags belong to servers, FileMaker Go or plug-ins.
5. **Styled-text functions** (TextColor, TextStyleAdd, …) keep the plain text only, and
   **CryptEncrypt / CryptDecrypt** use their own byte layout (not readable by FileMaker).
6. **Web viewers** show their page on Windows, macOS and Linux under X11 or XWayland
   (native webview: WebView2, WKWebView, WebKitGTK 4.1) and in the web
   version (a sandboxed `<iframe>` over the canvas: scripts and forms only, no same origin,
   no popups or top navigation), in the main window only (a placeholder box elsewhere:
   Linux when BaseCraft's window is a Wayland window — wry's child webviews need an X11
   parent, `env -u WAYLAND_DISPLAY basecraft` runs it on XWayland —, the file's other
   windows and other files' windows). On Linux the webviews are untested on a real
   desktop 🔸. A partly scrolled-away viewer
   is shrunk, not clipped, and viewers are hidden while a menu or modal dialog is open. In
   the web version the browser's cross-origin rules apply: the `FileMaker.PerformScript`
   bridge and Perform JavaScript in Web Viewer work only for `data:text/html` pages (the
   bridge is written into the page); other sites get no bridge, Perform JavaScript, Back
   and Forward do nothing there, Flush Web Viewer Cookies does nothing, sites that refuse
   framing (`X-Frame-Options`, CSP `frame-ancestors`) stay blank, and pages have no cookies
   or storage of their own (opaque origin). Set Web Viewer's action codes and Perform JavaScript's parameter layout are confirmed in FileMaker;
   PerformScriptWithOption ignores its option (every call waits for the running script).
7. **Steps FileMaker runs on Windows that BaseCraft refuses**: the dialogs BaseCraft does
   not have: Open Manage Database, Manage Value
   Lists, Manage Layouts, Manage Data Sources, Manage Containers, Manage Themes, File
   Options, Sharing (error 9 without full access, else 3), Open Help, Edit Saved Finds,
   Hosts, Upload to Host, Favorites, Starter Solution (3). Open Script Workspace, Open
   Settings and Open Find/Replace open BaseCraft's own. Insert from URL, Save a Copy
   as XML, Save Records as JSONL and Execute FileMaker Data API run on the desktop build
   (partial, below) and still return 3 in the web build.

## Calculation functions: partial (34)

Left to `Context::host_function`, which no BaseCraft host answers (error 3, "not
available", or empty text where FileMaker gives that on a desktop): Location,
LocationValues, GetSensor, RangeBeacons, Furigana, GetLiveText, GetLiveTextAsJSON,
ReadQRCode, GetAVPlayerAttribute (iOS, location and media features: FileMaker on a desktop
answers empty or "not available"); GetTextFromPDF (no PDF text extractor in the build);
GetLayoutObjectOwnerInfo (layout object UUIDs are not decoded); GetEmbedding,
GetTokenCount, ComputeModel, PredictFromModel, GetModelAttributes, GetRAGSpaceInfo (AI
models).

Answered by the script engine (`script::hostfns`, in scripts and the Data Viewer):
GetPersistentData and ListPersistentDataIDs read Configure Persistent Data's entries (name
and instance id, the value's type kept, IDs in creation order, "?" when missing; kept in
memory for the session, not saved in the file). GetThumbnail (computed by calc, so in
every calculation) scales PNG, JPEG and BMP pictures down to fit, proportions kept, never
enlarged; a GIF comes back unscaled, a file path empty.

Short of FileMaker: TextColor, TextColorRemove, TextFont, TextFontRemove, TextFormatRemove,
TextSize, TextSizeRemove, TextStyleAdd, TextStyleRemove (styles dropped); CryptEncrypt,
CryptDecrypt, CryptEncryptBase64, CryptDecryptBase64 (own byte layout); ExecuteSQL,
ExecuteSQLe (no correlated subqueries or EXISTS, ModCount NULL, no FileMaker_ValueLists,
ExecuteSQLe's messages are BaseCraft's); GetAddonInfo (empty: no add-ons are installed;
FileMaker's answer for that is not documented or checked); GetLayoutObjectAttribute
(answers in scripts and the Data Viewer only, so empty in layout calculations; rotation 0;
the window's chrome is taken to sit above the content; the front tab panel is the stored
one; a graphic's source and a chart's content are empty, a web viewer's content is its
page's HTML once loaded (desktop app); `content` is the
value's text, not its display format).

Missing: none.

## `Get()` flags

ApplicationVersion answers "Pro 26.0.1", the FileMaker Pro release BaseCraft is checked
against (PROGRESS.md M2). Flags that need a server answer FileMaker's value for a local
file (HostName = the computer's name, MultiUserState 0, UserCount 1). FileName, FilePath,
FileSize, QuickFindText, WindowContentWidth/Height, ScreenWidth/Height and
ScreenScaleFactor come from the app (`script::HostInfo`; the coverage test fills it as the
app does). CurrentDate, CurrentTime, CurrentTimestamp and CurrentHostTimestamp are the
computer's local time (Windows' `ActiveTimeBias`, `date +%z` elsewhere, the JavaScript
`Date` offset in the browser); the UTC milli- and microseconds stay UTC. SystemLanguage is
`LANG` / `LC_ALL` or the OS's language (English in the browser). LastErrorDetail gives
Revert Transaction's custom message, else the failing script, step and line of a
transaction that fails or reverts; Claris's other cases (ODBC, SSL, cURL, plug-ins, locked
records) do not arise in BaseCraft. RecordAccess and LayoutAccess come from the privilege
set; EncryptionState is 0 (also for files opened with encryption at rest).

RecordOpenState and RecordOpenCount see a record a script opened, and the app's record
being edited when a script starts (an OnRecordCommit script sees it open: 1 new, 2
modified).

Partial (8): ActiveFieldName, ActiveFieldTableName, ActiveFieldContents,
ActiveRepetitionNumber, ActiveLayoutObjectName (empty / 0 with no active field, but only
fields and objects a script entered: the app's own focus is not passed to the engine);
ModifiedFields (only a record a script opened); ActiveModifierKeys (no Caps Lock);
RecordModificationCount (the file's record key 252, then each commit of a changed existing
record in the app or a script; written back on save, where a record changed without such a
commit (Replace Field Contents, Relookup, imports) counts one). LastErrorDetail is set by Revert Transaction, a
failed transaction, Insert from URL and Execute FileMaker Data API.

PageNumber is the page being previewed, printed or saved as PDF, 0 otherwise; PageCount
is the page count while printing or saving as PDF, empty otherwise (Preview included, as
Claris documents it: "the total page count in the current print job while it's
printing"; not checked against FileMaker). Both reach calculations (a footer field
`"Page " & Get ( PageNumber )`) and the `{{Symbol}}` text of layouts (below).

Missing (34): PrinterName, ScreenDepth, HighContrastState, SystemAppearance,
SystemIPAddress, SystemNICAddress, WindowDesktopHeight, WindowDesktopWidth, HostIPAddress,
AllowFormattingBarState, CustomMenuSetName, UseSystemFormatsState, HostApplicationVersion,
DocumentsPathListing, WindowStyle, PersistentID, TriggerGestureInfo, WindowOrientation,
NetworkType, ConnectionAttributes, TouchKeyboardState, RegionMonitorEvents,
OpenDataFileInfo, InstalledFMPluginsAsJSON, SessionIdentifier, SystemLocaleElements,
FileLocaleElements, CacheFileName, CacheFilePath, SystemStorageAvailable,
LastStepTokensUsed, GuidedAccessState, AccountPasswordDaysRemaining, WindowUUID.

## Script steps

Partial (29): 18 Check Selection, 19 Check Record, 20 Check Found Set (no spelling check);
35 Import Records, 36 Export Records
(stored options not used, the dialog opens); 37 Save a Copy as (stored path not used);
42 Print Setup (no dialog; pages are US Letter, portrait); 43 Print (no print dialog of
its own and no stored print options, see "Printing" below); 56 Insert Picture, 131 Insert File (stored path and options not
decoded, a file picker opens); 132 Export Field Contents (stored path not decoded);
63 Send Mail (`mailto:` only, no SMTP); 81 Scroll Window, 84 Set Multi-User, 94 Set Use
System Formats, 120 Arrange All Windows (do nothing); 128 Perform Find/Replace (a
script's "current field" searches every field); 138 Re-Login ("with dialog" shows none);
175 Perform JavaScript in Web Viewer (error 3 where there are no webviews); 169 Close Popover (does nothing); 209 Set
Dictionary (no dictionaries); 205 Open Transaction (Skip auto-enter / Skip data entry
validation / Override ESS locking options decoded, not applied); 237 Flush Web Viewer
Cookies (desktop app only).

Desktop build only (the web build returns 3; `crates/script/src/net.rs` and `net/`): 160
Insert from URL (in the app the request runs on its own thread and the script waits, the
window staying responsive; the CLI and MCP wait in the step; http and https only, other protocols 1626; the cURL options `-X`, `-H`,
`-d`, `--data-ascii`, `--data-raw`, `--data-binary`, `--data-urlencode` with `@$var`, `-I`,
`-D $var`, `-u` (basic only), `-b`, `-A`, `-e`, `-L`, `--connect-timeout`, `-m`, `-k`, `-S`,
`-o`, `--upload-file $var`, `--FM-return-container-variable`; the others, `--form` and
`--cookie-jar` among them, are ignored as FileMaker ignores unsupported ones; errors as
Claris documents them: 401 gives 1627, with `-S` 1631 and curl's message or "Response
code: nnn", no content 10, a bad URL 1630; "With dialog" shows no dialog); 203 Execute FileMaker Data API (read with query, omit, related
criteria, sort, offset, limit, `layout.response`, portals; metaData; create, update,
duplicate, delete; not done: `portalData` writes, `prohibitmode`, the `modId` check: every
record answers `"0"`; the request read as parameter 0); 225 Save Records as JSONL (both
formats; the plain format's predefined system prompt is not documented, so no system
message is written; the Table option is decoded only as its bit `0x100`: where FileMaker
keeps the table is not known, the layout's table is used); 3 Save a Copy as XML (`FMSaveAsXML`
2.3.0.0 with base tables, occurrences, relationships, custom value lists, fields with
calculation text, script catalog, layouts with parts and top-level objects, steps with id,
name, state and options; left out: UUIDs, `DDR_INFO`, `Metadata`, themes, custom menus,
accounts and privileges, auto-enter and validation, step parameters, objects inside
portals, panels and groups, object styles; not yet loaded in FileMaker's tools).

FileMaker's desktop error by design (67): listed in PROGRESS.md M5, "Step coverage".

## Script triggers: never fired (4)

8 OnObjectAVPlayerChange, 112 OnGestureTap, 113 OnExternalCommandReceived,
206 OnFileAVPlayerChange: their events do not happen on a desktop client (the AVPlayer
steps play media on FileMaker Go only, and gestures and lock-screen or remote commands
are FileMaker Go's), so there is nothing to fire them on.

Fired (22): OnObjectEnter, OnObjectExit, OnObjectModify, OnObjectSave, OnObjectKeystroke,
OnObjectValidate, OnPanelSwitch, OnLayoutEnter, OnLayoutKeystroke, OnRecordLoad,
OnRecordCommit, OnRecordRevert, OnModeEnter, OnModeExit, OnLayoutExit, OnViewChange,
OnLayoutSizeChange, OnFirstWindowOpen, OnLastWindowClose, OnWindowOpen, OnWindowClose,
OnWindowTransaction. The "pre" ones (OnObjectValidate, OnObjectKeystroke,
OnLayoutKeystroke, OnRecordCommit, OnRecordRevert, OnModeExit, OnPanelSwitch,
OnWindowClose, OnLastWindowClose, and OnObjectExit) run their script at once, and a False result (Exit
Script) cancels the event; the others are queued. Script steps fire them too
(`crates/script/src/triggers.rs`): pre triggers before the step (False: error 20), post
triggers after it, before the next step. Limits: PROGRESS.md, "More triggers".

## Layout objects and control styles

Partial objects (4): Graphic (PNG, JPEG, BMP pictures drawn in Browse, Preview and PDF; not
GIF, SVG or PDF pictures; no Insert › Picture in Layout mode), Rounded Rectangle
(drawn as read; no tool makes one), Web Viewer (its page in a native webview on Windows,
macOS and Linux under X11, a sandboxed iframe in the web version, a placeholder box with its address elsewhere; Layout mode edits its address and options), Chart (drawn; Chart Setup edits existing charts,
series added and removed; no Chart tool in Layout mode).

Text is drawn in the font the file asks for (theme and local font-family, font-weight,
font-style; the font of each style run, FORMAT.md §10), looked up among the installed fonts
(`print::fonts`, through fontique: Windows, macOS and Linux font folders, user fonts
included) and embedded whole in PDFs (TrueType or OpenType, WinAnsi encoding); a family that
is not installed gets the system's default sans-serif font, as FileMaker substitutes one, and
a family without a bold or italic face is emboldened or slanted. The web build keeps egui's
font, and Helvetica in PDFs; non-Latin text in PDFs is not covered (WinAnsi). Merge fields stored
as references resolve in Browse, Preview and PDF; Layout mode shows the raw reference.

Partial control styles (2): Checkbox Set, Radio Button Set (one checkbox or radio button per
value-list value, in columns, in Browse and Find modes; no "Other…" item, "Allow editing"
ignored, though their flags are decoded, see the top gaps). Drop-down Calendar (a month popup, the picked day written in the file's date
format) and Concealed Edit Box (bullets) are implemented.

## Error codes

"Raised" means the number appears in an error position of the calc, engine, script or app
source (a `u32` constant, `Err(n)`, `EvalError::new(n`, a validation rule's code). It is a
heuristic: a `u32` constant that happens to equal a code counts too, and a code computed
elsewhere would be missed.

Raised (65): 0, 1, 3, 4, 5, 9, 10, 12, 14, 15, 17, 20, 100, 101, 102, 104, 105, 106, 112, 116,
200, 201, 212, 217, 300, 400, 401, 414, 500, 501, 502, 503, 504, 505, 506, 507, 509, 511,
800, 829, 833, 1200–1209, 1211, 1212, 1214, 1215, 1218, 1223, 1224, 1225, 1626, 1627,
1630, 1631, 1708, 1710.

Never raised (235): -1, 2, 6, 7, 11, 13, 16, 18, 19, 21, 103, 107–111, 113–115, 117–119, 130,
131, 202–211, 214–216, 218, 219, 301–308, 310, 402, 404–410, 412, 413, 415–418, 508, 510,
512, 513, 600–608, 700, 706–708, 711, 714–727, 729–736, 738, 801–817, 819–827, 830–832,
850–853, 870–872, 875–879, 882, 883, 885–887, 892, 900, 901, 903, 905, 906, 920–923,
951–960, 1210, 1213, 1216, 1217, 1219–1222, 1300, 1301, 1400–1409, 1413–1415, 1450, 1451,
1501–1507, 1541–1543, 1550–1559, 1628, 1629, 1632–1635, 1638, 1700–1707, 1709, 1711–1715. Most belong to features
BaseCraft does not have (hosting, ODBC, plug-ins, email, web, AI, Data API); the ranges that
matter most for local files are 103–119, 202–211, 301–310, 402–418 and 508–513.

## Printing

Print (the step and File › Print…) renders the found set on the current layout to the
same PDF as Save Records as PDF, writes it to BaseCraft's own temporary file
(`basecraft-print-<pid>-<n>.pdf` in the system's temporary folder) and hands that file to
the OS with `std::process::Command` and explicit arguments (no shell parses anything of
the user's; the file name is BaseCraft's):

| | With dialog (File › Print…, the step's default) | Without dialog ("Perform without dialog") |
|---|---|---|
| Windows | `rundll32 url.dll,FileProtocolHandler <file>`: the default PDF viewer opens, the user prints from its Print dialog | `powershell -NoProfile -NonInteractive -Command "Start-Process -FilePath $env:BASECRAFT_PRINT -Verb Print"`, the path in the environment variable: the PDF handler's "print" verb, default printer |
| macOS | `open <file>` (Preview) | `lp <file>` (CUPS), default printer |
| Linux, BSD | `xdg-open <file>` | `lp <file>` |
| Browser | the PDF is downloaded | the same |

When the print verb or `lp` fails (no handler registers "print", no CUPS), the PDF opens in
the viewer instead. Gaps: no print dialog of BaseCraft's own (FileMaker's shows printer,
copies, records being browsed / current record / blank record), no Print Setup dialog and
no stored print options (printer, paper, orientation): pages are US Letter, portrait. An
account without the "Allow printing" privilege gets FileMaker's privilege message, and the
File › Print… item is off. Tests never reach the OS.

Symbols in layout text (Preview and PDF): `{{Name}}` shows `Get ( Name )` for any `Get`
flag, the name in any case with nothing else between the braces (Claris, "Inserting the
date, page number, or other variable onto a layout"): `{{PageNumber}}`, `{{PageCount}}`,
`{{CurrentDate}}`, `{{CurrentTime}}`, `{{UserName}}`, `{{RecordNumber}}`, `{{AccountName}}`…;
other text in braces stays as typed. Browse and Find modes do not resolve them yet, and
`{{RecordNumber}}` in a portal row gives the record's number, not the row's. The pre-12
symbols `##`, `//`, `::`, `@@` are not handled: Claris's current help documents only the
`{{…}}` forms, and replacing `//` or `::` in any text would break URLs and field names.

## Import and export

Formats (Claris, "Supported import/export file formats"), as `engine::export` and
`engine::import` handle them; none of the rules below has been compared byte for byte with
a FileMaker export except the HTML table:

| Format | Export | Import |
|---|---|---|
| Comma-separated, tab-separated, Merge | ✅ quotes, VT for returns, GS between repetitions and related values, tabs as spaces; output character set | ✅ character set (a byte order mark wins), custom delimiter |
| Custom-separated | — (FileMaker: import only) | ✅ any one-character delimiter, quoted values as in CSV |
| FMPXMLRESULT XML | ✅ one `DATA` per repetition (MAXREPEAT) and per related record | ✅ METADATA names, one value per `DATA`; FMPDSORESULT and XSLT ❌ |
| HTML table | ✅ (repeating fields as a nested table) | — (export only) |
| Excel | ✅ | ✅ |
| dBASE DBF | ✅ DBF III, Claris's limits enforced (an error when two names collide once cut) | ✅ C/N/D/L fields; memo (`.dbt`) ❌ |
| FileMaker Pro | ✅ a new file, one table, field types and repetitions; its layout has no field objects 🔸 | ✅ one source table at a time |
| Folder of files | — | ✅ pictures and movies (Image, File Name, File Path, Image Thumbnail), text files (Text Content, File Name, File Path), enclosed folders, references only; desktop only |
| ODBC | — | ❌ |

Character sets: UTF-8, UTF-16 (little endian with a byte order mark on export), Windows ANSI
(code page 1252), Macintosh (Mac OS Roman), ASCII (7-bit; FileMaker's "ASCII (DOS)" is
probably an OEM code page 🔸). "Apply current layout's data formatting" uses the current
layout's field objects' data formats (the first object showing the field); it is dimmed for
DBF and FileMaker Pro. Gaps: Number fields export as stored (FileMaker documents exporting
only their numeric characters), XML dates are imported as written, auto-enter on import is
all-or-nothing, and the Import / Export Records script steps still open the dialogs (their
stored options are not decoded yet: `engine::import::Options` and `engine::export::Options`
are what a decoder fills).
