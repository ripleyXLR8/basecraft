//! BaseCraft desktop application (egui).
//!
//! Browse, Find and Layout modes over a `.fmp12` file: layouts drawn at
//! their real coordinates, editable fields, new/delete record, find
//! requests, sort, and layout editing (`design.rs`). Scripts run
//! interactively (custom dialogs and pauses wait for the user), script
//! triggers fire on layout, record, mode and field events, and the Script
//! Workspace (`workspace.rs`) edits scripts. File › Save writes records,
//! layouts and scripts back to the `.fmp12`.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use engine::{Database, Order};
use fmp12::layouts::Layout;

mod browse;
mod chart;
mod container;
mod controls;
#[cfg(test)]
mod coverage_tests;
mod custom_functions;
mod debugger;
mod design;
mod editing;
mod i18n;
mod launch;
mod layout_setup;
mod look;
#[cfg(test)]
mod m4_tests;
mod objects;
mod prefs;
mod preview;
mod records;
mod remote_ui;
mod script_ui;
mod security_ui;
mod toolbar;
#[cfg(test)]
mod trigger_tests;
mod triggers;
#[cfg(test)]
mod ui_tests;
mod views;
mod windows;
mod workspace;
use fmp12::triggers::event;
use i18n::tr;
use look::Look;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Find,
    Layout,
    Preview,
}

struct Doc {
    /// File name shown in the title bar.
    name: String,
    db: Database,
    /// Layouts that have a table occurrence (folders excluded).
    layouts: Vec<Layout>,
    /// The file as last opened or saved; saving rebuilds from it.
    file: fmp12::File,
    /// Where the file came from (desktop only).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    path: Option<std::path::PathBuf>,
    /// Script triggers of layouts, objects and the file.
    triggers: fmp12::triggers::Triggers,
}

/// A script run waiting for the user (custom dialog or pause).
struct Active {
    run: script::Run,
    wait: script::Suspend,
    /// Text typed in the dialog's input fields.
    inputs: Vec<String>,
    /// When a timed pause started (seconds, `egui` input time).
    since: f64,
}

use script::View;

/// A file picked asynchronously (browser), waiting to be opened.
type Incoming = Rc<RefCell<Option<(String, Vec<u8>)>>>;

#[derive(Default)]
struct App {
    /// Tools › Data Viewer: open flag, watched expressions, new expression.
    data_viewer: bool,
    watches: Vec<String>,
    watch_input: String,
    incoming: Incoming,
    view: View,
    doc: Option<Doc>,
    error: Option<String>,
    layout: usize,
    /// Found set per base table (record indexes), and the current position.
    found: BTreeMap<u32, Vec<usize>>,
    current: BTreeMap<u32, usize>,
    sorted: BTreeMap<u32, bool>,
    mode: Option<Mode>,
    /// Find request being edited: field id → criterion.
    request: BTreeMap<u32, String>,
    /// Whether the request being edited omits records.
    omit: bool,
    /// All find requests (the one being edited is copied in on change).
    requests: Vec<(BTreeMap<u32, String>, bool)>,
    /// Index of the request being edited.
    req: usize,
    dirty: bool,
    /// Record modified since its last commit (table, record index).
    editing: Option<(u32, usize)>,
    /// Validation failure waiting for the user: message and whether it may be overridden.
    alert: Option<(String, bool)>,
    /// Script engine of the open file (keeps `$$globals` between runs).
    engine: Option<script::Engine>,
    /// Button action clicked this frame, run after drawing.
    pending_step: Option<fmp12::scripts::Step>,
    /// Script chosen in the Scripts menu, run after drawing.
    pending_script: Option<u32>,
    /// Messages for the user (title, message).
    messages: Vec<(String, String)>,
    /// Script run suspended on a dialog or a pause.
    active: Option<Active>,
    /// Scripts started by triggers, run one after the other: (script,
    /// parameter calculation).
    trigger_queue: VecDeque<(u32, Option<fmp12::scripts::Calc>)>,
    /// Field object events of this frame (object id, trigger event).
    obj_events: Vec<(u64, u16)>,
    /// Script Workspace window.
    workspace: workspace::Workspace,
    /// The running script was started by a trigger: its layout changes do
    /// not fire more triggers (no trigger loops).
    from_trigger: bool,
    /// Sort applied to the found set (for sub-summaries in Preview).
    last_sort: Vec<(u32, Order)>,
    /// Preview mode: current page and page count.
    page: usize,
    pages: usize,
    /// Front panel chosen by the user, per tab control or slide control id.
    tabs: BTreeMap<u64, u64>,
    /// Popovers, web viewers and the tab order in Browse mode (`objects.rs`).
    objs: objects::State,
    /// Layout mode state (the layout being edited).
    designer: Option<design::Designer>,
    /// Records menu, Find/Replace, import and export dialogs (`records.rs`).
    m8: records::Dialogs,
    /// Sort, Go to Record, Saved Finds, Revert dialogs (`browse.rs`).
    m5: browse::Dialogs,
    /// Script steps done by the app (`script_ui.rs`).
    steps_ui: script_ui::State,
    /// Tools › Script Debugger (`debugger.rs`).
    debugger: debugger::Debugger,
    /// The window as scripts last left it: name, zoom, toolbar…
    win: script::Window,
    /// Login dialog and File › Manage › Security (`security_ui.rs`).
    security: security_ui::State,
    /// File › Open Remote… and the hosted file's session (`remote_ui.rs`).
    remote: remote_ui::State,
    /// Background windows of this file (`windows.rs`).
    wins: windows::State,
    /// Other files open at once, each in its own OS window.
    others: Vec<App>,
    /// Status toolbar, Quick Find, find operators (`toolbar.rs`).
    bar: toolbar::State,
    /// Undo/Redo of field edits, auto-complete (`editing.rs`).
    edits: editing::State,
    /// Table view columns, widths, totals (`views.rs`).
    views: views::State,
    /// Layout Setup's default view, per window and layout (`layout_setup.rs`).
    layout_views: layout_setup::Views,
    /// Edit › Preferences… being edited (`prefs.rs`).
    prefs: Option<prefs::Prefs>,
    /// Launch Center (`launch.rs`).
    launch: launch::State,
    /// The window as last drawn: content size, screen, modifier keys
    /// (`Get ( WindowContentWidth )`, `Get ( ScreenWidth )`…).
    seen: script::HostInfo,
    /// Content size of each window last frame (OnLayoutSizeChange).
    sizes: BTreeMap<String, egui::Vec2>,
}

