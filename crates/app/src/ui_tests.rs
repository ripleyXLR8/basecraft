//! The app driven through `egui_kittest`: clicks and keys go through egui's
//! input like a user's. Set `BASECRAFT_SHOTS` to a folder to also render
//! each step to a PNG there (needs a wgpu adapter).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use fmp12::scripts::{Step, StepParams, id};

fn harness(name: &str) -> Option<Harness<'static, App>> {
    let mut app = tests::template(name)?;
    app.doc.as_mut()?.path = None;
    Some(harness_for(app))
}

fn harness_for(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size([1060.0, 900.0])
        .with_accessibility_check(false)
        // eframe's native fallback for the glyphs egui's fonts lack (arrows).
        .with_context_setup(|ctx| {
            ctx.add_font_provider(std::sync::Arc::new(
                egui_system_font_provider::SystemFontProvider::new(),
            ));
        })
        .build_eframe(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            app
        })
}

/// Renders the window to `$BASECRAFT_SHOTS/<name>.png`.
fn shot(h: &mut Harness<'_, App>, name: &str) {
    let Some(dir) = std::env::var_os("BASECRAFT_SHOTS") else {
        return;
    };
    // One render at a time: parallel wgpu devices crashed the driver here.
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let img = h.render().unwrap();
    img.save(std::path::Path::new(&dir).join(format!("{name}.png")))
        .unwrap();
}

#[test]
fn script_workspace_flow() {
    let Some(mut h) = harness("Tasks") else {
        return;
    };
    h.run_steps(3);
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::S,
    );
    h.run_steps(3);
    assert!(h.state().workspace.open);
    shot(&mut h, "10_ws_open");

    // Pick a script and one of its steps.
    h.get_by_label("Sort by Name").click();
    h.run_steps(2);
    h.get_by_label_contains("Sort Records").click();
    h.run_steps(2);
    shot(&mut h, "11_ws_step_selected");

    // Add Set Variable from the palette, after the selected step.
    h.get_by_label("Set Variable").click();
    h.run_steps(2);
    shot(&mut h, "12_ws_step_added");
    let steps = |h: &Harness<'_, App>| -> Vec<Step> {
        let ws = &h.state().workspace;
        ws.scripts
            .iter()
            .find(|s| s.name == "Sort by Name")
            .unwrap()
            .steps
            .clone()
    };
    assert_eq!(steps(&h).len(), 2);
    assert_eq!(steps(&h)[1].id, id::SET_VARIABLE);

    // Edit its parameters: the name, then the value (compiled on focus loss).
    h.get_by_label(tr("Name:")).focus();
    h.run_steps(1);
    h.get_by_label(tr("Name:")).type_text("$count");
    h.run_steps(1);
    h.get_by_label(tr("Value:")).focus();
    h.run_steps(1);
    h.get_by_label(tr("Value:"))
        .type_text("Get ( FoundCount ) + 1");
    h.run_steps(1);
    h.get_by_label(tr("Name:")).focus();
    h.run_steps(2);
    shot(&mut h, "13_ws_params_edited");
    let StepParams::SetVariable { name, value, .. } = &steps(&h)[1].params else {
        panic!("not Set Variable")
    };
    assert_eq!(name, "$count");
    assert!(value.is_some(), "value not compiled");

    // Move it up, then disable it.
    // (the status toolbar has a sort ↑ too)
    h.get_all_by_label("\u{2191}")
        .find(|n| n.rect().top() > 90.0)
        .unwrap()
        .click();
    h.run_steps(2);
    assert_eq!(steps(&h)[0].id, id::SET_VARIABLE);
    h.get_by_label(tr("Disable/Enable")).click();
    h.run_steps(2);
    assert!(!steps(&h)[0].enabled);
    shot(&mut h, "14_ws_moved_disabled");

    // A new script with a Beep, run, then Save.
    h.get_by_label(tr("New Script")).click();
    h.run_steps(2);
    shot(&mut h, "15_ws_new_script_rename");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    // Beep is in a closed category: search for it.
    palette_search(&h).focus();
    h.run_steps(1);
    palette_search(&h).type_text("beep");
    h.run_steps(2);
    shot(&mut h, "15b_ws_palette_search");
    h.get_by_label("Beep").click();
    h.run_steps(2);
    h.get_by_label(tr("Run")).click();
    h.run_steps(3);
    shot(&mut h, "16_ws_ran");
    let app = h.state();
    assert!(!app.workspace.dirty, "Run saves first");
    assert!(
        app.engine
            .as_ref()
            .unwrap()
            .scripts
            .iter()
            .any(|s| s.name == tr("New Script") && s.steps.len() == 1)
    );
    assert!(app.dirty, "saved scripts make the file dirty");

    // File › Save (Ctrl+S) to a copy: the script list changed, so the app
    // warns that FileMaker will report the file as damaged.
    let out = std::env::temp_dir().join("basecraft_ui_ws_save.fmp12");
    h.state_mut().doc.as_mut().unwrap().path = Some(out.clone());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(3);
    shot(&mut h, "17_saved_warning");
    let app = h.state();
    assert!(!app.dirty);
    assert!(
        app.messages.iter().any(|m| m.1 == tr("Saved. Scripts were added, renamed or deleted: BaseCraft opens this file, but FileMaker Pro will report it as damaged.")),
        "no warning: {:?}",
        app.messages
    );
    let back = fmp12::scripts::scripts(&fmp12::File::open(&out).unwrap()).unwrap();
    assert!(back.iter().any(|s| s.name == tr("New Script")));
    let _ = std::fs::remove_file(out);
}

