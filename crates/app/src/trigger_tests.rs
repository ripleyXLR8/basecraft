//! Script triggers fired by the app (`triggers.rs`), driven through
//! `egui_kittest` on the Shop file of `ui_tests` (Invoices layout: fields
//! ClientId, ClientName, Size, objects 100–102).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use fmp12::scripts::{Script, Step, StepParams, id};
use fmp12::triggers::{Trigger, event};

use super::*;
use crate::ui_tests::{click_input, harness_for, retype, shop_app};

fn calc(app: &App, src: &str) -> fmp12::scripts::Calc {
    let names = script::text::Names {
        db: &app.doc.as_ref().unwrap().db,
        scripts: &app.engine.as_ref().unwrap().scripts,
    };
    names.compile(src).unwrap()
}

/// Script `id`: `$$<var>` = `value`, then Exit Script [ `result` ].
fn script(app: &mut App, sid: u32, var: &str, value: &str, result: Option<&str>) {
    let mut set = Step::new(id::SET_VARIABLE);
    set.params = StepParams::SetVariable {
        name: format!("$${var}"),
        value: Some(calc(app, value)),
        repetition: None,
    };
    let mut steps = vec![set];
    if let Some(r) = result {
        let mut exit = Step::new(id::EXIT_SCRIPT);
        exit.params = StepParams::ExitScript(Some(calc(app, r)));
        steps.push(exit);
    }
    let e = app.engine.as_mut().unwrap();
    e.scripts.retain(|s| s.id != sid);
    e.scripts.push(Script {
        id: sid,
        name: format!("Trigger {sid}"),
        options: 8,
        steps,
    });
}

fn trigger(event: u16, script: u32) -> Trigger {
    Trigger {
        event,
        script,
        modes: fmp12::triggers::mode::BROWSE,
        parameter: None,
    }
}

fn layout_id(app: &App) -> u32 {
    app.doc.as_ref().unwrap().layouts[app.layout].id
}

fn on_object(app: &mut App, object: u64, t: Trigger) {
    let l = layout_id(app);
    let d = app.doc.as_mut().unwrap();
    d.triggers.objects.entry((l, object)).or_default().push(t);
}

fn on_layout(app: &mut App, t: Trigger) {
    let l = layout_id(app);
    let d = app.doc.as_mut().unwrap();
    d.triggers.layouts.entry(l).or_default().push(t);
}

fn global(h: &Harness<'_, App>, var: &str) -> String {
    let g = &h.state().engine.as_ref().unwrap().globals;
    g.get(&(var.to_lowercase(), 1))
        .map(|v| v.to_text())
        .unwrap_or_default()
}

fn invoice(h: &Harness<'_, App>, field: &str) -> String {
    let db = &h.state().doc.as_ref().unwrap().db;
    let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
    let f = t.field_by_name(field).unwrap().id;
    db.value(t.id, 0, f).to_text()
}

/// The field being edited has the keyboard focus.
fn in_field(h: &Harness<'_, App>) -> bool {
    let ctx = &h.ctx;
    toolbar::last_focus(ctx).is_some_and(|f| ctx.memory(|m| m.has_focus(f.id)))
}

/// OnObjectValidate returning False keeps the focus in the modified field
/// (no OnObjectSave, no OnObjectExit); returning True lets it go.
#[test]
fn object_validate_cancels() {
    let mut app = shop_app();
    script(&mut app, 90, "v", "$$v + 1", Some("$$allow"));
    script(&mut app, 91, "saved", "$$saved + 1", None);
    script(&mut app, 92, "exited", "$$exited + 1", None);
    on_object(&mut app, 100, trigger(event::OBJECT_VALIDATE, 90));
    on_object(&mut app, 100, trigger(event::OBJECT_SAVE, 91));
    on_object(&mut app, 100, trigger(event::OBJECT_EXIT, 92));
    let mut h = harness_for(app);
    h.run_steps(3);
    // Leaving without a change: no validation.
    click_input(&mut h, 0);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(
        (global(&h, "$$v"), global(&h, "$$exited")),
        ("".into(), "1".into())
    );
    // $$allow is empty: Exit Script [ "" ] would not cancel, so set it to 0.
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    retype(&mut h, 0, "2");
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(global(&h, "$$v"), "1");
    assert_eq!(global(&h, "$$saved"), "");
    assert_eq!(global(&h, "$$exited"), "1");
    assert!(in_field(&h), "the field keeps the focus");
    // Allowed: saved and exited, in that order after the validation.
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    assert_eq!(global(&h, "$$v"), "2", "still modified: validated again");
    assert_eq!(global(&h, "$$saved"), "1");
    assert_eq!(global(&h, "$$exited"), "2");
    assert_eq!(invoice(&h, "ClientId"), "2");
}

