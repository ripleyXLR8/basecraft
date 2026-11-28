//! M4 driven through `egui_kittest`: windows, the status toolbar, Quick
//! Find, Find mode, List and Table views, Undo, the Insert menu,
//! Preferences and the shortcuts. `BASECRAFT_SHOTS=<folder>` also renders
//! the screens, in English and in French (`m4_*_en.png`, `m4_*_fr.png`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::ui_tests::{below_toolbar, harness_for, shot};

/// The Contacts starter file with five contacts, on Contact Details.
fn contacts() -> Option<App> {
    let mut app = tests::template("Contacts")?;
    app.trigger_queue.clear();
    let d = app.doc.as_mut()?;
    d.path = None;
    let t = d.db.tables.iter().find(|t| t.name == "Contacts")?.id;
    let f = |d: &Doc, n: &str| d.db.table(t).and_then(|x| x.field_by_name(n)).map(|f| f.id);
    let (first, last, company, job) = (
        f(d, "First Name")?,
        f(d, "Last Name")?,
        f(d, "Company")?,
        f(d, "Job Title")?,
    );
    for (a, b, c, j) in [
        ("Ada", "Lovelace", "Analytical Engines", "Mathematician"),
        ("Grace", "Hopper", "US Navy", "Rear Admiral"),
        ("Alan", "Turing", "Bletchley Park", "Cryptanalyst"),
        ("Edsger", "Dijkstra", "Eindhoven University", "Professor"),
        ("Barbara", "Liskov", "MIT", "Professor"),
    ] {
        let r = d.db.new_record(t)?;
        d.db.set_values(
            t,
            r,
            &[
                (first, a.into()),
                (last, b.into()),
                (company, c.into()),
                (job, j.into()),
            ],
        );
    }
    app.layout = d.layouts.iter().position(|l| l.name == "Contact Details")?;
    app.found.clear();
    Some(app)
}

fn values(app: &mut App, field: &str) -> Vec<String> {
    let t = app.table().unwrap();
    let found = app.found(t).clone();
    let d = app.doc.as_ref().unwrap();
    let f = d.db.table(t).unwrap().field_by_name(field).unwrap().id;
    found
        .iter()
        .map(|&r| d.db.value(t, r, f).to_text())
        .collect()
}

fn pos(app: &App) -> usize {
    let t = app.table().unwrap();
    app.current.get(&t).copied().unwrap_or(0)
}

/// The topmost node with this label (the main window's, not one of an
/// embedded window below it).
fn top<'a>(h: &'a Harness<'_, App>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label)
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap()
}

fn menu(h: &mut Harness<'_, App>, top_label: &str, item: &str) {
    top(h, top_label).click();
    h.run_steps(2);
    h.get_by_label(item).click();
    h.run_steps(3);
}

/// A menu open, in both languages.
fn shots_menu(h: &mut Harness<'_, App>, name: &str, menu_label: &'static str) {
    if std::env::var_os("BASECRAFT_SHOTS").is_none() {
        return;
    }
    let fr = i18n::french();
    for (lang, on) in [("en", false), ("fr", true)] {
        i18n::set_french(Some(on));
        h.run_steps(2);
        top(h, tr(menu_label)).click();
        h.run_steps(3);
        shot(h, &format!("m4_{name}_{lang}"));
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
    }
    i18n::set_french(Some(fr));
    h.run_steps(2);
}

/// Screens in both languages.
fn shots(h: &mut Harness<'_, App>, name: &str) {
    if std::env::var_os("BASECRAFT_SHOTS").is_none() {
        return;
    }
    let fr = i18n::french();
    for (lang, on) in [("en", false), ("fr", true)] {
        i18n::set_french(Some(on));
        h.run_steps(2);
        shot(h, &format!("m4_{name}_{lang}"));
    }
    i18n::set_french(Some(fr));
    h.run_steps(2);
}

fn press(h: &mut Harness<'_, App>, at: egui::Pos2, button: egui::PointerButton) {
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_steps(2);
}

