//! The application side of script steps the engine cannot do alone
//! (`script::Suspend::Ui`): Open URL and Send Mail behind a confirmation,
//! Sort Records with dialog, Insert from Index, Get Folder Path, Print
//! (to PDF), Save Records as PDF / Excel, Export Field Contents, Save a
//! Copy as, Import / Export Records, New / Open / Close File, Adjust
//! Window, writes of the data file steps outside the file's folder, PDF
//! documents. The run waits until the answer is ready.

use eframe::egui;
use script::{Adjust, Answer, UiRequest};

use crate::{App, i18n, tr};

fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return tr(key);
    }
    match key {
        "Open URL" => "Ouvrir l'URL",
        "A script wants to open:" => "Un script veut ouvrir :",
        "Open" => "Ouvrir",
        "Insert from Index" => "Insérer d'après l'index",
        "OK" => "OK",
        "Cancel" => "Annuler",
        "Write a file" => "Écrire un fichier",
        "A script wants to write outside the file's folder:" => {
            "Un script veut écrire hors du dossier du fichier :"
        }
        "Allow" => "Autoriser",
        k => tr(k),
    }
}

/// File commands done once the script has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    New,
    Open,
    Close,
}

#[derive(Default)]
pub struct State {
    /// Insert from Index: the value picked.
    pick: Option<usize>,
    /// Sort Records [With dialog]: the dialog was opened.
    sort_open: bool,
    pub after: Option<After>,
    /// Last URL handed to the OS (tests).
    pub opened: Option<String>,
    /// Last print job: the PDF file and whether it went to the viewer
    /// (with dialog) rather than the default printer (tests).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub printed: Option<(std::path::PathBuf, bool)>,
    /// Data file steps: the file's folder, writes the user agreed to.
    pub disk: script::files::Disk,
}

/// Schemes Open URL hands to the OS.
const SCHEMES: [&str; 4] = ["http:", "https:", "mailto:", "ftp:"];

