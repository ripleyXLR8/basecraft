//! `schema …` and `new-file`: Manage Database from the command line.
//!
//! ```text
//! schema add-table   <file> <Name>
//! schema add-field   <file> <Table> <Name> [--type T] [field options]
//! schema set-options <file> <Table> <Field> [field options] [--name New]
//! schema rename      <file> table <Old> <New>  |  field <Table> <Old> <New>
//! schema delete      <file> table <Name>       |  field <Table> <Field>
//! new-file <out.fmp12> [--table Name]
//! ```
//!
//! Field options: `--type text|number|date|time|timestamp|container`,
//! `--comment C`, `--calc F` (`--unstored`), `--summary op:Field`
//! (total, average, count, minimum, maximum, stdev, fraction, list),
//! auto-enter `--serial NEXT`, `--stamp creation-date|…|modification-account`,
//! `--auto-calc F` (`--replace`, `--always-evaluate`), `--constant V`,
//! `--prohibit-modification`; validation `--not-empty`, `--unique`,
//! `--existing`, `--range LOW:HIGH`, `--validate F`, `--always-validate`,
//! `--strict` (no override); storage `--global`, `--repetitions N`,
//! `--index none|minimal|all`, `--no-auto-index`. `--no-<option>` turns an
//! option off (set-options).

use fmp12::schema::{AutoStamp, DataType, Indexing, SummaryOp};
use fmp12::schema_edit::{FieldSpec, FieldType};
use serde_json::{Value as Json, json};

use crate::{Doc, Res};

fn data_type(s: &str) -> Res<DataType> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "text" => DataType::Text,
        "number" => DataType::Number,
        "date" => DataType::Date,
        "time" => DataType::Time,
        "timestamp" => DataType::Timestamp,
        "container" => DataType::Container,
        _ => return Err(format!("unknown type {s:?}")),
    })
}

fn stamp(s: &str) -> Res<AutoStamp> {
    use AutoStamp::*;
    Ok(match s.to_ascii_lowercase().as_str() {
        "creation-date" => CreationDate,
        "creation-time" => CreationTime,
        "creation-timestamp" => CreationTimestamp,
        "creation-name" => CreationName,
        "creation-account" => CreationAccountName,
        "modification-date" => ModificationDate,
        "modification-time" => ModificationTime,
        "modification-timestamp" => ModificationTimestamp,
        "modification-name" => ModificationName,
        "modification-account" => ModificationAccountName,
        _ => return Err(format!("unknown stamp {s:?}")),
    })
}

fn summary_op(s: &str) -> Res<SummaryOp> {
    use SummaryOp::*;
    Ok(match s.to_ascii_lowercase().as_str() {
        "total" => Total,
        "average" => Average,
        "count" => Count,
        "minimum" | "min" => Minimum,
        "maximum" | "max" => Maximum,
        "stdev" => StandardDeviation,
        "fraction" => FractionOfTotal,
        "list" => List,
        _ => return Err(format!("unknown summary {s:?}")),
    })
}

