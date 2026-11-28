//! Edit › Preferences… (FileMaker's « Réglages… »): interface language,
//! user name (`Get ( UserName )`), layout units, the light or dark theme,
//! recent files and their number. Kept on desktop in a small text file
//! (`%APPDATA%\BaseCraft\preferences.txt`, `~/.config/basecraft/…`, or
//! `BASECRAFT_CONFIG`); in the browser for the session only.

use std::cell::RefCell;

use eframe::egui;

use crate::{App, i18n};

pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Preferences…" => "Réglages…",
        "Preferences" => "Réglages",
        "General" => "Général",
        "Layout" => "Modèle",
        "User name:" => "Utilisateur :",
        "User interface language:" => "Langue de l'interface utilisateur :",
        "System" => "Système",
        "Same as system" => "Comme le système",
        "Recent files list:" => "Liste des fichiers récents :",
        "Allow drag and drop" => "Autoriser Glisser-déposer",
        "Ruler units:" => "Unités de la règle :",
        "Points" => "Points",
        "Inches" => "Pouces",
        "Centimeters" => "Centimètres",
        "Theme:" => "Thème :",
        "Light" => "Clair",
        "Dark" => "Sombre",
        "OK" => "OK",
        "Cancel" => "Annuler",
        "Open Recent" => "Récent",
        "Clear Recent" => "Effacer la liste",
        _ => key,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    #[default]
    Points,
    Inches,
    Centimeters,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// `None`: the system's language; `Some(true)`: French.
    pub french: Option<bool>,
    /// `Get ( UserName )`; empty: the system account's name.
    pub user: String,
    pub units: Units,
    /// `None`: the system's; `Some(true)`: dark.
    pub dark: Option<bool>,
    pub drag_drop: bool,
    pub recent_max: usize,
    /// Most recent first.
    pub recent: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            french: None,
            user: String::new(),
            units: Units::Points,
            dark: Some(false),
            drag_drop: true,
            recent_max: 10,
            recent: Vec::new(),
        }
    }
}

impl Prefs {
    fn to_text(&self) -> String {
        let mut s = String::new();
        let opt = |b: Option<bool>, yes: &str, no: &str| match b {
            None => "system".to_string(),
            Some(true) => yes.into(),
            Some(false) => no.into(),
        };
        s += &format!("language={}\n", opt(self.french, "fr", "en"));
        s += &format!("user={}\n", self.user);
        let u = match self.units {
            Units::Points => "pt",
            Units::Inches => "in",
            Units::Centimeters => "cm",
        };
        s += &format!("units={u}\n");
        s += &format!("theme={}\n", opt(self.dark, "dark", "light"));
        s += &format!("drag_drop={}\n", self.drag_drop);
        s += &format!("recent_max={}\n", self.recent_max);
        for r in &self.recent {
            s += &format!("recent={r}\n");
        }
        s
    }

    fn from_text(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k {
                "language" => p.french = (v != "system").then_some(v == "fr"),
                "user" => p.user = v.to_string(),
                "units" => {
                    p.units = match v {
                        "in" => Units::Inches,
                        "cm" => Units::Centimeters,
                        _ => Units::Points,
                    }
                }
                "theme" => p.dark = (v != "system").then_some(v == "dark"),
                "drag_drop" => p.drag_drop = v != "false",
                "recent_max" => p.recent_max = v.parse().unwrap_or(10),
                "recent" => p.recent.push(v.to_string()),
                _ => {}
            }
        }
        p
    }
}

thread_local! {
    static PREFS: RefCell<Option<Prefs>> = const { RefCell::new(None) };
}

/// The settings file (none in the tests and the browser).
fn path() -> Option<std::path::PathBuf> {
    if cfg!(test) || cfg!(target_arch = "wasm32") {
        return None;
    }
    if let Some(p) = std::env::var_os("BASECRAFT_CONFIG") {
        return Some(p.into());
    }
    let dir = std::env::var_os("APPDATA")
        .map(|d| std::path::PathBuf::from(d).join("BaseCraft"))
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config"))
                })
                .map(|d| d.join("basecraft"))
        })?;
    Some(dir.join("preferences.txt"))
}

/// The preferences (read from the settings file the first time).
pub fn get() -> Prefs {
    PREFS.with(|p| {
        p.borrow_mut()
            .get_or_insert_with(|| {
                let prefs = path()
                    .and_then(|f| std::fs::read_to_string(f).ok())
                    .map(|s| Prefs::from_text(&s))
                    .unwrap_or_default();
                apply(&prefs);
                prefs
            })
            .clone()
    })
}

/// Changes the preferences: applied at once and written to the settings file.
pub fn set(prefs: Prefs) {
    apply(&prefs);
    if let Some(f) = path() {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, prefs.to_text());
    }
    PREFS.with(|p| *p.borrow_mut() = Some(prefs));
}

