//! Export Records, Import Records and print to PDF (CLI `export`, `import`,
//! `print-pdf`; MCP `export_records`, `import_records`, `print_pdf`).

use engine::export::{Export, Format, columns};
use engine::import::{Mapping, Mode, Options, import, read_rows};
use serde_json::{Value as Json, json};

use crate::{Doc, Opts, Res, found_set, sort_keys};

/// Occurrence a table name refers to: the occurrence of that name, else
/// the first occurrence of the table.
fn occurrence(d: &Doc, name: &str, table: u32) -> Res<u32> {
    d.db.occurrences
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(name) && o.base_table == table)
        .or_else(|| d.db.occurrences.iter().find(|o| o.base_table == table))
        .map(|o| o.id)
        .ok_or_else(|| format!("table {name:?} has no table occurrence"))
}

fn format_of(explicit: Option<&str>, path: Option<&str>) -> Res<Format> {
    match (explicit, path) {
        (Some(f), _) => Format::from_name(f).ok_or_else(|| format!("unknown format {f:?}")),
        (None, Some(p)) => Format::from_name(p).ok_or_else(|| {
            format!("cannot tell the format of {p:?}: add --format csv|tab|mer|xml|xlsx|json")
        }),
        (None, None) => Ok(Format::Csv),
    }
}

pub fn export(d: &Doc, table: &str, o: &Opts) -> Res<Json> {
    let t = d.table(table)?;
    let to = occurrence(d, table, t.id)?;
    let mut found = found_set(d, t, &o.query)?;
    found.truncate(o.query.limit.unwrap_or(usize::MAX));
    let format = format_of(o.format.as_deref(), o.out.as_deref())?;
    let cols = columns(&d.db, to, &o.fields)?;
    let file_name = std::path::Path::new(&d.path)
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let bytes = Export {
        db: &d.db,
        to,
        found: &found,
        columns: &cols,
        formatted: o.formatted,
        file_name: &file_name,
        group_by: &[],
    }
    .write(format);
    let fields: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    let summary =
        json!({ "exported": found.len(), "format": format!("{format:?}"), "fields": fields });
    match &o.out {
        Some(path) => {
            std::fs::write(path, &bytes).map_err(|e| format!("{path}: {e}"))?;
            Ok(json!({ "path": path, "bytes": bytes.len(), "export": summary }))
        }
        None if format == Format::Xlsx => Err("xlsx export needs -o <file.xlsx>".into()),
        None => Ok(json!({ "export": summary, "content": String::from_utf8_lossy(&bytes) })),
    }
}

pub fn import_file(d: &mut Doc, table: &str, source: &str, o: &Opts) -> Res<Json> {
    let tid = d.table(table)?.id;
    let format = format_of(o.format.as_deref(), Some(source))?;
    let bytes = std::fs::read(source).map_err(|e| format!("{source}: {e}"))?;
    let rows = read_rows(format, &bytes)?;
    let mapping = if o.by_name || (o.names && o.map.is_empty()) {
        Mapping::ByName
    } else {
        Mapping::ByOrder(o.map.clone())
    };
    let mode = match o.mode.as_deref().unwrap_or("add") {
        "add" => Mode::Add,
        "update" => Mode::Update {
            match_fields: o.matches.clone(),
            add_remaining: o.add_remaining,
        },
        "replace" => Mode::Replace {
            add_remaining: o.add_remaining,
        },
        m => return Err(format!("unknown import mode {m:?} (add, update, replace)")),
    };
    let found = found_set(d, d.table(table)?, &o.query)?;
    let opts = Options {
        format,
        first_row_names: o.names,
        mapping,
        mode,
        auto_enter: !o.no_auto_enter,
    };
    let r = import(&mut d.db, tid, &found, rows, &opts)?;
    let t = d.db.table(tid).ok_or("no table")?;
    Ok(json!({
        "table": t.name,
        "added": r.added.len(),
        "updated": r.updated.len(),
        "skipped": r.skipped,
        "added_ids": r.added,
        "updated_ids": r.updated,
        "validation_errors": r.errors.iter().map(|e| json!({
            "row": e.row,
            "field": t.field(e.field).map(|f| f.name.clone()),
            "rule": format!("{:?}", e.rule),
            "record_skipped": e.skipped,
        })).collect::<Vec<_>>(),
    }))
}

pub fn print_pdf(d: &Doc, layout: &str, o: &Opts) -> Res<Json> {
    let l =
        d.db.layouts
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(layout) && l.table_occurrence != 0)
            .ok_or_else(|| format!("no layout named {layout:?}"))?;
    let t =
        d.db.table_of((l.table_occurrence & 0xFFF) as u32)
            .ok_or("layout has no table")?;
    let mut found = found_set(d, t, &o.query)?;
    found.truncate(o.query.limit.unwrap_or(usize::MAX));
    let sort = sort_keys(d, t, &o.query)?;
    let mut paper = match o.paper.as_deref().unwrap_or("letter") {
        "letter" => print::Paper::letter(),
        "a4" => print::Paper::a4(),
        p => return Err(format!("unknown paper {p:?} (letter, a4)")),
    };
    if o.landscape {
        paper = paper.landscape();
    }
    if let Some(m) = o.margin {
        paper = paper.with_margins(m);
    }
    let out = o.out.as_deref().ok_or("print-pdf needs -o <file.pdf>")?;
    let charts = fmp12::charts::charts(&d.file).map_err(|e| e.to_string())?;
    let (bytes, pages) = print::pdf_with_charts(&d.db, &charts, l.id, &found, &sort, &paper)?;
    std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
    Ok(json!({ "path": out, "pages": pages, "records": found.len(), "layout": l.name }))
}

#[cfg(test)]
mod tests {
    use crate::{Doc, opts, run};

    /// Import, export and print through the command dispatcher (skipped
    /// without the FileMaker templates).
    #[test]
    fn import_export_print_on_template() {
        let path = r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates\Contacts\en.fmp12";
        if !std::path::Path::new(path).exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("basecraft-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("in.csv");
        std::fs::write(
            &csv,
            "First Name,Last Name\r\nAda,Lovelace\r\nAlan,Turing\r\n",
        )
        .unwrap();
        let mut d = Doc::open(path).unwrap();
        let args = |v: &[&str]| opts(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let r = run(
            &mut d,
            "import",
            &args(&["Contacts", csv.to_str().unwrap(), "--names"]),
        )
        .unwrap();
        assert_eq!(r["added"], 2);
        let r = run(
            &mut d,
            "export",
            &args(&[
                "Contacts",
                "--field",
                "Last Name",
                "--sort",
                "Last Name:desc",
                "--format",
                "tab",
            ]),
        )
        .unwrap();
        assert_eq!(r["content"], "Turing\r\nLovelace\r\n");
        let pdf = dir.join("out.pdf");
        let r = run(
            &mut d,
            "print-pdf",
            &args(&["Contact List", "-o", pdf.to_str().unwrap(), "--paper", "a4"]),
        )
        .unwrap();
        assert_eq!(r["pages"], 1);
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
