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
| `Get()` flags | [get_functions.txt](reference/get_functions.txt) | 139 | 95 | 8 | 36 | `cargo test -p script get_coverage -- --nocapture` |
| Script steps | [script_steps.txt](reference/script_steps.txt) | 222 | 128 | 27 | 0 (+67 desktop error) | `cargo test -p script step_coverage -- --nocapture` |
| Script triggers | [script_triggers.txt](reference/script_triggers.txt) | 26 | 22 | 0 | 4 | `cargo test -p basecraft trigger_coverage -- --nocapture` |
| Layout object kinds | [layout_objects.txt](reference/layout_objects.txt) | 19 | 15 | 4 | 0 | `cargo test -p basecraft layout_object_coverage -- --nocapture` |
| Field control styles | [control_styles.txt](reference/control_styles.txt) | 7 | 5 | 2 | 0 | `cargo test -p basecraft control_style_coverage -- --nocapture` |
| Error codes | [error_codes.txt](reference/error_codes.txt) | 300 | 65 raised | — | 235 never raised | `cargo test -p basecraft error_code_coverage -- --nocapture` |

Not inventoried: script step options (each step's parameters are covered step by step in
FORMAT.md §8 and PROGRESS.md M5), and File Options / Preferences (FileMaker 26 draws those
dialogs from compiled WPF resources, which have no plain list of settings).

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
2. **Revert Transaction**'s condition, error code and message are read from the step
   by guess (no FileMaker-written step checked them), and Open Transaction's Skip
   auto-enter / Skip validation options are ignored.
3. **Checkbox sets and radio button sets** have no "Other…" item and ignore "Allow
   editing"; their columns are not checked against FileMaker's (PROGRESS.md M4). The
   flags of those two options are not known: the local Claris templates hold one checkbox
   set (field options `0x20`) and no radio button set, too little to tell which bit is
   which.
