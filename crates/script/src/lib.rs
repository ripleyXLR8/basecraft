//! Script engine: runs FileMaker scripts decoded by [`fmp12::scripts`]
//! against an [`engine::Database`], with FileMaker's semantics for control
//! flow, `$local`/`$$global` variables, script parameters and results,
//! `Get ( LastError )` with Set Error Capture, and found sets.
//!
//! Everything a window shows (layout, mode, found set, current record) is
//! a [`Window`] the [`Host`] owns; dialogs, field focus and redrawing go
//! through the [`Host`] trait, so the engine runs headless in tests.
//!
//! Scripts are resumable, so that a GUI that cannot block (egui, the web)
//! can wait for the user: [`Engine::start`] (or [`Engine::start_step`]
//! for a button) returns a [`Run`], and [`Run::resume`] runs it until it
//! finishes or stops on Show Custom Dialog, a confirmation, or Pause/Resume
//! Script ([`Status::Suspended`]); the next `resume` carries the user's
//! [`Answer`]. No threads, so it works on `wasm32` too.
//!
//! [`Engine::perform`], [`Engine::perform_named`] and [`Engine::run_step`]
//! run to the end, answering dialogs through [`Host::dialog`] (button 1 by
//! default) and continuing pauses: the CLI and MCP server use those.

use std::cmp::Ordering;
use std::collections::HashMap;

mod accounts;
pub mod datafiles;
pub mod files;
mod hostfns;
mod net;
mod steps;
pub mod text;
mod triggers;
pub use triggers::CANCELED;

pub use steps::{UNSUPPORTED, stored_path};

use calc::Value;
use engine::replace::{
    Direction, FindReplace, Hit, ReplaceWith, find_next, replace_field_contents,
};
use engine::{Database, Request};
use fmp12::scripts::{
    Calc, FieldRef, LayoutDest, RecordDest, Script, SortOrder, Step, StepParams, Target, id,
};

/// FileMaker error codes the engine reports.
/// Container steps (FileMaker step ids).
const INSERT_PICTURE: u16 = 56;
const INSERT_FILE: u16 = 131;
const EXPORT_FIELD_CONTENTS: u16 = 132;

pub mod error {
    pub const USER_CANCELED: u32 = 1;
    pub const UNAVAILABLE: u32 = 3;
    pub const RECORD_MISSING: u32 = 101;
    pub const FIELD_MISSING: u32 = 102;
    pub const SCRIPT_MISSING: u32 = 104;
    pub const LAYOUT_MISSING: u32 = 105;
    /// Go to Object, Set Web Viewer: no object of that name (FileMaker 26).
    pub const OBJECT_MISSING: u32 = 116;
    pub const NO_CRITERIA: u32 = 400;
    pub const NO_MATCH: u32 = 401;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Browse,
    Find,
    Preview,
}

/// What a window shows. The host owns it; scripts change it.
#[derive(Debug, Clone)]
pub struct Window {
    /// Current layout id (`[4].[5].[L]`).
    pub layout: u32,
    pub mode: Mode,
    /// Found set: record indexes into the layout table's `records`, in
    /// display order.
    pub found: Vec<usize>,
    /// Current record: index into `found`.
    pub current: usize,
    pub sorted: bool,
    /// Fields of the layout's table the found set is sorted by, in order
    /// (GetSummary needs its break field among them); empty when unsorted.
    pub sort: Vec<u32>,
    /// Find requests (Find mode) and the current one.
    pub requests: Vec<Request>,
    pub current_request: usize,
    /// Window name (`Get ( WindowName )`), set by New Window and Set
    /// Window Title; empty for the host's main window (the host shows
    /// the file name).
    pub name: String,
    /// View As: form, list or table (`Get ( LayoutViewState )` 0, 1, 2).
    pub view: View,
    /// Zoom percentage (Set Zoom Level), 100 by default.
    pub zoom: u32,
    /// Show/Hide Toolbars (`Get ( StatusAreaState )`), menu bar, text ruler.
    pub toolbar: bool,
    pub menubar: bool,
    pub ruler: bool,
    /// Move/Resize Window: top, left, width, height in points.
    pub top: f64,
    pub left: f64,
    pub width: f64,
    pub height: f64,
    /// Adjust Window: hidden or minimized.
    pub hidden: bool,
    /// Field with the focus (Go to Field, Go to Next/Previous Field) and
    /// its selection (Set Selection: start, size; 0-based characters).
    pub field: Option<FieldRef>,
    pub selection: (usize, usize),
    /// Current portal row, 1-based (0: no portal row is active).
    pub portal_row: usize,
    /// Requests of the last find (Modify Last Find, Constrain/Extend).
    pub last_find: Vec<Request>,
    /// Layout object given the focus by Go to Object (`Get (
    /// ActiveLayoutObjectName )`); cleared when a field is entered otherwise.
    pub object: String,
    /// What only the application knows: the file, the screen, Quick Find.
    pub host: HostInfo,
}

/// The host's answers to `Get ( FileName )`, `Get ( ScreenWidth )`…,
/// filled by the application for each script run. `None` (or an empty
/// file name): the flag needs a host.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostInfo {
    /// File name without its extension.
    pub file_name: String,
    /// FileMaker path of the file (`file:/C:/dir/x.fmp12`); empty when it
    /// is not on disk.
    pub file_path: String,
    /// Size of the file in bytes.
    pub file_size: Option<u64>,
    /// Width and height in points of the window's content area (less the
    /// title bar, menu bar and toolbars).
    pub content: Option<(f64, f64)>,
    /// Width and height in points of the window's screen.
    pub screen: Option<(f64, f64)>,
    /// Pixels per point of the window's screen.
    pub scale: Option<f64>,
    /// Modifier keys held down: Shift 1, Caps Lock 2, Ctrl 4, Alt/Option
    /// 8, Command 16.
    pub modifier_keys: Option<u32>,
    /// Text of the window's Quick Find box.
    pub quick_find: String,
    /// The app's record being edited, if it is the window's current
    /// record: 1 new, 2 modified (`Get ( RecordOpenState )`); 0 none.
    /// A script started then sees it open (an OnRecordCommit script).
    pub record_open: u8,
    /// Pages shown by the layout's web viewers, by object name: (URL,
    /// HTML source), for GetLayoutObjectAttribute "source" and "content".
    pub web_pages: std::collections::BTreeMap<String, (String, String)>,
}

/// View As.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Form,
    List,
    Table,
}

impl Default for Window {
    /// 100 %, toolbar and menu bar shown, 800 × 600.
    fn default() -> Window {
        Window {
            layout: 0,
            mode: Mode::Browse,
            found: Vec::new(),
            current: 0,
            sorted: false,
            sort: Vec::new(),
            requests: Vec::new(),
            current_request: 0,
            name: String::new(),
            view: View::Form,
            zoom: 100,
            toolbar: true,
            menubar: true,
            ruler: false,
            top: 0.0,
            left: 0.0,
            width: 800.0,
            height: 600.0,
            hidden: false,
            field: None,
            selection: (0, 0),
            portal_row: 0,
            last_find: Vec::new(),
            object: String::new(),
            host: HostInfo::default(),
        }
    }
}

impl Window {
    /// A window on `layout` showing every record of its table.
    pub fn new(db: &Database, layout: u32) -> Window {
        let mut w = Window {
            layout,
            ..Default::default()
        };
        if let Some(t) = layout_table(db, layout) {
            w.found = (0..t.1.records.len()).collect();
        }
        w
    }

    /// Record index (in the table) of the current record.
    pub fn record(&self) -> Option<usize> {
        self.found.get(self.current).copied()
    }
}

