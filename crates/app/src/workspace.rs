//! Script Workspace (Scripts › Script Workspace…, Ctrl+Shift+S), modelled
//! on FileMaker's: the script list with folders on the left, the steps of
//! the selected script in FileMaker's step text in the middle (blocks
//! indented, disabled steps greyed), the step palette by category with a
//! search field on the right, and the selected step's parameters below
//! the steps. Calculations open a calculation editor with the function
//! list.
//!
//! The workspace edits a copy of the scripts; "Save" (or File › Save)
//! copies it into the script engine, and File › Save writes it to the
//! `.fmp12` through `fmp12::scripts_save::save_scripts`.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32, RichText};
use fmp12::scripts::{
    Calc, DialogButton, FieldRef, FindRequest, LayoutDest, RecordDest, Script, SortOrder, Step,
    StepParams, Target, id, step_name,
};
use script::text::{self, Names};

use crate::Doc;
use crate::i18n::{french, tr};

/// Viewport / window key of the workspace.
pub const KEY: &str = "script_workspace";

/// A step's name as the palette and the step list show it (French in the
/// French UI).
pub fn step_label(id: u16) -> Option<&'static str> {
    if french() {
        text::step_name_fr(id)
    } else {
        step_name(id)
    }
}

/// Step categories of the palette (FileMaker's names), with the step ids
/// they hold; steps not listed go to "Miscellaneous".
const CATEGORIES: &[(&str, &[u16])] = &[
    (
        "Control",
        &[
            1, 164, 210, 62, 103, 90, 68, 125, 69, 70, 71, 72, 73, 85, 86, 141, 148, 142, 205, 206,
            207, 223,
        ],
    ),
    (
        "Navigation",
        &[6, 16, 74, 99, 145, 17, 4, 5, 41, 55, 22, 228, 169],
    ),
    ("Editing", &[45, 46, 47, 48, 49, 50, 130, 128, 129, 167]),
    (
        "Fields",
        &[
            76, 147, 116, 61, 77, 11, 12, 13, 14, 60, 131, 158, 159, 56, 160, 161, 40, 91, 132, 215,
        ],
    ),
    (
        "Records",
        &[
            7, 8, 9, 10, 104, 182, 133, 51, 75, 101, 98, 35, 36, 143, 144, 225,
        ],
    ),
    (
        "Found Sets",
        &[
            23, 27, 25, 26, 28, 24, 126, 127, 39, 154, 21, 155, 150, 218, 221,
        ],
    ),
    (
        "Windows",
        &[
            122, 123, 121, 119, 120, 79, 80, 81, 124, 30, 29, 97, 166, 115,
        ],
    ),
    (
        "Files",
        &[
            82, 33, 34, 139, 37, 3, 95, 43, 42, 181, 188, 189, 190, 191, 192, 193, 196, 197, 199,
        ],
    ),
    ("Accounts", &[134, 135, 136, 137, 138, 83]),
    (
        "Miscellaneous",
        &[
            89, 87, 93, 66, 63, 111, 224, 117, 175, 146, 237, 57, 67, 102, 44, 200, 239,
        ],
    ),
];

/// A calculation of the selected step that the editors can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    /// Value, condition, parameter, result, layout/record calculation.
    Main,
    Repetition,
    Title,
    Message,
}

fn slot_calc(p: &StepParams, slot: Slot) -> Option<&Calc> {
    match (p, slot) {
        (StepParams::Condition(c), Slot::Main)
        | (StepParams::ExitScript(c), Slot::Main)
        | (StepParams::Pause(c), Slot::Main)
        | (StepParams::Records { count: c, .. }, Slot::Main)
        | (StepParams::PerformScript { parameter: c, .. }, Slot::Main)
        | (StepParams::SetVariable { value: c, .. }, Slot::Main)
        | (StepParams::SetField { value: c, .. }, Slot::Main)
        | (StepParams::GoToObject { name: c, .. }, Slot::Main)
        | (StepParams::SetVariable { repetition: c, .. }, Slot::Repetition)
        | (StepParams::ShowCustomDialog { title: c, .. }, Slot::Title)
        | (StepParams::ShowCustomDialog { message: c, .. }, Slot::Message) => c.as_ref(),
        (
            StepParams::GoToLayout {
                dest: LayoutDest::NameByCalc(c) | LayoutDest::NumberByCalc(c),
                ..
            },
            Slot::Main,
        )
        | (
            StepParams::GoToRecord {
                dest: RecordDest::ByCalc(c),
                ..
            },
            Slot::Main,
        ) => Some(c),
        _ => None,
    }
}

fn set_slot(p: &mut StepParams, slot: Slot, v: Option<Calc>) {
    match (p, slot) {
        (StepParams::Condition(c), Slot::Main)
        | (StepParams::ExitScript(c), Slot::Main)
        | (StepParams::Pause(c), Slot::Main)
        | (StepParams::Records { count: c, .. }, Slot::Main)
        | (StepParams::PerformScript { parameter: c, .. }, Slot::Main)
        | (StepParams::SetVariable { value: c, .. }, Slot::Main)
        | (StepParams::SetField { value: c, .. }, Slot::Main)
        | (StepParams::GoToObject { name: c, .. }, Slot::Main)
        | (StepParams::SetVariable { repetition: c, .. }, Slot::Repetition)
        | (StepParams::ShowCustomDialog { title: c, .. }, Slot::Title)
        | (StepParams::ShowCustomDialog { message: c, .. }, Slot::Message) => *c = v,
        (
            StepParams::GoToLayout {
                dest: LayoutDest::NameByCalc(c) | LayoutDest::NumberByCalc(c),
                ..
            },
            Slot::Main,
        )
        | (
            StepParams::GoToRecord {
                dest: RecordDest::ByCalc(c),
                ..
            },
            Slot::Main,
        ) => *c = v.unwrap_or_default(),
        _ => {}
    }
}

