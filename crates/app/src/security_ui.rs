//! Accounts and privileges in the app: the login dialog a protected file
//! shows when it opens, File › Manage › Security… (accounts and privilege
//! sets, saved with their digests by `fmp12::security::save`), and the
//! alert for actions the account's privileges refuse.
//!
//! Nothing here opens a file, or changes its security, without a login:
//! the file opens only with an account name and password that match, and
//! Manage Security asks again for a full access account before it writes.

use eframe::egui;
use fmp12::security::{
    self, Account, Level, Menus, PrivilegeSet, RecordAccess, Security, TableAccess, flag,
};

use crate::{App, Database, i18n};

/// UI strings: English keys, French as FileMaker 26's French UI writes them.
pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Manage" => "Gérer",
        "Security…" => "Sécurité…",
        "Manage Security" => "Gérer la sécurité",
        "Open" => "Ouvrir",
        "Sign In" => "Connexion",
        "Cancel" => "Annuler",
        "Account Name:" => "Nom du compte :",
        "Password:" => "Mot de passe :",
        "Sign in to open the file" => "Connectez-vous pour ouvrir le fichier",
        "Accounts" => "Comptes",
        "Privilege Sets" => "Jeux de privilèges",
        "New" => "Nouveau",
        "Duplicate" => "Dupliquer",
        "Delete" => "Supprimer",
        "Active" => "Actif",
        "Account" => "Compte",
        "Privilege Set" => "Jeu de privilèges",
        "Name:" => "Nom :",
        "Description:" => "Description :",
        "New password:" => "Nouveau mot de passe :",
        "(unchanged)" => "(inchangé)",
        "User must change password on next sign-in" => {
            "L'utilisateur doit modifier son mot de passe à la prochaine connexion"
        }
        "Records" => "Enregistrements",
        "Other tables" => "Autres tables",
        "Create" => "Créer",
        "View" => "Afficher",
        "Edit" => "Modifier",
        "Fields" => "Rubriques",
        "yes" => "oui",
        "no" => "non",
        "limited" => "limité",
        "Layouts" => "Modèles",
        "Scripts" => "Scripts",
        "Value Lists" => "Listes de valeurs",
        "All layouts" => "Tous les modèles",
        "All scripts" => "Tous les scripts",
        "modifiable" => "modifiable",
        "view only" => "affichage seulement",
        "executable only" => "exécutable seulement",
        "no access" => "pas d'accès",
        "Extended Privileges" => "Privilèges étendus",
        "Other Privileges" => "Autres privilèges",
        "Allow printing" => "Autoriser l'impression",
        "Allow exporting" => "Autoriser l'exportation",
        "Manage extended privileges" => "Gérer les privilèges étendus",
        "Allow user to override data validation warnings" => {
            "Autoriser l'utilisateur à ignorer les avertissements de validation"
        }
        "Allow user to modify their own password" => {
            "Autoriser l'utilisateur à modifier son propre mot de passe"
        }
        "Minimum password length:" => "Longueur minimale du mot de passe :",
        "Available menu commands:" => "Commandes de menu disponibles :",
        "All" => "Toutes",
        "Editing only" => "Édition seulement",
        "Minimum" => "Minimum",
        "Calculation:" => "Calcul :",
        "Built-in privilege sets cannot be changed." => {
            "Les jeux de privilèges prédéfinis ne peuvent pas être modifiés."
        }
        "Confirm Full Access Login" => "Confirmer la connexion avec accès intégral",
        "Confirm new password:" => "Confirmation du mot de passe :",
        "The new passwords do not match." => "Les nouveaux mots de passe ne correspondent pas.",
        "The password for this account must be changed." => {
            "Vous devez modifier le mot de passe de votre compte FileMaker avant d'ouvrir ce fichier. Veuillez entrer votre ancien mot de passe une fois et votre nouveau mot de passe deux fois."
        }
        "Enter a full access account name and password to save these changes." => {
            "Saisissez le nom et le mot de passe d'un compte à accès intégral pour enregistrer ces modifications."
        }
        "OK" => "OK",
        other => i18n::tr(other),
    }
}