impl App {
    fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Browse)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_dialog(&mut self, _ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("FileMaker", &["fmp12"])
            .pick_file()
        {
            self.open_path(&p);
        }
    }

    /// In the browser the picker is asynchronous: the file lands in
    /// `incoming` and is opened on the next frame.
    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        let slot = self.incoming.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new()
                .add_filter("FileMaker", &["fmp12"])
                .pick_file()
                .await
            {
                let data = h.read().await;
                *slot.borrow_mut() = Some((h.file_name(), data));
                ctx.request_repaint();
            }
        });
    }

    /// Whether a file is open (or waiting for its login) in this window.
    fn busy(&self) -> bool {
        self.doc.is_some() || self.security.login.is_some()
    }

    /// Opens a file; with a file already open, in a window of its own
    /// (FileMaker keeps several files open).
    fn open_more(&mut self, name: String, bytes: Vec<u8>) {
        if !self.busy() {
            return self.open_any(name, bytes);
        }
        let mut a = App::default();
        a.open_any(name, bytes);
        self.others.push(a);
    }

    /// A `.fmp12` file, or a CSV, tab-separated or Excel file converted
    /// into a new one.
    fn open_any(&mut self, name: String, bytes: Vec<u8>) {
        if launch::convertible(&name) {
            self.convert(name, bytes);
        } else {
            self.open(name, bytes);
        }
    }

    /// File › Open, File › Open Recent, the command line and dropped files.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_path(&mut self, p: &std::path::Path) {
        // Converted files get no path: saving must not overwrite the source.
        if launch::convertible(&p.to_string_lossy()) {
            match std::fs::read(p) {
                Ok(b) => {
                    let name = p
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    self.open_more(name, b);
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            return;
        }
        if self.busy() {
            if self.others.iter().chain([&*self]).any(|a| a.is_file(p)) {
                return;
            }
            let mut a = App::default();
            a.load_path(p);
            self.others.push(a);
        } else {
            self.load_path(p);
        }
        prefs::add_recent(p);
    }

    /// Whether this window has the file at `p` open.
    #[cfg(not(target_arch = "wasm32"))]
    fn is_file(&self, p: &std::path::Path) -> bool {
        self.doc
            .as_ref()
            .and_then(|d| d.path.as_ref())
            .is_some_and(|q| q == p)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_path(&mut self, p: &std::path::Path) {
        match std::fs::read(p) {
            Ok(b) => {
                let name = p
                    .file_stem()
                    .map_or("BaseCraft".into(), |s| s.to_string_lossy().into_owned());
                self.open(name, b);
                if let Some(d) = self.doc.as_mut() {
                    d.path = Some(p.to_path_buf());
                    // Container files kept in open external storage.
                    if let Some(dir) = p.parent() {
                        d.db.read_external_containers(dir);
                    }
                }
                if let Some(l) = self.security.login.as_mut() {
                    l.path = Some(p.to_path_buf());
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn open(&mut self, name: String, bytes: Vec<u8>) {
        let name = name.trim_end_matches(".fmp12").to_string();
        let file = match fmp12::File::from_bytes(bytes) {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        match Database::load(&file) {
            Ok(db) => self.opened(name, db, file),
            // A protected file: the login dialog opens it (`security_ui.rs`).
            Err(fmp12::Error::LoginRequired) => self.ask_login(name, file),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Shows a file once it is loaded (and its account logged in).
    fn opened(&mut self, name: String, db: Database, file: fmp12::File) {
        {
            {
                // Layouts the account has no access to are not listed.
                let layouts = db
                    .layouts
                    .iter()
                    .filter(|l| {
                        l.table_occurrence != 0
                            && db.layout_access(l.id) != fmp12::security::NO_ACCESS
                    })
                    .cloned()
                    .collect();
                let engine = script::Engine::load(&file).ok();
                let triggers = fmp12::triggers::triggers(&file).unwrap_or_default();
                *self = App {
                    engine,
                    win: script::Window {
                        name: name.clone(),
                        ..Default::default()
                    },
                    doc: Some(Doc {
                        name,
                        db,
                        layouts,
                        file,
                        path: None,
                        triggers,
                    }),
                    incoming: self.incoming.clone(),
                    others: std::mem::take(&mut self.others),
                    wins: windows::State::new(),
                    ..Default::default()
                };
                // The first window opens on File Options' "Switch to
                // layout" (else the first layout): its layout triggers,
                // then the file's OnFirstWindowOpen.
                if let Some(d) = self.doc.as_ref()
                    && let Some(i) = d
                        .triggers
                        .open_layout
                        .and_then(|id| d.layouts.iter().position(|l| l.id == id))
                {
                    self.layout = i;
                }
                self.fire(event::LAYOUT_ENTER);
                self.fire(event::MODE_ENTER);
                self.fire(event::RECORD_LOAD);
                self.fire_file(event::FIRST_WINDOW_OPEN);
                self.fire_file(event::WINDOW_OPEN);
            }
        }
    }

    /// Saves the records into a new `.fmp12` image. `as_copy` asks where.
    fn save(&mut self, ctx: &egui::Context, as_copy: bool) {
        if self.remote_save() || !self.commit_record() {
            return;
        }
        let Some(d) = self.doc.as_mut() else { return };
        // In Layout mode, saving the file saves the layout first; an open
        // Script Workspace saves its scripts.
        if let Some(ds) = self.designer.as_mut() {
            ds.apply(d);
        }
        if self.workspace.dirty
            && let Some(e) = self.engine.as_mut()
        {
            e.scripts = self.workspace.scripts.clone();
            self.workspace.dirty = false;
        }
        let scripts = self.engine.as_ref().map(|e| e.scripts.clone());
        let saved = d.db.save(&d.file).and_then(|b| {
            let f = fmp12::File::from_bytes(b)?;
            let b = fmp12::save::save_layouts(&f, &d.db.layouts)?;
            match &scripts {
                Some(list) => fmp12::scripts_save::save_scripts(&fmp12::File::from_bytes(b)?, list),
                None => Ok(b),
            }
        });
        let bytes = match saved {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        match fmp12::File::from_bytes(bytes.clone()) {
            Ok(f) => {
                // Scripts as written (steps get their parameter nodes).
                if let (Some(e), Ok(list)) = (self.engine.as_mut(), fmp12::scripts::scripts(&f)) {
                    e.scripts = list;
                    if !self.workspace.dirty {
                        self.workspace.scripts = e.scripts.clone();
                    }
                }
                d.file = f;
                // Layouts as written (new parts get their node keys).
                if let Ok(ls) = fmp12::layouts::layouts(&d.file) {
                    d.db.layouts = ls;
                    design::reload_layouts(d);
                    if let Some(ds) = self.designer.as_mut() {
                        ds.reloaded(d);
                    }
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        }

        let file_name = format!("{}.fmp12", d.name);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let target = match (&d.path, as_copy) {
                (Some(p), false) => Some(p.clone()),
                _ => rfd::FileDialog::new()
                    .add_filter("FileMaker", &["fmp12"])
                    .set_file_name(&file_name)
                    .save_file(),
            };
            let Some(t) = target else { return };
            if let Err(e) = std::fs::write(&t, &bytes) {
                self.error = Some(e.to_string());
                return;
            }
            if let Some(dir) = t.parent()
                && let Err(e) = d.db.write_external_containers(dir)
            {
                self.error = Some(e.to_string());
                return;
            }
            if !as_copy {
                d.path = Some(t);
            }
        }
        // In the browser, saving downloads the file.
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, as_copy);
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(h) = rfd::AsyncFileDialog::new()
                    .set_file_name(&file_name)
                    .save_file()
                    .await
                {
                    let _ = h.write(&bytes).await;
                }
            });
        }
        self.dirty = false;
        self.error = None;
    }

    /// Leaves the record being edited: commits it, or with Layout Setup's
    /// "Save record changes automatically" off asks Save / Don't Save /
    /// Cancel first (`browse::save_ui`; false: the action is not done).
    fn commit(&mut self) -> bool {
        if self.editing.is_some() && !self.auto_save() {
            self.ask_save();
            return false;
        }
        self.commit_record()
    }

    /// Commits the record being edited: runs field validation like
    /// FileMaker. Returns false (and shows a message) when it fails.
    fn commit_record(&mut self) -> bool {
        let Some((t, r)) = self.editing else {
            return true;
        };
        let Some(d) = self.doc.as_ref() else {
            return true;
        };
        let errs = d.db.validate(t, r);
        if errs.is_empty() {
            self.editing = None;
            self.fire(event::RECORD_COMMIT);
            self.window_transaction("Modified", t, &[r]);
            return true;
        }
        let tbl = d.db.table(t);
        let msg: Vec<String> = errs
            .iter()
            .map(|e| {
                // A custom message replaces FileMaker's.
                if let Some(m) = d.db.validation_message(t, r, e.field) {
                    return m;
                }
                let f = tbl.and_then(|x| x.field(e.field));
                let name = f.map_or(String::new(), |f| f.name.clone());
                let rule = match e.rule {
                    engine::Rule::NotEmpty => tr("must not be empty.").to_string(),
                    engine::Rule::Unique => tr("must contain a unique value.").to_string(),
                    engine::Rule::Existing => tr("must contain an existing value.").to_string(),
                    engine::Rule::Numeric => tr("must contain a number.").to_string(),
                    engine::Rule::Date => {
                        tr("must contain a date with a four-digit year.").to_string()
                    }
                    engine::Rule::Time => tr("must contain a time of day.").to_string(),
                    engine::Rule::Range => {
                        let (lo, hi) = f.and_then(|f| f.range.clone()).unwrap_or_default();
                        format!("{} {lo} – {hi}", tr("must contain a value in the range"))
                    }
                    engine::Rule::Calculation => {
                        tr("does not pass its validation calculation.").to_string()
                    }
                    engine::Rule::ValueList => {
                        tr("must contain a value from the value list.").to_string()
                    }
                    engine::Rule::MaxChars => format!(
                        "{} {}",
                        tr("must contain at most this many characters:"),
                        f.and_then(|f| f.def.max_size()).unwrap_or(0)
                    ),
                };
                format!("« {name} » {rule}")
            })
            .collect();
        self.alert = Some((msg.join("\n"), errs.iter().all(|e| e.can_override)));
        false
    }

    /// The window as scripts see it: layout, mode, found set, record.
    fn script_window(&mut self) -> Option<script::Window> {
        let t = self.table()?;
        let found = self.found(t).clone();
        let d = self.doc.as_ref()?;
        let mut window = script::Window {
            view: self.view,
            ..self.win.clone()
        };
        window.layout = d.layouts.get(self.layout)?.id;
        if window.name.is_empty() {
            // Get ( WindowName ) of the file's window: the file name.
            window.name = d.name.clone();
        }
        window.found = found;
        window.current = *self.current.get(&t).unwrap_or(&0);
        window.sorted = self.sorted.get(&t).copied().unwrap_or(false);
        if window.sorted {
            window.sort = self.last_sort.iter().map(|(f, _)| *f).collect();
        }
        window.host = script::HostInfo {
            file_name: d.name.clone(),
            file_path: d.path.as_ref().map_or_else(String::new, |p| {
                format!("file:{}", calc::functions::fm_path(p))
            }),
            file_size: d
                .path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len()),
            quick_find: self.bar.quick.clone(),
            ..self.seen.clone()
        };
        window.mode = match self.mode() {
            Mode::Find => script::Mode::Find,
            Mode::Browse | Mode::Layout => script::Mode::Browse,
            Mode::Preview => script::Mode::Preview,
        };
        Some(window)
    }

    /// Runs `f` with the app as script host: the window state (layout,
    /// mode, found set, record) goes in, and comes back out changed.
    fn with_host<R>(
        &mut self,
        f: impl FnOnce(&mut script::Engine, &mut AppHost) -> R,
    ) -> Option<R> {
        let mode = self.mode();
        let window = self.script_window()?;
        let d = self.doc.as_mut()?;
        let mut engine = self.engine.take()?;
        let before = data_fingerprint(&d.db);
        let mut disk = std::mem::take(&mut self.steps_ui.disk);
        disk.folder = d
            .path
            .as_ref()
            .and_then(|p| p.parent().map(|x| x.to_path_buf()));
        // Steps fire triggers (`script::triggers`), except in a script a
        // trigger started (no trigger loops).
        let mut host = AppHost {
            db: &mut d.db,
            window,
            disk,
            triggers: (!self.from_trigger).then_some(&d.triggers),
        };
        let out = f(&mut engine, &mut host);
        self.steps_ui.disk = host.disk;
        let window = host.window;
        self.engine = Some(engine);
        self.view = window.view;
        self.win = window.clone();
        if let Some(i) = d.layouts.iter().position(|l| l.id == window.layout) {
            self.layout = i;
        }
        self.mode = Some(match window.mode {
            script::Mode::Find => Mode::Find,
            script::Mode::Preview => Mode::Preview,
            script::Mode::Browse => Mode::Browse,
        });
        if mode == Mode::Find && window.mode == script::Mode::Find {
            // Requests built by the script show in the find form.
            self.requests = window
                .requests
                .iter()
                .map(|r| (r.criteria.iter().cloned().collect(), r.omit))
                .collect();
        }
        if let Some(t2) = self.table() {
            self.found.insert(t2, window.found);
            self.current.insert(t2, window.current);
            self.sorted.insert(t2, window.sorted);
        }
        // Only data changes need saving; a sort or a find does not.
        if self
            .doc
            .as_ref()
            .is_some_and(|d| data_fingerprint(&d.db) != before)
        {
            self.dirty = true;
        }
        Some(out)
    }

    /// Starts a script or a button step and runs it until it ends or waits.
    fn start_run(
        &mut self,
        trigger: bool,
        start: impl FnOnce(&mut script::Engine, &mut AppHost) -> script::Run,
    ) {
        if self.active.is_some() || !self.commit_record() {
            return;
        }
        self.from_trigger = trigger;
        // Tools › Script Debugger open: stop at the first step and at
        // the workspace's breakpoints.
        let debug = self
            .debugger
            .open
            .then(|| self.workspace.breakpoints.clone());
        let r = self.with_host(|e, h| {
            let mut run = start(e, h);
            if let Some(bps) = debug {
                run.set_debug(true);
                run.breakpoints = bps;
            }
            let st = run.resume(e, h, script::Answer::Start);
            (run, st)
        });
        if let Some((run, st)) = r {
            self.after(run, st, 0.0);
        }
    }

    /// Answers the waiting run and continues it.
    fn answer(&mut self, a: script::Answer, now: f64) {
        let Some(mut act) = self.active.take() else {
            return;
        };
        if let Some(st) = self.with_host(|e, h| act.run.resume(e, h, a)) {
            self.after(act.run, st, now);
        }
    }

    fn after(&mut self, run: script::Run, st: script::Status, now: f64) {
        match st {
            script::Status::Finished(o) => {
                if o.last_error != 0 && o.halted {
                    self.error = Some(format!("{} {}", tr("Script error"), o.last_error));
                }
            }
            script::Status::Suspended(wait) => {
                let inputs = match &wait {
                    script::Suspend::Dialog(d) => d.inputs.iter().map(|i| i.2.clone()).collect(),
                    _ => Vec::new(),
                };
                self.active = Some(Active {
                    run,
                    wait,
                    inputs,
                    since: now,
                });
            }
        }
    }

    /// Queues the current layout's trigger for `ev` (in the current mode).
    fn fire(&mut self, ev: u16) {
        if let Some(t) = self.layout_trigger(ev) {
            self.trigger_queue.push_back((t.script, t.parameter));
        }
    }

    /// Queues an object's trigger for `ev`.
    fn fire_object(&mut self, object: u64, ev: u16) {
        if let Some(t) = self.object_trigger(object, ev) {
            self.trigger_queue.push_back((t.script, t.parameter));
        }
    }

    /// Starts the next triggered script when nothing runs.
    fn run_triggers(&mut self) {
        if self.active.is_some() {
            return;
        }
        let Some((script, param)) = self.trigger_queue.pop_front() else {
            return;
        };
        let value = self.trigger_value(param);
        if let Some(e) = self.engine.as_mut() {
            e.trigger = Default::default();
        }
        self.start_run(true, |e, h| e.start(h, script, value));
    }

    /// The dialog or the pause bar of the waiting script.
    fn script_wait_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut answer = None;
        if let Some(script::Suspend::Ui(r)) = self.active.as_ref().map(|a| a.wait.clone()) {
            if let Some(a) = self.ui_request(ctx, &r) {
                self.answer(a, now);
            }
            return;
        }
        let Some(act) = self.active.as_mut() else {
            return;
        };
        match &act.wait {
            script::Suspend::Dialog(d) => {
                let d = d.clone();
                egui::Modal::new(egui::Id::NULL.with("script_dialog")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.set_max_width(480.0);
                    if !d.title.is_empty() {
                        ui.heading(&d.title);
                    }
                    if !d.message.is_empty() {
                        ui.label(&d.message);
                    }
                    for (i, (_, label, _)) in d.inputs.iter().enumerate() {
                        if !label.is_empty() {
                            ui.label(label);
                        }
                        if let Some(v) = act.inputs.get_mut(i) {
                            ui.text_edit_singleline(v);
                        }
                    }
                    ui.add_space(6.0);
                    // FileMaker puts button 1 (the default) on the right.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (i, b) in d.buttons.iter().enumerate() {
                            let r = ui.button(b);
                            if r.clicked()
                                || i == 0 && ui.input(|x| x.key_pressed(egui::Key::Enter))
                            {
                                answer = Some(script::Answer::Dialog(script::DialogAnswer {
                                    button: i + 1,
                                    inputs: act.inputs.clone(),
                                }));
                            }
                        }
                    });
                });
            }
            script::Suspend::Pause(secs) => {
                if let Some(s) = secs {
                    if act.since == 0.0 {
                        act.since = now;
                    }
                    let left = act.since + s - now;
                    if left <= 0.0 {
                        answer = Some(script::Answer::Continue);
                    } else {
                        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
                    }
                }
            }
            // Driven by the debugger window; Ui requests are handled above.
            script::Suspend::Breakpoint | script::Suspend::Ui(_) => {}
        }
        if let Some(a) = answer {
            self.answer(a, now);
        }
    }

    /// Tools › Data Viewer: watch expressions evaluated on the current record.
    fn data_viewer_window(&mut self, ctx: &egui::Context) {
        if !self.data_viewer {
            return;
        }
        // Evaluated like FileMaker's: in the window's context (Get
        // functions see the found set and record) with the script globals.
        let window = self.script_window();
        let values: Vec<String> = match (self.doc.as_mut(), self.engine.as_mut(), window) {
            (Some(d), Some(e), Some(window)) => {
                let mut host = AppHost {
                    db: &mut d.db,
                    window,
                    disk: Default::default(),
                    triggers: None,
                };
                self.watches
                    .iter()
                    .map(|w| {
                        let names = script::text::Names {
                            db: host.db,
                            scripts: &e.scripts,
                        };
                        match names.compile(w) {
                            Ok(c) => e.evaluate(&mut host, &c).map_or_else(
                                |code| format!("{} {code}", tr("Error")),
                                |v| v.to_text(),
                            ),
                            Err(err) => err.to_string(),
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        let mut open = true;
        let mut remove = None;
        tool_window(
            ctx,
            DATA_VIEWER,
            tr("Data Viewer"),
            [520.0, 240.0],
            // Below the status toolbar, not over it.
            Some([500.0, 120.0]),
            &mut open,
            |ui| {
                // FileMaker's two tabs: Current (the paused script's
                // variables) and Watch.
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.debugger.current_tab, true, tr("Current"));
                    ui.selectable_value(&mut self.debugger.current_tab, false, tr("Watch"));
                });
                if self.debugger.current_tab {
                    self.data_viewer_current(ui);
                    return;
                }
                egui::Grid::new("watches")
                    .striped(true)
                    .num_columns(3)
                    .show(ui, |ui| {
                        ui.strong(tr("Expression"));
                        ui.strong(tr("Value"));
                        ui.label("");
                        ui.end_row();
                        for (i, w) in self.watches.iter().enumerate() {
                            let value = values.get(i).cloned().unwrap_or_default();
                            ui.monospace(w);
                            // FileMaker shows paragraph returns as the pilcrow.
                            ui.label(value.replace('\r', "\u{b6}"));
                            if ui.small_button("✕").clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.watch_input)
                            .hint_text(tr("Calculation, e.g. Get ( FoundCount )"))
                            .desired_width(380.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button(tr("Add")).clicked() || enter)
                        && !self.watch_input.trim().is_empty()
                    {
                        self.watches.push(std::mem::take(&mut self.watch_input));
                    }
                });
            },
        );
        if let Some(i) = remove {
            self.watches.remove(i);
        }
        self.data_viewer = open;
    }

    /// Base table id of the current layout.
    fn table(&self) -> Option<u32> {
        let d = self.doc.as_ref()?;
        let l = d.layouts.get(self.layout)?;
        Some(d.db.table_of((l.table_occurrence & 0xFFF) as u32)?.id)
    }

    fn found(&mut self, t: u32) -> &mut Vec<usize> {
        let n = self
            .doc
            .as_ref()
            .and_then(|d| d.db.table(t))
            .map_or(0, |t| t.records.len());
        self.found.entry(t).or_insert_with(|| (0..n).collect())
    }

    fn current_record(&mut self) -> Option<usize> {
        let t = self.table()?;
        let pos = *self.current.get(&t).unwrap_or(&0);
        self.found(t).get(pos).copied()
    }

    fn new_record(&mut self) {
        if !self.commit() {
            return;
        }
        let Some(t) = self.table() else { return };
        let Some(d) = self.doc.as_mut() else { return };
        let Some(r) = d.db.new_record(t) else {
            return;
        };
        let f = self.found(t);
        f.push(r);
        let n = f.len();
        self.current.insert(t, n - 1);
        self.dirty = true;
        self.fire(event::RECORD_LOAD);
    }

    fn show_all(&mut self) {
        if !self.commit() {
            return;
        }
        if let Some(t) = self.table() {
            self.found.remove(&t);
            self.sorted.remove(&t);
            self.current.insert(t, 0);
        }
    }

    fn enter_find(&mut self) {
        if !self.commit() || !self.mode_exit() {
            return;
        }
        self.mode = Some(Mode::Find);
        self.request.clear();
        self.omit = false;
        self.requests = vec![(BTreeMap::new(), false)];
        self.req = 0;
        self.fire(event::MODE_ENTER);
    }

    /// Stores the edited request and switches to request `i` (a new one past the end).
    fn switch_request(&mut self, i: usize) {
        let cur = (std::mem::take(&mut self.request), self.omit);
        if self.req < self.requests.len() {
            self.requests[self.req] = cur;
        } else {
            self.requests.push(cur);
        }
        if i >= self.requests.len() {
            self.requests.push((BTreeMap::new(), false));
        }
        self.req = i.min(self.requests.len() - 1);
        (self.request, self.omit) = self.requests[self.req].clone();
    }

    fn perform_find(&mut self) {
        if self.commit() {
            self.find_combined(None);
        }
    }

    /// Records › Omit Record: removes the current record from the found set.
    fn omit_record(&mut self) {
        let Some(t) = self.table() else { return };
        if !self.commit() {
            return;
        }
        let pos = *self.current.get(&t).unwrap_or(&0);
        let f = self.found(t);
        if pos < f.len() {
            f.remove(pos);
        }
        let n = f.len();
        self.current.insert(t, pos.min(n.saturating_sub(1)));
    }

    /// Records › Show Omitted Only: swaps the found set with its complement.
    fn show_omitted(&mut self) {
        let Some(t) = self.table() else { return };
        if !self.commit() {
            return;
        }
        let n = self
            .doc
            .as_ref()
            .and_then(|d| d.db.table(t))
            .map_or(0, |x| x.records.len());
        let f = self.found(t).clone();
        let omitted: Vec<usize> = (0..n).filter(|r| !f.contains(r)).collect();
        self.found.insert(t, omitted);
        self.sorted.remove(&t);
        self.current.insert(t, 0);
    }

    /// View › Layout Mode: edits a copy of the current layout.
    fn enter_layout(&mut self) {
        if self.mode() == Mode::Layout
            || self.remote_refuses(tr("Layout Mode"))
            || !self.commit()
            || !self.mode_exit()
        {
            return;
        }
        let Some(d) = self.doc.as_ref() else { return };
        if self.layout >= d.layouts.len() {
            return;
        }
        self.designer = Some(design::Designer::new(d, self.layout));
        self.mode = Some(Mode::Layout);
    }

    /// Leaves the layout (exit, other layout, new layout): FileMaker asks
    /// whether to save the changes first.
    fn leave_layout(&mut self, then: design::Then) {
        match self.designer.as_mut() {
            Some(ds) if ds.changed() => ds.ask = Some(then),
            _ => self.finish_layout(then),
        }
    }

    fn finish_layout(&mut self, then: design::Then) {
        match then {
            design::Then::Exit => {
                self.designer = None;
                self.mode = Some(Mode::Browse);
            }
            design::Then::Switch(i) => {
                self.layout = i;
                self.designer = self.doc.as_ref().map(|d| design::Designer::new(d, i));
            }
            // Layouts menu: written to the file (unsaved edits of the
            // current layout were saved or dropped by the question before).
            design::Then::Op(op) => {
                let Some(d) = self.doc.as_mut() else { return };
                match design::run_layout_op(d, self.layout, &op) {
                    Ok(i) => {
                        self.layout = i;
                        self.designer = Some(design::Designer::new(d, i));
                        self.dirty = true;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File"), |ui| {
                if ui
                    .add(egui::Button::new(tr("Open…")).shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    ui.close();
                    self.open_dialog(&ui.ctx().clone());
                }
                self.recent_menu(ui);
                let lc =
                    egui::Button::new(launch::t("Launch Center")).shortcut_text("Ctrl+Shift+O");
                if ui.add(lc).clicked() {
                    ui.close();
                    self.launch.open = self.doc.is_some();
                }
                self.remote_menu(ui);
                // FileMaker saves by itself and has no Save (Ctrl+S sorts).
                if ui
                    .add_enabled(self.doc.is_some(), egui::Button::new(tr("Save")))
                    .clicked()
                {
                    ui.close();
                    self.save(&ui.ctx().clone(), false);
                }
                if ui.button(tr("Save a Copy As…")).clicked() {
                    ui.close();
                    self.save(&ui.ctx().clone(), true);
                }
                ui.separator();
                self.file_menu_items(ui);
                ui.menu_button(security_ui::t("Manage"), |ui| {
                    let on = self.doc.as_ref().is_some_and(|d| d.db.full_access());
                    if ui
                        .add_enabled(on, egui::Button::new(security_ui::t("Security…")))
                        .clicked()
                    {
                        ui.close();
                        self.open_security();
                    }
                    if ui
                        .add_enabled(
                            on,
                            egui::Button::new(custom_functions::t("Custom Functions…")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.open_custom_functions(ui.ctx());
                    }
                });
                ui.separator();
                let close = egui::Button::new(tr("Close")).shortcut_text("Ctrl+W");
                if ui.add_enabled(self.doc.is_some(), close).clicked() {
                    ui.close();
                    self.close_window();
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    if ui
                        .add(egui::Button::new(tr("Exit")).shortcut_text("Ctrl+Q"))
                        .clicked()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            ui.menu_button(tr("Edit"), |ui| {
                let Some(ds) = self.designer.as_mut() else {
                    self.edit_menu(ui);
                    return;
                };
                let undo = egui::Button::new(tr("Undo")).shortcut_text("Ctrl+Z");
                if ui.add_enabled(ds.can_undo(), undo).clicked() {
                    ds.undo();
                    ui.close();
                }
                let redo = egui::Button::new(tr("Redo")).shortcut_text("Ctrl+Y");
                if ui.add_enabled(ds.can_redo(), redo).clicked() {
                    ds.redo();
                    ui.close();
                }
                ui.separator();
                let any = ds.has_selection();
                if ui
                    .add_enabled(any, egui::Button::new(tr("Cut")).shortcut_text("Ctrl+X"))
                    .clicked()
                {
                    ds.copy();
                    ds.delete_selection();
                    ui.close();
                }
                if ui
                    .add_enabled(any, egui::Button::new(tr("Copy")).shortcut_text("Ctrl+C"))
                    .clicked()
                {
                    ds.copy();
                    ui.close();
                }
                if ui
                    .add(egui::Button::new(tr("Paste")).shortcut_text("Ctrl+V"))
                    .clicked()
                {
                    ds.paste();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        any,
                        egui::Button::new(tr("Duplicate")).shortcut_text("Ctrl+D"),
                    )
                    .clicked()
                {
                    ds.duplicate();
                    ui.close();
                }
                if ui
                    .add(egui::Button::new(tr("Delete")).shortcut_text("Del"))
                    .clicked()
                {
                    ds.delete_selection();
                    ui.close();
                }
                if ui
                    .add(egui::Button::new(tr("Select All")).shortcut_text("Ctrl+A"))
                    .clicked()
                {
                    ds.select_all();
                    ui.close();
                }
            });
            ui.menu_button(tr("View"), |ui| {
                let on = self.doc.is_some();
                let browse = egui::Button::new(tr("Browse Mode")).shortcut_text("Ctrl+B");
                if ui.add_enabled(on, browse).clicked() {
                    self.browse_mode();
                    ui.close();
                }
                let find = egui::Button::new(tr("Find Mode")).shortcut_text("Ctrl+F");
                if ui
                    .add_enabled(on && self.mode() != Mode::Layout, find)
                    .clicked()
                {
                    if self.mode() != Mode::Find {
                        self.enter_find();
                    }
                    ui.close();
                }
                let layout = egui::Button::new(tr("Layout Mode")).shortcut_text("Ctrl+L");
                if ui.add_enabled(on, layout).clicked() {
                    self.enter_layout();
                    ui.close();
                }
                let preview = egui::Button::new(tr("Preview Mode")).shortcut_text("Ctrl+U");
                if ui
                    .add_enabled(on && self.mode() != Mode::Layout, preview)
                    .clicked()
                {
                    if self.mode() != Mode::Preview {
                        self.toggle_preview();
                    }
                    ui.close();
                }
                self.view_menu_more(ui);
            });
            ui.menu_button(tr("Insert"), |ui| {
                if let Some(ds) = self.designer.as_mut() {
                    ds.insert_menu(ui);
                    return;
                }
                // Into the container field last clicked (Browse mode).
                let target = container::active(ui.ctx()).filter(|_| self.mode() == Mode::Browse);
                let pick = egui::Button::new(tr("Picture or File…"));
                if ui.add_enabled(target.is_some(), pick).clicked() {
                    if let (Some(tg), Some(d)) = (target, self.doc.as_mut())
                        && container::perform(&mut d.db, container::Action::Insert, tg)
                    {
                        self.dirty = true;
                    }
                    ui.close();
                }
                self.insert_menu_items(ui);
            });
            ui.menu_button(tr("Format"), |ui| {
                match (self.designer.as_mut(), self.doc.as_ref()) {
                    (Some(ds), Some(d)) => ds.format_menu(ui, d),
                    _ => {
                        ui.label(tr("Not implemented yet"));
                    }
                }
            });
            // Layout mode adds the Layouts and Arrange menus.
            if let (Some(ds), Some(d)) = (self.designer.as_mut(), self.doc.as_ref()) {
                ui.menu_button(tr("Layouts"), |ui| ds.layouts_menu(ui, d));
                ui.menu_button(tr("Arrange"), |ui| ds.arrange_menu(ui));
            }
            if self.mode() == Mode::Find {
                ui.menu_button(toolbar::tx("Requests"), |ui| self.requests_menu(ui));
            } else {
                ui.menu_button(tr("Records"), |ui| self.records_menu(ui));
            }
            ui.menu_button(tr("Scripts"), |ui| {
                let ws = egui::Button::new(tr("Script Workspace…")).shortcut_text("Ctrl+Shift+S");
                if ui.add_enabled(self.engine.is_some(), ws).clicked() {
                    self.open_workspace();
                    focus_tool_window(ui.ctx(), workspace::KEY);
                    ui.close();
                }
                ui.separator();
                let list: Vec<(u32, String)> = self
                    .engine
                    .as_ref()
                    .map(|e| {
                        e.scripts
                            .iter()
                            .filter(|s| !s.is_folder() && !s.steps.is_empty())
                            .filter(|s| {
                                self.doc.as_ref().is_none_or(|d| {
                                    d.db.script_access(s.id) != fmp12::security::NO_ACCESS
                                })
                            })
                            .map(|s| (s.id, s.name.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                if list.is_empty() {
                    ui.label(tr("No scripts"));
                }
                for (id, name) in list {
                    if ui.button(name).clicked() {
                        self.pending_script = Some(id);
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr("Tools"), |ui| {
                if ui.button(tr("Data Viewer")).clicked() {
                    self.data_viewer = true;
                    focus_tool_window(ui.ctx(), DATA_VIEWER);
                    ui.close();
                }
                if ui
                    .checkbox(&mut self.debugger.open, tr("Script Debugger"))
                    .clicked()
                {
                    focus_tool_window(ui.ctx(), debugger::KEY);
                    ui.close();
                }
            });
            ui.menu_button(tr("Window"), |ui| self.window_menu(ui));
            ui.menu_button(tr("Help"), |ui| {
                ui.label(tr("Not implemented yet"));
            });
        });
    }

    fn open_workspace(&mut self) {
        if let Some(e) = self.engine.as_ref() {
            self.workspace.show_scripts(&e.scripts);
        }
    }

    /// Draws the current layout: form (current record), list (body part
    /// repeated per record) or table (one row per record).
    fn content(&mut self, ui: &mut egui::Ui) {
        let Some(t) = self.table() else { return };
        let record = self.current_record();
        let found = self.found(t).clone();
        let pos = self.current.get(&t).copied().unwrap_or(0);
        let mode = self.mode();
        let view = self.view;
        let globals = self
            .engine
            .as_ref()
            .map(|e| e.globals.clone())
            .unwrap_or_default();
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(layout) = doc.layouts.get(self.layout).cloned() else {
            return;
        };
        if mode == Mode::Preview {
            self.pages = preview::draw(ui, doc, &layout, &found, &self.last_sort, self.page);
            self.page = self.page.min(self.pages.saturating_sub(1));
            return;
        }
        let theme = doc
            .db
            .themes
            .iter()
            .find(|x| x.number == layout.theme)
            .cloned()
            // With the styles saved in the layout itself.
            .map(|mut t| {
                let own = fmp12::theme::Theme::parse_styles(&layout.styles_raw);
                t.styles.extend(own);
                t
            });
        let looks: Vec<Look> = layout
            .objects
            .iter()
            .map(|o| {
                look::resolve(
                    theme.as_ref(),
                    &fmp12::theme::ObjectStyle::parse(&o.style_raw),
                )
            })
            .collect();
        let layout_look = look::resolve(
            theme.as_ref(),
            &fmp12::theme::ObjectStyle::of_layout(&layout),
        );
        if view == View::Table && mode == Mode::Browse {
            self.table_view(ui, t, &layout, &found);
            return;
        }
        let (btop, bh) = layout
            .parts
            .iter()
            .find(|p| p.kind == 4)
            .map_or((0.0, 0.0), |p| (p.top, p.height));
        let rows: Vec<Option<usize>> = match (view, mode) {
            (View::List, Mode::Browse) => found.iter().map(|r| Some(*r)).collect(),
            _ => vec![record],
        };
        let extra = rows.len().saturating_sub(1) as f64 * bh;
        let height = layout
            .parts
            .iter()
            .map(|p| p.top + p.height)
            .fold(0.0, f64::max)
            + extra;
        let (resp, painter) = ui.allocate_painter(
            Vec2::new(layout.width as f32, height as f32),
            egui::Sense::hover(),
        );
        let origin = resp.rect.min;
        let bg = if mode == Mode::Find {
            Color32::from_rgb(235, 240, 250)
        } else {
            layout_look.background.unwrap_or(Color32::WHITE)
        };
        painter.rect_filled(resp.rect, 0.0, bg);
        // Parts below the body move down by the extra list rows.
        let shift = |y: f64| if y >= btop + bh { y + extra } else { y };
        for p in &layout.parts {
            if mode == Mode::Browse {
                let pl = look::resolve(
                    theme.as_ref(),
                    &fmp12::theme::ObjectStyle::parse(&p.style_raw),
                );
                if let Some(c) = pl.background {
                    let (top, bottom) = if p.kind == 4 {
                        (p.top, p.top + p.height + extra)
                    } else {
                        (shift(p.top), shift(p.top + p.height))
                    };
                    painter.rect_filled(
                        Rect::from_min_max(
                            origin + Vec2::new(0.0, top as f32),
                            origin + Vec2::new(layout.width as f32, bottom as f32),
                        ),
                        0.0,
                        c,
                    );
                }
            }
            let y = origin.y + shift(p.top + p.height) as f32;
            painter.line_segment(
                [
                    Pos2::new(origin.x, y),
                    Pos2::new(origin.x + layout.width as f32, y),
                ],
                Stroke::new(0.5, Color32::from_gray(210)),
            );
        }
        // List view: the current record's band highlighted, with
        // FileMaker's dark bar on its left; a click selects a record.
        let mut clicked_row = None;
        if view == View::List && mode == Mode::Browse && bh > 0.0 {
            let band = |i: usize| {
                Rect::from_min_size(
                    origin + Vec2::new(0.0, (btop + i as f64 * bh) as f32),
                    Vec2::new(layout.width as f32, bh as f32),
                )
            };
            let cur = band(pos);
            painter.rect_filled(cur, 0.0, Color32::from_rgba_unmultiplied(0, 90, 180, 24));
            // Layout Setup › "Show current record indicator in List view".
            if layout_setup::record_indicator(&layout) {
                painter.rect_filled(
                    Rect::from_min_size(cur.min, Vec2::new(4.0, cur.height())),
                    0.0,
                    Color32::from_gray(40),
                );
            }
            if let Some(p) =
                ui.input(|i| i.pointer.press_origin().filter(|_| i.pointer.any_pressed()))
                && resp.rect.contains(p)
                && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
            {
                let i = ((p.y - origin.y) as f64 - btop) / bh;
                if i >= 0.0 && (i as usize) < rows.len() {
                    clicked_row = Some(i as usize);
                }
            }
        }
        // The active record: the one being edited or with a field entered.
        let active = self
            .editing
            .or_else(|| {
                toolbar::last_focus(ui.ctx())
                    .filter(|f| ui.memory(|m| m.has_focus(f.id)))
                    .and_then(|f| f.record)
                    .map(|r| (r.0, r.1))
            })
            .filter(|a| a.0 == t)
            .map(|a| a.1);
        let row_ctx = |rec: Option<usize>| {
            let (frames, plain) = if mode == Mode::Browse {
                let list = view == View::List;
                layout_setup::field_marks(layout.options, list, rec, active, record)
            } else {
                (false, false)
            };
            DrawCtx {
                table: t,
                mode,
                frames,
                plain,
            }
        };
        let layout_to = (layout.table_occurrence & 0xFFF) as u32;
        let by_id: BTreeMap<u64, usize> = layout
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.id, i))
            .collect();
        let ancestors = |o: &fmp12::layouts::LayoutObject| {
            let mut v = Vec::new();
            let mut p = o.parent;
            while let Some(id) = p {
                let Some(&i) = by_id.get(&id) else { break };
                v.push(&layout.objects[i]);
                p = layout.objects[i].parent;
            }
            v
        };
        // Front panel of each tab control: the user's choice, else field 6, else the first panel.
        let front = |tc: &fmp12::layouts::LayoutObject, tabs: &BTreeMap<u64, u64>| {
            tabs.get(&tc.id).copied().or(tc.active_child).or_else(|| {
                layout
                    .objects
                    .iter()
                    .find(|x| x.parent == Some(tc.id) && x.kind == 12)
                    .map(|x| x.id)
            })
        };
        let mut clicked_tab = None;
        let tabs_before = self.tabs.clone();
        let browse = mode == Mode::Browse;
        if browse {
            objects::tab_navigation(ui, &layout, &mut self.objs);
        }
        // The object list runs front to back: paint it in reverse (the open
        // popover last).
        for i in objects::paint_order(&layout, &self.objs) {
            let (o, lk) = (&layout.objects[i], &looks[i]);
            let anc = ancestors(o);
            // Hidden behind another tab, or drawn by its portal.
            let hidden = anc.iter().any(|a| {
                a.kind == 12
                    && a.parent
                        .and_then(|tc| by_id.get(&tc))
                        .is_some_and(|&i| front(&layout.objects[i], &self.tabs) != Some(a.id))
            });
            // Panels are drawn by their tab control; portal rows by their portal.
            if hidden || o.kind == 12 || anc.iter().any(|a| a.kind == 9) {
                continue;
            }
            let ab = layout.absolute(o);
            let in_body = ab.top >= btop && ab.top < btop + bh;
            let instances: Vec<(f64, Option<usize>)> = if in_body {
                rows.iter()
                    .enumerate()
                    .map(|(i, r)| (i as f64 * bh, *r))
                    .collect()
            } else {
                vec![(shift(ab.top) - ab.top, record)]
            };
            for (dy, rec) in instances {
                let r = Rect::from_min_max(
                    origin + Vec2::new(ab.left as f32, (ab.top + dy) as f32),
                    origin + Vec2::new(ab.right as f32, (ab.bottom + dy) as f32),
                );
                let oc = objects::context(t, layout_to, rec);
                let skip = if browse {
                    objects::skipped(doc, &layout, o, &self.objs, &self.tabs, oc)
                } else {
                    // Find mode: no record to test "hide object when" on.
                    o.kind == fmp12::layouts::kind::SLIDE_PANEL
                        || objects::popover_of(&layout, o)
                            .is_some_and(|p| self.objs.popover != Some(p))
                };
                if skip {
                    continue;
                }
                let cond_look;
                let lk = if browse && !o.conditions.is_empty() {
                    let owner = owner_of(doc, o, t, layout_to, rec);
                    let v = objects::object_value(doc, o, owner);
                    cond_look = objects::conditional_look(doc, theme.as_ref(), o, &v, oc);
                    &cond_look
                } else {
                    lk
                };
                match o.kind {
                    14 => objects::popover_button(ui, &painter, doc, o, lk, r, &mut self.objs, oc),
                    15 => objects::popover(ui, &painter, doc, o, lk, r, &mut self.objs, oc),
                    16 => objects::slide_control(ui, &painter, &layout, o, lk, r, &mut self.tabs),
                    13 if objects::web_viewer(ui, &painter, doc, o, lk, r, &mut self.objs, oc) => {}
                    // Tab control: one clickable tab per panel.
                    11 => {
                        lk.paint_box(&painter, r);
                        let active = front(o, &self.tabs);
                        let mut x = r.left();
                        let mut panels: Vec<_> = layout
                            .objects
                            .iter()
                            .filter(|p| p.parent == Some(o.id))
                            .collect();
                        // In the order and at the place of their tabs (offset,
                        // width); a panel without them after the others.
                        panels.sort_by_key(|p| (p.tab.is_none(), p.tab.map(|t| t.0), p.id));
                        for p in panels {
                            let label = p.text.clone().unwrap_or_default();
                            let w = (label.chars().count() as f32 * 8.0 + 32.0).max(60.0);
                            let (tx, w) = match p.tab {
                                Some((off, tw)) if tw > 0 => (r.left() + off as f32, tw as f32),
                                _ => (x, w),
                            };
                            let tab =
                                Rect::from_min_size(egui::pos2(tx, r.top()), Vec2::new(w, 30.0));
                            let on = active == Some(p.id);
                            painter.rect_filled(
                                tab,
                                0.0,
                                if on {
                                    Color32::WHITE
                                } else {
                                    Color32::from_gray(232)
                                },
                            );
                            painter.rect_stroke(
                                tab,
                                0.0,
                                Stroke::new(1.0, Color32::from_gray(200)),
                                StrokeKind::Inside,
                            );
                            painter.text(
                                tab.center(),
                                egui::Align2::CENTER_CENTER,
                                &label,
                                lk.font(),
                                Color32::from_gray(if on { 20 } else { 110 }),
                            );
                            let id = ui.scope_id().with(("tab", o.id, p.id));
                            if ui.interact(tab, id, egui::Sense::click()).clicked() {
                                clicked_tab = Some((o.id, p.id));
                            }
                            x = tab.right();
                        }
                    }
                    // Portal: one row per related record.
                    9 => {
                        lk.paint_box(&painter, r);
                        let Some((pto, nrows, first)) = o.portal else {
                            continue;
                        };
                        let pto = (pto & 0xFFF) as u32;
                        let Some(ptable) = doc.db.table_of(pto).map(|x| x.id) else {
                            continue;
                        };
                        let related = match (mode, rec) {
                            (Mode::Browse, Some(rr)) => doc.db.related(layout_to, rr, pto),
                            _ => Vec::new(),
                        };
                        let nrows = nrows.max(1) as usize;
                        let row_h = (o.bounds.bottom - o.bounds.top) / nrows as f64;
                        let children: Vec<usize> = layout
                            .objects
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.parent == Some(o.id))
                            .map(|(i, _)| i)
                            .collect();
                        for row in 0..nrows {
                            let related_rec =
                                related.get(first.saturating_sub(1) as usize + row).copied();
                            let y0 = r.top() + (row as f64 * row_h) as f32;
                            if row > 0 {
                                painter.line_segment(
                                    [egui::pos2(r.left(), y0), egui::pos2(r.right(), y0)],
                                    Stroke::new(0.5, Color32::from_gray(220)),
                                );
                            }
                            // Find mode: the first row takes criteria.
                            let rr = match related_rec {
                                Some(rr) => Some(rr),
                                None if mode == Mode::Find && row == 0 => None,
                                None => continue,
                            };
                            for &ci in &children {
                                let c = &layout.objects[ci];
                                let cr = Rect::from_min_size(
                                    egui::pos2(
                                        r.left() + c.bounds.left as f32,
                                        y0 + c.bounds.top as f32,
                                    ),
                                    Vec2::new(
                                        (c.bounds.right - c.bounds.left) as f32,
                                        (c.bounds.bottom - c.bounds.top) as f32,
                                    ),
                                );
                                // Merge fields show the row's record.
                                if c.text.as_ref().is_some_and(|x| x.contains("<<"))
                                    || !c.text_runs.is_empty()
                                {
                                    looks[ci].paint_box(&painter, cr);
                                    looks[ci].paint_styled(&painter, cr, c, |s| {
                                        merge_fields(doc, &globals, s, ptable, pto, rr)
                                    });
                                    continue;
                                }
                                draw_object(
                                    ui,
                                    &painter,
                                    doc,
                                    &row_ctx(rec),
                                    c,
                                    &looks[ci],
                                    cr,
                                    Some((ptable, rr)),
                                    &mut self.request,
                                    &mut self.dirty,
                                    &mut self.obj_events,
                                );
                            }
                        }
                    }
                    13 if chart::draw(ui.ctx(), &painter, doc, &layout, o, r, &found, rec) => {}
                    _ => {
                        if let Some(step) = o.button.as_ref().filter(|_| mode == Mode::Browse) {
                            let id = ui.scope_id().with(("button", o.id, rec));
                            if ui
                                .interact(r, id, egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.pending_step = Some(step.clone());
                            }
                        }
                        if let Some(l) = o.label.as_ref().filter(|_| matches!(o.kind, 10 | 18)) {
                            let text = doc.db.eval_tokens(l, t, layout_to, rec).to_text();
                            lk.paint_box(&painter, r);
                            objects::segment_highlight(&painter, &layout, o, r);
                            lk.paint_text(&painter, r, &text);
                            continue;
                        }
                        if o.text.as_ref().is_some_and(|x| x.contains("<<"))
                            || !o.text_runs.is_empty()
                        {
                            lk.paint_box(&painter, r);
                            lk.paint_styled(&painter, r, o, |s| {
                                merge_fields(doc, &globals, s, t, layout_to, rec)
                            });
                            continue;
                        }
                        let owner = owner_of(doc, o, t, layout_to, rec);
                        draw_object(
                            ui,
                            &painter,
                            doc,
                            &row_ctx(rec),
                            o,
                            lk,
                            r,
                            owner,
                            &mut self.request,
                            &mut self.dirty,
                            &mut self.obj_events,
                        );
                    }
                }
            }
        }
        if let Some(i) = clicked_row.filter(|i| *i != pos) {
            self.go_to(i);
        }
        if let Some((tc, p)) = clicked_tab {
            self.tabs.insert(tc, p);
        }
        self.panel_switches(&tabs_before);
        if browse {
            objects::close_popover_on_click(ui, &mut self.objs);
            for (id, ev) in &self.obj_events {
                if *ev == event::OBJECT_ENTER {
                    self.objs.focus = Some(*id);
                }
            }
        }
    }
}

impl App {
    /// Layout mode dialogs: the designer's own, and "save changes?".
    fn layout_dialogs(&mut self, ctx: &egui::Context) {
        let scripts: Vec<(u32, String)> = self
            .engine
            .as_ref()
            .map(|e| {
                e.scripts
                    .iter()
                    .filter(|s| !s.is_folder() && !s.steps.is_empty())
                    .map(|s| (s.id, s.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let (Some(ds), Some(d)) = (self.designer.as_mut(), self.doc.as_mut()) else {
            return;
        };
        ds.dialogs(ctx, d, &scripts);
        if let Some(t) = ds.pending.take() {
            // Deleting the layout needs no "save changes?" first.
            if t == design::Then::Op(design::LayoutOp::Delete) {
                self.finish_layout(t);
            } else {
                self.leave_layout(t);
            }
            return;
        }
        let Some(then) = ds.ask.clone() else { return };
        let mut answer = None;
        egui::Modal::new(egui::Id::NULL.with("save_layout")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(i18n::save_layout_question(&ds.layout.name));
            ui.horizontal(|ui| {
                if ui.button(tr("Save")).clicked() {
                    answer = Some(true);
                }
                if ui.button(tr("Don't Save")).clicked() {
                    answer = Some(false);
                }
                if ui.button(tr("Cancel")).clicked() {
                    ds.ask = None;
                }
            });
        });
        if let Some(save) = answer {
            ds.ask = None;
            if save {
                ds.apply(d);
                self.dirty = true;
            }
            self.finish_layout(then);
        }
    }
}

/// Replaces `<<Field>>` and `<<TO::Field>>` merge fields in layout text.
fn merge_fields(
    doc: &Doc,
    globals: &Globals,
    text: &str,
    t: u32,
    layout_to: u32,
    rec: Option<usize>,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("<<") {
        out.push_str(&rest[..a]);
        let Some(b) = rest[a..].find(">>") else {
            out.push_str(&rest[a..]);
            return out;
        };
        let name = &rest[a + 2..a + b];
        // Merge variable `<<$$name>>` (globals keep lower-case names).
        if name.starts_with("$$") {
            if let Some(v) = globals.get(&(name.to_lowercase(), 1)) {
                out.push_str(&v.to_text());
            }
            rest = &rest[a + b + 2..];
            continue;
        }
        let value = doc.db.merge_field(name, layout_to).and_then(|(to, f)| {
            let tbl = doc.db.table_of(to)?;
            let r = if to == layout_to {
                rec?
            } else {
                *doc.db.related(layout_to, rec?, to).first()?
            };
            let _ = t;
            Some(doc.db.value(tbl.id, r, f).to_text())
        });
        out.push_str(&value.unwrap_or_default());
        rest = &rest[a + b + 2..];
    }
    out.push_str(rest);
    out
}

/// `$$` variables of the script engine, by (lower-case name, repetition).
pub(crate) type Globals = std::collections::HashMap<(String, usize), calc::Value>;

/// Table and record whose value a field object shows: the layout's own
/// record, or the first related record for a field of another occurrence.
fn owner_of(
    doc: &Doc,
    o: &fmp12::layouts::LayoutObject,
    t: u32,
    layout_to: u32,
    rec: Option<usize>,
) -> Option<(u32, Option<usize>)> {
    let (to, _, _) = o.field?;
    let to = (to & 0xFFF) as u32;
    let ft = doc.db.table_of(to)?.id;
    // Another occurrence of the layout's table shows the layout's record
    // only when no relationship leads to it.
    let same = ft == t && doc.db.table_of(layout_to).map(|x| x.id) == Some(ft);
    if to == layout_to || same && !doc.db.is_related(layout_to, to) {
        return Some((t, rec));
    }
    let first = rec.and_then(|r| doc.db.related(layout_to, r, to).first().copied());
    Some((ft, first))
}

/// Hash of every record value, to tell whether a script changed data.
fn data_fingerprint(db: &Database) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for t in &db.tables {
        t.id.hash(&mut h);
        for r in &t.records {
            r.id.hash(&mut h);
            for (k, v) in &r.values {
                k.hash(&mut h);
                v.hash(&mut h);
            }
        }
    }
    h.finish()
}

/// The app as a script host. Dialogs and pauses suspend the run (see
/// `App::start_run`), so `Host::dialog` is not used.
struct AppHost<'a> {
    db: &'a mut Database,
    window: script::Window,
    /// Data file steps: the file's folder and the writes the user agreed to.
    disk: script::files::Disk,
    /// The file's script triggers, fired by steps (`None` in a triggered
    /// script).
    triggers: Option<&'a fmp12::triggers::Triggers>,
}

impl script::Host for AppHost<'_> {
    fn db(&mut self) -> &mut Database {
        self.db
    }
    fn window(&mut self) -> &mut script::Window {
        &mut self.window
    }
    fn trigger(
        &mut self,
        ev: u16,
        object: Option<u64>,
    ) -> Option<(u32, Option<fmp12::scripts::Calc>)> {
        let t = self.triggers?;
        let l = self.window.layout;
        let bit = match self.window.mode {
            script::Mode::Browse => fmp12::triggers::mode::BROWSE,
            script::Mode::Find => fmp12::triggers::mode::FIND,
            script::Mode::Preview => fmp12::triggers::mode::PREVIEW,
        };
        let found = match object {
            Some(o) => t.object(l, o, ev),
            None if ev >= 200 => t.file(ev),
            None => t.layout(l, ev, bit),
        };
        found.map(|t| (t.script, t.parameter.clone()))
    }
    fn pick_file(&mut self, picture: bool) -> Option<(String, Vec<u8>)> {
        container::pick_file(picture)
    }
    fn save_file(&mut self, name: &str, data: &[u8]) -> bool {
        container::save_file(name, data)
    }
    fn file_stat(&mut self, path: &str) -> Result<Option<script::FileStat>, u32> {
        self.disk.stat(path)
    }
    fn file_read(&mut self, path: &str) -> Result<Vec<u8>, u32> {
        self.disk.read(path)
    }
    fn file_write(&mut self, path: &str, data: &[u8], folders: bool) -> Result<(), u32> {
        self.disk.write(path, data, folders)
    }
    fn file_delete(&mut self, path: &str) -> Result<(), u32> {
        self.disk.delete(path)
    }
    fn file_rename(&mut self, from: &str, to: &str) -> Result<(), u32> {
        self.disk.rename(from, to)
    }
    fn file_confirm(&mut self, path: &str) {
        self.disk.confirm(path);
    }
}

struct DrawCtx {
    table: u32,
    mode: Mode,
    /// Field frames drawn (the active record, Layout Setup).
    frames: bool,
    /// Fields drawn without fill or border (a list row other than the
    /// current one, "Delineate fields on current row only").
    plain: bool,
}

#[allow(clippy::too_many_arguments)]
fn draw_object(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    doc: &mut Doc,
    c: &DrawCtx,
    o: &fmp12::layouts::LayoutObject,
    lk: &Look,
    r: Rect,
    owner: Option<(u32, Option<usize>)>,
    request: &mut BTreeMap<u32, String>,
    dirty: &mut bool,
    events: &mut Vec<(u64, u16)>,
) {
    if !(c.plain && o.kind == 1) {
        lk.paint_shape(painter, r, o.kind);
    }
    if let Some(p) = o.picture.and_then(|id| doc.db.pictures.get(&id)) {
        container::picture(ui.ctx(), painter, r, p.data());
    }
    if o.kind != 1 {
        if o.text.is_some() {
            lk.paint_styled(painter, r, o, str::to_string);
        }
        return;
    }
    let (Some((_, field, first)), Some((t, record))) = (o.field, owner) else {
        return;
    };
    let field = field as u32;
    let def = doc.db.table(t).and_then(|x| x.field(field));
    let reps = def.map_or(1, engine::repetitions);
    // ponytail: the object's last repetition shown is not decoded: from its
    // first one, as many of the field's repetitions as fit in its bounds.
    let first = (first as u32).clamp(1, reps);
    let fit = (r.height() / (lk.font().size + 8.0)).floor().max(1.0) as u32;
    let shown = (reps - first + 1).min(fit);
    let h = r.height() / shown as f32;
    for rep in first..first + shown {
        let top = r.top() + (rep - first) as f32 * h;
        let rr = Rect::from_min_size(egui::pos2(r.left(), top), Vec2::new(r.width(), h));
        if shown > 1 && rep > first {
            let line = Stroke::new(0.5, Color32::from_gray(200));
            painter.line_segment([rr.left_top(), rr.right_top()], line);
        }
        let target = (t, record, field, rep);
        draw_field(
            ui, painter, doc, c, o, lk, rr, target, request, dirty, events,
        );
    }
    if c.frames {
        painter.rect_stroke(
            r,
            0.0,
            Stroke::new(1.0, Color32::from_gray(150)),
            StrokeKind::Inside,
        );
    }
}

/// One repetition of a field object.
#[allow(clippy::too_many_arguments)]
fn draw_field(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    doc: &mut Doc,
    c: &DrawCtx,
    o: &fmp12::layouts::LayoutObject,
    lk: &Look,
    r: Rect,
    (t, record, field, rep): (u32, Option<usize>, u32, u32),
    request: &mut BTreeMap<u32, String>,
    dirty: &mut bool,
    events: &mut Vec<(u64, u16)>,
) {
    let key = engine::rep_key(field, rep);
    let def = doc.db.table(t).and_then(|x| x.field(field));
    let is_calc = def.is_some_and(|f| f.calc.is_some());
    let is_container = def.is_some_and(|f| f.data_type == fmp12::schema::DataType::Container);
    let inner = lk.content_rect(r);
    match (c.mode, record) {
        (Mode::Find, _) if rep == 1 => {
            // Related fields: criteria keyed by their occurrence.
            let field = o
                .field
                .map(|(to, _, _)| (to & 0xFFF) as u32)
                .filter(|_| t != c.table)
                .map_or(field, |to| engine::related_key(to, field));
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_gray(150)),
                StrokeKind::Inside,
            );
            let s = request.entry(field).or_default();
            let id = ui.scope_id().with(("find", o.id, field));
            // Checkbox and radio button sets stay sets in a find request.
            if let (3 | 4, Some(vl)) = (o.display, o.value_list) {
                let values = doc.db.value_list_values(vl);
                if let Some(v) = ui
                    .push_id(id, |ui| {
                        controls::value_set(ui, lk, inner, o.display, &values, s)
                    })
                    .inner
                {
                    *s = v;
                }
            } else if ui
                .put(lk.edit_rect(ui.ctx(), inner), lk.edit(s).id(id))
                .has_focus()
            {
                toolbar::set_focus(
                    ui.ctx(),
                    toolbar::Focus {
                        id,
                        record: None,
                        find: Some(field),
                    },
                );
            }
        }
        (Mode::Find | Mode::Layout | Mode::Preview, _) | (Mode::Browse, None) => {}
        // FileMaker draws <No Access> for a field the account cannot see.
        (Mode::Browse, Some(rec)) if !doc.db.can_see(t, rec, field) => {
            lk.paint_text(painter, r, "<No Access>");
        }
        (Mode::Browse, Some(rec)) if is_container => {
            let v = doc.db.value_rep(t, rec, field, rep);
            // Scoped by object: a field shown twice is two widgets.
            if let Some(a) = ui
                .push_id(o.id, |ui| {
                    container::draw(ui, painter, lk, r, &v, (t, rec, field, rep))
                })
                .inner
                && container::perform(&mut doc.db, a, (t, rec, field, rep))
            {
                *dirty = true;
            }
        }
        // Calculations, and fields the privileges make read-only.
        (Mode::Browse, Some(rec)) if is_calc || !doc.db.can_edit_field(t, rec, field) => {
            let value = doc.db.value_rep(t, rec, field, rep);
            if controls::concealed(o) {
                let n = value.to_text().chars().count();
                lk.paint_text(painter, r, &"\u{2022}".repeat(n));
                return;
            }
            match objects::formatted(doc, o, &value) {
                Some((s, color)) => {
                    let mut l2 = lk.clone();
                    l2.color = color.unwrap_or(lk.color);
                    l2.paint_text(painter, r, &s);
                }
                None => lk.paint_text(painter, r, &value.to_text().replace('\r', " ")),
            }
        }
        (Mode::Browse, Some(rec)) => {
            // Placeholder text (a calculation, usually a literal).
            let placeholder = o
                .placeholder
                .as_ref()
                .map(|l| doc.db.eval_tokens(l, t, 0, Some(rec)).to_text());
            let tip = o
                .tooltip
                .as_ref()
                .map(|l| doc.db.eval_tokens(l, t, 0, Some(rec)).to_text());
            // Data formats show while the field is not being edited.
            let edit_id = ui.scope_id().with(("field", o.id, rec, rep));
            let shown = if ui.memory(|m| m.has_focus(edit_id)) || controls::concealed(o) {
                None
            } else {
                objects::formatted(doc, o, &doc.db.value_rep(t, rec, field, rep))
            };
            if ui.ctx().data(|d| d.get_temp::<u64>(objects::focus_id())) == Some(o.id) {
                ui.ctx().data_mut(|d| d.remove::<u64>(objects::focus_id()));
                ui.memory_mut(|m| m.request_focus(edit_id));
            }
            // Pop-up menus and drop-down lists offer their value list.
            // So do checkbox sets and radio button sets.
            let choices = match (o.display, o.value_list) {
                (1..=4, Some(vl)) => Some(doc.db.value_list_values(vl)),
                _ => None,
            };
            // Lists showing only their second field's values show those.
            let labels = match (o.display, o.value_list) {
                (1 | 2, Some(vl)) => doc.db.value_list_labels(vl),
                _ => None,
            }
            .unwrap_or_default();
            let label = |v: &String| labels.get(v).cloned().unwrap_or_else(|| v.clone());
            // A drop-down calendar writes the day in the file's date format.
            let calendar = (o.display == 6).then(|| doc.db.locale.clone());
            let cal_id = edit_id.with("calendar");
            // Edit boxes set to auto-complete offer the values entered in
            // other records while the field is being edited (or the list
            // is under the pointer, so that a click can reach it).
            let list_id = edit_id.with("complete");
            let completions = if o.display == 0
                && o.field_flags & fmp12::layouts::field_flags::AUTO_COMPLETE != 0
                && (ui.memory(|m| m.has_focus(edit_id))
                    || ui.data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false))
            {
                let typed = doc.db.value_rep(t, rec, field, rep).to_text();
                doc.db.completions(t, field, &typed)
            } else {
                Vec::new()
            };
            let mut changed = false;
            let mut left = false;
            if let Some(s) = doc
                .db
                .tables
                .iter_mut()
                .find(|x| x.id == t)
                .and_then(|x| x.raw_mut(rec, key))
            {
                let before = s.clone();
                match choices {
                    Some(values) => {
                        let mut picked = None;
                        if o.display >= 3 {
                            picked = ui
                                .push_id(edit_id, |ui| {
                                    controls::value_set(ui, lk, inner, o.display, &values, s)
                                })
                                .inner;
                        } else {
                            // A drop-down list without "Include arrow icon"
                            // looks like an edit box (the field's own look).
                            let plain = o.display == 1
                                && o.field_flags & fmp12::layouts::field_flags::ARROW == 0;
                            ui.put(inner, |ui: &mut egui::Ui| {
                                if plain {
                                    let w = &mut ui.visuals_mut().widgets;
                                    for v in [
                                        &mut w.inactive,
                                        &mut w.hovered,
                                        &mut w.active,
                                        &mut w.open,
                                    ] {
                                        v.weak_bg_fill = Color32::TRANSPARENT;
                                        v.bg_stroke = Stroke::NONE;
                                        v.fg_stroke.color = lk.color;
                                    }
                                }
                                let combo = egui::ComboBox::from_id_salt(("vl", o.id, rec, rep));
                                let combo = if plain {
                                    combo.icon(|_, _, _, _| {})
                                } else {
                                    combo
                                };
                                combo
                                    .selected_text(egui::RichText::new(label(s)).font(lk.font()))
                                    .width(inner.width())
                                    .show_ui(ui, |ui| {
                                        for v in &values {
                                            if ui.selectable_label(s == v, label(v)).clicked() {
                                                picked = Some(v.clone());
                                            }
                                        }
                                    })
                                    .response
                            });
                        }
                        if let Some(v) = picked {
                            editing::record(
                                ui.ctx(),
                                editing::Edit {
                                    table: t,
                                    record: rec,
                                    key,
                                    before: before.clone(),
                                    after: v.clone(),
                                },
                            );
                            *s = v;
                            changed = true;
                            left = true;
                            events.push((o.id, event::OBJECT_MODIFY));
                        }
                    }
                    None => {
                        if let Some(ph) = placeholder.as_ref().filter(|_| s.is_empty()) {
                            painter.text(
                                inner.left_center(),
                                egui::Align2::LEFT_CENTER,
                                ph,
                                lk.font(),
                                Color32::from_gray(170),
                            );
                        }
                        let r = match &shown {
                            Some((text, color)) => {
                                let mut l2 = lk.clone();
                                l2.color = color.unwrap_or(lk.color);
                                l2.paint_text(painter, inner, text);
                                let click = ui.interact(
                                    inner,
                                    edit_id.with("formatted"),
                                    egui::Sense::click(),
                                );
                                if click.clicked() {
                                    ui.memory_mut(|m| m.request_focus(edit_id));
                                }
                                click
                            }
                            None => ui.put(
                                lk.edit_rect(ui.ctx(), inner),
                                lk.edit(s).id(edit_id).password(controls::concealed(o)),
                            ),
                        };
                        // Drop-down calendar: the icon (Inspector "Include
                        // icon…", bit 0x400) and the month below the field
                        // while it is active.
                        let mut cal_over = false;
                        if let Some(loc) = &calendar {
                            let icon = o.field_flags & fmp12::layouts::field_flags::ARROW != 0;
                            if icon
                                && ui
                                    .interact(
                                        controls::calendar_icon(painter, inner, lk.color),
                                        cal_id.with("icon"),
                                        egui::Sense::click(),
                                    )
                                    .clicked()
                            {
                                ui.memory_mut(|m| m.request_focus(edit_id));
                            }
                            if r.has_focus()
                                || ui.data(|d| d.get_temp::<bool>(cal_id)).unwrap_or(false)
                            {
                                let day = engine::typed_in(s, fmp12::schema::DataType::Date, loc)
                                    .to_date();
                                let (pick, over) = controls::calendar(ui, cal_id, inner, day);
                                cal_over = over;
                                if let Some(d) = pick {
                                    let v = engine::stored_text(&calc::Value::Date(d), loc);
                                    editing::record(
                                        ui.ctx(),
                                        editing::Edit {
                                            table: t,
                                            record: rec,
                                            key,
                                            before: s.clone(),
                                            after: v.clone(),
                                        },
                                    );
                                    // The edit is recorded: leaving the field does not again.
                                    ui.data_mut(|d| {
                                        d.insert_temp(edit_id.with("before"), v.clone())
                                    });
                                    *s = v;
                                    changed = true;
                                    left = true;
                                    cal_over = false;
                                    events.push((o.id, event::OBJECT_MODIFY));
                                    ui.memory_mut(|m| m.surrender_focus(edit_id));
                                }
                            }
                            ui.data_mut(|d| d.insert_temp(cal_id, cal_over));
                        }
                        if let Some(tip) = &tip {
                            r.clone().on_hover_text(tip.as_str());
                        }
                        let was = edit_id.with("before");
                        // Back after a cancelled OnObjectValidate: no OnObjectEnter.
                        let back = edit_id.with("refocus");
                        if r.gained_focus() {
                            if ui.data_mut(|d| d.remove_temp::<bool>(back)).is_none() {
                                events.push((o.id, event::OBJECT_ENTER));
                            }
                            ui.data_mut(|d| d.insert_temp(was, before.clone()));
                        }
                        if r.has_focus() {
                            toolbar::set_focus(
                                ui.ctx(),
                                toolbar::Focus {
                                    id: edit_id,
                                    record: Some((t, rec, key)),
                                    find: None,
                                },
                            );
                        }
                        // egui can report the loss twice: the field is left once.
                        let leaving = r
                            .lost_focus()
                            .then(|| ui.data_mut(|d| d.remove_temp::<String>(was)))
                            .flatten();
                        if let Some(b) = leaving.clone() {
                            editing::record(
                                ui.ctx(),
                                editing::Edit {
                                    table: t,
                                    record: rec,
                                    key,
                                    before: b,
                                    after: s.clone(),
                                },
                            );
                        }
                        if r.changed() {
                            changed = true;
                            events.push((o.id, event::OBJECT_MODIFY));
                        }
                        let mut over = false;
                        if !completions.is_empty() {
                            // ponytail: picked with the mouse only, no arrow keys.
                            let shown = egui::Area::new(list_id)
                                .order(egui::Order::Foreground)
                                .fixed_pos(inner.left_bottom())
                                .show(ui.ctx(), |ui| {
                                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                                        ui.set_min_width(inner.width());
                                        completions
                                            .iter()
                                            .take(12)
                                            .find(|v| ui.selectable_label(false, *v).clicked())
                                            .cloned()
                                    })
                                });
                            over = shown.response.contains_pointer();
                            if let Some(v) = shown.inner.inner {
                                editing::record(
                                    ui.ctx(),
                                    editing::Edit {
                                        table: t,
                                        record: rec,
                                        key,
                                        before: s.clone(),
                                        after: v.clone(),
                                    },
                                );
                                *s = v;
                                changed = true;
                                left = true;
                                over = false;
                                events.push((o.id, event::OBJECT_MODIFY));
                            }
                        }
                        ui.data_mut(|d| d.insert_temp(list_id, over));
                        if r.lost_focus() {
                            left = true;
                        }
                        if leaving.is_some() {
                            // A modified field: OnObjectValidate (which can
                            // keep the focus in it), then OnObjectSave.
                            let m = egui::Id::NULL.with(("modified", t, rec, key));
                            if changed || ui.data(|d| d.get_temp::<bool>(m)).is_some() {
                                let v = egui::Id::NULL.with(("bc_validate", o.id));
                                ui.data_mut(|d| d.insert_temp(v, (edit_id, m)));
                                events.push((o.id, event::OBJECT_VALIDATE));
                                events.push((o.id, event::OBJECT_SAVE));
                            }
                            let x = egui::Id::NULL.with(("bc_exit", o.id));
                            ui.data_mut(|d| d.insert_temp(x, edit_id));
                            events.push((o.id, event::OBJECT_EXIT));
                        }
                    }
                }
            }
            if changed {
                *dirty = true;
                doc.db.touch(t, rec);
            }
            commit_field(ui, &mut doc.db, (t, rec, key), changed, left);
        }
    }
}
/// Once a field changed and is left (or a value is picked), lookups
/// matching on it copy again, as in FileMaker.
fn commit_field(
    ui: &egui::Ui,
    db: &mut Database,
    (t, rec, field): (u32, usize, u32),
    changed: bool,
    left: bool,
) {
    let key = egui::Id::NULL.with(("modified", t, rec, field));
    if changed {
        ui.data_mut(|d| d.insert_temp(key, true));
    }
    if !left || !ui.data_mut(|d| d.remove_temp::<bool>(key)).unwrap_or(false) {
        return;
    }
    let v = db
        .table(t)
        .and_then(|x| x.records.get(rec)?.values.get(&field).cloned())
        .unwrap_or_default();
    db.set_values(t, rec, &[(field, v)]);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        windows::begin_frame(ui.ctx());
        self.file_frame(ui);
    }
}

impl App {
    /// One frame of an open file: its scripts and dialogs, then its windows
    /// (`windows.rs`), then the other open files in their own OS windows.
    fn file_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let picked = self.incoming.borrow_mut().take();
        if let Some((name, bytes)) = picked {
            self.open_more(name, bytes);
        }
        // Drag and drop (Preferences › Allow drag and drop): read
        // synchronously on desktop, asynchronously in the browser.
        let drops = ctx.input(|i| i.raw.dropped_files.clone());
        for f in drops.into_iter().filter(|_| prefs::get().drag_drop) {
            let name = f
                .path()
                .file_name()
                .map_or("BaseCraft".into(), |n| n.to_string_lossy().into_owned());
            #[cfg(not(target_arch = "wasm32"))]
            if f.path().is_file() {
                self.open_path(f.path());
            } else {
                match f.bytes() {
                    Ok(b) => self.open_more(name, b),
                    Err(e) => self.error = Some(e),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let slot = self.incoming.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(b) = f.bytes_async().await {
                        *slot.borrow_mut() = Some((name, b));
                        ctx.request_repaint();
                    }
                });
            }
        }
        self.layout_dialogs(&ctx);
        objects::url_confirmation(&ctx, &mut self.objs);
        self.records_dialogs(&ctx);
        self.m5_frame(&ctx);
        self.security_dialogs(&ctx);
        self.custom_functions_dialog(&ctx);
        self.prefs_dialog(&ctx);
        self.editing_frame(&ctx);
        let mut kept = None;
        let mut entered = Vec::new();
        for (o, ev) in std::mem::take(&mut self.obj_events) {
            if kept == Some(o) {
                continue;
            }
            if ev == event::OBJECT_VALIDATE {
                let t = self.object_trigger(o, ev);
                if !self.run_pre(t, Default::default()) {
                    // The field keeps the focus and stays modified; its
                    // OnObjectSave and OnObjectExit do not fire.
                    kept = Some(o);
                    let v = egui::Id::NULL.with(("bc_validate", o));
                    if let Some((edit, m)) = ctx.data(|d| d.get_temp::<(egui::Id, egui::Id)>(v)) {
                        ctx.data_mut(|d| {
                            d.insert_temp(m, true);
                            d.insert_temp(edit.with("refocus"), true);
                        });
                        ctx.data_mut(|d| d.insert_temp(objects::focus_id(), o));
                    }
                }
                continue;
            }
            // OnObjectSave runs now, so that it comes before OnObjectExit.
            if ev == event::OBJECT_SAVE && self.active.is_none() {
                let t = self.object_trigger(o, ev);
                self.run_pre(t, Default::default());
                continue;
            }
            // OnObjectExit's False keeps the focus in the field.
            if ev == event::OBJECT_EXIT {
                let t = self.object_trigger(o, ev);
                if !self.run_pre(t, Default::default()) {
                    kept = Some(o);
                    let x = egui::Id::NULL.with(("bc_exit", o));
                    if let Some(edit) = ctx.data(|d| d.get_temp::<egui::Id>(x)) {
                        ctx.data_mut(|d| {
                            d.insert_temp(edit.with("refocus"), true);
                            d.insert_temp(objects::focus_id(), o);
                        });
                    }
                }
                continue;
            }
            if ev == event::OBJECT_ENTER {
                entered.push(o);
                continue;
            }
            self.fire_object(o, ev);
        }
        // Entering another field, unless the one left kept the focus.
        for o in entered.into_iter().filter(|_| kept.is_none()) {
            self.m8.focus = Some(o);
            self.fire_object(o, event::OBJECT_ENTER);
        }
        if let Some(step) = self.pending_step.take() {
            self.start_run(false, |e, h| e.start_step(h, &step));
        }
        if let Some(id) = self.pending_script.take() {
            self.start_run(false, |e, h| e.start(h, id, None));
        }
        self.run_triggers();
        self.script_wait_ui(&ctx);
        if let Some(d) = self.doc.as_ref() {
            self.workspace.ui(&ctx, d);
        }
        if std::mem::take(&mut self.workspace.save)
            && let Some(e) = self.engine.as_mut()
        {
            e.scripts = self.workspace.scripts.clone();
            self.workspace.dirty = false;
            self.dirty = true;
        }
        if let Some(id) = self.workspace.run.take() {
            self.pending_script = Some(id);
        }
        if let Some((title, msg)) = self.messages.first().cloned() {
            egui::Modal::new(egui::Id::NULL.with("script_dialog")).show(&ctx, |ui| {
                ui.set_max_width(420.0);
                if !title.is_empty() {
                    ui.heading(title);
                }
                ui.label(msg);
                if ui.button("OK").clicked() {
                    self.messages.remove(0);
                }
            });
        }
        if let Some((msg, can_override)) = self.alert.clone() {
            egui::Modal::new(egui::Id::NULL.with("validation")).show(&ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(msg);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        self.alert = None;
                    }
                    if can_override && ui.button(tr("Accept anyway")).clicked() {
                        self.alert = None;
                        self.editing = None;
                    }
                });
            });
        }
        self.data_viewer_window(&ctx);
        self.launch_window(&ctx);
        self.windows_frame(ui);
        self.others_frame(&ctx);
    }

    /// Keyboard shortcuts of a window (FileMaker 26 on Windows).
    fn window_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        let key = |m: M, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if self.engine.is_some() && key(M::COMMAND | M::SHIFT, Key::S) {
            self.open_workspace();
            focus_tool_window(ctx, workspace::KEY);
        }
        if key(M::COMMAND | M::SHIFT, Key::O) {
            self.launch.open = self.doc.is_some();
        }
        if key(M::COMMAND, Key::O) {
            self.open_dialog(ctx);
        }
        if key(M::COMMAND, Key::Q) {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        }
        if self.doc.is_none() {
            return;
        }
        if key(M::COMMAND, Key::W) {
            self.close_window();
            return;
        }
        if key(M::COMMAND, Key::L) {
            if self.mode() == Mode::Layout {
                self.leave_layout(design::Then::Exit);
            } else {
                self.enter_layout();
            }
        }
        // Shift and Alt combinations first: Ctrl+F would take Ctrl+Shift+F.
        self.edit_keys(ctx);
        self.record_keys(ctx);
        if self.mode() != Mode::Layout {
            self.mode_keys(ctx);
        }
    }

    /// One window of the file: menus, status toolbar, layout bar and the
    /// layout (or Layout mode). `keys`: this window takes the keyboard.
    fn window_ui(&mut self, ui: &mut egui::Ui, keys: bool) {
        let ctx = ui.ctx().clone();
        if keys {
            self.window_keys(&ctx);
            self.keystrokes(&ctx);
        }
        if self.doc.is_some() && self.mode() != Mode::Layout {
            self.layout_size(ui.max_rect().size());
        }
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        self.remote_frame(ui);
        if self.doc.is_some() && self.win.toolbar {
            egui::Panel::top("status").show(ui, |ui| {
                self.status_toolbar(ui);
                self.layout_bar(ui);
            });
        }
        if let (Some(ds), Some(d)) = (self.designer.as_mut(), self.doc.as_ref()) {
            egui::Panel::right("inspector")
                .default_size(270.0)
                .resizable(true)
                .show(ui, |ui| {
                    ui.heading(tr("Inspector"));
                    ds.inspector(ui, d);
                });
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::RED, e);
                }
                egui::ScrollArea::both().show(ui, |ui| ds.canvas(ui, d));
            });
            if keys {
                ds.keys(&ctx);
            }
            return;
        }
        if self.doc.is_none() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    if let Some(e) = &self.error {
                        egui::Frame::NONE
                            .inner_margin(4.0)
                            .show(ui, |ui| ui.colored_label(Color32::RED, e));
                    }
                    self.launch_center(ui);
                });
            return;
        }
        let full = ui.max_rect().size();
        egui::CentralPanel::default().show(ui, |ui| {
            let content = ui.max_rect().size();
            self.wins.chrome = Some(full - content);
            let (screen, m) = ui.input(|i| (i.viewport().monitor_size, i.modifiers));
            self.seen = script::HostInfo {
                content: Some((content.x as f64, content.y as f64)),
                screen: screen.map(|s| (s.x as f64, s.y as f64)),
                scale: Some(ui.ctx().pixels_per_point() as f64),
                // Shift 1, Ctrl 4, Alt/Option 8, Command 16 (egui does
                // not report Caps Lock, 2).
                modifier_keys: Some(
                    m.shift as u32
                        | (m.ctrl as u32) << 2
                        | (m.alt as u32) << 3
                        | (m.mac_cmd as u32) << 4,
                ),
                ..Default::default()
            };
            if let Some(e) = &self.error {
                ui.colored_label(Color32::RED, e);
            }
            let was_dirty = self.dirty;
            self.dirty = false;
            egui::ScrollArea::both().show(ui, |ui| self.content(ui));
            if self.dirty
                && self.mode() == Mode::Browse
                && let (Some(t), Some(r)) = (self.table(), self.current_record())
            {
                self.editing = Some((t, r));
            }
            self.dirty |= was_dirty;
        });
    }

    /// Title of the front window: the window name (the file's name for
    /// its first window), and the mode in Layout mode.
    fn window_title(&self) -> String {
        let Some(d) = self.doc.as_ref() else {
            return "BaseCraft".into();
        };
        let name = if self.win.name.is_empty() {
            d.name.clone()
        } else {
            self.win.name.clone()
        };
        if self.designer.is_some() {
            format!("{name} — {}", tr("Layout Mode"))
        } else {
            name
        }
    }
}