/// The calculation editor dialog.
struct CalcDialog {
    slot: Slot,
    text: String,
    error: Option<String>,
    search: String,
    /// Table occurrence whose fields are listed.
    to: u32,
}

#[derive(Default)]
pub struct Workspace {
    pub open: bool,
    /// Working copy of the scripts, in display order.
    pub scripts: Vec<Script>,
    /// Edited since the last save.
    pub dirty: bool,
    /// Copy the working scripts into the engine (Save).
    pub save: bool,
    /// Script to run (after saving).
    pub run: Option<u32>,
    selected: Option<u32>,
    step: Option<usize>,
    search: String,
    rename: Option<(u32, String)>,
    /// The rename field takes the focus once, when renaming starts.
    rename_focus: bool,
    collapsed: BTreeSet<u32>,
    /// Text of each calculation field of the selected step, and errors.
    bufs: BTreeMap<Slot, String>,
    errors: BTreeMap<Slot, String>,
    calc: Option<CalcDialog>,
    /// Button labels and the input label of a Show Custom Dialog step.
    labels: Option<[String; 4]>,
    /// Breakpoints (script id, step index), kept for the session: the
    /// Script Debugger stops before these steps.
    pub breakpoints: BTreeSet<(u32, usize)>,
}

impl Workspace {
    /// Opens the workspace on the engine's scripts (keeps unsaved edits).
    pub fn show_scripts(&mut self, scripts: &[Script]) {
        if !self.dirty {
            self.scripts = scripts.to_vec();
        }
        self.open = true;
        if self.selected.is_none() {
            self.selected = self
                .scripts
                .iter()
                .find(|s| !s.is_folder() && s.options & 2 == 0)
                .map(|s| s.id);
        }
    }

    fn script_mut(&mut self) -> Option<&mut Script> {
        let id = self.selected?;
        self.scripts.iter_mut().find(|s| s.id == id)
    }

    fn select_step(&mut self, i: Option<usize>) {
        if self.step != i {
            self.step = i;
            self.bufs.clear();
            self.errors.clear();
            self.labels = None;
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
    }

    /// Draws the workspace window (its own OS window on desktop).
    pub fn ui(&mut self, ctx: &egui::Context, doc: &Doc) {
        if !self.open {
            return;
        }
        let mut open = true;
        crate::tool_window(
            ctx,
            KEY,
            tr("Script Workspace"),
            [980.0, 620.0],
            None,
            &mut open,
            |ui| {
                // Ctrl+S in the workspace's own window saves the scripts.
                if self.dirty
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
                {
                    self.save = true;
                }
                self.toolbar(ui);
                ui.separator();
                egui::Panel::left("ws_scripts")
                    .resizable(true)
                    .default_size(220.0)
                    .show(ui, |ui| self.script_list(ui));
                egui::Panel::right("ws_palette")
                    .resizable(true)
                    .default_size(220.0)
                    .show(ui, |ui| self.palette(ui));
                egui::CentralPanel::default().show(ui, |ui| {
                    let names = Names {
                        db: &doc.db,
                        scripts: &self.scripts,
                    };
                    let texts = self.step_texts(&names);
                    egui::Panel::bottom("ws_params")
                        .resizable(true)
                        .default_size(220.0)
                        .min_size(160.0)
                        .show(ui, |ui| self.params(ui, doc));
                    egui::CentralPanel::default().show(ui, |ui| self.steps(ui, &texts));
                });
                // In the workspace's window.
                self.calc_dialog(ui.ctx(), doc);
            },
        );
        if !open {
            self.open = false;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(tr("New Script")).clicked() {
                self.new_script();
            }
            let has = self.selected.is_some();
            if ui
                .add_enabled(has, egui::Button::new(tr("Rename")))
                .clicked()
                && let Some(s) = self.script_mut()
            {
                let r = (s.id, s.name.clone());
                self.rename = Some(r);
                self.rename_focus = true;
            }
            if ui
                .add_enabled(has, egui::Button::new(tr("Delete Script")))
                .clicked()
            {
                self.delete_script();
            }
            ui.separator();
            if ui
                .add_enabled(self.dirty, egui::Button::new(tr("Save")))
                .clicked()
            {
                self.save = true;
            }
            if ui.add_enabled(has, egui::Button::new(tr("Run"))).clicked() {
                self.save = self.dirty;
                self.run = self.selected;
            }
            if self.dirty {
                ui.colored_label(Color32::from_rgb(180, 90, 0), tr("Unsaved changes"));
            }
        });
    }

    fn new_script(&mut self) {
        let id = self.scripts.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let base = tr("New Script");
        let name = std::iter::once(base.to_string())
            .chain((2..).map(|n| format!("{base} {n}")))
            .find(|n| self.scripts.iter().all(|s| &s.name != n))
            .unwrap_or_default();
        let at = self
            .selected
            .and_then(|s| self.scripts.iter().position(|x| x.id == s))
            .map_or(self.scripts.len(), |i| i + 1);
        self.scripts.insert(
            at,
            Script {
                id,
                name: name.clone(),
                options: 8,
                steps: Vec::new(),
            },
        );
        self.selected = Some(id);
        self.select_step(None);
        self.rename = Some((id, name));
        self.rename_focus = true;
        self.changed();
    }