/// FileMaker's alert for an account name and password that do not open
/// the file, also shown for an inactive account (French text as FileMaker
/// 26 shows it).
pub fn wrong_password() -> &'static str {
    if i18n::french() {
        "Le nom de compte et le mot de passe fournis ne permettent pas d'accéder à ce fichier."
    } else {
        security::LoginError::Invalid.message()
    }
}

/// "Your access privileges do not allow you to perform this action."
pub fn denied() -> &'static str {
    if i18n::french() {
        "Vos privilèges d'accès ne vous permettent pas d'effectuer cette action."
    } else {
        engine::DENIED_MESSAGE
    }
}

/// A protected file waiting for its login dialog.
pub struct Login {
    pub name: String,
    pub file: fmp12::File,
    pub path: Option<std::path::PathBuf>,
    pub account: String,
    pub password: String,
    /// The account must change its password: the new one, typed twice.
    pub change: Option<(String, String)>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Accounts,
    Sets,
}

/// The Manage Security dialog: a working copy of the file's security.
pub struct Manage {
    sec: Security,
    tab: Tab,
    account: Option<u32>,
    set: Option<u32>,
    /// New password typed for an account (account id → text).
    passwords: std::collections::BTreeMap<u32, String>,
    /// Calculations of limited record actions being typed: (set, table, action).
    limits: std::collections::BTreeMap<(u32, u32, usize), String>,
    /// The full access login asked for on OK.
    confirm: Option<(String, String)>,
    error: Option<String>,
}

#[derive(Default)]
pub struct State {
    pub login: Option<Login>,
    manage: Option<Manage>,
}

fn access_label(v: u8, script: bool) -> &'static str {
    match v {
        security::MODIFIABLE => t("modifiable"),
        security::VIEW_ONLY if script => t("executable only"),
        security::VIEW_ONLY => t("view only"),
        _ => t("no access"),
    }
}

/// A combo box over modifiable / view only (executable) / no access.
fn access_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    v: &mut u8,
    script: bool,
) -> bool {
    let before = *v;
    egui::ComboBox::from_id_salt(id)
        .selected_text(access_label(*v, script))
        .show_ui(ui, |ui| {
            for x in [
                security::MODIFIABLE,
                security::VIEW_ONLY,
                security::NO_ACCESS,
            ] {
                ui.selectable_value(v, x, access_label(x, script));
            }
        });
    *v != before
}

fn level_label(l: Level) -> &'static str {
    match l {
        Level::Yes => t("yes"),
        Level::No => t("no"),
        Level::Limited => t("limited"),
    }
}

impl App {
    /// Opens a file that needs a login: keeps it for the login dialog.
    pub(crate) fn ask_login(&mut self, name: String, file: fmp12::File) {
        self.security.login = Some(Login {
            name,
            file,
            path: None,
            account: String::new(),
            password: String::new(),
            change: None,
            error: None,
        });
    }