/// Window › New Window: a second window with its own record and found
/// set; the Window menu brings either to the front; File › Close (Ctrl+W)
/// closes the front one only. Then a second file in its own window.
#[test]
fn windows_new_select_close() {
    let Some(app) = contacts() else { return };
    let mut h = harness_for(app);
    h.run_steps(3);
    assert_eq!(h.state().window_names(), ["Contacts"]);
    menu(&mut h, tr("Window"), tr("New Window"));
    assert_eq!(h.state().window_names(), ["Contacts - 2", "Contacts"]);
    // The new window goes to the third contact and finds "Pro"fessors.
    h.state_mut().go_to(2);
    h.run_steps(2);
    assert_eq!(pos(h.state()), 2);
    h.state_mut().quick_find("professor");
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "Last Name"), ["Dijkstra", "Liskov"]);
    shots(&mut h, "windows_two");
    // The first window kept its record and found set.
    h.state_mut().select_window("Contacts");
    h.run_steps(3);
    assert_eq!(h.state().front_name(), "Contacts");
    assert_eq!(pos(h.state()), 0);
    assert_eq!(values(h.state_mut(), "Last Name").len(), 5);
    // The Window menu lists both, the front one first.
    shots_menu(&mut h, "window_menu", "Window");
    top(&h, tr("Window")).click();
    h.run_steps(2);
    h.get_by_label("2 Contacts - 2").click();
    h.run_steps(3);
    assert_eq!(h.state().front_name(), "Contacts - 2");
    assert_eq!(values(h.state_mut(), "Last Name"), ["Dijkstra", "Liskov"]);
    // Ctrl+W closes the front window; the file stays open.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    h.run_steps(3);
    assert_eq!(h.state().window_names(), ["Contacts"]);
    assert!(h.state().doc.is_some());
    assert_eq!(values(h.state_mut(), "Last Name").len(), 5);

    // Script steps act on the same windows: New Window, then Close Window.
    h.state_mut().new_window();
    h.run_steps(2);
    assert_eq!(h.state().window_names().len(), 2);
    let step = fmp12::scripts::Step::new(fmp12::scripts::id::CLOSE_WINDOW);
    h.state_mut()
        .start_run(false, |e, host| e.start_step(host, &step));
    h.run_steps(2);
    assert_eq!(h.state().window_names(), ["Contacts"]);

    // A second file opens in a window of its own; the menu lists it.
    let bytes =
        std::fs::read(r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates\Tasks\en.fmp12")
            .unwrap();
    h.state_mut().open_more("Tasks".into(), bytes);
    h.run_steps(4);
    assert_eq!(h.state().others.len(), 1);
    assert_eq!(h.state().doc.as_ref().unwrap().name, "Contacts");
    top(&h, tr("Window")).click();
    h.run_steps(2);
    h.get_by_label("2 Tasks");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    shots_menu(&mut h, "two_files", "Window");
    // Closing the first file leaves the second one.
    h.state_mut().close_window();
    h.run_steps(3);
    assert_eq!(h.state().doc.as_ref().unwrap().name, "Tasks");
    assert!(h.state().others.is_empty());
}

/// The status toolbar: record number, previous/next, Show All, New
/// Record, Quick Find; the counts; Ctrl+Up/Down.
#[test]
fn status_toolbar_and_quick_find() {
    let Some(app) = contacts() else { return };
    let mut h = harness_for(app);
    h.run_steps(3);
    shots(&mut h, "toolbar_browse");
    // Record number: type 4, Enter.
    let rec = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .filter(|n| !below_toolbar(n))
        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap()
        .rect()
        .center();
    press(&mut h, rec, egui::PointerButton::Primary);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("4".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(pos(h.state()), 3);
    h.get_by_label("◀").click();
    h.run_steps(2);
    assert_eq!(pos(h.state()), 2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(pos(h.state()), 3);
    // Quick Find (the box at the right): "alan" finds Turing.
    let qf = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .filter(|n| !below_toolbar(n))
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap()
        .rect()
        .center();
    press(&mut h, qf, egui::PointerButton::Primary);
    h.event(egui::Event::Text("alan".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(values(h.state_mut(), "Last Name"), ["Turing"]);
    h.get_by_label_contains(toolbar::tx("Found"));
    shots(&mut h, "quick_find");
    // No match: FileMaker's message, the found set stays.
    h.state_mut().quick_find("zzz");
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "Last Name"), ["Turing"]);
    assert_eq!(h.state_mut().messages.len(), 1);
    h.state_mut().messages.clear();
    // A field whose "Include field for Quick Find" is off is not searched.
    {
        let a = h.state_mut();
        let li = a.layout;
        let d = a.doc.as_mut().unwrap();
        let t = d.db.tables.iter().find(|t| t.name == "Contacts").unwrap();
        let job = t.field_by_name("Job Title").unwrap().id as u64;
        for o in &mut d.layouts[li].objects {
            if o.field.is_some_and(|f| f.1 == job) {
                o.field_flags |= fmp12::layouts::field_flags::NO_QUICK_FIND;
            }
        }
    }
    h.state_mut().quick_find("professor");
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "Last Name"), ["Turing"]);
    assert_eq!(h.state_mut().messages.len(), 1);
    h.state_mut().messages.clear();
    h.get_by_label(tr("Show All")).click();
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "Last Name").len(), 5);
    // Ctrl+S opens Sort Records (FileMaker), Ctrl+Alt+S hides the toolbar.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(2);
    assert!(h.state().m5.sort.is_some());
    h.state_mut().m5.sort = None;
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::S,
    );
    h.run_steps(2);
    assert!(!h.state().win.toolbar);
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::S,
    );
    h.run_steps(2);
    assert!(h.state().win.toolbar);
}

