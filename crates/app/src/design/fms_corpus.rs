//! Files for FileMaker Server's consistency check and XML export
//! (`tools/oracle/fms_batch.sh`, `fms_check.sh`), written by File › Save's
//! own path ([`crate::saved_bytes`]) after edits made the way the app
//! makes them (Designer, Layout Setup, Manage Database, data entry).
//!
//! `BASECRAFT_FMS_CORPUS=<dir> cargo test -p basecraft fms_corpus -- --nocapture`
//! writes `<dir>/<case>.fmp12` and prints what each case intends. Needs the
//! FileMaker Pro starter templates (skipped without them or the variable).

#![allow(clippy::unwrap_used, clippy::panic)]

use super::{Designer, LayoutOp, kind, new_style, run_layout_op};
use crate::App;
use fmp12::layouts::{Bounds, LayoutObject, field_flags, setup};
use fmp12::schema::{DataType, rep_key};
use fmp12::schema_edit::FieldSpec;
use fmp12::scripts::{Script, Step, id};
use fmp12::triggers::{Trigger, event, mode};

const RED_PNG: &str = "89504e470d0a1a0a0000000d4948445200000004000000030806000000b4f4aec6000000017352474200aece1ce90000000467414d410000b18f0bfc6105000000097048597300000ec300000ec301c76fa8640000001549444154185763f80f030c0c60cc80290065c0300090cc1fe1a78d04800000000049454e44ae426082";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn app(name: &str) -> App {
    let mut app = crate::tests::template(name).unwrap();
    app.trigger_queue.clear();
    app
}

/// Adds Beep scripts named `BC Script n` until the last id is at least
/// `upto`; returns the new ids.
fn add_scripts(app: &mut App, upto: u32) -> Vec<u32> {
    let e = app.engine.as_mut().unwrap();
    let mut ids = Vec::new();
    let mut next = e.scripts.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    while next <= upto {
        e.scripts.push(Script {
            id: next,
            name: format!("BC Script {next}"),
            options: 8,
            steps: vec![Step::new(id::BEEP)],
        });
        ids.push(next);
        next += 1;
    }
    ids
}

fn layout_index(app: &App, name: &str) -> usize {
    let d = app.doc.as_ref().unwrap();
    d.layouts
        .iter()
        .position(|l| l.name == name)
        .unwrap_or_else(|| panic!("no layout {name}"))
}

fn write(dir: &str, case: &str, app: &mut App, intent: &str) {
    if let (Some(ds), Some(d)) = (app.designer.as_mut(), app.doc.as_mut()) {
        ds.apply(d);
    }
    let scripts = app.engine.as_ref().map(|e| e.scripts.clone());
    let d = app.doc.as_ref().unwrap();
    let bytes = crate::saved_bytes(d, scripts.as_deref()).unwrap();
    // What the app reads back must be what it meant to write.
    fmp12::File::from_bytes(bytes.clone()).unwrap();
    std::fs::write(format!("{dir}/{case}.fmp12"), bytes).unwrap();
    println!("{case}: {intent}");
}

fn rect(top: f64, left: f64, h: f64, w: f64) -> Bounds {
    Bounds {
        top,
        left,
        bottom: top + h,
        right: left + w,
    }
}

fn designer(app: &mut App, layout: &str) {
    let i = layout_index(app, layout);
    app.layout = i;
    app.designer = Some(Designer::new(app.doc.as_ref().unwrap(), i));
}

fn ds(app: &mut App) -> &mut Designer {
    app.designer.as_mut().unwrap()
}

