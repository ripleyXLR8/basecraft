//! Export Records: a found set, in its sort order, with a chosen list of
//! fields (related fields included), as comma-separated text, tab-separated
//! text, Merge, FMPXMLRESULT XML, HTML table, Excel `.xlsx`, dBASE DBF, a
//! new FileMaker Pro file or JSON.
//!
//! Output conventions follow FileMaker's: in CSV, tab and Merge files a
//! return inside a field becomes a vertical tab (0x0B), and the values of
//! a repeating field, or of the records of a related field, are joined with
//! the group separator (0x1D). Records end with CR LF. Text is in the
//! chosen character set (UTF-8 by default). A container exports as its file
//! name (embedded) or its path (stored by reference).

use calc::Value;
use fmp12::schema::{DataType, FieldKind};

use crate::charset::{self, Charset};
use crate::{Database, rep_key, repetitions, stored_text};

/// Value separator for repetitions and related records (ASCII GS).
pub const GROUP_SEPARATOR: char = '\u{1D}';
/// What a return inside a field becomes in text exports (ASCII VT).
pub const VERTICAL_TAB: char = '\u{0B}';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Comma-separated text (`.csv`).
    #[default]
    Csv,
    /// Tab-separated text (`.tab`, `.txt`).
    Tab,
    /// Merge (`.mer`): CSV with a row of field names first.
    Merge,
    /// FMPXMLRESULT (`.xml`).
    Xml,
    /// Excel workbook (`.xlsx`).
    Xlsx,
    /// JSON array of records (a BaseCraft extra).
    Json,
    /// HTML table (`.htm`): a header row of field names, one row per record.
    Html,
    /// dBASE III DBF (`.dbf`).
    Dbf,
    /// A new FileMaker Pro file (`.fmp12`) with one table of the exported fields.
    Fmp12,
}

impl Format {
    /// Format from a name or file extension (`csv`, `tab`, `txt`, `mer`,
    /// `xml`, `xlsx`, `json`) or a path ending with one.
    pub fn from_name(s: &str) -> Option<Format> {
        let ext = s.rsplit('.').next().unwrap_or(s).to_ascii_lowercase();
        Some(match ext.as_str() {
            "csv" => Format::Csv,
            "tab" | "txt" | "tsv" => Format::Tab,
            "mer" | "merge" => Format::Merge,
            "xml" | "fmpxmlresult" => Format::Xml,
            "xlsx" | "excel" => Format::Xlsx,
            "json" => Format::Json,
            "htm" | "html" => Format::Html,
            "dbf" => Format::Dbf,
            "fmp12" => Format::Fmp12,
            _ => return None,
        })
    }
}

/// One exported field: a field of the context occurrence or of a related one.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Table occurrence the field is seen through.
    pub to: u32,
    pub table: u32,
    pub field: u32,
    /// `Field`, or `Occurrence::Field` for a related field.
    pub name: String,
    pub data_type: DataType,
}

/// Resolves `Field` and `Occurrence::Field` names from occurrence `to`.
/// No names: every field of the occurrence's table, in definition order.
pub fn columns(db: &Database, to: u32, names: &[String]) -> Result<Vec<Column>, String> {
    let base = db
        .table_of(to)
        .ok_or_else(|| format!("no table occurrence {to}"))?;
    if names.is_empty() {
        return Ok(base
            .fields
            .iter()
            .map(|f| Column {
                to,
                table: base.id,
                field: f.id,
                name: f.name.clone(),
                data_type: f.data_type,
            })
            .collect());
    }
    names
        .iter()
        .map(|n| {
            let (occ, fname) = match n.split_once("::") {
                Some((o, f)) => {
                    let occ = db
                        .occurrences
                        .iter()
                        .find(|x| x.name.eq_ignore_ascii_case(o))
                        .ok_or_else(|| format!("no table occurrence named {o:?}"))?;
                    (occ.id, f)
                }
                None => (to, n.as_str()),
            };
            let t = db.table_of(occ).ok_or("occurrence without a table")?;
            let f = t
                .field_by_name(fname)
                .ok_or_else(|| format!("no field {fname:?} in table {:?}", t.name))?;
            let name = if occ == to {
                f.name.clone()
            } else {
                format!(
                    "{}::{}",
                    db.occurrences
                        .iter()
                        .find(|x| x.id == occ)
                        .map_or("", |x| &x.name),
                    f.name
                )
            };
            Ok(Column {
                to: occ,
                table: t.id,
                field: f.id,
                name,
                data_type: f.data_type,
            })
        })
        .collect()
}

