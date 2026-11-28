//! Windows, as FileMaker has them: several windows on the same file (Window
//! › New Window), each with its own layout, mode, found set and sort, and
//! several files open at once.
//!
//! The front window lives in the `App` fields; the file's other windows are
//! the script engine's background windows (`script::Engine::windows`), so
//! New / Select / Close Window in scripts and the Window menu act on the
//! same list. A background window is drawn by swapping it into the `App`
//! fields for the time of its frame. Each window is an OS window (an egui
//! viewport) on desktop, and an `egui::Window` where viewports are embedded
//! (the browser, the UI tests). Activating a window brings it to the front.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;

use crate::{App, Mode, design, tr};

/// What a window keeps besides its `script::Window`: the found sets of the
/// other tables, the find requests being edited, Layout mode.
#[derive(Default)]
pub struct Extras {
    found: BTreeMap<u32, Vec<usize>>,
    current: BTreeMap<u32, usize>,
    sorted: BTreeMap<u32, bool>,
    designer: Option<design::Designer>,
    request: BTreeMap<u32, String>,
    omit: bool,
    requests: Vec<(BTreeMap<u32, String>, bool)>,
    req: usize,
    tabs: BTreeMap<u64, u64>,
    page: usize,
    last_sort: Vec<(u32, engine::Order)>,
}

/// The windows of one open file.
#[derive(Default)]
pub struct State {
    /// Identifies the file's OS windows (viewport ids).
    pub file: u64,
    /// Background windows' extras, by window name.
    extras: BTreeMap<String, Extras>,
    /// The window the file's first OS window shows.
    root: String,
    /// Window activated by the user this frame (brought to the front after).
    activate: Option<String>,
    /// Window to close after this frame.
    close: Option<String>,
    /// Front window of the last frame (its OS window gets the focus when
    /// a script or the Window menu changes it).
    last_front: String,
    /// OS windows that had the focus last frame.
    focused: BTreeMap<String, bool>,
}

impl State {
    /// Whether the OS window of window `name` just got the focus.
    fn gained(&mut self, name: &str, now: bool) -> bool {
        let was = self.focused.insert(name.to_string(), now).unwrap_or(false);
        now && !was
    }

    pub fn new() -> State {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        State {
            file: NEXT.fetch_add(1, Ordering::Relaxed),
            ..Default::default()
        }
    }
}

/// One entry of the Window menu: file, window name, OS window.
#[derive(Clone)]
struct Listed {
    file: u64,
    name: String,
    viewport: egui::ViewportId,
}

fn listed_id(next: bool) -> egui::Id {
    egui::Id::NULL.with(("bc_windows", next))
}

/// Starts a frame of the whole app: the windows listed during the last
/// frame become the Window menu's list.
pub fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        let next: Vec<Listed> = d.get_temp(listed_id(true)).unwrap_or_default();
        d.insert_temp(listed_id(false), next);
        d.insert_temp(listed_id(true), Vec::<Listed>::new());
    });
}

fn list_window(ctx: &egui::Context, l: Listed) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Listed>>(listed_id(true))
            .push(l)
    });
}

impl App {
    /// The front window's name.
    pub fn front_name(&self) -> String {
        match (&self.doc, self.win.name.is_empty()) {
            (Some(d), true) => d.name.clone(),
            _ => self.win.name.clone(),
        }
    }

    /// The file's windows, front first, then the most recently active.
    pub fn window_names(&self) -> Vec<String> {
        let mut v = vec![self.front_name()];
        if let Some(e) = &self.engine {
            v.extend(e.windows.iter().rev().map(|w| w.name.clone()));
        }
        v
    }

    fn bg_index(&self, name: &str) -> Option<usize> {
        self.engine
            .as_ref()?
            .windows
            .iter()
            .rposition(|w| w.name == name)
    }