    fn delete_script(&mut self) {
        let Some(id) = self.selected else { return };
        let Some(i) = self.scripts.iter().position(|s| s.id == id) else {
            return;
        };
        // A folder goes with its contents, up to its end marker.
        let end = if self.scripts[i].is_folder() {
            let mut depth = 0;
            let mut j = i;
            while j < self.scripts.len() {
                let s = &self.scripts[j];
                if s.is_folder() {
                    depth += 1;
                } else if s.options & 2 != 0 {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            j.min(self.scripts.len() - 1)
        } else {
            i
        };
        self.scripts.drain(i..=end);
        self.selected = None;
        self.select_step(None);
        self.changed();
    }

    fn script_list(&mut self, ui: &mut egui::Ui) {
        ui.strong(tr("Scripts"));
        egui::ScrollArea::vertical()
            .id_salt("ws_script_list")
            .show(ui, |ui| {
                let mut depth = 0usize;
                // Folders whose contents are hidden, innermost last.
                let mut hidden_from: Option<usize> = None;
                let mut pick = None;
                let mut toggle = None;
                let mut renamed = None;
                for s in &self.scripts {
                    if s.options & 2 != 0 && !s.is_folder() {
                        depth = depth.saturating_sub(1);
                        if hidden_from == Some(depth) {
                            hidden_from = None;
                        }
                        continue;
                    }
                    if hidden_from.is_some() {
                        if s.is_folder() {
                            depth += 1;
                        }
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 14.0);
                        if let Some((rid, buf)) = self.rename.as_mut().filter(|r| r.0 == s.id) {
                            let r = ui.text_edit_singleline(buf);
                            if std::mem::take(&mut self.rename_focus) {
                                r.request_focus();
                            }
                            if r.lost_focus() {
                                renamed = Some((*rid, buf.clone()));
                            }
                            return;
                        }
                        if s.name == "-" {
                            ui.separator();
                            return;
                        }
                        let selected = self.selected == Some(s.id);
                        let label = if s.is_folder() {
                            let open = !self.collapsed.contains(&s.id);
                            let arrow = if open { "\u{25BE}" } else { "\u{25B8}" };
                            if ui.small_button(arrow).clicked() {
                                toggle = Some(s.id);
                            }
                            RichText::new(&s.name).strong()
                        } else {
                            RichText::new(&s.name)
                        };
                        let r = ui.selectable_label(selected, label);
                        if r.clicked() {
                            pick = Some(s.id);
                        }
                        if r.double_clicked() && !s.is_folder() {
                            // Like Run: the edited version runs.
                            self.save = self.dirty;
                            self.run = Some(s.id);
                        }
                    });
                    if s.is_folder() {
                        if self.collapsed.contains(&s.id) {
                            hidden_from = Some(depth);
                        }
                        depth += 1;
                    }
                }
                if let Some(id) = toggle
                    && !self.collapsed.remove(&id)
                {
                    self.collapsed.insert(id);
                }
                if let Some(id) = pick {
                    if self.selected != Some(id) {
                        self.step = None;
                        self.select_step(None);
                    }
                    self.selected = Some(id);
                }
                if let Some((id, name)) = renamed {
                    self.rename = None;
                    let name = name.trim().to_string();
                    if let Some(s) = self.scripts.iter_mut().find(|s| s.id == id)
                        && !name.is_empty()
                        && s.name != name
                    {
                        s.name = name;
                        self.dirty = true;
                    }
                }
            });
    }

    fn step_texts(&self, names: &Names) -> Vec<(usize, String, bool, bool)> {
        let Some(s) = self
            .selected
            .and_then(|id| self.scripts.iter().find(|s| s.id == id))
        else {
            return Vec::new();
        };
        s.steps
            .iter()
            .zip(text::indents(&s.steps))
            .map(|(st, i)| {
                (
                    i,
                    text::step_text_in(st, names, french()),
                    st.enabled,
                    st.id == id::COMMENT,
                )
            })
            .collect()
    }