/// Values of `field` of occurrence `field_to` for record `record` seen from
/// occurrence `from`: one value for the record's own field, one per related
/// record otherwise. Summary fields summarize `found` (running ones up to
/// the record).
pub fn values_through(
    db: &Database,
    from: u32,
    record: usize,
    field_to: u32,
    field: u32,
    found: &[usize],
) -> Vec<Value> {
    let Some(t) = db.table_of(field_to) else {
        return Vec::new();
    };
    if field_to == from {
        let summary = t.field(field).is_some_and(|f| f.kind == FieldKind::Summary);
        return vec![if summary {
            crate::summary_at(db, t.id, field, record, found, &[])
        } else {
            db.value(t.id, record, field)
        }];
    }
    db.related(from, record, field_to)
        .into_iter()
        .map(|r| db.value(t.id, r, field))
        .collect()
}

/// Export Records' options, as its dialog and the script step's stored
/// options give them: what [`export`] needs besides the database.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options {
    pub format: Format,
    /// Output file character set (CSV, tab, Merge, DBF).
    pub charset: Charset,
    /// "Apply current layout's data formatting".
    pub formatted: bool,
    /// `Field` or `Occurrence::Field` names, in export order; empty: every
    /// field of the context table.
    pub fields: Vec<String>,
    /// "Group by" fields (sort fields of the context table, outermost first).
    pub group_by: Vec<String>,
}

/// Exports `found` (in sort order) of occurrence `to` with `opts`; `layout`
/// is the current layout's id (its field formats, when `opts.formatted`).
pub fn export(
    db: &Database,
    to: u32,
    found: &[usize],
    layout: Option<u32>,
    file_name: &str,
    opts: &Options,
) -> Result<Vec<u8>, String> {
    let cols = columns(db, to, &opts.fields)?;
    let table = db
        .table_of(to)
        .ok_or_else(|| format!("no table occurrence {to}"))?;
    let group: Vec<u32> = opts
        .group_by
        .iter()
        .map(|n| {
            table
                .field_by_name(n)
                .map(|f| f.id)
                .ok_or_else(|| format!("no field {n:?} in table {:?}", table.name))
        })
        .collect::<Result<_, String>>()?;
    Export {
        db,
        to,
        found,
        columns: &cols,
        formatted: opts.formatted,
        file_name,
        group_by: &group,
        charset: opts.charset,
        layout,
    }
    .write(opts.format)
}

/// An export job: what Export Records takes besides the format.
#[derive(Clone)]
pub struct Export<'a> {
    pub db: &'a Database,
    /// Context table occurrence (the layout's).
    pub to: u32,
    /// Found set, in sort order (record indexes).
    pub found: &'a [usize],
    pub columns: &'a [Column],
    /// "Apply current layout's data formatting": values as the field
    /// objects of `layout` format them (number, date, time formats), other
    /// values in the file's formats, instead of as stored.
    pub formatted: bool,
    /// File name for FMPXMLRESULT's `DATABASE NAME`.
    pub file_name: &'a str,
    /// "Group by" fields (the sort fields, outermost first): a grouped
    /// field's value is written in the first record of its group only.
    pub group_by: &'a [u32],
    /// Output file character set (CSV, tab, Merge, DBF).
    pub charset: Charset,
    /// Current layout (its id): its field objects' data formats.
    pub layout: Option<u32>,
}

