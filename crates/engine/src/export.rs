//! Export Records: a found set, in its sort order, with a chosen list of
//! fields (related fields included), as comma-separated text, tab-separated
//! text, Merge, FMPXMLRESULT XML, Excel `.xlsx` or JSON.
//!
//! Output conventions follow FileMaker's: in CSV, tab and Merge files a
//! return inside a field becomes a vertical tab (0x0B), and the values of
//! a repeating field, or of the records of a related field, are joined with
//! the group separator (0x1D). Records end with CR LF. Text is UTF-8.

use calc::Value;
use fmp12::schema::{DataType, FieldKind};

use crate::{Database, stored_text};

/// Value separator for repetitions and related records (ASCII GS).
pub const GROUP_SEPARATOR: char = '\u{1D}';
/// What a return inside a field becomes in text exports (ASCII VT).
pub const VERTICAL_TAB: char = '\u{0B}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Comma-separated text (`.csv`).
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

/// An export job: what Export Records takes besides the format.
pub struct Export<'a> {
    pub db: &'a Database,
    /// Context table occurrence (the layout's).
    pub to: u32,
    /// Found set, in sort order (record indexes).
    pub found: &'a [usize],
    pub columns: &'a [Column],
    /// Values as displayed (in the file's formats) instead of as stored.
    /// ponytail: layout-specific number/date formats are not decoded, so
    /// "formatted" means the file's locale formats.
    pub formatted: bool,
    /// File name for FMPXMLRESULT's `DATABASE NAME`.
    pub file_name: &'a str,
    /// "Group by" fields (the sort fields, outermost first): a grouped
    /// field's value is written in the first record of its group only.
    pub group_by: &'a [u32],
}

impl Export<'_> {
    /// Text of every cell: rows × columns × values.
    pub fn cells(&self) -> Vec<Vec<Vec<String>>> {
        let mut rows: Vec<Vec<Vec<String>>> = self
            .found
            .iter()
            .map(|&r| self.columns.iter().map(|c| self.texts(r, c)).collect())
            .collect();
        let table = self.db.table_of(self.to).map_or(0, |t| t.id);
        for k in 0..self.group_by.len() {
            // A level-k field shows where a group of levels 0..=k starts.
            let starts: Vec<usize> =
                crate::groups(self.db, table, self.found, &self.group_by[..=k])
                    .into_iter()
                    .map(|g| g.start)
                    .collect();
            for (ci, c) in self.columns.iter().enumerate() {
                if c.to != self.to || c.field != self.group_by[k] {
                    continue;
                }
                for (i, row) in rows.iter_mut().enumerate() {
                    if !starts.contains(&i) {
                        row[ci] = vec![String::new()];
                    }
                }
            }
        }
        rows
    }

    fn texts(&self, record: usize, c: &Column) -> Vec<String> {
        let raw = !self.formatted && c.to == self.to;
        let stored = self.db.table(c.table).filter(|_| raw).and_then(|t| {
            let f = t.field(c.field)?;
            (f.kind == FieldKind::Normal)
                .then(|| t.records.get(record)?.values.get(&c.field).cloned())?
        });
        if let Some(s) = stored {
            return vec![s];
        }
        values_through(self.db, self.to, record, c.to, c.field, self.found)
            .iter()
            .map(|v| stored_text(v, &self.db.locale))
            .collect()
    }

    pub fn write(&self, format: Format) -> Vec<u8> {
        match format {
            Format::Csv => self.delimited(false).into_bytes(),
            Format::Merge => self.delimited(true).into_bytes(),
            Format::Tab => self.tab().into_bytes(),
            Format::Xml => self.xml().into_bytes(),
            Format::Json => self.json().into_bytes(),
            Format::Xlsx => self.xlsx(),
            Format::Html => self.html().into_bytes(),
        }
    }

    /// HTML table: one `TH` per field, one `TR` per record; values are
    /// escaped and returns become `<BR>`.
    /// 🔍 page layout after FileMaker's documentation, not compared byte
    /// for byte with a FileMaker export.
    fn html(&self) -> String {
        let cell = |s: &str| xml_escape(s).replace("&#13;", "<BR>").replace('\n', "<BR>");
        let mut out = String::from(
            "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.01 Transitional//EN\">\n<HTML>\n<HEAD>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;charset=utf-8\">\n",
        );
        out.push_str(&format!(
            "<TITLE>\n{}\n</TITLE>\n</HEAD>\n<BODY>\n<TABLE BORDER=1>\n<TR>\n",
            cell(self.file_name)
        ));
        for c in self.columns {
            out.push_str(&format!("<TH>\n{}\n</TH>\n", cell(&c.name)));
        }
        out.push_str("</TR>\n");
        for row in self.cells() {
            out.push_str("<TR>\n");
            for v in row {
                let v = cell(&v.join("\r"));
                let v = if v.is_empty() { "<BR>".to_string() } else { v };
                out.push_str(&format!("<TD>\n{v}\n</TD>\n"));
            }
            out.push_str("</TR>\n");
        }
        out.push_str("</TABLE>\n</BODY>\n</HTML>\n");
        out
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
        for (&r, row) in self.found.iter().zip(self.cells()) {
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
        for (&r, texts) in self.found.iter().zip(self.cells()) {
            let row = self
                .columns
                .iter()
                .zip(texts)
                .map(|(c, texts)| {
                    let vals = values_through(self.db, self.to, r, c.to, c.field, self.found);
                    match vals.as_slice() {
                        [v] if !v.is_empty() && !self.formatted => Cell::of(v),
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
mod tests {
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
        };
        String::from_utf8(e.write(format)).unwrap()
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
        let html = run(Format::Html, &["Name", "Note"], false);
        assert!(html.contains("<TITLE>\nPeople.fmp12\n</TITLE>"));
        assert!(html.contains("<TR>\n<TH>\nName\n</TH>\n<TH>\nNote\n</TH>\n</TR>"));
        assert!(html.contains("<TD>\nline 1<BR>line 2\n</TD>"));
        assert!(html.contains("<TD>\nBob &quot;B&quot;\n</TD>\n<TD>\n<BR>\n</TD>"));
        // Grouped by City (sorted): the city shows once per group.
        let db = sample();
        let cols = columns(&db, T, &["City".into(), "Name".into()]).unwrap();
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
        };
        let csv = String::from_utf8(e.write(Format::Csv)).unwrap();
        assert_eq!(
            csv,
            "\"Oslo\",\"Di\"\r\n\"Paris\",\"Bob \"\"B\"\"\"\r\n\"Rome\",\"Ada\"\r\n\"\",\"Cy\"\r\n\"\",\"Ed\"\r\n"
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
        };
        assert_eq!(e.write(Format::Tab), b"010.50\t12/10/1815\r\n");
        e.formatted = true;
        assert_eq!(e.write(Format::Tab), b"10.5\t12/10/1815\r\n");
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
