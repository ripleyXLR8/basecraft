//! Import Records from comma-separated text, tab-separated text, custom-
//! separated text, Merge, Excel `.xlsx`, FMPXMLRESULT XML, dBASE DBF,
//! another FileMaker file or a folder of files: map source columns to fields
//! by name or by order, then add records, update matching records, or
//! replace the found set's data.
//!
//! A source value holding group separators (0x1D, as FileMaker exports a
//! repeating field) or several `DATA` elements (FMPXMLRESULT) goes into the
//! target field's repetitions, or into separate records with
//! [`Options::split_repetitions`].

use fmp12::schema::{DataType, FieldKind};

use crate::charset::{self, Charset};
use crate::export::{Format, GROUP_SEPARATOR, VERTICAL_TAB};
use crate::{Database, Rule, rep_key, repetitions, stored_text, typed};

/// Parses a UTF-8 source file into rows of cells (see [`read_rows_in`]).
pub fn read_rows(format: Format, bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    read_rows_in(format, bytes, Charset::Utf8, None)
}

/// Parses a source file in `charset` (a byte order mark overrides it) into
/// rows of cells. A vertical tab inside a value becomes a return again.
/// `delimiter`: custom-separated text, values separated by that character
/// (quoted values as in CSV). XML (FMPXMLRESULT) and DBF sources start with
/// a row of their field names; a repeating value's repetitions are joined
/// with the group separator.
pub fn read_rows_in(
    format: Format,
    bytes: &[u8],
    cs: Charset,
    delimiter: Option<char>,
) -> Result<Vec<Vec<String>>, String> {
    let text = || charset::decode(bytes, cs);
    let vt = |s: &str| s.replace(VERTICAL_TAB, "\r");
    let delimited = |d: char| {
        csv(&text(), d)
            .into_iter()
            .map(|r| r.iter().map(|c| vt(c)).collect())
            .collect()
    };
    match (format, delimiter) {
        (Format::Csv | Format::Merge | Format::Tab, Some(d)) => Ok(delimited(d)),
        (Format::Csv | Format::Merge, None) => Ok(delimited(',')),
        (Format::Tab, None) => Ok(text()
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .map(|l| l.split('\t').map(vt).collect())
            .collect()),
        (Format::Xlsx, _) => crate::xlsx::read(bytes),
        (Format::Xml, _) => xml_rows(&text()),
        (Format::Dbf, _) => crate::dbf::read(bytes, cs),
        (Format::Json | Format::Html | Format::Fmp12, _) => Err(format!(
            "importing {format:?} is not supported here (FileMaker files: fmp12_source)"
        )),
    }
}

/// Rows of an FMPXMLRESULT document: the `METADATA` field names, then one
/// row per `ROW`, a `COL`'s `DATA` elements joined with the group separator.
// ponytail: dates and times are taken as written; the DATABASE element's
// DATEFORMAT / TIMEFORMAT are not used to convert them.
fn xml_rows(s: &str) -> Result<Vec<Vec<String>>, String> {
    let doc = roxmltree::Document::parse(s).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "FMPXMLRESULT" {
        return Err("only the FMPXMLRESULT grammar can be imported".into());
    }
    fn child<'a, 'i>(
        n: roxmltree::Node<'a, 'i>,
        name: &'static str,
    ) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> {
        n.children()
            .filter(move |c| c.is_element() && c.tag_name().name() == name)
    }
    let mut rows = Vec::new();
    let names: Vec<String> = child(root, "METADATA")
        .flat_map(|m| child(m, "FIELD"))
        .map(|f| f.attribute("NAME").unwrap_or("").to_string())
        .collect();
    if names.is_empty() {
        return Err("FMPXMLRESULT without METADATA fields".into());
    }
    rows.push(names);
    for row in child(root, "RESULTSET").flat_map(|r| child(r, "ROW")) {
        rows.push(
            child(row, "COL")
                .map(|col| {
                    let data: Vec<String> = child(col, "DATA")
                        .map(|d| d.text().unwrap_or("").to_string())
                        .collect();
                    let mut v = data.join(&GROUP_SEPARATOR.to_string());
                    // An empty COL's single DATA: no trailing separators.
                    v.truncate(v.trim_end_matches(GROUP_SEPARATOR).len());
                    v
                })
                .collect(),
        );
    }
    Ok(rows)
}

/// What a folder import reads: "Picture and movie files" or "Text files".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FolderKind {
    #[default]
    Pictures,
    Text,
}