impl Export<'_> {
    /// Text of every cell: rows × columns × values. A repeating field's
    /// empty repetitions at the end are left out.
    pub fn cells(&self) -> Vec<Vec<Vec<String>>> {
        self.cells_with(false)
    }

    /// Record exported on each row (a group's first record when grouped).
    fn row_records(&self) -> Vec<usize> {
        if self.group_by.is_empty() {
            return self.found.to_vec();
        }
        let table = self.db.table_of(self.to).map_or(0, |t| t.id);
        crate::groups(self.db, table, self.found, self.group_by)
            .iter()
            .map(|g| self.found[g.start])
            .collect()
    }

    /// `all_reps`: every repetition of a repeating field, empty ones included.
    fn cells_with(&self, all_reps: bool) -> Vec<Vec<Vec<String>>> {
        let row = |r: usize| -> Vec<Vec<String>> {
            self.columns
                .iter()
                .map(|c| self.texts(r, c, all_reps))
                .collect()
        };
        if self.group_by.is_empty() {
            return self.found.iter().map(|&r| row(r)).collect();
        }
        // Grouped: one row per group, with the values of its first record;
        // summary fields hold the grand total on the first row only.
        let table = self.db.table_of(self.to).map_or(0, |t| t.id);
        let groups = crate::groups(self.db, table, self.found, self.group_by);
        groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let mut cells = row(self.found[g.start]);
                for (ci, c) in self.columns.iter().enumerate() {
                    let summary = c.to == self.to
                        && self
                            .db
                            .table(c.table)
                            .and_then(|t| t.field(c.field))
                            .is_some_and(|f| f.kind == FieldKind::Summary);
                    if summary {
                        cells[ci] = vec![if i == 0 {
                            let v =
                                crate::summarize(self.db, c.table, c.field, self.found, self.found);
                            stored_text(&v, &self.db.locale)
                        } else {
                            String::new()
                        }];
                    }
                }
                cells
            })
            .collect()
    }

    /// The data format of the current layout's first object showing the
    /// column's field, when the export applies the layout's formatting.
    fn layout_format(&self, c: &Column) -> Option<&fmp12::formats::Format> {
        if !self.formatted {
            return None;
        }
        let l = self.db.layouts.iter().find(|l| Some(l.id) == self.layout)?;
        l.objects
            .iter()
            .find(|o| {
                o.format.is_some()
                    && o.field.is_some_and(|(to, f, _)| {
                        (to & 0xFFF) as u32 == c.to && f as u32 == c.field
                    })
            })?
            .format
            .as_ref()
    }

    /// Whether a column is a repeating field of the context table.
    fn repeating(&self, c: &Column) -> bool {
        c.to == self.to
            && self
                .db
                .table(c.table)
                .and_then(|t| t.field(c.field))
                .is_some_and(|f| repetitions(f) > 1)
    }

    fn texts(&self, record: usize, c: &Column, all_reps: bool) -> Vec<String> {
        let loc = &self.db.locale;
        let fmt = self.layout_format(c);
        let text = |v: &Value| match fmt.and_then(|f| crate::format::apply(f, v, loc)) {
            Some((s, _)) => s,
            None => stored_text(v, loc),
        };
        let through = || values_through(self.db, self.to, record, c.to, c.field, self.found);
        let (Some(t), true) = (self.db.table(c.table), c.to == self.to) else {
            return through().iter().map(text).collect();
        };
        let Some(f) = t.field(c.field) else {
            return vec![String::new()];
        };
        // Unformatted, a normal field exports as stored (a container as
        // its file name or path).
        let raw =
            !self.formatted && f.kind == FieldKind::Normal && f.data_type != DataType::Container;
        let mut out: Vec<String> = (1..=repetitions(f))
            .map(|n| {
                if raw {
                    return t
                        .raw(record, rep_key(c.field, n))
                        .cloned()
                        .unwrap_or_default();
                }
                let v = if n == 1 {
                    through().into_iter().next().unwrap_or_else(Value::empty)
                } else {
                    self.db.value_rep(c.table, record, c.field, n)
                };
                text(&v)
            })
            .collect();
        if !all_reps {
            while out.len() > 1 && out.last().is_some_and(|s| s.is_empty()) {
                out.pop();
            }
        }
        out
    }

    /// The exported file. Fails for a DBF export beyond the format's limits
    /// and when the new FileMaker file cannot be built.
    pub fn write(&self, format: Format) -> Result<Vec<u8>, String> {
        let enc = |s: String| charset::encode(&s, self.charset);
        Ok(match format {
            Format::Csv => enc(self.delimited(false)),
            Format::Merge => enc(self.delimited(true)),
            Format::Tab => enc(self.tab()),
            Format::Xml => self.xml().into_bytes(),
            Format::Json => self.json().into_bytes(),
            Format::Xlsx => self.xlsx(),
            Format::Html => self.html(),
            Format::Dbf => self.dbf()?,
            Format::Fmp12 => self.fmp12()?,
        })
    }

    /// dBASE DBF: the first repetition of each field, never formatted
    /// (FileMaker dims "Apply current layout's data formatting" for it).
    fn dbf(&self) -> Result<Vec<u8>, String> {
        let plain = Export {
            formatted: false,
            ..self.clone()
        };
        let cols: Vec<(String, DataType)> = self
            .columns
            .iter()
            .map(|c| (c.name.clone(), c.data_type))
            .collect();
        let rows: Vec<Vec<String>> = plain
            .cells()
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|v| v.into_iter().next().unwrap_or_default())
                    .collect()
            })
            .collect();
        crate::dbf::write(&cols, &rows, &self.db.locale, self.charset)
    }

    /// A new FileMaker Pro file: one table named after the context table,
    /// one field per column (its type; a repeating field keeps its
    /// repetitions; a related field named `Occurrence_Field`), one record
    /// per row. Global fields become normal fields, as FileMaker documents.
    // ponytail: the new file's layout shows no field objects (the file
    // builder does not place them); they can be added in Layout mode.
    fn fmp12(&self) -> Result<Vec<u8>, String> {
        use fmp12::schema_edit::{Editor, FieldSpec, new_file};
        let name = self
            .db
            .table_of(self.to)
            .map_or("Export".to_string(), |t| t.name.clone());
        let bytes = new_file(&name).map_err(|e| e.to_string())?;
        let file = fmp12::File::from_bytes(bytes).map_err(|e| e.to_string())?;
        let mut ed = Editor::open(&file).map_err(|e| e.to_string())?;
        let table = ed.table_id(&name).ok_or("the new file has no table")?;
        let mut ids = Vec::new();
        for c in self.columns {
            let mut spec = FieldSpec::new(&c.name.replace("::", "_"), c.data_type);
            if self.repeating(c) {
                spec.storage.repetitions = self
                    .db
                    .table(c.table)
                    .and_then(|t| t.field(c.field))
                    .map_or(1, |f| repetitions(f) as u16);
            }
            ids.push(ed.add_field(table, &spec).map_err(|e| e.to_string())?);
        }
        let file = fmp12::File::from_bytes(ed.finish()).map_err(|e| e.to_string())?;
        let mut out = Database::load(&file).map_err(|e| e.to_string())?;
        let ti = out
            .tables
            .iter()
            .position(|t| t.id == table)
            .ok_or("the new file has no table")?;
        let plain = Export {
            formatted: false,
            ..self.clone()
        };
        for (&r, row) in self.row_records().iter().zip(plain.cells_with(true)) {
            let nr = out.tables[ti].new_record();
            for ((c, &id), texts) in self.columns.iter().zip(&ids).zip(row) {
                let reps = if self.repeating(c) { texts.len() } else { 1 };
                for (i, s) in texts.iter().take(reps).enumerate() {
                    let rep = i as u32 + 1;
                    let v = match (c.data_type, c.to == self.to) {
                        (DataType::Container, true) => self.db.value_rep(c.table, r, c.field, rep),
                        (DataType::Container, false) => {
                            values_through(self.db, self.to, r, c.to, c.field, self.found)
                                .into_iter()
                                .next()
                                .unwrap_or_else(Value::empty)
                        }
                        _ => crate::typed_in(s, c.data_type, &self.db.locale),
                    };
                    if v.is_empty() {
                        continue;
                    }
                    let text = out.store_value(table, id, &v);
                    out.tables[ti].records[nr]
                        .values
                        .insert(rep_key(id, rep), text);
                }
            }
        }
        out.save(&file).map_err(|e| e.to_string())
    }

    /// HTML table as FileMaker writes it (ISO-8859-1, CR LF): a row of
    /// field names, one row per record; returns become `<BR>` and an empty
    /// value is `<BR>`. Characters outside Latin-1 become `&#n;`.
    fn html(&self) -> Vec<u8> {
        let cell = |s: &str| {
            let mut out = String::new();
            for c in s.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '"' => out.push_str("&quot;"),
                    '\r' | '\n' => out.push_str("<BR>"),
                    c if (c as u32) > 0xFF => out.push_str(&format!("&#{};", c as u32)),
                    c => out.push(c),
                }
            }
            out
        };
        let mut out = String::from(
            "<HTML>\r\n<HEAD><META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;CHARSET=ISO-8859-1\"></HEAD>\r\n<BODY><TABLE BORDER=1>\r\n<TR>\r\n",
        );
        for c in self.columns {
            out.push_str(&format!("<TH>{}</TH>\r\n", cell(&c.name)));
        }
        out.push_str("</TR>\r\n");
        for row in self.cells() {
            out.push_str("<TR>\r\n");
            for (c, v) in self.columns.iter().zip(row) {
                // A repeating field: a nested table, one column per value.
                if self.repeating(c) && v.len() > 1 {
                    let tds: String = v.iter().map(|x| format!("<TD>{}</TD>", cell(x))).collect();
                    out.push_str(&format!("<TD><TABLE><TR>{tds}</TR></TABLE></TD>\r\n"));
                    continue;
                }
                let v = cell(&v.join("\r"));
                let v = if v.is_empty() { "<BR>".to_string() } else { v };
                out.push_str(&format!("<TD>{v}</TD>\r\n"));
            }
            out.push_str("</TR>\r\n");
        }
        out.push_str("</TABLE></BODY></HTML>\r\n");
        // Latin-1: every char is below 0x100 here.
        out.chars().map(|c| c as u32 as u8).collect()
    }

    fn delimited(&self, header: bool) -> String {
        let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::new();
        if header {
            let names: Vec<String> = self.columns.iter().map(|c| quote(&c.name)).collect();
            out.push_str(&names.join(","));
            out.push_str("\r\n");
        }
        for row in self.cells() {
            let cols: Vec<String> = row.iter().map(|v| quote(&joined(v))).collect();
            out.push_str(&cols.join(","));
            out.push_str("\r\n");
        }
        out
    }

    fn tab(&self) -> String {
        let mut out = String::new();
        for row in self.cells() {
            let cols: Vec<String> = row.iter().map(|v| joined(v).replace('\t', " ")).collect();
            out.push_str(&cols.join("\t"));
            out.push_str("\r\n");
        }
        out
    }

    fn xml(&self) -> String {
        use fmp12::schema::DateOrder;
        let loc = &self.db.locale;
        let s = loc.date_separator;
        let date = match loc.date_order {
            DateOrder::Mdy => format!("M{s}d{s}yyyy"),
            DateOrder::Dmy => format!("dd{s}MM{s}yyyy"),
            DateOrder::Ymd => format!("yyyy{s}MM{s}dd"),
        };
        let ts = loc.time_separator;
        let time = if loc.am.is_empty() {
            format!("HH{ts}mm{ts}ss")
        } else {
            format!("h{ts}mm{ts}ss a")
        };
        let table = self.db.table_of(self.to);
        let layout = self
            .db
            .layouts
            .iter()
            .find(|l| (l.table_occurrence & 0xFFF) as u32 == self.to)
            .map_or("", |l| l.name.as_str());
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n");
        out.push_str("<FMPXMLRESULT xmlns=\"http://www.filemaker.com/fmpxmlresult\">");
        out.push_str("<ERRORCODE>0</ERRORCODE>");
        out.push_str(&format!(
            "<PRODUCT BUILD=\"\" NAME=\"BaseCraft\" VERSION=\"{}\"/>",
            env!("CARGO_PKG_VERSION")
        ));
        out.push_str(&format!(
            "<DATABASE DATEFORMAT=\"{date}\" LAYOUT=\"{}\" NAME=\"{}\" RECORDS=\"{}\" TIMEFORMAT=\"{time}\"/>",
            xml_escape(layout),
            xml_escape(self.file_name),
            table.map_or(0, |t| t.records.len())
        ));
        out.push_str("<METADATA>");
        for c in self.columns {
            let def = self.db.table(c.table).and_then(|t| t.field(c.field));
            let empty_ok = def.is_none_or(|f| !f.def.validation().not_empty);
            let repeat = def.map_or(1, |f| f.def.max_repetitions().max(1));
            let ty = match c.data_type {
                DataType::Number => "NUMBER",
                DataType::Date => "DATE",
                DataType::Time => "TIME",
                DataType::Timestamp => "TIMESTAMP",
                DataType::Container => "CONTAINER",
                _ => "TEXT",
            };
            out.push_str(&format!(
                "<FIELD EMPTYOK=\"{}\" MAXREPEAT=\"{repeat}\" NAME=\"{}\" TYPE=\"{ty}\"/>",
                if empty_ok { "YES" } else { "NO" },
                xml_escape(&c.name)
            ));
        }
        out.push_str("</METADATA>");
        out.push_str(&format!("<RESULTSET FOUND=\"{}\">", self.found.len()));
        // Every repetition, as MAXREPEAT says.
        for (&r, row) in self.row_records().iter().zip(self.cells_with(true)) {
            let id = table.and_then(|t| t.records.get(r)).map_or(0, |x| x.id);
            // ponytail: modification counts are not kept per record; MODID is 0.
            out.push_str(&format!("<ROW MODID=\"0\" RECORDID=\"{id}\">"));
            for values in row {
                out.push_str("<COL>");
                if values.is_empty() {
                    out.push_str("<DATA></DATA>");
                }
                for v in values {
                    out.push_str(&format!("<DATA>{}</DATA>", xml_escape(&v)));
                }
                out.push_str("</COL>");
            }
            out.push_str("</ROW>");
        }
        out.push_str("</RESULTSET></FMPXMLRESULT>");
        out
    }

    fn json(&self) -> String {
        let value = |c: &Column, v: &str| {
            let numeric = c.data_type == DataType::Number && is_json_number(v);
            if numeric {
                v.to_string()
            } else {
                json_string(v)
            }
        };
        let rows: Vec<String> = self
            .cells()
            .into_iter()
            .map(|row| {
                let fields: Vec<String> = self
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(c, vals)| {
                        let v = if c.to == self.to && vals.len() == 1 {
                            value(c, &vals[0])
                        } else {
                            let items: Vec<String> = vals.iter().map(|v| value(c, v)).collect();
                            format!("[{}]", items.join(","))
                        };
                        format!("{}:{v}", json_string(&c.name))
                    })
                    .collect();
                format!("{{{}}}", fields.join(","))
            })
            .collect();
        format!("[{}]\n", rows.join(",\n"))
    }

    fn xlsx(&self) -> Vec<u8> {
        use crate::xlsx::Cell;
        let mut rows = vec![
            self.columns
                .iter()
                .map(|c| Cell::Text(c.name.clone()))
                .collect::<Vec<_>>(),
        ];
        for (&r, texts) in self.row_records().iter().zip(self.cells()) {
            let row = self
                .columns
                .iter()
                .zip(texts)
                .map(|(c, texts)| {
                    let vals = values_through(self.db, self.to, r, c.to, c.field, self.found);
                    match vals.as_slice() {
                        [v] if !v.is_empty()
                            && !self.formatted
                            && texts.len() == 1
                            && c.data_type != DataType::Container =>
                        {
                            Cell::of(v)
                        }
                        _ => Cell::Text(texts.join("\n")),
                    }
                })
                .collect();
            rows.push(row);
        }
        crate::xlsx::write(&rows)
    }
}