#[test]
fn fms_corpus() {
    let Ok(dir) = std::env::var("BASECRAFT_FMS_CORPUS") else {
        return;
    };
    if crate::tests::template("Tasks").is_none() {
        return;
    }
    let only = std::env::var("BASECRAFT_FMS_CASES").unwrap_or_default();
    let want = |c: &str| only.is_empty() || only.split(',').any(|x| x == c);

    // Global fields: new globals set by data entry, short, long, repeating.
    for (case, long) in [("glob-short", false), ("glob-long", true)] {
        if !want(case) {
            continue;
        }
        let mut a = app("Tasks");
        let d = a.doc.as_mut().unwrap();
        let t = d.db.tables[0].id;
        let mut g = FieldSpec::new("gBC Text", DataType::Text);
        g.storage.global = true;
        let gt = d.db.add_field(&mut d.file, t, &g).unwrap();
        let mut n = FieldSpec::new("gBC Num", DataType::Number);
        n.storage.global = true;
        n.storage.repetitions = 3;
        let gn = d.db.add_field(&mut d.file, t, &n).unwrap();
        let text = if long {
            "Long global ".repeat(400)
        } else {
            "hello".into()
        };
        d.db.set_values(
            t,
            0,
            &[
                (gt, text.clone()),
                (rep_key(gn, 1), "42".into()),
                (rep_key(gn, 3), "7".into()),
            ],
        );
        let intent = format!(
            "table {t}: gBC Text ({gt}) = {} chars, gBC Num ({gn}) reps 1 = 42, 3 = 7",
            text.len()
        );
        write(&dir, case, &mut a, &intent);
    }
    if want("glob-clear") {
        // A global saved, then emptied and saved again: the node goes.
        let mut a = app("Tasks");
        let d = a.doc.as_mut().unwrap();
        let t = d.db.tables[0].id;
        let mut g = FieldSpec::new("gBC Text", DataType::Text);
        g.storage.global = true;
        let gt = d.db.add_field(&mut d.file, t, &g).unwrap();
        d.db.set_values(t, 0, &[(gt, "x".into())]);
        let b = crate::saved_bytes(d, None).unwrap();
        d.file = fmp12::File::from_bytes(b).unwrap();
        d.db.set_values(t, 0, &[(gt, String::new())]);
        write(
            &dir,
            "glob-clear",
            &mut a,
            "gBC Text set then cleared: no [T].[8]",
        );
    }

    // Layout Setup: every setting of the General, Views and Printing tabs.
    if want("setup-all") {
        let mut a = app("Contacts");
        designer(&mut a, "Contact Details");
        let l = &mut ds(&mut a).layout;
        let mut edited = l.clone();
        edited.options |= setup::DELINEATE
            | setup::NO_LIST
            | setup::NO_AUTO_SAVE
            | setup::FIELD_FRAMES
            | setup::FIXED_MARGINS
            | setup::FACING_PAGES
            | setup::COLUMNS
            | setup::NO_QUICK_FIND
            | setup::NO_RECORD_INDICATOR;
        edited.flags |= setup::HIDDEN;
        edited.menu_set = 1;
        edited.columns = 3;
        edited.margins = [20.0, 30.0, 40.0, 50.0];
        super::super::layout_setup::apply(l, &edited);
        let intent = format!(
            "Contact Details: options {:#x}, flags {:#x}, menu set 1, 3 columns, margins 20/30/40/50",
            edited.options, edited.flags
        );
        write(&dir, "setup-all", &mut a, &intent);
    }

    // Layout triggers naming scripts from id 128 on (two-byte ids), a new
    // trigger next to a kept one, and a layout without triggers.
    if want("setup-trig") {
        let mut a = app("Contacts");
        let ids = add_scripts(&mut a, 131);
        let (s130, s131) = (ids[ids.len() - 2], ids[ids.len() - 1]);
        designer(&mut a, "Contact List");
        let mut ts = ds(&mut a).layout.triggers.clone();
        let kept = ts.first().map(|t| (t.event, t.script));
        if let Some(t) = ts.first_mut() {
            t.script = s130 - 1;
        }
        ts.push(Trigger {
            event: event::RECORD_COMMIT,
            script: s131,
            modes: mode::BROWSE | mode::FIND,
            parameter: None,
        });
        ds(&mut a).layout.triggers = ts;
        a.designer.as_mut().unwrap().apply(a.doc.as_mut().unwrap());
        designer(&mut a, "Contact Details");
        ds(&mut a).layout.triggers = vec![Trigger {
            event: event::LAYOUT_ENTER,
            script: s130,
            modes: mode::BROWSE,
            parameter: None,
        }];
        let intent = format!(
            "Contact List: {kept:?} retargeted to {} + OnRecordCommit -> {s131} (Browse, Find); Contact Details: OnLayoutEnter -> {s130} (Browse)",
            s130 - 1
        );
        write(&dir, "setup-trig", &mut a, &intent);
    }

    // Field object options: auto-complete, Quick Find off, concealed, and
    // one field with every option cleared.
    if want("fflags") {
        let mut a = app("Contacts");
        designer(&mut a, "Contact Details");
        let fields: Vec<(u64, u64)> = ds(&mut a)
            .layout
            .objects
            .iter()
            .filter(|o| o.kind == kind::FIELD && o.display == 0)
            .map(|o| (o.id, o.field_flags))
            .take(4)
            .collect();
        assert_eq!(fields.len(), 4);
        let sets = [
            field_flags::AUTO_COMPLETE,
            field_flags::NO_QUICK_FIND,
            field_flags::CONCEALED,
        ];
        let mut intent = String::new();
        for (i, (oid, f)) in fields.iter().enumerate() {
            let new = match sets.get(i) {
                Some(b) => f | b,
                None => 0,
            };
            ds(&mut a).object_mut(*oid).unwrap().field_flags = new;
            intent += &format!("object {oid}: {f:#x} -> {new:#x}; ");
        }
        write(&dir, "fflags", &mut a, &intent);
    }

    // Buttons: new ones performing a script and going to a layout with ids
    // from 128 on, and an existing button retargeted.
    if want("buttons") {
        let mut a = app("Contacts");
        let ids = add_scripts(&mut a, 140);
        let i = layout_index(&a, "Contact Details");
        let to = (a.doc.as_ref().unwrap().layouts[i].table_occurrence & 0xFFF) as u32;
        let mut target = 0;
        let mut n = 0;
        while target < 130 {
            n += 1;
            let op = LayoutOp::New {
                name: format!("BC Layout {n}"),
                to,
            };
            let d = a.doc.as_mut().unwrap();
            let li = run_layout_op(d, i, &op).unwrap();
            target = d.layouts[li].id;
        }
        designer(&mut a, "Contact Details");
        let (s1, s2) = (ids[ids.len() - 1], ids[ids.len() - 2]);
        // A button of the template (Back to Contacts: Go to Layout).
        let old = ds(&mut a)
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::BUTTON && o.button.is_some())
            .map(|o| o.id);
        ds(&mut a).set_button(rect(10.0, 400.0, 24.0, 120.0), None, "BC Script", true, s1);
        ds(&mut a).set_button(
            rect(40.0, 400.0, 24.0, 120.0),
            None,
            "BC Layout",
            false,
            target,
        );
        if let Some(o) = old {
            let r = ds(&mut a).object(o).unwrap().bounds.clone();
            ds(&mut a).set_button(r, Some(o), "Retargeted", true, s2);
        }
        let intent = format!(
            "Contact Details: new button Perform Script [{s1}], new button Go to Layout [{target}], button {old:?} -> Perform Script [{s2}] label Retargeted"
        );
        write(&dir, "buttons", &mut a, &intent);
    }

    // Objects: a renamed object, a merge-field text edited, a new text
    // object with a merge field.
    for (case, part) in [
        ("obj-merge", 1),
        ("obj-name", 2),
        ("obj-text", 4),
        ("obj-dup", 8),
        ("objects", 15),
    ] {
        if !want(case) {
            continue;
        }
        let mut a = app("Meetings");
        designer(&mut a, "Meeting Details");
        let l = &a.designer.as_ref().unwrap().layout;
        let merge = l
            .objects
            .iter()
            .find(|o| o.kind == kind::TEXT && o.text.as_deref().is_some_and(|t| t.contains("<<")))
            .map(|o| (o.id, o.text.clone().unwrap_or_default()))
            .unwrap();
        let named = l
            .objects
            .iter()
            .find(|o| o.kind == kind::FIELD)
            .map(|o| (o.id, o.name.clone()))
            .unwrap();
        let to_name = a
            .doc
            .as_ref()
            .unwrap()
            .db
            .occurrences
            .iter()
            .find(|o| o.id as u64 == (l.table_occurrence & 0xFFF))
            .map(|o| o.name.clone())
            .unwrap();
        let edited = format!("{} (BC)", merge.1);
        if part & 1 != 0 {
            ds(&mut a).object_mut(merge.0).unwrap().text = Some(edited.clone());
        }
        if part & 2 != 0 {
            ds(&mut a).object_mut(named.0).unwrap().name = "BC Named".into();
        }
        let text = format!("BC <<{to_name}::Topic>>");
        let mut nid = 0;
        if part & 4 != 0 {
            nid = ds(&mut a).add(LayoutObject {
                kind: kind::TEXT,
                style_raw: new_style(70),
                text: Some(text.clone()),
                bounds: rect(5.0, 300.0, 20.0, 200.0),
                ..Default::default()
            });
            ds(&mut a).place_inside(nid);
        }
        if part & 8 != 0 {
            // Edit › Duplicate of the text with a stored merge reference.
            ds(&mut a).select([merge.0]);
            ds(&mut a).duplicate();
        }
        let intent = format!(
            "Meeting Details: text {} {:?} -> {edited:?}; object {} name {:?} -> BC Named; new text {nid} {text:?}",
            merge.0, merge.1, named.0, named.1
        );
        write(&dir, case, &mut a, &intent);
    }

    // Container data: a picture put in a record's container field, a
    // larger file in another record.
    if want("containers") {
        let mut a = app("Assets");
        let d = a.doc.as_mut().unwrap();
        let (t, f) =
            d.db.tables
                .iter()
                .find_map(|t| {
                    let f = t
                        .fields
                        .iter()
                        .find(|f| f.data_type == DataType::Container)?;
                    Some((t.id, f.id))
                })
                .unwrap();
        let mut recs = d.db.table(t).unwrap().records.len();
        while recs < 2 {
            d.db.new_record(t).unwrap();
            recs += 1;
        }
        d.db.insert_file(t, 0, f, 1, "red.png", hex(RED_PNG));
        let big: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        d.db.insert_file(t, 1, f, 1, "data.bin", big);
        let intent =
            format!("table {t} field {f}: record 1 red.png (4x3), record 2 data.bin 300000 bytes");
        write(&dir, "containers", &mut a, &intent);
    }
}