/// Folder of Files Import Options.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Folder {
    pub path: std::path::PathBuf,
    /// "Include all enclosed folders" (100 levels at most, as FileMaker).
    pub subfolders: bool,
    pub kind: FolderKind,
}

/// Prefix of an "Image Thumbnail" cell of a folder import: the file whose
/// thumbnail goes into the container field.
pub const THUMBNAIL: &str = "thumbnail:";

const PICTURES: [&str; 15] = [
    "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "pdf", "mov", "mp4", "m4v",
    "avi", "mpg",
];
/// Text files FileMaker's folder import reads (Claris documentation).
const TEXTS: [&str; 10] = [
    "csv", "json", "log", "mer", "tab", "tsv", "txt", "xml", "xsl", "xslt",
];

/// Rows of a folder import: a row of source field names, as FileMaker's
/// field mapping shows them, then one row per file in path order.
/// Pictures: `Image` (the file's path), `File Name`, `File Path`, `Image
/// Thumbnail` ([`THUMBNAIL`] and the path). Text files: `Text Content` (in
/// `cs`), `File Name`, `File Path`. Import with
/// [`Options::container_files`] set, so container fields get the files.
pub fn read_folder(f: &Folder, cs: Charset) -> Result<Vec<Vec<String>>, String> {
    fn walk(dir: &std::path::Path, deep: u32, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if deep > 0 {
                    walk(&p, deep - 1, out);
                }
            } else {
                out.push(p);
            }
        }
    }
    if !f.path.is_dir() {
        return Err(format!("{} is not a folder", f.path.display()));
    }
    let mut files = Vec::new();
    walk(&f.path, if f.subfolders { 100 } else { 0 }, &mut files);
    files.sort();
    let kinds: &[&str] = match f.kind {
        FolderKind::Pictures => &PICTURES,
        FolderKind::Text => &TEXTS,
    };
    files.retain(|p| {
        p.extension()
            .is_some_and(|e| kinds.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
    });
    let header: &[&str] = match f.kind {
        FolderKind::Pictures => &["Image", "File Name", "File Path", "Image Thumbnail"],
        FolderKind::Text => &["Text Content", "File Name", "File Path"],
    };
    let mut rows = vec![header.iter().map(|s| s.to_string()).collect()];
    for p in files {
        let name = p
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let path = calc::functions::fm_path(&p);
        let abs = p.to_string_lossy().into_owned();
        rows.push(match f.kind {
            FolderKind::Pictures => vec![abs.clone(), name, path, format!("{THUMBNAIL}{abs}")],
            FolderKind::Text => {
                let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                vec![charset::decode(&bytes, cs), name, path]
            }
        });
    }
    Ok(rows)
}

/// How a folder import puts files into container fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Files {
    /// A copy of each file (embedded, or externally stored as the field's
    /// storage options say).
    Copy,
    /// "Import only a reference to each file".
    Reference,
}

/// Longest side of an "Image Thumbnail", in pixels.
// ponytail: FileMaker's thumbnail size is not documented; 256 px, as PNG.
const THUMB_SIZE: u32 = 256;

/// A container value for the file at `cell` (a folder import's path).
fn file_value(cell: &str, files: Files) -> calc::Value {
    use calc::value::Container;
    let (thumb, path) = match cell.strip_prefix(THUMBNAIL) {
        Some(p) => (true, p),
        None => (false, cell),
    };
    let p = std::path::Path::new(path);
    if files == Files::Reference && !thumb {
        return calc::Value::Container(Container {
            path: format!("file:{}", calc::functions::fm_path(p)),
            ..Default::default()
        });
    }
    let Ok(data) = std::fs::read(p) else {
        return calc::Value::empty();
    };
    let name = p
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    if !thumb {
        return calc::Value::Container(Container {
            name,
            data,
            path: String::new(),
        });
    }
    let Ok(img) = image::load_from_memory(&data) else {
        return calc::Value::empty();
    };
    let mut png = std::io::Cursor::new(Vec::new());
    if img
        .thumbnail(THUMB_SIZE, THUMB_SIZE)
        .write_to(&mut png, image::ImageFormat::Png)
        .is_err()
    {
        return calc::Value::empty();
    }
    let stem = p
        .file_stem()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    calc::Value::Container(Container {
        name: format!("{stem}.png"),
        data: png.into_inner(),
        path: String::new(),
    })
}

