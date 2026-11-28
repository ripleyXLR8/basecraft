//! Steps added for M5: windows, portal rows, related records, editing,
//! fields, records, found sets, files and the miscellaneous steps. Their
//! parameters stay raw ([`StepParams::Other`]); the storage of each one
//! was read from steps pasted into FileMaker Pro 26 (FORMAT.md §8).

use super::*;

/// Step ids handled here.
pub mod sid {
    pub const NEXT_FIELD: u16 = 4;
    pub const PREVIOUS_FIELD: u16 = 5;
    pub const INSERT_FROM_INDEX: u16 = 11;
    pub const INSERT_FROM_LAST_VISITED: u16 = 12;
    pub const MODIFY_LAST_FIND: u16 = 24;
    pub const SHOW_HIDE_TOOLBARS: u16 = 29;
    pub const VIEW_AS: u16 = 30;
    pub const ADJUST_WINDOW: u16 = 31;
    pub const OPEN_FILE: u16 = 33;
    pub const CLOSE_FILE: u16 = 34;
    pub const IMPORT_RECORDS: u16 = 35;
    pub const EXPORT_RECORDS: u16 = 36;
    pub const SAVE_A_COPY_AS: u16 = 37;
    pub const PRINT_SETUP: u16 = 42;
    pub const PRINT: u16 = 43;
    pub const UNDO_REDO: u16 = 45;
    pub const CUT: u16 = 46;
    pub const COPY: u16 = 47;
    pub const CLEAR: u16 = 49;
    pub const SELECT_ALL: u16 = 50;
    pub const SEND_MAIL: u16 = 63;
    pub const DIAL_PHONE: u16 = 65;
    pub const SPEAK: u16 = 66;
    pub const GO_TO_RELATED_RECORD: u16 = 74;
    pub const SCROLL_WINDOW: u16 = 81;
    pub const NEW_FILE: u16 = 82;
    pub const SET_MULTI_USER: u16 = 84;
    pub const SHOW_HIDE_TEXT_RULER: u16 = 92;
    pub const RECOVER_FILE: u16 = 95;
    pub const SET_ZOOM_LEVEL: u16 = 97;
    pub const COPY_ALL_RECORDS: u16 = 98;
    pub const GO_TO_PORTAL_ROW: u16 = 99;
    pub const COPY_RECORD: u16 = 101;
    pub const FLUSH_CACHE: u16 = 102;
    pub const OPEN_URL: u16 = 111;
    pub const SET_NEXT_SERIAL: u16 = 116;
    pub const EXECUTE_SQL: u16 = 117;
    pub const MOVE_RESIZE_WINDOW: u16 = 119;
    pub const ARRANGE_WINDOWS: u16 = 120;
    pub const CLOSE_WINDOW: u16 = 121;
    pub const NEW_WINDOW: u16 = 122;
    pub const SELECT_WINDOW: u16 = 123;
    pub const SET_WINDOW_TITLE: u16 = 124;
    pub const CONSTRAIN: u16 = 126;
    pub const EXTEND: u16 = 127;
    pub const SET_SELECTION: u16 = 130;
    pub const EXPORT_FIELD: u16 = 132;
    pub const OPEN_RECORD: u16 = 133;
    pub const SAVE_AS_EXCEL: u16 = 143;
    pub const SAVE_AS_PDF: u16 = 144;
    pub const SET_WEB_VIEWER: u16 = 146;
    pub const INSTALL_ON_TIMER: u16 = 148;
    pub const QUICK_FIND: u16 = 150;
    pub const FIND_MATCHING: u16 = 155;
    pub const INSERT_FROM_URL: u16 = 160;
    pub const SHOW_HIDE_MENUBAR: u16 = 166;
    pub const SET_ANIMATION: u16 = 168;
    pub const GET_FOLDER_PATH: u16 = 181;
    pub const TRUNCATE_TABLE: u16 = 182;
    pub const REGION_MONITOR: u16 = 185;
    pub const SET_ERROR_LOGGING: u16 = 200;
    pub const ENABLE_TOUCH_KEYBOARD: u16 = 174;
    pub const CHECK_SELECTION: u16 = 18;
    pub const CHECK_RECORD: u16 = 19;
    pub const CHECK_FOUND_SET: u16 = 20;
    pub const EXIT_APPLICATION: u16 = 44;
    pub const SET_USE_SYSTEM_FORMATS: u16 = 94;
    pub const DELETE_PORTAL_ROW: u16 = 104;
    pub const SORT_BY_FIELD: u16 = 154;
    pub const REFRESH_OBJECT: u16 = 167;
    pub const CLOSE_POPOVER: u16 = 169;
    pub const REFRESH_PORTAL: u16 = 180;
    pub const OPEN_TRANSACTION: u16 = 205;
    pub const COMMIT_TRANSACTION: u16 = 206;
    pub const REVERT_TRANSACTION: u16 = 207;
    pub const SET_DICTIONARY: u16 = 209;
    pub const REVERT_ON_ERROR: u16 = 223;
    pub const FLUSH_WEB_COOKIES: u16 = 237;
    pub const SET_ALLOWED_ORIENTATIONS: u16 = 176;
    pub const LOCAL_NOTIFICATION: u16 = 187;
    pub const CONFIGURE_AI_ACCOUNT: u16 = 212;
}

