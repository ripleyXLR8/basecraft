//! Script triggers fired by script steps, as in FileMaker: a step that
//! changes the mode, the record, the layout, the window or the active
//! object fires the triggers a user doing the same would fire.
//!
//! "Pre" triggers (OnObjectExit, OnRecordCommit, OnRecordRevert,
//! OnLayoutExit, OnModeExit, OnWindowClose, OnLastWindowClose) run their
//! script before the step; an `Exit Script [ False ]` cancels the step,
//! which fails with error 20 (`Get ( LastError )`, shown only without
//! Set Error Capture). "Post" triggers (OnObjectEnter, OnLayoutEnter,
//! OnModeEnter, OnRecordLoad, OnViewChange, OnWindowOpen) run after the
//! step, before the next one: the running script waits for them (it is
//! not interrupted in the middle of a step), and `Get ( LastError )` is
//! the step's again afterwards.
//!
//! Not fired by steps, as FileMaker documents ("Actions that don't
//! activate script triggers"): OnObjectModify for Set Field, Set Field By
//! Name and Show Custom Dialog's input fields (they change the data, not
//! a layout object).
//!
//! ponytail: no trigger fires while a triggered script runs (no trigger
//! loops; FileMaker fires them and can loop); the host's [`Host::trigger`]
//! finds the scripts.

use fmp12::scripts::{Calc, Step, StepParams, id};
use fmp12::triggers::event;

use crate::steps::sid;
use crate::{Call, Engine, Frame, Host, Mode, Run, View, Window};

/// FileMaker's "Command was canceled" (a pre trigger returned False).
pub const CANCELED: u32 = 20;

/// What kind of call a run's call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Kind {
    /// A script (Perform Script, or the run's own).
    #[default]
    Script,
    /// A pre trigger's script: its result lets the step run or cancels it.
    Pre,
    /// A post trigger's script; the step's error to restore after it.
    Post(u32),
}

/// What a step may change that fires post triggers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Seen {
    name: String,
    windows: usize,
    layout: u32,
    mode: Mode,
    record: Option<usize>,
    view: View,
    focus: Option<u64>,
}

/// An event and the layout object it is on (`None`: the layout's or the
/// file's).
type Ev = (u16, Option<u64>);

impl Run {
    /// A triggered script is running.
    fn in_trigger(&self) -> bool {
        self.stack.iter().any(|c| c.kind != Kind::Script)
    }
}

/// The layout object with the focus: Go to Object's, else the first one
/// showing the active field.
fn focus(host: &mut dyn Host) -> Option<u64> {
    let w = host.window().clone();
    let l = host.db().layouts.iter().find(|l| l.id == w.layout)?;
    if !w.object.is_empty() {
        return l.objects.iter().find(|o| o.name == w.object).map(|o| o.id);
    }
    let f = w.field?;
    l.objects
        .iter()
        .find(|o| {
            o.field
                .is_some_and(|(to, fd, _)| (to & 0xFFF) as u32 == f.to && fd as u32 == f.field)
        })
        .map(|o| o.id)
}

fn seen(e: &Engine, host: &mut dyn Host) -> Seen {
    let focus = focus(host);
    let w: &Window = host.window();
    Seen {
        name: w.name.clone(),
        windows: e.windows.len(),
        layout: w.layout,
        mode: w.mode,
        record: w.record().filter(|_| w.mode != Mode::Find),
        view: w.view,
        focus,
    }
}

impl Engine {
    /// Before the top call's current step: runs its next pre trigger.
    /// True when a trigger's script was pushed (the step waits for it).
    pub(crate) fn pre_trigger(&mut self, run: &mut Run, host: &mut dyn Host) -> bool {
        if run.in_trigger() {
            return false;
        }
        let Some(top) = run.stack.last_mut() else {
            return false;
        };
        if top.pres.is_none() {
            let Call {
                steps, pc, frame, ..
            } = top;
            let s = steps[*pc].clone();
            let evs = self.pre_events(host, frame, &s);
            top.pres = Some(evs);
        }
        loop {
            let Some(top) = run.stack.last_mut() else {
                return false;
            };
            let Some(ev) = top
                .pres
                .as_mut()
                .filter(|v| !v.is_empty())
                .map(|v| v.remove(0))
            else {
                return false;
            };
            if self.push_trigger(run, host, ev, Kind::Pre) {
                return true;
            }
        }
    }