    /// The login dialog ("Open <file>"): FileMaker's account name and
    /// password check; the file opens only with a matching pair.
    fn login_ui(&mut self, ctx: &egui::Context) {
        let Some(l) = self.security.login.as_mut() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::NULL.with("login")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(format!("{} « {} »", t("Open"), l.name));
            ui.label(t("Sign in to open the file"));
            egui::Grid::new("login_grid").num_columns(2).show(ui, |ui| {
                let lab = ui.label(t("Account Name:"));
                ui.add(egui::TextEdit::singleline(&mut l.account).id_salt("login_account"))
                    .labelled_by(lab.id);
                ui.end_row();
                let lab = ui.label(t("Password:"));
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut l.password)
                            .password(true)
                            .id_salt("login_password"),
                    )
                    .labelled_by(lab.id);
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.end_row();
                if let Some((new, again)) = l.change.as_mut() {
                    for (label, v, id) in [
                        ("New password:", new, "login_new"),
                        ("Confirm new password:", again, "login_again"),
                    ] {
                        let lab = ui.label(t(label));
                        ui.add(egui::TextEdit::singleline(v).password(true).id_salt(id))
                            .labelled_by(lab.id);
                        ui.end_row();
                    }
                }
            });
            if let Some(e) = &l.error {
                ui.colored_label(egui::Color32::RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    cancel = true;
                }
                if ui.button(t("Sign In")).clicked() {
                    submit = true;
                }
            });
        });
        if cancel {
            self.security.login = None;
            return;
        }
        if !submit {
            return;
        }
        let r = match &l.change {
            Some((new, again)) if new != again => {
                l.error = Some(t("The new passwords do not match.").to_string());
                return;
            }
            Some((new, _)) => {
                Database::load_changing_password(&l.file, &l.account, &l.password, new)
            }
            None => Database::load_as(&l.file, &l.account, &l.password),
        };
        match r {
            Ok(db) => {
                let Some(l) = self.security.login.take() else {
                    return;
                };
                self.opened(l.name, db, l.file);
                if let Some(d) = self.doc.as_mut() {
                    d.path = l.path;
                }
            }
            // FileMaker asks for a new password, then opens the file.
            Err(fmp12::Error::Login(security::LoginError::MustChange)) if l.change.is_none() => {
                l.change = Some(Default::default());
                l.error = Some(t("The password for this account must be changed.").to_string());
            }
            // Wrong name or password, or an inactive account: one alert.
            Err(fmp12::Error::Login(_)) => {
                l.error = Some(wrong_password().to_string());
                l.password.clear();
                l.change = None;
            }
            Err(fmp12::Error::Security(e)) => l.error = Some(e),
            Err(e) => {
                self.error = Some(e.to_string());
                self.security.login = None;
            }
        }
    }

    /// File › Manage › Security…: only for a full access account.
    pub(crate) fn open_security(&mut self) {
        if self.remote_refuses(t("Manage Security")) {
            return;
        }
        let Some(d) = self.doc.as_ref() else { return };
        if !d.db.full_access() {
            self.messages.push((String::new(), denied().to_string()));
            return;
        }
        match Security::load(&d.file) {
            Ok(sec) => {
                self.security.manage = Some(Manage {
                    account: sec.accounts.first().map(|a| a.id),
                    set: sec.sets.first().map(|s| s.id),
                    sec,
                    tab: Tab::Accounts,
                    passwords: Default::default(),
                    limits: Default::default(),
                    confirm: None,
                    error: None,
                })
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The login dialog, Manage Security, and the alert for an action the
    /// privileges refused (once per refusal).
    pub(crate) fn security_dialogs(&mut self, ctx: &egui::Context) {
        self.login_ui(ctx);
        if let Some(d) = self.doc.as_mut()
            && d.db.last_denied().is_some()
        {
            self.messages.push((String::new(), denied().to_string()));
        }
        self.manage_ui(ctx);
    }

    fn manage_ui(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.security.manage.take() else {
            return;
        };
        let Some(d) = self.doc.as_ref() else { return };
        let names = fmp12::calc::FileNames::load(&d.file).unwrap_or_default();
        let mut close = None;
        egui::Modal::new(egui::Id::NULL.with("manage_security")).show(ctx, |ui| {
            ui.set_min_width(760.0);
            ui.heading(t("Manage Security"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut m.tab, Tab::Accounts, t("Accounts"));
                ui.selectable_value(&mut m.tab, Tab::Sets, t("Privilege Sets"));
            });
            ui.separator();
            match m.tab {
                Tab::Accounts => accounts_tab(ui, &mut m),
                Tab::Sets => sets_tab(ui, &mut m, d),
            }
            ui.separator();
            if let Some((acc, pw)) = m.confirm.as_mut() {
                ui.strong(t("Confirm Full Access Login"));
                ui.label(t(
                    "Enter a full access account name and password to save these changes.",
                ));
                egui::Grid::new("confirm_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(t("Account Name:"));
                        ui.add(egui::TextEdit::singleline(acc).id_salt("confirm_account"));
                        ui.end_row();
                        ui.label(t("Password:"));
                        ui.add(
                            egui::TextEdit::singleline(pw)
                                .password(true)
                                .id_salt("confirm_password"),
                        );
                        ui.end_row();
                    });
            }
            if let Some(e) = &m.error {
                ui.colored_label(egui::Color32::RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    close = Some(false);
                }
                if ui.button(t("OK")).clicked() {
                    close = Some(true);
                }
            });
        });
        match close {
            None => self.security.manage = Some(m),
            Some(false) => {}
            Some(true) => {
                if let Err(e) = self.apply_security(&mut m, &names) {
                    m.error = (!e.is_empty()).then_some(e);
                    self.security.manage = Some(m);
                }
            }
        }
    }

    /// OK in Manage Security: asks for a full access login (the first OK
    /// shows the fields), then writes the accounts and privilege sets.
    fn apply_security(
        &mut self,
        m: &mut Manage,
        names: &fmp12::calc::FileNames,
    ) -> Result<(), String> {
        let Some((acc, pw)) = m.confirm.clone() else {
            m.confirm = Some((
                self.doc
                    .as_ref()
                    .and_then(|d| d.db.session())
                    .map(|s| s.account_name().to_string())
                    .unwrap_or_default(),
                String::new(),
            ));
            // No message: the dialog stays open on the login fields.
            return Err(String::new());
        };
        let Some(d) = self.doc.as_mut() else {
            return Ok(());
        };
        let old = Security::load(&d.file).map_err(|e| e.to_string())?;
        let session = old.login(&acc, &pw).map_err(|e| match e {
            security::LoginError::Invalid => wrong_password().to_string(),
            e => e.to_string(),
        })?;
        if !session.full_access() {
            return Err(denied().to_string());
        }
        let mut new = m.sec.clone();
        for (id, p) in &m.passwords {
            if let Some(a) = new.accounts.iter_mut().find(|a| a.id == *id)
                && !p.is_empty()
            {
                a.set_password(p);
            }
        }
        for ((set, table, action), src) in &m.limits {
            let to = names
                .occurrences
                .iter()
                .find(|o| o.base_table == *table)
                .map_or(0, |o| o.id);
            let l = security::limit(src, to, names).map_err(|e| format!("{src}: {e}"))?;
            if let Some(ta) = new
                .sets
                .iter_mut()
                .find(|s| s.id == *set)
                .and_then(|s| s.tables.get_mut(table))
            {
                ta.limits[*action] = Some(l);
            }
        }
        let bytes = security::save(&d.file, &session, &new).map_err(|e| e.to_string())?;
        let file = fmp12::File::from_bytes(bytes).map_err(|e| e.to_string())?;
        d.db.refresh_security(&file).map_err(|e| e.to_string())?;
        d.file = file;
        self.dirty = true;
        Ok(())
    }
}