#[test]
fn contacts_opens_sorted() {
    let Some(mut h) = harness("Contacts") else {
        return;
    };
    h.run_steps(4);
    shot(&mut h, "01_contacts_open");
    let app = h.state_mut();
    let t = app.table().unwrap();
    assert_eq!(app.sorted.get(&t), Some(&true));
    h.get_by_label_contains(tr("Sorted"));
}

/// The search field of the step palette (right column of the workspace).
fn palette_search<'a>(h: &'a Harness<'_, App>) -> egui_kittest::Node<'a> {
    h.get_by_label(tr("Steps"))
}

/// Adds a script built from calculation sources to the engine.
fn add_script(h: &mut Harness<'_, App>, id: u32, name: &str, steps: &[(u16, &str, &str)]) {
    let app = h.state_mut();
    let d = app.doc.as_ref().unwrap();
    let names = script::text::Names {
        db: &d.db,
        scripts: &[],
    };
    let calc = |s: &str| (!s.is_empty()).then(|| names.compile(s).unwrap());
    let steps = steps
        .iter()
        .map(|(sid, a, b)| {
            let mut st = Step::new(*sid);
            match &mut st.params {
                StepParams::SetVariable { name, value, .. } => {
                    *name = a.to_string();
                    *value = calc(b);
                }
                StepParams::Pause(d) => *d = calc(a),
                StepParams::ShowCustomDialog {
                    title,
                    message,
                    buttons,
                    ..
                } => {
                    *title = calc(a);
                    *message = calc(b);
                    *buttons = ["\"OK\"", "\"Cancel\""]
                        .iter()
                        .map(|l| fmp12::scripts::DialogButton {
                            label: calc(l),
                            commit: true,
                        })
                        .collect();
                }
                _ => {}
            }
            st
        })
        .collect();
    app.engine
        .as_mut()
        .unwrap()
        .scripts
        .push(fmp12::scripts::Script {
            id,
            name: name.into(),
            options: 8,
            steps,
        });
}

fn global(h: &Harness<'_, App>, name: &str) -> Option<String> {
    let g = &h.state().engine.as_ref()?.globals;
    g.get(&(name.into(), 1)).map(|v| v.to_text())
}