/// A table of a FileMaker import source: its name and rows.
pub type SourceTable = (String, Vec<Vec<String>>);

/// A FileMaker file as an import source: per table, its name and rows (a
/// row of field names, then one row per record). Values are written in
/// `locale` (the destination file's formats), so dates and numbers keep
/// their meaning between files of different languages; a repeating field's
/// repetitions are joined with the group separator. Summary and container
/// fields are left out.
pub fn fmp12_source(
    bytes: &[u8],
    locale: &fmp12::schema::Locale,
) -> Result<Vec<SourceTable>, String> {
    let file = fmp12::File::from_bytes(bytes.to_vec()).map_err(|e| e.to_string())?;
    let src = Database::load(&file).map_err(|e| e.to_string())?;
    Ok(src
        .tables
        .iter()
        .map(|t| {
            let fields: Vec<&crate::Field> = t
                .fields
                .iter()
                .filter(|f| f.kind != FieldKind::Summary && f.data_type != DataType::Container)
                .collect();
            let mut rows = vec![fields.iter().map(|f| f.name.clone()).collect()];
            rows.extend((0..t.records.len()).map(|r| {
                fields
                    .iter()
                    .map(|f| {
                        let mut reps: Vec<String> = (1..=repetitions(f))
                            .map(|n| stored_text(&src.value_rep(t.id, r, f.id, n), locale))
                            .collect();
                        while reps.len() > 1 && reps.last().is_some_and(|s| s.is_empty()) {
                            reps.pop();
                        }
                        reps.join(&GROUP_SEPARATOR.to_string())
                    })
                    .collect()
            }));
            (t.name.clone(), rows)
        })
        .collect())
}