impl App {
    /// Does what `r` asks; the answer once done (`None`: next frame).
    pub fn ui_request(&mut self, ctx: &egui::Context, r: &UiRequest) -> Option<Answer> {
        match r {
            UiRequest::OpenUrl { url, .. } => self.confirm_url(ctx, url),
            UiRequest::Sort => {
                if !self.steps_ui.sort_open {
                    self.steps_ui.sort_open = true;
                    self.m5.script_sort = None;
                    self.open_sort(true);
                }
                let ok = self.m5.script_sort.take()?;
                self.steps_ui.sort_open = false;
                Some(if ok { Answer::Continue } else { Answer::Cancel })
            }
            UiRequest::Index { values } => self.pick_index(ctx, values),
            UiRequest::FolderPath { title, .. } => Some(pick_folder(title)),
            UiRequest::Print { dialog } => {
                self.print(ctx, *dialog);
                Some(Answer::Continue)
            }
            // ponytail: no Print Setup dialog (BaseCraft prints on US
            // Letter, portrait); the step goes on as with no dialog.
            UiRequest::PrintSetup => Some(Answer::Continue),
            UiRequest::Dialog(d) => {
                match d {
                    script::AppDialog::ScriptWorkspace => {
                        self.open_workspace();
                        crate::focus_tool_window(ctx, crate::workspace::KEY);
                    }
                    script::AppDialog::Preferences => self.prefs = Some(crate::prefs::get()),
                    script::AppDialog::FindReplace => self.open_find_replace(),
                }
                Some(Answer::Continue)
            }
            UiRequest::SavePdf { path, dialog } => {
                self.save_pdf(ctx, path.as_deref(), *dialog);
                Some(Answer::Continue)
            }
            UiRequest::SaveExcel { path, dialog } => {
                let bytes = self.excel_bytes()?;
                self.write_output(ctx, path.as_deref(), *dialog, "xlsx", bytes);
                Some(Answer::Continue)
            }
            UiRequest::ExportField { path, text } => {
                let bytes = text.replace('\r', "\r\n").into_bytes();
                self.write_output(ctx, path.as_deref(), path.is_none(), "txt", bytes);
                Some(Answer::Continue)
            }
            // A stored path in the file's folder is used with no dialog.
            // ponytail: compacted and self-contained copies are plain
            // copies (BaseCraft always writes a fresh image).
            UiRequest::SaveCopy {
                path,
                kind,
                folders,
            } => {
                let target = self.stored_target(path.as_deref(), *folders);
                self.save_to(ctx, true, target, *kind == 3);
                Some(Answer::Continue)
            }
            UiRequest::Import { .. } => {
                self.pick_import_file(ctx);
                Some(Answer::Continue)
            }
            UiRequest::Export { .. } => {
                self.open_export();
                Some(Answer::Continue)
            }
            UiRequest::NewFile => self.after_run(After::New),
            UiRequest::OpenFile => self.after_run(After::Open),
            UiRequest::CloseFile => self.after_run(After::Close),
            UiRequest::ConfirmFile { path } => self.confirm_file(ctx, path),
            // Insert from URL: the window stays live while the request
            // runs on its thread; the script goes on once it has answered.
            UiRequest::Fetch { .. } => {
                if self.engine.as_ref().is_none_or(|e| e.fetch_done()) {
                    return Some(Answer::Continue);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
                None
            }
            // Print PDF and Close PDF without a path: saved where the user
            // says (BaseCraft prints to PDF).
            UiRequest::PdfDocument { path, data, .. } => {
                self.write_output(ctx, path.as_deref(), true, "pdf", data.clone());
                Some(Answer::Continue)
            }
            UiRequest::Adjust(a) => {
                let cmd = match a {
                    Adjust::Maximize => egui::ViewportCommand::Maximized(true),
                    Adjust::Minimize | Adjust::Hide => egui::ViewportCommand::Minimized(true),
                    Adjust::Restore | Adjust::ResizeToFit => {
                        egui::ViewportCommand::Maximized(false)
                    }
                };
                ctx.send_viewport_cmd(cmd);
                Some(Answer::Continue)
            }
        }
    }

    fn after_run(&mut self, a: After) -> Option<Answer> {
        self.steps_ui.after = Some(a);
        Some(Answer::Continue)
    }

    /// New / Open / Close File once the script has ended (they replace
    /// the open file).
    pub fn script_ui_frame(&mut self, ctx: &egui::Context) {
        if self.active.is_some() {
            return;
        }
        match self.steps_ui.after.take() {
            Some(After::Open) => self.open_dialog(ctx),
            Some(After::Close) => {
                *self = App {
                    incoming: self.incoming.clone(),
                    ..Default::default()
                };
            }
            Some(After::New) => match fmp12::schema_edit::new_file("Table 1") {
                Ok(b) => self.open("Untitled".into(), b),
                Err(e) => self.error = Some(e.to_string()),
            },
            None => {}
        }
    }

    /// Open URL: FileMaker hands the URL to the OS; BaseCraft asks first.
    fn confirm_url(&mut self, ctx: &egui::Context, url: &str) -> Option<Answer> {
        let lower = url.trim().to_lowercase();
        if !SCHEMES.iter().any(|s| lower.starts_with(s)) {
            return Some(Answer::Cancel);
        }
        let mut answer = None;
        egui::Modal::new(egui::Id::NULL.with("open_url")).show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.heading(t("Open URL"));
            ui.label(t("A script wants to open:"));
            ui.monospace(url);
            ui.horizontal(|ui| {
                if ui.button(t("Open")).clicked() {
                    answer = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let yes = answer?;
        if yes {
            open_url(url.trim());
            self.steps_ui.opened = Some(url.trim().to_string());
        }
        Some(if yes {
            Answer::Continue
        } else {
            Answer::Cancel
        })
    }

    /// A data file step writes outside the file's folder: ask first.
    fn confirm_file(&mut self, ctx: &egui::Context, path: &str) -> Option<Answer> {
        let mut answer = None;
        egui::Modal::new(egui::Id::NULL.with("confirm_file")).show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.heading(t("Write a file"));
            ui.label(t("A script wants to write outside the file's folder:"));
            ui.monospace(path);
            ui.horizontal(|ui| {
                if ui.button(t("Allow")).clicked() {
                    answer = Some(Answer::Continue);
                }
                if ui.button(t("Cancel")).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        answer
    }

    fn pick_index(&mut self, ctx: &egui::Context, values: &[String]) -> Option<Answer> {
        let mut answer = None;
        egui::Modal::new(egui::Id::NULL.with("insert_index")).show(ctx, |ui| {
            ui.heading(t("Insert from Index"));
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for (i, v) in values.iter().enumerate() {
                        let r = ui.selectable_label(self.steps_ui.pick == Some(i), v);
                        if r.clicked() {
                            self.steps_ui.pick = Some(i);
                        }
                        if r.double_clicked() {
                            answer = Some(true);
                        }
                    }
                });
            ui.horizontal(|ui| {
                if ui.button(t("OK")).clicked() {
                    answer = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let ok = answer?;
        let pick = self.steps_ui.pick.take();
        Some(match pick.and_then(|i| values.get(i)) {
            Some(v) if ok => Answer::Text(v.clone()),
            _ => Answer::Cancel,
        })
    }

    /// The found set on the current layout as PDF (Print, Save Records as PDF).
    fn pdf_bytes(&mut self) -> Option<Vec<u8>> {
        let t = self.table()?;
        let found = self.found(t).clone();
        let d = self.doc.as_ref()?;
        let l = d.layouts.get(self.layout)?;
        match print::pdf(
            &d.db,
            l.id,
            &found,
            &self.last_sort,
            &print::Paper::letter(),
        ) {
            Ok((b, _)) => Some(b),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    fn save_pdf(&mut self, ctx: &egui::Context, path: Option<&str>, dialog: bool) {
        if let Some(b) = self.pdf_bytes() {
            self.write_output(ctx, path, dialog, "pdf", b);
        }
    }

    /// Print (the step and File > Print...): the found set as PDF, handed
    /// to the OS. With dialog, the PDF opens in the system viewer, whose
    /// Print dialog takes over; without, it goes to the default printer
    /// (`print_file`). In the browser it is downloaded.
    pub(crate) fn print(&mut self, ctx: &egui::Context, dialog: bool) {
        if self.doc.as_ref().is_some_and(|d| !d.db.can_print()) {
            self.error = Some(engine::DENIED_MESSAGE.into());
            return;
        }
        let Some(bytes) = self.pdf_bytes() else {
            return;
        };
        #[cfg(target_arch = "wasm32")]
        {
            let _ = dialog;
            self.write_output(ctx, None, true, "pdf", bytes);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            // Our own name in the temporary folder: nothing of the user's
            // goes on a command line.
            // ponytail: the file stays for the viewer; the OS cleans the
            // temporary folder.
            let n = PRINT_JOBS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("basecraft-print-{}-{n}.pdf", std::process::id()));
            if let Err(e) = std::fs::write(&path, &bytes) {
                self.error = Some(e.to_string());
                return;
            }
            if let Err(e) = print_file(&path, dialog) {
                self.error = Some(e);
            }
            self.steps_ui.printed = Some((path, dialog));
        }
    }

    /// Save Records as Excel: the layout's fields, found set in order.
    fn excel_bytes(&mut self) -> Option<Vec<u8>> {
        let t = self.table()?;
        let found = self.found(t).clone();
        let d = self.doc.as_ref()?;
        let l = d.layouts.get(self.layout)?;
        let to = (l.table_occurrence & 0xFFF) as u32;
        let mut columns: Vec<engine::export::Column> = Vec::new();
        for o in l.objects.iter().filter(|o| o.kind == 1) {
            let Some((fto, f, _)) = o.field else { continue };
            let fto = (fto & 0xFFF) as u32;
            let Some(tbl) = d.db.table_of(fto) else {
                continue;
            };
            let Some(fd) = tbl.field(f as u32) else {
                continue;
            };
            if tbl.id != t || columns.iter().any(|c| c.field == fd.id) {
                continue;
            }
            columns.push(engine::export::Column {
                to: fto,
                table: tbl.id,
                field: fd.id,
                name: fd.name.clone(),
                data_type: fd.data_type,
            });
        }
        let file_name = format!("{}.fmp12", d.name);
        Some(
            engine::export::Export {
                db: &d.db,
                to,
                found: &found,
                columns: &columns,
                formatted: true,
                file_name: &file_name,
                group_by: &[],
            }
            .write(engine::export::Format::Xlsx),
        )
    }

    /// A stored `file:` path inside the open file's folder (its folders
    /// made when `folders`); `None`: ask.
    fn stored_target(&self, path: Option<&str>, folders: bool) -> Option<std::path::PathBuf> {
        let rel = path.and_then(relative_path)?;
        let p = self.doc.as_ref()?.path.as_ref()?.parent()?.join(rel);
        if folders && let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Some(p)
    }

    /// Writes a script's output file: next to the open file when the step
    /// names a plain relative `file:` path and has no dialog; otherwise a
    /// save dialog (a download in the browser) asks where.
    fn write_output(
        &mut self,
        ctx: &egui::Context,
        path: Option<&str>,
        dialog: bool,
        ext: &str,
        bytes: Vec<u8>,
    ) {
        let name = self
            .doc
            .as_ref()
            .map_or("Untitled".into(), |d| d.name.clone());
        #[cfg(not(target_arch = "wasm32"))]
        if !dialog
            && let Some(rel) = path.and_then(relative_path)
            && let Some(dir) = self
                .doc
                .as_ref()
                .and_then(|d| d.path.as_ref()?.parent().map(|p| p.to_path_buf()))
        {
            if let Err(e) = std::fs::write(dir.join(rel), &bytes) {
                self.error = Some(e.to_string());
            }
            return;
        }
        let _ = (path, dialog);
        crate::records::save_bytes(ctx, format!("{name}.{ext}"), ext, bytes, &mut self.error);
    }
}

/// `file:out.pdf` → `out.pdf`; `None` for absolute paths, other kinds
/// (`filewin:`…) and paths leaving the file's folder.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn relative_path(stored: &str) -> Option<&str> {
    let rel = stored.strip_prefix("file:")?;
    let bad = rel.is_empty()
        || rel.starts_with(['/', '\\'])
        || rel.contains(':')
        || rel.split(['/', '\\']).any(|p| p == "..");
    (!bad).then_some(rel)
}

#[cfg(not(target_arch = "wasm32"))]
fn pick_folder(title: &str) -> Answer {
    let mut d = rfd::FileDialog::new();
    if !title.is_empty() {
        d = d.set_title(title);
    }
    match d.pick_folder() {
        // FileMaker paths: `/C:/dir/` on Windows, `/dir/` elsewhere.
        Some(p) => {
            let s = p.to_string_lossy().replace('\\', "/");
            let s = if s.starts_with('/') {
                s
            } else {
                format!("/{s}")
            };
            Answer::Text(if s.ends_with('/') { s } else { format!("{s}/") })
        }
        None => Answer::Cancel,
    }
}

/// No folders in the browser: cancelled (error 1).
#[cfg(target_arch = "wasm32")]
fn pick_folder(_title: &str) -> Answer {
    Answer::Cancel
}

#[cfg(not(target_arch = "wasm32"))]
static PRINT_JOBS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Hands a PDF to the OS, with no shell parsing a user's text (the path
/// is BaseCraft's own temporary file, passed as one argument or, to
/// PowerShell, in an environment variable):
///
/// * with dialog: the system viewer (`rundll32 url.dll,FileProtocolHandler`
///   on Windows, `open` on macOS, `xdg-open` elsewhere), whose Print
///   dialog the user prints from;
/// * without: the default printer, by the PDF handler's "print" verb on
///   Windows (`powershell Start-Process -Verb Print`), by CUPS `lp`
///   elsewhere; when that fails the PDF opens in the viewer instead.
///
/// Tests never reach the OS.
#[cfg(not(target_arch = "wasm32"))]
fn print_file(path: &std::path::Path, dialog: bool) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = (path, dialog);
        Ok(())
    }
    #[cfg(not(test))]
    {
        use std::process::Command;
        if !dialog {
            #[cfg(target_os = "windows")]
            let r = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Process -FilePath $env:BASECRAFT_PRINT -Verb Print",
                ])
                .env("BASECRAFT_PRINT", path)
                .status();
            #[cfg(not(target_os = "windows"))]
            let r = Command::new("lp").arg(path).status();
            if r.is_ok_and(|s| s.success()) {
                return Ok(());
            }
        }
        let s = path.to_string_lossy();
        #[cfg(target_os = "windows")]
        let r = Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &s])
            .spawn();
        #[cfg(target_os = "macos")]
        let r = Command::new("open").arg(&*s).spawn();
        #[cfg(all(unix, not(target_os = "macos")))]
        let r = Command::new("xdg-open").arg(&*s).spawn();
        r.map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Hands a URL to the OS (no shell: the URL is one argument).
#[cfg(not(target_arch = "wasm32"))]
fn open_url(url: &str) {
    #[cfg(test)]
    {
        let _ = url;
    }
    #[cfg(all(not(test), target_os = "windows"))]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(all(not(test), target_os = "macos"))]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[cfg(target_arch = "wasm32")]
fn open_url(url: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target(url, "_blank");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn output_paths_stay_next_to_the_file() {
        use super::relative_path as r;
        assert_eq!(r("file:out.pdf"), Some("out.pdf"));
        assert_eq!(r("file:sub/out.pdf"), Some("sub/out.pdf"));
        for bad in [
            "file:../x",
            "file:/etc/x",
            "filewin:/C:/x",
            "file:C:/x",
            "file:",
        ] {
            assert_eq!(r(bad), None, "{bad}");
        }
    }
}