/// Show Custom Dialog, as the host should display it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dialog {
    pub title: String,
    pub message: String,
    /// Button labels, left to right as FileMaker numbers them (1 = default).
    pub buttons: Vec<String>,
    /// Input fields: (field, label, current value).
    pub inputs: Vec<(FieldRef, String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DialogAnswer {
    /// Button pressed, 1-based (`Get ( LastMessageChoice )`).
    pub button: usize,
    /// Values typed into the input fields.
    pub inputs: Vec<String>,
}

/// The application side of a running script.
pub trait Host {
    fn db(&mut self) -> &mut Database;
    fn window(&mut self) -> &mut Window;
    /// Show Custom Dialog (and confirmations). Headless default: the
    /// first button, inputs unchanged.
    fn dialog(&mut self, d: &Dialog) -> DialogAnswer {
        DialogAnswer {
            button: 1,
            inputs: d.inputs.iter().map(|i| i.2.clone()).collect(),
        }
    }
    /// Go to Field (`None`: leave every field). `false` if not on the layout.
    fn go_to_field(&mut self, field: Option<FieldRef>, select: bool) -> bool {
        let _ = (field, select);
        true
    }
    /// Go to Object. `false` if there is no such object.
    fn go_to_object(&mut self, name: &str, repetition: u32) -> bool {
        let _ = (name, repetition);
        true
    }
    /// Text on the clipboard, for Paste (`None`: nothing to paste).
    fn clipboard(&mut self) -> Option<String> {
        None
    }
    fn refresh(&mut self) {}
    fn freeze(&mut self) {}
    fn beep(&mut self) {}
    /// An error while Set Error Capture is off: FileMaker shows it and lets
    /// the user continue (`true`) or cancel the script (`false`).
    fn error(&mut self, step: &Step, code: u32) -> bool {
        let _ = (step, code);
        true
    }
    /// Insert Picture / Insert File: the file to insert, as (name, bytes);
    /// `None` when cancelled (the default: no file system).
    fn pick_file(&mut self, picture: bool) -> Option<(String, Vec<u8>)> {
        let _ = picture;
        None
    }
    /// Export Field Contents: saves a container's file; `false` if cancelled.
    fn save_file(&mut self, name: &str, data: &[u8]) -> bool {
        let _ = (name, data);
        false
    }
    /// Steps the engine does not run itself: returns the error code
    /// (default 3, "command unavailable").
    fn other(&mut self, step: &Step) -> u32 {
        let _ = step;
        error::UNAVAILABLE
    }
    /// Cut, Copy, Copy Record, Copy All Records: text for the clipboard.
    fn set_clipboard(&mut self, text: String) {
        let _ = text;
    }
    /// Data file steps. Paths are FileMaker paths (`file:x.txt`, relative
    /// to the database file's folder; [`files::resolve`]). `None`: no
    /// such file or folder. A host with no file system (the default)
    /// answers 3; [`files::Disk`] is the local one.
    fn file_stat(&mut self, path: &str) -> Result<Option<FileStat>, u32> {
        let _ = path;
        Err(error::UNAVAILABLE)
    }
    fn file_read(&mut self, path: &str) -> Result<Vec<u8>, u32> {
        let _ = path;
        Err(error::UNAVAILABLE)
    }
    /// Writes (replaces) a whole file; `folders` creates missing folders.
    /// [`files::CONFIRM`] when the path leaves the file's folder and the
    /// user has not agreed yet ([`Host::file_confirm`]).
    fn file_write(&mut self, path: &str, data: &[u8], folders: bool) -> Result<(), u32> {
        let _ = (path, data, folders);
        Err(error::UNAVAILABLE)
    }
    fn file_delete(&mut self, path: &str) -> Result<(), u32> {
        let _ = path;
        Err(error::UNAVAILABLE)
    }
    fn file_rename(&mut self, from: &str, to: &str) -> Result<(), u32> {
        let _ = (from, to);
        Err(error::UNAVAILABLE)
    }
    /// The user agreed to let the script write `path`.
    fn file_confirm(&mut self, path: &str) {
        let _ = path;
    }
    /// What a headless host answers to a [`UiRequest`] when the run is
    /// finished with [`Run::finish`]: nothing is shown, the step goes on
    /// (pickers are cancelled).
    /// Script triggers fired by steps (`triggers.rs`): the script and
    /// parameter of the trigger set for `event` (`fmp12::triggers::event`)
    /// on layout object `object` of the window's layout, else on the
    /// window's layout in its mode (events 100–199) or on the file (200 and
    /// up). `None` (the default): nothing runs.
    fn trigger(&mut self, event: u16, object: Option<u64>) -> Option<(u32, Option<Calc>)> {
        let _ = (event, object);
        None
    }
    /// Insert from URL runs its request on another thread and the run
    /// waits ([`UiRequest::Fetch`]) so the window stays responsive.
    /// Default (CLI, MCP): the step waits for the answer itself.
    fn background_requests(&self) -> bool {
        false
    }
    fn ui(&mut self, r: &UiRequest) -> Answer {
        match r {
            UiRequest::Index { .. } | UiRequest::FolderPath { .. } => Answer::Cancel,
            _ => Answer::Continue,
        }
    }
    /// Set Web Viewer and Perform JavaScript in Web Viewer on the web
    /// viewer `name` of the window's layout (checked to exist), Flush Web
    /// Viewer Cookies (`name` empty). Returns an error code. Default (no
    /// web viewers): nothing happens, Perform JavaScript answers 3.
    fn web_viewer(&mut self, name: &str, action: WebAction) -> u32 {
        let _ = name;
        match action {
            WebAction::Js { .. } => error::UNAVAILABLE,
            _ => 0,
        }
    }
}

/// What a script asks of a web viewer ([`Host::web_viewer`]).
#[derive(Debug, Clone, PartialEq)]
pub enum WebAction {
    /// Set Web Viewer: back to its address calculation.
    Reset,
    Reload,
    Forward,
    Back,
    /// Set Web Viewer › Go to URL.
    Go(String),
    /// Perform JavaScript in Web Viewer: `function(parameters…)`, each
    /// parameter passed as a string.
    Js {
        function: String,
        parameters: Vec<String>,
    },
    FlushCookies,
}

/// Something only the application can do (a dialog, a file, the OS):
/// the run waits ([`Suspend::Ui`]) and goes on with [`Answer::Continue`]
/// (done), [`Answer::Cancel`] (error 1) or [`Answer::Text`] (a value).
#[derive(Debug, Clone, PartialEq)]
pub enum UiRequest {
    /// Open URL (and Send Mail as a `mailto:` URL): the host asks before
    /// handing it to the OS. `dialog` = "With dialog: On".
    OpenUrl {
        url: String,
        dialog: bool,
    },
    /// Sort Records with dialog (the stored order is already applied).
    Sort,
    /// Insert from Index: the values of the field's index; answer with
    /// the chosen one ([`Answer::Text`]).
    Index {
        values: Vec<String>,
    },
    /// Get Folder Path: answer with the folder ([`Answer::Text`]).
    FolderPath {
        title: String,
        default: String,
    },
    /// Print, Print Setup; Save Records as PDF / Excel; Export Field
    /// Contents (`text`); Save a Copy as; Import / Export Records. `path`
    /// is the stored output file, if any.
    Print {
        dialog: bool,
    },
    PrintSetup,
    SavePdf {
        path: Option<String>,
        dialog: bool,
    },
    SaveExcel {
        path: Option<String>,
        dialog: bool,
    },
    ExportField {
        path: Option<String>,
        text: String,
    },
    SaveCopy {
        path: Option<String>,
    },
    Import {
        dialog: bool,
    },
    Export {
        dialog: bool,
    },
    /// New File, Open File, Close File (also Close Window on the last window).
    NewFile,
    OpenFile,
    CloseFile,
    /// Adjust Window.
    Adjust(Adjust),
    /// A data file step wants to write outside the database file's
    /// folder: [`Answer::Continue`] lets it ([`Host::file_confirm`]).
    ConfirmFile {
        path: String,
    },
    /// Insert from URL's request runs in the background
    /// ([`Host::background_requests`]): answer [`Answer::Continue`] once
    /// [`Engine::fetch_done`] (earlier blocks until it is), the step then
    /// takes the response.
    Fetch {
        url: String,
    },
    /// Close PDF / Print PDF: the PDF document a script built.
    /// `path` is where to save it (`None`: ask), `print` to print it.
    PdfDocument {
        path: Option<String>,
        data: Vec<u8>,
        print: bool,
    },
    /// Open Script Workspace, Open Settings, Open Find/Replace.
    Dialog(AppDialog),
}

/// FileMaker dialogs a script step opens that BaseCraft has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDialog {
    ScriptWorkspace,
    Preferences,
    FindReplace,
}

/// A file or folder seen by [`Host::file_stat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub folder: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjust {
    ResizeToFit,
    Maximize,
    Minimize,
    Restore,
    Hide,
}

/// Why a run stopped before its end.
#[derive(Debug, Clone, PartialEq)]
pub enum Suspend {
    /// Show Custom Dialog or a confirmation: answer with [`Answer::Dialog`].
    Dialog(Dialog),
    /// Pause/Resume Script, or a mode entered with "Pause": answer with
    /// [`Answer::Continue`] or [`Answer::Cancel`]. The duration (seconds)
    /// when the pause ends by itself.
    Pause(Option<f64>),
    /// The debugger stopped before a step (a breakpoint, or stepping):
    /// [`Run::current`] is the step; answer with [`Answer::Continue`]
    /// after choosing how to go on ([`Run::set_stepping`]), or
    /// [`Answer::Cancel`] to halt.
    Breakpoint,
    /// The application has to do something: see [`UiRequest`].
    Ui(UiRequest),
}

/// How the debugger goes on from a [`Suspend::Breakpoint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stepping {
    /// Run to the next breakpoint.
    #[default]
    Run,
    /// Step: the next step of this script or its callers.
    Over,
    /// Step Into: the next step, also inside a performed script.
    Into,
    /// Step Out: the next step of a calling script.
    Out,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Suspended(Suspend),
    Finished(Outcome),
}

/// The user's reply to a [`Suspend`].
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// First call of [`Run::resume`], nothing to answer.
    Start,
    Dialog(DialogAnswer),
    Continue,
    Cancel,
    /// A value for a [`UiRequest`] (index value, folder path).
    Text(String),
}

/// What a suspended step does with the answer.
#[derive(Debug, Clone)]
enum Pending {
    /// Show Custom Dialog: input fields and whether each button commits.
    Dialog {
        inputs: Vec<FieldRef>,
        commits: Vec<bool>,
    },
    /// Delete Record / Delete All Records confirmation (button 1 deletes).
    Delete {
        table: u32,
        records: Vec<usize>,
    },
    Pause,
    /// Stopped by the debugger before the current step.
    Break,
    /// A [`UiRequest`]: what to do with the answer.
    Ui(UiPending),
}

#[derive(Debug, Clone)]
enum UiPending {
    /// Nothing: Cancel is error 1.
    Done,
    /// Run the step again (Insert from URL, its response ready).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Rerun,
    /// Store the text in a variable (Get Folder Path).
    Variable(String),
    /// Insert the text into a field (Insert from Index).
    Insert(FieldRef, bool),
    /// The last window closed: the script stops.
    Halt,
    /// The user agreed to a write outside the file's folder: run the
    /// step again.
    Retry(String),
}

/// One script call on the run's stack.
struct Call {
    /// Script id (0 for a button's step).
    script: u32,
    steps: Vec<Step>,
    links: Links,
    pc: usize,
    frame: Frame,
    /// A script or a triggered script.
    kind: triggers::Kind,
    /// Pre triggers of the current step still to run (`None`: not
    /// looked at yet).
    pres: Option<Vec<(u16, Option<u64>)>>,
}

/// A script run in progress ([`Engine::start`]).
pub struct Run {
    stack: Vec<Call>,
    pending: Option<Pending>,
    result: Value,
    halted: bool,
    /// Error before anything ran (script missing).
    early: Option<u32>,
    /// Debugger: breakpoints (script id, step index); how it goes on and
    /// the stack depth when it last stopped; `resumed` lets the step it
    /// stopped on run.
    pub breakpoints: std::collections::BTreeSet<(u32, usize)>,
    stepping: Option<(Stepping, usize)>,
    resumed: bool,
    /// The window before the running step, for its post triggers.
    seen: Option<triggers::Seen>,
}

/// One level of a run's call stack ([`Run::stack`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    pub script: u32,
    pub name: String,
    /// Index of the step about to run (or running).
    pub step: usize,
}

impl Run {
    fn new(stack: Vec<Call>) -> Run {
        Run {
            stack,
            pending: None,
            result: Value::empty(),
            halted: false,
            early: None,
            breakpoints: Default::default(),
            stepping: None,
            resumed: false,
            seen: None,
        }
    }

    /// Runs until the end or the next suspension. `answer` replies to the
    /// previous [`Status::Suspended`] ([`Answer::Start`] the first time).
    pub fn resume(&mut self, e: &mut Engine, host: &mut dyn Host, answer: Answer) -> Status {
        if let Some(code) = self.early.take() {
            e.last_error = code;
        }
        if let Some(p) = self.pending.take() {
            e.answer(self, host, p, answer);
        }
        e.drive(self, host)
    }

    /// Runs to the end, answering dialogs with [`Host::dialog`] and
    /// continuing pauses.
    pub fn finish(mut self, e: &mut Engine, host: &mut dyn Host) -> Outcome {
        let mut answer = Answer::Start;
        loop {
            match self.resume(e, host, answer) {
                Status::Finished(o) => return o,
                Status::Suspended(Suspend::Dialog(d)) => answer = Answer::Dialog(host.dialog(&d)),
                Status::Suspended(Suspend::Pause(_) | Suspend::Breakpoint) => {
                    answer = Answer::Continue
                }
                Status::Suspended(Suspend::Ui(r)) => answer = host.ui(&r),
            }
        }
    }

    /// Whether the run is waiting for an answer.
    pub fn is_suspended(&self) -> bool {
        self.pending.is_some()
    }

    /// Debugger on: the run stops before its next step (the first one of
    /// a new run), then as [`Run::set_stepping`] says. Off: it ignores
    /// breakpoints and runs to the end.
    pub fn set_debug(&mut self, on: bool) {
        self.stepping = on.then_some((Stepping::Into, 0));
    }

    /// Whether the debugger is on for this run.
    pub fn debugging(&self) -> bool {
        self.stepping.is_some()
    }

    /// How to go on after a [`Suspend::Breakpoint`] (debugger on).
    pub fn set_stepping(&mut self, s: Stepping) {
        self.stepping = Some((s, self.stack.len()));
    }

    /// The call stack, outermost first.
    pub fn stack(&self) -> Vec<Level> {
        self.stack
            .iter()
            .map(|c| Level {
                script: c.script,
                name: c.frame.name.clone(),
                step: c.pc,
            })
            .collect()
    }

    /// (script id, step index) about to run.
    pub fn current(&self) -> Option<(u32, usize)> {
        self.stack.last().map(|c| (c.script, c.pc))
    }