/// Values separated by `delim` (comma-separated values): quoted values may
/// hold the delimiter, doubled quotes and line breaks; records end with CR,
/// LF or CR LF.
fn csv(s: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, c) if c == delim => row.push(std::mem::take(&mut cell)),
            (false, '\r' | '\n') => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            (false, c) => cell.push(c),
        }
    }
    if any {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// Which field each source column goes to.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Mapping {
    /// Columns go to the fields named in the first row (needs
    /// `first_row_names`); unknown names are skipped.
    #[default]
    ByName,
    /// Column *i* goes to the *i*-th of these field names (an empty name
    /// skips the column), or, when the list is empty, to the *i*-th field
    /// of the table in definition order.
    ByOrder(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Mode {
    /// Add every row as a new record.
    #[default]
    Add,
    /// Update the records of the found set whose match fields equal the
    /// row's; rows without a match are added when `add_remaining`. A row
    /// whose match fields are all empty matches no record.
    Update {
        match_fields: Vec<String>,
        add_remaining: bool,
    },
    /// Replace the data of the found set's records, row *i* into record *i*;
    /// rows beyond the found set are added when `add_remaining`.
    Replace { add_remaining: bool },
}

/// Import Records' options, as its dialogs and the script step's stored
/// options give them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options {
    pub format: Format,
    pub first_row_names: bool,
    pub mapping: Mapping,
    pub mode: Mode,
    /// Apply auto-enter options (serials, stamps, calculations) to new and
    /// changed records.
    pub auto_enter: bool,
    /// Source character set (text formats, DBF); a byte order mark wins.
    pub charset: Charset,
    /// Custom-separated values: the field delimiter.
    pub delimiter: Option<char>,
    /// "Split values into separate records": a source value's repetitions
    /// become one record each (the other fields repeated in each), instead
    /// of "Keep values in the original record" (into the field's repetitions).
    pub split_repetitions: bool,
    /// A folder import: container fields get the files the cells name.
    pub container_files: Option<Files>,
}

/// Parses a source file with `opts`' format, character set and delimiter.
pub fn read_source(opts: &Options, bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    read_rows_in(opts.format, bytes, opts.charset, opts.delimiter)
}

/// A validation failure on one source row.
#[derive(Debug, Clone, PartialEq)]
pub struct RowError {
    /// 1-based row number in the source (header row included).
    pub row: usize,
    pub field: u32,
    pub rule: Rule,
    /// The record was not imported (the field validates "always").
    pub skipped: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// Record ids of the added and updated records.
    pub added: Vec<u32>,
    pub updated: Vec<u32>,
    /// Rows not imported (no match, no room, or failed validation).
    pub skipped: usize,
    pub errors: Vec<RowError>,
    /// Indexes of the imported records: the found set FileMaker shows after an import.
    pub found: Vec<usize>,
}

/// Imports `rows` into `table`. `found` is the found set that Update and
/// Replace act on.
pub fn import(
    db: &mut Database,
    table: u32,
    found: &[usize],
    mut rows: Vec<Vec<String>>,
    opts: &Options,
) -> Result<Report, String> {
    let t = db.table(table).ok_or("no such table")?;
    let header = if opts.first_row_names && !rows.is_empty() {
        Some(rows.remove(0))
    } else {
        None
    };
    let first_row = if header.is_some() { 2 } else { 1 };
    let importable = |name: &str| {
        t.field_by_name(name)
            .filter(|f| f.kind == FieldKind::Normal)
            .map(|f| (f.id, f.data_type, repetitions(f)))
    };
    // Target (field, type, repetitions) per source column.
    let targets: Vec<Option<(u32, DataType, u32)>> = match &opts.mapping {
        Mapping::ByName => header
            .as_ref()
            .ok_or("mapping by name needs the first row to hold field names")?
            .iter()
            .map(|n| importable(n.trim()))
            .collect(),
        Mapping::ByOrder(names) if names.is_empty() => t
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::Normal)
            .map(|f| Some((f.id, f.data_type, repetitions(f))))
            .collect(),
        Mapping::ByOrder(names) => names
            .iter()
            .map(|n| {
                if n.is_empty() {
                    return Ok(None);
                }
                importable(n)
                    .map(Some)
                    .ok_or_else(|| format!("no importable field {n:?} in table {:?}", t.name))
            })
            .collect::<Result<_, String>>()?,
    };
    let match_ids: Vec<u32> = match &opts.mode {
        Mode::Update { match_fields, .. } => {
            if match_fields.is_empty() {
                return Err("update needs at least one match field".into());
            }
            match_fields
                .iter()
                .map(|n| {
                    let id = importable(n).ok_or_else(|| format!("no field {n:?}"))?.0;
                    targets
                        .iter()
                        .any(|x| x.is_some_and(|(f, _, _)| f == id))
                        .then_some(id)
                        .ok_or_else(|| format!("match field {n:?} is not mapped to a column"))
                })
                .collect::<Result<_, String>>()?
        }
        _ => Vec::new(),
    };
    let ti = db
        .tables
        .iter()
        .position(|x| x.id == table)
        .ok_or("no such table")?;
    // Source line of each row (1-based, header included).
    let mut lines: Vec<usize> = (0..rows.len()).map(|k| first_row + k).collect();
    if opts.split_repetitions {
        let mut split = Vec::new();
        let mut split_lines = Vec::new();
        for (row, line) in rows.iter().zip(&lines) {
            let parts: Vec<Vec<&str>> = row
                .iter()
                .map(|c| c.split(GROUP_SEPARATOR).collect())
                .collect();
            let n = parts.iter().map(|p| p.len()).max().unwrap_or(1);
            for i in 0..n {
                split.push(
                    parts
                        .iter()
                        .map(|p| match p.as_slice() {
                            [one] => one.to_string(),
                            _ => p.get(i).map_or(String::new(), |s| s.to_string()),
                        })
                        .collect(),
                );
                split_lines.push(*line);
            }
        }
        rows = split;
        lines = split_lines;
    }
    let loc = db.locale.clone();
    let mut report = Report::default();
    for (k, row) in rows.iter().enumerate() {
        let line = lines[k];
        // (field, repetitions, stored text per repetition)
        let mut values: Vec<(u32, u32, Vec<String>)> = Vec::new();
        for (tg, v) in targets.iter().zip(row) {
            let Some((f, dt, reps)) = *tg else { continue };
            let mut texts = Vec::new();
            for p in v.split(GROUP_SEPARATOR).take(reps as usize) {
                texts.push(match (dt, opts.container_files) {
                    (DataType::Container, Some(files)) if !p.is_empty() => {
                        let val = file_value(p, files);
                        if val.is_empty() {
                            String::new()
                        } else {
                            db.store_value(table, f, &val)
                        }
                    }
                    _ => convert(p, dt, opts.format, &loc),
                });
            }
            values.push((f, reps, texts));
        }
        let get = |f: u32| {
            values
                .iter()
                .find(|(x, _, _)| *x == f)
                .and_then(|(_, _, v)| v.first())
                .map_or("", |v| v.as_str())
        };
        let no_match = match_ids.iter().all(|&f| get(f).is_empty());
        // Records this row goes to; None = add a new one.
        let dest: Vec<usize> = match &opts.mode {
            Mode::Add => Vec::new(),
            Mode::Update { .. } if no_match => Vec::new(),
            Mode::Update { .. } => found
                .iter()
                .copied()
                .filter(|&r| {
                    match_ids.iter().all(|&f| {
                        let have = db.tables[ti].records[r]
                            .values
                            .get(&f)
                            .map_or("", |s| s.as_str());
                        have.eq_ignore_ascii_case(get(f))
                    })
                })
                .collect(),
            Mode::Replace { .. } => found.get(k).copied().into_iter().collect(),
        };
        let add = dest.is_empty()
            && match &opts.mode {
                Mode::Add => true,
                Mode::Update { add_remaining, .. } | Mode::Replace { add_remaining } => {
                    *add_remaining
                }
            };
        if dest.is_empty() && !add {
            report.skipped += 1;
            continue;
        }
        let targets_now: Vec<(usize, bool)> = if add {
            let r = if opts.auto_enter {
                db.new_record(table).ok_or("no such table")?
            } else {
                db.tables[ti].new_record()
            };
            vec![(r, true)]
        } else {
            dest.into_iter().map(|r| (r, false)).collect()
        };
        let mut skipped_row = false;
        for (r, new) in targets_now {
            let before = db.tables[ti].records[r].values.clone();
            for (f, reps, texts) in &values {
                for n in 1..=*reps {
                    let key = rep_key(*f, n);
                    let vals = &mut db.tables[ti].records[r].values;
                    match texts.get(n as usize - 1).filter(|s| !s.is_empty()) {
                        Some(v) => vals.insert(key, v.clone()),
                        None => vals.remove(&key),
                    };
                }
            }
            if opts.auto_enter {
                // Lookups whose match fields were imported, as FileMaker does.
                let changed: Vec<u32> = values.iter().map(|(f, _, _)| *f).collect();
                db.apply_lookups(table, r, Some(&changed));
                if !new {
                    db.touch(table, r);
                }
            }
            let mut rejected = false;
            for e in db.validate(table, r) {
                rejected |= e.always;
                report.errors.push(RowError {
                    row: line,
                    field: e.field,
                    rule: e.rule,
                    skipped: e.always,
                });
            }
            if rejected {
                skipped_row = true;
                if new {
                    db.tables[ti].records.remove(r);
                } else {
                    db.tables[ti].records[r].values = before;
                }
                continue;
            }
            let id = db.tables[ti].records[r].id;
            if new {
                report.added.push(id);
            } else {
                report.updated.push(id);
            }
            report.found.push(r);
        }
        if skipped_row {
            report.skipped += 1;
        }
    }
    Ok(report)
}

