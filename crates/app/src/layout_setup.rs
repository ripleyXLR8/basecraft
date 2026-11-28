//! Layouts › Layout Setup… (FileMaker's « Format de modèle »): General,
//! Views, Printing and Script Triggers tabs, laid out and labelled like
//! FileMaker Pro 26's dialog. The settings are fields of
//! [`fmp12::layouts::Layout`] (`docs/FORMAT.md` §10, "Layout Setup"),
//! written by the layout writer. Browse mode applies the available views,
//! Quick Find and "Include in layout menus" ([`App::enforce_view`] and the
//! layout bar).

use eframe::egui;
use fmp12::layouts::{Layout, setup};
// `ev`, not `event`: coverage_tests counts `event::` uses as triggers
// the app fires; this module only lists them.
use fmp12::triggers::{Trigger, event as ev, mode};
use script::View;

use crate::{App, Doc, i18n};

pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Layout Setup" => "Format de modèle",
        "General" => "Général",
        "Views" => "Vues",
        "Printing" => "Impression",
        "Script Triggers" => "Déclencheurs de script",
        "Layout Name" => "Nom",
        "Include in layout menus" => "Afficher dans les menus de modèle",
        "Show records from:" => "Afficher les enregistrements de :",
        "Save record changes automatically" => "Enregistrer autom. les modif. d'enreg.",
        "Show field frames when record is active" => "Afficher cadres de rubr. quand enreg. actif",
        "Delineate fields on current row only" => {
            "Délimiter les rubr. pour l'enreg. actuel uniquement"
        }
        "Show current record indicator in List view" => {
            "Afficher indicateur enreg. actif dans la vue Liste"
        }
        "Menu Set:" => "Jeu de menus :",
        "[Standard FileMaker Menus]" => "[Menus FileMaker standard]",
        "[File Default]" => "[Menus par défaut du fichier]",
        "Menu set" => "Jeu de menus",
        "Enable Quick Find" => "Activer la recherche rapide",
        "Reset Quick Find" => "Réinitialiser la recherche rapide",
        "Views available in Browse and Find mode:" => {
            "Vues disponibles en mode Utilisation et mode Recherche :"
        }
        "Form" => "Formulaire",
        "List" => "Liste",
        "Table" => "Tableau",
        "Properties…" => "Propriétés...",
        "(at least one view must be selected)" => "(une vue au moins doit être sélectionnée)",
        "Default view:" => "Vue par défaut :",
        "Form View" => "Vue Formulaire",
        "List View" => "Vue Liste",
        "Table View" => "Vue Tableau",
        "Print in" => "Imprimer sur",
        "columns" => "colonne(s)",
        "Across first" => "Sens horizontal",
        "Down first" => "Sens vertical",
        "Use fixed page margins" => "Utiliser des marges de page fixes",
        "Top:" => "Haut :",
        "Bottom:" => "Bas :",
        "Left:" => "Gauche :",
        "Right:" => "Droite :",
        "Inside:" => "Intérieure :",
        "Outside:" => "Extérieure :",
        "Facing pages" => "Recto verso",
        "Specify a script to run for an event" => {
            "Spécifier un script à exécuter pour un événement"
        }
        "Event" => "Evénement",
        "Script Trigger Properties" => "Propriétés de déclencheur de script",
        "Event:" => "Evénement :",
        "Script:" => "Script :",
        "Enable in:" => "Activer en :",
        "Browse" => "Utilisation",
        "Find" => "Recherche",
        "Preview" => "Prévisualisation",
        "OnRecordLoad" => "SurChargementEnregistrement",
        "OnRecordCommit" => "SurValidationEnregistrement",
        "OnRecordRevert" => "SurRetablirEnregistrement",
        "OnLayoutKeystroke" => "SurFrappeClavierModele",
        "OnLayoutEnter" => "SurEntreeModele",
        "OnLayoutExit" => "SurSortieModele",
        "OnLayoutSizeChange" => "SurModificationTailleModele",
        "OnModeEnter" => "SurEntreeMode",
        "OnModeExit" => "SurSortieMode",
        "OnViewChange" => "SurChangementVue",
        "OnGestureTap" => "SurGesteAppui",
        "OnExternalCommandReceived" => "SurRéceptionCommandeExterne",
        k => i18n::tr(k),
    }
}

