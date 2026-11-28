//! Stored options of Import Records, Export Records, Insert Picture and
//! Insert File (FORMAT.md §8, decoded from steps FileMaker Pro 26 wrote),
//! and those steps run without a dialog when they name a file.
//!
//! * Import Records: bytes 22–23 the source (1 a file, 2 a folder, 4 XML);
//!   step root key 132 = `len TO | action | options | charset | u16 n |
//!   n × (len 01 field, kind)`, one entry per source column, kind 0
//!   import, 1 skip, 2 match; root key 240 = `00 | u16 n | n × (len 01
//!   field, u8)` (a per-field option, not used).
//! * Export Records: root key 133 = `u16 n | n × (len TO, len 01 field,
//!   u16, len 01 id, len 01 summary field, u16) | u16 g | g × (len TO,
//!   len 01 field, u16, u8 selected) | formatting | charset | 01`.
//! * Insert Picture / Insert File: `0x800` store only a reference; Insert
//!   File's dialog options at [128] key 3 (storage), [129] key 3
//!   (compression), [130] key 3 (display), `0x200` when they are set.

use super::*;
use crate::datafiles::write_result;
use engine::export::Format;
use fmp12::scripts::RawParams;

/// Option bits (record bytes 24–27).
pub const NO_DIALOG: u32 = 0x80;
/// Export Records: create folders.
pub const CREATE_FOLDERS: u32 = 0x200;
/// Insert Picture / Insert File: store only a reference.
pub const REFERENCE: u32 = 0x800;

/// Import options byte (root key 132, byte 6).
pub mod import_opt {
    pub const SPLIT_REPETITIONS: u8 = 0x01;
    pub const AUTO_ENTER: u8 = 0x02;
    pub const MATCH_NAMES: u8 = 0x04;
    pub const ADD_REMAINING: u8 = 0x08;
    pub const KEEP_CONTAINERS: u8 = 0x10;
    pub const KEEP_ORIGINAL: u8 = 0x20;
}

/// Import Records' stored mapping.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportSpec {
    /// The occurrence the records go to.
    pub to: u32,
    /// 0 add, 1 update existing records (XML "Replace"), 2 update
    /// matching records (XML "Update").
    pub action: u8,
    /// [`import_opt`] bits.
    pub options: u8,
    /// 0 Windows ANSI, 1 DOS, 2 Mac Roman, 3 Unicode (UTF-16), 5 UTF-8.
    pub charset: u8,
    /// Per source column: the target field and 0 import, 1 skip, 2 match.
    pub map: Vec<(u32, u8)>,
}

/// Export Records' stored order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExportSpec {
    /// (occurrence, field) in export order.
    pub fields: Vec<(u32, u32)>,
    /// Group-by fields (occurrence, field), with whether each is ticked.
    pub group_by: Vec<(u32, u32, bool)>,
    /// "Apply current layout's data formatting".
    pub formatted: bool,
    /// 0 Windows ANSI, 1 DOS, 2 Macintosh, 3 UTF-16, 4 UTF-8.
    pub charset: u8,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    /// `len bytes`.
    fn chunk(&mut self) -> Option<&[u8]> {
        let n = self.u8()? as usize;
        let b = self.0.get(..n)?;
        self.0 = &self.0[n..];
        Some(b)
    }
    /// `len 03 D0 hi lo`.
    fn to(&mut self) -> Option<u32> {
        fmp12::graph::to_ref(self.chunk()?)
    }
    /// `len 01 path-int` (0 for `02 01 00`).
    fn field(&mut self) -> Option<u32> {
        fmp12::chunk::path_int(self.chunk()?.get(1..)?)
    }
}

pub fn import_spec(p: &RawParams) -> Option<ImportSpec> {
    let b = p.get(&[], 132)?;
    let mut r = Reader(&b);
    let mut s = ImportSpec {
        to: r.to()?,
        action: r.u8()?,
        options: r.u8()?,
        charset: r.u8()?,
        map: Vec::new(),
    };
    for _ in 0..r.u16()? {
        s.map.push((r.field()?, r.u8()?));
    }
    Some(s)
}

pub fn export_spec(p: &RawParams) -> Option<ExportSpec> {
    let b = p.get(&[], 133)?;
    let mut r = Reader(&b);
    let mut s = ExportSpec::default();
    for _ in 0..r.u16()? {
        let (to, f) = (r.to()?, r.field()?);
        r.u16()?;
        r.field()?;
        r.field()?;
        r.u16()?;
        s.fields.push((to, f));
    }
    for _ in 0..r.u16()? {
        let (to, f) = (r.to()?, r.field()?);
        r.u16()?;
        s.group_by.push((to, f, r.u8()? != 0));
    }
    s.formatted = r.u8()? != 0;
    s.charset = r.u8()?;
    Some(s)
}

