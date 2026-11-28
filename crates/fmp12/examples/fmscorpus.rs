//! Writes edited copies of a file, for checking the writer with FileMaker
//! Server's consistency check (`tools/oracle/fms_batch.sh`).
//! `cargo run -p fmp12 --example fmscorpus -- <in.fmp12> <out dir> [prefix]`
//! Writes `<prefix>-<edit>.fmp12` for each edit: `rt` (unchanged rewrite),
//! `tadd`/`tren`/`tdel` (tables), `fadd`/`fren`/`fdel` (fields of the first
//! table), `ladd`/`ldup`/`lren`/`ldel` (layouts), `sadd`/`sren`/`sdel`
//! (scripts). Prints `ERR <edit>: <error>` for an edit that does not apply.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::schema::DataType;
use fmp12::schema_edit::{Editor, FieldSpec};
use fmp12::scripts::{Script, Step, id, scripts};
use fmp12::scripts_save::save_scripts;

type R = Result<Vec<u8>, String>;

fn edit(f: &fmp12::File, op: impl FnOnce(&mut Editor) -> Result<(), String>) -> R {
    let mut ed = Editor::open(f).map_err(|e| e.to_string())?;
    op(&mut ed)?;
    Ok(ed.finish())
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f = fmp12::File::open(&a[0]).unwrap();
    let prefix = a.get(2).map_or("out", |s| s.as_str());
    let s = |e: fmp12::schema_edit::EditError| e.to_string();
    let tables = fmp12::schema::tables(&f).unwrap_or_default();
    let t0 = tables.first().map(|t| t.id);
    let tlast = tables.last().map(|t| t.id);
    let fields: Vec<u32> = tables
        .first()
        .map(|t| t.fields.keys().copied().collect())
        .unwrap_or_default();
    let lays: Vec<_> = fmp12::layouts::layouts(&f)
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.table_occurrence != 0)
        .collect();
    let to0 = lays.first().map(|l| (l.table_occurrence & 0xFFF) as u32);
    let l0 = lays.first().map(|l| l.id);
    let llast = lays.last().map(|l| l.id);
    let list = scripts(&f).unwrap_or_default();
    let save = |l: Vec<Script>| save_scripts(&f, &l).map_err(|e| e.to_string());
    let none = || "nothing to edit".to_string();
    let ops: Vec<(&str, R)> = vec![
        ("rt", fmp12::write::rewrite(&f).map_err(|e| e.to_string())),
        (
            "tadd",
            edit(&f, |e| {
                let (t, _, _) = e.add_table("BC Table").map_err(s)?;
                e.add_field(t, &FieldSpec::new("BC Text", DataType::Text))
                    .map_err(s)?;
                e.add_field(t, &FieldSpec::new("BC Num", DataType::Number))
                    .map_err(s)?;
                Ok(())
            }),
        ),
        (
            "tren",
            edit(&f, |e| {
                e.rename_table(t0.ok_or_else(none)?, "BC Renamed")
                    .map_err(s)
            }),
        ),
        (
            "tdel",
            edit(&f, |e| e.delete_table(tlast.ok_or_else(none)?).map_err(s)),
        ),
        (
            "fadd",
            edit(&f, |e| {
                let t = t0.ok_or_else(none)?;
                e.add_field(t, &FieldSpec::new("BC Text", DataType::Text))
                    .map_err(s)?;
                e.add_field(t, &FieldSpec::new("BC Date", DataType::Date))
                    .map_err(s)?;
                let c =
                    FieldSpec::calculated("BC Calc", DataType::Text, "\"x\" & Get ( RecordID )");
                e.add_field(t, &c).map_err(s)?;
                Ok(())
            }),
        ),
        (
            "fren",
            edit(&f, |e| {
                let fid = *fields.first().ok_or_else(none)?;
                e.rename_field(t0.ok_or_else(none)?, fid, "BC Renamed")
                    .map_err(s)
            }),
        ),
        (
            "fdel",
            edit(&f, |e| {
                let fid = *fields.last().ok_or_else(none)?;
                e.delete_field(t0.ok_or_else(none)?, fid).map_err(s)
            }),
        ),
        (
            "ladd",
            edit(&f, |e| {
                e.new_layout(to0.ok_or_else(none)?, "BC Layout")
                    .map(|_| ())
                    .map_err(s)
            }),
        ),
        (
            "ldup",
            edit(&f, |e| {
                e.duplicate_layout(l0.ok_or_else(none)?, "BC Copy")
                    .map(|_| ())
                    .map_err(s)
            }),
        ),
        (
            "lren",
            edit(&f, |e| {
                e.rename_layout(l0.ok_or_else(none)?, "BC Renamed")
                    .map_err(s)
            }),
        ),
        (
            "ldel",
            edit(&f, |e| e.delete_layout(llast.ok_or_else(none)?).map_err(s)),
        ),
        ("sadd", {
            let mut l = list.clone();
            l.push(Script {
                id: l.iter().map(|s| s.id).max().unwrap_or(0) + 1,
                name: "BC Script".into(),
                options: 8,
                steps: vec![Step::new(id::BEEP)],
            });
            save(l)
        }),
        ("sren", {
            let mut l = list.clone();
            match l.iter_mut().find(|s| !s.is_folder() && !s.is_separator()) {
                Some(x) => {
                    x.name = "BC Renamed".into();
                    save(l)
                }
                None => Err(none()),
            }
        }),
        ("sdel", {
            let mut l = list.clone();
            match l.iter().rposition(|s| !s.is_folder() && !s.is_separator()) {
                Some(i) => {
                    l.remove(i);
                    save(l)
                }
                None => Err(none()),
            }
        }),
    ];
    for (name, r) in ops {
        match r {
            Ok(b) => std::fs::write(format!("{}/{prefix}-{name}.fmp12", a[1]), b).unwrap(),
            Err(e) => println!("ERR {prefix}-{name}: {e}"),
        }
    }
}