/// The BaseCraft icon, for the main window and the tool windows.
#[cfg(not(target_arch = "wasm32"))]
fn icon() -> Option<std::sync::Arc<egui::IconData>> {
    static ICON: std::sync::OnceLock<Option<std::sync::Arc<egui::IconData>>> =
        std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon/basecraft-256.png"))
            .ok()
            .map(std::sync::Arc::new)
    })
    .clone()
}

/// A tool window (Script Workspace, Data Viewer). FileMaker shows them as
/// separate OS windows: an egui viewport on desktop, with the BaseCraft
/// icon and the size it had when last closed. Where viewports are embedded
/// (the browser, the UI tests) it is an `egui::Window` in the main window.
/// Clearing `open` closes it; closing the OS window clears `open`.
pub fn tool_window(
    ctx: &egui::Context,
    key: &'static str,
    title: &str,
    size: [f32; 2],
    pos: Option<[f32; 2]>,
    open: &mut bool,
    mut add: impl FnMut(&mut egui::Ui),
) {
    #[cfg(not(target_arch = "wasm32"))]
    if !ctx.embed_viewports() {
        let saved = egui::Id::NULL.with((key, "size"));
        let live = egui::Id::NULL.with((key, "live"));
        let size = ctx
            .data(|d| d.get_temp::<egui::Vec2>(saved))
            .unwrap_or(size.into());
        let mut b = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(size)
            .with_app_id("basecraft");
        if let Some(i) = icon() {
            b = b.with_icon(i);
        }
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(key), b, |ui, _| {
            let (close, rect) =
                ui.input(|i| (i.viewport().close_requested(), i.viewport().inner_rect));
            if let Some(r) = rect {
                ui.ctx().data_mut(|d| d.insert_temp(live, r.size()));
            }
            if close {
                *open = false;
            }
            egui::CentralPanel::default().show(ui, |ui| add(ui));
        });
        if !*open && let Some(s) = ctx.data(|d| d.get_temp::<egui::Vec2>(live)) {
            ctx.data_mut(|d| d.insert_temp(saved, s));
        }
        return;
    }
    let mut w = egui::Window::new(title)
        .id(egui::Id::NULL.with(key))
        .open(open)
        .default_size(size)
        .resizable(true)
        .collapsible(false);
    if let Some(p) = pos {
        w = w.default_pos(p);
    }
    w.show(ctx, |ui| add(ui));
}