fn apply(p: &Prefs) {
    i18n::set_french(p.french);
    let user = p.user.trim();
    calc::functions::set_user_name((!user.is_empty()).then(|| user.to_string()));
}

/// Light or dark visuals, as the preferences say.
pub fn visuals(ctx: &egui::Context) {
    let dark = get()
        .dark
        .unwrap_or_else(|| ctx.system_theme() == Some(egui::Theme::Dark));
    ctx.set_visuals(if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    });
}

/// Layout units: points per unit and the unit's suffix.
pub fn unit() -> (f64, &'static str) {
    match get().units {
        Units::Points => (1.0, " pt"),
        Units::Inches => (72.0, " in"),
        Units::Centimeters => (72.0 / 2.54, " cm"),
    }
}

/// File › Open Recent: a file opened (most recent first).
#[cfg(not(target_arch = "wasm32"))]
pub fn add_recent(p: &std::path::Path) {
    let mut prefs = get();
    let s = p.to_string_lossy().into_owned();
    prefs.recent.retain(|r| *r != s);
    prefs.recent.insert(0, s);
    prefs.recent.truncate(prefs.recent_max);
    set(prefs);
}

impl App {
    /// File › Open Recent.
    pub fn recent_menu(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        ui.menu_button(t("Open Recent"), |ui| {
            let mut prefs = get();
            for r in prefs.recent.clone() {
                let name = std::path::Path::new(&r)
                    .file_name()
                    .map_or(r.clone(), |n| n.to_string_lossy().into_owned());
                if ui.button(name).on_hover_text(&r).clicked() {
                    ui.close();
                    self.open_path(std::path::Path::new(&r));
                }
            }
            if !prefs.recent.is_empty() {
                ui.separator();
            }
            if ui
                .add_enabled(
                    !prefs.recent.is_empty(),
                    egui::Button::new(t("Clear Recent")),
                )
                .clicked()
            {
                ui.close();
                prefs.recent.clear();
                set(prefs);
            }
        });
        #[cfg(target_arch = "wasm32")]
        let _ = ui;
    }

    /// Edit › Preferences….
    pub fn prefs_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(t("Preferences…")).clicked() {
            ui.close();
            self.prefs = Some(get());
        }
    }

    /// The Preferences dialog: OK applies and saves, Cancel drops.
    pub fn prefs_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = self.prefs.as_mut() else { return };
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("preferences")).show(ctx, |ui| {
            ui.set_min_width(440.0);
            ui.heading(t("Preferences"));
            ui.label(egui::RichText::new(t("General")).strong());
            egui::Grid::new("prefs_general")
                .num_columns(2)
                .show(ui, |ui| {
                    let lab = ui.label(t("User name:"));
                    ui.add(egui::TextEdit::singleline(&mut p.user).id_salt("prefs_user"))
                        .labelled_by(lab.id);
                    ui.end_row();
                    ui.label(t("User interface language:"));
                    egui::ComboBox::from_id_salt("prefs_language")
                        .selected_text(match p.french {
                            None => t("Same as system"),
                            Some(true) => "Français",
                            Some(false) => "English",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut p.french, None, t("Same as system"));
                            ui.selectable_value(&mut p.french, Some(false), "English");
                            ui.selectable_value(&mut p.french, Some(true), "Français");
                        });
                    ui.end_row();
                    ui.label(t("Recent files list:"));
                    ui.add(egui::DragValue::new(&mut p.recent_max).range(0..=30));
                    ui.end_row();
                    ui.label(t("Theme:"));
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut p.dark, Some(false), t("Light"));
                        ui.radio_value(&mut p.dark, Some(true), t("Dark"));
                        ui.radio_value(&mut p.dark, None, t("System"));
                    });
                    ui.end_row();
                });
            ui.checkbox(&mut p.drag_drop, t("Allow drag and drop"));
            ui.separator();
            ui.label(egui::RichText::new(t("Layout")).strong());
            ui.horizontal(|ui| {
                ui.label(t("Ruler units:"));
                ui.radio_value(&mut p.units, Units::Points, t("Points"));
                ui.radio_value(&mut p.units, Units::Inches, t("Inches"));
                ui.radio_value(&mut p.units, Units::Centimeters, t("Centimeters"));
            });
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t("OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                if let Some(mut p) = self.prefs.take() {
                    p.recent.truncate(p.recent_max);
                    set(p);
                    visuals(ctx);
                }
            }
            Some(false) => self.prefs = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let p = Prefs {
            french: Some(true),
            user: "Ada".into(),
            units: Units::Centimeters,
            dark: None,
            drag_drop: false,
            recent_max: 3,
            recent: vec!["C:\\a.fmp12".into(), "/b/c.fmp12".into()],
        };
        assert_eq!(Prefs::from_text(&p.to_text()), p);
        assert_eq!(Prefs::from_text(""), Prefs::default());
    }
}