/// OnObjectKeystroke sees each keystroke (Get ( TriggerKeystroke )) and a
/// False result drops it.
#[test]
fn object_keystroke_filters() {
    let mut app = shop_app();
    script(
        &mut app,
        90,
        "keys",
        "$$keys & Get ( TriggerKeystroke )",
        Some("Get ( TriggerKeystroke ) <> \"x\""),
    );
    on_object(&mut app, 101, trigger(event::OBJECT_KEYSTROKE, 90));
    let mut h = harness_for(app);
    h.run_steps(3);
    let before = invoice(&h, "ClientName");
    click_input(&mut h, 1);
    for c in ["a", "x", "b"] {
        h.event(egui::Event::Text(c.into()));
        h.run_steps(1);
    }
    h.run_steps(2);
    assert_eq!(global(&h, "$$keys"), "axb");
    let after = invoice(&h, "ClientName");
    assert_eq!(after.len(), before.len() + 2, "{before:?} -> {after:?}");
    assert!(!after.contains('x') && after.contains('a') && after.contains('b'));
}

/// OnModeExit returning False keeps Browse mode (Ctrl+F does nothing).
#[test]
fn mode_exit_cancels() {
    let mut app = shop_app();
    script(&mut app, 90, "left", "$$left + 1", Some("$$allow"));
    on_layout(&mut app, trigger(event::MODE_EXIT, 90));
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    let mut h = harness_for(app);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run_steps(3);
    assert_eq!(h.state().mode(), Mode::Browse);
    assert_eq!(global(&h, "$$left"), "1");
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run_steps(3);
    assert_eq!(h.state().mode(), Mode::Find);
}

/// OnRecordRevert runs before Revert Record…; False keeps the changes.
#[test]
fn record_revert_trigger() {
    let mut app = shop_app();
    script(&mut app, 90, "reverts", "$$reverts + 1", Some("$$allow"));
    on_layout(&mut app, trigger(event::RECORD_REVERT, 90));
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    let mut h = harness_for(app);
    h.run_steps(3);
    let revert = |h: &mut Harness<'_, App>| {
        // A change to the record, as ui_tests' Revert test makes it.
        let app = h.state_mut();
        let d = app.doc.as_mut().unwrap();
        let t =
            d.db.tables
                .iter_mut()
                .find(|t| t.name == "Invoices")
                .unwrap();
        let f = t.field_by_name("Size").unwrap().id;
        t.records[0].values.insert(f, "L".into());
        app.editing = Some((t.id, 0));
        h.get_by_label(tr("Records")).click();
        h.run_steps(2);
        h.get_by_label_contains("Revert Record").click();
        h.run_steps(1);
        h.run_steps(2);
        h.get_by_label("Revert").click();
        h.run_steps(3);
    };
    revert(&mut h);
    assert_eq!(global(&h, "$$reverts"), "1");
    assert_eq!(invoice(&h, "Size"), "L");
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    revert(&mut h);
    assert_eq!(global(&h, "$$reverts"), "2");
    assert_eq!(invoice(&h, "Size"), "M");
}

/// OnWindowOpen after Window › New Window; OnWindowClose and
/// OnLastWindowClose can keep a window open.
#[test]
fn window_open_and_close() {
    let mut app = shop_app();
    script(&mut app, 90, "opened", "$$opened + 1", None);
    script(&mut app, 91, "closing", "$$closing + 1", Some("$$allow"));
    script(&mut app, 92, "last", "$$last + 1", Some("0"));
    let d = app.doc.as_mut().unwrap();
    d.triggers.file = vec![
        trigger(event::WINDOW_OPEN, 90),
        trigger(event::WINDOW_CLOSE, 91),
        trigger(event::LAST_WINDOW_CLOSE, 92),
    ];
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    let mut h = harness_for(app);
    h.run_steps(3);
    h.get_by_label(tr("Window")).click();
    h.run_steps(2);
    h.get_by_label(tr("New Window")).click();
    h.run_steps(4);
    assert_eq!(h.state().window_names().len(), 2);
    assert_eq!(global(&h, "$$opened"), "1");
    // Kept open by OnWindowClose.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    h.run_steps(3);
    assert_eq!(h.state().window_names().len(), 2);
    assert_eq!(global(&h, "$$closing"), "1");
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    h.run_steps(3);
    assert_eq!(h.state().window_names().len(), 1);
    assert_eq!(global(&h, "$$last"), "");
    // The last window: OnLastWindowClose returns False, the file stays open.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    h.run_steps(3);
    assert_eq!(global(&h, "$$last"), "1");
    assert!(h.state().doc.is_some());
}