/// A source value as stored text: the first repetition only, Excel serials
/// and numbers rewritten in the file's formats.
fn convert(v: &str, dt: DataType, format: Format, loc: &fmp12::schema::Locale) -> String {
    let v = v.split(GROUP_SEPARATOR).next().unwrap_or("");
    // DBF dates are `YYYYMMDD`.
    if format == Format::Dbf && dt == DataType::Date && v.len() == 8 {
        let n = |r: std::ops::Range<usize>| v.get(r).and_then(|s| s.parse::<i64>().ok());
        if let (Some(y), Some(m), Some(d)) = (n(0..4), n(4..6), n(6..8)) {
            let day = calc::value::day_number(y, m, d);
            return stored_text(&calc::Value::Date(day), loc);
        }
    }
    if format != Format::Xlsx || v.is_empty() {
        return v.to_string();
    }
    let Ok(n) = v.parse::<f64>() else {
        return v.to_string();
    };
    let us = match dt {
        DataType::Number => v.to_string(),
        _ => match crate::xlsx::serial_to_us(n, dt) {
            Some(s) => s,
            None => return v.to_string(),
        },
    };
    stored_text(&typed(&us, dt), loc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{Export, columns};
    use crate::tests::{AMOUNT, CITY, NAME, NOTE, T, sample};

    fn opts(format: Format, mode: Mode) -> Options {
        Options {
            format,
            first_row_names: true,
            mapping: Mapping::ByName,
            mode,
            auto_enter: true,
            ..Default::default()
        }
    }

    #[test]
    fn parses_each_format() {
        type Case = (Format, &'static [u8], Vec<Vec<&'static str>>);
        let cases: [Case; 4] = [
            (
                Format::Csv,
                b"\"a\",\"b,c\"\r\n\"d \"\"q\"\"\",\"x\x0by\"\r\n",
                vec![vec!["a", "b,c"], vec!["d \"q\"", "x\ry"]],
            ),
            (
                Format::Csv,
                b"1,2\n3,\"4\n5\"",
                vec![vec!["1", "2"], vec!["3", "4\n5"]],
            ),
            (
                Format::Tab,
                b"a\tb\r\nc\t\r\n",
                vec![vec!["a", "b"], vec!["c", ""]],
            ),
            (
                Format::Merge,
                b"\"N\"\r\n\"v\"\r\n",
                vec![vec!["N"], vec!["v"]],
            ),
        ];
        for (f, bytes, want) in cases {
            assert_eq!(read_rows(f, bytes).unwrap(), want, "{f:?}");
        }
        assert!(read_rows(Format::Json, b"[]").is_err());
    }

    /// Export every format that can be imported, import it into an empty
    /// copy, and compare the records.
    #[test]
    fn round_trip() {
        for format in [Format::Csv, Format::Tab, Format::Merge, Format::Xlsx] {
            let src = sample();
            let cols = columns(&src, T, &[]).unwrap();
            let found: Vec<usize> = (0..5).collect();
            let bytes = Export {
                db: &src,
                to: T,
                found: &found,
                columns: &cols,
                formatted: false,
                file_name: "",
                group_by: &[],
                charset: Default::default(),
                layout: None,
            }
            .write(format)
            .unwrap();
            let mut dst = sample();
            dst.tables[0].records.clear();
            let header = matches!(format, Format::Merge | Format::Xlsx);
            let o = Options {
                format,
                first_row_names: header,
                mapping: if header {
                    Mapping::ByName
                } else {
                    Mapping::ByOrder(Vec::new())
                },
                mode: Mode::Add,
                auto_enter: false,
                ..Default::default()
            };
            let rows = read_rows(format, &bytes).unwrap();
            let r = import(&mut dst, T, &[], rows, &o).unwrap();
            assert_eq!(r.added.len(), 5, "{format:?}");
            let want: Vec<_> = src.tables[0].records.iter().map(|r| &r.values).collect();
            let got: Vec<_> = dst.tables[0].records.iter().map(|r| &r.values).collect();
            // Tabs inside fields become spaces in tab-separated text, as in FileMaker.
            if format == Format::Tab {
                assert_eq!(got[3][&NOTE], "tab here");
                assert_eq!(got[..3], want[..3]);
            } else {
                assert_eq!(got, want, "{format:?}");
            }
        }
    }

    #[test]
    fn update_and_replace() {
        let mut db = sample();
        let rows = read_rows(Format::Csv, b"Name,City\r\nada,Turin\r\nZed,Nice\r\n").unwrap();
        let all: Vec<usize> = (0..5).collect();
        let mode = Mode::Update {
            match_fields: vec!["Name".into()],
            add_remaining: false,
        };
        let r = import(&mut db, T, &all, rows.clone(), &opts(Format::Csv, mode)).unwrap();
        assert_eq!((r.updated, r.added.len(), r.skipped), (vec![1], 0, 1));
        assert_eq!(db.tables[0].records[0].values[&CITY], "Turin");
        assert_eq!(db.tables[0].records[0].values[&AMOUNT], "10");

        let mode = Mode::Update {
            match_fields: vec!["Name".into()],
            add_remaining: true,
        };
        let r = import(&mut db, T, &all, rows.clone(), &opts(Format::Csv, mode)).unwrap();
        assert_eq!(r.added, vec![6]);

        let mut db = sample();
        let r = import(
            &mut db,
            T,
            &[3, 4],
            rows,
            &opts(
                Format::Csv,
                Mode::Replace {
                    add_remaining: false,
                },
            ),
        )
        .unwrap();
        assert_eq!(r.updated, vec![4, 5]);
        assert_eq!(db.tables[0].records[3].values[&NAME], "ada");
        assert_eq!(db.tables[0].records[4].values[&CITY], "Nice");
    }

    #[test]
    fn validation_and_auto_enter() {
        let mut db = sample();
        // Name: not empty, validated always; Amount: auto-enter serial.
        let name = &mut db.tables[0].fields[0].def.options;
        name.resize(26, 0);
        name[14] |= 1 << 2;
        name[15] |= 1 << 3;
        let amount = &mut db.tables[0].fields[1].def.options;
        amount.resize(26, 0);
        amount[11] |= 1 << 1;
        let rows = read_rows(Format::Csv, b"Name,City\r\nNew,Oslo\r\n,Nowhere\r\n").unwrap();
        let r = import(&mut db, T, &[], rows.clone(), &opts(Format::Csv, Mode::Add)).unwrap();
        assert_eq!(r.added, vec![6]);
        assert_eq!(r.skipped, 1);
        assert_eq!(
            r.errors,
            vec![RowError {
                row: 3,
                field: NAME,
                rule: Rule::NotEmpty,
                skipped: true
            }]
        );
        assert_eq!(db.tables[0].records.len(), 6);
        assert_eq!(db.tables[0].records[5].values[&AMOUNT], "41");
        let mut o = opts(Format::Csv, Mode::Add);
        o.auto_enter = false;
        import(&mut db, T, &[], rows, &o).unwrap();
        assert!(!db.tables[0].records[6].values.contains_key(&AMOUNT));
    }

    #[test]
    fn xml_dbf_delimiters_and_charsets() {
        // FMPXMLRESULT as exported, back into an empty copy (repetitions too).
        let src = crate::export::tests::repeating_sample();
        let o = crate::export::Options {
            format: Format::Xml,
            fields: vec!["Name".into(), "Note".into()],
            ..Default::default()
        };
        let xml = crate::export::export(&src, T, &[0, 1], None, "", &o).unwrap();
        let rows = read_rows(Format::Xml, &xml).unwrap();
        assert_eq!(rows[0], ["Name", "Note"]);
        assert_eq!(rows[1], ["Ada", "line 1\rline 2\u{1d}\u{1d}third"]);
        assert_eq!(rows[2], ["Bob \"B\"", ""]);
        let mut dst = crate::export::tests::repeating_sample();
        dst.tables[0].records.clear();
        let r = import(&mut dst, T, &[], rows, &opts(Format::Xml, Mode::Add)).unwrap();
        assert_eq!(r.added.len(), 2);
        assert_eq!(dst.tables[0].records[0].values, {
            let mut v = src.tables[0].records[0].values.clone();
            v.retain(|k, _| [NAME, NOTE, rep_key(NOTE, 3)].contains(k));
            v
        });
        assert!(read_rows(Format::Xml, b"<FMPDSORESULT/>").is_err());

        // DBF: names and dates as dBASE writes them.
        let o = crate::export::Options {
            format: Format::Dbf,
            fields: vec!["Name".into(), "Born".into()],
            ..Default::default()
        };
        let dbf = crate::export::export(&src, T, &[0], None, "", &o).unwrap();
        let rows = read_rows(Format::Dbf, &dbf).unwrap();
        assert_eq!(rows[1], ["Ada", "18151210"]);
        let mut dst = sample();
        let mut op = opts(Format::Dbf, Mode::Add);
        op.mapping = Mapping::ByOrder(vec!["Name".into(), "Born".into()]);
        import(&mut dst, T, &[], rows, &op).unwrap();
        assert_eq!(
            dst.tables[0].records[5].values[&crate::tests::BORN],
            "12/10/1815"
        );

        // Custom delimiter and Windows ANSI.
        let mut o = Options {
            format: Format::Tab,
            delimiter: Some(';'),
            charset: Charset::WindowsAnsi,
            ..Default::default()
        };
        let rows = read_source(&o, b"Zo\xEB;\"a;b\"\r\nx;y").unwrap();
        assert_eq!(rows, [vec!["Zoë", "a;b"], vec!["x", "y"]]);
        o.delimiter = None;
        o.charset = Charset::MacRoman;
        assert_eq!(read_source(&o, b"Zo\x91\tb").unwrap(), [vec!["Zoë", "b"]]);
    }

    #[test]
    fn repetitions_keep_or_split_and_empty_matches() {
        let mut db = crate::export::tests::repeating_sample();
        let rows = vec![
            vec!["Name".to_string(), "Note".to_string()],
            vec!["Zed".into(), "a\u{1d}b\u{1d}c\u{1d}d".into()],
        ];
        // Kept in one record: as many repetitions as the field has.
        let r = import(&mut db, T, &[], rows.clone(), &opts(Format::Csv, Mode::Add)).unwrap();
        let rec = &db.tables[0].records[r.found[0]].values;
        assert_eq!(
            [
                &rec[&NOTE],
                &rec[&rep_key(NOTE, 2)],
                &rec[&rep_key(NOTE, 3)]
            ],
            ["a", "b", "c"]
        );
        // Split: one record per value, the other fields repeated.
        let mut o = opts(Format::Csv, Mode::Add);
        o.split_repetitions = true;
        let r = import(&mut db, T, &[], rows, &o).unwrap();
        assert_eq!(r.added.len(), 4);
        let notes: Vec<_> = r
            .found
            .iter()
            .map(|&i| db.tables[0].records[i].values[&NOTE].clone())
            .collect();
        assert_eq!(notes, ["a", "b", "c", "d"]);
        assert!(
            r.found
                .iter()
                .all(|&i| db.tables[0].records[i].values[&NAME] == "Zed")
        );
        // A row with empty match fields updates nothing.
        let mut db = sample();
        let all: Vec<usize> = (0..5).collect();
        let mode = Mode::Update {
            match_fields: vec!["Born".into()],
            add_remaining: false,
        };
        let rows = read_rows(Format::Csv, b"Born,City\r\n,Nowhere\r\n").unwrap();
        let r = import(&mut db, T, &all, rows, &opts(Format::Csv, mode)).unwrap();
        assert_eq!((r.updated.len(), r.skipped), (0, 1));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn folder_of_files() {
        let dir = std::env::temp_dir().join(format!("basecraft_folder_{}", std::process::id()));
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(600, 300)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.join("a.png"), png.get_ref()).unwrap();
        std::fs::write(sub.join("b.txt"), "hello").unwrap();
        std::fs::write(dir.join("skip.exe"), "x").unwrap();
        let mut f = Folder {
            path: dir.clone(),
            subfolders: false,
            kind: FolderKind::Pictures,
        };
        let rows = read_folder(&f, Charset::Utf8).unwrap();
        assert_eq!(
            rows[0],
            ["Image", "File Name", "File Path", "Image Thumbnail"]
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][1], "a.png");
        f.kind = FolderKind::Text;
        assert_eq!(read_folder(&f, Charset::Utf8).unwrap().len(), 1);
        f.subfolders = true;
        let text = read_folder(&f, Charset::Utf8).unwrap();
        assert_eq!(text[1][..2], ["hello", "b.txt"]);

        // Image and thumbnail into container fields; name and path as text.
        let mut db = sample();
        db.tables[0].fields[4] = crate::tests::field(NOTE, "Photo", 1, 6);
        db.tables[0].fields[3] = crate::tests::field(4, "Thumb", 1, 6);
        let mut o = opts(Format::Tab, Mode::Add);
        o.mapping = Mapping::ByOrder(vec![
            "Photo".into(),
            "Name".into(),
            "City".into(),
            "Thumb".into(),
        ]);
        o.container_files = Some(Files::Copy);
        let r = import(&mut db, T, &[], rows.clone(), &o).unwrap();
        let rec = r.found[0];
        let photo = db.value(T, rec, NOTE);
        let thumb = db.value(T, rec, 4);
        let (calc::Value::Container(p), calc::Value::Container(t)) = (&photo, &thumb) else {
            panic!("{photo:?} {thumb:?}");
        };
        assert_eq!(
            (p.name.as_str(), p.data.len()),
            ("a.png", png.get_ref().len())
        );
        let small = image::load_from_memory(&t.data).unwrap();
        assert_eq!((small.width(), small.height()), (256, 128));
        assert_eq!(db.tables[0].records[rec].values[&NAME], "a.png");
        assert!(db.tables[0].records[rec].values[&CITY].ends_with("/a.png"));
        // Only a reference.
        o.container_files = Some(Files::Reference);
        let r = import(&mut db, T, &[], rows, &o).unwrap();
        let v = &db.tables[0].records[r.found[0]].values[&NOTE];
        assert!(v.starts_with("file:") && v.ends_with("/a.png"), "{v}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mapping_errors() {
        let mut db = sample();
        let rows = vec![vec!["x".to_string()]];
        let mut o = opts(Format::Csv, Mode::Add);
        o.first_row_names = false;
        assert!(import(&mut db, T, &[], rows.clone(), &o).is_err());
        o.mapping = Mapping::ByOrder(vec!["Nope".into()]);
        assert!(import(&mut db, T, &[], rows, &o).is_err());
    }
}
