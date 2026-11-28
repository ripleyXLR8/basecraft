//! File › Open Remote…: a file hosted by FileMaker Server, through its
//! Data API (`crates/remote`). Its layouts and records are loaded into the
//! engine, so Browse, Find, sort, List and Table views work as on a local
//! file; committed changes (edits, new and deleted records) go back to the
//! server at once, with each record's modification id.
//!
//! What a Data API client cannot do is refused with a message: schema
//! editing, Layout mode, Manage Security and the file's own scripts (only
//! "Perform Script on Server" runs them, on the server). Not in the web
//! build (no sockets there).
//!
//! The password lives in the dialog only until the login; neither it nor
//! the session token is logged or kept on disk.

use eframe::egui;

use crate::{App, i18n};

/// UI strings: English keys, French as FileMaker 26's French UI writes them.
fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Open Remote…" => "Ouvrir à distance…",
        "Open Remote File" => "Ouvrir un fichier distant",
        "Host (https://…):" => "Hôte (https://…) :",
        "Database:" => "Base de données :",
        "Account Name:" => "Nom du compte :",
        "Password:" => "Mot de passe :",
        "Accept a self-signed certificate" => "Accepter un certificat auto-signé",
        "Open" => "Ouvrir",
        "Cancel" => "Annuler",
        "Hosted on" => "Hébergé sur",
        "Perform Script on Server…" => "Exécuter le script sur le serveur…",
        "Refresh from Server" => "Actualiser depuis le serveur",
        "Script:" => "Script :",
        "Parameter:" => "Paramètre :",
        "Perform" => "Exécuter",
        "Close" => "Fermer",
        "Result:" => "Résultat :",
        "Error:" => "Erreur :",
        "Changes not saved on the server" => "Modifications non enregistrées sur le serveur",
        "certificate not checked" => "certificat non vérifié",
        "not available for a file opened through the Data API" => {
            "n'est pas disponible pour un fichier ouvert par la Data API"
        }
        "only the first records were loaded" => {
            "seuls les premiers enregistrements ont été chargés"
        }
        "Opening" => "Ouverture de",
        "records loaded" => "enregistrements chargés",
        _ => key,
    }
}

/// The Open Remote dialog.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Dialog {
    host: String,
    database: String,
    account: String,
    password: String,
    insecure: bool,
    error: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
/// Perform Script on Server: script list, chosen script, parameter, result.
#[derive(Default)]
struct RunDialog {
    scripts: Vec<String>,
    script: String,
    param: String,
    result: Option<String>,
}

/// Login and loading running on another thread, with their progress.
#[cfg(not(target_arch = "wasm32"))]
struct Opening {
    rx: std::sync::mpsc::Receiver<remote::Result<(remote::source::Remote, engine::Database)>>,
    /// Records loaded so far.
    loaded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    database: String,
    insecure: bool,
}

#[derive(Default)]
pub(crate) struct State {
    #[cfg(not(target_arch = "wasm32"))]
    dialog: Option<Dialog>,
    #[cfg(not(target_arch = "wasm32"))]
    opening: Option<Opening>,
    #[cfg(not(target_arch = "wasm32"))]
    run: Option<RunDialog>,
    #[cfg(not(target_arch = "wasm32"))]
    session: Option<Session>,
}

impl State {
    #[cfg(not(target_arch = "wasm32"))]
    fn hosted(&self) -> bool {
        self.session.is_some()
    }

