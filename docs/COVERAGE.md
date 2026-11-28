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
| Calculation functions | [functions.txt](reference/functions.txt) | 234 | 197 | 37 | 0 | `cargo test -p calc registry_coverage -- --nocapture` |
| `Get()` flags | [get_functions.txt](reference/get_functions.txt) | 139 | 53 | 8 | 78 | `cargo test -p script get_coverage -- --nocapture` |
| Script steps | [script_steps.txt](reference/script_steps.txt) | 222 | 128 | 23 | 0 (+71 desktop error) | `cargo test -p script step_coverage -- --nocapture` |
| Script triggers | [script_triggers.txt](reference/script_triggers.txt) | 26 | 22 | 0 | 4 | `cargo test -p basecraft trigger_coverage -- --nocapture` |
| Layout object kinds | [layout_objects.txt](reference/layout_objects.txt) | 19 | 15 | 4 | 0 | `cargo test -p basecraft layout_object_coverage -- --nocapture` |
| Field control styles | [control_styles.txt](reference/control_styles.txt) | 7 | 3 | 3 | 1 | `cargo test -p basecraft control_style_coverage -- --nocapture` |
| Error codes | [error_codes.txt](reference/error_codes.txt) | 300 | 58 raised | — | 242 never raised | `cargo test -p basecraft error_code_coverage -- --nocapture` |

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
  DBEngine.dll's string table; kinds and ids from FORMAT.md §10. The control style id of
  Concealed Edit Box is not decoded.
* **Error codes**: the code column of Claris's public error code page, cross-checked
  against the English error messages DBEngine.dll carries (numbers only; the reserved
  range 5000–5499 left out).

The resource tables were read with REA's PE resource inspector (FMRSRC.dll) and a plain
string scan of DBEngine.dll, FMEngine.dll, ClientUI.dll and Support.dll; no code was
decompiled.

## Top gaps by likely user impact

1. **`Get ( UUIDNumber )`, `Get ( FileName )`, `Get ( FilePath )`, `Get ( ApplicationVersion )`,
   `Get ( SystemLanguage )`, `Get ( ModifiedFields )`, `Get ( ActiveRepetitionNumber )`,
   `Get ( RecordModificationCount )`, `Get ( ActiveLayoutObjectName )`, `Get ( QuickFindText )`,
   `Get ( WindowContentWidth/Height )`, `Get ( DesktopPath/DocumentsPath/TemporaryPath )`**: common
   in auto-enter keys and scripts; they fail with error 1225 ("needs a host") where FileMaker
   answers. `Get ( PrivilegeSetName )` does not even compile in a script.
2. **Script triggers fired by script steps**: triggers fire on user actions; of the steps,
   only a layout change, New Window and a view change fire them (FileMaker also fires
   OnModeEnter/Exit, OnRecordRevert, OnWindowClose… for Enter Find Mode, Revert Record,
   Close Window…).
3. **Checkbox sets, radio button sets and drop-down calendars** show as edit boxes in
   Browse mode (PROGRESS.md M4).
4. **Set Revert Transaction on Error** does nothing, and **Perform Find** drops criteria
   on related fields: scripts can behave differently from FileMaker without an error.
5. **Styled-text functions** (TextColor, TextStyleAdd, …) keep the plain text only, and
   **CryptEncrypt / CryptDecrypt** use their own byte layout (not readable by FileMaker).
6. **Web viewers** are a placeholder box; Set Web Viewer, Perform JavaScript in Web Viewer
   and Flush Web Viewer Cookies have nothing to act on.
7. **Steps FileMaker runs on Windows that BaseCraft refuses**: Insert from URL (no HTTP
   client), Save a Copy as XML, Save Records as JSONL, Execute FileMaker Data API,
   Perform JavaScript in Web Viewer, Open Manage Database and the other design dialogs.

## Calculation functions: partial (37)

