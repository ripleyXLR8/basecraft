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
    h.get_by_label_contains(step(39)).click();
    h.run_steps(2);
    shot(&mut h, "11_ws_step_selected");

    // Add Set Variable from the palette, after the selected step.
    h.get_by_label(step(id::SET_VARIABLE)).click();
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
    palette_search(&h).type_text(&step(id::BEEP).to_lowercase());
    h.run_steps(2);
    shot(&mut h, "15b_ws_palette_search");
    h.get_by_label(step(id::BEEP)).click();
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

    // File › Save (Ctrl+S) to a copy: the new script is written with the
    // script catalog digest (key 8), so FileMaker opens the file.
    let out = std::env::temp_dir().join("basecraft_ui_ws_save.fmp12");
    h.state_mut().doc.as_mut().unwrap().path = Some(out.clone());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(3);
    shot(&mut h, "17_saved");
    let app = h.state();
    assert!(!app.dirty);
    assert!(app.messages.is_empty(), "{:?}", app.messages);
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

/// A step's name in the UI language.
fn step(id: u16) -> &'static str {
    workspace::step_label(id).unwrap()
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

/// Values of an Invoices field over the found set.
fn invoices(h: &Harness<'_, App>, field: &str) -> Vec<String> {
    let app = h.state();
    let db = &app.doc.as_ref().unwrap().db;
    let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
    let f = t.field_by_name(field).unwrap().id;
    app.found
        .get(&t.id)
        .cloned()
        .unwrap_or_else(|| (0..t.records.len()).collect())
        .iter()
        .map(|&r| db.value(t.id, r, f).to_text())
        .collect()
}

fn menu(h: &mut Harness<'_, App>, top: &'static str, item: &'static str) {
    h.get_by_label(tr(top)).click();
    h.run_steps(2);
    h.get_by_label_contains(records::t(item)).click();
    h.run_steps(3);
}

