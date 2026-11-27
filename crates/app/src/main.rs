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
use engine::{Database, Order, Request};
use fmp12::layouts::Layout;

mod design;
mod i18n;
mod look;
mod preview;
#[cfg(test)]
mod ui_tests;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Form,
    List,
    Table,
}

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
    sort_field: Option<u32>,
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
    /// Front panel chosen by the user, per tab control id.
    tabs: BTreeMap<u64, u64>,
    /// Layout mode state (the layout being edited).
    designer: Option<design::Designer>,
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

    #[cfg(not(target_arch = "wasm32"))]
    fn open_path(&mut self, p: &std::path::Path) {
        match std::fs::read(p) {
            Ok(b) => {
                let name = p
                    .file_stem()
                    .map_or("BaseCraft".into(), |s| s.to_string_lossy().into_owned());
                self.open(name, b);
                if let Some(d) = self.doc.as_mut() {
                    d.path = Some(p.to_path_buf());
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn open(&mut self, name: String, bytes: Vec<u8>) {
        let name = name.trim_end_matches(".fmp12").to_string();
        let loaded = fmp12::File::from_bytes(bytes).and_then(|f| Ok((Database::load(&f)?, f)));
        match loaded {
            Ok((db, file)) => {
                let layouts = db
                    .layouts
                    .iter()
                    .filter(|l| l.table_occurrence != 0)
                    .cloned()
                    .collect();
                let engine = script::Engine::load(&file).ok();
                let triggers = fmp12::triggers::triggers(&file).unwrap_or_default();
                *self = App {
                    engine,
                    doc: Some(Doc {
                        name,
                        db,
                        layouts,
                        file,
                        path: None,
                        triggers,
                    }),
                    incoming: self.incoming.clone(),
                    ..Default::default()
                };
                // The first window opens on the first layout: its layout
                // triggers, then the file's OnFirstWindowOpen.
                self.fire(event::LAYOUT_ENTER);
                self.fire(event::MODE_ENTER);
                self.fire(event::RECORD_LOAD);
                if let Some(t) = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.triggers.file(event::FIRST_WINDOW_OPEN))
                    .cloned()
                {
                    self.trigger_queue.push_back((t.script, t.parameter));
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Saves the records into a new `.fmp12` image. `as_copy` asks where.
    fn save(&mut self, ctx: &egui::Context, as_copy: bool) {
        if !self.commit() {
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
        // FileMaker refuses a file whose script list changed (FORMAT.md §8, key 8).
        let names = |l: &[fmp12::scripts::Script]| {
            l.iter().map(|s| (s.id, s.name.clone())).collect::<Vec<_>>()
        };
        let list_changed = scripts.as_ref().is_some_and(|l| {
            fmp12::scripts::scripts(&d.file).map_or(true, |old| names(&old) != names(l))
        });
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
        if list_changed {
            self.messages.push((String::new(), tr("Saved. Scripts were added, renamed or deleted: BaseCraft opens this file, but FileMaker Pro will report it as damaged.").to_string()));
        }
    }

    /// Commits the record being edited: runs field validation like
    /// FileMaker. Returns false (and shows a message) when it fails.
    fn commit(&mut self) -> bool {
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
        let mut window = script::Window::new(&d.db, d.layouts.get(self.layout)?.id);
        window.found = found;
        window.current = *self.current.get(&t).unwrap_or(&0);
        window.sorted = self.sorted.get(&t).copied().unwrap_or(false);
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
        let mut host = AppHost {
            db: &mut d.db,
            window,
        };
        let out = f(&mut engine, &mut host);
        let window = host.window;
        self.engine = Some(engine);
        let old_layout = self.layout;
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
        if self.layout != old_layout && !self.from_trigger {
            self.fire(event::LAYOUT_ENTER);
            self.fire(event::RECORD_LOAD);
        }
        Some(out)
    }

    /// Starts a script or a button step and runs it until it ends or waits.
    fn start_run(
        &mut self,
        trigger: bool,
        start: impl FnOnce(&mut script::Engine, &mut AppHost) -> script::Run,
    ) {
        if self.active.is_some() || !self.commit() {
            return;
        }
        self.from_trigger = trigger;
        let r = self.with_host(|e, h| {
            let mut run = start(e, h);
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
                    script::Suspend::Pause(_) => Vec::new(),
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
        let Some(d) = self.doc.as_ref() else { return };
        let Some(l) = d.layouts.get(self.layout) else {
            return;
        };
        let bit = match self.mode() {
            Mode::Find => fmp12::triggers::mode::FIND,
            Mode::Preview => fmp12::triggers::mode::PREVIEW,
            Mode::Browse | Mode::Layout => fmp12::triggers::mode::BROWSE,
        };
        if self.mode() == Mode::Layout {
            return;
        }
        if let Some(t) = d.triggers.layout(l.id, ev, bit) {
            self.trigger_queue
                .push_back((t.script, t.parameter.clone()));
        }
    }

    /// Queues an object's trigger for `ev`.
    fn fire_object(&mut self, object: u64, ev: u16) {
        let Some(d) = self.doc.as_ref() else { return };
        let Some(l) = d.layouts.get(self.layout) else {
            return;
        };
        if let Some(t) = d.triggers.object(l.id, object, ev) {
            self.trigger_queue
                .push_back((t.script, t.parameter.clone()));
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
        let value = param.and_then(|c| {
            let t = self.table()?;
            let rec = self.current_record();
            let d = self.doc.as_ref()?;
            let to = (d.layouts.get(self.layout)?.table_occurrence & 0xFFF) as u32;
            Some(d.db.eval_tokens(&c.0, t, to, rec))
        });
        self.start_run(true, |e, h| e.start(h, script, value));
    }

    /// The dialog or the pause bar of the waiting script.
    fn script_wait_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut answer = None;
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

    fn delete_record(&mut self) {
        let Some(t) = self.table() else { return };
        let pos = *self.current.get(&t).unwrap_or(&0);
        let Some(r) = self.found(t).get(pos).copied() else {
            return;
        };
        if let Some(tbl) = self
            .doc
            .as_mut()
            .and_then(|d| d.db.tables.iter_mut().find(|x| x.id == t))
        {
            tbl.records.remove(r);
        }
        // Re-index the found set after removal.
        let f = self.found(t);
        f.retain(|&x| x != r);
        for x in f.iter_mut() {
            if *x > r {
                *x -= 1;
            }
        }
        let n = f.len();
        self.current.insert(t, pos.min(n.saturating_sub(1)));
        self.dirty = true;
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
        if !self.commit() {
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
        if !self.commit() {
            return;
        }
        self.switch_request(self.req);
        let (Some(t), Some(d)) = (self.table(), self.doc.as_ref()) else {
            return;
        };
        let reqs: Vec<Request> = self
            .requests
            .iter()
            .map(|(c, omit)| Request {
                criteria: c
                    .iter()
                    .filter(|(_, v)| !v.trim().is_empty())
                    .map(|(k, v)| (*k, v.clone()))
                    .collect(),
                omit: *omit,
            })
            .collect();
        let found = engine::find(&d.db, t, &reqs);
        self.found.insert(t, found);
        self.current.insert(t, 0);
        self.mode = Some(Mode::Browse);
        self.fire(event::MODE_ENTER);
        self.fire(event::RECORD_LOAD);
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

    fn sort(&mut self, order: Order) {
        if !self.commit() {
            return;
        }
        let (Some(t), Some(f)) = (self.table(), self.sort_field) else {
            return;
        };
        let mut found = self.found(t).clone();
        if let Some(d) = &self.doc {
            engine::sort(&d.db, t, &mut found, &[(f, order)]);
        }
        self.last_sort = vec![(f, order)];
        self.found.insert(t, found);
        self.sorted.insert(t, true);
        self.current.insert(t, 0);
    }

    /// View › Layout Mode: edits a copy of the current layout.
    fn enter_layout(&mut self) {
        if self.mode() == Mode::Layout || !self.commit() {
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
                if ui.button(tr("Open…")).clicked() {
                    ui.close();
                    self.open_dialog(&ui.ctx().clone());
                }
                if ui
                    .add(egui::Button::new(tr("Save")).shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    ui.close();
                    self.save(&ui.ctx().clone(), false);
                }
                if ui.button(tr("Save a Copy As…")).clicked() {
                    ui.close();
                    self.save(&ui.ctx().clone(), true);
                }
                if ui.button(tr("Close")).clicked() {
                    ui.close();
                    *self = App {
                        incoming: self.incoming.clone(),
                        ..Default::default()
                    };
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    if ui.button(tr("Exit")).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            ui.menu_button(tr("Edit"), |ui| {
                let Some(ds) = self.designer.as_mut() else {
                    ui.label(tr("Not implemented yet"));
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
                if ui.button(tr("Browse Mode")).clicked() {
                    if self.mode() == Mode::Layout {
                        self.leave_layout(design::Then::Exit);
                    } else {
                        self.mode = Some(Mode::Browse);
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.mode() != Mode::Layout,
                        egui::Button::new(tr("Find Mode")),
                    )
                    .clicked()
                {
                    self.enter_find();
                    ui.close();
                }
                let layout = egui::Button::new(tr("Layout Mode")).shortcut_text("Ctrl+L");
                if ui.add_enabled(self.doc.is_some(), layout).clicked() {
                    self.enter_layout();
                    ui.close();
                }
            });
            ui.menu_button(tr("Insert"), |ui| {
                if let Some(ds) = self.designer.as_mut() {
                    ds.insert_menu(ui);
                    return;
                }
                ui.label(tr("Not implemented yet"));
            });
            ui.menu_button(tr("Format"), |ui| {
                ui.label(tr("Not implemented yet"));
            });
            // Layout mode adds the Layouts and Arrange menus.
            if let (Some(ds), Some(d)) = (self.designer.as_mut(), self.doc.as_ref()) {
                ui.menu_button(tr("Layouts"), |ui| ds.layouts_menu(ui, d));
                ui.menu_button(tr("Arrange"), |ui| ds.arrange_menu(ui));
            }
            ui.menu_button(tr("Records"), |ui| {
                if ui.button(tr("New Record")).clicked() {
                    self.new_record();
                    ui.close();
                }
                if ui.button(tr("Delete Record…")).clicked() {
                    self.delete_record();
                    ui.close();
                }
                ui.separator();
                if ui.button(tr("Show All Records")).clicked() {
                    self.show_all();
                    ui.close();
                }
                if ui.button(tr("Show Omitted Only")).clicked() {
                    self.show_omitted();
                    ui.close();
                }
                if ui.button(tr("Omit Record")).clicked() {
                    self.omit_record();
                    ui.close();
                }
            });
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
            });
            for m in ["Window", "Help"] {
                ui.menu_button(tr(m), |ui| {
                    ui.label(tr("Not implemented yet"));
                });
            }
        });
    }

    fn open_workspace(&mut self) {
        if let Some(e) = self.engine.as_ref() {
            self.workspace.show_scripts(&e.scripts);
        }
    }

    /// Status toolbar: navigation, counts, record commands.
    fn status_toolbar(&mut self, ui: &mut egui::Ui) {
        if matches!(
            self.active.as_ref().map(|a| &a.wait),
            Some(script::Suspend::Pause(_))
        ) {
            let now = ui.input(|i| i.time);
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(0, 90, 180), tr("Script paused"));
                if ui.button(tr("Continue")).clicked() {
                    self.answer(script::Answer::Continue, now);
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.answer(script::Answer::Cancel, now);
                }
            });
            ui.separator();
        }
        if let Some(ds) = self.designer.as_mut() {
            ui.horizontal(|ui| ds.tools(ui));
            return;
        }
        let Some(t) = self.table() else { return };
        let total = self
            .doc
            .as_ref()
            .and_then(|d| d.db.table(t))
            .map_or(0, |x| x.records.len());
        let found = self.found(t).len();
        let pos = *self.current.get(&t).unwrap_or(&0);
        ui.horizontal(|ui| {
            if ui.button("◀").clicked() && pos > 0 && self.commit() {
                self.current.insert(t, pos - 1);
                self.fire(event::RECORD_LOAD);
            }
            if ui.button("▶").clicked() && pos + 1 < found && self.commit() {
                self.current.insert(t, pos + 1);
                self.fire(event::RECORD_LOAD);
            }
            ui.label(if found == 0 {
                "0".to_string()
            } else {
                (pos + 1).to_string()
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.label(format!("{found} / {total}"));
                let s = if self.sorted.get(&t).copied().unwrap_or(false) {
                    "Sorted"
                } else {
                    "Unsorted"
                };
                ui.label(format!("{} ({})", tr("Total"), tr(s)));
            });
            ui.separator();
            match self.mode() {
                Mode::Browse => {
                    if ui.button(tr("Show All")).clicked() {
                        self.show_all();
                    }
                    if ui.button(tr("New Record")).clicked() {
                        self.new_record();
                    }
                    if ui
                        .add_enabled(found > 0, egui::Button::new(tr("Delete Record")))
                        .clicked()
                    {
                        self.delete_record();
                    }
                    if ui.button(tr("Find")).clicked() {
                        self.enter_find();
                    }
                    let fields: Vec<(u32, String)> = self
                        .doc
                        .as_ref()
                        .and_then(|d| d.db.table(t))
                        .map(|x| x.fields.iter().map(|f| (f.id, f.name.clone())).collect())
                        .unwrap_or_default();
                    let label = self
                        .sort_field
                        .and_then(|id| fields.iter().find(|f| f.0 == id))
                        .map_or(tr("Sort"), |f| f.1.as_str());
                    egui::ComboBox::from_id_salt("sortfield")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (id, name) in &fields {
                                ui.selectable_value(&mut self.sort_field, Some(*id), name);
                            }
                        });
                    if ui.button("↑").clicked() {
                        self.sort(Order::Ascending);
                    }
                    if ui.button("↓").clicked() {
                        self.sort(Order::Descending);
                    }
                }
                Mode::Layout => {}
                Mode::Preview => {
                    if ui.button("◀").clicked() && self.page > 0 {
                        self.page -= 1;
                    }
                    ui.label(format!(
                        "{} {}/{}",
                        tr("Page"),
                        self.page + 1,
                        self.pages.max(1)
                    ));
                    if ui.button("▶").clicked() && self.page + 1 < self.pages {
                        self.page += 1;
                    }
                    if ui.button(tr("Exit Preview")).clicked() {
                        self.mode = Some(Mode::Browse);
                        self.fire(event::MODE_ENTER);
                    }
                }
                Mode::Find => {
                    if ui.button(tr("Perform Find")).clicked() {
                        self.perform_find();
                    }
                    let n = self.requests.len().max(self.req + 1);
                    if ui.button("◀").clicked() && self.req > 0 {
                        self.switch_request(self.req - 1);
                    }
                    ui.label(format!("{} {}/{n}", tr("Request"), self.req + 1));
                    if ui.button("▶").clicked() && self.req + 1 < n {
                        self.switch_request(self.req + 1);
                    }
                    if ui.button(tr("New Request")).clicked() {
                        self.switch_request(n);
                    }
                    ui.checkbox(&mut self.omit, tr("Omit"));
                    if ui.button(tr("Cancel")).clicked() {
                        self.mode = Some(Mode::Browse);
                        self.fire(event::MODE_ENTER);
                    }
                }
            }
            if self.dirty {
                ui.colored_label(Color32::from_rgb(180, 90, 0), tr("Unsaved changes"));
            }
        });
    }

    fn layout_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            return;
        };
        let names: Vec<String> = doc.layouts.iter().map(|l| l.name.clone()).collect();
        let mut chosen = None;
        if self.mode() == Mode::Layout {
            let mut then = None;
            ui.horizontal(|ui| {
                ui.label(tr("Layout:"));
                let current = names.get(self.layout).map_or("", |n| n.as_str());
                egui::ComboBox::from_id_salt("layout")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            if ui.selectable_label(i == self.layout, n).clicked()
                                && i != self.layout
                            {
                                then = Some(design::Then::Switch(i));
                            }
                        }
                    });
                if ui.button(tr("New Layout/Report")).clicked()
                    && let (Some(ds), Some(d)) = (self.designer.as_mut(), self.doc.as_ref())
                {
                    ds.open_new_layout(d);
                }
                if ui.button(tr("Layout Setup…")).clicked()
                    && let Some(ds) = self.designer.as_mut()
                {
                    ds.open_setup();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Exit Layout")).clicked() {
                        then = Some(design::Then::Exit);
                    }
                });
            });
            if let Some(t) = then {
                self.leave_layout(t);
            }
            return;
        }
        ui.horizontal(|ui| {
            ui.label(tr("Layout:"));
            let current = names.get(self.layout).map_or("", |n| n.as_str());
            egui::ComboBox::from_id_salt("layout")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        if ui.selectable_label(i == self.layout, n).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            ui.label(tr("View As:"));
            for (v, label) in [
                (View::Form, "Form"),
                (View::List, "List"),
                (View::Table, "Table"),
            ] {
                if ui.selectable_label(self.view == v, tr(label)).clicked() {
                    self.view = v;
                }
            }
            let in_preview = self.mode() == Mode::Preview;
            if ui
                .add_enabled(
                    self.mode() != Mode::Layout,
                    egui::Button::selectable(in_preview, tr("Preview")),
                )
                .clicked()
            {
                if in_preview {
                    self.mode = Some(Mode::Browse);
                    self.fire(event::MODE_ENTER);
                } else if self.commit() {
                    self.mode = Some(Mode::Preview);
                    self.page = 0;
                    self.fire(event::MODE_ENTER);
                }
            }
            let mut edit = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                edit = ui.button(tr("Edit Layout")).clicked();
            });
            if edit {
                chosen = None;
                self.enter_layout();
            }
        });
        if let Some(i) = chosen.filter(|i| *i != self.layout && self.commit()) {
            self.fire(event::LAYOUT_EXIT);
            self.layout = i;
            self.fire(event::LAYOUT_ENTER);
            self.fire(event::RECORD_LOAD);
        }
    }

    /// Draws the current layout: form (current record), list (body part
    /// repeated per record) or table (one row per record).
    fn content(&mut self, ui: &mut egui::Ui) {
        let Some(t) = self.table() else { return };
        let record = self.current_record();
        let found = self.found(t).clone();
        let mode = self.mode();
        let view = self.view;
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
            .cloned();
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
            table_view(ui, doc, t, &layout, &found, &mut self.dirty);
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
        let ctx = DrawCtx { table: t, mode };
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
        // The object list runs front to back: paint it in reverse.
        for (o, lk) in layout.objects.iter().zip(&looks).rev() {
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
                match o.kind {
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
                        panels.sort_by_key(|p| p.id);
                        for p in panels {
                            let label = p.text.clone().unwrap_or_default();
                            let w = (label.chars().count() as f32 * 8.0 + 32.0).max(60.0);
                            let tab =
                                Rect::from_min_size(egui::pos2(x, r.top()), Vec2::new(w, 30.0));
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
                            x += w;
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
                            let Some(rr) = related_rec else { continue };
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
                                draw_object(
                                    ui,
                                    &painter,
                                    doc,
                                    &ctx,
                                    c,
                                    &looks[ci],
                                    cr,
                                    Some((ptable, Some(rr))),
                                    &mut self.request,
                                    &mut self.dirty,
                                    &mut self.obj_events,
                                );
                            }
                        }
                    }
                    _ => {
                        if let Some(step) = o.button.as_ref().filter(|_| mode == Mode::Browse) {
                            let id = ui.scope_id().with(("button", o.id));
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
                            lk.paint_text(&painter, r, &text);
                            continue;
                        }
                        if let Some(txt) = o.text.as_ref().filter(|x| x.contains("<<")) {
                            let merged = merge_fields(doc, txt, t, layout_to, rec);
                            lk.paint_box(&painter, r);
                            lk.paint_text(&painter, r, &merged);
                            continue;
                        }
                        let owner = owner_of(doc, o, t, layout_to, rec);
                        draw_object(
                            ui,
                            &painter,
                            doc,
                            &ctx,
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
        if let Some((tc, p)) = clicked_tab {
            self.tabs.insert(tc, p);
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
fn merge_fields(doc: &Doc, text: &str, t: u32, layout_to: u32, rec: Option<usize>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("<<") {
        out.push_str(&rest[..a]);
        let Some(b) = rest[a..].find(">>") else {
            out.push_str(&rest[a..]);
            return out;
        };
        let name = &rest[a + 2..a + b];
        let (to_name, field) = name.split_once("::").unwrap_or(("", name));
        let to = doc
            .db
            .occurrences
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(to_name))
            .map_or(layout_to, |o| o.id);
        let value = doc.db.table_of(to).and_then(|tbl| {
            let f = tbl.field_by_name(field)?;
            let r = if to == layout_to {
                rec?
            } else {
                *doc.db.related(layout_to, rec?, to).first()?
            };
            let _ = t;
            Some(doc.db.value(tbl.id, r, f.id).to_text())
        });
        out.push_str(&value.unwrap_or_default());
        rest = &rest[a + b + 2..];
    }
    out.push_str(rest);
    out
}

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
    if to == layout_to || ft == t && doc.db.table_of(layout_to).map(|x| x.id) == Some(ft) {
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
}

impl script::Host for AppHost<'_> {
    fn db(&mut self) -> &mut Database {
        self.db
    }
    fn window(&mut self) -> &mut script::Window {
        &mut self.window
    }
}

struct DrawCtx {
    table: u32,
    mode: Mode,
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
    lk.paint_shape(painter, r, o.kind);
    if o.kind != 1 {
        if let Some(txt) = &o.text {
            lk.paint_text(painter, r, txt);
        }
        return;
    }
    let (Some((_, field, _)), Some((t, record))) = (o.field, owner) else {
        return;
    };
    let field = field as u32;
    let is_calc = doc
        .db
        .table(t)
        .and_then(|x| x.field(field))
        .is_some_and(|f| f.calc.is_some());
    let inner = lk.content_rect(r);
    match (c.mode, record) {
        (Mode::Find, _) if t == c.table => {
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_gray(150)),
                StrokeKind::Inside,
            );
            let s = request.entry(field).or_default();
            ui.put(inner, lk.edit(s));
        }
        (Mode::Find | Mode::Layout | Mode::Preview, _) | (Mode::Browse, None) => {}
        (Mode::Browse, Some(rec)) if is_calc => {
            let v = doc.db.value(t, rec, field).to_text().replace('\r', " ");
            lk.paint_text(painter, r, &v);
        }
        (Mode::Browse, Some(rec)) => {
            // Placeholder text (a calculation, usually a literal).
            let placeholder = o
                .label
                .as_ref()
                .map(|l| doc.db.eval_tokens(l, t, 0, Some(rec)).to_text());
            // Pop-up menus and drop-down lists offer their value list.
            let choices = match (o.display, o.value_list) {
                (1 | 2, Some(vl)) => Some(doc.db.value_list_values(vl)),
                _ => None,
            };
            let mut changed = false;
            let mut left = false;
            if let Some(tbl) = doc.db.tables.iter_mut().find(|x| x.id == t) {
                let s = tbl.records[rec].values.entry(field).or_default();
                match choices {
                    Some(values) => {
                        let mut picked = None;
                        ui.put(inner, |ui: &mut egui::Ui| {
                            egui::ComboBox::from_id_salt(("vl", o.id, rec))
                                .selected_text(s.as_str())
                                .width(inner.width())
                                .show_ui(ui, |ui| {
                                    for v in &values {
                                        if ui.selectable_label(s == v, v).clicked() {
                                            picked = Some(v.clone());
                                        }
                                    }
                                })
                                .response
                        });
                        if let Some(v) = picked {
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
                        let r = ui.put(inner, lk.edit(s));
                        if r.gained_focus() {
                            events.push((o.id, event::OBJECT_ENTER));
                        }
                        if r.changed() {
                            changed = true;
                            events.push((o.id, event::OBJECT_MODIFY));
                        }
                        if r.lost_focus() {
                            left = true;
                            events.push((o.id, event::OBJECT_EXIT));
                        }
                    }
                }
            }
            if changed {
                *dirty = true;
                doc.db.touch(t, rec);
            }
            commit_field(ui, &mut doc.db, (t, rec, field), changed, left);
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

/// Table view: one column per field of the layout's body, one row per record.
fn table_view(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    t: u32,
    layout: &Layout,
    found: &[usize],
    dirty: &mut bool,
) {
    let mut cols: Vec<(f64, u32)> = layout
        .objects
        .iter()
        .filter(|o| o.kind == 1)
        .filter_map(|o| {
            let (to, f, _) = o.field?;
            (doc.db.table_of((to & 0xFFF) as u32)?.id == t).then_some((o.bounds.left, f as u32))
        })
        .collect();
    cols.sort_by(|a, b| a.0.total_cmp(&b.0));
    cols.dedup_by_key(|c| c.1);
    egui::Grid::new("table_view").striped(true).show(ui, |ui| {
        for (_, f) in &cols {
            let name = doc
                .db
                .table(t)
                .and_then(|x| x.field(*f))
                .map_or(String::new(), |f| f.name.clone());
            ui.strong(name);
        }
        ui.end_row();
        for &rec in found {
            for (_, f) in &cols {
                let is_calc = doc
                    .db
                    .table(t)
                    .and_then(|x| x.field(*f))
                    .is_some_and(|f| f.calc.is_some());
                if is_calc {
                    ui.label(doc.db.value(t, rec, *f).to_text());
                } else if let Some(tbl) = doc.db.tables.iter_mut().find(|x| x.id == t) {
                    let s = tbl.records[rec].values.entry(*f).or_default();
                    let r = ui.text_edit_singleline(s);
                    if r.changed() {
                        *dirty = true;
                    }
                    commit_field(ui, &mut doc.db, (t, rec, *f), r.changed(), r.lost_focus());
                }
            }
            ui.end_row();
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let picked = self.incoming.borrow_mut().take();
        if let Some((name, bytes)) = picked {
            self.open(name, bytes);
        }
        // Drag and drop: read synchronously on desktop, asynchronously in the browser.
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = f
                .path()
                .file_name()
                .map_or("BaseCraft".into(), |n| n.to_string_lossy().into_owned());
            #[cfg(not(target_arch = "wasm32"))]
            match f.bytes() {
                Ok(b) => self.open(name, b),
                Err(e) => self.error = Some(e),
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
        if self.engine.is_some()
            && ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                )
            })
        {
            self.open_workspace();
            focus_tool_window(&ctx, workspace::KEY);
        }
        if self.doc.is_some()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            self.save(&ctx, false);
        }
        if self.doc.is_some()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::L))
        {
            if self.mode() == Mode::Layout {
                self.leave_layout(design::Then::Exit);
            } else {
                self.enter_layout();
            }
        }
        self.layout_dialogs(&ctx);
        for (o, ev) in std::mem::take(&mut self.obj_events) {
            self.fire_object(o, ev);
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
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        if self.doc.is_some() {
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
            ds.keys(&ctx);
            let title = format!("{} — {}", d.name, tr("Layout Mode"));
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            return;
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::RED, e);
            }
            if self.doc.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        if ui.button(tr("Open…")).clicked() {
                            self.open_dialog(&ctx);
                        }
                        ui.label(tr("or drop a .fmp12 file here"));
                    });
                });
                return;
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
        let title = self
            .doc
            .as_ref()
            .map_or("BaseCraft".to_string(), |d| d.name.clone());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
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
        // FileMaker's desktop UI is light.
        cc.egui_ctx.set_visuals(egui::Visuals::light());
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