    #[cfg(target_arch = "wasm32")]
    fn hosted(&self) -> bool {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Session {
    remote: remote::source::Remote,
    insecure: bool,
    /// Data as last sent to (or loaded from) the server.
    synced: u64,
    /// Errors of the last sync, shown until the next one.
    errors: Vec<String>,
    /// Records whose containers were downloaded (table, record).
    fetched: std::collections::BTreeSet<(u32, usize)>,
}

impl App {
    /// Whether the open file is hosted (opened through the Data API).
    pub(crate) fn hosted(&self) -> bool {
        self.remote.hosted()
    }

    /// Refuses `what` for a hosted file, with a message. True if refused.
    pub(crate) fn remote_refuses(&mut self, what: &str) -> bool {
        if !self.hosted() {
            return false;
        }
        self.messages.push((
            String::new(),
            format!(
                "{what}: {}.",
                t("not available for a file opened through the Data API")
            ),
        ));
        true
    }

    /// File menu entries: Open Remote…, and for a hosted file Perform
    /// Script on Server… and Refresh from Server.
    pub(crate) fn remote_menu(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if ui.button(t("Open Remote…")).clicked() {
                ui.close();
                self.remote.dialog = Some(Dialog {
                    host: "https://".into(),
                    ..Default::default()
                });
            }
            if self.hosted() {
                if ui.button(t("Perform Script on Server…")).clicked() {
                    ui.close();
                    self.open_run_dialog();
                }
                if ui.button(t("Refresh from Server")).clicked() {
                    ui.close();
                    self.remote_refresh();
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ui;
    }

    /// Saving a hosted file: commit the record and send the changes.
    /// True when the file is hosted (nothing is written locally).
    pub(crate) fn remote_save(&mut self) -> bool {
        if !self.hosted() {
            return false;
        }
        if self.commit() {
            #[cfg(not(target_arch = "wasm32"))]
            self.remote_sync();
        }
        true
    }

    /// Each frame: the dialogs, the "Hosted on" bar, and changes committed
    /// since the last frame sent to the server.
    pub(crate) fn remote_frame(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ctx = ui.ctx().clone();
            self.open_remote_ui(&ctx);
            self.run_dialog_ui(&ctx);
            if self.editing.is_none() {
                self.remote_sync();
            }
            self.fetch_containers();
            if let Some(s) = &self.remote.session {
                let host = s.remote.client.target.host.clone();
                let insecure = s.insecure;
                let errors = s.errors.clone();
                let truncated = s.remote.layouts.iter().any(|l| l.truncated);
                egui::Panel::bottom("remote_status").show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} {host}", t("Hosted on")));
                        if insecure {
                            ui.colored_label(
                                egui::Color32::from_rgb(190, 90, 0),
                                t("certificate not checked"),
                            )
                            .on_hover_text(remote::INSECURE_WARNING);
                        }
                        if truncated {
                            ui.label(format!("({})", t("only the first records were loaded")));
                        }
                    });
                    for e in &errors {
                        ui.colored_label(
                            egui::Color32::RED,
                            format!("{}: {e}", t("Changes not saved on the server")),
                        );
                    }
                });
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ui;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl App {
    fn open_remote_ui(&mut self, ctx: &egui::Context) {
        if self.remote.opening.is_some() {
            return self.opening_ui(ctx);
        }
        let Some(d) = self.remote.dialog.as_mut() else {
            return;
        };
        let (mut submit, mut cancel) = (false, false);
        egui::Modal::new(egui::Id::NULL.with("open_remote")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(t("Open Remote File"));
            egui::Grid::new("remote_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    for (label, v, id) in [
                        ("Host (https://…):", &mut d.host, "remote_host"),
                        ("Database:", &mut d.database, "remote_db"),
                        ("Account Name:", &mut d.account, "remote_account"),
                    ] {
                        let lab = ui.label(t(label));
                        ui.add(egui::TextEdit::singleline(v).id_salt(id))
                            .labelled_by(lab.id);
                        ui.end_row();
                    }
                    let lab = ui.label(t("Password:"));
                    let r = ui
                        .add(
                            egui::TextEdit::singleline(&mut d.password)
                                .password(true)
                                .id_salt("remote_password"),
                        )
                        .labelled_by(lab.id);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    ui.end_row();
                });
            ui.checkbox(&mut d.insecure, t("Accept a self-signed certificate"));
            if d.insecure {
                ui.colored_label(
                    egui::Color32::from_rgb(190, 90, 0),
                    remote::INSECURE_WARNING,
                );
            }
            if let Some(e) = &d.error {
                ui.colored_label(egui::Color32::RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    cancel = true;
                }
                if ui.button(t("Open")).clicked() {
                    submit = true;
                }
            });
        });
        if cancel {
            self.remote.dialog = None;
            return;
        }
        if !submit {
            return;
        }
        // Login and loading run on another thread: the window stays
        // responsive and shows the records loaded so far.
        let target = remote::Target {
            host: d.host.trim().trim_end_matches('/').to_string(),
            database: d.database.trim().to_string(),
        };
        let options = remote::Options {
            insecure: d.insecure,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let loaded = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = loaded.clone();
        let account = d.account.trim().to_string();
        let password = std::mem::take(&mut d.password);
        let database = target.database.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let r = remote::Client::login(&target, &account, &password, options)
                .and_then(|c| remote::source::Remote::open_counting(c, &count));
            let _ = tx.send(r);
            repaint.request_repaint();
        });
        self.remote.opening = Some(Opening {
            rx,
            loaded,
            database,
            insecure: options.insecure,
        });
    }

    /// While the file opens: a spinner, the records loaded, Cancel; then
    /// the file, or the error in the dialog.
    fn opening_ui(&mut self, ctx: &egui::Context) {
        let Some(op) = self.remote.opening.as_ref() else {
            return;
        };
        let result = match op.rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                let n = op.loaded.load(std::sync::atomic::Ordering::Relaxed);
                let mut cancel = false;
                egui::Modal::new(egui::Id::NULL.with("remote_opening")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("{} « {} »…", t("Opening"), op.database));
                    });
                    ui.label(format!("{n} {}", t("records loaded")));
                    if ui.button(t("Cancel")).clicked() {
                        cancel = true;
                    }
                });
                if cancel {
                    // The thread finishes alone; its result is dropped.
                    self.remote.opening = None;
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(remote::Error::Protocol("the login thread stopped".into()))
            }
        };
        let Some(op) = self.remote.opening.take() else {
            return;
        };
        match result {
            Ok((remote, db)) => {
                let file = match fmp12::schema_edit::new_file("Remote")
                    .map_err(|e| e.to_string())
                    .and_then(|b| fmp12::File::from_bytes(b).map_err(|e| e.to_string()))
                {
                    Ok(f) => f,
                    Err(e) => {
                        if let Some(d) = self.remote.dialog.as_mut() {
                            d.error = Some(e);
                        }
                        return;
                    }
                };
                let synced = crate::data_fingerprint(&db);
                self.opened(op.database, db, file);
                // A hosted file's scripts run on the server only.
                self.engine = None;
                self.remote.session = Some(Session {
                    remote,
                    insecure: op.insecure,
                    synced,
                    errors: Vec::new(),
                    fetched: Default::default(),
                });
            }
            Err(e) => {
                if let Some(d) = self.remote.dialog.as_mut() {
                    d.error = Some(e.to_string());
                }
            }
        }
    }

    /// The current record's containers, downloaded once when it is shown.
    fn fetch_containers(&mut self) {
        let (Some(t), Some(r)) = (self.table(), self.current_record()) else {
            return;
        };
        let (Some(s), Some(d)) = (self.remote.session.as_mut(), self.doc.as_mut()) else {
            return;
        };
        if !s.fetched.insert((t, r)) {
            return;
        }
        match s.remote.fetch_containers(&mut d.db, t, r) {
            Ok(0) => {}
            Ok(_) => s.synced = crate::data_fingerprint(&d.db),
            Err(e) => s.errors = vec![e.to_string()],
        }
    }

    /// Sends committed changes when the data differs from the server's.
    fn remote_sync(&mut self) {
        let (Some(s), Some(d)) = (self.remote.session.as_mut(), self.doc.as_ref()) else {
            return;
        };
        let now = crate::data_fingerprint(&d.db);
        if now == s.synced {
            return;
        }
        s.errors = s
            .remote
            .sync(&d.db)
            .into_iter()
            .map(|e| e.to_string())
            .collect();
        s.synced = now;
        self.dirty = false;
    }

    /// Reloads the current layout's records (other users' changes, or
    /// after a conflict).
    fn remote_refresh(&mut self) {
        if !self.commit() {
            return;
        }
        self.remote_sync();
        let Some(t) = self.table() else { return };
        let (Some(s), Some(d)) = (self.remote.session.as_mut(), self.doc.as_mut()) else {
            return;
        };
        match s.remote.refresh(&mut d.db, t) {
            Ok(()) => s.errors.clear(),
            Err(e) => s.errors = vec![e.to_string()],
        }
        s.synced = crate::data_fingerprint(&d.db);
        s.fetched.clear();
        self.found.remove(&t);
        self.sorted.remove(&t);
        self.current.insert(t, 0);
    }

    fn open_run_dialog(&mut self) {
        let Some(s) = self.remote.session.as_ref() else {
            return;
        };
        match s.remote.client.scripts() {
            Ok(scripts) => {
                self.remote.run = Some(RunDialog {
                    script: scripts.first().cloned().unwrap_or_default(),
                    scripts,
                    ..Default::default()
                })
            }
            Err(e) => self.messages.push((String::new(), e.to_string())),
        }
    }

    /// Perform Script on Server: runs in the context of the current
    /// layout; shows the script's result or error, then reloads the
    /// layout's records (the script may have changed them).
    fn run_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(r) = self.remote.run.as_mut() else {
            return;
        };
        let (mut perform, mut close) = (false, false);
        egui::Modal::new(egui::Id::NULL.with("remote_run")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(t("Perform Script on Server…").trim_end_matches('…'));
            egui::Grid::new("remote_run_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(t("Script:"));
                    egui::ComboBox::from_id_salt("remote_script")
                        .selected_text(r.script.clone())
                        .show_ui(ui, |ui| {
                            for s in &r.scripts {
                                ui.selectable_value(&mut r.script, s.clone(), s);
                            }
                        });
                    ui.end_row();
                    let lab = ui.label(t("Parameter:"));
                    ui.add(egui::TextEdit::singleline(&mut r.param).id_salt("remote_param"))
                        .labelled_by(lab.id);
                    ui.end_row();
                });
            if let Some(res) = &r.result {
                ui.label(res);
            }
            ui.horizontal(|ui| {
                if ui.button(t("Close")).clicked() {
                    close = true;
                }
                if ui
                    .add_enabled(!r.script.is_empty(), egui::Button::new(t("Perform")))
                    .clicked()
                {
                    perform = true;
                }
            });
        });
        if close {
            self.remote.run = None;
            return;
        }
        if !perform {
            return;
        }
        let layout = self
            .doc
            .as_ref()
            .and_then(|d| d.layouts.get(self.layout))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let (Some(s), Some(r)) = (self.remote.session.as_ref(), self.remote.run.as_mut()) else {
            return;
        };
        let param = (!r.param.is_empty()).then_some(r.param.as_str());
        r.result = Some(
            match s.remote.client.run_script(&layout, &r.script, param) {
                Ok(x) if x.error == 0 => {
                    format!("{} {}", t("Result:"), x.result.unwrap_or_default())
                }
                Ok(x) => format!(
                    "{} {} {}",
                    t("Error:"),
                    x.error,
                    x.result.unwrap_or_default()
                ),
                Err(e) => format!("{} {e}", t("Error:")),
            },
        );
        self.remote_refresh();
    }
}