    fn steps(&mut self, ui: &mut egui::Ui, texts: &[(usize, String, bool, bool)]) {
        let Some(sel) = self.selected else {
            ui.label(tr("No script selected"));
            return;
        };
        if self
            .scripts
            .iter()
            .find(|s| s.id == sel)
            .is_some_and(|s| s.is_folder())
        {
            ui.label(tr("Folder"));
            return;
        }
        let n = texts.len();
        let cur = self.step.filter(|i| *i < n);
        ui.horizontal(|ui| {
            let has = cur.is_some();
            if ui
                .add_enabled(has && cur > Some(0), egui::Button::new("\u{2191}"))
                .on_hover_text(tr("Move Up"))
                .clicked()
                && let Some(i) = cur
            {
                self.move_step(i, i - 1);
            }
            if ui
                .add_enabled(
                    has && cur.is_some_and(|i| i + 1 < n),
                    egui::Button::new("\u{2193}"),
                )
                .on_hover_text(tr("Move Down"))
                .clicked()
                && let Some(i) = cur
            {
                self.move_step(i, i + 1);
            }
            if ui
                .add_enabled(has, egui::Button::new(tr("Duplicate")))
                .clicked()
                && let Some(i) = cur
                && let Some(s) = self.script_mut()
            {
                let st = s.steps[i].clone();
                s.steps.insert(i + 1, st);
                self.select_step(Some(i + 1));
                self.changed();
            }
            if ui
                .add_enabled(has, egui::Button::new(tr("Disable/Enable")))
                .clicked()
                && let Some(i) = cur
                && let Some(s) = self.script_mut()
            {
                s.steps[i].enabled = !s.steps[i].enabled;
                self.changed();
            }
            if ui
                .add_enabled(has, egui::Button::new(tr("Delete")))
                .clicked()
                && let Some(i) = cur
            {
                self.delete_step(i);
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Delete))
            && !ui.ctx().egui_wants_keyboard_input()
            && let Some(i) = cur
        {
            self.delete_step(i);
        }
        ui.separator();
        let mut moved = None;
        let mut clicked = None;
        egui::ScrollArea::both()
            .id_salt("ws_steps")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, (indent, t, enabled, comment)) in texts.iter().enumerate() {
                    let mut text = RichText::new(format!("{}{t}", "    ".repeat(*indent)))
                        .family(egui::FontFamily::Monospace);
                    if !enabled {
                        text = text.color(Color32::from_gray(160));
                    } else if *comment {
                        text = text.color(Color32::from_rgb(40, 120, 40));
                    }
                    let id = egui::Id::NULL.with(("ws_step", sel, i));
                    // The click lands on the button inside; the drag source
                    // around it only senses drags. The gutter on the left
                    // sets and clears breakpoints, as in FileMaker.
                    let egui::InnerResponse { inner, response: r } = ui
                        .horizontal(|ui| {
                            let on = self.breakpoints.contains(&(sel, i));
                            let dot = RichText::new(if on { "\u{25CF}" } else { "\u{25CB}" })
                                .color(if on {
                                    Color32::from_rgb(200, 30, 30)
                                } else {
                                    Color32::from_gray(200)
                                });
                            if ui
                                .add(egui::Button::new(dot).frame(false))
                                .on_hover_text(tr("Breakpoint"))
                                .clicked()
                                && !self.breakpoints.remove(&(sel, i))
                            {
                                self.breakpoints.insert((sel, i));
                            }
                            ui.dnd_drag_source(id, i, |ui| {
                                ui.add(egui::Button::selectable(cur == Some(i), text))
                            })
                        })
                        .inner;
                    if inner.clicked() {
                        clicked = Some(i);
                    }
                    if let Some(from) = r.dnd_release_payload::<usize>() {
                        moved = Some((*from, i));
                    }
                }
                if texts.is_empty() {
                    ui.label(
                        RichText::new(tr("Add steps from the list on the right"))
                            .color(Color32::from_gray(140)),
                    );
                }
            });
        if let Some(i) = clicked {
            self.select_step(Some(i));
        }
        if let Some((from, to)) = moved
            && from != to
        {
            self.move_step(from, to);
        }
    }

    fn move_step(&mut self, from: usize, to: usize) {
        let Some(s) = self.script_mut() else { return };
        if from >= s.steps.len() || to >= s.steps.len() {
            return;
        }
        let st = s.steps.remove(from);
        s.steps.insert(to, st);
        self.step = Some(to);
        self.changed();
    }

    fn delete_step(&mut self, i: usize) {
        let Some(s) = self.script_mut() else { return };
        if i < s.steps.len() {
            s.steps.remove(i);
            let n = s.steps.len();
            self.select_step(if n == 0 { None } else { Some(i.min(n - 1)) });
            self.changed();
        }
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        let l = ui.strong(tr("Steps"));
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("Search")))
            .labelled_by(l.id);
        let q = self.search.to_lowercase();
        let mut add = None;
        let listed: BTreeSet<u16> = CATEGORIES
            .iter()
            .flat_map(|c| c.1.iter().copied())
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("ws_palette")
            .show(ui, |ui| {
                let mut show = |ui: &mut egui::Ui, ids: &mut dyn Iterator<Item = u16>| {
                    for i in ids {
                        let Some(n) = step_label(i) else { continue };
                        if !q.is_empty() && !n.to_lowercase().contains(&q) {
                            continue;
                        }
                        if ui.selectable_label(false, n).clicked() {
                            add = Some(i);
                        }
                    }
                };
                for (cat, ids) in CATEGORIES {
                    let has = ids
                        .iter()
                        .filter_map(|i| step_label(*i))
                        .any(|n| q.is_empty() || n.to_lowercase().contains(&q));
                    if !has {
                        continue;
                    }
                    egui::CollapsingHeader::new(tr(cat))
                        .default_open(!q.is_empty() || *cat == "Control")
                        .open((!q.is_empty()).then_some(true))
                        .show(ui, |ui| show(ui, &mut ids.iter().copied()));
                }
                let rest: Vec<u16> = (1..=247u16)
                    .filter(|i| !listed.contains(i))
                    .filter(|i| {
                        step_label(*i)
                            .is_some_and(|n| q.is_empty() || n.to_lowercase().contains(&q))
                    })
                    .collect();
                if rest.is_empty() {
                    return;
                }
                egui::CollapsingHeader::new(tr("Other"))
                    .open((!q.is_empty()).then_some(true))
                    .show(ui, |ui| show(ui, &mut rest.iter().copied()));
            });
        if let Some(i) = add {
            self.add_step(i);
        }
    }

    fn add_step(&mut self, step: u16) {
        let at = self.step.map(|i| i + 1);
        let mut st = Step::new(step);
        if let StepParams::ShowCustomDialog { buttons, .. } = &mut st.params {
            *buttons = vec![
                DialogButton {
                    label: Some(Calc(fmp12::save::text_literal_tokens("OK"))),
                    commit: true,
                },
                DialogButton {
                    label: Some(Calc(fmp12::save::text_literal_tokens(tr("Cancel")))),
                    commit: false,
                },
            ];
        }
        // If and Loop come with their closing step, like FileMaker.
        let close = match step {
            id::IF => Some(id::END_IF),
            id::LOOP => Some(id::END_LOOP),
            _ => None,
        };
        let Some(s) = self.script_mut() else { return };
        if s.is_folder() {
            return;
        }
        let at = at.unwrap_or(s.steps.len()).min(s.steps.len());
        s.steps.insert(at, st);
        if let Some(c) = close {
            s.steps.insert(at + 1, Step::new(c));
        }
        self.select_step(Some(at));
        self.changed();
    }

    /// The parameter editor of the selected step.
    fn params(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        let names = Names {
            db: &doc.db,
            scripts: &[],
        };
        let script_list: Vec<(u32, String)> = self
            .scripts
            .iter()
            .filter(|s| !s.is_folder() && s.options & 2 == 0)
            .map(|s| (s.id, s.name.clone()))
            .collect();
        let Some(i) = self.step else {
            ui.label(RichText::new(tr("Select a step to edit its options")).color(Color32::GRAY));
            return;
        };
        let Some(mut st) = self
            .selected
            .and_then(|id| self.scripts.iter().find(|s| s.id == id))
            .and_then(|s| s.steps.get(i))
            .cloned()
        else {
            return;
        };
        ui.strong(step_label(st.id).unwrap_or("?"));
        let before = st.clone();
        egui::ScrollArea::vertical()
            .id_salt("ws_params_scroll")
            .show(ui, |ui| {
                for (label, slot) in self.edit(ui, &mut st, &names, doc, &script_list) {
                    self.calc_field(ui, label, &mut st, slot, &names);
                }
            });
        if st != before
            && let Some(s) = self.script_mut()
            && let Some(x) = s.steps.get_mut(i)
        {
            *x = st;
            self.changed();
        }
    }

    /// A calculation field: one line of text compiled when it loses focus,
    /// and a button opening the calculation editor.
    fn calc_field(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        st: &mut Step,
        slot: Slot,
        names: &Names,
    ) {
        let current = slot_calc(&st.params, slot).map(|c| names.calc(c));
        let buf = self
            .bufs
            .entry(slot)
            .or_insert_with(|| current.clone().unwrap_or_default());
        let mut apply = None;
        ui.horizontal(|ui| {
            let l = ui.label(label);
            let r = ui
                .add(egui::TextEdit::singleline(buf).desired_width(ui.available_width() - 40.0))
                .labelled_by(l.id);
            if r.lost_focus() && Some(buf.as_str()) != current.as_deref() {
                apply = Some(buf.clone());
            }
            if ui
                .button("\u{2026}")
                .on_hover_text(tr("Specify Calculation"))
                .clicked()
            {
                self.calc = Some(CalcDialog {
                    slot,
                    text: buf.clone(),
                    error: None,
                    search: String::new(),
                    to: names.db.occurrences.first().map_or(0, |o| o.id),
                });
            }
        });
        if let Some(e) = self.errors.get(&slot) {
            ui.colored_label(Color32::RED, e);
        }
        if let Some(src) = apply {
            self.apply_calc(st, slot, &src, names);
        }
    }

    fn apply_calc(&mut self, st: &mut Step, slot: Slot, src: &str, names: &Names) {
        if src.trim().is_empty() {
            set_slot(&mut st.params, slot, None);
            self.errors.remove(&slot);
            return;
        }
        match names.compile(src) {
            Ok(c) => {
                set_slot(&mut st.params, slot, Some(c));
                self.errors.remove(&slot);
            }
            Err(e) => {
                self.errors.insert(slot, e.to_string());
            }
        }
        self.bufs.insert(slot, src.to_string());
    }

    fn edit(
        &mut self,
        ui: &mut egui::Ui,
        st: &mut Step,
        names: &Names,
        doc: &Doc,
        scripts: &[(u32, String)],
    ) -> Vec<(&'static str, Slot)> {
        let step_id = st.id;
        let mut calcs = Vec::new();
        match &mut st.params {
            StepParams::Comment(t) => {
                ui.add(
                    egui::TextEdit::multiline(t)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            }
            StepParams::Switch(on) => {
                ui.checkbox(on, tr("On"));
            }
            StepParams::Condition(_) | StepParams::ExitScript(_) => {
                let label = if step_id == id::EXIT_SCRIPT {
                    tr("Result:")
                } else {
                    tr("Calculation:")
                };
                calcs.push((label, Slot::Main));
            }
            StepParams::SetVariable { name, .. } => {
                ui.horizontal(|ui| {
                    let l = ui.label(tr("Name:"));
                    ui.text_edit_singleline(name).labelled_by(l.id);
                });
                if !name.starts_with('$') {
                    ui.colored_label(Color32::RED, tr("Variable names start with $ or $$"));
                }
                calcs.push((tr("Value:"), Slot::Main));
                calcs.push((tr("Repetition:"), Slot::Repetition));
            }
            StepParams::SetField { field, .. } => {
                ui.horizontal(|ui| {
                    ui.label(tr("Field:"));
                    field_picker(ui, "sf_field", doc, field);
                });
                calcs.push((tr("Value:"), Slot::Main));
            }
            StepParams::GoToField { field, select } => {
                ui.horizontal(|ui| {
                    ui.label(tr("Field:"));
                    field_picker(ui, "gf_field", doc, field);
                });
                ui.checkbox(select, tr("Select/perform"));
            }
            StepParams::InsertText {
                target,
                text,
                select,
            } => {
                ui.checkbox(select, tr("Select entire contents"));
                let mut var = match target {
                    Target::Variable(v) => v.clone(),
                    _ => String::new(),
                };
                let mut f = match target {
                    Target::Field(f) => Some(*f),
                    _ => None,
                };
                ui.horizontal(|ui| {
                    ui.label(tr("Field:"));
                    field_picker(ui, "it_field", doc, &mut f);
                    ui.label(tr("or variable:"));
                    ui.text_edit_singleline(&mut var);
                });
                *target = match (f, var.is_empty()) {
                    (_, false) => Target::Variable(var),
                    (Some(f), true) => Target::Field(f),
                    (None, true) => Target::None,
                };
                ui.add(egui::TextEdit::multiline(text).desired_rows(2));
            }
            StepParams::PerformScript { script, .. } => {
                ui.horizontal(|ui| {
                    ui.label(tr("Script:"));
                    let cur = script
                        .and_then(|s| scripts.iter().find(|x| x.0 == s))
                        .map_or("?", |x| x.1.as_str());
                    egui::ComboBox::from_id_salt("ps_script")
                        .selected_text(cur)
                        .show_ui(ui, |ui| {
                            for (sid, n) in scripts {
                                ui.selectable_value(script, Some(*sid), n);
                            }
                        });
                });
                calcs.push((tr("Parameter:"), Slot::Main));
            }
            StepParams::GoToLayout { dest, .. } => {
                let kinds = [
                    tr("original layout"),
                    tr("Layout"),
                    tr("Layout Name by calculation"),
                    tr("Layout Number by calculation"),
                ];
                let mut k = match dest {
                    LayoutDest::Original => 0,
                    LayoutDest::Layout(_) => 1,
                    LayoutDest::NameByCalc(_) => 2,
                    LayoutDest::NumberByCalc(_) => 3,
                };
                let old = k;
                egui::ComboBox::from_id_salt("gl_kind")
                    .selected_text(kinds[k])
                    .show_ui(ui, |ui| {
                        for (i, n) in kinds.iter().enumerate() {
                            ui.selectable_value(&mut k, i, *n);
                        }
                    });
                if k != old {
                    *dest = match k {
                        0 => LayoutDest::Original,
                        1 => LayoutDest::Layout(doc.layouts.first().map_or(0, |l| l.id)),
                        2 => LayoutDest::NameByCalc(Calc::default()),
                        _ => LayoutDest::NumberByCalc(Calc::default()),
                    };
                    self.bufs.clear();
                }
                match dest {
                    LayoutDest::Layout(l) => {
                        let cur = doc
                            .db
                            .layouts
                            .iter()
                            .find(|x| x.id == *l)
                            .map_or("?", |x| x.name.as_str());
                        egui::ComboBox::from_id_salt("gl_layout")
                            .selected_text(cur)
                            .show_ui(ui, |ui| {
                                for x in &doc.layouts {
                                    ui.selectable_value(l, x.id, &x.name);
                                }
                            });
                    }
                    LayoutDest::Original => {}
                    _ => calcs.push((tr("Calculation:"), Slot::Main)),
                }
            }
            StepParams::GoToRecord {
                dest,
                exit_after_last,
                dialog,
            } => {
                let kinds = [
                    tr("First"),
                    tr("Last"),
                    tr("Previous"),
                    tr("Next"),
                    tr("By Calculation"),
                ];
                let mut k = match dest {
                    RecordDest::First => 0,
                    RecordDest::Last => 1,
                    RecordDest::Previous => 2,
                    RecordDest::Next => 3,
                    RecordDest::ByCalc(_) => 4,
                };
                let old = k;
                egui::ComboBox::from_id_salt("gr_kind")
                    .selected_text(kinds[k])
                    .show_ui(ui, |ui| {
                        for (i, n) in kinds.iter().enumerate() {
                            ui.selectable_value(&mut k, i, *n);
                        }
                    });
                if k != old {
                    *dest = match k {
                        0 => RecordDest::First,
                        1 => RecordDest::Last,
                        2 => RecordDest::Previous,
                        3 => RecordDest::Next,
                        _ => RecordDest::ByCalc(Calc::default()),
                    };
                    self.bufs.clear();
                }
                if matches!(dest, RecordDest::Previous | RecordDest::Next) {
                    ui.checkbox(exit_after_last, tr("Exit after last"));
                }
                ui.checkbox(dialog, tr("With dialog"));
                if matches!(dest, RecordDest::ByCalc(_)) {
                    calcs.push((tr("Calculation:"), Slot::Main));
                }
            }
            StepParams::GoToObject { .. } => {
                calcs.push((tr("Object Name:"), Slot::Main));
            }
            StepParams::ShowCustomDialog { buttons, input, .. } => {
                // Button labels and the input label are edited as text.
                let labels = self.labels.get_or_insert_with(|| {
                    let lab =
                        |c: Option<&Calc>| c.map(|c| literal(&names.calc(c))).unwrap_or_default();
                    [
                        lab(buttons.first().and_then(|b| b.label.as_ref())),
                        lab(buttons.get(1).and_then(|b| b.label.as_ref())),
                        lab(buttons.get(2).and_then(|b| b.label.as_ref())),
                        lab(input.as_ref().and_then(|i| i.1.as_ref())),
                    ]
                });
                buttons.resize(
                    3,
                    DialogButton {
                        label: None,
                        commit: false,
                    },
                );
                for (i, b) in buttons.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} {}:", tr("Button"), i + 1));
                        if ui.text_edit_singleline(&mut labels[i]).changed() {
                            b.label = (!labels[i].is_empty())
                                .then(|| Calc(fmp12::save::text_literal_tokens(&labels[i])));
                        }
                        ui.checkbox(&mut b.commit, tr("Commit data"));
                    });
                }
                let mut f = input.as_ref().map(|i| i.0);
                ui.horizontal(|ui| {
                    ui.label(tr("Input field:"));
                    field_picker(ui, "scd_input", doc, &mut f);
                    if f.is_some() && ui.small_button("\u{2715}").clicked() {
                        f = None;
                    }
                });
                if let Some(f) = f {
                    ui.horizontal(|ui| {
                        ui.label(tr("Label:"));
                        ui.text_edit_singleline(&mut labels[3]);
                    });
                    let label = (!labels[3].is_empty())
                        .then(|| Calc(fmp12::save::text_literal_tokens(&labels[3])));
                    *input = Some((f, label));
                } else {
                    *input = None;
                }
                calcs.push((tr("Title:"), Slot::Title));
                calcs.push((tr("Message:"), Slot::Message));
            }
            StepParams::Records { dialog, .. } => {
                if step_id != id::NEW_RECORD && step_id != id::DUPLICATE_RECORD {
                    ui.checkbox(dialog, tr("With dialog"));
                }
                if step_id == id::OMIT_MULTIPLE {
                    calcs.push((tr("Calculation:"), Slot::Main));
                }
            }
            StepParams::Commit {
                dialog,
                skip_validation,
                force,
            } => {
                ui.checkbox(dialog, tr("With dialog"));
                ui.checkbox(skip_validation, tr("Skip data entry validation"));
                ui.checkbox(force, tr("Override ESS locking conflicts"));
            }
            StepParams::Mode { pause } => {
                ui.checkbox(pause, tr("Pause"));
            }
            StepParams::Pause(d) => {
                let mut timed = d.is_some();
                if ui
                    .checkbox(&mut timed, tr("For duration (seconds)"))
                    .changed()
                {
                    *d = timed.then(Calc::default);
                    self.bufs.clear();
                }
                if timed {
                    calcs.push((tr("Duration:"), Slot::Main));
                }
            }
            StepParams::Sort {
                dialog,
                restore,
                keys,
            } => {
                ui.checkbox(dialog, tr("With dialog"));
                ui.checkbox(restore, tr("Specify sort order"));
                let mut remove = None;
                for (i, (f, o)) in keys.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let mut ff = Some(*f);
                        field_picker(ui, ("sort", i), doc, &mut ff);
                        if let Some(x) = ff {
                            *f = x;
                        }
                        let mut desc = *o == SortOrder::Descending;
                        if ui.checkbox(&mut desc, tr("Descending")).changed() {
                            *o = if desc {
                                SortOrder::Descending
                            } else {
                                SortOrder::Ascending
                            };
                        }
                        if ui.small_button("\u{2715}").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    keys.remove(i);
                }
                if ui.button(tr("Add sort field")).clicked()
                    && let Some(f) = first_field(doc)
                {
                    keys.push((f, SortOrder::Ascending));
                    *restore = true;
                }
            }
            StepParams::Find {
                pause,
                restore,
                requests,
            } => {
                if step_id == id::ENTER_FIND_MODE {
                    ui.checkbox(pause, tr("Pause"));
                }
                ui.checkbox(restore, tr("Specify find requests"));
                let mut remove = None;
                for (r, q) in requests.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} {}", tr("Request"), r + 1));
                        ui.checkbox(&mut q.omit, tr("Omit"));
                        if ui.small_button("\u{2715}").clicked() {
                            remove = Some(r);
                        }
                    });
                    let mut drop = None;
                    for (k, (f, t)) in q.criteria.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            let mut ff = Some(*f);
                            field_picker(ui, ("find", r, k), doc, &mut ff);
                            if let Some(x) = ff {
                                *f = x;
                            }
                            ui.text_edit_singleline(t);
                            if ui.small_button("\u{2715}").clicked() {
                                drop = Some(k);
                            }
                        });
                    }
                    if let Some(k) = drop {
                        q.criteria.remove(k);
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        if ui.small_button(tr("Add criterion")).clicked()
                            && let Some(f) = first_field(doc)
                        {
                            q.criteria.push((f, String::new()));
                        }
                    });
                }
                if let Some(r) = remove {
                    requests.remove(r);
                }
                if ui.button(tr("New Request")).clicked() {
                    requests.push(FindRequest::default());
                    *restore = true;
                }
            }
            StepParams::RefreshWindow {
                flush_joins,
                flush_external,
            } => {
                ui.checkbox(flush_joins, tr("Flush cached join results"));
                ui.checkbox(flush_external, tr("Flush cached external data"));
            }
            StepParams::None => {
                ui.label(RichText::new(tr("This step has no options")).color(Color32::GRAY));
            }
            StepParams::Other => {
                ui.label(
                    RichText::new(tr("Editing this step's options is not supported yet"))
                        .color(Color32::GRAY),
                );
            }
        }
        calcs
    }

    fn calc_dialog(&mut self, ctx: &egui::Context, doc: &Doc) {
        let Some(d) = self.calc.as_mut() else { return };
        let names = Names {
            db: &doc.db,
            scripts: &[],
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::NULL.with("ws_calc")).show(ctx, |ui| {
            ui.set_min_width(640.0);
            ui.heading(tr("Specify Calculation"));
            ui.columns(2, |cols| {
                // Fields of a table occurrence.
                let ui = &mut cols[0];
                let cur = doc
                    .db
                    .occurrences
                    .iter()
                    .find(|o| o.id == d.to)
                    .map_or("?", |o| o.name.as_str());
                egui::ComboBox::from_id_salt("calc_to")
                    .selected_text(cur)
                    .show_ui(ui, |ui| {
                        for o in &doc.db.occurrences {
                            ui.selectable_value(&mut d.to, o.id, &o.name);
                        }
                    });
                let to_name = cur.to_string();
                egui::ScrollArea::vertical()
                    .id_salt("calc_fields")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for f in doc
                            .db
                            .table_of(d.to)
                            .map(|t| t.fields.as_slice())
                            .unwrap_or_default()
                        {
                            if ui.selectable_label(false, &f.name).double_clicked() {
                                d.text.push_str(&format!("{to_name}::{}", f.name));
                            }
                        }
                    });
                // Functions and Get flags.
                let ui = &mut cols[1];
                ui.add(egui::TextEdit::singleline(&mut d.search).hint_text(tr("Search functions")));
                let (funcs, flags) = fmp12::calc::function_names();
                let q = d.search.to_lowercase();
                egui::ScrollArea::vertical()
                    .id_salt("calc_funcs")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for f in funcs.iter().filter(|f| f.to_lowercase().contains(&q)) {
                            if ui.selectable_label(false, *f).double_clicked() {
                                d.text.push_str(&format!("{f} ( )"));
                            }
                        }
                        for f in flags.iter().filter(|f| f.to_lowercase().contains(&q)) {
                            let g = format!("Get ( {f} )");
                            if ui.selectable_label(false, &g).double_clicked() {
                                d.text.push_str(&g);
                            }
                        }
                    });
            });
            ui.horizontal_wrapped(|ui| {
                for op in [
                    "+", "-", "*", "/", "^", "&", "=", "\u{2260}", "<", ">", "\u{2264}",
                    "\u{2265}", "and", "or", "not", "\"\"", "\u{B6}", "( )",
                ] {
                    if ui.small_button(op).clicked() {
                        d.text.push_str(&format!(" {op} "));
                    }
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut d.text)
                    .code_editor()
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
            if let Some(e) = &d.error {
                ui.colored_label(Color32::RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    done = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                let src = d.text.clone();
                if !src.trim().is_empty()
                    && let Err(e) = names.compile(&src)
                {
                    d.error = Some(e.to_string());
                    return;
                }
                let slot = d.slot;
                self.calc = None;
                let Some(i) = self.step else { return };
                let Some(mut st) = self
                    .selected
                    .and_then(|id| self.scripts.iter().find(|s| s.id == id))
                    .and_then(|s| s.steps.get(i))
                    .cloned()
                else {
                    return;
                };
                self.apply_calc(&mut st, slot, &src, &names);
                if let Some(s) = self.script_mut()
                    && let Some(x) = s.steps.get_mut(i)
                {
                    *x = st;
                    self.changed();
                }
            }
            Some(false) => self.calc = None,
            None => {}
        }
    }
}

