//! Data file steps (188–199), PDF documents (242–247), Go to List of
//! Records, Configure Persistent Data and the container inserts added
//! after M5. Storage and behaviour as FileMaker 26 showed them (FORMAT.md
//! §8, `tools/oracle/datafiles`): a path list in `[128]` key 130 (option
//! `0x20`), the file id at parameter 0, a second number at parameter 1
//! (option `0x8000`), the target a variable (`[160]` key 1) or the inline
//! field, the character set in bytes 22–23.

use std::collections::BTreeMap;

use super::*;
use crate::files::{CONFIRM, FILE_MISSING, decode_path_list};

pub const INSERT_OBJECT: u16 = 78;
pub const INSERT_PDF: u16 = 158;
pub const INSERT_AUDIO_VIDEO: u16 = 159;
pub const GET_FILE_EXISTS: u16 = 188;
pub const GET_FILE_SIZE: u16 = 189;
pub const CREATE_DATA_FILE: u16 = 190;
pub const OPEN_DATA_FILE: u16 = 191;
pub const WRITE_DATA_FILE: u16 = 192;
pub const READ_DATA_FILE: u16 = 193;
pub const GET_DATA_FILE_POSITION: u16 = 194;
pub const SET_DATA_FILE_POSITION: u16 = 195;
pub const CLOSE_DATA_FILE: u16 = 196;
pub const DELETE_FILE: u16 = 197;
pub const RENAME_FILE: u16 = 199;
pub const GO_TO_LIST_OF_RECORDS: u16 = 228;
pub const CONFIGURE_PERSISTENT_DATA: u16 = 238;
pub const PRINT_PDF: u16 = 242;
pub const CREATE_PDF: u16 = 243;
pub const APPEND_PDF: u16 = 244;
pub const CLOSE_PDF: u16 = 245;
pub const OPEN_PDF: u16 = 246;
pub const CANCEL_PDF: u16 = 247;

/// FileMaker 26's error codes for these steps.
mod err {
    pub const COMMAND_UNKNOWN: u32 = 4;
    pub const COMMAND_INVALID: u32 = 5;
    /// Requested data is missing: a read past the end of the file.
    pub const DATA_MISSING: u32 = 10;
    /// Invalid value: a negative position.
    pub const OUT_OF_RANGE: u32 = 14;
    /// File is locked or in use: Open Data File on an open file.
    pub const IN_USE: u32 = 300;
    /// Append PDF with no PDF open; Create / Open PDF with one open.
    pub const NO_PDF: u32 = 829;
    pub const PDF_OPEN: u32 = 833;
}

/// Option bits (record bytes 24–27).
const CREATE_FOLDERS: u32 = 0x200;
const APPEND_LINE_FEED: u32 = 0x200;
/// The step holds its second number (position, count).
const SECOND: u32 = 0x8000;

/// An open data file: its FileMaker path and read-write position.
#[derive(Debug, Clone)]
pub(crate) struct DataFile {
    path: String,
    pos: usize,
}

/// What the engine keeps for these steps.
#[derive(Debug, Default)]
pub struct State {
    files: BTreeMap<u32, DataFile>,
    last_id: u32,
    /// Configure Persistent Data: name → value.
    pub persistent: BTreeMap<String, String>,
    /// The PDF being built (Create / Open PDF): the documents appended.
    pdf: Option<Vec<Vec<u8>>>,
}

/// Whether `id` is one of the steps run here.
pub fn handles(id: u16) -> bool {
    matches!(id, 78 | 158 | 159 | 188..=197 | 199 | 228 | 238 | 242..=247)
}

/// A step's target: the variable in `[160]` key 1 or the inline field.
fn target(s: &Step) -> Target {
    match s.raw_params.text(32) {
        Some(v) if !v.is_empty() => Target::Variable(v),
        _ => s.inline_field().map_or(Target::None, Target::Field),
    }
}