/// Scripts menu › script: a custom dialog waits for a click, then a pause
/// waits for Continue, then a timed pause ends by itself.
#[test]
fn dialog_and_pause_wait_for_the_user() {
    let Some(mut h) = harness("Tasks") else {
        return;
    };
    h.run_steps(3);
    add_script(
        &mut h,
        900,
        "Ask and wait",
        &[
            (
                id::SHOW_CUSTOM_DIALOG,
                "\"Question\"",
                "\"Continue the test?\"",
            ),
            (id::SET_VARIABLE, "$$choice", "Get ( LastMessageChoice )"),
            (id::PAUSE_RESUME, "", ""),
            (id::SET_VARIABLE, "$$after", "1"),
            (id::PAUSE_RESUME, "1", ""),
            (id::SET_VARIABLE, "$$timed", "1"),
        ],
    );
    h.get_by_label(tr("Scripts")).click();
    h.run_steps(2);
    shot(&mut h, "20_scripts_menu");
    h.get_by_label("Ask and wait").click();
    h.run_steps(3);
    shot(&mut h, "21_custom_dialog");
    assert!(h.state().active.is_some(), "dialog does not wait");
    assert_eq!(global(&h, "$$choice"), None);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(global(&h, "$$choice").as_deref(), Some("2"));
    assert!(matches!(
        h.state().active.as_ref().map(|a| &a.wait),
        Some(script::Suspend::Pause(None))
    ));
    shot(&mut h, "22_paused");
    h.get_by_label(tr("Continue")).click();
    h.run_steps(3);
    assert_eq!(global(&h, "$$after").as_deref(), Some("1"));
    // The timed pause (1 s) ends by itself.
    assert_eq!(global(&h, "$$timed"), None);
    for _ in 0..40 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(global(&h, "$$timed").as_deref(), Some("1"));
    assert!(h.state().active.is_none());
}
/// New records, Tools › Data Viewer watching them, then Preview.
#[test]
fn data_viewer_and_preview() {
    let Some(mut h) = harness("Tasks") else {
        return;
    };
    h.run_steps(3);
    for _ in 0..3 {
        h.get_by_label(tr("New Record")).click();
        h.run_steps(2);
    }
    // Type a task on the current (last) record.
    let task = |h: &Harness<'_, App>| {
        h.get_all_by_role(egui::accesskit::Role::TextInput)
            .find(|n| n.rect().left() < 100.0)
            .unwrap()
            .rect()
            .center()
    };
    let at = task(&h);
    h.hover_at(at);
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    h.event(egui::Event::Text("Write the report".into()));
    h.run_steps(2);
    h.get_by_label(tr("Tools")).click();
    h.run_steps(2);
    h.get_by_label(tr("Data Viewer")).click();
    h.run_steps(4);
    for expr in [
        "Get ( FoundCount )",
        "Get ( RecordNumber )",
        "Upper ( \"abc\" ) & ¶ & 2",
    ] {
        watch_input(&h).focus();
        h.run_steps(1);
        watch_input(&h).type_text(expr);
        h.run_steps(1);
        h.get_by_label(tr("Add")).click();
        h.run_steps(2);
    }
    h.run_steps(4);
    shot(&mut h, "30_data_viewer");
    assert_eq!(h.state().watches.len(), 3);
    // Get ( FoundCount ) and Get ( RecordNumber ): the window's 3 records.
    let dv = data_viewer(&h);
    assert_eq!(dv.get_all_by_label("3").count(), 2);
    h.get_by_label_contains("3 / 3");
    h.get_by_label("ABC\u{b6}2");

    h.get_by_label(tr("Preview")).click();
    h.run_steps(3);
    shot(&mut h, "31_preview");
    assert_eq!(h.state().mode(), Mode::Preview);
    h.get_by_label_contains(&format!("{} 1/", tr("Page")));
}
/// The expression field of the Data Viewer (the window opens on the right).
fn watch_input<'a>(h: &'a Harness<'_, App>) -> egui_kittest::Node<'a> {
    data_viewer(h).get_by_role(egui::accesskit::Role::TextInput)
}

fn data_viewer<'a>(h: &'a Harness<'_, App>) -> egui_kittest::Node<'a> {
    h.get_all_by_role(egui::accesskit::Role::Window)
        .find(|w| w.accesskit_node().label().as_deref() == Some(tr("Data Viewer")))
        .unwrap()
}

