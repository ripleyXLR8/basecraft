//! Tools › Script Debugger, as in FileMaker: while it is open a script
//! stops before its first step, and before every breakpoint set in the
//! Script Workspace (kept for the session). Step, Step Into, Step Out, Run
//! and Halt go on from there; the window shows the current script with
//! the next step highlighted, the call stack and the last error. The
//! Data Viewer's "Current" tab lists the local variables of the stopped
//! script.
//!
//! Built on the resumable run (`script::Run`): the run suspends with
//! `Suspend::Breakpoint` and resumes after `Run::set_stepping`. Its own OS
//! window on desktop, an in-page window in the browser and the UI tests.

use eframe::egui::{self, Color32, RichText};
use script::text::{self, Names};
use script::{Answer, Stepping, Suspend};

use crate::{App, i18n, tr};

/// Viewport / window key of the debugger.
pub const KEY: &str = "script_debugger";

/// UI strings: English keys, French as FileMaker 26 shows them.
pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return tr(key);
    }
    match key {
        "Script Debugger" => "Débogueur de scripts",
        "Step" => "Pas à pas",
        "Step Into" => "Pas à pas détaillé",
        "Step Out" => "Pas à pas sortant",
        "Run" => "Exécuter",
        "Halt Script" => "Arrêter le script",
        "Call Stack" => "Pile d'appels",
        "Last Error:" => "Dernière erreur :",
        "No script is paused." => "Aucun script en pause.",
        "Current" => "Actif",
        "Watch" => "Surveiller",
        "Name" => "Nom",
        "Value" => "Valeur",
        k => tr(k),
    }
}

#[derive(Default)]
pub struct Debugger {
    /// Tools › Script Debugger: scripts stop in it while it is open.
    pub open: bool,
    /// Data Viewer: the "Current" tab is shown (else "Watch").
    pub current_tab: bool,
}

impl App {
    /// Whether a run is stopped in the debugger.
    fn at_break(&self) -> bool {
        matches!(
            self.active.as_ref().map(|a| &a.wait),
            Some(Suspend::Breakpoint)
        )
    }

    /// Goes on from a breakpoint: `None` halts the script.
    pub fn debug_go(&mut self, how: Option<Stepping>, now: f64) {
        if !self.at_break() {
            return;
        }
        match how {
            Some(s) => {
                if let Some(a) = self.active.as_mut() {
                    a.run.set_stepping(s);
                }
                self.answer(Answer::Continue, now);
            }
            None => self.answer(Answer::Cancel, now),
        }
    }

    /// The debugger window (Tools › Script Debugger).
    pub fn debugger_window(&mut self, ctx: &egui::Context) {
        if !self.debugger.open {
            return;
        }
        let now = ctx.input(|i| i.time);
        let paused = self.at_break();
        let stack = self
            .active
            .as_ref()
            .filter(|_| paused)
            .map(|a| a.run.stack())
            .unwrap_or_default();
        // Steps of the innermost script, as the Script Workspace shows them.
        let lines: Vec<(usize, String, bool)> = match (stack.last(), &self.engine, &self.doc) {
            (Some(l), Some(e), Some(d)) => e
                .scripts
                .iter()
                .find(|s| s.id == l.script)
                .map(|s| {
                    let names = Names {
                        db: &d.db,
                        scripts: &e.scripts,
                    };
                    s.steps
                        .iter()
                        .zip(text::indents(&s.steps))
                        .map(|(st, i)| {
                            (
                                i,
                                text::step_text_in(st, &names, i18n::french()),
                                st.enabled,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let last_error = self.engine.as_ref().map_or(0, |e| e.last_error());
        let mut go: Option<Option<Stepping>> = None;
        let mut toggle = None;
        let mut open = true;
        crate::tool_window(
            ctx,
            KEY,
            t("Script Debugger"),
            [560.0, 420.0],
            None,
            &mut open,
            |ui| {
                ui.horizontal(|ui| {
                    for (label, how) in [
                        ("Step", Some(Stepping::Over)),
                        ("Step Into", Some(Stepping::Into)),
                        ("Step Out", Some(Stepping::Out)),
                        ("Run", Some(Stepping::Run)),
                        ("Halt Script", None),
                    ] {
                        if ui
                            .add_enabled(paused, egui::Button::new(t(label)))
                            .clicked()
                        {
                            go = Some(how);
                        }
                    }
                });
                ui.separator();
                let Some(level) = stack.last() else {
                    ui.label(t("No script is paused."));
                    return;
                };
                ui.strong(&level.name);
                egui::ScrollArea::vertical()
                    .id_salt("dbg_steps")
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (i, (indent, line, enabled)) in lines.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let on = self.workspace.breakpoints.contains(&(level.script, i));
                                let dot = RichText::new(if on { "\u{25CF}" } else { "\u{25CB}" })
                                    .color(if on {
                                        Color32::from_rgb(200, 30, 30)
                                    } else {
                                        Color32::from_gray(200)
                                    });
                                if ui.add(egui::Button::new(dot).frame(false)).clicked() {
                                    toggle = Some((level.script, i));
                                }
                                let mut txt =
                                    RichText::new(format!("{}{line}", "    ".repeat(*indent)))
                                        .family(egui::FontFamily::Monospace);
                                if !enabled {
                                    txt = txt.color(Color32::from_gray(160));
                                }
                                // The step about to run.
                                if i == level.step {
                                    txt = txt.background_color(Color32::from_rgb(255, 236, 140));
                                }
                                ui.label(txt);
                            });
                        }
                    });
                ui.separator();
                ui.strong(t("Call Stack"));
                for l in stack.iter().rev() {
                    ui.label(format!("{}  [{}]", l.name, l.step + 1));
                }
                ui.separator();
                ui.label(format!("{} {last_error}", t("Last Error:")));
            },
        );
        if let Some(k) = toggle {
            if !self.workspace.breakpoints.remove(&k) {
                self.workspace.breakpoints.insert(k);
            }
            if let Some(a) = self.active.as_mut() {
                a.run.breakpoints = self.workspace.breakpoints.clone();
            }
        }
        self.debugger.open = open;
        // Closing the debugger lets a stopped script run on.
        if !open {
            if let Some(a) = self.active.as_mut() {
                a.run.set_debug(false);
            }
            go = Some(Some(Stepping::Run));
        }
        if let Some(how) = go {
            self.debug_go(how, now);
        }
    }

    /// Data Viewer › Current: the stopped script's local variables and
    /// the `$$` globals.
    pub fn data_viewer_current(&self, ui: &mut egui::Ui) {
        let mut rows: Vec<(String, String)> = self
            .active
            .as_ref()
            .map(|a| {
                a.run
                    .locals()
                    .into_iter()
                    .map(|(n, r, v)| (var_name(&n, r), v.to_text()))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(e) = &self.engine {
            let mut g: Vec<_> = e
                .globals
                .iter()
                .map(|((n, r), v)| (var_name(n, *r), v.to_text()))
                .collect();
            g.sort();
            rows.extend(g);
        }
        egui::Grid::new("dv_current")
            .striped(true)
            .num_columns(2)
            .show(ui, |ui| {
                ui.strong(t("Name"));
                ui.strong(t("Value"));
                ui.end_row();
                for (n, v) in rows {
                    ui.monospace(n);
                    ui.label(v.replace('\r', "\u{b6}"));
                    ui.end_row();
                }
            });
    }
}

/// `$x`, or `$x[2]` for another repetition.
fn var_name(n: &str, r: usize) -> String {
    if r == 1 {
        n.to_string()
    } else {
        format!("{n}[{r}]")
    }
}