/// The engine's character set for a step's stored number (0 Windows ANSI,
/// 1 DOS, 2 Macintosh, 3 Unicode, 4/5 UTF-8).
// ponytail: DOS maps to 7-bit ASCII (FileMaker's is likely code page 437).
pub fn step_charset(code: u8) -> engine::charset::Charset {
    use engine::charset::Charset as C;
    match code {
        0 => C::WindowsAnsi,
        1 => C::Ascii,
        2 => C::MacRoman,
        3 => C::Utf16,
        _ => C::Utf8,
    }
}

impl Engine {
    /// Import Records with no dialog and a stored file: the mapping as
    /// stored. `None`: FileMaker would ask (the host's dialog).
    pub(crate) fn import_stored(
        &mut self,
        host: &mut dyn Host,
        frame: &Frame,
        s: &Step,
    ) -> Option<Result<Flow, u32>> {
        if s.options & NO_DIALOG == 0 || s.arg != 1 {
            return None;
        }
        let spec = import_spec(&s.raw_params)?;
        let path = self.paths(frame, s).into_iter().next()?;
        let bytes = match host.file_read(&path) {
            // No file system (the web build): the dialog.
            Err(error::UNAVAILABLE) => return None,
            r => r,
        };
        Some((|| {
            self.commit(host)?;
            let bytes = bytes?;
            let format = Format::from_name(&path).unwrap_or(Format::Tab);
            let charset = step_charset(spec.charset);
            let db = host.db();
            let t = db.table_of(spec.to).ok_or(error::FIELD_MISSING)?;
            let tid = t.id;
            let name = |f: u32| t.field(f).map_or(String::new(), |x| x.name.clone());
            let names: Vec<String> = spec
                .map
                .iter()
                .map(|&(f, k)| if k == 1 { String::new() } else { name(f) })
                .collect();
            let matching: Vec<String> = spec
                .map
                .iter()
                .filter(|m| m.1 == 2)
                .map(|m| name(m.0))
                .collect();
            let add_remaining = spec.options & import_opt::ADD_REMAINING != 0;
            let mode = match spec.action {
                1 => engine::import::Mode::Replace { add_remaining },
                2 => engine::import::Mode::Update {
                    match_fields: matching,
                    add_remaining,
                },
                _ => engine::import::Mode::Add,
            };
            let opts = engine::import::Options {
                format,
                // ponytail: where FileMaker keeps "Don't import first
                // record (contains field names)" is not decoded; every row
                // is imported.
                first_row_names: false,
                mapping: engine::import::Mapping::ByOrder(names),
                mode,
                auto_enter: spec.options & import_opt::AUTO_ENTER != 0 && self.auto_enter(),
                charset,
                delimiter: None,
                split_repetitions: spec.options & import_opt::SPLIT_REPETITIONS != 0,
                container_files: None,
            };
            let rows = engine::import::read_source(&opts, &bytes).map_err(|_| 100u32)?;
            let (table, _, _) = Self::context(host);
            let found = if tid == table {
                host.window().found.clone()
            } else {
                Vec::new()
            };
            let rep = engine::import::import(host.db(), tid, &found, rows, &opts)
                .map_err(|_| error::UNAVAILABLE)?;
            if tid == table {
                let w = host.window();
                w.found = rep.found;
                w.current = 0;
                w.sorted = false;
                w.sort.clear();
            }
            Ok(Flow::Next)
        })())
    }

    /// Export Records with no dialog and a stored file, in the order
    /// stored; the format from the file's extension.
    pub(crate) fn export_stored(
        &mut self,
        host: &mut dyn Host,
        frame: &Frame,
        s: &Step,
    ) -> Option<Result<Flow, u32>> {
        if s.options & NO_DIALOG == 0 {
            return None;
        }
        let spec = export_spec(&s.raw_params)?;
        let path = self.paths(frame, s).into_iter().next()?;
        Some((|| {
            self.commit(host)?;
            if !host.db().can_export() {
                return Err(9);
            }
            let (_, to, _) = Self::context(host);
            let found = host.window().found.clone();
            let file = host.window().host.file_name.clone();
            let layout = Some(host.window().layout);
            let db = host.db();
            let mut columns = Vec::new();
            for &(fto, f) in &spec.fields {
                let Some(t) = db.table_of(fto) else { continue };
                let Some(fd) = t.field(f) else { continue };
                let name = match db.occurrences.iter().find(|o| o.id == fto) {
                    Some(o) if fto != to => format!("{}::{}", o.name, fd.name),
                    _ => fd.name.clone(),
                };
                columns.push(engine::export::Column {
                    to: fto,
                    table: t.id,
                    field: f,
                    name,
                    data_type: fd.data_type,
                });
            }
            let group: Vec<u32> = spec.group_by.iter().filter(|g| g.2).map(|g| g.1).collect();
            // ponytail: the file type is taken from the extension (the
            // step stores none); tab-separated text otherwise.
            let format = Format::from_name(&path).unwrap_or(Format::Tab);
            let file_name = format!("{file}.fmp12");
            let out = engine::export::Export {
                db,
                to,
                found: &found,
                columns: &columns,
                formatted: spec.formatted,
                file_name: &file_name,
                group_by: &group,
                charset: step_charset(spec.charset),
                layout,
            }
            .write(format)
            .map_err(|_| error::UNAVAILABLE)?;
            let folders = s.options & CREATE_FOLDERS != 0;
            write_result(host.file_write(&path, &out, folders), &path)
        })())
    }

