//! Script triggers fired by script steps (`triggers.rs`): pre triggers
//! run before the step and a False result cancels it (error 20); post
//! triggers run after the step, before the next one.

use super::*;
use crate::steps::sid;
use fmp12::scripts::{LayoutDest, RecordDest};
use fmp12::triggers::event;

/// The People host with triggers: (event, object, script).
struct Trig {
    h: TestHost,
    on: Vec<(u16, Option<u64>, u32)>,
}

impl Host for Trig {
    fn db(&mut self) -> &mut Database {
        &mut self.h.db
    }
    fn window(&mut self) -> &mut Window {
        &mut self.h.win
    }
    fn error(&mut self, s: &Step, code: u32) -> bool {
        self.h.error(s, code)
    }
    fn trigger(&mut self, ev: u16, object: Option<u64>) -> Option<(u32, Option<Calc>)> {
        self.on
            .iter()
            .find(|t| t.0 == ev && t.1 == object)
            .map(|t| (t.2, None))
    }
}

/// Script `id`: `$$log` gets `tag` appended, then Exit Script [ `$$ok` ].
fn logger(id: u32, tag: &str) -> Script {
    script(
        id,
        vec![var("$$log", &format!("$$log & \"{tag}\"")), exit("$$ok")],
    )
}

fn host(on: &[(u16, Option<u64>, u32)]) -> Trig {
    let mut h = TestHost::new(PEOPLE);
    // Name and Age field objects on layout 1.
    for (id, f) in [(10, NAME), (11, AGE)] {
        h.db.layouts[0].objects.push(fmp12::layouts::LayoutObject {
            id,
            kind: 1,
            field: Some((0x104000 | T as u64, f as u64, 1)),
            ..Default::default()
        });
    }
    Trig { h, on: on.to_vec() }
}

fn engine(main: Vec<Step>, ok: bool) -> Engine {
    let mut scripts = vec![script(1, main)];
    for (id, tag) in [(2, "A"), (3, "B"), (4, "C"), (5, "D")] {
        scripts.push(logger(id, tag));
    }
    let mut e = Engine::new(scripts);
    e.globals
        .insert(("$$ok".into(), 1), Value::int(i64::from(ok)));
    e
}

fn next_record() -> Step {
    step(
        id::GO_TO_RECORD,
        StepParams::GoToRecord {
            dest: RecordDest::Next,
            exit_after_last: false,
            dialog: false,
        },
    )
}

fn global(e: &Engine, name: &str) -> String {
    e.globals
        .get(&(name.to_lowercase(), 1))
        .map(|v| v.to_text())
        .unwrap_or_default()
}

/// Go to Record on a modified record: OnRecordCommit before it (False
/// cancels with error 20, the record stays), OnRecordLoad after it and
/// before the next step; `Get ( LastError )` is the step's again.
#[test]
fn record_commit_and_load() {
    let main = vec![
        set_field(NAME, "\"Zed\""),
        next_record(),
        var("$$err", "Get(LastError)"),
        var("$$seen", "$$log"),
    ];
    let on = [
        (event::RECORD_COMMIT, None, 2),
        (event::RECORD_LOAD, None, 3),
    ];
    let mut h = host(&on);
    let mut e = engine(main.clone(), false);
    e.error_capture = true;
    e.perform(&mut h, 1, None);
    assert_eq!(global(&e, "$$log"), "A");
    assert_eq!(global(&e, "$$err"), "20");
    assert_eq!(h.h.win.current, 0, "the step was canceled");
    assert!(e.open.is_some(), "still uncommitted");

    let mut h = host(&on);
    let mut e = engine(main, true);
    e.perform(&mut h, 1, None);
    assert_eq!(
        global(&e, "$$seen"),
        "AB",
        "OnRecordLoad ran before the next step"
    );
    assert_eq!(global(&e, "$$err"), "0");
    assert_eq!(h.h.win.current, 1);
    assert!(h.h.errors.is_empty());
}

/// Enter Find Mode: OnModeExit (False keeps Browse mode), OnModeEnter
/// after; Enter Browse Mode back fires both again.
#[test]
fn mode_exit_and_enter() {
    let find = step(
        id::ENTER_FIND_MODE,
        StepParams::Find {
            pause: false,
            restore: false,
            requests: vec![],
        },
    );
    let browse = step(id::ENTER_BROWSE_MODE, StepParams::Mode { pause: false });
    let on = [(event::MODE_EXIT, None, 2), (event::MODE_ENTER, None, 3)];
    let mut h = host(&on);
    let mut e = engine(vec![find.clone()], false);
    e.perform(&mut h, 1, None);
    assert_eq!(h.h.win.mode, Mode::Browse);
    assert_eq!(global(&e, "$$log"), "A");
    // Without Set Error Capture, FileMaker reports the canceled step.
    assert_eq!(h.h.errors, [20]);

    let mut h = host(&on);
    let mut e = engine(vec![find, browse], true);
    e.perform(&mut h, 1, None);
    assert_eq!(h.h.win.mode, Mode::Browse);
    assert_eq!(global(&e, "$$log"), "ABAB");
}