/// Applies `--option` arguments to `spec`. `field` resolves a field name of
/// the table (summaries).
pub fn apply_options(
    spec: &mut FieldSpec,
    args: &[String],
    field: &dyn Fn(&str) -> Res<u32>,
) -> Res<()> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        let (au, v) = (&mut spec.auto_enter, &mut spec.validation);
        match a.as_str() {
            "--name" => spec.name = val()?,
            "--type" => spec.data_type = data_type(&val()?)?,
            "--comment" => spec.comment = val()?,
            "--calc" => {
                let stored =
                    !matches!(spec.field_type, FieldType::Calculated { stored: false, .. });
                spec.field_type = FieldType::Calculated {
                    formula: val()?,
                    stored,
                };
            }
            "--unstored" | "--stored" => match &mut spec.field_type {
                FieldType::Calculated { stored, .. } => *stored = a == "--stored",
                _ => return Err(format!("{a} applies to calculated fields")),
            },
            "--summary" => {
                let s = val()?;
                let (op, f) = s.split_once(':').ok_or("--summary takes operation:Field")?;
                spec.field_type = FieldType::Summary {
                    op: summary_op(op)?,
                    field: field(f)?,
                };
                if spec.data_type == DataType::Text {
                    spec.data_type = DataType::Number;
                }
            }
            "--serial" => au.serial = Some(val()?),
            "--stamp" => au.stamp = Some(stamp(&val()?)?),
            "--auto-calc" => au.calculation = Some(val()?),
            "--replace" => au.replace_existing = true,
            "--always-evaluate" => au.always_evaluate = true,
            "--constant" => au.constant = Some(val()?),
            "--prohibit-modification" => au.prohibit_modification = true,
            "--not-empty" => v.not_empty = true,
            "--unique" => v.unique = true,
            "--existing" => v.existing = true,
            "--range" => {
                let r = val()?;
                let (lo, hi) = r.split_once(':').ok_or("--range takes LOW:HIGH")?;
                v.range = Some((lo.to_string(), hi.to_string()));
            }
            "--validate" => v.calculation = Some(val()?),
            "--always-validate" => v.always = true,
            "--strict" => v.allow_override = false,
            "--global" => spec.storage.global = true,
            "--repetitions" => {
                spec.storage.repetitions = val()?.parse().map_err(|_| "bad --repetitions")?
            }
            "--index" => {
                spec.storage.indexing = match val()?.to_ascii_lowercase().as_str() {
                    "none" => Indexing::None,
                    "minimal" => Indexing::Minimal,
                    "all" => Indexing::All,
                    x => return Err(format!("unknown index {x:?}")),
                }
            }
            "--no-auto-index" => spec.storage.auto_index = false,
            "--auto-index" => spec.storage.auto_index = true,
            "--no-serial" => au.serial = None,
            "--no-stamp" => au.stamp = None,
            "--no-auto-calc" => au.calculation = None,
            "--no-replace" => au.replace_existing = false,
            "--no-constant" => au.constant = None,
            "--no-prohibit-modification" => au.prohibit_modification = false,
            "--no-not-empty" => v.not_empty = false,
            "--no-unique" => v.unique = false,
            "--no-existing" => v.existing = false,
            "--no-range" => v.range = None,
            "--no-validate" => v.calculation = None,
            "--no-always-validate" => v.always = false,
            "--no-strict" => v.allow_override = true,
            "--no-global" => spec.storage.global = false,
            _ => return Err(format!("unknown field option {a:?}")),
        }
    }
    Ok(())
}

/// Base table id of a table or occurrence name.
fn table_id(d: &Doc, name: &str) -> Res<u32> {
    Ok(d.table(name)?.id)
}

fn field_of(d: &Doc, table: u32, name: &str) -> Res<u32> {
    let t = d.db.table(table).ok_or("no table")?;
    d.field_id(t, name)
}

fn field_json(d: &Doc, table: u32, id: u32) -> Json {
    let t = d.db.table(table);
    let f = t.and_then(|t| t.field(id));
    json!({
        "table": t.map(|t| t.name.clone()),
        "field": f.map(|f| f.name.clone()),
        "id": id,
        "type": f.map(|f| format!("{:?}", f.data_type)),
        "kind": f.map(|f| format!("{:?}", f.kind)),
        "calculation": f.and_then(|f| f.calc.clone().or_else(|| f.auto_calc.clone())),
    })
}