/// The step waits for the user to agree to a write outside the folder.
fn ask(path: &str) -> Flow {
    Flow::Suspend(
        Suspend::Ui(UiRequest::ConfirmFile {
            path: path.to_string(),
        }),
        Pending::Ui(UiPending::Retry(path.to_string())),
    )
}

/// `Err(CONFIRM)` becomes the confirmation, other errors stay.
fn write_result(r: Result<(), u32>, path: &str) -> Result<Flow, u32> {
    match r {
        Ok(()) => Ok(Flow::Next),
        Err(CONFIRM) => Ok(ask(path)),
        Err(e) => Err(e),
    }
}

fn encode(text: &str, charset: u16) -> Vec<u8> {
    if charset == 2 {
        return text.as_bytes().to_vec();
    }
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// The new path of Rename File: `name` in the folder of `path`.
fn renamed(path: &str, name: &str) -> String {
    match path.rfind(['/', ':']) {
        Some(i) => format!("{}{name}", &path[..=i]),
        None => name.to_string(),
    }
}

impl Engine {
    /// Runs the steps of this module.
    pub(crate) fn exec_files(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Flow, u32> {
        let o = s.options;
        match s.id {
            // Insert Object (OLE) is obsolete in FileMaker 26: 4.
            INSERT_OBJECT => Err(err::COMMAND_UNKNOWN),
            INSERT_PDF | INSERT_AUDIO_VIDEO => {
                // No field in the step and none active: 3 (FileMaker 26).
                let f = s
                    .inline_field()
                    .or(host.window().field)
                    .ok_or(error::UNAVAILABLE)?;
                let (name, data) = host.pick_file(false).ok_or(error::USER_CANCELED)?;
                let file = calc::value::Container {
                    name,
                    data,
                    ..Default::default()
                };
                self.set_field(host, f, Value::Container(file))?;
                Ok(Flow::Next)
            }
            GET_FILE_EXISTS | GET_FILE_SIZE => {
                let path = self.paths(frame, s).into_iter().next().unwrap_or_default();
                let stat = host.file_stat(&path)?.filter(|x| !x.folder);
                let v = match (s.id, stat) {
                    (GET_FILE_EXISTS, st) => num(st.is_some() as usize),
                    (_, Some(st)) => num(st.size as usize),
                    (_, None) => return Err(FILE_MISSING),
                };
                self.put(host, frame, s, v)?;
                Ok(Flow::Next)
            }
            CREATE_DATA_FILE => {
                let path = self.paths(frame, s).into_iter().next().unwrap_or_default();
                let r = host.file_write(&path, &[], o & CREATE_FOLDERS != 0);
                write_result(r, &path)
            }
            OPEN_DATA_FILE => {
                let mut found = None;
                for path in self.paths(frame, s) {
                    if host.file_stat(&path)?.is_some_and(|x| !x.folder) {
                        found = Some(path);
                        break;
                    }
                }
                let path = found.ok_or(FILE_MISSING)?;
                if self.more.files.values().any(|f| f.path == path) {
                    return Err(err::IN_USE);
                }
                self.more.last_id += 1;
                let id = self.more.last_id;
                self.more.files.insert(id, DataFile { path, pos: 0 });
                self.put(host, frame, s, num(id as usize))?;
                Ok(Flow::Next)
            }
            WRITE_DATA_FILE => {
                let id = self.file_id(host, frame, s)?;
                let v = match target(s) {
                    Target::Variable(n) => variable(&self.globals, frame, &n),
                    Target::Field(f) => {
                        Value::text(self.field_value(host, f).ok_or(error::RECORD_MISSING)?)
                    }
                    Target::None => Value::empty(),
                };
                let mut bytes = match (&v, s.arg) {
                    (Value::Container(c), 0 | 3) => c.data.clone(),
                    _ => encode(&v.to_text(), s.arg),
                };
                if o & APPEND_LINE_FEED != 0 {
                    bytes.extend(encode("\n", s.arg));
                }
                let f = self.more.files.get(&id).ok_or(FILE_MISSING)?.clone();
                let mut data = host.file_read(&f.path)?;
                if data.len() < f.pos {
                    data.resize(f.pos, 0);
                }
                let end = (f.pos + bytes.len()).min(data.len());
                data.splice(f.pos..end, bytes.iter().copied());
                if let Err(e) = host.file_write(&f.path, &data, false) {
                    return write_result(Err(e), &f.path);
                }
                if let Some(x) = self.more.files.get_mut(&id) {
                    x.pos = f.pos + bytes.len();
                }
                Ok(Flow::Next)
            }
            READ_DATA_FILE => {
                let id = self.file_id(host, frame, s)?;
                let f = self.more.files.get(&id).ok_or(FILE_MISSING)?.clone();
                let data = host.file_read(&f.path)?;
                // No count: the whole file, whatever the position.
                let (from, want) = match self.second(host, frame, s)? {
                    None => (0, None),
                    Some(n) => (f.pos, Some(n.max(0.0) as usize)),
                };
                let unit = if s.arg == 1 { 2 } else { 1 };
                let len = want.map_or(data.len().saturating_sub(from), |n| n * unit);
                let start = from.min(data.len());
                let end = (from + len).min(data.len());
                let chunk = &data[start..end];
                let v = match s.arg {
                    1 => Value::text(String::from_utf16_lossy(
                        &chunk
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|c| u16::from_le_bytes(*c))
                            .collect::<Vec<_>>(),
                    )),
                    2 => Value::text(String::from_utf8_lossy(chunk).into_owned()),
                    _ => {
                        let name = f.path.rsplit(['/', ':', '\\']).next().unwrap_or_default();
                        Value::Container(calc::value::Container {
                            name: name.to_string(),
                            data: chunk.to_vec(),
                            ..Default::default()
                        })
                    }
                };
                self.put(host, frame, s, v)?;
                if let Some(x) = self.more.files.get_mut(&id) {
                    x.pos = end;
                }
                if want.is_some() && end - start < len {
                    return Err(err::DATA_MISSING);
                }
                Ok(Flow::Next)
            }
            GET_DATA_FILE_POSITION => {
                let id = self.file_id(host, frame, s)?;
                let pos = self.more.files.get(&id).ok_or(FILE_MISSING)?.pos;
                self.put(host, frame, s, num(pos))?;
                Ok(Flow::Next)
            }
            SET_DATA_FILE_POSITION => {
                let id = self.file_id(host, frame, s)?;
                self.more.files.get(&id).ok_or(FILE_MISSING)?;
                let n = self.second(host, frame, s)?.unwrap_or(0.0);
                if n < 0.0 {
                    return Err(err::OUT_OF_RANGE);
                }
                if let Some(x) = self.more.files.get_mut(&id) {
                    x.pos = n as usize;
                }
                Ok(Flow::Next)
            }
            CLOSE_DATA_FILE => {
                let id = self.file_id(host, frame, s)?;
                self.more.files.remove(&id).ok_or(FILE_MISSING)?;
                Ok(Flow::Next)
            }
            DELETE_FILE => {
                let path = self.paths(frame, s).into_iter().next().unwrap_or_default();
                write_result(host.file_delete(&path), &path)
            }
            RENAME_FILE => {
                let from = self.paths(frame, s).into_iter().next().unwrap_or_default();
                let name = match s.raw_params.calc(0) {
                    Some(c) => self.eval(host, frame, &c)?.to_text(),
                    None => return Err(err::COMMAND_INVALID),
                };
                let to = renamed(&from, name.trim());
                match host.file_rename(&from, &to) {
                    // Both paths must be agreed to; ask for the new one too.
                    Err(CONFIRM) => Ok(ask(&from)),
                    r => r.map(|_| Flow::Next),
                }
            }
            GO_TO_LIST_OF_RECORDS => self.go_to_list(host, frame, s),
            CONFIGURE_PERSISTENT_DATA => {
                // [128] key 1 the name (required: 5), parameter 1 the value.
                let name = s
                    .raw_params
                    .text(0)
                    .filter(|n| !n.is_empty())
                    .ok_or(err::COMMAND_INVALID)?;
                let v = match s.raw_params.calc(1) {
                    Some(c) => Some(self.eval(host, frame, &c)?.to_text()),
                    None => None,
                };
                match v {
                    Some(v) => self.more.persistent.insert(name, v),
                    None => self.more.persistent.remove(&name),
                };
                Ok(Flow::Next)
            }
            CREATE_PDF => {
                if self.more.pdf.is_some() {
                    return Err(err::PDF_OPEN);
                }
                self.more.pdf = Some(Vec::new());
                Ok(Flow::Next)
            }
            OPEN_PDF => {
                if self.more.pdf.is_some() {
                    return Err(err::PDF_OPEN);
                }
                let doc = self.pdf_source(host, frame, s)?;
                self.more.pdf = Some(vec![doc]);
                Ok(Flow::Next)
            }
            APPEND_PDF => {
                if self.more.pdf.is_none() {
                    return Err(err::NO_PDF);
                }
                let doc = self.pdf_source(host, frame, s)?;
                print::merge::pages(&doc).map_err(|_| err::COMMAND_INVALID)?;
                if let Some(p) = self.more.pdf.as_mut() {
                    p.push(doc);
                }
                Ok(Flow::Next)
            }
            CLOSE_PDF => {
                let docs = self.more.pdf.clone().ok_or(err::COMMAND_INVALID)?;
                let parts: Vec<&[u8]> = docs.iter().map(Vec::as_slice).collect();
                let pdf = print::merge::merge(&parts).map_err(|_| err::COMMAND_INVALID)?;
                // Root key 139: 0 a file, 1 the target (field or variable).
                if self.from(s) == 1 {
                    let v = Value::Container(calc::value::Container {
                        name: "Untitled.pdf".into(),
                        data: pdf,
                        ..Default::default()
                    });
                    self.put(host, frame, s, v)?;
                    self.more.pdf = None;
                    return Ok(Flow::Next);
                }
                let path = self.paths(frame, s).into_iter().next();
                let Some(path) = path else {
                    self.more.pdf = None;
                    return ui_done(UiRequest::PdfDocument {
                        path: None,
                        data: pdf,
                        print: false,
                    });
                };
                let r = host.file_write(&path, &pdf, o & CREATE_FOLDERS != 0);
                if r.is_ok() {
                    self.more.pdf = None;
                }
                write_result(r, &path)
            }
            CANCEL_PDF => {
                self.more.pdf = None;
                Ok(Flow::Next)
            }
            PRINT_PDF => {
                let data = self.pdf_source(host, frame, s)?;
                ui_done(UiRequest::PdfDocument {
                    path: None,
                    data,
                    print: true,
                })
            }
            _ => Err(error::UNAVAILABLE),
        }
    }

    /// The step's path list, variables (`$path`) replaced by their value;
    /// one entry per line.
    fn paths(&self, frame: &Frame, s: &Step) -> Vec<String> {
        let list = s
            .raw_params
            .get(&[128], 130)
            .map(|b| decode_path_list(&b))
            .unwrap_or_default();
        list.into_iter()
            .flat_map(|p| {
                let p = if p.starts_with('$') {
                    variable(&self.globals, frame, &p).to_text()
                } else {
                    p
                };
                crate::files::candidates(&p)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The file id at parameter 0 (an unknown id: 100).
    fn file_id(&mut self, host: &mut dyn Host, frame: &mut Frame, s: &Step) -> Result<u32, u32> {
        let c = s.raw_params.calc(0).ok_or(FILE_MISSING)?;
        let v = self.eval(host, frame, &c)?.to_text();
        v.trim().parse().map_err(|_| FILE_MISSING)
    }

    /// The second number (parameter 1), when option `0x8000` says it is set.
    fn second(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Option<f64>, u32> {
        if s.options & SECOND == 0 {
            return Ok(None);
        }
        match s.raw_params.calc(1) {
            Some(c) => Ok(Some(
                self.eval(host, frame, &c)?
                    .to_text()
                    .trim()
                    .parse()
                    .unwrap_or(0.0),
            )),
            None => Ok(None),
        }
    }

    /// Root key 139 of the PDF steps: 0 a file, 1 a field or variable.
    fn from(&self, s: &Step) -> u32 {
        s.raw_params
            .get(&[], 139)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_be_bytes)
    }

    /// The PDF a step reads: its file, or its field or variable.
    fn pdf_source(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Vec<u8>, u32> {
        if s.id != PRINT_PDF && self.from(s) == 1 {
            let v = match target(s) {
                Target::Variable(n) => variable(&self.globals, frame, &n),
                Target::Field(f) => {
                    let (table, _, rec) = Self::context(host);
                    let rec = rec.ok_or(error::RECORD_MISSING)?;
                    host.db()
                        .value_rep(table, rec, f.field, f.repetition.max(1) as u32)
                }
                Target::None => return Err(err::COMMAND_INVALID),
            };
            return match v {
                Value::Container(c) if !c.data.is_empty() => Ok(c.data),
                _ => Err(err::COMMAND_INVALID),
            };
        }
        let mut last = FILE_MISSING;
        for path in self.paths(frame, s) {
            match host.file_read(&path) {
                Ok(b) => return Ok(b),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Stores `v` in the step's target (variable or field).
    fn put(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
        v: Value,
    ) -> Result<(), u32> {
        match target(s) {
            Target::Variable(name) => {
                let key = (name.to_lowercase(), 1);
                if name.starts_with("$$") {
                    self.globals.insert(key, v);
                } else {
                    frame.locals.insert(key, v);
                }
                Ok(())
            }
            Target::Field(f) => self.set_field(host, f, v),
            Target::None => Ok(()),
        }
    }

    /// Go to List of Records: the record ids of parameter 7, in their
    /// order, on the inline layout (bytes 22–23 = 5) or the current one.
    /// Ids that match no record: 101 (the others are shown); none: 401.
    fn go_to_list(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<Flow, u32> {
        self.commit(host)?;
        let list = match s.raw_params.calc(7) {
            Some(c) => self.eval(host, frame, &c)?.to_text(),
            None => String::new(),
        };
        if let Some(l) = s.inline_id().filter(|_| s.arg == 5) {
            self.go_to_layout(host, l)?;
        }
        let (table, _, _) = Self::context(host);
        let ids: Vec<u32> = list
            .split(['\r', '\n'])
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        let db = host.db();
        let records = db.table(table).map(|t| t.records.as_slice()).unwrap_or(&[]);
        let mut found: Vec<usize> = Vec::new();
        let mut missing = list.split(['\r', '\n']).any(|v| {
            let v = v.trim();
            !v.is_empty() && v.parse::<u32>().is_err()
        });
        for id in ids {
            match records.iter().position(|r| r.id == id) {
                Some(i) if !found.contains(&i) => found.push(i),
                Some(_) => {}
                None => missing = true,
            }
        }
        let w = host.window();
        w.mode = Mode::Browse;
        w.found = found;
        w.current = 0;
        w.sorted = false;
        match (w.found.is_empty(), missing) {
            (true, _) => Err(error::NO_MATCH),
            (false, true) => Err(error::RECORD_MISSING),
            _ => Ok(Flow::Next),
        }
    }
}

/// A variable's value (`$local` in the frame, `$$global` in the engine).
fn variable(globals: &Vars, frame: &Frame, name: &str) -> Value {
    let key = (name.to_lowercase(), 1);
    let vars = if name.starts_with("$$") {
        globals
    } else {
        &frame.locals
    };
    vars.get(&key).cloned().unwrap_or_else(Value::empty)
}

fn ui_done(r: UiRequest) -> Result<Flow, u32> {
    Ok(Flow::Suspend(Suspend::Ui(r), Pending::Ui(UiPending::Done)))
}