/// Steps that cannot run in a desktop, local application: iOS-only,
/// server-only, plug-ins, AI, macOS-only, transactions over a host and
/// the dialogs of FileMaker's own design tools. They return
/// [`error::UNAVAILABLE`] as FileMaker does on Windows (FORMAT.md §8).
pub const UNSUPPORTED: &[u16] = &[
    3, 32, 38, 57, 59, 64, 67, 88, 96, 105, 106, 107, 108, 109, 112, 113, 114, 115, 118, 129, 139,
    140, 142, 149, 151, 152, 156, 157, 161, 164, 165, 172, 175, 177, 178, 179, 183, 184, 201, 202,
    203, 208, 210, 211, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 224, 225, 226, 227, 239,
    240, 241,
];

/// Option bits of these steps (record bytes 24–27, FORMAT.md §8).
mod bit {
    /// "With dialog: Off".
    pub const NO_DIALOG: u32 = 0x80;
    /// Go to Portal Row, Insert…, Cut/Copy/Paste/Clear: select entire contents.
    pub const SELECT: u32 = 0x1000;
    /// Go to Portal Row: exit after last.
    pub const EXIT_AFTER_LAST: u32 = 0x2000;
    /// Set Layout Object Animation and other switches: On.
    pub const ON: u32 = 0x2_0000;
    /// Go to Related Record: show only related records, match all
    /// records of the found set, show in a new window.
    pub const ONLY_RELATED: u32 = 0x100;
    pub const MATCH_ALL: u32 = 0x200;
    pub const NEW_WINDOW: u32 = 0x2_0000;
}

/// FileMaker's error codes used here, as FileMaker 26 returned them
/// (FORMAT.md §8).
mod err {
    /// Command is invalid: Set Next Serial Value on a field that is not
    /// a serial number, Execute
    /// SQL without a data source.
    pub const COMMAND_INVALID: u32 = 5;
    /// Set Allowed Orientations on the desktop.
    pub const UNKNOWN_COMMAND: u32 = 4;
    pub const WINDOW_MISSING: u32 = 112;
    pub const TABLE_MISSING: u32 = 106;
    /// Go to Related Record with a layout not based on the related table.
    pub const WRONG_LAYOUT: u32 = 414;
}

/// The output path stored in parameter `[128]` key 130 (the first of
/// its path list, [`crate::files::decode_path_list`]): `file:out.txt` as
/// FileMaker shows it.
pub fn stored_path(p: &fmp12::scripts::RawParams) -> Option<String> {
    crate::files::decode_path_list(&p.get(&[128], 130)?)
        .into_iter()
        .next()
}

