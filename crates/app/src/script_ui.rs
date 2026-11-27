//! The application side of script steps the engine cannot do alone
//! (`script::Suspend::Ui`): Open URL and Send Mail behind a confirmation,
//! Sort Records with dialog, Insert from Index, Get Folder Path, Print
//! (to PDF), Save Records as PDF / Excel, Export Field Contents, Save a
//! Copy as, Import / Export Records, New / Open / Close File, Adjust
//! Window. The run waits until the answer is ready.

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
            UiRequest::Print { .. } => {
                self.save_pdf(ctx, None, true);
                Some(Answer::Continue)
            }
            UiRequest::PrintSetup => Some(Answer::Continue),
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
            // ponytail: the stored path of Save a Copy as is not used;
            // the save dialog always asks.
            UiRequest::SaveCopy { .. } => {
                self.save(ctx, true);
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
    fn save_pdf(&mut self, ctx: &egui::Context, path: Option<&str>, dialog: bool) {
        let Some(t) = self.table() else { return };
        let found = self.found(t).clone();
        let Some(d) = self.doc.as_ref() else { return };
        let Some(l) = d.layouts.get(self.layout) else {
            return;
        };
        match print::pdf(
            &d.db,
            l.id,
            &found,
            &self.last_sort,
            &print::Paper::letter(),
        ) {
            Ok((b, _)) => self.write_output(ctx, path, dialog, "pdf", b),
            Err(e) => self.error = Some(e),
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