/// Import (field mapping dialog), Find/Replace, Replace Field Contents,
/// Duplicate, Omit Multiple, Delete (All) with confirmations, Modify Last
/// Find and Export, driven through menus, shortcuts and dialogs.
#[test]
fn records_menu_and_m8_dialogs() {
    let t = records::t;
    let mut h = harness_for(shop_app());
    h.run_steps(3);
    // Import: three invoices; ClientName is looked up while importing.
    h.state_mut().start_import(
        "new.csv".into(),
        b"ClientId,Size\r\n2,S\r\n1,L\r\n2,M\r\n".to_vec(),
    );
    h.run_steps(3);
    shot(&mut h, "m8_01_import_mapping");
    // The header row names the fields: matching names map both columns.
    h.get_by_label(t("Don't import first record (contains field names)"))
        .click();
    h.run_steps(2);
    h.get_by_label("▼").click();
    h.run_steps(2);
    h.get_by_label(t("Matching names")).click();
    h.run_steps(2);
    // The dialog's Import button, below the mapping buttons of the same name.
    h.get_all_by_label(t("Import"))
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap()
        .click();
    h.run_steps(3);
    shot(&mut h, "m8_02_import_summary");
    assert_eq!(invoices(&h, "ClientName"), ["Bob", "Ada", "Bob"]);
    assert_eq!(invoices(&h, "Size"), ["S", "L", "M"]);
    h.get_by_label("OK").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::J);
    h.run_steps(2);
    assert_eq!(invoices(&h, "Size").len(), 4);

    // Find/Replace (Ctrl+Shift+F): Replace All over the found set.
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::F,
    );
    h.run_steps(3);
    h.get_by_label(t("Find what:")).click();
    h.run_steps(1);
    h.get_by_label(t("Find what:")).type_text("bob");
    h.get_by_label(t("Replace with:")).click();
    h.run_steps(1);
    h.get_by_label(t("Replace with:")).type_text("Robert");
    h.run_steps(1);
    h.get_by_label(t("Find Next")).click();
    h.run_steps(2);
    shot(&mut h, "m8_03_find_replace");
    h.get_by_label(t("Replace All")).click();
    h.run_steps(2);
    assert_eq!(
        invoices(&h, "ClientName"),
        ["Ada", "Robert", "Ada", "Robert"]
    );
    h.get_by_label_contains("2 occurrence");
    // Match case: "robert" no longer matches.
    h.get_by_label(t("Match case")).click();
    h.get_by_label(t("Find what:")).click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(t("Find what:")).type_text("robert");
    h.run_steps(1);
    h.get_by_label(t("Find Next")).click();
    h.run_steps(2);
    h.get_by_label(t("No match found."));
    h.state_mut().close_find();
    h.run_steps(2);

    // Replace Field Contents with serial numbers.
    menu(&mut h, "Records", "Replace Field Contents…");
    h.get_by_label(t("Replace with serial numbers:")).click();
    h.run_steps(1);
    h.get_by_label(t("Initial value:")).click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(t("Initial value:")).type_text("10");
    h.get_by_label(t("Increment by:")).click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(t("Increment by:")).type_text("5");
    h.run_steps(2);
    shot(&mut h, "m8_04_replace_field_contents");
    h.get_by_label(t("Replace")).click();
    h.run_steps(2);
    assert_eq!(invoices(&h, "ClientId"), ["10", "15", "20", "25"]);

    // Duplicate (Ctrl+D): the copy follows the original (the record
    // Find Next went to).
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(2);
    assert_eq!(invoices(&h, "ClientId"), ["10", "15", "15", "20", "25"]);
    // Omit Multiple (Ctrl+Shift+T): 2 records from the current one.
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::T,
    );
    h.run_steps(2);
    h.get_by_label(t("Number of records to omit:")).click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(t("Number of records to omit:"))
        .type_text("2");
    h.run_steps(1);
    shot(&mut h, "m8_05_omit_multiple");
    h.get_by_label(t("Omit")).click();
    h.run_steps(2);
    assert_eq!(invoices(&h, "ClientId"), ["10", "15", "25"]);
    // Delete Record (Ctrl+E) asks first; Cancel keeps it.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::E);
    h.run_steps(2);
    shot(&mut h, "m8_06_delete_confirm");
    h.get_by_label(t("Cancel")).click();
    h.run_steps(2);
    assert_eq!(invoices(&h, "ClientId").len(), 3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::E);
    h.run_steps(2);
    h.get_by_label(t("Delete")).click();
    h.run_steps(2);
    assert_eq!(invoices(&h, "ClientId"), ["10", "15"]);

    // Export Records: every field, CSV.
    menu(&mut h, "File", "Export Records…");
    h.get_by_label(t("Move All")).click();
    h.run_steps(2);
    shot(&mut h, "m8_07_export_order");
    let csv = String::from_utf8(h.state_mut().export_now().unwrap()).unwrap();
    assert_eq!(csv, "\"10\",\"Ada\",\"M\"\r\n\"15\",\"Robert\",\"S\"\r\n");

    // Delete All Records: the found set only.
    menu(&mut h, "Records", "Delete All Records…");
    h.get_by_label_contains("2 record");
    h.get_by_label(t("Delete All")).click();
    h.run_steps(2);
    assert!(invoices(&h, "ClientId").is_empty());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::J);
    h.run_steps(2);
    assert_eq!(invoices(&h, "ClientId"), ["20", "15"]);

    // Modify Last Find (Ctrl+R): Find mode with the last requests.
    let size = {
        let db = &h.state().doc.as_ref().unwrap().db;
        let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
        t.field_by_name("Size").unwrap().id
    };
    h.state_mut().requests = vec![([(size, "M".to_string())].into(), false)];
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.run_steps(2);
    assert_eq!(h.state().mode(), Mode::Find);
    assert_eq!(h.state().request.get(&size).map(|s| s.as_str()), Some("M"));
    assert!(!h.state().dialogs_open());
}