/// An edit box set to "Auto-complete using previously entered values"
/// lists the other records' values that start with what is typed.
#[test]
fn edit_box_auto_complete() {
    let Some(mut app) = contacts() else { return };
    let li = app.layout;
    let d = app.doc.as_mut().unwrap();
    let t = d.db.tables.iter().find(|t| t.name == "Contacts").unwrap();
    let job = t.field_by_name("Job Title").unwrap().id;
    let o = d.layouts[li]
        .objects
        .iter_mut()
        .find(|o| o.display == 0 && o.field.is_some_and(|f| f.1 == job as u64))
        .unwrap();
    o.field_flags |= fmp12::layouts::field_flags::AUTO_COMPLETE;
    let mut h = harness_for(app);
    h.run_steps(3);
    let at = h.get_by_value("Mathematician").rect().center();
    press(&mut h, at, egui::PointerButton::Primary);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("r".into()));
    h.run_steps(3);
    shots(&mut h, "auto_complete");
    h.get_by_label("Rear Admiral").click();
    h.run_steps(3);
    let at = pos(h.state());
    let jobs = values(h.state_mut(), "Job Title");
    assert_eq!(jobs[at], "Rear Admiral");
    assert_eq!(jobs.iter().filter(|j| *j == "Rear Admiral").count(), 2);
}

/// Find mode: Insert › Operators into the field edited, Omit, Perform
/// Find; Constrain and Extend; Ctrl+F, Ctrl+B, Ctrl+U.
#[test]
fn find_mode_operators_constrain_extend() {
    let Some(app) = contacts() else { return };
    let mut h = harness_for(app);
    h.run_steps(3);
    // Last Name: where Browse mode shows it.
    let last = h.get_by_label("Last Name").rect();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run_steps(3);
    assert_eq!(h.state().mode(), Mode::Find);
    // The Find mode menu is Requests.
    h.get_by_label(toolbar::tx("Requests"));
    // Last Name: the second text field of the layout.
    let field = |h: &Harness<'_, App>| {
        let mut v: Vec<egui::Rect> = h
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .filter(below_toolbar)
            .map(|n| n.rect())
            .collect();
        v.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        v
    };
    let _ = field;
    press(&mut h, last.center(), egui::PointerButton::Primary);
    h.event(egui::Event::Text("L".into()));
    h.run_steps(2);
    // Insert › Operators › "*" at the cursor: L*.
    h.get_by_label_contains(toolbar::tx("Operators")).click();
    h.run_steps(2);
    shots(&mut h, "find_operators");
    h.get_by_label(&format!("*   {}", toolbar::tx("Zero or more characters")))
        .click();
    h.run_steps(2);
    let crit: Vec<String> = h
        .state()
        .request
        .values()
        .filter(|v| !v.is_empty())
        .cloned()
        .collect();
    assert_eq!(crit, ["L*"]);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let st = h.state();
    assert_eq!(
        st.mode(),
        Mode::Browse,
        "{:?} {:?}",
        st.messages,
        st.requests
    );
    let mut names = values(h.state_mut(), "Last Name");
    names.sort();
    assert_eq!(names, ["Liskov", "Lovelace"]);
    // Constrain: of those, first name Ada.
    let app = h.state_mut();
    app.enter_find();
    let t = app.table().unwrap();
    let first = app.doc.as_ref().unwrap().db.table(t).unwrap();
    let first = first.field_by_name("First Name").unwrap().id;
    app.request.insert(first, "Ada".into());
    app.find_combined(Some(false));
    assert_eq!(values(app, "Last Name"), ["Lovelace"]);
    // Extend: plus Turing.
    app.enter_find();
    app.request.insert(first, "Alan".into());
    app.find_combined(Some(true));
    assert_eq!(values(app, "Last Name"), ["Lovelace", "Turing"]);
    // Omit: everybody but Ada.
    app.enter_find();
    app.request.insert(first, "Ada".into());
    app.omit = true;
    app.find_combined(None);
    assert_eq!(values(app, "Last Name").len(), 4);
    // Ctrl+U Preview, Ctrl+B back to Browse.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::U);
    h.run_steps(2);
    assert_eq!(h.state().mode(), Mode::Preview);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    h.run_steps(2);
    assert_eq!(h.state().mode(), Mode::Browse);
}