/// Values of one cell joined with the group separator, returns as VT.
fn joined(values: &[String]) -> String {
    let s = values.join(&GROUP_SEPARATOR.to_string());
    s.replace("\r\n", "\r")
        .replace(['\r', '\n'], &VERTICAL_TAB.to_string())
}

pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            // Control characters other than tab and LF are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `-?digits[.digits]`, without leading zeros: valid as a JSON number.
fn is_json_number(s: &str) -> bool {
    let d = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = d.split_once('.').unwrap_or((d, "0"));
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    digits(int) && digits(frac) && (int == "0" || !int.starts_with('0'))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tests::{AMOUNT, T, sample};

    fn run(format: Format, names: &[&str], formatted: bool) -> String {
        let db = sample();
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        let cols = columns(&db, T, &names).unwrap();
        let found = [0, 1, 2];
        let e = Export {
            db: &db,
            to: T,
            found: &found,
            columns: &cols,
            formatted,
            file_name: "People.fmp12",
            group_by: &[],
            charset: Default::default(),
            layout: None,
        };
        String::from_utf8(e.write(format).unwrap()).unwrap()
    }

    #[test]
    fn text_formats() {
        let cases: [(Format, &[&str], &str); 5] = [
            (
                Format::Csv,
                &["Name", "Amount"],
                "\"Ada\",\"10\"\r\n\"Bob \"\"B\"\"\",\"20\"\r\n\"Cy\",\"30\"\r\n",
            ),
            (
                Format::Csv,
                &["Note"],
                "\"line 1\u{b}line 2\"\r\n\"\"\r\n\"a,b\"\r\n",
            ),
            (
                Format::Csv,
                &["Name", "Pets::Pet"],
                "\"Ada\",\"Cat\u{1d}Dog\"\r\n\"Bob \"\"B\"\"\",\"\"\r\n\"Cy\",\"Fish\"\r\n",
            ),
            (
                Format::Tab,
                &["Name", "Note"],
                "Ada\tline 1\u{b}line 2\r\nBob \"B\"\t\r\nCy\ta,b\r\n",
            ),
            (
                Format::Merge,
                &["Name", "Pets::Pet"],
                "\"Name\",\"Pets::Pet\"\r\n\"Ada\",\"Cat\u{1d}Dog\"\r\n\"Bob \"\"B\"\"\",\"\"\r\n\"Cy\",\"Fish\"\r\n",
            ),
        ];
        for (format, names, want) in cases {
            assert_eq!(run(format, names, false), want, "{format:?} {names:?}");
        }
    }

    #[test]
    fn html_and_grouped() {
        // As FileMaker 26 writes it (Latin-1, CR LF, <BR> for returns and empty values).
        let html = run(Format::Html, &["Name", "Note"], false);
        assert_eq!(
            html,
            "<HTML>\r\n<HEAD><META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;CHARSET=ISO-8859-1\"></HEAD>\r\n<BODY><TABLE BORDER=1>\r\n<TR>\r\n<TH>Name</TH>\r\n<TH>Note</TH>\r\n</TR>\r\n\
             <TR>\r\n<TD>Ada</TD>\r\n<TD>line 1<BR>line 2</TD>\r\n</TR>\r\n\
             <TR>\r\n<TD>Bob &quot;B&quot;</TD>\r\n<TD><BR></TD>\r\n</TR>\r\n\
             <TR>\r\n<TD>Cy</TD>\r\n<TD>a,b</TD>\r\n</TR>\r\n</TABLE></BODY></HTML>\r\n"
        );
        // Grouped by City (sorted): one row per city, its first record's
        // values; a summary field holds the grand total on the first row.
        let mut db = sample();
        crate::tests::summary_field(&mut db, fmp12::schema::SummaryOp::Total, AMOUNT);
        let cols = columns(&db, T, &["City".into(), "Name".into(), "Total".into()]).unwrap();
        let mut found = vec![0, 1, 2, 3, 4];
        crate::sort(
            &db,
            T,
            &mut found,
            &[(crate::tests::CITY, crate::Order::Ascending)],
        );
        let e = Export {
            db: &db,
            to: T,
            found: &found,
            columns: &cols,
            formatted: false,
            file_name: "",
            group_by: &[crate::tests::CITY],
            charset: Default::default(),
            layout: None,
        };
        let csv = String::from_utf8(e.write(Format::Csv).unwrap()).unwrap();
        assert_eq!(
            csv,
            "\"Oslo\",\"Di\",\"100\"\r\n\"Paris\",\"Bob \"\"B\"\"\",\"\"\r\n\"Rome\",\"Ada\",\"\"\r\n"
        );
    }

    #[test]
    fn xml_and_json() {
        let xml = run(Format::Xml, &["Name", "Amount", "Pets::Pet"], false);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<FMPXMLRESULT xmlns=\"http://www.filemaker.com/fmpxmlresult\"><ERRORCODE>0</ERRORCODE>"));
        assert!(xml.contains("<DATABASE DATEFORMAT=\"M/d/yyyy\" LAYOUT=\"\" NAME=\"People.fmp12\" RECORDS=\"5\" TIMEFORMAT=\"h:mm:ss a\"/>"));
        assert!(
            xml.contains(
                "<FIELD EMPTYOK=\"YES\" MAXREPEAT=\"1\" NAME=\"Amount\" TYPE=\"NUMBER\"/>"
            )
        );
        assert!(xml.contains("<RESULTSET FOUND=\"3\"><ROW MODID=\"0\" RECORDID=\"1\"><COL><DATA>Ada</DATA></COL><COL><DATA>10</DATA></COL><COL><DATA>Cat</DATA><DATA>Dog</DATA></COL></ROW>"));
        assert!(xml.contains("<COL><DATA>Bob &quot;B&quot;</DATA></COL><COL><DATA>20</DATA></COL><COL><DATA></DATA></COL>"));
        roxmltree::Document::parse(&xml).unwrap();

        let json = run(Format::Json, &["Name", "Amount", "Pets::Pet"], false);
        assert_eq!(
            json.lines().next().unwrap(),
            r#"[{"Name":"Ada","Amount":10,"Pets::Pet":["Cat","Dog"]},"#
        );
        assert!(json.contains(r#"{"Name":"Bob \"B\"","Amount":20,"Pets::Pet":[]}"#));
    }

    #[test]
    fn formatted_and_raw_values() {
        let mut db = sample();
        db.tables[0].records[0]
            .values
            .insert(AMOUNT, "010.50".into());
        let cols = columns(&db, T, &["Amount".into(), "Born".into()]).unwrap();
        let found = [0];
        let mut e = Export {
            db: &db,
            to: T,
            found: &found,
            columns: &cols,
            formatted: false,
            file_name: "",
            group_by: &[],
            charset: Default::default(),
            layout: None,
        };
        assert_eq!(e.write(Format::Tab).unwrap(), b"010.50\t12/10/1815\r\n");
        e.formatted = true;
        assert_eq!(e.write(Format::Tab).unwrap(), b"10.5\t12/10/1815\r\n");
    }

    /// Note (field 5) with three repetitions.
    pub(crate) fn repeating_sample() -> Database {
        let mut db = sample();
        let o = &mut db.tables[0].fields[4].def.options;
        o.resize(26, 0);
        o[25] = 3;
        db.tables[0].records[0]
            .values
            .insert(rep_key(5, 3), "third".into());
        db
    }

    fn export_with(db: &Database, names: &[&str], format: Format, opts: Options) -> Vec<u8> {
        let o = Options {
            format,
            fields: names.iter().map(|s| s.to_string()).collect(),
            ..opts
        };
        export(db, T, &[0, 1], Some(1), "People.fmp12", &o).unwrap()
    }

    #[test]
    fn repeating_fields_per_format() {
        let db = repeating_sample();
        let txt =
            |f| String::from_utf8(export_with(&db, &["Note"], f, Options::default())).unwrap();
        // Group separator between repetitions; empty ones at the end left out.
        assert_eq!(
            txt(Format::Csv),
            "\"line 1\u{b}line 2\u{1d}\u{1d}third\"\r\n\"\"\r\n"
        );
        assert_eq!(
            txt(Format::Tab),
            "line 1\u{b}line 2\u{1d}\u{1d}third\r\n\r\n"
        );
        // XML: one DATA per repetition (MAXREPEAT of them).
        let xml = txt(Format::Xml);
        assert!(xml.contains("MAXREPEAT=\"3\" NAME=\"Note\""));
        assert!(
            xml.contains(
                "<COL><DATA>line 1&#13;line 2</DATA><DATA></DATA><DATA>third</DATA></COL>"
            )
        );
        assert!(xml.contains("<COL><DATA></DATA><DATA></DATA><DATA></DATA></COL>"));
        // HTML: a nested table, one column per value.
        let html = txt(Format::Html);
        assert!(html.contains(
            "<TD><TABLE><TR><TD>line 1<BR>line 2</TD><TD></TD><TD>third</TD></TR></TABLE></TD>"
        ));
        // DBF: the first repetition only.
        let dbf = export_with(&db, &["Note"], Format::Dbf, Options::default());
        let rows = crate::dbf::read(&dbf, Charset::Utf8).unwrap();
        assert_eq!(rows[0], ["NOTE"]);
        assert_eq!(rows[1], ["line 1\rline 2"]);
    }

    #[test]
    fn containers_charsets_and_layout_formats() {
        let mut db = sample();
        db.tables[0].fields[4] = crate::tests::field(5, "Photo", 1, 6);
        db.insert_file(T, 0, 5, 1, "cat.png", vec![1, 2, 3]);
        db.tables[0].records[1]
            .values
            .insert(5, "file:/C:/pics/dog.jpg".into());
        let csv = export_with(&db, &["Photo"], Format::Csv, Options::default());
        assert_eq!(csv, b"\"cat.png\"\r\n\"file:/C:/pics/dog.jpg\"\r\n");

        db.tables[0].records[0].values.insert(1, "Zoë".into());
        let cs = |c| Options {
            charset: c,
            ..Options::default()
        };
        assert_eq!(
            export_with(&db, &["Name"], Format::Tab, cs(Charset::WindowsAnsi)),
            b"Zo\xEB\r\nBob \"B\"\r\n"
        );
        assert_eq!(
            export_with(&db, &["Name"], Format::Tab, cs(Charset::MacRoman))[..3],
            *b"Zo\x91"
        );
        assert_eq!(
            export_with(&db, &["Name"], Format::Tab, cs(Charset::Utf16))[..4],
            [0xFF, 0xFE, b'Z', 0]
        );

        // Amount shown with two fixed decimals on layout 1.
        let mut f = fmp12::formats::Format::new_default();
        f.number.flags = fmp12::formats::num::FIXED_DIGITS;
        f.number.digits = 2;
        f.number.decimal = '.';
        db.layouts.push(fmp12::layouts::Layout {
            id: 1,
            table_occurrence: T as u64,
            objects: vec![fmp12::layouts::LayoutObject {
                field: Some((T as u64, AMOUNT as u64, 1)),
                format: Some(f),
                ..Default::default()
            }],
            ..Default::default()
        });
        let formatted = Options {
            formatted: true,
            ..Options::default()
        };
        assert_eq!(
            export_with(&db, &["Amount"], Format::Tab, formatted.clone()),
            b"10.00\r\n20.00\r\n"
        );
        assert_eq!(
            export_with(&db, &["Amount"], Format::Tab, Options::default()),
            b"10\r\n20\r\n"
        );
        // DBF never applies the layout's formatting.
        let dbf = export_with(&db, &["Amount"], Format::Dbf, formatted);
        assert_eq!(crate::dbf::read(&dbf, Charset::Utf8).unwrap()[1], ["10"]);
    }

    /// A new FileMaker file holding the exported fields, read back as an
    /// import source.
    #[test]
    fn fmp12_export() {
        let db = repeating_sample();
        let o = Options {
            format: Format::Fmp12,
            fields: vec![
                "Name".into(),
                "Amount".into(),
                "Note".into(),
                "Pets::Pet".into(),
            ],
            ..Options::default()
        };
        let bytes = export(&db, T, &[0, 2], None, "People.fmp12", &o).unwrap();
        let tables = crate::import::fmp12_source(&bytes, &db.locale).unwrap();
        let (name, rows) = tables.iter().find(|t| t.0 == "People").unwrap();
        assert_eq!(name, "People");
        let col = |n: &str| rows[0].iter().position(|x| x == n).unwrap();
        let (n, a, note, pet) = (col("Name"), col("Amount"), col("Note"), col("Pets_Pet"));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            [&rows[1][n], &rows[1][a], &rows[1][note], &rows[1][pet]],
            ["Ada", "10", "line 1\rline 2\u{1d}\u{1d}third", "Cat"]
        );
        assert_eq!([&rows[2][n], &rows[2][note]], ["Cy", "a,b"]);
    }

    #[test]
    fn unknown_fields_are_errors() {
        let db = sample();
        assert!(columns(&db, T, &["Nope".into()]).is_err());
        assert!(columns(&db, T, &["Zoo::Name".into()]).is_err());
        assert_eq!(columns(&db, T, &[]).unwrap().len(), 5);
        assert_eq!(Format::from_name("out.MER"), Some(Format::Merge));
    }
}
