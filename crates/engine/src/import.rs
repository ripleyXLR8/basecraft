//! Import Records from comma-separated text, tab-separated text, Merge and
//! Excel `.xlsx`: map source columns to fields by name or by order, then
//! add records, update matching records, or replace the found set's data.

use fmp12::schema::{DataType, FieldKind};

use crate::export::{Format, GROUP_SEPARATOR, VERTICAL_TAB};
use crate::{Database, Rule, stored_text, typed};

/// Parses a source file into rows of cells. A vertical tab inside a value
/// becomes a return again.
pub fn read_rows(format: Format, bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let text = || {
        let s = String::from_utf8_lossy(bytes);
        s.strip_prefix('\u{FEFF}').unwrap_or(&s).to_string()
    };
    let vt = |s: &str| s.replace(VERTICAL_TAB, "\r");
    match format {
        Format::Csv | Format::Merge => Ok(csv(&text())
            .into_iter()
            .map(|r| r.iter().map(|c| vt(c)).collect())
            .collect()),
        Format::Tab => Ok(text()
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .map(|l| l.split('\t').map(vt).collect())
            .collect()),
        Format::Xlsx => crate::xlsx::read(bytes),
        Format::Xml | Format::Json => Err(format!("importing {format:?} is not supported")),
    }
}

/// Comma-separated values: quoted values may hold commas, doubled quotes
/// and line breaks; records end with CR, LF or CR LF.
fn csv(s: &str) -> Vec<Vec<String>> {
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
            (false, ',') => row.push(std::mem::take(&mut cell)),
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
#[derive(Debug, Clone, PartialEq)]
pub enum Mapping {
    /// Columns go to the fields named in the first row (needs
    /// `first_row_names`); unknown names are skipped.
    ByName,
    /// Column *i* goes to the *i*-th of these field names (an empty name
    /// skips the column), or, when the list is empty, to the *i*-th field
    /// of the table in definition order.
    ByOrder(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Add every row as a new record.
    Add,
    /// Update the records of the found set whose match fields equal the
    /// row's; rows without a match are added when `add_remaining`.
    Update {
        match_fields: Vec<String>,
        add_remaining: bool,
    },
    /// Replace the data of the found set's records, row *i* into record *i*;
    /// rows beyond the found set are added when `add_remaining`.
    Replace { add_remaining: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub format: Format,
    pub first_row_names: bool,
    pub mapping: Mapping,
    pub mode: Mode,
    /// Apply auto-enter options (serials, stamps, calculations) to new and
    /// changed records.
    pub auto_enter: bool,
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
            .map(|f| (f.id, f.data_type))
    };
    // Target (field, type) per source column.
    let targets: Vec<Option<(u32, DataType)>> = match &opts.mapping {
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
            .map(|f| Some((f.id, f.data_type)))
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
                        .any(|x| x.is_some_and(|(f, _)| f == id))
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
    let mut report = Report::default();
    for (k, row) in rows.iter().enumerate() {
        let line = first_row + k;
        let values: Vec<(u32, String)> = targets
            .iter()
            .zip(row)
            .filter_map(|(t, v)| {
                let (f, dt) = (*t)?;
                Some((f, convert(v, dt, opts.format, &db.locale)))
            })
            .collect();
        let get = |f: u32| {
            values
                .iter()
                .find(|(x, _)| *x == f)
                .map(|(_, v)| v.as_str())
        };
        // Records this row goes to; None = add a new one.
        let dest: Vec<usize> = match &opts.mode {
            Mode::Add => Vec::new(),
            Mode::Update { .. } => found
                .iter()
                .copied()
                .filter(|&r| {
                    match_ids.iter().all(|&f| {
                        let have = db.tables[ti].records[r]
                            .values
                            .get(&f)
                            .map_or("", |s| s.as_str());
                        have.eq_ignore_ascii_case(get(f).unwrap_or(""))
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
            for (f, v) in &values {
                if v.is_empty() {
                    db.tables[ti].records[r].values.remove(f);
                } else {
                    db.tables[ti].records[r].values.insert(*f, v.clone());
                }
            }
            if opts.auto_enter && !new {
                db.touch(table, r);
            }
            let mut rejected = false;
            for e in db.validate(table, r) {
                let always = db.tables[ti]
                    .field(e.field)
                    .is_some_and(|f| f.def.validation().always);
                rejected |= always;
                report.errors.push(RowError {
                    row: line,
                    field: e.field,
                    rule: e.rule,
                    skipped: always,
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
            }
            .write(format);
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