/// The app on the hosted test file of the FileMaker Server VM
/// (`tools/oracle/remote_fixture.sh`); skipped unless `BASECRAFT_FMS_URL`,
/// `BASECRAFT_FMS_ACCOUNT` and `BASECRAFT_FMS_PASSWORD` are set
/// (`BASECRAFT_FMS_INSECURE=1` for its self-signed certificate).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    /// Runs frames until the login thread is done.
    fn wait_open(h: &mut Harness<'static, App>) {
        for _ in 0..400 {
            h.run_steps(1);
            if h.state().remote.opening.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        h.run_steps(3);
    }

    /// Serves canned Data API replies (method, path prefix, body) on
    /// 127.0.0.1 (plain HTTP is allowed there only).
    fn serve(routes: Vec<(&'static str, &'static str, String)>) -> String {
        use std::io::{BufRead, BufReader, Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':')
                        && k.eq_ignore_ascii_case("content-length")
                    {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                // A little latency: the window must stay responsive.
                std::thread::sleep(std::time::Duration::from_millis(30));
                let reply = routes
                    .iter()
                    .find(|(m, p, _)| *m == method && path.starts_with(p))
                    .map_or("{}".to_string(), |(_, _, b)| b.clone());
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                s.write_all(head.as_bytes()).unwrap();
            }
        });
        url
    }

    fn ok(response: &str) -> String {
        format!(r#"{{"response":{response},"messages":[{{"code":"0","message":"OK"}}]}}"#)
    }

    /// Open Remote against a mock Data API: login and loading on another
    /// thread (the spinner shows meanwhile), then the related field, the
    /// value list as a pop-up menu and the portal's rows in Browse mode.
    #[test]
    fn open_remote_mock_portal_value_list() {
        let db = "/fmi/data/vLatest/databases/Shop";
        let records = ok(
            r#"{"dataInfo":{"totalRecordCount":2,"foundCount":2,"returnedCount":2},"data":[{"fieldData":{"Name":"Ada","City":"London","Orders::Total":12},"portalData":{"Orders":[{"recordId":"7","Orders::Item":"Pen","Orders::Total":12,"modId":"0"},{"recordId":"8","Orders::Item":"Ink","Orders::Total":5,"modId":"0"}]},"recordId":"1","modId":"0"},{"fieldData":{"Name":"Grace","City":"Paris","Orders::Total":""},"portalData":{"Orders":[]},"recordId":"2","modId":"0"}]}"#,
        );
        let meta = ok(
            r#"{"fieldMetaData":[{"name":"Name","type":"normal","displayType":"editText","result":"text","global":false,"maxRepeat":1},{"name":"City","type":"normal","displayType":"popupMenu","result":"text","valueList":"Cities","global":false,"maxRepeat":1},{"name":"Orders::Total","type":"normal","displayType":"editText","result":"number","global":false,"maxRepeat":1}],"portalMetaData":{"Orders":[{"name":"Orders::Item","type":"normal","result":"text","global":false,"maxRepeat":1},{"name":"Orders::Total","type":"normal","result":"number","global":false,"maxRepeat":1}]},"valueLists":[{"name":"Cities","type":"customList","values":[{"displayValue":"London","value":"London"},{"displayValue":"Paris","value":"Paris"}]}]}"#,
        );
        let routes = vec![
            (
                "POST",
                "/fmi/data/vLatest/databases/Shop/sessions",
                ok(r#"{"token":"t"}"#),
            ),
            (
                "GET",
                "/fmi/data/vLatest/databases/Shop/layouts/Clients/records",
                records,
            ),
            (
                "GET",
                "/fmi/data/vLatest/databases/Shop/layouts/Clients",
                meta,
            ),
            (
                "GET",
                "/fmi/data/vLatest/databases/Shop/layouts",
                ok(r#"{"layouts":[{"name":"Clients","table":"Clients"}]}"#),
            ),
        ];
        let _ = db;
        let url = serve(routes);
        let mut app = App::default();
        app.remote.dialog = Some(Dialog {
            host: url,
            database: "Shop".into(),
            account: "api".into(),
            password: "pw".into(),
            insecure: false,
            error: None,
        });
        let mut h = crate::ui_tests::harness_for(app);
        h.run_steps(2);
        h.get_by_label(t("Open")).click();
        h.run_steps(1);
        // Still loading: the window draws the progress.
        assert!(h.state().remote.opening.is_some());
        crate::ui_tests::shot(&mut h, "m4_remote_opening");
        wait_open(&mut h);
        assert!(
            h.state().hosted(),
            "{:?}",
            h.state()
                .remote
                .dialog
                .as_ref()
                .and_then(|d| d.error.clone())
        );
        crate::ui_tests::shot(&mut h, "m4_remote_portal");
        // The City pop-up menu, the portal's rows on screen.
        assert!(
            h.get_all_by_role(egui::accesskit::Role::ComboBox)
                .any(|n| format!("{n:?}").contains("London"))
        );
        let shown: Vec<String> = h
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| format!("{n:?}"))
            .collect();
        for v in ["\"Ada\"", "\"12\"", "\"Pen\"", "\"Ink\"", "\"5\""] {
            assert!(shown.iter().any(|s| s.contains(v)), "{v} not shown");
        }
    }

    #[test]
    fn open_remote_browse_edit_refuse() {
        let (Ok(url), Ok(account), Ok(password)) = (
            std::env::var("BASECRAFT_FMS_URL"),
            std::env::var("BASECRAFT_FMS_ACCOUNT"),
            std::env::var("BASECRAFT_FMS_PASSWORD"),
        ) else {
            eprintln!("BASECRAFT_FMS_URL/ACCOUNT/PASSWORD not set: skipped");
            return;
        };
        let insecure = std::env::var("BASECRAFT_FMS_INSECURE").is_ok_and(|v| v == "1");
        let mut app = App::default();
        app.remote.dialog = Some(Dialog {
            host: url.clone(),
            database: "BCRemote".into(),
            account: account.clone(),
            password: password.clone(),
            insecure,
            error: None,
        });
        let mut h = Harness::builder()
            .with_size([1000.0, 700.0])
            .with_accessibility_check(false)
            // The status toolbar's arrows come from the system's fonts.
            .with_context_setup(|ctx| {
                ctx.add_font_provider(std::sync::Arc::new(
                    egui_system_font_provider::SystemFontProvider::new(),
                ));
            })
            .build_eframe(move |_| app);
        h.run_steps(2);
        h.get_by_label(t("Open")).click();
        wait_open(&mut h);
        assert!(
            h.state().hosted(),
            "{:?}",
            h.state()
                .remote
                .dialog
                .as_ref()
                .and_then(|d| d.error.clone())
        );
        h.get_by_label_contains(t("Hosted on"));
        if let Some(dir) = std::env::var_os("BASECRAFT_SHOTS") {
            let img = h.render().unwrap();
            img.save(std::path::Path::new(&dir).join("remote_browse.png"))
                .unwrap();
        }
        let d = h.state().doc.as_ref().unwrap();
        assert_eq!(d.name, "BCRemote");
        let t_id =
            d.db.tables
                .iter()
                .find(|t| t.name == "Contacts")
                .unwrap()
                .id;
        assert!(d.db.tables[0].records.len() >= 3);

        // Layout mode is refused with a message.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
        h.run_steps(2);
        assert_ne!(h.state().mode(), crate::Mode::Layout);
        assert!(
            h.state()
                .messages
                .iter()
                .any(|(_, m)| m.contains(t("not available for a file opened through the Data API")))
        );
        h.state_mut().messages.clear();

        // An edit, committed, reaches the server.
        let stamp = format!("BC {}", std::process::id());
        {
            let app = h.state_mut();
            let d = app.doc.as_mut().unwrap();
            let city = d.db.table(t_id).unwrap().field_by_name("City").unwrap().id;
            let i =
                d.db.table(t_id)
                    .unwrap()
                    .records
                    .iter()
                    .position(|r| r.values.values().any(|v| v == "Linus"));
            d.db.set_values(t_id, i.unwrap(), &[(city, stamp.clone())]);
        }
        h.run_steps(2);
        let target = remote::Target::parse(&format!("{url}/BCRemote")).unwrap();
        let c = remote::Client::login(&target, &account, &password, remote::Options { insecure })
            .unwrap();
        let q = |c: &remote::Client| {
            c.find(
                "Contacts",
                &[remote::FindRequest {
                    criteria: vec![("Name".into(), "==Linus".into())],
                    omit: false,
                }],
                &remote::Query::default(),
            )
            .unwrap()
            .records[0]
                .clone()
        };
        let linus = q(&c);
        assert_eq!(linus.fields["City"], stamp);
        assert!(h.state().remote.session.as_ref().unwrap().errors.is_empty());
        c.edit(
            "Contacts",
            linus.id,
            &[("City".into(), "Helsinki".into())],
            None,
        )
        .unwrap();
    }
}