    /// `$local` variables of the innermost call: (name, repetition, value).
    pub fn locals(&self) -> Vec<(String, usize, Value)> {
        let mut v: Vec<_> = self
            .stack
            .last()
            .map(|c| {
                c.frame
                    .locals
                    .iter()
                    .map(|((n, r), x)| (n.clone(), *r, x.clone()))
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        v
    }

    /// Whether the debugger stops before the step at the top of the stack.
    fn should_break(&self) -> bool {
        let (Some(c), Some((mode, d))) = (self.stack.last(), self.stepping) else {
            return false;
        };
        let depth = self.stack.len();
        self.breakpoints.contains(&(c.script, c.pc))
            || match mode {
                Stepping::Into => true,
                Stepping::Over => depth <= d,
                Stepping::Out => depth < d,
                Stepping::Run => false,
            }
    }
}

/// The trigger that started a script, for `Get ( Trigger… )`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriggerInfo {
    pub keystroke: String,
    pub modifier_keys: u32,
    pub current_panel: String,
    pub target_panel: String,
    pub external_event: String,
}

/// Result of a script run.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Exit Script result (`Get ( ScriptResult )` for the caller).
    pub result: Value,
    /// `Get ( LastError )` after the last step.
    pub last_error: u32,
    /// Stopped by Halt Script or by cancelling an error.
    pub halted: bool,
}

type Vars = HashMap<(String, usize), Value>;

pub struct Engine {
    pub scripts: Vec<Script>,
    /// `$$global` variables (lower-case names), kept between runs.
    pub globals: Vars,
    last_error: u32,
    last_choice: usize,
    error_capture: bool,
    allow_abort: bool,
    /// Record being edited and its values before the first change
    /// (`None` for a new record), for Revert Record.
    open: Option<(u32, usize, Option<engine::Record>)>,
    /// Trigger information of the run (`Get ( TriggerKeystroke )`…), set
    /// by the host before [`Engine::start`].
    pub trigger: TriggerInfo,
    /// `Get ( LastErrorLocation )`: script, step and line of the last error.
    last_error_location: String,
    /// Windows behind the host's current one (New Window, Select Window),
    /// the most recently active last.
    pub windows: Vec<Window>,
    /// Install OnTimer Script: script, parameter calculation (evaluated
    /// when the timer fires) and interval in seconds. The host runs it.
    pub timer: Option<Timer>,
    /// Set Layout Object Animation (`Get ( ScriptAnimationState )`).
    animation: bool,
    /// Set Error Logging: on, and the custom debug information.
    pub error_logging: Option<Option<Calc>>,
    /// Record committed or left last (Insert from Last Visited).
    last_visited: Option<(u32, usize)>,
    /// Values undone by Undo/Redo (table, record, values).
    redo: Option<(u32, usize, engine::Record)>,
    /// Text of the last Cut, Copy, Copy Record (also given to
    /// [`Host::set_clipboard`]); Paste uses it when the host has none.
    pub clipboard: Option<String>,
    /// Open Transaction: the data as it was before it.
    transaction: Option<Transaction>,
    /// Set Revert Transaction on Error (on by default and again when the
    /// run ends).
    revert_on_error: bool,
    /// Revert Transaction's custom error message (`Get ( LastErrorDetail )`).
    error_detail: String,
    /// `Get ( LastErrorDetail )` a step sets for itself (Insert from URL,
    /// Execute FileMaker Data API); taken when the step ends.
    step_detail: String,
    /// Open data files, the PDF being built, persistent data.
    pub more: datafiles::State,
    /// Insert from URL's request running in the background.
    fetch: net::Fetch,
    /// `open` is the app's record ([`HostInfo::record_open`]), and the
    /// record it is when that record is new.
    host_open: bool,
    host_new: Option<(u32, usize)>,
}

/// Install OnTimer Script.
#[derive(Debug, Clone, PartialEq)]
pub struct Timer {
    pub script: u32,
    pub parameter: Option<Calc>,
    pub interval: f64,
}

/// One script call.
struct Frame {
    name: String,
    locals: Vars,
    parameter: Value,
    result: Value,
    original_layout: u32,
}

/// An open transaction: the tables, found set and record before it, and
/// the call level of the script that opened it (0 until the step ends).
struct Transaction {
    tables: Vec<engine::Table>,
    found: Vec<usize>,
    current: usize,
    level: usize,
}

enum Flow {
    Next,
    /// Revert Transaction (the opening script's call level, the custom
    /// error code or 0): on after that script's Commit Transaction.
    Reverted(usize, u32),
    Jump(usize),
    Exit(Value),
    Halt,
    /// Perform Script: script index and parameter.
    Call(usize, Value),
    /// Wait for the user.
    Suspend(Suspend, Pending),
}

/// Control-flow links of a step list: `next` = next branch (If, Else If)
/// or the loop start (End Loop); `end` = End If / End Loop of a block;
/// `in_loop` = innermost Loop around a step.
struct Links {
    next: Vec<Option<usize>>,
    end: Vec<Option<usize>>,
    in_loop: Vec<Option<usize>>,
}

fn links(steps: &[Step]) -> Links {
    let n = steps.len();
    let mut l = Links {
        next: vec![None; n],
        end: vec![None; n],
        in_loop: vec![None; n],
    };
    // (Loop index or If branches, exits)
    let mut stack: Vec<(bool, Vec<usize>)> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        l.in_loop[i] = stack
            .iter()
            .rev()
            .find(|(is_loop, _)| *is_loop)
            .map(|(_, v)| v[0]);
        if !s.enabled {
            continue;
        }
        match s.id {
            id::IF => stack.push((false, vec![i])),
            id::ELSE_IF | id::ELSE => {
                if let Some((false, b)) = stack.last_mut() {
                    b.push(i);
                }
            }
            id::END_IF => {
                if let Some((false, b)) = stack.pop() {
                    for (k, &x) in b.iter().enumerate() {
                        l.next[x] = Some(b.get(k + 1).copied().unwrap_or(i));
                        l.end[x] = Some(i);
                    }
                }
            }
            id::LOOP => stack.push((true, vec![i])),
            id::EXIT_LOOP_IF => {
                if let Some((_, v)) = stack.iter_mut().rev().find(|(is_loop, _)| *is_loop) {
                    v.push(i);
                }
            }
            id::END_LOOP => {
                // Close any If left open inside the loop.
                while let Some((false, _)) = stack.last() {
                    stack.pop();
                }
                if let Some((true, v)) = stack.pop() {
                    l.next[i] = Some(v[0]);
                    for &x in &v {
                        l.end[x] = Some(i);
                    }
                }
            }
            _ => {}
        }
    }
    l
}

/// (table occurrence, base table) of a layout.
fn layout_table(db: &Database, layout: u32) -> Option<(u32, &engine::Table)> {
    let l = db.layouts.iter().find(|l| l.id == layout)?;
    let to = (l.table_occurrence & 0xFFF) as u32;
    Some((to, db.table_of(to)?))
}

/// Calculation environment of a step: the frame's locals, the engine's
/// globals and the `Get` flags only a script knows.
struct ScriptEnv<'a> {
    locals: &'a mut Vars,
    globals: &'a mut Vars,
    flags: HashMap<&'static str, Value>,
    seen: hostfns::Seen<'a>,
}

impl engine::Env for ScriptEnv<'_> {
    fn variable(&mut self, name: &str, repetition: usize) -> Value {
        let vars = if name.starts_with("$$") {
            &self.globals
        } else {
            &self.locals
        };
        vars.get(&(name.to_lowercase(), repetition))
            .cloned()
            .unwrap_or_else(Value::empty)
    }
    fn set_variable(&mut self, name: &str, repetition: usize, value: Value) {
        let vars = if name.starts_with("$$") {
            &mut self.globals
        } else {
            &mut self.locals
        };
        vars.insert((name.to_lowercase(), repetition), value);
    }
    fn get(&mut self, flag: &str) -> Option<Value> {
        self.flags.get(flag.to_lowercase().as_str()).cloned()
    }
    fn host_function(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, calc::EvalError>> {
        self.seen.call(name, args).map(Ok)
    }
}

fn num(n: usize) -> Value {
    Value::int(n as i64)
}

impl Engine {
    pub fn new(scripts: Vec<Script>) -> Engine {
        Engine {
            scripts,
            globals: HashMap::new(),
            last_error: 0,
            last_choice: 0,
            error_capture: false,
            allow_abort: true,
            open: None,
            trigger: TriggerInfo::default(),
            last_error_location: String::new(),
            windows: Vec::new(),
            timer: None,
            animation: true,
            error_logging: None,
            last_visited: None,
            redo: None,
            clipboard: None,
            transaction: None,
            revert_on_error: true,
            error_detail: String::new(),
            step_detail: String::new(),
            more: Default::default(),
            fetch: Default::default(),
            host_open: false,
            host_new: None,
        }
    }

    /// Takes the app's open record ([`HostInfo::record_open`]) as the one
    /// the run has open, or drops the one taken earlier once the app has
    /// closed it.
    fn adopt_open(&mut self, host: &mut dyn Host) {
        let open = host.window().host.record_open;
        if open == 0 {
            if std::mem::take(&mut self.host_open) {
                self.open = None;
                self.host_new = None;
            }
            return;
        }
        let (table, _, Some(r)) = Self::context(host) else {
            return;
        };
        if !matches!(&self.open, Some((t, x, _)) if (*t, *x) == (table, r)) {
            // Its values before the edit are the app's: Revert Record in
            // the script leaves them as they are.
            let now = host
                .db()
                .table(table)
                .and_then(|t| t.records.get(r))
                .cloned();
            self.open = Some((table, r, now));
            self.host_open = true;
        }
        self.host_new = (open == 1).then_some((table, r));
    }

    /// `Get ( LastError )` after the last step.
    pub fn last_error(&self) -> u32 {
        self.last_error
    }

    /// The scripts of a file.
    pub fn load(file: &fmp12::File) -> Result<Engine, fmp12::Error> {
        Ok(Engine::new(fmp12::scripts::scripts(file)?))
    }

    /// Starts script `id` with an optional parameter; [`Run::resume`] runs it.
    pub fn start(&mut self, host: &mut dyn Host, id: u32, parameter: Option<Value>) -> Run {
        self.adopt_open(host);
        self.error_capture = false;
        self.allow_abort = true;
        let mut run = Run::new(Vec::new());
        match self.scripts.iter().position(|s| s.id == id) {
            Some(i) => self.push(host, &mut run, i, parameter.unwrap_or_else(Value::empty)),
            None => run.early = Some(error::SCRIPT_MISSING),
        }
        run
    }

    /// Starts one step outside a script: a layout button's action
    /// (`LayoutObject::button`).
    pub fn start_step(&mut self, host: &mut dyn Host, step: &Step) -> Run {
        self.adopt_open(host);
        self.error_capture = false;
        let steps = vec![step.clone()];
        Run::new(vec![Call {
            script: 0,
            links: links(&steps),
            steps,
            pc: 0,
            frame: Frame {
                name: String::new(),
                locals: HashMap::new(),
                parameter: Value::empty(),
                result: Value::empty(),
                original_layout: host.window().layout,
            },
            kind: Default::default(),
            pres: None,
        }])
    }

    /// Runs script `id` to the end (Perform Script): dialogs are answered
    /// by [`Host::dialog`], pauses continue.
    pub fn perform(&mut self, host: &mut dyn Host, id: u32, parameter: Option<Value>) -> Outcome {
        self.start(host, id, parameter).finish(self, host)
    }

    /// Runs the script named `name` (case-insensitive).
    pub fn perform_named(
        &mut self,
        host: &mut dyn Host,
        name: &str,
        parameter: Option<Value>,
    ) -> Outcome {
        let id = self.script_named(name).unwrap_or(0);
        self.perform(host, id, parameter)
    }