/// The 14 charts of `fmp12/tests/fixtures/charts.fmp12` (a BaseCraft-made
/// file whose charts were made in FileMaker Pro 26) in Browse and Preview.
#[test]
fn charts_render() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fmp12/tests/fixtures/charts.fmp12"
    ))
    .unwrap();
    let mut app = App::default();
    app.open("charts".into(), bytes);
    let mut h = harness_for(app);
    h.run_steps(3);
    let key = {
        let doc = h.state().doc.as_ref().unwrap();
        egui::Id::NULL.with(("charts", &doc.name, doc.file.bytes().len()))
    };
    let cached = h
        .ctx
        .data(|d| d.get_temp::<std::sync::Arc<print::Charts>>(key));
    assert_eq!(cached.unwrap().len(), 14);
    shot(&mut h, "ours_charts_browse");
    h.state_mut().mode = Some(Mode::Preview);
    h.run_steps(3);
    shot(&mut h, "ours_charts_preview");
}

/// A mouse drag on the layout canvas, from and to layout coordinates.
fn drag_canvas(h: &mut Harness<'_, App>, from: (f32, f32), to: (f32, f32)) {
    let o = h.state().designer.as_ref().unwrap().canvas_origin;
    let a = o + egui::vec2(from.0, from.1);
    let b = o + egui::vec2(to.0, to.1);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(a));
    h.run_steps(1);
    h.event(button(a, true));
    h.run_steps(1);
    for i in 1..=4 {
        h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    h.event(button(b, false));
    h.run_steps(2);
}

fn designer<'a>(h: &'a Harness<'_, App>) -> &'a design::Designer {
    h.state().designer.as_ref().unwrap()
}