impl Engine {
    /// Runs the steps of this module; `None` for any other step.
    pub(crate) fn exec_more(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Option<Result<Flow, u32>> {
        use sid::*;
        let p = &s.raw_params;
        let o = s.options;
        let dialog = o & bit::NO_DIALOG == 0;
        let r: Result<Flow, u32> = match s.id {
            // Control.
            INSTALL_ON_TIMER => (|| {
                let interval = match p.calc(1) {
                    Some(c) => self.eval(host, frame, &c)?.to_text().trim().parse().ok(),
                    None => None,
                };
                self.timer = match (s.inline_id(), interval) {
                    (Some(script), Some(i)) if i > 0.0 => Some(Timer {
                        script,
                        parameter: p.calc(0),
                        interval: i,
                    }),
                    _ => None,
                };
                Ok(Flow::Next)
            })(),
            SET_ANIMATION => {
                self.animation = o & bit::ON != 0;
                Ok(Flow::Next)
            }
            SET_ERROR_LOGGING => {
                self.error_logging = (o & bit::ON != 0).then(|| p.calc(0));
                Ok(Flow::Next)
            }
            // Navigation.
            GO_TO_PORTAL_ROW => self.go_to_portal_row(host, frame, s),
            GO_TO_RELATED_RECORD => self.go_to_related(host, frame, s),
            NEXT_FIELD | PREVIOUS_FIELD => {
                let fields = layout_fields(host);
                let w = host.window();
                let at = w.field.and_then(|f| fields.iter().position(|x| *x == f));
                let n = fields.len();
                let next = match (at, s.id) {
                    (_, _) if n == 0 => return Some(Err(error::FIELD_MISSING)),
                    (None, NEXT_FIELD) => 0,
                    (None, _) => n - 1,
                    (Some(i), NEXT_FIELD) => (i + 1) % n,
                    (Some(i), _) => (i + n - 1) % n,
                };
                let f = fields[next];
                host.window().field = Some(f);
                host.window().object.clear();
                host.go_to_field(Some(f), false);
                Ok(Flow::Next)
            }
            // Editing.
            CUT | sid::COPY | CLEAR => self.cut_copy_clear(host, s),
            SELECT_ALL => {
                // No active field: nothing to select (3 without a record).
                let Some(f) = host.window().field else {
                    let none = Self::context(host).2.is_none();
                    return Some(if none {
                        Err(error::UNAVAILABLE)
                    } else {
                        Ok(Flow::Next)
                    });
                };
                let len = self.field_value(host, f).map_or(0, |v| v.chars().count());
                host.window().selection = (0, len);
                Ok(Flow::Next)
            }
            SET_SELECTION => (|| {
                let f = s
                    .inline_field()
                    .or(host.window().field)
                    .ok_or(error::FIELD_MISSING)?;
                let num =
                    |e: &mut Self, h: &mut dyn Host, fr: &mut Frame, i| -> Result<usize, u32> {
                        Ok(match p.calc(i) {
                            Some(c) => e
                                .eval(h, fr, &c)?
                                .to_text()
                                .trim()
                                .parse::<f64>()
                                .unwrap_or(1.0) as usize,
                            None => 1,
                        })
                    };
                let start = num(self, host, frame, 0)?.max(1);
                let end = num(self, host, frame, 1)?.max(start - 1);
                if !host.go_to_field(Some(f), false) {
                    return Err(error::FIELD_MISSING);
                }
                let w = host.window();
                w.field = Some(f);
                w.object.clear();
                w.selection = (start - 1, end + 1 - start);
                Ok(Flow::Next)
            })(),
            UNDO_REDO => {
                // Bytes 22–23: 1 undo, 2 redo, 3 toggle.
                match (s.arg, self.redo.is_some()) {
                    (2, _) | (3, true) => self.redo_edit(host),
                    _ => self.undo_edit(host),
                }
                Ok(Flow::Next)
            }
            // Fields.
            SET_NEXT_SERIAL => (|| {
                let f = s.inline_field().ok_or(error::FIELD_MISSING)?;
                let v = match p.calc(0) {
                    Some(c) => self.eval(host, frame, &c)?,
                    None => return Err(err::COMMAND_INVALID),
                };
                let db = host.db();
                let tid = db.table_of(f.to).ok_or(error::FIELD_MISSING)?.id;
                let t = db
                    .tables
                    .iter_mut()
                    .find(|t| t.id == tid)
                    .ok_or(error::FIELD_MISSING)?;
                let fd = t
                    .fields
                    .iter_mut()
                    .find(|x| x.id == f.field)
                    .ok_or(error::FIELD_MISSING)?;
                if !fd.def.auto_serial() {
                    return Err(err::COMMAND_INVALID);
                }
                fd.def.serial_next = v.to_text();
                Ok(Flow::Next)
            })(),
            INSERT_FROM_INDEX => (|| {
                let f = s
                    .inline_field()
                    .or(host.window().field)
                    .ok_or(error::FIELD_MISSING)?;
                let db = host.db();
                let t = db.table_of(f.to).ok_or(error::FIELD_MISSING)?;
                t.field(f.field).ok_or(error::FIELD_MISSING)?;
                let mut values: Vec<String> = (0..t.records.len())
                    .map(|r| db.value(t.id, r, f.field).to_text())
                    .filter(|v| !v.is_empty())
                    .collect();
                values.sort_by_key(|v| v.to_lowercase());
                values.dedup();
                Ok(Flow::Suspend(
                    Suspend::Ui(UiRequest::Index { values }),
                    Pending::Ui(UiPending::Insert(f, o & bit::SELECT != 0)),
                ))
            })(),
            INSERT_FROM_LAST_VISITED => (|| {
                let f = s
                    .inline_field()
                    .or(host.window().field)
                    .ok_or(error::FIELD_MISSING)?;
                let (t, r) = self.last_visited.ok_or(error::RECORD_MISSING)?;
                let db = host.db();
                if db.table_of(f.to).map(|x| x.id) != Some(t) {
                    return Err(error::RECORD_MISSING);
                }
                let v = db.value(t, r, f.field).to_text();
                self.insert(host, f, &v, o & bit::SELECT != 0)?;
                Ok(Flow::Next)
            })(),
            // ponytail: no HTTP client in BaseCraft; Insert from URL is off.
            INSERT_FROM_URL => Err(error::UNAVAILABLE),
            EXPORT_FIELD => (|| {
                let f = s.inline_field().ok_or(error::FIELD_MISSING)?;
                let text = self.field_value(host, f).ok_or(error::RECORD_MISSING)?;
                ui(UiRequest::ExportField {
                    path: stored_path(p),
                    text,
                })
            })(),
            // Records.
            OPEN_RECORD => (|| {
                let (table, _, rec) = Self::context(host);
                let r = rec.ok_or(error::RECORD_MISSING)?;
                if !matches!(&self.open, Some((t, x, _)) if *t == table && *x == r) {
                    self.commit(host)?;
                    let old = host
                        .db()
                        .table(table)
                        .and_then(|t| t.records.get(r))
                        .cloned();
                    self.open = Some((table, r, old));
                }
                Ok(Flow::Next)
            })(),
            COPY_RECORD | COPY_ALL_RECORDS => self.copy_records(host, s.id == COPY_ALL_RECORDS),
            TRUNCATE_TABLE => (|| {
                // Inline slot `03 02 <path int>`: a base table (arg 1).
                // No table chosen: 106, nothing deleted (FileMaker 26).
                let table = match (s.arg, s.raw.get(6..)) {
                    (1, Some([3, 2, rest @ ..])) => {
                        fmp12::path_int(rest.get(..2).unwrap_or_default())
                            .ok_or(err::TABLE_MISSING)?
                    }
                    _ => return Err(err::TABLE_MISSING),
                };
                let n = host
                    .db()
                    .table(table)
                    .ok_or(err::TABLE_MISSING)?
                    .records
                    .len();
                self.open = None;
                let all: Vec<usize> = (0..n).collect();
                if dialog {
                    return Ok(confirm_delete(
                        "Permanently delete ALL records in this table?",
                        table,
                        all,
                    ));
                }
                delete_records(host, table, &all);
                Ok(Flow::Next)
            })(),
            IMPORT_RECORDS => ui(UiRequest::Import { dialog }),
            EXPORT_RECORDS => ui(UiRequest::Export { dialog }),
            // Found sets.
            MODIFY_LAST_FIND => (|| {
                self.commit(host)?;
                let w = host.window();
                w.requests = if w.last_find.is_empty() {
                    vec![Request::default()]
                } else {
                    w.last_find.clone()
                };
                w.current_request = 0;
                w.mode = Mode::Find;
                Ok(Flow::Next)
            })(),
            CONSTRAIN | EXTEND => (|| {
                self.commit(host)?;
                let (_, to, _) = Self::context(host);
                let reqs = if o & fmp12::scripts::opt::RESTORE != 0 {
                    let stored = fmp12::scripts::find_requests(p);
                    self.requests(host, to, &stored)
                } else {
                    host.window().requests.clone()
                };
                if reqs.iter().all(|r| r.criteria.is_empty()) {
                    return Err(error::NO_CRITERIA);
                }
                let hits = engine::find_from(host.db(), to, &reqs);
                self.combine(host, hits, s.id == EXTEND, reqs)
            })(),
            FIND_MATCHING => (|| {
                self.commit(host)?;
                let f = s
                    .inline_field()
                    .or(host.window().field)
                    .ok_or(error::NO_MATCH)?;
                let (table, _, _) = Self::context(host);
                if host.db().table_of(f.to).map(|t| t.id) != Some(table) {
                    return Err(error::FIELD_MISSING);
                }
                let v = self.field_value(host, f).ok_or(error::RECORD_MISSING)?;
                let crit = if v.is_empty() {
                    "=".to_string()
                } else {
                    format!("=={v}")
                };
                let reqs = vec![Request {
                    criteria: vec![(f.field, crit)],
                    omit: false,
                }];
                let hits = engine::find(host.db(), table, &reqs);
                // ponytail: bytes 22–23 read as 2 constrain, 3 extend, as
                // the other window steps number their choices.
                match s.arg {
                    2 => self.combine(host, hits, false, reqs),
                    3 => self.combine(host, hits, true, reqs),
                    _ => self.found(host, hits, reqs),
                }
            })(),
            QUICK_FIND => (|| {
                self.commit(host)?;
                let text = match p.calc(0) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                let (table, _, _) = Self::context(host);
                // The layout's fields with "Include field for Quick Find" on.
                let layout = host.window().layout;
                let db = host.db();
                let fields: Vec<u32> = db
                    .layouts
                    .iter()
                    .filter(|l| l.id == layout)
                    .flat_map(|l| &l.objects)
                    .filter(|o| {
                        o.kind == 1
                            && o.field_flags & fmp12::layouts::field_flags::NO_QUICK_FIND == 0
                    })
                    .filter_map(|o| o.field)
                    .filter(|(to, _, _)| {
                        db.table_of((to & 0xFFF) as u32)
                            .is_some_and(|t| t.id == table)
                    })
                    .map(|(_, f, _)| f as u32)
                    .collect();
                let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
                if words.is_empty() {
                    return Err(error::NO_CRITERIA);
                }
                let db = host.db();
                let n = db.table(table).map_or(0, |t| t.records.len());
                let hits: Vec<usize> = (0..n)
                    .filter(|&r| {
                        let all: Vec<String> = fields
                            .iter()
                            .map(|f| db.value(table, r, *f).to_text().to_lowercase())
                            .collect();
                        words
                            .iter()
                            .all(|w| all.iter().any(|v| v.contains(w.as_str())))
                    })
                    .collect();
                // No match: error 401 and an empty found set (FileMaker 26).
                self.found(host, hits, Vec::new())
            })(),
            // Windows.
            NEW_WINDOW => self.new_window(host, frame, s, s.inline_id().filter(|_| s.arg == 5)),
            SELECT_WINDOW => (|| {
                if let Some(name) = self.window_name(host, frame, s)? {
                    self.select_window(host, &name)?;
                }
                Ok(Flow::Next)
            })(),
            CLOSE_WINDOW => (|| {
                if let Some(name) = self.window_name(host, frame, s)?
                    && !host.window().name.eq_ignore_ascii_case(&name)
                {
                    let i = self.window_index(&name).ok_or(err::WINDOW_MISSING)?;
                    self.windows.remove(i);
                    return Ok(Flow::Next);
                }
                self.commit(host)?;
                match self.windows.pop() {
                    Some(w) => {
                        *host.window() = w;
                        Ok(Flow::Next)
                    }
                    // The file's last window: FileMaker closes the file.
                    None => Ok(Flow::Suspend(
                        Suspend::Ui(UiRequest::CloseFile),
                        Pending::Ui(UiPending::Halt),
                    )),
                }
            })(),
            MOVE_RESIZE_WINDOW => (|| {
                let geo = self.geometry(host, frame, s)?;
                self.with_window(host, frame, s, |w| apply_geometry(w, geo))?;
                Ok(Flow::Next)
            })(),
            SET_WINDOW_TITLE => (|| {
                // ponytail: the new title read at position 1 ([129]); the
                // window name, when chosen by name, at position 0.
                let title = match p.calc(1) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => return Ok(Flow::Next),
                };
                self.with_window(host, frame, s, |w| w.name = title.clone())?;
                Ok(Flow::Next)
            })(),
            ADJUST_WINDOW => {
                let a = match s.arg {
                    2 => Adjust::Maximize,
                    3 => Adjust::Minimize,
                    4 => Adjust::Restore,
                    5 => Adjust::Hide,
                    _ => Adjust::ResizeToFit,
                };
                host.window().hidden = matches!(a, Adjust::Minimize | Adjust::Hide);
                ui(UiRequest::Adjust(a))
            }
            ARRANGE_WINDOWS | SCROLL_WINDOW | FLUSH_CACHE | SET_MULTI_USER => Ok(Flow::Next),
            SHOW_HIDE_TOOLBARS | SHOW_HIDE_MENUBAR | SHOW_HIDE_TEXT_RULER => {
                let w = host.window();
                let v = match s.id {
                    SHOW_HIDE_TOOLBARS => &mut w.toolbar,
                    SHOW_HIDE_MENUBAR => &mut w.menubar,
                    _ => &mut w.ruler,
                };
                // Bytes 22–23: 1 show, 2 hide, 3 toggle.
                *v = match s.arg {
                    1 => true,
                    2 => false,
                    _ => !*v,
                };
                Ok(Flow::Next)
            }
            VIEW_AS => {
                let w = host.window();
                w.view = match (s.arg, w.view) {
                    (1, _) | (4, View::Table) => View::Form,
                    (2, _) | (4, View::Form) => View::List,
                    _ => View::Table,
                };
                Ok(Flow::Next)
            }
            SET_ZOOM_LEVEL => {
                const LEVELS: [u32; 8] = [25, 50, 75, 100, 150, 200, 300, 400];
                let w = host.window();
                let at = LEVELS.iter().position(|z| *z >= w.zoom).unwrap_or(3);
                w.zoom = match s.arg {
                    k @ 1..=8 => LEVELS[k as usize - 1],
                    9 => LEVELS[(at + 1).min(7)],
                    10 => LEVELS[at.saturating_sub(1)],
                    _ => w.zoom,
                };
                Ok(Flow::Next)
            }
            // Files.
            NEW_FILE => ui(UiRequest::NewFile),
            OPEN_FILE => ui(UiRequest::OpenFile),
            // Closing the current file stops the script.
            CLOSE_FILE => Ok(Flow::Suspend(
                Suspend::Ui(UiRequest::CloseFile),
                Pending::Ui(UiPending::Halt),
            )),
            SAVE_A_COPY_AS => ui(UiRequest::SaveCopy {
                path: stored_path(p),
            }),
            // Recover File would rewrite the file from its blocks: refused.
            RECOVER_FILE => Err(error::UNAVAILABLE),
            PRINT_SETUP => ui(UiRequest::PrintSetup),
            PRINT => ui(UiRequest::Print { dialog }),
            SAVE_AS_PDF => ui(UiRequest::SavePdf {
                path: stored_path(p),
                dialog,
            }),
            SAVE_AS_EXCEL => ui(UiRequest::SaveExcel {
                path: stored_path(p),
                dialog,
            }),
            // Miscellaneous.
            OPEN_URL => (|| {
                let url = match p.calc(0) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                if url.trim().is_empty() {
                    return Err(err::COMMAND_INVALID);
                }
                ui(UiRequest::OpenUrl { url, dialog })
            })(),
            SEND_MAIL => (|| {
                // [128] To, [129] CC, [130] BCC, [131] subject, [132] message.
                let mut part = |i| -> Result<String, u32> {
                    Ok(match p.calc(i) {
                        Some(c) => self.eval(host, frame, &c)?.to_text(),
                        None => String::new(),
                    })
                };
                let to = part(0)?;
                let q: Vec<String> = [("cc", 1), ("bcc", 2), ("subject", 3), ("body", 4)]
                    .into_iter()
                    .filter_map(|(k, i)| match part(i) {
                        Ok(v) if !v.is_empty() => Some(Ok(format!("{k}={}", url_escape(&v)))),
                        Ok(_) => None,
                        Err(e) => Some(Err(e)),
                    })
                    .collect::<Result<_, u32>>()?;
                let mut url = format!("mailto:{}", url_escape(&to));
                if !q.is_empty() {
                    url = format!("{url}?{}", q.join("&"));
                }
                ui(UiRequest::OpenUrl { url, dialog })
            })(),
            GET_FOLDER_PATH => (|| {
                let name = p.text(0).ok_or(err::COMMAND_INVALID)?;
                // [131] dialog title, [132] default location.
                let title = match p.calc(3) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                let default = match p.calc(4) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                Ok(Flow::Suspend(
                    Suspend::Ui(UiRequest::FolderPath { title, default }),
                    Pending::Ui(UiPending::Variable(name)),
                ))
            })(),
            // Web viewers are not drawn: a named one is left as it is.
            SET_WEB_VIEWER => (|| {
                let name = match p.calc(0) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => String::new(),
                };
                let layout = host.window().layout;
                let found = host
                    .db()
                    .layouts
                    .iter()
                    .find(|l| l.id == layout)
                    .is_some_and(|l| l.objects.iter().any(|o| o.name == name));
                if found {
                    Ok(Flow::Next)
                } else {
                    Err(error::OBJECT_MISSING)
                }
            })(),
            // No speech, telephony, region monitoring or SQL data source
            // on Windows; FileMaker 26 returns these codes.
            SPEAK | DIAL_PHONE | REGION_MONITOR => Err(error::UNAVAILABLE),
            EXECUTE_SQL => Err(err::COMMAND_INVALID),
            ENABLE_TOUCH_KEYBOARD => Ok(Flow::Next),
            // No spelling dictionaries, web viewers or popovers are
            // drawn, and the layout is drawn every frame: nothing to do.
            CHECK_SELECTION
            | CHECK_RECORD
            | CHECK_FOUND_SET
            | SET_DICTIONARY
            | SET_USE_SYSTEM_FORMATS
            | REFRESH_OBJECT
            | CLOSE_POPOVER
            | REFRESH_PORTAL
            | FLUSH_WEB_COOKIES => Ok(Flow::Next),
            REVERT_ON_ERROR => {
                self.revert_on_error = o & bit::ON != 0;
                Ok(Flow::Next)
            }
            // Exit Application: BaseCraft closes the file.
            EXIT_APPLICATION => Ok(Flow::Suspend(
                Suspend::Ui(UiRequest::CloseFile),
                Pending::Ui(UiPending::Halt),
            )),
            DELETE_PORTAL_ROW => self.delete_portal_row(host, dialog),
            SORT_BY_FIELD => (|| {
                self.commit(host)?;
                let f = s
                    .inline_field()
                    .or(host.window().field)
                    .ok_or(error::FIELD_MISSING)?;
                // ponytail: ascending; the stored order (bytes 22–23 read
                // as 2 = descending) was not seen in a file.
                let o = if s.arg == 2 {
                    SortOrder::Descending
                } else {
                    SortOrder::Ascending
                };
                let (table, _, _) = Self::context(host);
                let mut found = host.window().found.clone();
                sort(host.db(), table, &mut found, &[(f, o)]);
                let fields = crate::sort_fields(host.db(), table, std::iter::once(f));
                let w = host.window();
                w.found = found;
                w.current = 0;
                w.sorted = true;
                w.sort = fields;
                Ok(Flow::Next)
            })(),
            // Transactions: every table as it was at Open Transaction. A
            // sub-script's Open and Commit (one already open) are skipped.
            OPEN_TRANSACTION => (|| {
                if self.transaction.is_some() {
                    return Err(error::UNAVAILABLE);
                }
                self.commit(host)?;
                let w = host.window().clone();
                self.transaction = Some(crate::Transaction {
                    tables: host.db().tables.clone(),
                    found: w.found,
                    current: w.current,
                    level: 0,
                });
                Ok(Flow::Next)
            })(),
            COMMIT_TRANSACTION => (|| {
                self.commit(host)?;
                self.transaction.take().ok_or(err::COMMAND_INVALID)?;
                Ok(Flow::Next)
            })(),
            // ponytail: the stored condition, error code and message are
            // read as parameters 0, 1, 2 (their dialog's order), not checked
            // against a FileMaker-written step.
            REVERT_TRANSACTION => (|| {
                if self.transaction.is_none() {
                    return Err(error::UNAVAILABLE);
                }
                let cond = p.calc(0);
                if cond.is_some() && !self.truth(host, frame, &cond)? {
                    return Ok(Flow::Next);
                }
                let code = match p.calc(1) {
                    Some(c) => self.eval(host, frame, &c)?.to_text().trim().parse().ok(),
                    None => None,
                }
                .filter(|c| (5000..=5499).contains(c));
                let detail = match (code, p.calc(2)) {
                    (Some(_), Some(c)) => self.eval(host, frame, &c)?.to_text(),
                    _ => String::new(),
                };
                let level = self.revert_transaction(host).unwrap_or(1);
                self.error_detail = detail;
                Ok(Flow::Reverted(level, code.unwrap_or(0)))
            })(),
            SET_ALLOWED_ORIENTATIONS => Err(err::UNKNOWN_COMMAND),
            // iOS notifications and AI accounts: 5 with no parameters
            // (FileMaker 26 on Windows), 3 otherwise.
            LOCAL_NOTIFICATION | CONFIGURE_AI_ACCOUNT => Err(if !has_params(p) {
                err::COMMAND_INVALID
            } else {
                error::UNAVAILABLE
            }),
            id if UNSUPPORTED.contains(&id) => Err(error::UNAVAILABLE),
            _ => return None,
        };
        Some(r)
    }

    /// Delete Portal Row: the related record of the current portal row.
    fn delete_portal_row(&mut self, host: &mut dyn Host, dialog: bool) -> Result<Flow, u32> {
        let (pto, related) = self.active_portal(host)?;
        let row = host.window().portal_row;
        let r = *row
            .checked_sub(1)
            .and_then(|i| related.get(i))
            .ok_or(error::RECORD_MISSING)?;
        let table = host.db().table_of(pto).ok_or(err::TABLE_MISSING)?.id;
        if dialog {
            return Ok(confirm_delete(
                "Permanently delete this related record?",
                table,
                vec![r],
            ));
        }
        delete_records(host, table, &[r]);
        host.window().portal_row = 0;
        Ok(Flow::Next)
    }

    /// The portal holding the active field, or the layout's first portal:
    /// its occurrence and the current record's related records.
    fn active_portal(&mut self, host: &mut dyn Host) -> Result<(u32, Vec<usize>), u32> {
        let (_, to, rec) = Self::context(host);
        let w = host.window().clone();
        let db = host.db();
        let layout = db
            .layouts
            .iter()
            .find(|l| l.id == w.layout)
            .ok_or(error::LAYOUT_MISSING)?;
        let portals: Vec<&fmp12::layouts::LayoutObject> = layout
            .objects
            .iter()
            .filter(|o| o.portal.is_some())
            .collect();
        let in_portal = |o: &&fmp12::layouts::LayoutObject| {
            w.field.is_some_and(|f| {
                layout.objects.iter().any(|x| {
                    x.parent == Some(o.id)
                        && x.field.is_some_and(|(t, fid, _)| {
                            (t & 0xFFF) as u32 == f.to && fid as u32 == f.field
                        })
                })
            })
        };
        // The active field's portal; a field outside any portal: 3
        // (FileMaker 26); no active field: the first portal.
        let portal = match w.field {
            Some(_) => portals.iter().find(|o| in_portal(o)),
            None => portals.first(),
        }
        .ok_or(error::UNAVAILABLE)?;
        let pto = portal.portal.map_or(0, |p| (p.0 & 0xFFF) as u32);
        let related = rec.map_or(Vec::new(), |r| db.related(to, r, pto));
        Ok((pto, related))
    }

    /// Go to Portal Row: the portal holding the active field, or the
    /// layout's first portal.
    fn go_to_portal_row(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Flow, u32> {
        self.commit(host)?;
        let (_, related) = self.active_portal(host)?;
        let rows = related.len();
        // No related records: nothing to go to, no error (FileMaker 26).
        if rows == 0 {
            return Ok(Flow::Next);
        }
        let w = host.window().clone();
        let cur = w.portal_row;
        let exit_after_last = s.options & bit::EXIT_AFTER_LAST != 0;
        let to_row = match s.arg {
            1 => 1,
            2 => rows,
            3 | 4 => {
                let next = if s.arg == 3 {
                    cur.checked_sub(1).filter(|r| *r >= 1)
                } else {
                    Some(cur + 1).filter(|r| *r <= rows)
                };
                match next {
                    Some(r) => r,
                    None if exit_after_last => return Ok(Flow::Next),
                    None => return Err(error::RECORD_MISSING),
                }
            }
            _ => {
                let n = match s.raw_params.calc(0) {
                    Some(c) => self
                        .eval(host, frame, &c)?
                        .to_text()
                        .trim()
                        .parse::<f64>()
                        .unwrap_or(0.0),
                    None => 0.0,
                };
                (n as usize).clamp(1, rows)
            }
        };
        host.window().portal_row = to_row;
        Ok(Flow::Next)
    }

    /// Go to Related Record: `[128]` key 17 = the related occurrence;
    /// the inline layout (bytes 22–23 = 5) or the current one (1).
    fn go_to_related(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Flow, u32> {
        self.commit(host)?;
        let p = &s.raw_params;
        let target = p
            .get(&[128], 17)
            .and_then(|b| fmp12::graph::to_ref(&b))
            .ok_or(err::TABLE_MISSING)?;
        let (_, to, rec) = Self::context(host);
        let w = host.window().clone();
        let db = host.db();
        let tt = db.table_of(target).ok_or(err::TABLE_MISSING)?.id;
        let layout = match s.inline_id().filter(|_| s.arg == 5) {
            Some(l) => l,
            None => w.layout,
        };
        if layout_table(db, layout).map(|(_, t)| t.id) != Some(tt) {
            return Err(err::WRONG_LAYOUT);
        }
        let from: Vec<usize> = if s.options & bit::MATCH_ALL != 0 {
            w.found.clone()
        } else {
            rec.into_iter().collect()
        };
        let mut related: Vec<usize> = from
            .iter()
            .flat_map(|&r| db.related(to, r, target))
            .collect();
        related.sort_unstable();
        related.dedup();
        if related.is_empty() {
            return Err(error::RECORD_MISSING);
        }
        if s.options & bit::NEW_WINDOW != 0 {
            self.new_window(host, frame, s, None)?;
        }
        self.go_to_layout(host, layout)?;
        let n = host.db().table(tt).map_or(0, |t| t.records.len());
        let w = host.window();
        if s.options & bit::ONLY_RELATED != 0 {
            w.found = related;
            w.current = 0;
        } else {
            w.found = (0..n).collect();
            w.current = related[0];
        }
        w.sorted = false;
        w.sort.clear();
        w.mode = Mode::Browse;
        Ok(Flow::Next)
    }

    /// Cut, Copy, Clear: the step's field or the active one; Copy with
    /// neither copies the record.
    fn cut_copy_clear(&mut self, host: &mut dyn Host, s: &Step) -> Result<Flow, u32> {
        let f = s.inline_field().or(host.window().field);
        let Some(f) = f else {
            if s.id == sid::COPY {
                return self.copy_records(host, false);
            }
            return Err(error::UNAVAILABLE);
        };
        let v = self.field_value(host, f).ok_or(error::RECORD_MISSING)?;
        if s.id != sid::CLEAR {
            self.clipboard = Some(v.clone());
            host.set_clipboard(v);
        }
        if s.id != sid::COPY {
            self.set_field(host, f, Value::text(""))?;
        }
        Ok(Flow::Next)
    }

    /// Copy Record (`all` = Copy All Records): the layout's fields of the
    /// table, tab-separated, one record per line.
    fn copy_records(&mut self, host: &mut dyn Host, all: bool) -> Result<Flow, u32> {
        let (table, _, rec) = Self::context(host);
        let fields = layout_fields(host);
        let records = if all {
            host.window().found.clone()
        } else {
            vec![rec.ok_or(error::RECORD_MISSING)?]
        };
        let db = host.db();
        let text = records
            .iter()
            .map(|&r| {
                fields
                    .iter()
                    .filter(|f| db.table_of(f.to).is_some_and(|t| t.id == table))
                    .map(|f| db.value(table, r, f.field).to_text().replace('\r', "\u{b}"))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\r");
        self.clipboard = Some(text.clone());
        host.set_clipboard(text);
        Ok(Flow::Next)
    }

    fn undo_edit(&mut self, host: &mut dyn Host) {
        let Some((t, r, Some(old))) = self.open.clone() else {
            return;
        };
        if let Some(tb) = host.db().tables.iter_mut().find(|x| x.id == t)
            && let Some(rec) = tb.records.get_mut(r)
        {
            self.redo = Some((t, r, std::mem::replace(rec, old)));
        }
    }

    fn redo_edit(&mut self, host: &mut dyn Host) {
        let Some((t, r, new)) = self.redo.take() else {
            return;
        };
        if let Some(tb) = host.db().tables.iter_mut().find(|x| x.id == t)
            && let Some(rec) = tb.records.get_mut(r)
        {
            *rec = new;
        }
    }

    /// The found set becomes `hits` (Browse mode); error 401 when empty.
    fn found(
        &mut self,
        host: &mut dyn Host,
        hits: Vec<usize>,
        reqs: Vec<Request>,
    ) -> Result<Flow, u32> {
        let w = host.window();
        w.mode = Mode::Browse;
        w.requests.clear();
        if !reqs.is_empty() {
            w.last_find = reqs;
        }
        w.found = hits;
        w.current = 0;
        w.sorted = false;
        w.sort.clear();
        if w.found.is_empty() {
            return Err(error::NO_MATCH);
        }
        Ok(Flow::Next)
    }

    /// Constrain (found set ∩ hits) or extend (found set ∪ hits).
    fn combine(
        &mut self,
        host: &mut dyn Host,
        hits: Vec<usize>,
        extend: bool,
        reqs: Vec<Request>,
    ) -> Result<Flow, u32> {
        let found = host.window().found.clone();
        let set: Vec<usize> = if extend {
            let mut v = found.clone();
            v.extend(hits.iter().filter(|r| !found.contains(r)));
            v
        } else {
            found.into_iter().filter(|r| hits.contains(r)).collect()
        };
        self.found(host, set, reqs)
    }

    /// Window chosen by a step: `Some(name)` (bytes 22–23 = 1, name at
    /// `[128]`), `None` for the current window.
    pub(crate) fn window_name(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Option<String>, u32> {
        if s.arg != 1 {
            return Ok(None);
        }
        match s.raw_params.calc(0) {
            Some(c) => Ok(Some(self.eval(host, frame, &c)?.to_text())),
            None => Err(err::WINDOW_MISSING),
        }
    }

    fn window_index(&self, name: &str) -> Option<usize> {
        self.windows
            .iter()
            .rposition(|w| w.name.eq_ignore_ascii_case(name))
    }

    /// Brings the window named `name` to the front.
    pub(crate) fn select_window(&mut self, host: &mut dyn Host, name: &str) -> Result<(), u32> {
        if host.window().name.eq_ignore_ascii_case(name) {
            return Ok(());
        }
        let i = self.window_index(name).ok_or(err::WINDOW_MISSING)?;
        self.commit(host)?;
        let w = self.windows.remove(i);
        let old = std::mem::replace(host.window(), w);
        self.windows.push(old);
        Ok(())
    }

    /// Runs `f` on the window a step chooses (current or by name).
    fn with_window(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
        f: impl FnOnce(&mut Window),
    ) -> Result<(), u32> {
        match self.window_name(host, frame, s)? {
            Some(n) if !host.window().name.eq_ignore_ascii_case(&n) => {
                let i = self.window_index(&n).ok_or(err::WINDOW_MISSING)?;
                f(&mut self.windows[i]);
            }
            _ => f(host.window()),
        }
        Ok(())
    }

    /// Height, width, top, left at positions 1–4 (`[129]`…`[132]`).
    fn geometry(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<[Option<f64>; 4], u32> {
        let mut g = [None; 4];
        for (i, v) in g.iter_mut().enumerate() {
            if let Some(c) = s.raw_params.calc(1 + i as u32) {
                *v = self.eval(host, frame, &c)?.to_text().trim().parse().ok();
            }
        }
        Ok(g)
    }

    /// New Window (and Go to Related Record in a new window): name at
    /// `[128]`, geometry at `[129…132]`, style in `[128]` key 3.
    fn new_window(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
        layout: Option<u32>,
    ) -> Result<Flow, u32> {
        self.commit(host)?;
        let name = match s.raw_params.calc(0) {
            Some(c) => self.eval(host, frame, &c)?.to_text(),
            None => String::new(),
        };
        let geo = self.geometry(host, frame, s)?;
        let base = host.window().name.clone();
        let name = if name.is_empty() {
            let n = self.windows.len() + 2;
            let base = base.rsplit_once(" - ").map_or(base.as_str(), |b| b.0);
            format!("{base} - {n}")
        } else {
            name
        };
        let mut w = host.window().clone();
        w.name = name;
        w.field = None;
        w.object.clear();
        w.portal_row = 0;
        w.hidden = false;
        apply_geometry(&mut w, geo);
        let old = std::mem::replace(host.window(), w);
        self.windows.push(old);
        if let Some(l) = layout {
            self.go_to_layout(host, l)?;
        }
        Ok(Flow::Next)
    }
}

fn apply_geometry(w: &mut Window, [h, wd, t, l]: [Option<f64>; 4]) {
    if let Some(h) = h {
        w.height = h;
    }
    if let Some(x) = wd {
        w.width = x;
    }
    if let Some(t) = t {
        w.top = t;
    }
    if let Some(l) = l {
        w.left = l;
    }
}

/// Whether a step stores any parameter node (`[128 + p]`).
pub(crate) fn has_params(p: &fmp12::scripts::RawParams) -> bool {
    p.kv.keys()
        .any(|(path, _)| path.first().is_some_and(|x| *x >= 128))
}

/// A step that only the application can do and that goes on when done.
fn ui(r: UiRequest) -> Result<Flow, u32> {
    Ok(Flow::Suspend(Suspend::Ui(r), Pending::Ui(UiPending::Done)))
}

/// Fields of the current layout in object order (FileMaker's default tab
/// order; ponytail: a custom tab order is not decoded).
fn layout_fields(host: &mut dyn Host) -> Vec<FieldRef> {
    let layout = host.window().layout;
    host.db()
        .layouts
        .iter()
        .find(|l| l.id == layout)
        .map(|l| {
            l.objects
                .iter()
                .filter(|o| o.kind == 1)
                .filter_map(|o| o.field)
                .map(|(t, f, r)| FieldRef {
                    to: (t & 0xFFF) as u32,
                    field: f as u32,
                    repetition: r.max(1) as u16,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Percent-encodes a `mailto:` part.
fn url_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'@' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