4. **`Get()` flags a local solution still misses**: PageNumber and PageCount (page
   headers in Preview and PDF), SystemLocaleElements / FileLocaleElements, CustomMenuSetName
   (custom menu sets are not decoded), WindowStyle, WindowUUID, PersistentID (error 1225
   where FileMaker answers). The active-field flags see only fields a script entered, and
   RecordModificationCount is kept in memory (a save keeps the file's previous count). The
   other missing flags belong to servers, FileMaker Go or plug-ins.
5. **Styled-text functions** (TextColor, TextStyleAdd, …) keep the plain text only, and
   **CryptEncrypt / CryptDecrypt** use their own byte layout (not readable by FileMaker).
6. **Web viewers** are a placeholder box; Set Web Viewer, Perform JavaScript in Web Viewer
   and Flush Web Viewer Cookies have nothing to act on.
7. **Steps FileMaker runs on Windows that BaseCraft refuses**: Perform JavaScript in Web
   Viewer, Open Manage Database and the other design dialogs. Insert from URL, Save a Copy
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
one; a graphic's source and a chart's or web viewer's content are empty; `content` is the
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
set; EncryptionState is 0 (encrypted files are not opened).

RecordOpenState and RecordOpenCount see a record a script opened, and the app's record
being edited when a script starts (an OnRecordCommit script sees it open: 1 new, 2
modified).

Partial (8): ActiveFieldName, ActiveFieldTableName, ActiveFieldContents,
ActiveRepetitionNumber, ActiveLayoutObjectName (empty / 0 with no active field, but only
fields and objects a script entered: the app's own focus is not passed to the engine);
ModifiedFields (only a record a script opened); ActiveModifierKeys (no Caps Lock);
RecordModificationCount (the file's record key 252, then each commit of a changed existing
record in the app or a script; kept in memory, not written on save; Replace Field Contents,
Relookup and imports do not count). LastErrorDetail is set by Revert Transaction, a
failed transaction, Insert from URL and Execute FileMaker Data API.

Missing (36): PageNumber, PrinterName, ScreenDepth, HighContrastState, SystemAppearance,
SystemIPAddress, SystemNICAddress, WindowDesktopHeight, WindowDesktopWidth, HostIPAddress,
AllowFormattingBarState, CustomMenuSetName, UseSystemFormatsState, HostApplicationVersion,
DocumentsPathListing, WindowStyle, PersistentID, TriggerGestureInfo, WindowOrientation,
NetworkType, ConnectionAttributes, TouchKeyboardState, RegionMonitorEvents, PageCount,
OpenDataFileInfo, InstalledFMPluginsAsJSON, SessionIdentifier, SystemLocaleElements,
FileLocaleElements, CacheFileName, CacheFilePath, SystemStorageAvailable,
LastStepTokensUsed, GuidedAccessState, AccountPasswordDaysRemaining, WindowUUID.

## Script steps

Partial (27): 18 Check Selection, 19 Check Record, 20 Check Found Set (no spelling check);
35 Import Records, 36 Export Records
(stored options not used, the dialog opens); 37 Save a Copy as (stored path not used);
43 Print (to PDF only); 56 Insert Picture, 131 Insert File (stored path and options not
decoded, a file picker opens); 132 Export Field Contents (stored path not decoded);
63 Send Mail (`mailto:` only, no SMTP); 81 Scroll Window, 84 Set Multi-User, 94 Set Use
System Formats, 120 Arrange All Windows (do nothing); 128 Perform Find/Replace (a
script's "current field" searches every field); 138 Re-Login ("with dialog" shows none);
146 Set Web Viewer (only checks the name); 169 Close Popover (does nothing); 209 Set
Dictionary (no dictionaries); 205 Open Transaction (Skip auto-enter / Skip data entry
validation ignored); 207 Revert Transaction (condition, error code and message read as
parameters 0–2, not checked against a FileMaker-written step); 237 Flush Web Viewer
Cookies (no web viewer).

Desktop build only (the web build returns 3; `crates/script/src/net.rs` and `net/`): 160
Insert from URL (http and https only, other protocols 1626; the cURL options `-X`, `-H`,
`-d`, `--data-ascii`, `--data-raw`, `--data-binary`, `--data-urlencode` with `@$var`, `-I`,
`-D $var`, `-u` (basic only), `-b`, `-A`, `-e`, `-L`, `--connect-timeout`, `-m`, `-k`, `-S`,
`-o`, `--upload-file $var`, `--FM-return-container-variable`; the others, `--form` and
`--cookie-jar` among them, are ignored as FileMaker ignores unsupported ones; errors as
Claris documents them: 401 gives 1627, with `-S` 1631 and curl's message or "Response
code: nnn", no content 10, a bad URL 1630; "With dialog" shows no dialog; "Automatically
encode URL" is always on); 203 Execute FileMaker Data API (read with query, omit, related
criteria, sort, offset, limit, `layout.response`, portals; metaData; create, update,
duplicate, delete; not done: `portalData` writes, `prohibitmode`, the `modId` check: every
record answers `"0"`; the request read as parameter 0); 225 Save Records as JSONL (both
formats; the plain format's predefined system prompt is not documented, so no system
message is written; prompts read as parameters 1–3 and the completion field as the inline
field, not checked against a FileMaker-written step); 3 Save a Copy as XML (`FMSaveAsXML`
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
(drawn as read; no tool makes one), Web Viewer (a placeholder box with its address;
Layout mode edits its address and options), Chart (drawn; Chart Setup edits existing charts,
not the number of series; no Chart tool in Layout mode).

Text objects draw in one style, their theme style's: style runs inside a text (several fonts,
sizes or colours) are not decoded, for want of a sample (FORMAT.md §10). Merge fields stored
as references resolve in Browse, Preview and PDF; Layout mode shows the raw reference.

Partial control styles (2): Checkbox Set, Radio Button Set (one checkbox or radio button per
value-list value, in columns, in Browse and Find modes; no "Other…" item, "Allow editing"
ignored: the flags of these options are not decoded, see the top gaps). Drop-down Calendar (a month popup, the picked day written in the file's date
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