/// Table view: sort by a click on a header (twice: descending), resize a
/// column, hide a field and add one back, the total row; List view: the
/// current record follows the arrows and clicks.
#[test]
fn table_and_list_views() {
    let Some(mut app) = contacts() else { return };
    let d = app.doc.as_ref().unwrap();
    app.layout = d
        .layouts
        .iter()
        .position(|l| l.name == "Contact List")
        .unwrap();
    app.view = View::Table;
    // Contact List offers List view only (Layout Setup): allow Table.
    let i = app.layout;
    app.doc.as_mut().unwrap().layouts[i].options &= !fmp12::layouts::setup::NO_TABLE;
    let mut h = harness_for(app);
    h.run_steps(4);
    shots(&mut h, "table");
    let header = |h: &Harness<'_, App>, n: &str| h.get_by_label(n).rect();
    let col = header(&h, "Last Name");
    press(&mut h, col.center(), egui::PointerButton::Primary);
    assert_eq!(
        values(h.state_mut(), "Last Name"),
        ["Dijkstra", "Hopper", "Liskov", "Lovelace", "Turing"]
    );
    press(&mut h, col.center(), egui::PointerButton::Primary);
    assert_eq!(values(h.state_mut(), "Last Name")[0], "Turing");
    // Resize: drag the right edge 60 points.
    let edge = egui::pos2(col.right(), col.center().y);
    h.hover_at(edge);
    h.event(egui::Event::PointerButton {
        pos: edge,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    for k in 1..=6 {
        h.hover_at(edge + egui::vec2(10.0 * k as f32, 0.0));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton {
        pos: edge + egui::vec2(60.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    let wider = header(&h, "Last Name");
    assert!(wider.width() > col.width() + 40.0, "{col:?} {wider:?}");
    // The header's menu: hide Last Name, add it back, show its count.
    h.get_by_label("Last Name").click_secondary();
    h.run_steps(2);
    shots(&mut h, "table_menu");
    h.get_by_label(views::tv("Hide Field")).click();
    h.run_steps(2);
    assert!(h.query_by_label("Last Name").is_none());
    let first = h.get_by_label("First Name").rect();
    press(&mut h, first.center(), egui::PointerButton::Secondary);
    h.get_by_label(views::tv("Add Field")).click();
    h.run_steps(2);
    h.get_by_label("Last Name").click();
    h.run_steps(2);
    h.get_by_label("Last Name");
    h.get_by_label("First Name").click_secondary();
    h.run_steps(2);
    h.get_by_label(views::tv("Show Total")).click();
    h.run_steps(2);
    h.get_by_label_contains(&format!("{} 5", views::tv("Count")));

    // List view: Ctrl+Down and the arrows move the current record.
    h.state_mut().view = View::List;
    h.run_steps(2);
    let p0 = pos(h.state());
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(pos(h.state()), p0 + 1);
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(pos(h.state()), p0);
    shots(&mut h, "list");
}

/// Edit › Undo / Redo of a committed field edit; Insert › Current User
/// Name (Preferences' user name) and From Index at the cursor.
#[test]
fn undo_redo_and_insert_menu() {
    let Some(app) = contacts() else { return };
    let mut h = harness_for(app);
    h.run_steps(3);
    let fields = |h: &Harness<'_, App>| {
        let mut v: Vec<egui::Rect> = h
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .filter(below_toolbar)
            .filter(|n| n.rect().left() < 400.0)
            .map(|n| n.rect())
            .collect();
        v.sort_by(|a, b| a.top().total_cmp(&b.top()));
        v
    };
    let _ = fields;
    // First Name: retype it, leave it.
    let job = h.get_by_label("First Name").rect();
    press(&mut h, job.center(), egui::PointerButton::Primary);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Countess".into()));
    h.run_steps(2);
    press(
        &mut h,
        egui::pos2(1000.0, 880.0),
        egui::PointerButton::Primary,
    );
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "First Name")[0], "Countess");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "First Name")[0], "Ada");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Y);
    h.run_steps(2);
    assert_eq!(values(h.state_mut(), "First Name")[0], "Countess");

    // Preferences › User name, then Insert › Current User Name.
    let mut p = prefs::get();
    p.user = "Ada L.".into();
    prefs::set(p);
    press(&mut h, job.center(), egui::PointerButton::Primary);
    h.key_press(egui::Key::End);
    h.event(egui::Event::Text(" ".into()));
    h.run_steps(2);
    shots_menu(&mut h, "insert_menu", "Insert");
    top(&h, tr("Insert")).click();
    h.run_steps(2);
    h.get_by_label_contains(editing::tx("Current User Name"))
        .click();
    h.run_steps(3);
    assert_eq!(values(h.state_mut(), "First Name")[0], "Countess Ada L.");
    // Get ( UserName ) gives the same name.
    let app = h.state_mut();
    let t = app.table().unwrap();
    let d = app.doc.as_ref().unwrap();
    assert_eq!(
        d.db.eval_source("Get ( UserName )", t, 0, Some(0))
            .to_text(),
        "Ada L."
    );
    // From Index (Ctrl+I): the values of Job Title in all records.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::I);
    h.run_steps(3);
    shots(&mut h, "insert_index");
    h.get_by_label("Grace").click();
    h.run_steps(1);
    h.get_by_label(editing::tx("OK")).click();
    h.run_steps(3);
    assert!(values(h.state_mut(), "First Name")[0].ends_with("Grace"));
    prefs::set(prefs::Prefs::default());
}