/// Layout events in the order of FileMaker's list.
const EVENTS: [u16; 12] = [
    ev::RECORD_LOAD,
    ev::RECORD_COMMIT,
    ev::RECORD_REVERT,
    ev::LAYOUT_KEYSTROKE,
    ev::LAYOUT_ENTER,
    ev::LAYOUT_EXIT,
    ev::LAYOUT_SIZE_CHANGE,
    ev::MODE_ENTER,
    ev::MODE_EXIT,
    ev::VIEW_CHANGE,
    ev::GESTURE_TAP,
    ev::EXTERNAL_COMMAND_RECEIVED,
];

/// Events FileMaker lets run in Preview mode (it greys the box out for
/// record events and keystrokes). ponytail: from the dialog of
/// OnRecordLoad only; the others follow FileMaker's documentation.
fn preview_allowed(e: u16) -> bool {
    !matches!(
        e,
        ev::RECORD_LOAD
            | ev::RECORD_COMMIT
            | ev::RECORD_REVERT
            | ev::LAYOUT_KEYSTROKE
            | ev::GESTURE_TAP
    )
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    General,
    Views,
    Printing,
    Triggers,
}

/// The dialog's state: a copy of the layout being set up.
pub struct Setup {
    pub layout: Layout,
    pub tab: Tab,
    /// Event selected in the Script Triggers list.
    event: u16,
}

impl Setup {
    pub fn new(l: &Layout) -> Self {
        Setup {
            layout: l.clone(),
            tab: Tab::General,
            event: EVENTS[0],
        }
    }