    /// Id of the script named `name` (case-insensitive), folders excluded.
    pub fn script_named(&self, name: &str) -> Option<u32> {
        self.scripts
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name) && !s.is_folder())
            .map(|s| s.id)
    }

    /// Evaluates a calculation outside any script, in the window's context
    /// (Data Viewer): Get functions see the window, `$$` variables the
    /// engine's globals.
    pub fn evaluate(&mut self, host: &mut dyn Host, c: &Calc) -> Result<Value, u32> {
        let mut frame = Frame {
            name: String::new(),
            locals: HashMap::new(),
            parameter: Value::empty(),
            result: Value::empty(),
            original_layout: host.window().layout,
        };
        self.eval(host, &mut frame, c)
    }

    /// Runs one step outside a script to the end (button actions).
    pub fn run_step(&mut self, host: &mut dyn Host, step: &Step) -> Outcome {
        self.start_step(host, step).finish(self, host)
    }

    /// Pushes a call of script index `i`.
    fn push(&mut self, host: &mut dyn Host, run: &mut Run, i: usize, parameter: Value) {
        let script = &self.scripts[i];
        let steps = script.steps.clone();
        run.stack.push(Call {
            script: script.id,
            links: links(&steps),
            steps,
            pc: 0,
            frame: Frame {
                name: script.name.clone(),
                locals: HashMap::new(),
                parameter,
                result: Value::empty(),
                original_layout: host.window().layout,
            },
            kind: Default::default(),
            pres: None,
        });
    }

    /// Runs steps until the run ends or suspends.
    fn drive(&mut self, run: &mut Run, host: &mut dyn Host) -> Status {
        loop {
            // Post triggers of the step that just ended run before the next.
            if let Some(was) = run.seen.take()
                && self.post_triggers(run, host, was)
            {
                continue;
            }
            let Some(top) = run.stack.last_mut() else {
                // Halt Script or a cancel reverts an open transaction.
                if run.halted {
                    self.revert_transaction(host);
                }
                self.revert_on_error = true;
                return Status::Finished(Outcome {
                    result: std::mem::replace(&mut run.result, Value::empty()),
                    last_error: self.last_error,
                    halted: run.halted,
                });
            };
            let Some(s) = top.steps.get(top.pc) else {
                self.ret(run, host, Value::empty(), false);
                continue;
            };
            if !s.enabled {
                top.pc += 1;
                continue;
            }
            let first = top.pres.is_none();
            if !std::mem::take(&mut run.resumed) && first && run.should_break() {
                run.pending = Some(Pending::Break);
                return Status::Suspended(Suspend::Breakpoint);
            }
            // Pre triggers run first and may cancel the step.
            if self.pre_trigger(run, host) {
                continue;
            }
            run.seen = self.watch(run, host);
            let Some(top) = run.stack.last_mut() else {
                continue;
            };
            let pc = top.pc;
            let r = self.exec(host, &top.steps, &top.links, pc, &mut top.frame);
            if let Some(t) = self.transaction.as_mut().filter(|t| t.level == 0) {
                t.level = run.stack.len();
            }
            // An action the account's privileges refused (FileMaker's 200,
            // 201): the step's error.
            let r = match (r, host.db().last_denied()) {
                (Ok(Flow::Next) | Err(_), Some(code)) => Err(code),
                (r, _) => r,
            };
            match r {
                Ok(Flow::Call(i, p)) => self.push(host, run, i, p),
                Ok(Flow::Suspend(s, p)) => {
                    run.pending = Some(p);
                    return Status::Suspended(s);
                }
                Ok(Flow::Reverted(l, e)) => self.complete(run, host, Flow::Reverted(l, e), e),
                Ok(flow) => self.complete(run, host, flow, 0),
                Err(code) => self.complete(run, host, Flow::Next, code),
            }
        }
    }

    /// Ends the current step of the top call with `flow` and error `err`.
    fn complete(&mut self, run: &mut Run, host: &mut dyn Host, flow: Flow, err: u32) {
        let Some(top) = run.stack.last_mut() else {
            return;
        };
        top.pres = None;
        let Some(s) = top.steps.get(top.pc) else {
            return;
        };
        // FileMaker's Get ( LastErrorLocation ), and Get ( LastErrorDetail )
        // of a transaction that fails or reverts: script, step, line.
        let location = format!(
            "{}\r{}\r{}",
            top.frame.name,
            s.name().unwrap_or_default(),
            top.pc + 1
        );
        if s.id != id::COMMENT {
            self.last_error = err;
            let detail = std::mem::take(&mut self.step_detail);
            match flow {
                // Revert Transaction's custom message, else where it reverted.
                Flow::Reverted(..) if self.error_detail.is_empty() => {
                    self.error_detail = location.clone()
                }
                Flow::Reverted(..) => {}
                _ => self.error_detail = detail,
            }
        }
        if err != 0 {
            self.last_error_location = location;
            if !self.error_capture && !host.error(s, err) {
                Self::halt(run);
                return;
            }
            // Validation and privilege errors revert an open transaction.
            let reverts = matches!(err, 200..=299 | 500..=511);
            if reverts
                && self.revert_on_error
                && let Some(level) = self.revert_transaction(host)
            {
                self.error_detail = self.last_error_location.clone();
                return Self::after_revert(run, level);
            }
        }
        match flow {
            Flow::Reverted(level, _) => Self::after_revert(run, level),
            Flow::Next => top.pc += 1,
            Flow::Jump(j) => top.pc = j,
            Flow::Exit(v) => self.ret(run, host, v, false),
            Flow::Halt => Self::halt(run),
            Flow::Call(..) | Flow::Suspend(..) => top.pc += 1,
        }
    }

    /// Puts the data back as it was at Open Transaction; the opening
    /// script's call level, `None` when no transaction is open.
    fn revert_transaction(&mut self, host: &mut dyn Host) -> Option<usize> {
        let t = self.transaction.take()?;
        self.open = None;
        host.db().tables = t.tables;
        let w = host.window();
        w.found = t.found;
        w.current = t.current;
        Some(t.level)
    }

    /// After a revert: back in the script that opened the transaction
    /// (call `level`), on after its Commit Transaction.
    fn after_revert(run: &mut Run, level: usize) {
        run.stack.truncate(level.max(1));
        let Some(top) = run.stack.last_mut() else {
            return;
        };
        let from = top.pc + 1;
        top.pc = top.steps[from.min(top.steps.len())..]
            .iter()
            .position(|s| s.id == steps::sid::COMMIT_TRANSACTION)
            .map_or(top.steps.len(), |i| from + i + 1);
    }

    /// Halt Script (or a cancelled error): every level stops.
    fn halt(run: &mut Run) {
        run.stack.clear();
        run.result = Value::empty();
        run.halted = true;
    }

    /// Returns from the top call with `result`.
    fn ret(&mut self, run: &mut Run, host: &mut dyn Host, result: Value, halted: bool) {
        let kind = run.stack.pop().map(|c| c.kind).unwrap_or_default();
        if run.stack.is_empty() {
            run.result = result;
            run.halted = halted;
            return;
        }
        if kind != triggers::Kind::Script {
            return self.trigger_returned(run, host, kind, &result);
        }
        if let Some(parent) = run.stack.last_mut() {
            parent.frame.result = result;
        }
        // The Perform Script step's own error is the sub-script's last error.
        let err = self.last_error;
        self.complete(run, host, Flow::Next, err);
    }

    /// Applies the user's answer to a suspended step, then ends the step.
    fn answer(&mut self, run: &mut Run, host: &mut dyn Host, p: Pending, a: Answer) {
        match (p, a) {
            (Pending::Pause, Answer::Cancel) => {
                self.last_error = error::USER_CANCELED;
                Self::halt(run);
            }
            (Pending::Pause, _) => self.complete(run, host, Flow::Next, 0),
            (Pending::Break, Answer::Cancel) => Self::halt(run),
            (Pending::Break, _) => run.resumed = true,
            (Pending::Ui(_), Answer::Cancel) => {
                // A cancelled background request: its response is dropped.
                self.fetch = Default::default();
                self.complete(run, host, Flow::Next, error::USER_CANCELED)
            }
            (Pending::Ui(UiPending::Halt), _) => Self::halt(run),
            (Pending::Ui(UiPending::Rerun), _) => run.resumed = true,
            (Pending::Ui(UiPending::Retry(path)), _) => {
                host.file_confirm(&path);
                run.resumed = true;
            }
            (Pending::Ui(UiPending::Variable(name)), Answer::Text(t)) => {
                let key = (name.to_lowercase(), 1);
                if name.starts_with("$$") {
                    self.globals.insert(key, Value::text(t));
                } else if let Some(top) = run.stack.last_mut() {
                    top.frame.locals.insert(key, Value::text(t));
                }
                self.complete(run, host, Flow::Next, 0);
            }
            (Pending::Ui(UiPending::Insert(f, select)), Answer::Text(t)) => {
                let err = self.insert(host, f, &t, select).err().unwrap_or(0);
                self.complete(run, host, Flow::Next, err);
            }
            (Pending::Ui(UiPending::Variable(_) | UiPending::Insert(..)), _) => {
                self.complete(run, host, Flow::Next, error::USER_CANCELED)
            }
            (Pending::Ui(UiPending::Done), _) => self.complete(run, host, Flow::Next, 0),
            (Pending::Dialog { inputs, commits }, a) => {
                let a = match a {
                    Answer::Dialog(d) => d,
                    _ => DialogAnswer {
                        button: 1,
                        inputs: Vec::new(),
                    },
                };
                self.last_choice = a.button;
                let mut err = 0;
                if commits.get(a.button.wrapping_sub(1)) == Some(&true) {
                    for (f, v) in inputs.iter().zip(a.inputs) {
                        if let Err(e) = self.set_field(host, *f, Value::text(v)) {
                            err = e;
                        }
                    }
                }
                self.complete(run, host, Flow::Next, err);
            }
            (Pending::Delete { table, records }, a) => {
                let yes = match a {
                    Answer::Dialog(d) => d.button == 1,
                    Answer::Cancel => false,
                    _ => true,
                };
                if yes {
                    delete_records(host, table, &records);
                    self.complete(run, host, Flow::Next, 0);
                } else {
                    self.complete(run, host, Flow::Next, error::USER_CANCELED);
                }
            }
        }
    }

    /// Context of the current window: (table, occurrence, record).
    fn context(host: &mut dyn Host) -> (u32, u32, Option<usize>) {
        let w = host.window().clone();
        let db = host.db();
        match layout_table(db, w.layout) {
            Some((to, t)) => (t.id, to, w.record().filter(|_| w.mode != Mode::Find)),
            None => (0, 0, None),
        }
    }

    fn eval(&mut self, host: &mut dyn Host, frame: &mut Frame, c: &Calc) -> Result<Value, u32> {
        let (table, to, record) = Self::context(host);
        let w = host.window().clone();
        let db = host.db();
        let layout = db.layouts.iter().find(|l| l.id == w.layout);
        let total = db.table(table).map_or(0, |t| t.records.len());
        let mut flags: HashMap<&'static str, Value> = HashMap::new();
        flags.insert("scriptparameter", frame.parameter.clone());
        flags.insert("scriptresult", frame.result.clone());
        flags.insert("scriptname", Value::text(&frame.name));
        flags.insert("lasterror", num(self.last_error as usize));
        flags.insert("lasterrorlocation", Value::text(&self.last_error_location));
        flags.insert("lasterrordetail", Value::text(&self.error_detail));
        flags.insert(
            "transactionopenstate",
            num(self.transaction.is_some() as usize),
        );
        flags.insert(
            "reverttransactiononerrorstate",
            num(self.revert_on_error as usize),
        );
        flags.insert("scriptanimationstate", num(self.animation as usize));
        flags.insert("windowname", Value::text(&w.name));
        flags.insert("windowtop", num(w.top.max(0.0) as usize));
        flags.insert("windowleft", num(w.left.max(0.0) as usize));
        flags.insert("windowwidth", num(w.width.max(0.0) as usize));
        flags.insert("windowheight", num(w.height.max(0.0) as usize));
        flags.insert("windowvisible", num(!w.hidden as usize));
        flags.insert("windowzoomlevel", num(w.zoom as usize));
        flags.insert(
            "layoutviewstate",
            num(match w.view {
                View::Form => 0,
                View::List => 1,
                View::Table => 2,
            }),
        );
        flags.insert("statusareastate", num(w.toolbar as usize));
        flags.insert("activeportalrownumber", num(w.portal_row));
        flags.insert("activeselectionstart", num(w.selection.0 + 1));
        flags.insert("activeselectionsize", num(w.selection.1));
        // No active field: empty (FileMaker 26), repetition 0.
        for f in [
            "activefieldname",
            "activefieldtablename",
            "activefieldcontents",
        ] {
            flags.insert(f, Value::empty());
        }
        flags.insert(
            "activerepetitionnumber",
            num(w.field.map_or(0, |f| f.repetition.max(1) as usize)),
        );
        // The Go to Object name, else the name of the active field's object.
        let object = match (&w.object, w.field) {
            (o, _) if !o.is_empty() => o.clone(),
            (_, Some(f)) => layout
                .and_then(|l| {
                    l.objects.iter().find(|o| {
                        o.field.is_some_and(|(t, x, _)| {
                            (t & 0xFFF) as u32 == f.to && x as u32 == f.field
                        })
                    })
                })
                .map(|o| o.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        flags.insert("activelayoutobjectname", Value::text(object));
        if let Some(f) = w.field {
            let fname = db
                .table_of(f.to)
                .and_then(|t| t.field(f.field))
                .map(|x| x.name.clone())
                .unwrap_or_default();
            flags.insert("activefieldname", Value::text(fname));
            let tname = db
                .occurrences
                .iter()
                .find(|o| o.id == f.to)
                .map(|o| o.name.clone())
                .unwrap_or_default();
            flags.insert("activefieldtablename", Value::text(tname));
            let rep = f.repetition.max(1) as u32;
            if let (Some(t), Some(r)) = (db.table_of(f.to), record) {
                // A related field: its first related record.
                let own = (t.id == table).then_some(r);
                let at = own.or_else(|| db.related(to, r, f.to).first().copied());
                if let Some(at) = at {
                    flags.insert("activefieldcontents", db.value_rep(t.id, at, f.field, rep));
                }
            }
        }
        flags.insert(
            "activerecordnumber",
            num(if record.is_some() { w.current + 1 } else { 0 }),
        );
        flags.insert("textrulervisible", num(w.ruler as usize));
        flags.insert("menubarstate", num(w.menubar as usize));
        flags.insert(
            "requestomitstate",
            num(
                (w.mode == Mode::Find && w.requests.get(w.current_request).is_some_and(|r| r.omit))
                    as usize,
            ),
        );
        flags.insert("modifiedfields", Value::text(self.modified_fields(db)));
        let h = &w.host;
        flags.insert("quickfindtext", Value::text(&h.quick_find));
        if !h.file_name.is_empty() {
            flags.insert("filename", Value::text(&h.file_name));
            flags.insert("filepath", Value::text(&h.file_path));
        }
        let pts = |x: f64| Value::int(x.round() as i64);
        let pairs = [
            ("filesize", h.file_size.map(|s| Value::int(s as i64))),
            ("windowcontentwidth", h.content.map(|c| pts(c.0))),
            ("windowcontentheight", h.content.map(|c| pts(c.1))),
            ("screenwidth", h.screen.map(|c| pts(c.0))),
            ("screenheight", h.screen.map(|c| pts(c.1))),
            (
                "screenscalefactor",
                h.scale
                    .and_then(|s| calc::num::from_f64(s, 2))
                    .map(Value::Number),
            ),
            (
                "activemodifierkeys",
                h.modifier_keys.map(|k| Value::int(k as i64)),
            ),
        ];
        for (k, v) in pairs {
            if let Some(v) = v {
                flags.insert(k, v);
            }
        }
        flags.insert("triggerkeystroke", Value::text(&self.trigger.keystroke));
        flags.insert(
            "triggermodifierkeys",
            num(self.trigger.modifier_keys as usize),
        );
        flags.insert(
            "triggercurrentpanel",
            Value::text(&self.trigger.current_panel),
        );
        flags.insert(
            "triggertargetpanel",
            Value::text(&self.trigger.target_panel),
        );
        flags.insert(
            "triggerexternalevent",
            Value::text(&self.trigger.external_event),
        );
        flags.insert("lastmessagechoice", num(self.last_choice));
        flags.insert("errorcapturestate", num(self.error_capture as usize));
        flags.insert("allowabortstate", num(self.allow_abort as usize));
        flags.insert("foundcount", num(w.found.len()));
        flags.insert("totalrecordcount", num(total));
        flags.insert(
            "recordnumber",
            num(if w.found.is_empty() { 0 } else { w.current + 1 }),
        );
        flags.insert(
            "recordid",
            num(record
                .and_then(|r| db.table(table)?.records.get(r).map(|r| r.id as usize))
                .unwrap_or(0)),
        );
        flags.insert("requestcount", num(w.requests.len()));
        flags.insert(
            "windowmode",
            num(match w.mode {
                Mode::Browse => 0,
                Mode::Find => 1,
                Mode::Preview => 2,
            }),
        );
        if let Some(l) = layout {
            flags.insert("layoutname", Value::text(&l.name));
            let n = db.layouts.iter().position(|x| x.id == l.id).unwrap_or(0) + 1;
            flags.insert("layoutnumber", num(n));
            // Records via this layout: 0 none, 1 view only, 2 modifiable.
            let access = db.layout_records(l.id).saturating_sub(1);
            flags.insert("layoutaccess", num(access as usize));
        }
        if let Some(t) = db.table(table) {
            flags.insert("layouttablename", Value::text(&t.name));
        }
        flags.insert("sortstate", num(w.sorted as usize));
        // The record a script opened, or the app's one it was handed.
        let open = self
            .open
            .as_ref()
            .filter(|o| (o.0, Some(o.1)) == (table, record));
        let state = match open {
            Some((t, r, old)) => 1 + (old.is_some() && self.host_new != Some((*t, *r))) as usize,
            None => 0,
        };
        flags.insert("recordopenstate", num(state));
        flags.insert("recordopencount", num(self.open.is_some() as usize));
        let db: &Database = db;
        let mut env = ScriptEnv {
            locals: &mut frame.locals,
            globals: &mut self.globals,
            flags,
            seen: hostfns::Seen {
                db,
                window: &w,
                table,
                to,
                record,
                persistent: &self.more.persistent,
            },
        };
        // The window's found set and sort: summary fields, GetSummary,
        // GetNthRecord.
        let scope = engine::Scope {
            found: Some(engine::FoundSet {
                table,
                records: &w.found,
                sort: &w.sort,
            }),
            self_value: None,
        };
        db.eval_tokens_scoped(&c.0, table, to, record, &scope, Some(&mut env))
            .map_err(|e| e.code)
    }

    fn eval_opt(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        c: &Option<Calc>,
    ) -> Result<Option<Value>, u32> {
        match c {
            Some(c) => self.eval(host, frame, c).map(Some),
            None => Ok(None),
        }
    }

    /// Commits the record being edited (FileMaker does on navigation),
    /// after checking its fields' validation options: the error code of
    /// the first failing rule (509 not empty…) leaves the record open.
    fn commit(&mut self, host: &mut dyn Host) -> Result<(), u32> {
        self.commit_with(host, false)
    }

    /// `Get ( ModifiedFields )`: the fields of the record the script is
    /// editing whose value changed since it was opened (for a new record,
    /// those with a value), one per line in the table's field order.
    // ponytail: only edits made by scripts open a record here; the app's
    // own typing is not seen until it hands its open record to the engine.
    fn modified_fields(&self, db: &Database) -> String {
        let Some((table, r, old)) = &self.open else {
            return String::new();
        };
        let Some(t) = db.table(*table) else {
            return String::new();
        };
        let Some(now) = t.records.get(*r) else {
            return String::new();
        };
        let values = |rec: Option<&engine::Record>, f: u32| -> Vec<(u32, String)> {
            rec.map(|rec| {
                rec.values
                    .iter()
                    .filter(|(k, v)| engine::split_rep_key(**k).0 == f && !v.is_empty())
                    .map(|(k, v)| (*k, v.clone()))
                    .collect()
            })
            .unwrap_or_default()
        };
        t.fields
            .iter()
            .filter(|f| values(Some(now), f.id) != values(old.as_ref(), f.id))
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join("\r")
    }

    /// Commit Records; `skip` = "Skip data entry validation" (fields
    /// validated "always" are still checked).
    fn commit_with(&mut self, host: &mut dyn Host, skip: bool) -> Result<(), u32> {
        let Some((table, r, ref old)) = self.open else {
            if let (t, _, Some(r)) = Self::context(host) {
                self.last_visited = Some((t, r));
            }
            return Ok(());
        };
        if let Some(e) = host
            .db()
            .validate_script(table, r)
            .into_iter()
            .find(|e| !skip || e.always)
        {
            return Err(e.rule.code());
        }
        // Counted when an existing record changed (`old` is `None` for a
        // record this edit created).
        let now = host.db().table(table).and_then(|t| t.records.get(r));
        if let Some(o) = old
            && now.is_some_and(|n| n.values != o.values)
        {
            host.db().commit_record(table, r);
        }
        self.open = None;
        self.redo = None;
        self.last_visited = Some((table, r));
        Ok(())
    }

    fn exec(
        &mut self,
        host: &mut dyn Host,
        steps: &[Step],
        l: &Links,
        pc: usize,
        frame: &mut Frame,
    ) -> Result<Flow, u32> {
        let s = &steps[pc];
        let after = |i: Option<usize>| Flow::Jump(i.map_or(usize::MAX, |e| e + 1));
        match (&s.params, s.id) {
            (_, id::COMMENT) => {}
            // Control flow.
            (StepParams::Condition(c), id::IF) => {
                if !self.truth(host, frame, c)? {
                    let mut k = l.next[pc];
                    while let Some(b) = k {
                        match (&steps[b].params, steps[b].id) {
                            (StepParams::Condition(c), id::ELSE_IF) => {
                                if self.truth(host, frame, c)? {
                                    return Ok(Flow::Jump(b + 1));
                                }
                                k = l.next[b];
                            }
                            _ => return Ok(Flow::Jump(b + 1)),
                        }
                    }
                    return Ok(Flow::Jump(usize::MAX));
                }
            }
            // Reached at the end of a branch that ran: skip to End If.
            (_, id::ELSE_IF | id::ELSE) => return Ok(after(l.end[pc])),
            (_, id::END_IF | id::LOOP) => {}
            (_, id::END_LOOP) => return Ok(Flow::Jump(l.next[pc].map_or(pc + 1, |s| s + 1))),
            (StepParams::Condition(c), id::EXIT_LOOP_IF) => {
                if self.truth(host, frame, c)? {
                    return Ok(after(l.end[pc]));
                }
            }
            (StepParams::PerformScript { script, parameter }, _) => {
                let p = self
                    .eval_opt(host, frame, parameter)?
                    .unwrap_or_else(Value::empty);
                // By name: the name's calculation at [130] (FORMAT.md §8).
                let script = match (script, s.raw_params.calc(2)) {
                    (None, Some(c)) => {
                        let name = self.eval(host, frame, &c)?.to_text();
                        self.script_named(&name)
                    }
                    (s, _) => *s,
                };
                let i = script
                    .and_then(|id| self.scripts.iter().position(|s| s.id == id))
                    .ok_or(error::SCRIPT_MISSING)?;
                // A script the account has no access to does not run.
                if host.db().script_access(self.scripts[i].id) == fmp12::security::NO_ACCESS {
                    return Err(engine::PRIVILEGES);
                }
                return Ok(Flow::Call(i, p));
            }
            (StepParams::ExitScript(c), _) => {
                let v = self.eval_opt(host, frame, c)?.unwrap_or_else(Value::empty);
                return Ok(Flow::Exit(v));
            }
            (_, id::HALT_SCRIPT) => return Ok(Flow::Halt),
            (StepParams::Switch(on), id::SET_ERROR_CAPTURE) => self.error_capture = *on,
            (StepParams::Switch(on), id::ALLOW_USER_ABORT) => self.allow_abort = *on,
            (
                StepParams::SetVariable {
                    name,
                    value,
                    repetition,
                },
                _,
            ) => {
                let v = self
                    .eval_opt(host, frame, value)?
                    .unwrap_or_else(Value::empty);
                let rep = self.repetition(host, frame, repetition)?;
                let key = (name.to_lowercase(), rep);
                if name.starts_with("$$") {
                    self.globals.insert(key, v);
                } else {
                    frame.locals.insert(key, v);
                }
            }
            (
                StepParams::SetField {
                    field,
                    value,
                    repetition,
                },
                _,
            ) => {
                let v = self
                    .eval_opt(host, frame, value)?
                    .unwrap_or_else(Value::empty);
                let rep = self.repetition(host, frame, repetition)?;
                let mut f = field.ok_or(error::FIELD_MISSING)?;
                if repetition.is_some() {
                    f.repetition = rep as u16;
                }
                self.set_field(host, f, v)?;
            }
            (
                StepParams::InsertText {
                    target,
                    text,
                    select,
                },
                _,
            ) => match target {
                Target::Variable(name) => {
                    let key = (name.to_lowercase(), 1);
                    let v = Value::text(text);
                    if name.starts_with("$$") {
                        self.globals.insert(key, v);
                    } else {
                        frame.locals.insert(key, v);
                    }
                }
                Target::Field(f) => self.insert(host, *f, text, *select)?,
                Target::None => return Err(error::FIELD_MISSING),
            },
            (
                StepParams::Other,
                id::INSERT_CALCULATED_RESULT
                | id::INSERT_CURRENT_DATE
                | id::INSERT_CURRENT_TIME
                | id::INSERT_CURRENT_USER_NAME
                | id::PASTE,
            ) => {
                let v = match s.id {
                    id::INSERT_CALCULATED_RESULT => {
                        let c = calcs(s).into_iter().next().ok_or(error::UNAVAILABLE)?;
                        self.eval(host, frame, &c)?
                    }
                    id::INSERT_CURRENT_DATE => now(host, "CurrentDate"),
                    id::INSERT_CURRENT_TIME => now(host, "CurrentTime"),
                    id::INSERT_CURRENT_USER_NAME => now(host, "UserName"),
                    _ => Value::text(
                        host.clipboard()
                            .or_else(|| self.clipboard.clone())
                            .ok_or(error::UNAVAILABLE)?,
                    ),
                };
                let select = s.options & fmp12::scripts::opt::SELECT != 0;
                let active = host.window().field;
                match (s.raw_params.text(32), s.inline_field().or(active)) {
                    (Some(name), _) => {
                        let key = (name.to_lowercase(), 1);
                        if name.starts_with("$$") {
                            self.globals.insert(key, v);
                        } else {
                            frame.locals.insert(key, v);
                        }
                    }
                    (None, Some(f)) => {
                        let text = engine::stored_text(&v, &host.db().locale);
                        self.insert(host, f, &text, select)?;
                    }
                    (None, None) => return Err(error::FIELD_MISSING),
                }
            }
            (StepParams::Other, id::SET_FIELD_BY_NAME) => {
                // ponytail: target name at position 0, value at 1, by
                // analogy with Set Variable; not checked in FileMaker.
                let mut c = calcs(s).into_iter();
                let (Some(name), value) = (c.next(), c.next()) else {
                    return Err(error::FIELD_MISSING);
                };
                let name = self.eval(host, frame, &name)?.to_text();
                let v = match value {
                    Some(v) => self.eval(host, frame, &v)?,
                    None => Value::empty(),
                };
                let f = field_named(host.db(), &name).ok_or(error::FIELD_MISSING)?;
                self.set_field(host, f, v)?;
            }
            (StepParams::Other, id::REPLACE_FIELD_CONTENTS) => {
                // Parameters (FORMAT.md §8): [129] k3 = with (2 serial
                // numbers, 3 calculation, otherwise current contents),
                // [128] calculation, [130] increment, [131] initial value,
                // [132] update Entry Options, [133] use Entry Options,
                // [134] perform auto-enter.
                self.commit(host)?;
                let f = s.inline_field().ok_or(error::FIELD_MISSING)?;
                let (table, _, rec) = Self::context(host);
                let p = &s.raw_params;
                let auto_enter = p.byte(6).is_none_or(|b| b != 0);
                let w = host.window().clone();
                let db = host.db();
                if db.table_of(f.to).map(|t| t.id) != Some(table) {
                    return Err(error::FIELD_MISSING);
                }
                let field = db.table(table).and_then(|t| t.field(f.field)).cloned();
                if p.byte(1) == Some(3) {
                    // Evaluated per record, with the script's variables.
                    let c = calcs(s).into_iter().next().ok_or(error::UNAVAILABLE)?;
                    for i in 0..w.found.len() {
                        host.window().current = i;
                        let v = match self.eval(host, frame, &c) {
                            Ok(v) => v,
                            Err(e) => {
                                host.window().current = w.current;
                                return Err(e);
                            }
                        };
                        let db = host.db();
                        let text = engine::stored_text(&v, &db.locale);
                        let with = ReplaceWith::Value(text);
                        let one = [w.found[i]];
                        let _ = replace_field_contents(db, table, &one, f.field, &with, auto_enter);
                    }
                    host.window().current = w.current;
                    return Ok(Flow::Next);
                }
                let with = if p.byte(1) == Some(2) {
                    let (initial, increment) = match &field {
                        // "Use Entry Options": the field's next serial and increment.
                        Some(fd) if p.byte(5) == Some(1) => {
                            (fd.def.serial_next.clone(), fd.def.serial_increment() as i64)
                        }
                        _ => (
                            p.text(3).unwrap_or_else(|| "1".into()),
                            p.byte(2).map_or(1, i64::from),
                        ),
                    };
                    ReplaceWith::Serial {
                        initial,
                        increment,
                        update_entry: p.byte(4) == Some(1),
                    }
                } else {
                    let r = rec.ok_or(error::RECORD_MISSING)?;
                    let v = db.value(table, r, f.field);
                    ReplaceWith::Value(engine::stored_text(&v, &db.locale))
                };
                replace_field_contents(host.db(), table, &w.found, f.field, &with, auto_enter)
                    .map_err(|_| error::FIELD_MISSING)?;
            }
            (StepParams::Other, id::PERFORM_FIND_REPLACE) => {
                // [128] k3 = operation (0 Find Next, 1 Replace & Find,
                // 2 Replace, 3 Replace All) and find calculation; [129]
                // k3 = direction (0 forward, 1 backward, 2 all) and
                // replace calculation; [130] current record, [131]
                // current field, [132] match case, [133] whole word.
                // ponytail: a script has no active field, so "current
                // field" searches every field of the layout.
                self.commit(host)?;
                let p = &s.raw_params;
                let find = match p.calc(0) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                let replace = match p.calc(1) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                let q = FindReplace {
                    find,
                    replace,
                    match_case: p.byte(4) == Some(1),
                    whole_word: p.byte(5) == Some(1),
                };
                let dir = match p.byte(1) {
                    Some(1) => Direction::Backward,
                    Some(2) => Direction::All,
                    _ => Direction::Forward,
                };
                let current_only = p.byte(2) == Some(1);
                let (table, _, _) = Self::context(host);
                let w = host.window().clone();
                let records: Vec<usize> = if current_only {
                    w.found.get(w.current).copied().into_iter().collect()
                } else {
                    w.found.clone()
                };
                let db = host.db();
                let mut fields: Vec<u32> = db
                    .layouts
                    .iter()
                    .find(|l| l.id == w.layout)
                    .map(|l| {
                        l.objects
                            .iter()
                            .filter(|o| o.kind == 1)
                            .filter_map(|o| o.field.map(|x| x.1 as u32))
                            .collect()
                    })
                    .unwrap_or_default();
                // A layout without fields (or none): every field of the table.
                if fields.is_empty() {
                    fields = db
                        .table(table)
                        .map(|t| t.fields.iter().map(|f| f.id).collect())
                        .unwrap_or_default();
                }
                let op = p.byte(0).unwrap_or(0);
                if op == 3 {
                    engine::replace::replace_all(db, table, &records, &fields, &q);
                } else {
                    let start = (w.current > 0 && !current_only).then(|| Hit {
                        pos: w.current - 1,
                        field: u32::MAX,
                        range: usize::MAX..usize::MAX,
                    });
                    let hit = find_next(db, table, &records, &fields, &q, start.as_ref(), dir)
                        .ok_or(error::RECORD_MISSING)?;
                    let mut go = hit.pos;
                    if op != 0 {
                        engine::replace::replace_hit(db, table, &records, &q, &hit);
                    }
                    if op == 1 {
                        let after = Hit {
                            range: hit.range.start..hit.range.start + q.replace.len(),
                            ..hit.clone()
                        };
                        if let Some(h) =
                            find_next(db, table, &records, &fields, &q, Some(&after), dir)
                        {
                            go = h.pos;
                        }
                    }
                    if !current_only {
                        host.window().current = go;
                    }
                }
            }
            (StepParams::Other, id::RELOOKUP) => {
                self.commit(host)?;
                let f = s.inline_field().ok_or(error::FIELD_MISSING)?;
                let (table, _, _) = Self::context(host);
                let found = host.window().found.clone();
                let db = host.db();
                if db.table_of(f.to).map(|t| t.id) != Some(table) {
                    return Err(error::FIELD_MISSING);
                }
                db.relookup(table, &found, f.field);
            }
            (StepParams::GoToLayout { dest, .. }, _) => {
                let target = match dest {
                    LayoutDest::Original => frame.original_layout,
                    LayoutDest::Layout(id) => *id,
                    LayoutDest::NameByCalc(c) => {
                        let name = self.eval(host, frame, c)?.to_text();
                        let db = host.db();
                        db.layouts
                            .iter()
                            .find(|l| l.name.eq_ignore_ascii_case(&name))
                            .map_or(0, |l| l.id)
                    }
                    LayoutDest::NumberByCalc(c) => {
                        let n = self
                            .eval(host, frame, c)?
                            .to_text()
                            .parse::<usize>()
                            .unwrap_or(0);
                        let db = host.db();
                        n.checked_sub(1)
                            .and_then(|i| db.layouts.get(i))
                            .map_or(0, |l| l.id)
                    }
                };
                self.go_to_layout(host, target)?;
            }
            (
                StepParams::GoToRecord {
                    dest,
                    exit_after_last,
                    ..
                },
                _,
            ) => {
                self.commit(host)?;
                let n = self.eval_dest(host, frame, dest)?;
                let w = host.window();
                let len = if w.mode == Mode::Find {
                    w.requests.len()
                } else {
                    w.found.len()
                };
                let cur = if w.mode == Mode::Find {
                    w.current_request
                } else {
                    w.current
                };
                if len == 0 {
                    return Err(error::RECORD_MISSING);
                }
                let to = match n {
                    Dest::First => 0,
                    Dest::Last => len - 1,
                    Dest::Previous if cur == 0 => {
                        return if *exit_after_last {
                            Ok(after(l.in_loop[pc].and_then(|s| l.end[s])))
                        } else {
                            Err(error::RECORD_MISSING)
                        };
                    }
                    Dest::Previous => cur - 1,
                    Dest::Next if cur + 1 >= len => {
                        return if *exit_after_last {
                            Ok(after(l.in_loop[pc].and_then(|s| l.end[s])))
                        } else {
                            Err(error::RECORD_MISSING)
                        };
                    }
                    Dest::Next => cur + 1,
                    Dest::Nth(k) if k >= 1 && k <= len => k - 1,
                    Dest::Nth(_) => return Err(error::RECORD_MISSING),
                };
                if w.mode == Mode::Find {
                    w.current_request = to;
                } else {
                    w.current = to;
                }
            }
            (StepParams::GoToField { field, select }, _) => {
                if !host.go_to_field(*field, *select) {
                    return Err(error::FIELD_MISSING);
                }
                host.window().field = *field;
                host.window().object.clear();
                let len = match field {
                    Some(f) if *select => {
                        self.field_value(host, *f).map_or(0, |v| v.chars().count())
                    }
                    _ => 0,
                };
                host.window().selection = (0, len);
            }
            (StepParams::GoToObject { name, repetition }, _) => {
                let n = self
                    .eval_opt(host, frame, name)?
                    .map(|v| v.to_text())
                    .unwrap_or_default();
                let r = self.repetition(host, frame, repetition)?;
                // No object of that name on the layout: 116 (FileMaker 26).
                let layout = host.window().layout;
                let named = host
                    .db()
                    .layouts
                    .iter()
                    .find(|l| l.id == layout)
                    .and_then(|l| l.objects.iter().find(|o| o.name == n))
                    .map(|o| o.field);
                let Some(field) = named else {
                    return Err(error::OBJECT_MISSING);
                };
                if !host.go_to_object(&n, r as u32) {
                    return Err(error::OBJECT_MISSING);
                }
                host.window().object = n;
                // A field object becomes the active field.
                if let Some((t, f, _)) = field {
                    host.window().field = Some(FieldRef {
                        to: (t & 0xFFF) as u32,
                        field: f as u32,
                        repetition: r as u16,
                    });
                }
            }
            (
                StepParams::ShowCustomDialog {
                    title,
                    message,
                    buttons,
                    input,
                },
                _,
            ) => {
                let mut d = Dialog {
                    title: self
                        .eval_opt(host, frame, title)?
                        .map(|v| v.to_text())
                        .unwrap_or_default(),
                    message: self
                        .eval_opt(host, frame, message)?
                        .map(|v| v.to_text())
                        .unwrap_or_default(),
                    ..Default::default()
                };
                let mut commits = Vec::new();
                for b in buttons {
                    if let Some(v) = self.eval_opt(host, frame, &b.label)? {
                        let t = v.to_text();
                        if !t.is_empty() {
                            d.buttons.push(t);
                            commits.push(b.commit);
                        }
                    }
                }
                if d.buttons.is_empty() {
                    d.buttons.push("OK".into());
                    commits.push(true);
                }
                if let Some((f, label)) = input {
                    let label = self
                        .eval_opt(host, frame, label)?
                        .map(|v| v.to_text())
                        .unwrap_or_default();
                    let value = self.field_value(host, *f).unwrap_or_default();
                    d.inputs.push((*f, label, value));
                }
                let inputs = d.inputs.iter().map(|i| i.0).collect();
                return Ok(Flow::Suspend(
                    Suspend::Dialog(d),
                    Pending::Dialog { inputs, commits },
                ));
            }
            (StepParams::Pause(duration), id::PAUSE_RESUME) => {
                let secs = match self.eval_opt(host, frame, duration)? {
                    Some(v) => v.to_text().trim().parse::<f64>().ok(),
                    None => None,
                };
                return Ok(Flow::Suspend(Suspend::Pause(secs), Pending::Pause));
            }
            (_, id::NEW_RECORD) => {
                self.commit(host)?;
                let (table, _, _) = Self::context(host);
                if host.window().mode == Mode::Find {
                    let w = host.window();
                    w.requests.push(Request::default());
                    w.current_request = w.requests.len() - 1;
                } else {
                    let r = host.db().new_record(table).ok_or(error::RECORD_MISSING)?;
                    let w = host.window();
                    w.found.push(r);
                    w.current = w.found.len() - 1;
                    self.open = Some((table, r, None));
                }
            }
            (_, id::DUPLICATE_RECORD) => {
                self.commit(host)?;
                let (table, _, rec) = Self::context(host);
                if host.window().mode == Mode::Find {
                    let w = host.window();
                    let q = w
                        .requests
                        .get(w.current_request)
                        .cloned()
                        .ok_or(error::RECORD_MISSING)?;
                    w.requests.push(q);
                    w.current_request = w.requests.len() - 1;
                } else {
                    let src = rec.ok_or(error::RECORD_MISSING)?;
                    let r = host
                        .db()
                        .duplicate_record(table, src)
                        .ok_or(error::RECORD_MISSING)?;
                    let w = host.window();
                    w.found.insert(w.current + 1, r);
                    w.current += 1;
                }
            }
            (StepParams::Records { dialog, .. }, id::DELETE_RECORD) => {
                self.open = None;
                if host.window().mode == Mode::Find {
                    let w = host.window();
                    if w.current_request < w.requests.len() {
                        w.requests.remove(w.current_request);
                    }
                    if w.requests.is_empty() {
                        w.requests.push(Request::default());
                    }
                    w.current_request = w.current_request.min(w.requests.len() - 1);
                } else {
                    let (table, _, rec) = Self::context(host);
                    let r = rec.ok_or(error::RECORD_MISSING)?;
                    if *dialog {
                        return Ok(confirm_delete(
                            "Permanently delete this ENTIRE record?",
                            table,
                            vec![r],
                        ));
                    }
                    delete_records(host, table, &[r]);
                }
            }
            (StepParams::Records { dialog, .. }, id::DELETE_ALL_RECORDS) => {
                self.open = None;
                let (table, _, _) = Self::context(host);
                let found = host.window().found.clone();
                if *dialog {
                    return Ok(confirm_delete(
                        "Permanently delete ALL records in the found set?",
                        table,
                        found,
                    ));
                }
                delete_records(host, table, &found);
            }
            (_, id::REVERT_RECORD) => {
                if let Some((table, r, old)) = self.open.take() {
                    match old {
                        Some(rec) => {
                            if let Some(t) = host.db().tables.iter_mut().find(|t| t.id == table) {
                                t.records[r] = rec;
                            }
                        }
                        None => delete_records(host, table, &[r]),
                    }
                }
            }
            (
                StepParams::Commit {
                    skip_validation, ..
                },
                id::COMMIT,
            ) => self.commit_with(host, *skip_validation)?,
            (_, id::COMMIT) => self.commit(host)?,
            (StepParams::Mode { pause }, id::ENTER_BROWSE_MODE) => {
                self.commit(host)?;
                let w = host.window();
                w.mode = Mode::Browse;
                w.requests.clear();
                if *pause {
                    return Ok(Flow::Suspend(Suspend::Pause(None), Pending::Pause));
                }
            }
            (StepParams::Mode { pause }, id::ENTER_PREVIEW_MODE) => {
                self.commit(host)?;
                host.window().mode = Mode::Preview;
                if *pause {
                    return Ok(Flow::Suspend(Suspend::Pause(None), Pending::Pause));
                }
            }
            (
                StepParams::Find {
                    restore,
                    requests,
                    pause,
                },
                id::ENTER_FIND_MODE,
            ) => {
                self.commit(host)?;
                let (table, _, _) = Self::context(host);
                let stored = self.requests(host, table, requests);
                let w = host.window();
                w.mode = Mode::Find;
                w.requests = if *restore && !stored.is_empty() {
                    stored
                } else {
                    vec![Request::default()]
                };
                w.current_request = 0;
                if *pause {
                    return Ok(Flow::Suspend(Suspend::Pause(None), Pending::Pause));
                }
            }
            (
                StepParams::Find {
                    restore, requests, ..
                },
                id::PERFORM_FIND,
            ) => {
                self.commit(host)?;
                let (_, to, _) = Self::context(host);
                let reqs = if *restore {
                    self.requests(host, to, requests)
                } else {
                    host.window().requests.clone()
                };
                if reqs.iter().all(|r| r.criteria.is_empty()) {
                    return Err(error::NO_CRITERIA);
                }
                let found = engine::find_from(host.db(), to, &reqs);
                let w = host.window();
                w.mode = Mode::Browse;
                w.requests.clear();
                w.last_find = reqs;
                w.found = found;
                w.current = 0;
                w.sorted = false;
                w.sort.clear();
                if w.found.is_empty() {
                    return Err(error::NO_MATCH);
                }
            }
            (_, id::SHOW_ALL) => {
                self.commit(host)?;
                let (table, _, _) = Self::context(host);
                let n = host.db().table(table).map_or(0, |t| t.records.len());
                let w = host.window();
                w.mode = Mode::Browse;
                w.found = (0..n).collect();
                w.current = 0;
                w.sorted = false;
                w.sort.clear();
            }
            (_, id::SHOW_OMITTED) => {
                self.commit(host)?;
                let (table, _, _) = Self::context(host);
                let n = host.db().table(table).map_or(0, |t| t.records.len());
                let w = host.window();
                w.found = (0..n).filter(|r| !w.found.contains(r)).collect();
                w.current = 0;
                w.sorted = false;
                w.sort.clear();
            }
            (_, id::OMIT_RECORD) => {
                self.commit(host)?;
                let w = host.window();
                if w.mode == Mode::Find {
                    if let Some(q) = w.requests.get_mut(w.current_request) {
                        q.omit = !q.omit;
                    }
                } else {
                    if w.current >= w.found.len() {
                        return Err(error::RECORD_MISSING);
                    }
                    w.found.remove(w.current);
                    w.current = w.current.min(w.found.len().saturating_sub(1));
                }
            }
            (StepParams::Records { count, .. }, id::OMIT_MULTIPLE) => {
                self.commit(host)?;
                let n = match self.eval_opt(host, frame, count)? {
                    Some(v) => v.to_text().parse::<usize>().unwrap_or(1),
                    None => 1,
                };
                let w = host.window();
                let end = (w.current + n).min(w.found.len());
                w.found.drain(w.current.min(end)..end);
                w.current = w.current.min(w.found.len().saturating_sub(1));
            }
            (StepParams::Sort { keys, dialog, .. }, id::SORT) => {
                self.commit(host)?;
                if !keys.is_empty() {
                    let (table, _, _) = Self::context(host);
                    let mut found = host.window().found.clone();
                    sort(host.db(), table, &mut found, keys);
                    let fields = sort_fields(host.db(), table, keys.iter().map(|k| k.0));
                    let w = host.window();
                    w.found = found;
                    w.current = 0;
                    w.sorted = true;
                    w.sort = fields;
                }
                // With dialog: the Sort Records dialog, on the stored order.
                if *dialog {
                    return Ok(Flow::Suspend(
                        Suspend::Ui(UiRequest::Sort),
                        Pending::Ui(UiPending::Done),
                    ));
                }
            }
            (_, id::UNSORT) => {
                self.commit(host)?;
                let w = host.window();
                w.found.sort_unstable();
                w.sorted = false;
                w.sort.clear();
            }
            (_, id::FREEZE_WINDOW) => host.freeze(),
            (_, id::REFRESH_WINDOW) => host.refresh(),
            (_, id::BEEP) => host.beep(),
            (
                _,
                accounts::CHANGE_PASSWORD
                | accounts::ADD_ACCOUNT
                | accounts::DELETE_ACCOUNT
                | accounts::RESET_ACCOUNT_PASSWORD
                | accounts::ENABLE_ACCOUNT
                | accounts::RE_LOGIN,
            ) => self.account_step(host, frame, s)?,
            // Export Field Contents goes through the host UI (`exec_more`).
            (_, INSERT_PICTURE | INSERT_FILE) => self.container_step(host, s)?,
            (_, x) if datafiles::handles(x) => return self.exec_files(host, frame, s),
            (_, x) if net::handles(x) => return self.exec_net(host, frame, s),
            _ => {
                return match self.exec_more(host, frame, s) {
                    Some(r) => r,
                    None => Err(host.other(s)),
                };
            }
        }
        Ok(Flow::Next)
    }

    fn truth(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        c: &Option<Calc>,
    ) -> Result<bool, u32> {
        Ok(self.eval_opt(host, frame, c)?.is_some_and(|v| v.truth()))
    }

    fn repetition(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        c: &Option<Calc>,
    ) -> Result<usize, u32> {
        Ok(match self.eval_opt(host, frame, c)? {
            Some(v) => v.to_text().trim().parse::<usize>().unwrap_or(1).max(1),
            None => 1,
        })
    }

    fn eval_dest(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        d: &RecordDest,
    ) -> Result<Dest, u32> {
        Ok(match d {
            RecordDest::First => Dest::First,
            RecordDest::Last => Dest::Last,
            RecordDest::Previous => Dest::Previous,
            RecordDest::Next => Dest::Next,
            RecordDest::ByCalc(c) => {
                let v = self.eval(host, frame, c)?.to_text();
                Dest::Nth(v.trim().parse::<f64>().map_or(0, |n| n as usize))
            }
        })
    }

    fn go_to_layout(&mut self, host: &mut dyn Host, layout: u32) -> Result<(), u32> {
        self.commit(host)?;
        let current = host.window().layout;
        let db = host.db();
        let (_, new) = layout_table(db, layout).ok_or(error::LAYOUT_MISSING)?;
        let new = (new.id, new.records.len());
        let old = layout_table(db, current).map(|(_, t)| t.id);
        let w = host.window();
        w.layout = layout;
        // ponytail: one found set per window; switching tables shows all records.
        if old != Some(new.0) {
            w.found = (0..new.1).collect();
            w.current = 0;
            w.sorted = false;
            w.sort.clear();
        }
        Ok(())
    }

    /// Stored find requests as engine requests from the layout's
    /// occurrence `to` (criteria on other tables as [`engine::related_key`]).
    fn requests(
        &self,
        host: &mut dyn Host,
        to: u32,
        stored: &[fmp12::scripts::FindRequest],
    ) -> Vec<Request> {
        let db = host.db();
        let table = db.table_of(to).map(|t| t.id);
        stored
            .iter()
            .map(|r| Request {
                omit: r.omit,
                criteria: r
                    .criteria
                    .iter()
                    .map(|(f, t)| {
                        let local = f.to == to || db.table_of(f.to).map(|t| t.id) == table;
                        let k = if local {
                            f.field
                        } else {
                            engine::related_key(f.to, f.field)
                        };
                        (k, t.clone())
                    })
                    .collect(),
            })
            .collect()
    }

    /// Insert Picture, Insert File (a file from the host into the step's
    /// field) and Export Field Contents (the field's file to the host).
    // ponytail: the file paths and options stored in the steps are not
    // decoded; the host asks for the file.
    fn container_step(&mut self, host: &mut dyn Host, s: &Step) -> Result<(), u32> {
        let f = s.inline_field().ok_or(error::FIELD_MISSING)?;
        if s.id != EXPORT_FIELD_CONTENTS {
            let (name, data) = host
                .pick_file(s.id == INSERT_PICTURE)
                .ok_or(error::USER_CANCELED)?;
            let file = calc::value::Container {
                name,
                data,
                ..Default::default()
            };
            return self.set_field(host, f, Value::Container(file));
        }
        let (table, to, rec) = Self::context(host);
        let db = host.db();
        let t = db.table_of(f.to).ok_or(error::FIELD_MISSING)?.id;
        let rec = rec.ok_or(error::RECORD_MISSING)?;
        let r = if t == table {
            rec
        } else {
            *db.related(to, rec, f.to)
                .first()
                .ok_or(error::RECORD_MISSING)?
        };
        match db.value_rep(t, r, f.field, f.repetition.max(1) as u32) {
            Value::Container(c) if !c.data.is_empty() => {
                if host.save_file(&c.name, &c.data) {
                    Ok(())
                } else {
                    Err(error::USER_CANCELED)
                }
            }
            _ => Ok(()),
        }
    }

    /// Current text of a field for the current record (or its first related record).
    fn field_value(&mut self, host: &mut dyn Host, f: FieldRef) -> Option<String> {
        let (table, to, rec) = Self::context(host);
        let db = host.db();
        let t = db.table_of(f.to)?;
        let r = if t.id == table {
            rec?
        } else {
            *db.related(to, rec?, f.to).first()?
        };
        Some(db.value(t.id, r, f.field).to_text())
    }

    /// Insert steps: `text` replaces the field's contents (`select`) or
    /// goes after them.
    fn insert(
        &mut self,
        host: &mut dyn Host,
        f: FieldRef,
        text: &str,
        select: bool,
    ) -> Result<(), u32> {
        let old = if select {
            String::new()
        } else {
            self.field_value(host, f).unwrap_or_default()
        };
        self.set_field(host, f, Value::text(format!("{old}{text}")))
    }

    fn set_field(&mut self, host: &mut dyn Host, f: FieldRef, v: Value) -> Result<(), u32> {
        let (table, to, rec) = Self::context(host);
        let mode = host.window().mode;
        let db = host.db();
        let t = db.table_of(f.to).ok_or(error::FIELD_MISSING)?;
        t.field(f.field).ok_or(error::FIELD_MISSING)?;
        let tid = t.id;
        if mode == Mode::Find {
            if tid != table {
                return Ok(()); // ponytail: related criteria not supported
            }
            let w = host.window();
            let q = w
                .requests
                .get_mut(w.current_request)
                .ok_or(error::RECORD_MISSING)?;
            q.criteria.retain(|(x, _)| *x != f.field);
            q.criteria.push((f.field, v.to_text()));
            return Ok(());
        }
        let rec = rec.ok_or(error::RECORD_MISSING)?;
        let r = if tid == table {
            rec
        } else {
            *db.related(to, rec, f.to)
                .first()
                .ok_or(error::RECORD_MISSING)?
        };
        // Containers join the table's store (Base64Decode, Insert File…).
        let text = db.store_value(tid, f.field, &v);
        let key = engine::rep_key(f.field, f.repetition.max(1) as u32);
        let ti = db
            .tables
            .iter()
            .position(|t| t.id == tid)
            .ok_or(error::FIELD_MISSING)?;
        if !matches!(&self.open, Some((ot, or, _)) if *ot == tid && *or == r) {
            self.open = Some((tid, r, Some(db.tables[ti].records[r].clone())));
        }
        // Lookups matching on the field copy again, stamps update.
        db.set_values(tid, r, &[(key, text)]);
        Ok(())
    }
}

/// The calculations of a step's parameters, in position order.
fn calcs(s: &Step) -> Vec<Calc> {
    (0..16).filter_map(|p| s.raw_params.calc(p)).collect()
}

/// `Get ( flag )` for the current record (Insert Current Date…).
fn now(host: &mut dyn Host, flag: &str) -> Value {
    let (table, to, rec) = Engine::context(host);
    host.db()
        .eval_source(&format!("Get ( {flag} )"), table, to, rec)
}

/// The field named `TO::Field` (Set Field By Name).
fn field_named(db: &Database, name: &str) -> Option<FieldRef> {
    let (to, field) = name.split_once("::")?;
    let to = db
        .occurrences
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(to.trim()))?;
    let f = db.table(to.base_table)?.field_by_name(field.trim())?;
    Some(FieldRef {
        to: to.id,
        field: f.id,
        repetition: 1,
    })
}