/// OnPanelSwitch: Get ( TriggerCurrentPanel ) / Get ( TriggerTargetPanel )
/// are "index¶name"; False keeps the front panel.
#[test]
fn panel_switch() {
    let mut app = shop_app();
    let l = app.layout;
    let d = app.doc.as_mut().unwrap();
    let objs = &mut d.layouts[l].objects;
    objs.push(fmp12::layouts::LayoutObject {
        id: 200,
        kind: 11,
        ..Default::default()
    });
    for (n, name) in [(201, "One"), (202, "Two")] {
        objs.push(fmp12::layouts::LayoutObject {
            id: n,
            kind: 12,
            parent: Some(200),
            name: name.into(),
            ..Default::default()
        });
    }
    script(
        &mut app,
        90,
        "panels",
        "Get ( TriggerCurrentPanel ) & \"|\" & Get ( TriggerTargetPanel )",
        Some("$$allow"),
    );
    on_object(&mut app, 200, trigger(event::PANEL_SWITCH, 90));
    let g = |app: &App| {
        let g = &app.engine.as_ref().unwrap().globals;
        g.get(&("$$panels".into(), 1)).map(|v| v.to_text())
    };
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    let before = app.tabs.clone();
    app.tabs.insert(200, 202);
    app.panel_switches(&before);
    assert_eq!(g(&app).as_deref(), Some("1\rOne|2\rTwo"));
    assert_eq!(app.tabs.get(&200), None);
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    app.tabs.insert(200, 202);
    app.panel_switches(&before);
    assert_eq!(app.tabs.get(&200), Some(&202));
}

/// OnWindowTransaction after a commit gets FileMaker's JSON; OnViewChange
/// after View as Table.
#[test]
fn window_transaction_and_view_change() {
    let mut app = shop_app();
    script(&mut app, 90, "json", "Get ( ScriptParameter )", None);
    script(&mut app, 91, "view", "$$view + 1", None);
    app.doc.as_mut().unwrap().triggers.file = vec![trigger(event::WINDOW_TRANSACTION, 90)];
    on_layout(&mut app, trigger(event::VIEW_CHANGE, 91));
    let mut h = harness_for(app);
    h.run_steps(3);
    retype(&mut h, 2, "S");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert!(h.state_mut().commit());
    h.run_steps(3);
    let id = {
        let db = &h.state().doc.as_ref().unwrap().db;
        db.tables
            .iter()
            .find(|t| t.name == "Invoices")
            .unwrap()
            .records[0]
            .id
    };
    assert_eq!(
        global(&h, "$$json"),
        format!("{{\"Shop\":{{\"Invoices\":[[\"Modified\",{id},\"\"]]}}}}")
    );
    h.state_mut().view = View::Form;
    h.get_by_label(tr("Table")).click();
    h.run_steps(3);
    assert_eq!(global(&h, "$$view"), "1");
}