/// Clients (Id, Name) and Invoices (ClientId, ClientName looked up from
/// Clients::Name, Size from the value list S/M/L with a custom message),
/// built with the schema editor; the Invoices layout shows its three
/// fields side by side. Opened on that layout.
fn shop_app() -> App {
    use fmp12::schema::{DataType, Lookup, NoMatch};
    use fmp12::schema_edit::{FieldSpec, RelationshipSpec};
    use fmp12::valuelists::Source;
    let mut file =
        fmp12::File::from_bytes(fmp12::schema_edit::new_file("Clients").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let c = db.tables[0].id;
    let num = |n: &str| FieldSpec::new(n, DataType::Number);
    let id = db.add_field(&mut file, c, &num("Id")).unwrap();
    let name = FieldSpec::new("Name", DataType::Text);
    let name = db.add_field(&mut file, c, &name).unwrap();
    let i = db.add_table(&mut file, "Invoices").unwrap();
    let cid = db.add_field(&mut file, i, &num("ClientId")).unwrap();
    let to = |db: &Database, n: &str| db.occurrences.iter().find(|o| o.name == n).unwrap().id;
    let (cto, ito) = (to(&db, "Clients"), to(&db, "Invoices"));
    db.add_relationship(&mut file, &RelationshipSpec::equal(cto, id, ito, cid))
        .unwrap();
    let mut l = FieldSpec::new("ClientName", DataType::Text);
    l.auto_enter.lookup = Some(Lookup {
        context: ito,
        from: cto,
        field: name,
        no_match: NoMatch::DontCopy,
        copy_empty: false,
    });
    let cname = db.add_field(&mut file, i, &l).unwrap();
    let sizes = Source::Custom("S\rM\rL".into());
    let sizes = db.add_value_list(&mut file, "Sizes", &sizes).unwrap();
    let mut size = FieldSpec::new("Size", DataType::Text);
    size.validation.value_list = Some(sizes);
    size.validation.message = Some("\"Pick S, M or L\"".into());
    let size = db.add_field(&mut file, i, &size).unwrap();
    for (k, n) in [("1", "Ada"), ("2", "Bob")] {
        let r = db.new_record(c).unwrap();
        db.set_values(c, r, &[(id, k.into()), (name, n.into())]);
    }
    let r = db.new_record(i).unwrap();
    db.set_values(i, r, &[(cid, "1".into()), (size, "M".into())]);
    let file = fmp12::File::from_bytes(db.save(&file).unwrap()).unwrap();
    let mut layouts = fmp12::layouts::layouts(&file).unwrap();
    let lay = layouts.iter_mut().find(|l| l.name == "Invoices").unwrap();
    let top = lay.parts.iter().find(|p| p.kind == 4).unwrap().top + 10.0;
    for (n, f) in [cid, cname, size].into_iter().enumerate() {
        let left = 10.0 + 130.0 * n as f64;
        lay.objects.push(fmp12::layouts::LayoutObject {
            id: 100 + n as u64,
            kind: 1,
            bounds: fmp12::layouts::Bounds {
                top,
                left,
                bottom: top + 22.0,
                right: left + 120.0,
            },
            field: Some((0x104000 | ito as u64, f as u64, 1)),
            ..Default::default()
        });
    }
    let bytes = fmp12::save::save_layouts(&file, &layouts).unwrap();
    let mut app = App::default();
    app.open("Shop".into(), bytes);
    let d = app.doc.as_ref().unwrap();
    app.layout = d.layouts.iter().position(|l| l.name == "Invoices").unwrap();
    app
}

/// Clicks the `n`-th text input from the left, selects its text and types `text`.
fn retype(h: &mut Harness<'_, App>, n: usize, text: &str) {
    click_input(h, n);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
}

/// Clicks the `n`-th text input from the left.
fn click_input(h: &mut Harness<'_, App>, n: usize) {
    let mut inputs: Vec<egui::Pos2> = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|i| i.rect().center())
        .collect();
    inputs.sort_by(|a, b| a.x.total_cmp(&b.x));
    let at = inputs[n];
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_steps(2);
}

/// Typing a match field and leaving it runs the lookup; committing an
/// invalid value shows the field's custom validation message.
#[test]
fn field_commit_runs_lookups_and_validation() {
    let mut h = harness_for(shop_app());
    h.run_steps(3);
    let looked_up = |h: &Harness<'_, App>| {
        let db = &h.state().doc.as_ref().unwrap().db;
        let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
        let f = t.field_by_name("ClientName").unwrap().id;
        db.value(t.id, 0, f).to_text()
    };
    assert_eq!(looked_up(&h), "Ada");
    // ClientId 1 → 2: Bob once the field is left.
    retype(&mut h, 0, "2");
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(looked_up(&h), "Bob");
    shot(&mut h, "40_lookup");
    // Leaving the field again without a change does not look up.
    let db = &mut h.state_mut().doc.as_mut().unwrap().db;
    let t = db.tables.iter_mut().find(|t| t.name == "Invoices").unwrap();
    let f = t.field_by_name("ClientName").unwrap().id;
    t.records[0].values.insert(f, "kept".into());
    click_input(&mut h, 0);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(looked_up(&h), "kept");
    // Size XL is not in the value list: committing (New Record) fails
    // with the custom message, and no record is added.
    retype(&mut h, 2, "XL");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    h.get_by_label(tr("New Record")).click();
    h.run_steps(3);
    shot(&mut h, "41_validation");
    let alert = h.state().alert.clone();
    assert_eq!(alert, Some(("Pick S, M or L".to_string(), true)));
    h.get_by_label("Pick S, M or L");
    let db = &h.state().doc.as_ref().unwrap().db;
    let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
    assert_eq!(t.records.len(), 1);
}