    /// Draws the dialog: `Some(true)` OK, `Some(false)` Cancel.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        doc: &Doc,
        scripts: &[(u32, String)],
    ) -> Option<bool> {
        let mut out = None;
        egui::Modal::new(egui::Id::NULL.with("layout_setup")).show(ctx, |ui| {
            ui.set_width(430.0);
            ui.heading(t("Layout Setup"));
            ui.horizontal(|ui| {
                for (tab, name) in [
                    (Tab::General, "General"),
                    (Tab::Views, "Views"),
                    (Tab::Printing, "Printing"),
                    (Tab::Triggers, "Script Triggers"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, t(name));
                }
            });
            ui.separator();
            match self.tab {
                Tab::General => self.general(ui, doc),
                Tab::Views => self.views(ui),
                Tab::Printing => self.printing(ui),
                Tab::Triggers => self.triggers(ui, scripts),
            }
            ui.separator();
            let ok = !self.layout.name.trim().is_empty() && self.layout.views().contains(&true);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t("Cancel")).clicked() {
                    out = Some(false);
                }
                if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                    self.layout.name = self.layout.name.trim().to_string();
                    out = Some(true);
                }
            });
        });
        out
    }

    fn general(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        let l = &mut self.layout;
        ui.label(t("Layout Name"));
        ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(f32::INFINITY));
        flag(
            ui,
            &mut l.flags,
            setup::HIDDEN,
            true,
            "Include in layout menus",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(t("Show records from:"));
            let cur = (l.table_occurrence & 0xFFF) as u32;
            let name = doc.db.occurrences.iter().find(|o| o.id == cur);
            egui::ComboBox::from_id_salt("setup_to")
                .selected_text(name.map_or("?", |o| o.name.as_str()))
                .show_ui(ui, |ui| {
                    for o in &doc.db.occurrences {
                        if ui.selectable_label(o.id == cur, &o.name).clicked() {
                            l.table_occurrence = 0x104000 | o.id as u64;
                        }
                    }
                });
        });
        flag(
            ui,
            &mut l.options,
            setup::NO_AUTO_SAVE,
            true,
            "Save record changes automatically",
        );
        flag(
            ui,
            &mut l.options,
            setup::FIELD_FRAMES,
            false,
            "Show field frames when record is active",
        );
        flag(
            ui,
            &mut l.options,
            setup::DELINEATE,
            false,
            "Delineate fields on current row only",
        );
        flag(
            ui,
            &mut l.options,
            setup::NO_RECORD_INDICATOR,
            true,
            "Show current record indicator in List view",
        );
        ui.horizontal(|ui| {
            ui.label(t("Menu Set:"));
            let name = |id: u64| match id {
                0 => t("[File Default]").to_string(),
                1 => t("[Standard FileMaker Menus]").to_string(),
                // ponytail: custom menu sets are not decoded; their id only.
                n => format!("{} {n}", t("Menu set")),
            };
            egui::ComboBox::from_id_salt("setup_menu_set")
                .selected_text(name(l.menu_set))
                .show_ui(ui, |ui| {
                    let mut ids = vec![1, 0];
                    if l.menu_set > 1 {
                        ids.push(l.menu_set);
                    }
                    for id in ids {
                        ui.selectable_value(&mut l.menu_set, id, name(id));
                    }
                });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            flag(
                ui,
                &mut l.options,
                setup::NO_QUICK_FIND,
                true,
                "Enable Quick Find",
            );
            // ponytail: FileMaker enables it once fields were excluded
            // from Quick Find on this layout; BaseCraft has nothing to reset.
            ui.add_enabled(false, egui::Button::new(t("Reset Quick Find")));
        });
    }

    fn views(&mut self, ui: &mut egui::Ui) {
        let l = &mut self.layout;
        ui.label(t("Views available in Browse and Find mode:"));
        for (bit, name) in [
            (setup::NO_FORM, "Form"),
            (setup::NO_LIST, "List"),
            (setup::NO_TABLE, "Table"),
        ] {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                flag(ui, &mut l.options, bit, true, name);
                if bit == setup::NO_TABLE {
                    // ponytail: Table View Properties (grid, parts, column
                    // headers, row height) are not decoded.
                    ui.add_enabled(false, egui::Button::new(t("Properties…")));
                }
            });
        }
        ui.label(t("(at least one view must be selected)"));
        let views = l.views();
        // The default view stays one of the available views.
        if !views[l.default_view()]
            && let Some(v) = views.iter().position(|v| *v)
        {
            l.set_default_view(v);
        }
        let names = ["Form View", "List View", "Table View"];
        ui.horizontal(|ui| {
            ui.label(t("Default view:"));
            egui::ComboBox::from_id_salt("setup_default_view")
                .selected_text(t(names[l.default_view()]))
                .show_ui(ui, |ui| {
                    for (v, name) in names.iter().enumerate() {
                        let r = ui.add_enabled(
                            views[v],
                            egui::Button::selectable(l.default_view() == v, t(name)),
                        );
                        if r.clicked() {
                            l.set_default_view(v);
                        }
                    }
                });
        });
    }

    fn printing(&mut self, ui: &mut egui::Ui) {
        let l = &mut self.layout;
        ui.horizontal(|ui| {
            flag(ui, &mut l.options, setup::COLUMNS, false, "Print in");
            let on = l.options & setup::COLUMNS != 0;
            ui.add_enabled(on, egui::DragValue::new(&mut l.columns).range(1..=99));
            ui.label(t("columns"));
        });
        let on = l.options & setup::COLUMNS != 0;
        ui.add_enabled_ui(on, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                let mut down = l.options & setup::COLUMNS_DOWN != 0;
                ui.radio_value(&mut down, false, t("Across first"));
                ui.radio_value(&mut down, true, t("Down first"));
                set(&mut l.options, setup::COLUMNS_DOWN, down);
            });
        });
        ui.add_space(8.0);
        flag(
            ui,
            &mut l.options,
            setup::FIXED_MARGINS,
            false,
            "Use fixed page margins",
        );
        let fixed = l.options & setup::FIXED_MARGINS != 0;
        let facing = l.options & setup::FACING_PAGES != 0;
        ui.add_enabled_ui(fixed, |ui| {
            let (left, right) = if facing {
                ("Inside:", "Outside:")
            } else {
                ("Left:", "Right:")
            };
            egui::Grid::new("setup_margins").show(ui, |ui| {
                // Field 12 order: top, left, bottom, right.
                for row in [[("Top:", 0), (left, 1)], [("Bottom:", 2), (right, 3)]] {
                    for (name, i) in row {
                        ui.label(t(name));
                        ui.add(
                            egui::DragValue::new(&mut l.margins[i])
                                .range(0.0..=1000.0)
                                .suffix(" pt"),
                        );
                    }
                    ui.end_row();
                }
            });
            flag(
                ui,
                &mut l.options,
                setup::FACING_PAGES,
                false,
                "Facing pages",
            );
        });
    }

    fn triggers(&mut self, ui: &mut egui::Ui, scripts: &[(u32, String)]) {
        let script_name = |id: u32| {
            scripts
                .iter()
                .find(|s| s.0 == id)
                .map_or(format!("#{id}"), |s| format!("“{}”", s.1))
        };
        ui.label(t("Specify a script to run for an event"));
        let ts = &mut self.layout.triggers;
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, |ui| {
                egui::Grid::new("setup_triggers")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong(t("Event"));
                        ui.strong("Script");
                        ui.end_row();
                        for e in EVENTS {
                            let name = fmp12::triggers::event_name(e).unwrap_or("?");
                            let i = ts.iter().position(|x| x.event == e);
                            ui.horizontal(|ui| {
                                let mut on = i.is_some();
                                if ui.checkbox(&mut on, "").changed() {
                                    match (on, i) {
                                        (true, None) => ts.push(Trigger {
                                            event: e,
                                            script: scripts.first().map_or(0, |s| s.0),
                                            modes: mode::BROWSE,
                                            parameter: None,
                                        }),
                                        (false, Some(i)) => {
                                            ts.remove(i);
                                        }
                                        _ => {}
                                    }
                                    self.event = e;
                                }
                                if ui.selectable_label(self.event == e, t(name)).clicked() {
                                    self.event = e;
                                }
                            });
                            ui.label(
                                ts.iter()
                                    .find(|x| x.event == e)
                                    .map_or(String::new(), |x| script_name(x.script)),
                            );
                            ui.end_row();
                        }
                    });
            });
        ui.separator();
        ui.strong(t("Script Trigger Properties"));
        let e = self.event;
        ui.label(format!(
            "{} {}",
            t("Event:"),
            t(fmp12::triggers::event_name(e).unwrap_or("?"))
        ));
        let Some(tr) = ts.iter_mut().find(|x| x.event == e) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(t("Script:"));
            egui::ComboBox::from_id_salt("setup_trigger_script")
                .selected_text(script_name(tr.script))
                .show_ui(ui, |ui| {
                    for (id, name) in scripts {
                        ui.selectable_value(&mut tr.script, *id, name);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(t("Enable in:"));
            for (bit, name) in [(mode::BROWSE, "Browse"), (mode::FIND, "Find")] {
                let mut on = tr.modes & bit != 0;
                ui.checkbox(&mut on, t(name));
                set32(&mut tr.modes, bit, on);
            }
            ui.add_enabled_ui(preview_allowed(e), |ui| {
                let mut on = tr.modes & mode::PREVIEW != 0;
                ui.checkbox(&mut on, t("Preview"));
                set32(&mut tr.modes, mode::PREVIEW, on);
            });
        });
    }
}