    /// Takes a window's state as scripts see it, back into the app fields.
    pub fn apply_window(&mut self, window: script::Window) {
        self.view = window.view;
        self.win = window.clone();
        if let Some(i) = self
            .doc
            .as_ref()
            .and_then(|d| d.layouts.iter().position(|l| l.id == window.layout))
        {
            self.layout = i;
        }
        self.mode = Some(match window.mode {
            script::Mode::Find => Mode::Find,
            script::Mode::Preview => Mode::Preview,
            script::Mode::Browse => Mode::Browse,
        });
        if let Some(t) = self.table() {
            self.found.insert(t, window.found);
            self.current.insert(t, window.current);
            self.sorted.insert(t, window.sorted);
        }
    }

    fn swap_extras(&mut self, x: &mut Extras) {
        use std::mem::swap;
        swap(&mut self.found, &mut x.found);
        swap(&mut self.current, &mut x.current);
        swap(&mut self.sorted, &mut x.sorted);
        swap(&mut self.designer, &mut x.designer);
        swap(&mut self.request, &mut x.request);
        swap(&mut self.omit, &mut x.omit);
        swap(&mut self.requests, &mut x.requests);
        swap(&mut self.req, &mut x.req);
        swap(&mut self.tabs, &mut x.tabs);
        swap(&mut self.page, &mut x.page);
        swap(&mut self.last_sort, &mut x.last_sort);
    }

    /// Exchanges the front window with background window `i`. Twice with
    /// the same `i` gives the windows back as they were.
    fn swap_with(&mut self, i: usize) -> bool {
        let Some(front) = self.script_window() else {
            return false;
        };
        let front_name = front.name.clone();
        let Some(bg) = self
            .engine
            .as_mut()
            .and_then(|e| e.windows.get_mut(i))
            .map(|w| std::mem::replace(w, front))
        else {
            return false;
        };
        let mut x = self.wins.extras.remove(&bg.name).unwrap_or_default();
        self.swap_extras(&mut x);
        self.wins.extras.insert(front_name, x);
        self.apply_window(bg);
        if self.designer.is_some() {
            self.mode = Some(Mode::Layout);
        }
        true
    }

    /// Brings window `name` to the front (Window menu, a click in it).
    pub fn select_window(&mut self, name: &str) {
        let Some(i) = self.bg_index(name) else { return };
        if self.mode() != Mode::Layout && !self.commit() {
            return;
        }
        if self.swap_with(i)
            && let Some(e) = self.engine.as_mut()
        {
            // The window left behind is now the most recently active.
            let w = e.windows.remove(i);
            e.windows.push(w);
        }
    }

    /// Window › New Window: a copy of the front window (layout, found set,
    /// sort, mode) named "<file> - 2", as FileMaker's.
    pub fn new_window(&mut self) {
        if self.engine.is_none() || self.designer.is_some() {
            return;
        }
        let step = fmp12::scripts::Step::new(fmp12::scripts::id::NEW_WINDOW);
        self.start_run(false, |e, h| e.start_step(h, &step));
    }

    /// File › Close (Ctrl+W): closes the front window; the file's last
    /// window closes the file.
    pub fn close_window(&mut self) {
        let front = self.front_name();
        self.close_named(&front);
    }

    fn close_named(&mut self, name: &str) {
        if self.designer.is_some() && name == self.front_name() {
            self.leave_layout(design::Then::Exit);
            return;
        }
        if name != self.front_name() {
            if let Some(i) = self.bg_index(name)
                && let Some(e) = self.engine.as_mut()
            {
                e.windows.remove(i);
                self.wins.extras.remove(name);
            }
            return;
        }
        if !self.commit() {
            return;
        }
        let next = self.engine.as_mut().and_then(|e| e.windows.pop());
        match next {
            Some(w) => {
                let mut x = self.wins.extras.remove(&w.name).unwrap_or_default();
                self.swap_extras(&mut x);
                self.apply_window(w);
                if self.designer.is_some() {
                    self.mode = Some(Mode::Layout);
                }
            }
            None => self.close_file(),
        }
    }

    /// Closes the file. The next open file, if any, takes its place.
    pub fn close_file(&mut self) {
        let mut others = std::mem::take(&mut self.others);
        let incoming = self.incoming.clone();
        *self = if others.is_empty() {
            App::default()
        } else {
            others.remove(0)
        };
        self.incoming = incoming;
        self.others.append(&mut others);
    }