/// Edit › Preferences: user name, language, units; OK applies them.
#[test]
fn preferences_dialog() {
    let Some(app) = contacts() else { return };
    let mut h = harness_for(app);
    h.run_steps(3);
    i18n::set_french(Some(false));
    menu(&mut h, "Edit", prefs::t("Preferences…"));
    assert!(h.state().prefs.is_some());
    shots(&mut h, "preferences");
    i18n::set_french(Some(false));
    h.run_steps(2);
    h.get_by_label(prefs::t("User name:")).focus();
    h.run_steps(1);
    h.get_by_label(prefs::t("User name:")).type_text("Grace");
    h.get_by_label(prefs::t("Centimeters")).click();
    h.run_steps(1);
    h.state_mut().prefs.as_mut().unwrap().french = Some(true);
    h.run_steps(2);
    h.get_by_label(prefs::t("OK")).click();
    h.run_steps(3);
    assert!(h.state().prefs.is_none());
    let p = prefs::get();
    assert_eq!(p.user, "Grace");
    assert_eq!(p.units, prefs::Units::Centimeters);
    assert_eq!(p.french, Some(true));
    // The menus are in French now.
    h.get_by_label("Fichier");
    assert_eq!(prefs::unit().1, " cm");
    prefs::set(prefs::Prefs::default());
    i18n::set_french(None);
}

/// Find mode on a related field: contacts with a matching phone number.
#[test]
fn find_on_related_field() {
    let Some(mut app) = contacts() else { return };
    let d = app.doc.as_mut().unwrap();
    let to = |d: &Doc, n: &str| d.db.occurrences.iter().find(|o| o.name == n).unwrap().id;
    let pto = to(d, "Phone Numbers");
    let c =
        d.db.tables
            .iter()
            .find(|t| t.name == "Contacts")
            .unwrap()
            .id;
    let p = d.db.table_of(pto).unwrap().id;
    let pk =
        d.db.table(c)
            .unwrap()
            .field_by_name("PrimaryKey")
            .unwrap()
            .id;
    let fk =
        d.db.table(p)
            .unwrap()
            .field_by_name("ForeignKey")
            .unwrap()
            .id;
    let num = d.db.table(p).unwrap().field_by_name("Number").unwrap().id;
    for (rec, n) in [(1, "555-0101"), (3, "555-0199")] {
        let key = d.db.value(c, rec, pk).to_text();
        let r = d.db.new_record(p).unwrap();
        d.db.set_values(p, r, &[(fk, key), (num, n.into())]);
    }
    app.enter_find();
    app.request
        .insert(engine::related_key(pto, num), "555-0101".into());
    app.find_combined(None);
    assert_eq!(values(&mut app, "Last Name"), ["Hopper"]);
    app.enter_find();
    app.request
        .insert(engine::related_key(pto, num), "555*".into());
    app.omit = true;
    app.find_combined(None);
    assert_eq!(values(&mut app, "Last Name").len(), 3);
}