/// `schema <sub> …` on an open document (`args[0]` = sub-command).
pub fn run(d: &mut Doc, args: &[String]) -> Res<Json> {
    let arg = |i: usize, what: &str| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("schema: missing {what}"))
    };
    let e = |e: fmp12::schema_edit::EditError| e.to_string();
    match arg(0, "sub-command")?.as_str() {
        "add-table" => {
            let name = arg(1, "table name")?;
            let t = d.db.add_table(&mut d.file, &name).map_err(e)?;
            Ok(json!({ "added_table": name, "id": t }))
        }
        "add-field" => {
            let t = table_id(d, &arg(1, "table")?)?;
            let mut spec = FieldSpec::new(&arg(2, "field name")?, DataType::Text);
            let opts = args.get(3..).unwrap_or_default();
            apply_options(&mut spec, opts, &|n| field_of(d, t, n))?;
            let f = d.db.add_field(&mut d.file, t, &spec).map_err(e)?;
            Ok(json!({ "added_field": field_json(d, t, f) }))
        }
        "set-options" => {
            let t = table_id(d, &arg(1, "table")?)?;
            let f = field_of(d, t, &arg(2, "field")?)?;
            let mut spec = fmp12::schema_edit::Editor::open(&d.file)
                .map_err(|x| x.to_string())?
                .field_spec(t, f)
                .map_err(e)?;
            let opts = args.get(3..).unwrap_or_default();
            apply_options(&mut spec, opts, &|n| field_of(d, t, n))?;
            d.db.set_field(&mut d.file, t, f, &spec).map_err(e)?;
            Ok(json!({ "changed_field": field_json(d, t, f) }))
        }
        "rename" => match arg(1, "table or field")?.as_str() {
            "table" => {
                let t = table_id(d, &arg(2, "table")?)?;
                let new = arg(3, "new name")?;
                d.db.rename_table(&mut d.file, t, &new).map_err(e)?;
                Ok(json!({ "renamed_table": new, "id": t }))
            }
            "field" => {
                let t = table_id(d, &arg(2, "table")?)?;
                let f = field_of(d, t, &arg(3, "field")?)?;
                let new = arg(4, "new name")?;
                d.db.rename_field(&mut d.file, t, f, &new).map_err(e)?;
                Ok(json!({ "renamed_field": field_json(d, t, f) }))
            }
            x => Err(format!("schema rename: expected table or field, got {x:?}")),
        },
        "delete" => match arg(1, "table or field")?.as_str() {
            "table" => {
                let name = arg(2, "table")?;
                let t = table_id(d, &name)?;
                d.db.delete_table(&mut d.file, t).map_err(e)?;
                Ok(json!({ "deleted_table": name }))
            }
            "field" => {
                let t = table_id(d, &arg(2, "table")?)?;
                let name = arg(3, "field")?;
                let f = field_of(d, t, &name)?;
                d.db.delete_field(&mut d.file, t, f).map_err(e)?;
                Ok(json!({ "deleted_field": name }))
            }
            x => Err(format!("schema delete: expected table or field, got {x:?}")),
        },
        x => Err(format!("unknown schema command {x:?}")),
    }
}

/// `new-file <out> [--table Name]`: a new, empty database.
pub fn new_file(out: &str, table: &str) -> Res<Json> {
    let bytes = fmp12::schema_edit::new_file(table).map_err(|e| e.to_string())?;
    std::fs::write(out, bytes).map_err(|e| format!("{out}: {e}"))?;
    Ok(json!({ "created": out, "table": table }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_field_options() {
        let mut spec = FieldSpec::new("X", DataType::Text);
        let a = s(&[
            "--type",
            "number",
            "--serial",
            "10",
            "--unique",
            "--range",
            "1:9",
            "--index",
            "all",
            "--global",
            "--repetitions",
            "2",
        ]);
        apply_options(&mut spec, &a, &|_| Ok(1)).unwrap();
        assert_eq!(spec.data_type, DataType::Number);
        assert_eq!(spec.auto_enter.serial.as_deref(), Some("10"));
        assert!(spec.validation.unique && spec.storage.global);
        assert_eq!(spec.validation.range, Some(("1".into(), "9".into())));
        assert_eq!(spec.storage.repetitions, 2);
        apply_options(&mut spec, &s(&["--summary", "count:Y"]), &|_| Ok(3)).unwrap();
        assert_eq!(
            spec.field_type,
            FieldType::Summary {
                op: SummaryOp::Count,
                field: 3
            }
        );
        assert!(apply_options(&mut spec, &s(&["--bogus"]), &|_| Ok(1)).is_err());
        assert!(apply_options(&mut spec, &s(&["--unstored"]), &|_| Ok(1)).is_err());
    }
}