    /// Draws every window of the file: the one the first OS window shows
    /// in `ui`, the others in their own OS windows.
    pub fn windows_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let names = self.window_names();
        let front = names[0].clone();
        if !names.contains(&self.wins.root) {
            self.wins.root = front.clone();
        }
        let embedded = ctx.embed_viewports();
        let root = self.wins.root.clone();
        let root_vp = ctx.viewport_id();
        // The window a script or the menu just brought forward gets the focus.
        if front != self.wins.last_front {
            if !self.wins.last_front.is_empty() {
                let vp = self.viewport_of(&front, root_vp);
                ctx.send_viewport_cmd_to(vp, egui::ViewportCommand::Focus);
            }
            self.wins.last_front = front.clone();
        }
        list_window(
            &ctx,
            Listed {
                file: self.wins.file,
                name: root.clone(),
                viewport: root_vp,
            },
        );
        let focused = ui.input(|i| i.viewport().focused == Some(true));
        let focused = self.wins.gained(&root, focused);
        if self.doc.is_some() && (focused || embedded && pressed_on_background(&ctx)) {
            self.wins.activate = Some(root.clone());
        }
        let keys = !embedded || root == front;
        self.draw_window(ui, &root, keys);
        if root_vp == egui::ViewportId::ROOT || !embedded {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title_of(&root)));
        }
        for name in names.iter().filter(|n| **n != root) {
            let vp = self.viewport_of(name, root_vp);
            list_window(
                &ctx,
                Listed {
                    file: self.wins.file,
                    name: name.clone(),
                    viewport: vp,
                },
            );
            let b = egui::ViewportBuilder::default()
                .with_title(self.title_of(name))
                .with_inner_size([1000.0, 760.0])
                .with_app_id("basecraft");
            let keys = !embedded || *name == front;
            ctx.show_viewport_immediate(vp, b, |ui, _| {
                let (close, focused) = ui.input(|i| {
                    (
                        i.viewport().close_requested(),
                        i.viewport().focused == Some(true),
                    )
                });
                if close {
                    self.wins.close = Some(name.clone());
                }
                let focused = self.wins.gained(name, focused);
                let pressed =
                    embedded && ui.ui_contains_pointer() && ui.input(|i| i.pointer.any_pressed());
                if focused || pressed {
                    self.wins.activate = Some(name.clone());
                }
                self.draw_window(ui, name, keys);
            });
        }
        if let Some(n) = self.wins.close.take() {
            self.close_named(&n);
        }
        if let Some(n) = self.wins.activate.take()
            && n != self.front_name()
        {
            self.select_window(&n);
            // The user chose it: no focus command back to it.
            self.wins.last_front = self.front_name();
        }
    }

    fn viewport_of(&self, name: &str, root: egui::ViewportId) -> egui::ViewportId {
        if name == self.wins.root {
            root
        } else {
            egui::ViewportId::from_hash_of(("bc_window", self.wins.file, name))
        }
    }

    fn title_of(&mut self, name: &str) -> String {
        if name == self.front_name() {
            return self.window_title();
        }
        match self.bg_index(name) {
            Some(i) => {
                self.swap_with(i);
                let t = self.window_title();
                self.swap_with(i);
                t
            }
            None => name.to_string(),
        }
    }

    /// Draws window `name`, swapping it to the front for the time of its frame.
    /// Widget ids are scoped by window name: the same field shown in two
    /// windows is two widgets, and keeps its focus when its window comes
    /// to the front.
    fn draw_window(&mut self, ui: &mut egui::Ui, name: &str, keys: bool) {
        if name == self.front_name() {
            ui.push_id(("bc_window", name), |ui| self.window_ui(ui, keys));
            return;
        }
        let Some(i) = self.bg_index(name) else { return };
        if self.swap_with(i) {
            ui.push_id(("bc_window", name), |ui| self.window_ui(ui, keys));
            self.swap_with(i);
        }
    }

    /// The other open files, each in its own OS window.
    pub fn others_frame(&mut self, ctx: &egui::Context) {
        for a in &mut self.others {
            let vp = egui::ViewportId::from_hash_of(("bc_file", a.wins.file));
            let b = egui::ViewportBuilder::default()
                .with_title(a.window_title())
                .with_inner_size([1000.0, 760.0])
                .with_app_id("basecraft");
            ctx.show_viewport_immediate(vp, b, |ui, _| {
                if ui.input(|i| i.viewport().close_requested()) {
                    a.close_all();
                }
                a.file_frame(ui);
            });
        }
        // Files closed in their window, and their own other files.
        let mut more = Vec::new();
        self.others.retain_mut(|a| {
            more.append(&mut a.others);
            a.busy()
        });
        self.others.append(&mut more);
    }

    /// Closes all the windows of the file.
    fn close_all(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.windows.clear();
        }
        if self.commit() {
            self.close_file();
        }
    }

    /// The Window menu, as FileMaker 26's: New Window, Minimize, Tile,
    /// Cascade, then the open windows (the front one checked).
    pub fn window_menu(&mut self, ui: &mut egui::Ui) {
        let on = self.doc.is_some() && self.engine.is_some() && self.designer.is_none();
        if ui
            .add_enabled(on, egui::Button::new(tr("New Window")))
            .clicked()
        {
            ui.close();
            self.new_window();
        }
        let ctx = ui.ctx().clone();
        if ui.button(tr("Minimize Window")).clicked() {
            ui.close();
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        ui.separator();
        let list: Vec<Listed> = ctx.data(|d| d.get_temp(listed_id(false)).unwrap_or_default());
        let arrange = |how: u8| {
            let n = list.len().max(1) as f32;
            let screen = ctx
                .input(|i| i.viewport().monitor_size)
                .unwrap_or(egui::vec2(1600.0, 900.0));
            for (k, l) in list.iter().enumerate() {
                let k = k as f32;
                let (pos, size) = match how {
                    // Tile horizontally: one above the other.
                    0 => (
                        egui::pos2(0.0, k * screen.y / n),
                        egui::vec2(screen.x, screen.y / n - 40.0),
                    ),
                    // Tile vertically: side by side.
                    1 => (
                        egui::pos2(k * screen.x / n, 0.0),
                        egui::vec2(screen.x / n, screen.y - 80.0),
                    ),
                    _ => (egui::pos2(30.0 * k, 30.0 * k), egui::vec2(1000.0, 760.0)),
                };
                ctx.send_viewport_cmd_to(l.viewport, egui::ViewportCommand::OuterPosition(pos));
                ctx.send_viewport_cmd_to(l.viewport, egui::ViewportCommand::InnerSize(size));
            }
        };
        for (how, label, key) in [
            (0, "Tile Horizontally", "Shift+F4"),
            (1, "Tile Vertically", ""),
            (2, "Cascade", "Shift+F5"),
        ] {
            if ui
                .add_enabled(
                    !ctx.embed_viewports(),
                    egui::Button::new(tr(label)).shortcut_text(key),
                )
                .clicked()
            {
                ui.close();
                arrange(how);
            }
        }
        ui.separator();
        // FileMaker lists the front window first.
        let front = self.front_name();
        let mut list = list.clone();
        list.sort_by_key(|l| !(l.file == self.wins.file && l.name == front));
        for (k, l) in list.iter().enumerate() {
            let me = l.file == self.wins.file;
            let label = format!("{} {}", k + 1, l.name);
            if ui
                .add(egui::Button::new(label).selected(me && l.name == front))
                .clicked()
            {
                ui.close();
                if me {
                    self.select_window(&l.name);
                } else {
                    ctx.send_viewport_cmd_to(l.viewport, egui::ViewportCommand::Focus);
                }
            }
        }
    }
}

/// Embedded windows: a press on the main window's own background (not on
/// a window, menu or pop-up over it).
fn pressed_on_background(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()))
        .flatten()
        .and_then(|p| ctx.layer_id_at(p))
        .is_some_and(|l| l.order == egui::Order::Background)
}