/// OnRecordCommit runs before the commit, the record still open: False
/// keeps it open and uncommitted (no OnWindowTransaction), True commits.
#[test]
fn record_commit_runs_before_and_cancels() {
    let mut app = shop_app();
    script(
        &mut app,
        90,
        "open",
        "$$open & \"[\" & $$tx & \"]\" & Invoices::Size & Get ( RecordOpenState )",
        Some("$$allow"),
    );
    script(&mut app, 91, "tx", "$$tx + 1", None);
    on_layout(&mut app, trigger(event::RECORD_COMMIT, 90));
    app.doc.as_mut().unwrap().triggers.file = vec![trigger(event::WINDOW_TRANSACTION, 91)];
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    let mut h = harness_for(app);
    h.run_steps(3);
    retype(&mut h, 2, "S");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert!(h.state().editing.is_some());
    assert!(!h.state_mut().commit());
    h.run_steps(3);
    assert!(
        h.state().editing.is_some(),
        "a False result keeps the record open"
    );
    assert_eq!(global(&h, "$$tx"), "", "nothing was committed");
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    assert!(h.state_mut().commit());
    h.run_steps(3);
    assert!(h.state().editing.is_none());
    assert_eq!(global(&h, "$$tx"), "1");
    // Both runs saw the record open (2, modified) with its new value,
    // before any transaction.
    assert_eq!(global(&h, "$$open"), "[]S2[]S2");
}

/// OnObjectExit returning False keeps the focus in the field (no other
/// field is entered); True lets it go.
#[test]
fn object_exit_cancels() {
    let mut app = shop_app();
    script(&mut app, 90, "exited", "$$exited + 1", Some("$$allow"));
    on_object(&mut app, 100, trigger(event::OBJECT_EXIT, 90));
    app.engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(0));
    let mut h = harness_for(app);
    h.run_steps(3);
    click_input(&mut h, 0);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(global(&h, "$$exited"), "1");
    assert_eq!(h.state().m8.focus, Some(100), "no other field entered");
    assert!(in_field(&h), "the field keeps the focus");
    h.state_mut()
        .engine
        .as_mut()
        .unwrap()
        .globals
        .insert(("$$allow".into(), 1), calc::Value::int(1));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(global(&h, "$$exited"), "2");
    assert_ne!(h.state().m8.focus, Some(100));
}

/// Script steps fire triggers inside the run: OnModeExit's False cancels
/// Enter Find Mode (error 20 for the next step), OnViewChange runs after
/// View As and before the next step; nothing is left queued.
#[test]
fn script_steps_fire_triggers() {
    let mut app = shop_app();
    script(&mut app, 90, "left", "$$left + 1", Some("0"));
    script(&mut app, 91, "view", "$$view + 1", None);
    on_layout(&mut app, trigger(event::MODE_EXIT, 90));
    on_layout(&mut app, trigger(event::VIEW_CHANGE, 91));
    let mut find = Step::new(id::ENTER_FIND_MODE);
    find.params = StepParams::Find {
        pause: false,
        restore: false,
        requests: vec![],
    };
    // View As [ Table ] (step 30).
    let mut view = Step::new(30);
    view.arg = 3;
    let set = |app: &App, var: &str, src: &str| {
        let mut s = Step::new(id::SET_VARIABLE);
        s.params = StepParams::SetVariable {
            name: var.into(),
            value: Some(calc(app, src)),
            repetition: None,
        };
        s
    };
    let steps = vec![
        find,
        set(&app, "$$err", "Get ( LastError )"),
        view,
        set(&app, "$$after", "$$view"),
    ];
    app.engine.as_mut().unwrap().scripts.push(Script {
        id: 95,
        name: "Main".into(),
        options: 8,
        steps,
    });
    app.trigger_queue.clear();
    app.start_run(false, |e, h| e.start(h, 95, None));
    assert_eq!(app.mode(), Mode::Browse);
    let g = |app: &App, v: &str| app.engine.as_ref().unwrap().globals[&(v.into(), 1)].to_text();
    assert_eq!(g(&app, "$$left"), "1");
    assert_eq!(g(&app, "$$err"), "20");
    assert_eq!(app.view, View::Table);
    assert_eq!(
        g(&app, "$$after"),
        "1",
        "OnViewChange ran before the next step"
    );
    assert!(app.trigger_queue.is_empty());
}

/// Get ( RecordModificationCount ): a commit of an existing record counts,
/// the commit that creates a record does not.
#[test]
fn record_modification_count() {
    let mut app = shop_app();
    let t = app.table().unwrap();
    let count = |app: &App, r: usize| {
        app.doc.as_ref().unwrap().db.table(t).unwrap().records[r].modifications
    };
    app.editing = Some((t, 0));
    assert!(app.commit_record());
    assert_eq!(count(&app, 0), 1);
    app.new_record();
    let r = app.current_record().unwrap();
    app.editing = Some((t, r));
    assert!(app.commit_record());
    assert_eq!(count(&app, r), 0);
}