/// Layout mode driven like a user: Ctrl+L, a click on a field, the
/// Inspector tabs, arrow keys, Ctrl+D, undo, the Arrange, Insert and
/// Layouts menus, the tab control tool on the canvas, then "Save" on
/// leaving; the edits reach the file. Screenshots `l01`… with
/// `BASECRAFT_SHOTS`.
#[test]
fn layout_mode_flow() {
    let Some(mut h) = harness("Contacts") else {
        return;
    };
    h.run_steps(4);
    let list = h
        .state()
        .doc
        .as_ref()
        .unwrap()
        .layouts
        .iter()
        .position(|l| l.name == "Contact List")
        .unwrap();
    h.state_mut().layout = list;
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    h.run_steps(3);
    assert!(h.state().designer.is_some(), "Ctrl+L enters Layout mode");
    shot(&mut h, "l01_layout_mode");

    // Click field 9 (Last Name, 366–632 × 112–156) on the canvas.
    drag_canvas(&mut h, (400.0, 130.0), (400.0, 130.0));
    assert_eq!(designer(&h).selection(), [9]);
    let bounds = |h: &Harness<'_, App>, id: u64| {
        let ds = designer(h);
        ds.layout
            .objects
            .iter()
            .find(|o| o.id == id)
            .unwrap()
            .bounds
            .clone()
    };
    // Arrow keys nudge by 1 point.
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(bounds(&h, 9).left, 367.0);
    // Inspector tabs.
    for tab in ["Styles", "Appearance", "Data", "Position"] {
        h.get_by_label(tr(tab)).click();
        h.run_steps(2);
        shot(&mut h, &format!("l02_inspector_{}", tab.to_lowercase()));
    }
    // Ctrl+D duplicates, Ctrl+Z takes it back.
    let n = designer(&h).layout.objects.len();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(2);
    assert_eq!(designer(&h).layout.objects.len(), n + 1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(designer(&h).layout.objects.len(), n);

    // Arrange › Group on fields 8 and 9.
    h.state_mut().designer.as_mut().unwrap().select([8, 9]);
    h.run_steps(1);
    h.get_by_label(tr("Arrange")).click();
    h.run_steps(2);
    shot(&mut h, "l03_arrange_menu");
    h.get_by_label_contains(tr("Group")).click();
    h.run_steps(2);
    let g = designer(&h).selection()[0];
    let kind = |h: &Harness<'_, App>, id: u64| {
        designer(h)
            .layout
            .objects
            .iter()
            .find(|o| o.id == id)
            .unwrap()
            .kind
    };
    assert_eq!(kind(&h, g), 8);

    // Insert › Part…: a footer.
    h.get_by_label(tr("Insert")).click();
    h.run_steps(2);
    h.get_by_label(tr("Part…")).click();
    h.run_steps(6);
    h.run_steps(2);
    h.get_by_label(design::part_name(7)).click();
    h.run_steps(1);
    shot(&mut h, "l04_part_dialog");
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(designer(&h).layout.parts.iter().any(|p| p.kind == 7));

    // The tab control tool, dragged on the canvas, then its setup dialog.
    h.get_by_label("[Tab]").click();
    h.run_steps(1);
    drag_canvas(&mut h, (150.0, 172.0), (450.0, 232.0));
    h.run_steps(4);
    shot(&mut h, "l05_tab_dialog");
    h.get_by_label("OK").click();
    h.run_steps(3);
    let tc = designer(&h).selection()[0];
    assert_eq!(kind(&h, tc), 11);
    shot(&mut h, "l06_tab_control");

    // Leave the layout: "Save" copies the edits into the document.
    h.get_by_label(tr("Exit Layout")).click();
    h.run_steps(6);
    shot(&mut h, "l07_save_question");
    h.get_by_label(tr("Save")).click();
    h.run_steps(3);
    assert!(h.state().designer.is_none());
    let d = h.state().doc.as_ref().unwrap();
    let bytes = fmp12::save::save_layouts(&d.file, &d.db.layouts).unwrap();
    let f = fmp12::File::from_bytes(bytes).unwrap();
    let back = fmp12::layouts::layouts(&f).unwrap();
    let l = back.iter().find(|l| l.name == "Contact List").unwrap();
    assert!(l.objects.iter().any(|o| o.kind == 11));
    assert!(l.objects.iter().any(|o| o.kind == 8));
    assert!(l.parts.iter().any(|p| p.kind == 7));
    shot(&mut h, "l08_browse_after");

    // Layouts › New Layout/Report…: written to the file, then opened.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    h.run_steps(3);
    h.get_by_label(tr("Layouts")).click();
    h.run_steps(2);
    h.get_by_label(tr("New Layout/Report…")).click();
    h.run_steps(1);
    h.run_steps(5);
    shot(&mut h, "l09_new_layout");
    let before = h.state().doc.as_ref().unwrap().layouts.len();
    h.get_by_label("OK").click();
    h.run_steps(3);
    let d = h.state().doc.as_ref().unwrap();
    assert_eq!(d.layouts.len(), before + 1);
    let name = designer(&h).layout.name.clone();
    assert!(
        fmp12::layouts::layouts(&d.file)
            .unwrap()
            .iter()
            .any(|l| l.name == name)
    );
    shot(&mut h, "l10_new_layout_open");
}

/// blank.fmp12 protected: Admin "test123" and "viewer" "test123" in a
/// read-only set (that cannot see the file's layouts with `hide_layouts`).
fn protected_blank(hide_layouts: bool) -> Vec<u8> {
    use fmp12::security::{self, Security};
    let f =
        fmp12::File::from_bytes(include_bytes!("../../fmp12/assets/blank.fmp12").to_vec()).unwrap();
    let s = Security::load(&f).unwrap();
    let admin = s.login("Admin", "").unwrap();
    let mut new = s.clone();
    let mut set = new.sets[2].clone();
    set.name = "Viewers".into();
    set.set_flag(security::flag::EXPORT, false);
    if hide_layouts {
        for l in fmp12::layouts::layouts(&f).unwrap() {
            set.set_item(security::LAYOUTS, Some(l.id), security::NO_ACCESS, 0);
        }
    }
    let id = new.add_set(set);
    new.add_account(security::Account::new("viewer", "test123", id));
    for a in new.accounts.iter_mut().filter(|a| a.name == "Admin") {
        a.set_password("test123");
    }
    security::save(&f, &admin, &new).unwrap()
}