    /// Export Field Contents to its stored file (`0x200` create folders):
    /// a container's file, or the text with CR LF line ends. `None`: no
    /// stored file, or no file system (the host asks).
    pub(crate) fn export_field_stored(
        &mut self,
        host: &mut dyn Host,
        frame: &Frame,
        s: &Step,
    ) -> Option<Result<Flow, u32>> {
        let path = self.paths(frame, s).into_iter().next()?;
        let f = s.inline_field()?;
        let (table, to, rec) = Self::context(host);
        let db = host.db();
        let t = db.table_of(f.to)?.id;
        let rec = rec?;
        let r = if t == table {
            rec
        } else {
            *db.related(to, rec, f.to).first()?
        };
        let data = match db.value_rep(t, r, f.field, f.repetition.max(1) as u32) {
            Value::Container(c) => c.data,
            v => v.to_text().replace('\r', "\r\n").into_bytes(),
        };
        match host.file_write(&path, &data, s.options & CREATE_FOLDERS != 0) {
            Err(error::UNAVAILABLE) => None,
            r => Some(write_result(r, &path)),
        }
    }

    /// Insert Picture / Insert File from the stored file (`image:` paths
    /// read as `file:`); `None` when the step stores none.
    pub(crate) fn insert_stored(
        &mut self,
        host: &mut dyn Host,
        frame: &Frame,
        s: &Step,
    ) -> Option<Result<calc::value::Container, u32>> {
        let path = self.paths(frame, s).into_iter().next()?;
        let path = match path.strip_prefix("image") {
            Some(rest) => format!("file{rest}"),
            None => path,
        };
        let data = match host.file_read(&path) {
            Err(error::UNAVAILABLE) => return None,
            r => r,
        };
        Some(data.map(|data| {
            calc::value::Container {
                name: path
                    .rsplit(['/', ':'])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                data,
                path: if s.options & REFERENCE != 0 {
                    path.clone()
                } else {
                    String::new()
                },
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(key: u32, hex: &str) -> RawParams {
        let mut p = RawParams::default();
        let b = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        p.kv.insert((vec![], key), b);
        p
    }

    /// Bytes FileMaker Pro 26 wrote for Import Records [file:in.csv; Add;
    /// UTF-8; auto-enter; Task ← 1, Due Date skipped, Description ← 3].
    #[test]
    fn import_mapping_as_filemaker_wrote_it() {
        let p = raw(132, "0403d000020002050003020101000201020102010300");
        let s = import_spec(&p).unwrap();
        assert_eq!(s.to, 130);
        assert_eq!(
            (s.action, s.options, s.charset),
            (0, import_opt::AUTO_ENTER, 5)
        );
        assert_eq!(s.map, vec![(1, 0), (2, 1), (3, 0)]);
        // Update matching, add remaining, Mac Roman, Task the match field.
        let s = import_spec(&raw(132, "0403d00002010802000102010102")).unwrap();
        assert_eq!(
            (s.action, s.options, s.charset),
            (1, import_opt::ADD_REMAINING, 2)
        );
        assert_eq!(s.map, vec![(1, 2)]);
        let s = import_spec(&raw(132, "0403d00002020403000102010100")).unwrap();
        assert_eq!((s.action, s.options), (2, import_opt::MATCH_NAMES));
    }

    /// Export Records [file:o.htm; DOS; Task; grouped by Status, Category]
    /// and [file:out.csv; UTF-8; formatted; Task, Description].
    #[test]
    fn export_order_as_filemaker_wrote_it() {
        let p = raw(
            133,
            "00010403d000020201010001020100020100000100020403d000020201040001010403d00002020105000101000101",
        );
        let s = export_spec(&p).unwrap();
        assert_eq!(s.fields, vec![(130, 1)]);
        assert_eq!(s.group_by, vec![(130, 4, true), (130, 5, true)]);
        assert_eq!((s.formatted, s.charset), (false, 1));
        let p = raw(
            133,
            "00020403d00002020101000102010002010000010403d00002020103000102010002010000010000010401",
        );
        let s = export_spec(&p).unwrap();
        assert_eq!(s.fields, vec![(130, 1), (130, 3)]);
        assert!(s.group_by.is_empty());
        assert_eq!((s.formatted, s.charset), (true, 4));
    }

    #[test]
    fn character_sets() {
        use engine::charset::Charset as C;
        assert_eq!(step_charset(0), C::WindowsAnsi);
        assert_eq!(step_charset(2), C::MacRoman);
        assert_eq!(step_charset(3), C::Utf16);
        assert_eq!(step_charset(4), C::Utf8);
        assert_eq!(step_charset(5), C::Utf8);
    }
}