fn accounts_tab(ui: &mut egui::Ui, m: &mut Manage) {
    let set_name =
        |sec: &Security, id: u32| sec.set(id).map(|s| s.name.clone()).unwrap_or_default();
    ui.horizontal(|ui| {
        if ui.button(t("New")).clicked() {
            let n = (1..)
                .map(|i| format!("{} {i}", t("Account")))
                .find(|n| m.sec.account_named(n).is_none())
                .unwrap_or_default();
            let mut a = Account::new(&n, "", 2);
            a.must_change = true;
            m.account = Some(m.sec.add_account(a));
        }
        let can_delete = m.account.is_some_and(|id| id != 1);
        if ui
            .add_enabled(can_delete, egui::Button::new(t("Delete")))
            .clicked()
            && let Some(id) = m.account.take()
        {
            m.sec.accounts.retain(|a| a.id != id);
            m.passwords.remove(&id);
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("accounts_list")
        .max_height(180.0)
        .show(ui, |ui| {
            egui::Grid::new("accounts_grid")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong(t("Active"));
                    ui.strong(t("Account"));
                    ui.strong(t("Privilege Set"));
                    ui.end_row();
                    let rows: Vec<(u32, String, bool, String)> = m
                        .sec
                        .accounts
                        .iter()
                        .map(|a| {
                            (
                                a.id,
                                a.name.clone(),
                                a.active,
                                set_name(&m.sec, a.privilege_set),
                            )
                        })
                        .collect();
                    for (id, name, mut active, set) in rows {
                        if ui.checkbox(&mut active, "").changed()
                            && let Some(a) = m.sec.accounts.iter_mut().find(|a| a.id == id)
                        {
                            a.active = active;
                        }
                        if ui.selectable_label(m.account == Some(id), name).clicked() {
                            m.account = Some(id);
                        }
                        ui.label(set);
                        ui.end_row();
                    }
                });
        });
    ui.separator();
    let sets: Vec<(u32, String)> = m.sec.sets.iter().map(|s| (s.id, s.name.clone())).collect();
    let Some(id) = m.account else { return };
    let pw = m.passwords.entry(id).or_default();
    let Some(a) = m.sec.accounts.iter_mut().find(|a| a.id == id) else {
        return;
    };
    egui::Grid::new("account_edit")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(t("Name:"));
            ui.add_enabled(
                !a.is_guest(),
                egui::TextEdit::singleline(&mut a.name).id_salt("acc_name"),
            );
            ui.end_row();
            if !a.is_guest() {
                ui.label(t("New password:"));
                ui.add(
                    egui::TextEdit::singleline(pw)
                        .password(true)
                        .hint_text(t("(unchanged)"))
                        .id_salt("acc_password"),
                );
                ui.end_row();
            }
            ui.label(t("Privilege Set"));
            egui::ComboBox::from_id_salt("acc_set")
                .selected_text(
                    sets.iter()
                        .find(|s| s.0 == a.privilege_set)
                        .map_or("", |s| s.1.as_str()),
                )
                .show_ui(ui, |ui| {
                    for (sid, n) in &sets {
                        ui.selectable_value(&mut a.privilege_set, *sid, n);
                    }
                });
            ui.end_row();
            ui.label(t("Description:"));
            ui.add(egui::TextEdit::singleline(&mut a.description).id_salt("acc_desc"));
            ui.end_row();
        });
    ui.checkbox(&mut a.active, t("Active"));
    if !a.is_guest() {
        ui.checkbox(
            &mut a.must_change,
            t("User must change password on next sign-in"),
        );
    }
}