/// The text of a calculation that is one text literal, unquoted (else the
/// calculation as is).
fn literal(src: &str) -> String {
    src.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .filter(|s| !s.contains('"'))
        .map_or_else(|| src.to_string(), str::to_string)
}

/// The first field of the first table occurrence (new sort keys, criteria).
fn first_field(doc: &Doc) -> Option<FieldRef> {
    let l = doc.layouts.first()?;
    let to = (l.table_occurrence & 0xFFF) as u32;
    let f = doc.db.table_of(to)?.fields.first()?;
    Some(FieldRef {
        to,
        field: f.id,
        repetition: 1,
    })
}

/// Table occurrence and field pop-up menus.
fn field_picker(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + Copy + std::fmt::Debug,
    doc: &Doc,
    f: &mut Option<FieldRef>,
) {
    let to = f
        .map(|x| x.to)
        .or_else(|| doc.db.occurrences.first().map(|o| o.id));
    let to_name = to
        .and_then(|t| doc.db.occurrences.iter().find(|o| o.id == t))
        .map_or("?", |o| o.name.as_str());
    let mut new_to = to;
    egui::ComboBox::from_id_salt(("fp_to", salt))
        .selected_text(to_name)
        .show_ui(ui, |ui| {
            for o in &doc.db.occurrences {
                ui.selectable_value(&mut new_to, Some(o.id), &o.name);
            }
        });
    let Some(t) = new_to else { return };
    let fields = doc
        .db
        .table_of(t)
        .map(|x| x.fields.as_slice())
        .unwrap_or_default();
    let cur = f
        .filter(|x| x.to == t)
        .and_then(|x| fields.iter().find(|y| y.id == x.field))
        .map_or("?", |y| y.name.as_str());
    egui::ComboBox::from_id_salt(("fp_field", salt))
        .selected_text(cur)
        .show_ui(ui, |ui| {
            for y in fields {
                if ui
                    .selectable_label(f.is_some_and(|x| x.to == t && x.field == y.id), &y.name)
                    .clicked()
                {
                    *f = Some(FieldRef {
                        to: t,
                        field: y.id,
                        repetition: 1,
                    });
                }
            }
        });
}