    /// The pre events of step `s`, in FileMaker's order (object, record,
    /// layout, mode, window).
    fn pre_events(&mut self, host: &mut dyn Host, frame: &mut Frame, s: &Step) -> Vec<Ev> {
        let open = self.open.is_some();
        let obj = focus(host);
        let mode = host.window().mode;
        let leave = |v: &mut Vec<Ev>| {
            if let Some(o) = obj {
                v.push((event::OBJECT_EXIT, Some(o)));
            }
        };
        let commit = |v: &mut Vec<Ev>| {
            if open {
                v.push((event::RECORD_COMMIT, None));
            }
        };
        let mut v = Vec::new();
        match (&s.params, s.id) {
            (_, id::ENTER_BROWSE_MODE | id::ENTER_FIND_MODE | id::ENTER_PREVIEW_MODE) => {
                let to = match s.id {
                    id::ENTER_BROWSE_MODE => Mode::Browse,
                    id::ENTER_FIND_MODE => Mode::Find,
                    _ => Mode::Preview,
                };
                if to != mode {
                    leave(&mut v);
                    commit(&mut v);
                    v.push((event::MODE_EXIT, None));
                }
            }
            (_, id::COMMIT) => {
                leave(&mut v);
                commit(&mut v);
            }
            (_, id::REVERT_RECORD) if open => v.push((event::RECORD_REVERT, None)),
            (StepParams::GoToRecord { .. }, _) => {
                leave(&mut v);
                commit(&mut v);
            }
            (StepParams::GoToLayout { dest, .. }, _) => {
                let same = matches!(dest, fmp12::scripts::LayoutDest::Layout(l)
                    if *l == host.window().layout);
                if !same {
                    leave(&mut v);
                    commit(&mut v);
                    v.push((event::LAYOUT_EXIT, None));
                }
            }
            (StepParams::GoToField { field, .. }, _) => {
                if *field != host.window().field || !host.window().object.is_empty() {
                    leave(&mut v);
                }
            }
            (StepParams::GoToObject { .. }, _) => leave(&mut v),
            (_, sid::NEXT_FIELD | sid::PREVIOUS_FIELD) => leave(&mut v),
            (_, sid::CLOSE_WINDOW) => {
                let name = self.window_name(host, frame, s).ok().flatten();
                let front = name.is_none_or(|n| host.window().name.eq_ignore_ascii_case(&n));
                if front {
                    leave(&mut v);
                    commit(&mut v);
                }
                v.push((event::WINDOW_CLOSE, None));
                if front && self.windows.is_empty() {
                    v.push((event::LAST_WINDOW_CLOSE, None));
                }
            }
            _ => {}
        }
        v
    }

    /// What the post triggers compare after the step about to run;
    /// `None` while a triggered script runs.
    pub(crate) fn watch(&self, run: &Run, host: &mut dyn Host) -> Option<Seen> {
        (!run.in_trigger()).then(|| seen(self, host))
    }

    /// After a step: pushes the post triggers of what it changed (the
    /// first event's script on top, so it runs first). True if any.
    pub(crate) fn post_triggers(&mut self, run: &mut Run, host: &mut dyn Host, was: Seen) -> bool {
        if run.stack.is_empty() || run.in_trigger() {
            return false;
        }
        let now = seen(self, host);
        let mut evs: Vec<Ev> = Vec::new();
        if now.windows > was.windows {
            evs.push((event::WINDOW_OPEN, None));
        } else if now.name == was.name && now.windows == was.windows {
            let layout = now.layout != was.layout;
            if layout {
                evs.push((event::LAYOUT_ENTER, None));
            }
            if now.mode != was.mode {
                evs.push((event::MODE_ENTER, None));
            }
            if now.record.is_some() && (layout || now.record != was.record) {
                evs.push((event::RECORD_LOAD, None));
            }
            if now.view != was.view {
                evs.push((event::VIEW_CHANGE, None));
            }
            if let Some(o) = now.focus.filter(|_| now.focus != was.focus || layout) {
                evs.push((event::OBJECT_ENTER, Some(o)));
            }
        }
        let err = self.last_error;
        let mut any = false;
        for ev in evs.into_iter().rev() {
            any |= self.push_trigger(run, host, ev, Kind::Post(err));
        }
        any
    }

    /// Pushes the script of the trigger set for `ev`, if any.
    fn push_trigger(&mut self, run: &mut Run, host: &mut dyn Host, ev: Ev, kind: Kind) -> bool {
        let Some((script, param)) = host.trigger(ev.0, ev.1) else {
            return false;
        };
        let Some(i) = self.scripts.iter().position(|s| s.id == script) else {
            return false;
        };
        let value = param
            .and_then(|c: Calc| {
                let mut frame = Frame {
                    name: String::new(),
                    locals: Default::default(),
                    parameter: calc::Value::empty(),
                    result: calc::Value::empty(),
                    original_layout: host.window().layout,
                };
                self.eval(host, &mut frame, &c).ok()
            })
            .unwrap_or_else(calc::Value::empty);
        self.push(host, run, i, value);
        if let Some(c) = run.stack.last_mut() {
            c.kind = kind;
        }
        true
    }

    /// A triggered call returned with `result`: a pre trigger's False
    /// cancels its step (error 20); after a post trigger, the step's
    /// error again.
    pub(crate) fn trigger_returned(
        &mut self,
        run: &mut Run,
        host: &mut dyn Host,
        kind: Kind,
        result: &calc::Value,
    ) {
        match kind {
            Kind::Pre if !(result.is_empty() || result.truth()) => {
                self.complete(run, host, crate::Flow::Next, CANCELED)
            }
            Kind::Post(err) => self.last_error = err,
            _ => {}
        }
    }
}