/// Revert Record: OnRecordRevert's False keeps the changes.
#[test]
fn record_revert() {
    let main = vec![set_field(NAME, "\"Zed\""), plain(id::REVERT_RECORD)];
    let on = [(event::RECORD_REVERT, None, 2)];
    let mut h = host(&on);
    let mut e = engine(main.clone(), false);
    e.perform(&mut h, 1, None);
    assert_eq!(h.h.db.tables[0].records[0].values[&NAME], "Zed");
    let mut h = host(&on);
    let mut e = engine(main, true);
    e.perform(&mut h, 1, None);
    assert_eq!(h.h.db.tables[0].records[0].values[&NAME], "Ann");
    assert_eq!(global(&e, "$$log"), "A");
}

/// Go to Layout: OnLayoutExit before (False stays), then OnLayoutEnter
/// and OnRecordLoad in that order.
#[test]
fn layout_exit_enter_load() {
    let go = step(
        id::GO_TO_LAYOUT,
        StepParams::GoToLayout {
            dest: LayoutDest::Layout(2),
            animation: 0,
        },
    );
    let on = [
        (event::LAYOUT_EXIT, None, 2),
        (event::LAYOUT_ENTER, None, 3),
        (event::RECORD_LOAD, None, 4),
    ];
    let mut h = host(&on);
    let mut e = engine(vec![go.clone()], false);
    e.perform(&mut h, 1, None);
    assert_eq!(h.h.win.layout, 1);
    let mut h = host(&on);
    let mut e = engine(vec![go], true);
    e.perform(&mut h, 1, None);
    assert_eq!(h.h.win.layout, 2);
    assert_eq!(global(&e, "$$log"), "ABC");
}

/// Go to Field: OnObjectExit of the active field (False keeps it), then
/// OnObjectEnter of the new one. A triggered script's own steps fire no
/// triggers.
#[test]
fn object_exit_keeps_focus() {
    let go = |f| {
        step(
            id::GO_TO_FIELD,
            StepParams::GoToField {
                field: Some(field(f)),
                select: false,
            },
        )
    };
    let on = [
        (event::OBJECT_EXIT, Some(10), 2),
        (event::OBJECT_ENTER, Some(10), 3),
        (event::OBJECT_ENTER, Some(11), 4),
    ];
    let mut h = host(&on);
    let mut e = engine(vec![go(NAME), go(AGE)], false);
    e.error_capture = true;
    e.perform(&mut h, 1, None);
    assert_eq!(global(&e, "$$log"), "BA");
    assert_eq!(h.h.win.field, Some(field(NAME)), "the focus stays");
    let mut h = host(&on);
    let mut e = engine(vec![go(NAME), go(AGE)], true);
    e.perform(&mut h, 1, None);
    assert_eq!(global(&e, "$$log"), "BAC");
    assert_eq!(h.h.win.field, Some(field(AGE)));

    // Script 2 (OnObjectExit) itself goes to Age: no trigger fires for
    // it, and the step then finds the focus already there.
    let mut h = host(&on);
    let mut e = engine(vec![go(NAME), go(AGE)], true);
    e.scripts[1].steps.insert(0, go(AGE));
    e.perform(&mut h, 1, None);
    assert_eq!(global(&e, "$$log"), "BA");
    assert_eq!(h.h.win.field, Some(field(AGE)));
}

/// Close Window after New Window: OnWindowOpen after New Window,
/// OnWindowClose before Close Window (False keeps the window open).
#[test]
fn window_open_close() {
    let main = vec![plain(sid::NEW_WINDOW), plain(sid::CLOSE_WINDOW)];
    let on = [
        (event::WINDOW_OPEN, None, 2),
        (event::WINDOW_CLOSE, None, 3),
    ];
    let mut h = host(&on);
    let mut e = engine(main.clone(), false);
    e.globals.insert(("$$ok".into(), 1), Value::empty());
    // Script 3 returns False; script 2's empty result does not cancel.
    e.scripts[2].steps[1] = exit("0");
    e.perform(&mut h, 1, None);
    assert_eq!(global(&e, "$$log"), "AB");
    assert_eq!(e.windows.len(), 1, "kept open");
    let mut h = host(&on);
    let mut e = engine(main, true);
    e.perform(&mut h, 1, None);
    assert_eq!(e.windows.len(), 0);
}

/// Set Field does not fire OnObjectModify (FileMaker: it changes the
/// data, not the layout object).
#[test]
fn set_field_fires_no_modify() {
    let go = step(
        id::GO_TO_FIELD,
        StepParams::GoToField {
            field: Some(field(NAME)),
            select: false,
        },
    );
    let mut h = host(&[(event::OBJECT_MODIFY, Some(10), 2)]);
    let mut e = engine(vec![go, set_field(NAME, "\"Zed\"")], true);
    e.perform(&mut h, 1, None);
    assert_eq!(global(&e, "$$log"), "");
}