fn set(v: &mut u64, bit: u64, on: bool) {
    if on {
        *v |= bit;
    } else {
        *v &= !bit;
    }
}

fn set32(v: &mut u32, bit: u32, on: bool) {
    if on {
        *v |= bit;
    } else {
        *v &= !bit;
    }
}

/// A checkbox for one bit; `inverted` when the bit means "off".
fn flag(ui: &mut egui::Ui, v: &mut u64, bit: u64, inverted: bool, label: &'static str) {
    let mut on = (*v & bit != 0) != inverted;
    if ui.checkbox(&mut on, t(label)).changed() {
        set(v, bit, on != inverted);
    }
}

/// Copies the Layout Setup settings of `from` into `to`; true if any changed.
pub fn apply(to: &mut Layout, from: &Layout) -> bool {
    let same = to.name == from.name
        && to.table_occurrence == from.table_occurrence
        && to.options == from.options
        && to.flags == from.flags
        && to.menu_set == from.menu_set
        && to.columns == from.columns
        && to.margins == from.margins
        && to.triggers == from.triggers;
    if !same {
        to.name.clone_from(&from.name);
        to.table_occurrence = from.table_occurrence;
        to.options = from.options;
        to.flags = from.flags;
        to.menu_set = from.menu_set;
        to.columns = from.columns;
        to.margins = from.margins;
        to.triggers.clone_from(&from.triggers);
    }
    !same
}

