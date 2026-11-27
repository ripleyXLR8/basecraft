//! Runs the FileMaker templates' own scripts (skipped when the templates
//! are not installed, e.g. on CI).

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use calc::Value;
use engine::Database;
use script::{Engine, Host, Window};
use std::path::PathBuf;

fn open(name: &str) -> Option<(Database, Engine)> {
    let p = PathBuf::from(r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates")
        .join(name)
        .join("en.fmp12");
    if !p.is_file() {
        eprintln!("no {name} template, skipping");
        return None;
    }
    let f = fmp12::File::open(p).unwrap();
    Some((Database::load(&f).unwrap(), Engine::load(&f).unwrap()))
}

struct H {
    db: Database,
    win: Window,
}

impl Host for H {
    fn db(&mut self) -> &mut Database {
        &mut self.db
    }
    fn window(&mut self) -> &mut Window {
        &mut self.win
    }
}

/// A host on the first layout of table occurrence `to`.
fn host(db: Database, to: u32) -> H {
    let l = db
        .layouts
        .iter()
        .find(|l| l.table_occurrence & 0xFFF == to as u64)
        .unwrap()
        .id;
    let win = Window::new(&db, l);
    H { db, win }
}

fn add(db: &mut Database, table: u32, values: &[(&str, &str)]) {
    let r = db.new_record(table).unwrap();
    let t = db.tables.iter_mut().find(|t| t.id == table).unwrap();
    for (f, v) in values {
        let id = t.field_by_name(f).unwrap().id;
        t.records[r].values.insert(id, v.to_string());
    }
}

fn column(h: &H, table: u32, field: &str) -> Vec<String> {
    let t = h.db.table(table).unwrap();
    let f = t.field_by_name(field).unwrap().id;
    h.win
        .found
        .iter()
        .map(|&r| h.db.value(table, r, f).to_text())
        .collect()
}

#[test]
fn contacts_sort_by_name() {
    let Some((mut db, mut e)) = open("Contacts") else {
        return;
    };
    for (first, last) in [
        ("Zoe", "Zeta"),
        ("Al", "Alpha"),
        ("Mo", "Moss"),
        ("Bea", "Beta"),
    ] {
        add(&mut db, 130, &[("First Name", first), ("Last Name", last)]);
    }
    let mut h = host(db, 130);
    let before = column(&h, 130, "Initial");
    let o = e.perform_named(&mut h, "Sort by Name", None);
    assert_eq!((o.last_error, o.halted), (0, false));
    let after = column(&h, 130, "Initial");
    let mut want = before.clone();
    want.sort();
    assert_eq!(after, want, "sorted by Initial");
    assert!(h.win.sorted);
    assert_eq!(after.len(), 4);
}

#[test]
fn tasks_sort_by_status_uses_the_value_list_order() {
    let Some((mut db, mut e)) = open("Tasks") else {
        return;
    };
    let order = db.value_list_values(2);
    assert!(order.len() >= 3, "{order:?}");
    for (task, status) in [
        ("a", &order[2]),
        ("b", &order[0]),
        ("c", &order[1]),
        ("d", &order[0]),
    ] {
        add(&mut db, 130, &[("Task", task), ("Status", status)]);
    }
    let mut h = host(db, 130);
    e.perform_named(&mut h, "Sort by Status", None);
    let got = column(&h, 130, "Status");
    assert_eq!(
        got,
        [&order[0], &order[0], &order[1], &order[2]].map(String::as_str)
    );
}

#[test]
fn tasks_create_assignee_entry() {
    let Some((db, mut e)) = open("Tasks") else {
        return;
    };
    let mut h = host(db, 130);
    let before = h.db.table(132).unwrap().records.len();
    // Go to Layout ["Add Assignee"]; New Record/Request; Go to Field [Assignees::First Name]
    let o = e.perform_named(&mut h, "Create Assignee Entry", None);
    assert_eq!(o.last_error, 0);
    let l = h.db.layouts.iter().find(|l| l.id == h.win.layout).unwrap();
    assert_eq!(l.name, "Add Assignee");
    assert_eq!(h.db.table(132).unwrap().records.len(), before + 1);
    // Then Cancel Add Assignee: Delete Record (no dialog) and Close Window
    // (not run by the engine: error 3).
    let o = e.perform_named(&mut h, "Cancel Add Assignee", None);
    assert_eq!(h.db.table(132).unwrap().records.len(), before);
    assert_eq!(o.last_error, 3);
}

#[test]
fn tasks_go_to_entry_field_passes_its_parameter() {
    struct Obj(H, Vec<String>);
    impl Host for Obj {
        fn db(&mut self) -> &mut Database {
            &mut self.0.db
        }
        fn window(&mut self) -> &mut Window {
            &mut self.0.win
        }
        fn go_to_object(&mut self, name: &str, _: u32) -> bool {
            self.1.push(name.to_string());
            true
        }
    }
    let Some((db, mut e)) = open("Tasks") else {
        return;
    };
    let mut h = Obj(host(db, 130), Vec::new());
    // Go to Object [Get ( ScriptParameter )]; Select All (not run: error 3).
    e.perform_named(&mut h, "Go to Entry Field", Some(Value::text("Due Date")));
    assert_eq!(h.1, ["Due Date"]);
}

#[test]
fn step_text_of_template_scripts() {
    let Some((db, e)) = open("Tasks") else {
        return;
    };
    let n = script::text::Names {
        db: &db,
        scripts: &e.scripts,
    };
    let s = e
        .scripts
        .iter()
        .find(|s| s.name == "Add Related Record")
        .unwrap();
    let lines: Vec<String> = s
        .steps
        .iter()
        .zip(script::text::indents(&s.steps))
        .map(|(st, i)| format!("{}{}", "  ".repeat(i), script::text::step_text(st, &n)))
        .collect();
    let all = lines.join(
        "
",
    );
    assert!(
        all.contains("Set Variable [ $$parentPrimaryKey ; Value: Tasks::PrimaryKey ]"),
        "{all}"
    );
    assert!(all.contains("If [ "), "{all}");
    assert!(all.contains("  Loop"), "{all}");
    // Every calculation in every step text compiles back to the same tokens.
    for s in &e.scripts {
        for st in &s.steps {
            if let fmp12::scripts::StepParams::Condition(Some(c)) = &st.params {
                let src = n.calc(c);
                assert_eq!(n.compile(&src).unwrap(), *c, "{src}");
            }
        }
    }
}