/// FileMaker's delete confirmation: Delete (button 1) or Cancel.
fn confirm_delete(message: &str, table: u32, records: Vec<usize>) -> Flow {
    let d = Dialog {
        message: message.into(),
        buttons: vec!["Delete".into(), "Cancel".into()],
        ..Default::default()
    };
    Flow::Suspend(Suspend::Dialog(d), Pending::Delete { table, records })
}

enum Dest {
    First,
    Last,
    Previous,
    Next,
    Nth(usize),
}

/// Deletes records (table indexes) and fixes the window's found set.
fn delete_records(host: &mut dyn Host, table: u32, records: &[usize]) {
    let mut del = records.to_vec();
    del.sort_unstable();
    del.dedup();
    if let Some(t) = host.db().tables.iter_mut().find(|t| t.id == table) {
        for &r in del.iter().rev() {
            if r < t.records.len() {
                t.records.remove(r);
            }
        }
    }
    // Records of another table (a portal row): the found set stays.
    let layout = host.window().layout;
    if layout_table(host.db(), layout).map(|(_, t)| t.id) != Some(table) {
        return;
    }
    let w = host.window();
    w.found = w
        .found
        .iter()
        .filter(|r| del.binary_search(r).is_err())
        .map(|&r| r - del.iter().filter(|&&d| d < r).count())
        .collect();
    w.current = w.current.min(w.found.len().saturating_sub(1));
}