/// A protected file asks for an account; a wrong password gets
/// FileMaker's alert, the right one opens the file with that account's
/// privileges: new records are refused with the privilege alert, and
/// File › Manage › Security… is off.
#[test]
fn login_dialog_and_privileges() {
    let mut app = App::default();
    app.open("Sec".into(), protected_blank(false));
    assert!(app.doc.is_none() && app.security.login.is_some());
    let mut h = harness_for(app);
    h.run_steps(2);
    shot(&mut h, "s01_login");
    h.get_by_label(security_ui::t("Account Name:")).focus();
    h.get_by_label(security_ui::t("Account Name:"))
        .type_text("viewer");
    h.get_by_label(security_ui::t("Password:")).focus();
    h.get_by_label(security_ui::t("Password:"))
        .type_text("wrong");
    h.run_steps(1);
    h.get_by_label(security_ui::t("Sign In")).click();
    h.run_steps(2);
    assert!(h.state().doc.is_none());
    h.get_by_label(security_ui::wrong_password());
    shot(&mut h, "s02_wrong_password");
    h.get_by_label(security_ui::t("Password:")).focus();
    h.get_by_label(security_ui::t("Password:"))
        .type_text("test123");
    h.run_steps(1);
    h.get_by_label(security_ui::t("Sign In")).click();
    h.run_steps(3);
    let d = h.state().doc.as_ref().expect("opened");
    assert_eq!(d.db.session().unwrap().account_name(), "viewer");
    assert!(!d.db.full_access());

    h.state_mut().new_record();
    h.run_steps(3);
    h.get_by_label(security_ui::denied());
    shot(&mut h, "s03_denied");
    h.get_by_label("OK").click();
    h.run_steps(2);

    // No export privilege: File › Export Records… is refused.
    h.get_by_label(tr("File")).click();
    h.run_steps(2);
    h.get_by_label(records::t("Export Records…")).click();
    h.run_steps(3);
    h.get_by_label(security_ui::denied());
    h.get_by_label("OK").click();
    h.run_steps(2);

    h.get_by_label(tr("File")).click();
    h.run_steps(2);
    h.get_by_label(security_ui::t("Manage")).click();
    h.run_steps(2);
    assert!(
        h.get_by_label(security_ui::t("Security…"))
            .accesskit_node()
            .is_disabled()
    );
}

/// Layouts the account has no access to are not listed; full access sees
/// them, and Manage Security lists the accounts and sets.
#[test]
fn hidden_layouts_and_manage_security() {
    let bytes = protected_blank(true);
    let open_as = |account: &str| {
        let mut app = App::default();
        app.open("Sec".into(), bytes.clone());
        let l = app.security.login.take().unwrap();
        let db = Database::load_as(&l.file, account, "test123").unwrap();
        app.opened(l.name, db, l.file);
        app
    };
    assert!(open_as("viewer").doc.unwrap().layouts.is_empty());
    let admin = open_as("admin");
    assert!(!admin.doc.as_ref().unwrap().layouts.is_empty());
    let mut h = harness_for(admin);
    h.run_steps(2);
    h.get_by_label(tr("File")).click();
    h.run_steps(2);
    h.get_by_label(security_ui::t("Manage")).click();
    h.run_steps(2);
    h.get_by_label(security_ui::t("Security…")).click();
    h.run_steps(3);
    shot(&mut h, "s04_manage_security");
    h.get_by_label("viewer");
    h.get_by_label(security_ui::t("Privilege Sets")).click();
    h.run_steps(2);
    h.get_by_label("Viewers");
}