Left to `Context::host_function`, which no BaseCraft host implements yet (error 3, "not
available", or empty text where FileMaker gives that on a desktop): Location,
LocationValues, GetSensor, RangeBeacons, Furigana, GetLiveText, GetLiveTextAsJSON,
ReadQRCode, GetTextFromPDF, GetThumbnail, GetAVPlayerAttribute, GetAddonInfo,
GetPersistentData, ListPersistentDataIDs, GetLayoutObjectAttribute,
GetLayoutObjectOwnerInfo, GetEmbedding, GetTokenCount, ComputeModel, PredictFromModel,
GetModelAttributes, GetRAGSpaceInfo.

Short of FileMaker: TextColor, TextColorRemove, TextFont, TextFontRemove, TextFormatRemove,
TextSize, TextSizeRemove, TextStyleAdd, TextStyleRemove (styles dropped); CryptEncrypt,
CryptDecrypt, CryptEncryptBase64, CryptDecryptBase64 (own byte layout); ExecuteSQL,
ExecuteSQLe (no joins, GROUP BY or system tables).

Missing: none.

## `Get()` flags

Partial (8): ActiveFieldName, ActiveFieldTableName, ActiveFieldContents (only while a field
is active; error 1225 instead of empty otherwise); CurrentDate, CurrentTime,
CurrentTimestamp, CurrentHostTimestamp (UTC: no time-zone data); LastErrorDetail (always
empty).

Missing (78): ApplicationVersion, HostName, FileName, FileSize, SystemLanguage, PageNumber,
MultiUserState, PrinterName, ScreenDepth, ScreenHeight, ActiveRepetitionNumber,
SystemVersion, ScreenWidth, UserCount, ActiveModifierKeys, NetworkProtocol, FilePath,
RecordModificationCount, RecordAccess, HighContrastState, LayoutAccess, SystemAppearance,
SystemIPAddress, SystemNICAddress, ApplicationLanguage, WindowContentHeight,
WindowContentWidth, WindowDesktopHeight, WindowDesktopWidth, FileMakerPath, HostIPAddress,
DesktopPath, DocumentsPath, RequestOmitState, PreferencesPath, TextRulerVisible,
AllowFormattingBarState, SystemDrive, CustomMenuSetName, ActiveLayoutObjectName,
UseSystemFormatsState, TemporaryPath, HostApplicationVersion, QuickFindText,
DocumentsPathListing, WindowStyle, InstalledFMPlugins, ConnectionState, PersistentID,
TriggerGestureInfo, EncryptionState, Device, WindowOrientation, NetworkType,
ConnectionAttributes, ModifiedFields, TouchKeyboardState, ScreenScaleFactor,
AccountGroupName, ActiveRecordNumber, MenubarState, RegionMonitorEvents, PageCount,
UUIDNumber, OpenDataFileInfo, InstalledFMPluginsAsJSON, SessionIdentifier,
SystemLocaleElements, FileLocaleElements, CacheFileName, CacheFilePath,
RevertTransactionOnErrorState, SystemStorageAvailable, LastStepTokensUsed,
GuidedAccessState, AccountPasswordDaysRemaining, WindowUUID, PrivilegeSetName (not known
to the script compiler).

## Script steps

Partial (23): 18 Check Selection, 19 Check Record, 20 Check Found Set (no spelling check);
28 Perform Find (criteria on related fields dropped); 35 Import Records, 36 Export Records
(stored options not used, the dialog opens); 37 Save a Copy as (stored path not used);
43 Print (to PDF only); 56 Insert Picture, 131 Insert File (stored path and options not
decoded, a file picker opens); 132 Export Field Contents (stored path not decoded);
63 Send Mail (`mailto:` only, no SMTP); 81 Scroll Window, 84 Set Multi-User, 94 Set Use
System Formats, 120 Arrange All Windows (do nothing); 128 Perform Find/Replace (a
script's "current field" searches every field); 138 Re-Login ("with dialog" shows none);
146 Set Web Viewer (only checks the name); 169 Close Popover (does nothing); 209 Set
Dictionary (no dictionaries); 223 Set Revert Transaction on Error (not kept); 237 Flush
Web Viewer Cookies (no web viewer).

FileMaker's desktop error by design (71): listed in PROGRESS.md M5, "Step coverage".

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
OnLayoutKeystroke, OnRecordRevert, OnModeExit, OnPanelSwitch, OnWindowClose,
OnLastWindowClose) run their script at once, and a False result (Exit Script) cancels
the event; the others are queued. Limits: PROGRESS.md, "More triggers".

## Layout objects and control styles

Partial objects (4): Graphic (drawn; no Insert › Picture in Layout mode), Rounded Rectangle
(drawn as read; no tool makes one), Web Viewer (a placeholder box with its address),
Chart (drawn; no Chart tool in Layout mode).

Partial control styles (3): Checkbox Set, Radio Button Set, Drop-down Calendar (chosen in
the Inspector, drawn as an edit box in Browse mode). Missing (1): Concealed Edit Box.

## Error codes

"Raised" means the number appears in an error position of the calc, engine, script or app
source (a `u32` constant, `Err(n)`, `EvalError::new(n`, a validation rule's code). It is a
heuristic: a `u32` constant that happens to equal a code counts too, and a code computed
elsewhere would be missed.

Raised (58): 0, 1, 3, 4, 5, 9, 10, 12, 14, 15, 17, 100, 101, 102, 104, 105, 106, 112, 116,
200, 201, 212, 217, 300, 400, 401, 414, 500, 501, 502, 503, 504, 505, 506, 507, 509, 511,
800, 829, 833, 1200–1209, 1211, 1212, 1214, 1215, 1218, 1223, 1224, 1225.

Never raised (242): -1, 2, 6, 7, 11, 13, 16, 18–21, 103, 107–111, 113–115, 117–119, 130,
131, 202–211, 214–216, 218, 219, 301–308, 310, 402, 404–410, 412, 413, 415–418, 508, 510,
512, 513, 600–608, 700, 706–708, 711, 714–727, 729–736, 738, 801–817, 819–827, 830–832,
850–853, 870–872, 875–879, 882, 883, 885–887, 892, 900, 901, 903, 905, 906, 920–923,
951–960, 1210, 1213, 1216, 1217, 1219–1222, 1300, 1301, 1400–1409, 1413–1415, 1450, 1451,
1501–1507, 1541–1543, 1550–1559, 1626–1635, 1638, 1700–1715. Most belong to features
BaseCraft does not have (hosting, ODBC, plug-ins, email, web, AI, Data API); the ranges that
matter most for local files are 103–119, 202–211, 301–310, 402–418 and 508–513.