/// Browse mode's Layout Setup effects on the fields of record `rec`:
/// (frames, plain). Frames around them when it is the `active` record
/// ("Show field frames when record is active"); no fill or border on a
/// list row other than the `current` one ("Delineate fields on current
/// row only").
pub fn field_marks(
    options: u64,
    list: bool,
    rec: Option<usize>,
    active: Option<usize>,
    current: Option<usize>,
) -> (bool, bool) {
    let frames = options & setup::FIELD_FRAMES != 0 && rec.is_some() && rec == active;
    let plain = list && options & setup::DELINEATE != 0 && rec != current;
    (frames, plain)
}

/// "Show current record indicator in List view" (on by default).
pub fn record_indicator(l: &Layout) -> bool {
    l.options & setup::NO_RECORD_INDICATOR == 0
}

/// The view each window last showed each layout in.
#[derive(Default)]
pub struct Views {
    /// (window, layout id) → view.
    shown: std::collections::BTreeMap<(String, u32), View>,
    /// The window and layout of the last frame, and its view.
    last: Option<((String, u32), View)>,
}

impl App {
    /// Views the current layout offers in Browse and Find mode.
    pub fn views_allowed(&self) -> [bool; 3] {
        self.doc
            .as_ref()
            .and_then(|d| d.layouts.get(self.layout))
            .map_or([true; 3], Layout::views)
    }

    /// The current layout's "Enable Quick Find".
    pub fn quick_find_enabled(&self) -> bool {
        self.doc
            .as_ref()
            .and_then(|d| d.layouts.get(self.layout))
            .is_none_or(|l| l.options & setup::NO_QUICK_FIND == 0)
    }

    /// FileMaker opens a layout in its default view the first time a
    /// window shows it, then in the view it was left in (a script that
    /// changes the layout and the view at once keeps its view). A view the
    /// layout does not offer becomes its default view.
    pub fn enforce_view(&mut self) {
        let key = (
            self.front_name(),
            self.doc
                .as_ref()
                .and_then(|d| d.layouts.get(self.layout))
                .map_or(0, |l| l.id),
        );
        let first = self
            .layout_views
            .last
            .as_ref()
            .is_none_or(|(k, v)| *k != key && *v == self.view);
        if first {
            let d = self
                .doc
                .as_ref()
                .and_then(|d| d.layouts.get(self.layout))
                .map_or(0, Layout::default_view);
            self.view = self
                .layout_views
                .shown
                .get(&key)
                .copied()
                .unwrap_or([View::Form, View::List, View::Table][d]);
        }
        self.allowed_view();
        self.layout_views.shown.insert(key.clone(), self.view);
        self.layout_views.last = Some((key, self.view));
    }