/// Brings an open tool window's OS window to the front.
fn focus_tool_window(ctx: &egui::Context, key: &'static str) {
    if !ctx.embed_viewports() {
        ctx.send_viewport_cmd_to(
            egui::ViewportId::from_hash_of(key),
            egui::ViewportCommand::Focus,
        );
    }
}

/// Viewport / window key of the Data Viewer.
const DATA_VIEWER: &str = "data_viewer";

fn creator(app: App) -> eframe::AppCreator<'static> {
    Box::new(|cc| {
        // FileMaker's desktop UI is light (Preferences › Theme).
        prefs::visuals(&cc.egui_ctx);
        Ok(Box::new(app))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let mut app = App::default();
    if let Some(p) = std::env::args_os().nth(1) {
        app.open_path(std::path::Path::new(&p));
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1060.0, 980.0])
        .with_app_id("basecraft");
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "BaseCraft",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        creator(app),
    )
}

/// Browser entry point: runs in the `<canvas id="basecraft">` of index.html.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    let fail = |msg: &str| web_sys::console::error_1(&msg.into());
    wasm_bindgen_futures::spawn_local(async move {
        let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("basecraft"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return fail("BaseCraft: no <canvas id=\"basecraft\"> in the page");
        };
        if let Err(e) = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                creator(App::default()),
            )
            .await
        {
            fail(&format!("failed to start BaseCraft: {e:?}"));
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use fmp12::scripts::{Calc, DialogButton, Script, Step, StepParams, id};

    pub(crate) fn template(name: &str) -> Option<App> {
        let p =
            std::path::PathBuf::from(r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates")
                .join(name)
                .join("en.fmp12");
        let bytes = std::fs::read(p).ok()?;
        let mut app = App::default();
        app.open(name.into(), bytes);
        Some(app)
    }

    /// Contacts runs "Sort by Name" from the Contact List's OnLayoutEnter
    /// trigger when it opens.
    #[test]
    fn layout_enter_trigger_on_open() {
        let Some(mut app) = template("Contacts") else {
            return;
        };
        assert_eq!(app.trigger_queue.front().map(|t| t.0), Some(3));
        app.run_triggers();
        let t = app.table().unwrap();
        assert_eq!(app.sorted.get(&t), Some(&true));
        assert!(app.active.is_none());
    }

    /// A custom dialog waits for the user; the answer resumes the script.
    #[test]
    fn dialog_waits_for_the_user() {
        let Some(mut app) = template("Tasks") else {
            return;
        };
        app.trigger_queue.clear();
        let lit = |s: &str| Some(Calc(fmp12::save::text_literal_tokens(s)));
        let mut d = Step::new(id::SHOW_CUSTOM_DIALOG);
        d.params = StepParams::ShowCustomDialog {
            title: lit("Hello"),
            message: lit("Pick one"),
            buttons: vec![
                DialogButton {
                    label: lit("OK"),
                    commit: true,
                },
                DialogButton {
                    label: lit("Cancel"),
                    commit: false,
                },
            ],
            input: None,
        };
        let mut set = Step::new(id::SET_VARIABLE);
        set.params = StepParams::SetVariable {
            name: "$$choice".into(),
            // Get ( LastMessageChoice )
            value: Some(Calc(vec![0x9B, 0x04, 0x9C, 0x21, 0x05])),
            repetition: None,
        };
        let e = app.engine.as_mut().unwrap();
        e.scripts.push(Script {
            id: 99,
            name: "Ask".into(),
            options: 8,
            steps: vec![d, set],
        });
        app.start_run(false, |e, h| e.start(h, 99, None));
        let Some(Active {
            wait: script::Suspend::Dialog(dlg),
            ..
        }) = app.active.as_ref()
        else {
            panic!("no dialog")
        };
        assert_eq!(dlg.buttons, ["OK", "Cancel"]);
        app.answer(
            script::Answer::Dialog(script::DialogAnswer {
                button: 2,
                inputs: Vec::new(),
            }),
            0.0,
        );
        assert!(app.active.is_none());
        let g = &app.engine.as_ref().unwrap().globals;
        assert_eq!(
            g.get(&("$$choice".into(), 1)).map(|v| v.to_text()),
            Some("2".into())
        );
    }

    /// Scripts edited in the workspace go through save_scripts.
    #[test]
    fn workspace_scripts_are_saved() {
        let Some(mut app) = template("Tasks") else {
            return;
        };
        app.open_workspace();
        let n = app.workspace.scripts.len();
        app.workspace.scripts.push(Script {
            id: 50,
            name: "From the workspace".into(),
            options: 8,
            steps: vec![Step::new(id::BEEP)],
        });
        app.workspace.save = true;
        let e = app.engine.as_mut().unwrap();
        e.scripts = app.workspace.scripts.clone();
        let d = app.doc.as_ref().unwrap();
        let b = fmp12::scripts_save::save_scripts(&d.file, &e.scripts).unwrap();
        let back = fmp12::scripts::scripts(&fmp12::File::from_bytes(b).unwrap()).unwrap();
        assert_eq!(back.len(), n + 1);
        assert_eq!(back.last().unwrap().steps[0].id, id::BEEP);
    }
}
