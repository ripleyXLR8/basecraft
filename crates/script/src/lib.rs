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

pub mod text;

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
    pub const OBJECT_MISSING: u32 = 112;
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
#[derive(Debug, Clone, Default)]
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
    /// Find requests (Find mode) and the current one.
    pub requests: Vec<Request>,
    pub current_request: usize,
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
}

/// One script call on the run's stack.
struct Call {
    steps: Vec<Step>,
    links: Links,
    pc: usize,
    frame: Frame,
}

/// A script run in progress ([`Engine::start`]).
pub struct Run {
    stack: Vec<Call>,
    pending: Option<Pending>,
    result: Value,
    halted: bool,
    /// Error before anything ran (script missing).
    early: Option<u32>,
}

impl Run {
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
                Status::Suspended(Suspend::Pause(_)) => answer = Answer::Continue,
            }
        }
    }

    /// Whether the run is waiting for an answer.
    pub fn is_suspended(&self) -> bool {
        self.pending.is_some()
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
}

/// One script call.
struct Frame {
    name: String,
    locals: Vars,
    parameter: Value,
    result: Value,
    original_layout: u32,
}

enum Flow {
    Next,
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
        }
    }

    /// The scripts of a file.
    pub fn load(file: &fmp12::File) -> Result<Engine, fmp12::Error> {
        Ok(Engine::new(fmp12::scripts::scripts(file)?))
    }

    /// Starts script `id` with an optional parameter; [`Run::resume`] runs it.
    pub fn start(&mut self, host: &mut dyn Host, id: u32, parameter: Option<Value>) -> Run {
        self.error_capture = false;
        self.allow_abort = true;
        let mut run = Run {
            stack: Vec::new(),
            pending: None,
            result: Value::empty(),
            halted: false,
            early: None,
        };
        match self.scripts.iter().position(|s| s.id == id) {
            Some(i) => self.push(host, &mut run, i, parameter.unwrap_or_else(Value::empty)),
            None => run.early = Some(error::SCRIPT_MISSING),
        }
        run
    }

    /// Starts one step outside a script: a layout button's action
    /// (`LayoutObject::button`).
    pub fn start_step(&mut self, host: &mut dyn Host, step: &Step) -> Run {
        self.error_capture = false;
        let steps = vec![step.clone()];
        Run {
            stack: vec![Call {
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
            }],
            pending: None,
            result: Value::empty(),
            halted: false,
            early: None,
        }
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
        });
    }

    /// Runs steps until the run ends or suspends.
    fn drive(&mut self, run: &mut Run, host: &mut dyn Host) -> Status {
        loop {
            let Some(top) = run.stack.last_mut() else {
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
            let pc = top.pc;
            match self.exec(host, &top.steps, &top.links, pc, &mut top.frame) {
                Ok(Flow::Call(i, p)) => self.push(host, run, i, p),
                Ok(Flow::Suspend(s, p)) => {
                    run.pending = Some(p);
                    return Status::Suspended(s);
                }
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
        let Some(s) = top.steps.get(top.pc) else {
            return;
        };
        if s.id != id::COMMENT {
            self.last_error = err;
        }
        if err != 0 {
            self.last_error_location = format!(
                "{}\r{}\r{}",
                top.frame.name,
                s.name().unwrap_or_default(),
                top.pc + 1
            );
            if !self.error_capture && !host.error(s, err) {
                Self::halt(run);
                return;
            }
        }
        match flow {
            Flow::Next => top.pc += 1,
            Flow::Jump(j) => top.pc = j,
            Flow::Exit(v) => self.ret(run, host, v, false),
            Flow::Halt => Self::halt(run),
            Flow::Call(..) | Flow::Suspend(..) => top.pc += 1,
        }
    }

    /// Halt Script (or a cancelled error): every level stops.
    fn halt(run: &mut Run) {
        run.stack.clear();
        run.result = Value::empty();
        run.halted = true;
    }

    /// Returns from the top call with `result`.
    fn ret(&mut self, run: &mut Run, host: &mut dyn Host, result: Value, halted: bool) {
        run.stack.pop();
        if run.stack.is_empty() {
            run.result = result;
            run.halted = halted;
            return;
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
        flags.insert("lasterrordetail", Value::text(""));
        flags.insert("scriptanimationstate", num(0));
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
        }
        if let Some(t) = db.table(table) {
            flags.insert("layouttablename", Value::text(&t.name));
        }
        let mut env = ScriptEnv {
            locals: &mut frame.locals,
            globals: &mut self.globals,
            flags,
        };
        db.eval_tokens_in(&c.0, table, to, record, Some(&mut env))
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

    /// Commit Records; `skip` = "Skip data entry validation" (fields
    /// validated "always" are still checked).
    fn commit_with(&mut self, host: &mut dyn Host, skip: bool) -> Result<(), u32> {
        let Some((table, r, _)) = self.open else {
            return Ok(());
        };
        let db = host.db();
        let always = |f: u32| {
            db.table(table)
                .and_then(|t| t.field(f))
                .is_some_and(|f| f.def.validation().always)
        };
        if let Some(e) = db
            .validate(table, r)
            .into_iter()
            .find(|e| !skip || always(e.field))
        {
            return Err(e.rule.code());
        }
        self.open = None;
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
                let i = script
                    .and_then(|id| self.scripts.iter().position(|s| s.id == id))
                    .ok_or(error::SCRIPT_MISSING)?;
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
                    _ => Value::text(host.clipboard().ok_or(error::UNAVAILABLE)?),
                };
                let select = s.options & fmp12::scripts::opt::SELECT != 0;
                match (s.raw_params.text(32), s.inline_field()) {
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
            }
            (StepParams::GoToObject { name, repetition }, _) => {
                let n = self
                    .eval_opt(host, frame, name)?
                    .map(|v| v.to_text())
                    .unwrap_or_default();
                let r = self.repetition(host, frame, repetition)?;
                // ponytail: error code for a missing object not verified against FileMaker.
                if !host.go_to_object(&n, r as u32) {
                    return Err(error::OBJECT_MISSING);
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
                let (table, _, _) = Self::context(host);
                let reqs = if *restore {
                    self.requests(host, table, requests)
                } else {
                    host.window().requests.clone()
                };
                if reqs.iter().all(|r| r.criteria.is_empty()) {
                    return Err(error::NO_CRITERIA);
                }
                let found = engine::find(host.db(), table, &reqs);
                let w = host.window();
                w.mode = Mode::Browse;
                w.requests.clear();
                w.found = found;
                w.current = 0;
                w.sorted = false;
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
            }
            (_, id::SHOW_OMITTED) => {
                self.commit(host)?;
                let (table, _, _) = Self::context(host);
                let n = host.db().table(table).map_or(0, |t| t.records.len());
                let w = host.window();
                w.found = (0..n).filter(|r| !w.found.contains(r)).collect();
                w.current = 0;
                w.sorted = false;
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
            (StepParams::Sort { keys, .. }, id::SORT) if !keys.is_empty() => {
                self.commit(host)?;
                let (table, _, _) = Self::context(host);
                let mut found = host.window().found.clone();
                sort(host.db(), table, &mut found, keys);
                let w = host.window();
                w.found = found;
                w.current = 0;
                w.sorted = true;
            }
            (_, id::UNSORT) => {
                self.commit(host)?;
                let w = host.window();
                w.found.sort_unstable();
                w.sorted = false;
            }
            (_, id::FREEZE_WINDOW) => host.freeze(),
            (_, id::REFRESH_WINDOW) => host.refresh(),
            (_, id::BEEP) => host.beep(),
            (_, INSERT_PICTURE | INSERT_FILE | EXPORT_FIELD_CONTENTS) => {
                self.container_step(host, s)?
            }
            _ => return Err(host.other(s)),
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
        }
        Ok(())
    }

    /// Stored find requests as engine requests on `table` (criteria on
    /// other tables are dropped: related finds are not supported yet).
    fn requests(
        &self,
        host: &mut dyn Host,
        table: u32,
        stored: &[fmp12::scripts::FindRequest],
    ) -> Vec<Request> {
        let db = host.db();
        stored
            .iter()
            .map(|r| Request {
                omit: r.omit,
                criteria: r
                    .criteria
                    .iter()
                    .filter(|(f, _)| db.table_of(f.to).is_some_and(|t| t.id == table))
                    .map(|(f, t)| (f.field, t.clone()))
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
    let w = host.window();
    w.found = w
        .found
        .iter()
        .filter(|r| del.binary_search(r).is_err())
        .map(|&r| r - del.iter().filter(|&&d| d < r).count())
        .collect();
    w.current = w.current.min(w.found.len().saturating_sub(1));
}

/// Sorts a found set by stored sort keys of `table` (other tables' fields
/// are ignored). Custom orders follow the value list, other values after.
fn sort(db: &Database, table: u32, found: &mut [usize], keys: &[(FieldRef, SortOrder)]) {
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