/// The fields of `table` among sort keys, for [`Window::sort`].
pub fn sort_fields(db: &Database, table: u32, keys: impl Iterator<Item = FieldRef>) -> Vec<u32> {
    keys.filter(|f| db.table_of(f.to).is_some_and(|t| t.id == table))
        .map(|f| f.field)
        .collect()
}

/// Sorts a found set by stored sort keys of `table` (other tables' fields
/// are ignored). Custom orders follow the value list, other values after.
pub fn sort(db: &Database, table: u32, found: &mut [usize], keys: &[(FieldRef, SortOrder)]) {
    let keys: Vec<(u32, SortOrder, Vec<String>)> = keys
        .iter()
        .filter(|(f, _)| db.table_of(f.to).is_some_and(|t| t.id == table))
        .map(|(f, o)| {
            let vl = match o {
                SortOrder::Custom(v) => db.value_list_values(*v),
                _ => Vec::new(),
            };
            (f.field, *o, vl)
        })
        .collect();
    found.sort_by(|&a, &b| {
        for (f, o, vl) in &keys {
            let (va, vb) = (db.value(table, a, *f), db.value(table, b, *f));
            let rank = |v: &Value| {
                let t = v.to_text();
                vl.iter()
                    .position(|x| x.eq_ignore_ascii_case(&t))
                    .unwrap_or(vl.len())
            };
            let c = match (va.is_empty(), vb.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => rank(&va)
                    .cmp(&rank(&vb))
                    .then_with(|| calc::eval::compare(&va, &vb)),
            };
            let c = if *o == SortOrder::Descending {
                c.reverse()
            } else {
                c
            };
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });
}

#[cfg(test)]
mod tests;