    fn allowed_view(&mut self) {
        let views = self.views_allowed();
        let i = match self.view {
            View::Form => 0,
            View::List => 1,
            View::Table => 2,
        };
        if views[i] {
            return;
        }
        let d = self
            .doc
            .as_ref()
            .and_then(|d| d.layouts.get(self.layout))
            .map_or(0, Layout::default_view);
        let d = if views[d] {
            d
        } else {
            views.iter().position(|v| *v).unwrap_or(0)
        };
        self.view = [View::Form, View::List, View::Table][d];
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui_tests::{harness_for, shot};

    /// Layouts › Layout Setup… in Layout mode: the four tabs, Quick Find
    /// and fixed margins changed, OK, Exit Layout › Save; the settings
    /// reach the file, and Browse mode greys out Quick Find. Screenshots
    /// `setup_*` with `BASECRAFT_SHOTS`.
    #[test]
    fn layout_setup_dialog() {
        let Some(mut app) = crate::tests::template("Contacts") else {
            return;
        };
        app.doc.as_mut().unwrap().path = None;
        let mut h = harness_for(app);
        h.run_steps(4);
        let d = h.state().doc.as_ref().unwrap();
        let i = d
            .layouts
            .iter()
            .position(|l| l.name == "Contact Details")
            .unwrap();
        h.state_mut().layout = i;
        h.run_steps(2);
        assert!(h.state().quick_find_enabled());
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
        h.run_steps(3);
        // The layout bar's button (the Layouts menu has the same item).
        h.get_all_by_label(i18n::tr("Layout Setup…"))
            .next()
            .unwrap()
            .click();
        h.run_steps(3);
        shot(&mut h, "setup_general");
        h.get_by_label(t("Include in layout menus"));
        h.get_by_label(t("Enable Quick Find")).click();
        h.run_steps(2);
        for tab in ["Views", "Script Triggers", "Printing"] {
            h.get_by_label(t(tab)).click();
            h.run_steps(2);
            shot(&mut h, &format!("setup_{tab}"));
        }
        h.get_by_label(t("Use fixed page margins")).click();
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        let ds = h.state().designer.as_ref().unwrap();
        let want = setup::NO_QUICK_FIND | setup::FIXED_MARGINS;
        assert_eq!(ds.layout.options & want, want);
        h.get_by_label(i18n::tr("Exit Layout")).click();
        h.run_steps(6);
        h.get_by_label(i18n::tr("Save")).click();
        h.run_steps(3);
        assert!(!h.state().quick_find_enabled());
        let d = h.state().doc.as_ref().unwrap();
        let bytes = fmp12::save::save_layouts(&d.file, &d.db.layouts).unwrap();
        let back = fmp12::layouts::layouts(&fmp12::File::from_bytes(bytes).unwrap()).unwrap();
        let l = back.iter().find(|l| l.name == "Contact Details").unwrap();
        assert_eq!(l.options & want, want);
    }

    #[test]
    fn views_follow_the_layout() {
        let Some(mut app) = crate::tests::template("Contacts") else {
            return;
        };
        let i = app.layout;
        let d = app.doc.as_mut().unwrap();
        d.layouts[i].options |= setup::NO_FORM | setup::NO_LIST;
        d.layouts[i].options &= !setup::NO_TABLE;
        app.view = View::Form;
        app.enforce_view();
        assert_eq!(app.view, View::Table);
        assert_eq!(app.views_allowed(), [false, false, true]);
    }

    #[test]
    fn apply_copies_settings() {
        let mut a = Layout::default();
        let mut b = a.clone();
        assert!(!apply(&mut a, &b));
        b.menu_set = 1;
        b.margins = [1.0, 2.0, 3.0, 4.0];
        assert!(apply(&mut a, &b));
        assert_eq!(a, b);
    }

    /// Field frames on the active record only; plain fields on the other
    /// list rows; the record indicator follows its option.
    #[test]
    fn field_marks_and_indicator() {
        let (f, d) = (setup::FIELD_FRAMES, setup::DELINEATE);
        assert_eq!(
            field_marks(0, true, Some(1), Some(1), Some(0)),
            (false, false)
        );
        assert_eq!(
            field_marks(f, false, Some(1), Some(1), Some(1)),
            (true, false)
        );
        assert_eq!(
            field_marks(f, false, Some(2), Some(1), Some(1)),
            (false, false)
        );
        assert_eq!(field_marks(f, false, None, None, None), (false, false));
        assert_eq!(field_marks(d, true, Some(2), None, Some(1)), (false, true));
        assert_eq!(field_marks(d, true, Some(1), None, Some(1)), (false, false));
        assert_eq!(
            field_marks(d, false, Some(2), None, Some(1)),
            (false, false)
        );
        let mut l = Layout::default();
        assert!(record_indicator(&l));
        l.options |= setup::NO_RECORD_INDICATOR;
        assert!(!record_indicator(&l));
    }

    /// A layout opens in its default view the first time the window shows
    /// it, then in the view it was left in; a script changing the layout
    /// and the view at once keeps its view.
    #[test]
    fn default_view_on_first_visit() {
        let mut app = crate::ui_tests::shop_app();
        let inv = app.layout;
        let d = app.doc.as_mut().unwrap();
        let cl = d.layouts.iter().position(|l| l.name == "Clients").unwrap();
        d.layouts[cl].set_default_view(1);
        app.enforce_view();
        assert_eq!(app.view, View::Form);
        app.layout = cl;
        app.enforce_view();
        assert_eq!(app.view, View::List, "first visit: default view");
        app.view = View::Table;
        app.enforce_view();
        app.layout = inv;
        app.enforce_view();
        assert_eq!(app.view, View::Form);
        app.layout = cl;
        app.enforce_view();
        assert_eq!(app.view, View::Table, "the view it was left in");
        // Layout and view changed together (Go to Layout, View As).
        app.layout = inv;
        app.view = View::List;
        app.enforce_view();
        assert_eq!(app.view, View::List);
    }

    /// "Save record changes automatically" off: leaving a modified record
    /// asks; Don't Save reverts it, Cancel stays in it, Save commits it.
    #[test]
    fn ask_before_saving() {
        let mut app = crate::ui_tests::shop_app();
        let i = app.layout;
        app.doc.as_mut().unwrap().layouts[i].options |= setup::NO_AUTO_SAVE;
        let mut h = harness_for(app);
        h.run_steps(3);
        let value = |h: &egui_kittest::Harness<'_, App>| {
            let db = &h.state().doc.as_ref().unwrap().db;
            let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
            let f = t.field_by_name("ClientId").unwrap().id;
            db.value(t.id, 0, f).to_text()
        };
        let before = value(&h);
        let leave = |h: &mut egui_kittest::Harness<'_, App>, button: &'static str| {
            crate::ui_tests::retype(h, 0, "7");
            h.key_press(egui::Key::Tab);
            h.run_steps(2);
            assert!(!h.state_mut().commit());
            h.run_steps(2);
            h.get_by_label(t(button)).click();
            h.run_steps(3);
        };
        leave(&mut h, "Cancel");
        assert_eq!(value(&h), "7");
        assert!(h.state().editing.is_some(), "Cancel: still modified");
        h.state_mut().revert_record();
        h.run_steps(2);
        leave(&mut h, "Don't Save");
        assert_eq!(value(&h), before);
        assert!(h.state().editing.is_none());
        leave(&mut h, "Save");
        assert_eq!(value(&h), "7");
        assert!(h.state().editing.is_none());
    }
}