fn sets_tab(ui: &mut egui::Ui, m: &mut Manage, d: &crate::Doc) {
    ui.horizontal(|ui| {
        if ui.button(t("New")).clicked() {
            let n = (1..)
                .map(|i| format!("{} {i}", t("Privilege Set")))
                .find(|n| m.sec.set_named(n).is_none())
                .unwrap_or_default();
            let mut s = PrivilegeSet {
                name: n,
                records: Some(RecordAccess::NONE),
                ..Default::default()
            };
            s.set_menus(Menus::Minimum);
            m.set = Some(m.sec.add_set(s));
        }
        if ui
            .add_enabled(m.set.is_some(), egui::Button::new(t("Duplicate")))
            .clicked()
            && let Some(mut s) = m.set.and_then(|id| m.sec.set(id)).cloned()
        {
            s.name = format!("{} 2", s.name);
            // A copy of a built-in set gets its privileges spelled out.
            if s.full_access() {
                s.records = Some(RecordAccess::ALL);
                s.flags = u16::MAX;
            }
            m.set = Some(m.sec.add_set(s));
        }
        let used = |id: u32| m.sec.accounts.iter().any(|a| a.privilege_set == id);
        let can_delete = m.set.is_some_and(|id| id > 3 && !used(id));
        if ui
            .add_enabled(can_delete, egui::Button::new(t("Delete")))
            .clicked()
            && let Some(id) = m.set.take()
        {
            m.sec.sets.retain(|s| s.id != id);
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("sets_list")
        .max_height(120.0)
        .show(ui, |ui| {
            for s in &m.sec.sets {
                if ui.selectable_label(m.set == Some(s.id), &s.name).clicked() {
                    m.set = Some(s.id);
                }
            }
        });
    ui.separator();
    let Some(id) = m.set else { return };
    if id <= 3 {
        ui.label(t("Built-in privilege sets cannot be changed."));
        return;
    }
    let extended = &mut m.sec.extended;
    let Some(s) = m.sec.sets.iter_mut().find(|s| s.id == id) else {
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("set_edit")
        .max_height(420.0)
        .show(ui, |ui| {
            egui::Grid::new("set_head").num_columns(2).show(ui, |ui| {
                ui.label(t("Name:"));
                ui.add(egui::TextEdit::singleline(&mut s.name).id_salt("set_name"));
                ui.end_row();
                ui.label(t("Description:"));
                ui.add(egui::TextEdit::singleline(&mut s.description).id_salt("set_desc"));
                ui.end_row();
            });
            ui.collapsing(t("Records"), |ui| records_grid(ui, s, d, &mut m.limits));
            ui.collapsing(t("Layouts"), |ui| {
                items_grid(
                    ui,
                    s,
                    security::LAYOUTS,
                    t("All layouts"),
                    false,
                    d.db.layouts.iter().map(|l| (l.id, l.name.clone())),
                );
            });
            ui.collapsing(t("Scripts"), |ui| {
                let scripts: Vec<(u32, String)> = fmp12::scripts::scripts(&d.file)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| !s.is_folder() && !s.is_separator())
                    .map(|s| (s.id, s.name))
                    .collect();
                items_grid(
                    ui,
                    s,
                    security::SCRIPTS,
                    t("All scripts"),
                    true,
                    scripts.into_iter(),
                );
            });
            ui.collapsing(t("Value Lists"), |ui| {
                let mut v = s.item_access(security::VALUE_LISTS, None);
                if access_combo(ui, "vl_all", &mut v, false) {
                    s.set_item_access(security::VALUE_LISTS, None, v);
                }
            });
            ui.collapsing(t("Extended Privileges"), |ui| {
                for e in extended.iter_mut() {
                    let mut on = e.sets.contains(&id);
                    if ui
                        .checkbox(&mut on, format!("{} ({})", e.description, e.keyword))
                        .changed()
                    {
                        if on {
                            e.sets.insert(id);
                        } else {
                            e.sets.remove(&id);
                        }
                    }
                }
            });
            ui.collapsing(t("Other Privileges"), |ui| {
                for (f, label) in [
                    (flag::PRINT, "Allow printing"),
                    (flag::EXPORT, "Allow exporting"),
                    (flag::MANAGE_EXTENDED, "Manage extended privileges"),
                    (
                        flag::OVERRIDE_VALIDATION,
                        "Allow user to override data validation warnings",
                    ),
                    (
                        flag::CHANGE_PASSWORD,
                        "Allow user to modify their own password",
                    ),
                ] {
                    let mut on = s.flags & f != 0;
                    if ui.checkbox(&mut on, t(label)).changed() {
                        s.set_flag(f, on);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(t("Minimum password length:"));
                    ui.add(egui::DragValue::new(&mut s.min_password).range(0..=255));
                });
                ui.horizontal(|ui| {
                    ui.label(t("Available menu commands:"));
                    let mut menus = s.menus();
                    let label = |m: Menus| match m {
                        Menus::All => t("All"),
                        Menus::EditingOnly => t("Editing only"),
                        Menus::Minimum => t("Minimum"),
                    };
                    egui::ComboBox::from_id_salt("set_menus")
                        .selected_text(label(menus))
                        .show_ui(ui, |ui| {
                            for x in [Menus::All, Menus::EditingOnly, Menus::Minimum] {
                                ui.selectable_value(&mut menus, x, label(x));
                            }
                        });
                    if menus != s.menus() {
                        s.set_menus(menus);
                    }
                });
            });
        });
}

/// Record privileges per table: create, view, edit, delete (yes, no, or
/// limited by a calculation) and the access of the fields.
fn records_grid(
    ui: &mut egui::Ui,
    s: &mut PrivilegeSet,
    d: &crate::Doc,
    limits: &mut std::collections::BTreeMap<(u32, u32, usize), String>,
) {
    let names = fmp12::calc::FileNames::load(&d.file).unwrap_or_default();
    egui::Grid::new("records_grid")
        .striped(true)
        .show(ui, |ui| {
            ui.strong("");
            for h in ["Create", "View", "Edit", "Delete", "Fields"] {
                ui.strong(t(h));
            }
            ui.end_row();
            let tables: Vec<(u32, String)> =
                d.db.tables.iter().map(|x| (x.id, x.name.clone())).collect();
            for (tid, tname) in tables {
                ui.label(&tname);
                let mut ta = s.tables.get(&tid).cloned().unwrap_or_else(|| TableAccess {
                    access: s.records.unwrap_or(RecordAccess::NONE),
                    ..Default::default()
                });
                let mut changed = false;
                let mut lv = [
                    ta.access.create,
                    ta.access.view,
                    ta.access.edit,
                    ta.access.delete,
                ];
                for (i, l) in lv.iter_mut().enumerate() {
                    let before = *l;
                    egui::ComboBox::from_id_salt(("rec", tid, i))
                        .width(70.0)
                        .selected_text(level_label(*l))
                        .show_ui(ui, |ui| {
                            for x in [Level::Yes, Level::No, Level::Limited] {
                                // Create has no calculation in FileMaker.
                                if x == Level::Limited && i == 0 {
                                    continue;
                                }
                                ui.selectable_value(l, x, level_label(x));
                            }
                        });
                    changed |= *l != before;
                }
                let mut fa = ta.access.fields;
                changed |= access_combo(ui, ("fields", tid), &mut fa, false);
                ui.end_row();
                // The calculations of limited actions.
                for (i, l) in lv.iter().enumerate() {
                    if *l != Level::Limited {
                        continue;
                    }
                    let key = (s.id, tid, i);
                    let text = limits.entry(key).or_insert_with(|| {
                        ta.limits[i]
                            .as_ref()
                            .and_then(|x| {
                                let to = fmp12::calc::context(&x.context).unwrap_or(0);
                                fmp12::calc::decompile(&x.tokens, to, &names).ok()
                            })
                            .unwrap_or_default()
                    });
                    ui.label("");
                    ui.label(format!(
                        "{} — {}",
                        t(["Create", "View", "Edit", "Delete"][i]),
                        t("Calculation:")
                    ));
                    ui.add(egui::TextEdit::singleline(text).id_salt(("limit", tid, i)));
                    ui.end_row();
                }
                if changed {
                    ta.access = RecordAccess {
                        create: lv[0],
                        view: lv[1],
                        edit: lv[2],
                        delete: lv[3],
                        fields: fa,
                    };
                    s.tables.insert(tid, ta);
                }
            }
        });
}

/// Default access of layouts or scripts, and the items that have their own.
fn items_grid(
    ui: &mut egui::Ui,
    s: &mut PrivilegeSet,
    cat: u8,
    all: &str,
    script: bool,
    items: impl Iterator<Item = (u32, String)>,
) {
    egui::Grid::new(("items", cat))
        .striped(true)
        .show(ui, |ui| {
            ui.label(all);
            let mut v = s.item_access(cat, None);
            if access_combo(ui, ("all", cat), &mut v, script) {
                s.set_item_access(cat, None, v);
            }
            ui.end_row();
            for (id, name) in items {
                ui.label(name);
                let mut v = s.item_access(cat, Some(id));
                if access_combo(ui, (cat, id), &mut v, script) {
                    s.set_item_access(cat, Some(id), v);
                }
                ui.end_row();
            }
        });
}
