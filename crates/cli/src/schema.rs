//! `schema …` and `new-file`: Manage Database from the command line.
//!
//! ```text
//! schema add-table        <file> <Name>
//! schema add-field        <file> <Table> <Name> [--type T] [field options]
//! schema set-options      <file> <Table> <Field> [field options] [--name New]
//! schema add-occurrence   <file> <Table> <Name>
//! schema add-relationship <file> <LeftTO> <RightTO> <Field op Field>… [relationship options]
//! schema set-relationship <file> <LeftTO> <RightTO> [<Field op Field>…] [relationship options]
//! schema add-valuelist    <file> <Name> [Value…] | --from-field TO::Field [value list options]
//! schema set-valuelist    <file> <Name> [Value…] [value list options] [--name New]
//! schema rename           <file> table|occurrence|valuelist <Old> <New>  |  field <Table> <Old> <New>
//! schema delete           <file> table|occurrence|valuelist <Name>  |  field <Table> <Field>
//!                                |  relationship <LeftTO> <RightTO>
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
//! `--strict` (no override), `--in-list VALUELIST`, `--max-chars N`,
//! `--message TEXT` (or `--message-calc F`); lookups `--lookup TO::Field`
//! (`--lookup-from TO`, `--if-no-match none|lower|higher|value:TEXT`,
//! `--copy-empty`); serial `--increment N`; summaries `--running`,
//! `--restart-by F`, `--weighted-by F`, `--subtotal-by F`; storage `--global`,
//! `--repetitions N`, `--index none|minimal|all`, `--no-auto-index`.
//! `--no-<option>` turns an option off (set-options).
//!
//! Relationship options: operators `=`, `≠` (`!=`), `<`, `≤` (`<=`), `>`,
//! `≥` (`>=`), `×` (`x`); `--create-left`, `--create-right`, `--delete-left`,
//! `--delete-right`, `--sort-left Field[:desc]`, `--sort-right Field[:desc]`
//! (repeated), and `--no-…` of each (set-relationship).
//!
//! Value list options: `--from-field TO::Field`, `--second TO::Field`,
//! `--second-only`, `--sort-second`, `--related-from TO`.

use fmp12::graph::{JoinOp, Predicate};
use fmp12::schema::{AutoStamp, DataType, Indexing, Lookup, NoMatch, SummaryOp};
use fmp12::schema_edit::{FieldSpec, FieldType, RelSide, RelationshipSpec};
use fmp12::valuelists::Source;
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

/// The names field options refer to, resolved for one table.
pub trait Names {
    /// A field of the table.
    fn field(&self, name: &str) -> Res<u32>;
    fn occurrence(&self, name: &str) -> Res<u32>;
    /// `Occurrence::Field`.
    fn to_field(&self, name: &str) -> Res<(u32, u32)>;
    fn value_list(&self, name: &str) -> Res<u32>;
    /// The table's first occurrence (where its lookups start).
    fn context(&self) -> Res<u32>;
}

/// A FileMaker text literal: quotes around, `"` and backslash escaped.
fn literal(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Sets the running flag and second field of a summary.
fn summary_by(spec: &mut FieldSpec, a: &str, by: Option<u32>, running: Option<bool>) -> Res<()> {
    match &mut spec.field_type {
        FieldType::Summary {
            by: b, running: r, ..
        } => {
            if let Some(x) = running {
                *r = x;
            }
            if by.is_some() || running == Some(false) {
                *b = by;
            }
            Ok(())
        }
        _ => Err(format!("{a} applies to summary fields")),
    }
}

/// Applies the lookup and field-reference options; false when `a` is
/// not one of them.
fn apply_reference(
    spec: &mut FieldSpec,
    a: &str,
    val: &mut dyn FnMut() -> Res<String>,
    names: &dyn Names,
) -> Res<bool> {
    let need = |spec: &mut FieldSpec| -> Res<()> {
        spec.auto_enter
            .lookup
            .is_some()
            .then_some(())
            .ok_or_else(|| format!("{a} needs --lookup"))
    };
    match a {
        "--increment" => {
            spec.auto_enter.serial_increment = val()?.parse().map_err(|_| "bad --increment")?
        }
        "--lookup" => {
            let (from, field) = names.to_field(&val()?)?;
            let old = spec.auto_enter.lookup.take();
            let context = match &old {
                Some(l) => l.context,
                None => names.context()?,
            };
            spec.auto_enter.lookup = Some(Lookup {
                context,
                from,
                field,
                no_match: old
                    .as_ref()
                    .map_or(NoMatch::DontCopy, |l| l.no_match.clone()),
                copy_empty: old.is_some_and(|l| l.copy_empty),
            });
        }
        "--lookup-from" | "--if-no-match" | "--copy-empty" | "--no-copy-empty" => {
            need(spec)?;
            let v = if a == "--lookup-from" || a == "--if-no-match" {
                val()?
            } else {
                String::new()
            };
            let to = if a == "--lookup-from" {
                Some(names.occurrence(&v)?)
            } else {
                None
            };
            if let Some(l) = spec.auto_enter.lookup.as_mut() {
                match a {
                    "--lookup-from" => l.context = to.unwrap_or(l.context),
                    "--if-no-match" => {
                        l.no_match = match v.as_str() {
                            "none" => NoMatch::DontCopy,
                            "lower" => NoMatch::Lower,
                            "higher" => NoMatch::Higher,
                            _ => match v.strip_prefix("value:") {
                                Some(t) => NoMatch::Constant(t.to_string()),
                                None => {
                                    return Err(format!(
                                        "--if-no-match takes none, lower, higher or value:TEXT, not {v:?}"
                                    ));
                                }
                            },
                        }
                    }
                    "--copy-empty" => l.copy_empty = true,
                    _ => l.copy_empty = false,
                }
            }
        }
        "--no-lookup" => spec.auto_enter.lookup = None,
        "--in-list" => spec.validation.value_list = Some(names.value_list(&val()?)?),
        "--no-in-list" => spec.validation.value_list = None,
        "--max-chars" => {
            spec.validation.max_chars = Some(val()?.parse().map_err(|_| "bad --max-chars")?)
        }
        "--no-max-chars" => spec.validation.max_chars = None,
        "--message" => spec.validation.message = Some(literal(&val()?)),
        "--message-calc" => spec.validation.message = Some(val()?),
        "--no-message" => spec.validation.message = None,
        "--running" => summary_by(spec, a, None, Some(true))?,
        "--no-running" => summary_by(spec, a, None, Some(false))?,
        "--restart-by" | "--weighted-by" | "--subtotal-by" => {
            let f = names.field(&val()?)?;
            let running = (a == "--restart-by").then_some(true);
            summary_by(spec, a, Some(f), running)?
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Applies `--option` arguments to `spec`; `names` resolves the fields,
/// occurrences and value lists they name.
pub fn apply_options(spec: &mut FieldSpec, args: &[String], names: &dyn Names) -> Res<()> {
    let field = |n: &str| names.field(n);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        if apply_reference(spec, a, &mut val, names)? {
            continue;
        }
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
                    running: false,
                    by: None,
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

fn occurrence(d: &Doc, name: &str) -> Res<u32> {
    d.db.occurrences
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(name))
        .map(|o| o.id)
        .ok_or_else(|| format!("no table occurrence named {name:?}"))
}

/// Field of an occurrence's table.
fn to_field_of(d: &Doc, to: u32, name: &str) -> Res<u32> {
    let t = d.db.table_of(to).ok_or("occurrence without table")?;
    d.field_id(t, name)
}

/// `Occurrence::Field`.
fn to_field(d: &Doc, name: &str) -> Res<(u32, u32)> {
    let (o, f) = name
        .split_once("::")
        .ok_or_else(|| format!("expected Occurrence::Field, got {name:?}"))?;
    let to = occurrence(d, o)?;
    Ok((to, to_field_of(d, to, f)?))
}

fn value_list(d: &Doc, name: &str) -> Res<u32> {
    d.db.value_lists
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case(name))
        .map(|v| v.id)
        .ok_or_else(|| format!("no value list named {name:?}"))
}

/// The names of a document, for the fields of table `table`.
pub struct DocNames<'a> {
    pub d: &'a Doc,
    pub table: u32,
}

impl Names for DocNames<'_> {
    fn field(&self, name: &str) -> Res<u32> {
        field_of(self.d, self.table, name)
    }
    fn occurrence(&self, name: &str) -> Res<u32> {
        occurrence(self.d, name)
    }
    fn to_field(&self, name: &str) -> Res<(u32, u32)> {
        to_field(self.d, name)
    }
    fn value_list(&self, name: &str) -> Res<u32> {
        value_list(self.d, name)
    }
    fn context(&self) -> Res<u32> {
        self.d
            .db
            .occurrences
            .iter()
            .find(|o| o.base_table == self.table)
            .map(|o| o.id)
            .ok_or_else(|| "the table has no occurrence".into())
    }
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

/// Reads `Field op Field` (operators `=`, `≠` `!=` `<>`, `<`, `≤` `<=`,
/// `>`, `≥` `>=`, `×` or ` x `).
fn predicate(d: &Doc, left: u32, right: u32, s: &str) -> Res<Predicate> {
    let ops = [
        "!=", "<>", "<=", ">=", "==", "≠", "≤", "≥", "×", " x ", " X ", "=", "<", ">",
    ];
    let (i, op) = ops
        .iter()
        .filter_map(|op| s.find(op).map(|i| (i, *op)))
        .min_by_key(|(i, op)| (*i, usize::MAX - op.len()))
        .ok_or_else(|| format!("expected Field op Field, got {s:?}"))?;
    let join = JoinOp::parse(op.trim()).ok_or_else(|| format!("unknown operator {op:?}"))?;
    Ok(Predicate {
        op: join,
        left_field: to_field_of(d, left, s[..i].trim())?,
        right_field: to_field_of(d, right, s[i + op.len()..].trim())?,
    })
}

/// Applies relationship options; `Field op Field` arguments replace the
/// predicates when there are any.
fn relationship_options(d: &Doc, spec: &mut RelationshipSpec, args: &[String]) -> Res<()> {
    let mut preds = Vec::new();
    let mut it = args.iter();
    let sort_key = |to: u32, v: &str| -> Res<(u32, bool)> {
        let (f, desc) = match v.rsplit_once(':') {
            Some((f, "desc")) => (f, true),
            Some((f, "asc")) => (f, false),
            _ => (v, false),
        };
        Ok((to_field_of(d, to, f)?, desc))
    };
    let (l, r) = (spec.left, spec.right);
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        let (ls, rs) = (&mut spec.left_options, &mut spec.right_options);
        match a.as_str() {
            "--create-left" => ls.create = true,
            "--create-right" => rs.create = true,
            "--delete-left" => ls.delete = true,
            "--delete-right" => rs.delete = true,
            "--no-create-left" => ls.create = false,
            "--no-create-right" => rs.create = false,
            "--no-delete-left" => ls.delete = false,
            "--no-delete-right" => rs.delete = false,
            "--sort-left" => ls.sort.push(sort_key(l, &val()?)?),
            "--sort-right" => rs.sort.push(sort_key(r, &val()?)?),
            "--no-sort-left" => ls.sort.clear(),
            "--no-sort-right" => rs.sort.clear(),
            x if x.starts_with("--") => return Err(format!("unknown relationship option {x:?}")),
            x => preds.push(predicate(d, l, r, x)?),
        }
    }
    if !preds.is_empty() {
        spec.predicates = preds;
    }
    Ok(())
}

pub fn relationship_json(db: &engine::Database, r: &fmp12::graph::Relationship) -> Json {
    let to = |id: u32| {
        db.occurrences
            .iter()
            .find(|o| o.id == id)
            .map_or(String::new(), |o| o.name.clone())
    };
    let f = |to: u32, id: u32| {
        db.table_of(to)
            .and_then(|t| t.field(id))
            .map_or(String::new(), |f| f.name.clone())
    };
    let sort = |keys: &[fmp12::graph::SortKey]| -> Vec<String> {
        keys.iter()
            .map(|k| {
                format!(
                    "{}{}",
                    f(k.to, k.field),
                    if k.descending { ":desc" } else { "" }
                )
            })
            .collect()
    };
    json!({
        "id": r.id,
        "left": to(r.left),
        "right": to(r.right),
        "predicates": r.predicates.iter()
            .map(|p| format!("{} {} {}", f(r.left, p.left_field), p.op.symbol(), f(r.right, p.right_field)))
            .collect::<Vec<_>>(),
        "left_options": { "create": r.left_cascade_create(), "delete": r.left_cascade_delete(), "sort": sort(&r.left_sort) },
        "right_options": { "create": r.right_cascade_create(), "delete": r.right_cascade_delete(), "sort": sort(&r.right_sort) },
    })
}

pub fn value_list_json(db: &engine::Database, v: &fmp12::valuelists::ValueList) -> Json {
    let tf = |to: u32, f: u32| {
        let o = db.occurrences.iter().find(|o| o.id == to);
        let field = db.table_of(to).and_then(|t| t.field(f));
        format!(
            "{}::{}",
            o.map_or("", |o| o.name.as_str()),
            field.map_or("", |f| f.name.as_str())
        )
    };
    let source = match &v.source {
        Source::Custom(_) => json!("custom"),
        Source::Field {
            to,
            field,
            second,
            second_only,
            sort_by_second,
            related_from,
        } => json!({
            "field": tf(*to, *field),
            "second": second.map(|(t, f)| tf(t, f)),
            "second_only": second_only,
            "sort_by_second": sort_by_second,
            "related_from": related_from.and_then(|r| db.occurrences.iter().find(|o| o.id == r)).map(|o| o.name.clone()),
        }),
        Source::Unknown(n) => json!(format!("unknown {n}")),
    };
    json!({ "name": v.name, "source": source, "values": db.value_list_values(v.id) })
}

/// Applies value list arguments: values, or `--from-field` and its options.
fn value_list_source(
    d: &Doc,
    old: Option<&Source>,
    args: &[String],
) -> Res<(Source, Option<String>)> {
    let mut values: Vec<String> = Vec::new();
    let mut src = old.cloned();
    let mut name = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        let field_src = |src: &mut Option<Source>| -> Res<()> {
            match src {
                Some(Source::Field { .. }) => Ok(()),
                _ => Err(format!("{a} needs --from-field")),
            }
        };
        match a.as_str() {
            "--from-field" => {
                let (to, field) = to_field(d, &val()?)?;
                src = Some(match src {
                    Some(Source::Field {
                        second,
                        second_only,
                        sort_by_second,
                        related_from,
                        ..
                    }) => Source::Field {
                        to,
                        field,
                        second,
                        second_only,
                        sort_by_second,
                        related_from,
                    },
                    _ => Source::Field {
                        to,
                        field,
                        second: None,
                        second_only: false,
                        sort_by_second: false,
                        related_from: None,
                    },
                });
            }
            "--second" | "--related-from" => {
                let v = val()?;
                field_src(&mut src)?;
                if let Some(Source::Field {
                    second,
                    related_from,
                    ..
                }) = src.as_mut()
                {
                    if a == "--second" {
                        *second = Some(to_field(d, &v)?);
                    } else {
                        *related_from = Some(occurrence(d, &v)?);
                    }
                }
            }
            "--second-only" | "--sort-second" | "--no-second" | "--no-related-from" => {
                field_src(&mut src)?;
                if let Some(Source::Field {
                    second,
                    second_only,
                    sort_by_second,
                    related_from,
                    ..
                }) = src.as_mut()
                {
                    match a.as_str() {
                        "--second-only" => *second_only = true,
                        "--sort-second" => *sort_by_second = true,
                        "--no-second" => {
                            *second = None;
                            *second_only = false;
                            *sort_by_second = false;
                        }
                        _ => *related_from = None,
                    }
                }
            }
            "--name" => name = Some(val()?),
            x if x.starts_with("--") => return Err(format!("unknown value list option {x:?}")),
            x => values.push(x.to_string()),
        }
    }
    if !values.is_empty() {
        src = Some(Source::Custom(values.join("\r")));
    }
    Ok((
        src.ok_or("a value list needs values or --from-field TO::Field")?,
        name,
    ))
}

/// The relationship between two occurrences, in either order.
fn relationship_between(d: &Doc, a: &str, b: &str) -> Res<u32> {
    let (a, b) = (occurrence(d, a)?, occurrence(d, b)?);
    d.db.relationships
        .iter()
        .find(|r| (r.left, r.right) == (a, b) || (r.left, r.right) == (b, a))
        .map(|r| r.id)
        .ok_or_else(|| "no relationship between these occurrences".into())
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
            apply_options(&mut spec, opts, &DocNames { d, table: t })?;
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
            apply_options(&mut spec, opts, &DocNames { d, table: t })?;
            d.db.set_field(&mut d.file, t, f, &spec).map_err(e)?;
            Ok(json!({ "changed_field": field_json(d, t, f) }))
        }
        "add-occurrence" => {
            let t = table_id(d, &arg(1, "table")?)?;
            let name = arg(2, "occurrence name")?;
            let to = d.db.add_occurrence(&mut d.file, t, &name).map_err(e)?;
            Ok(json!({ "added_occurrence": name, "id": to }))
        }
        "add-relationship" => {
            let (l, r) = (
                occurrence(d, &arg(1, "left occurrence")?)?,
                occurrence(d, &arg(2, "right occurrence")?)?,
            );
            let mut spec = RelationshipSpec {
                left: l,
                right: r,
                predicates: Vec::new(),
                left_options: RelSide::default(),
                right_options: RelSide::default(),
            };
            relationship_options(d, &mut spec, args.get(3..).unwrap_or_default())?;
            let id = d.db.add_relationship(&mut d.file, &spec).map_err(e)?;
            let rel =
                d.db.relationships
                    .iter()
                    .find(|x| x.id == id)
                    .ok_or("relationship lost")?;
            Ok(json!({ "added_relationship": relationship_json(&d.db, rel) }))
        }
        "set-relationship" => {
            let id =
                relationship_between(d, &arg(1, "left occurrence")?, &arg(2, "right occurrence")?)?;
            let mut spec = fmp12::schema_edit::Editor::open(&d.file)
                .map_err(|x| x.to_string())?
                .relationship_spec(id)
                .map_err(e)?;
            relationship_options(d, &mut spec, args.get(3..).unwrap_or_default())?;
            d.db.set_relationship(&mut d.file, id, &spec).map_err(e)?;
            let rel =
                d.db.relationships
                    .iter()
                    .find(|x| x.id == id)
                    .ok_or("relationship lost")?;
            Ok(json!({ "changed_relationship": relationship_json(&d.db, rel) }))
        }
        "add-valuelist" => {
            let name = arg(1, "value list name")?;
            let (src, _) = value_list_source(d, None, args.get(2..).unwrap_or_default())?;
            let id = d.db.add_value_list(&mut d.file, &name, &src).map_err(e)?;
            let v =
                d.db.value_lists
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or("value list lost")?;
            Ok(json!({ "added_valuelist": value_list_json(&d.db, v) }))
        }
        "set-valuelist" => {
            let id = value_list(d, &arg(1, "value list")?)?;
            let old =
                d.db.value_lists
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or("no value list")?
                    .clone();
            let (src, name) =
                value_list_source(d, Some(&old.source), args.get(2..).unwrap_or_default())?;
            let name = name.unwrap_or(old.name);
            d.db.set_value_list(&mut d.file, id, &name, &src)
                .map_err(e)?;
            let v =
                d.db.value_lists
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or("value list lost")?;
            Ok(json!({ "changed_valuelist": value_list_json(&d.db, v) }))
        }
        "rename" => match arg(1, "table, field, occurrence or valuelist")?.as_str() {
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
            "occurrence" => {
                let to = occurrence(d, &arg(2, "occurrence")?)?;
                let new = arg(3, "new name")?;
                d.db.rename_occurrence(&mut d.file, to, &new).map_err(e)?;
                Ok(json!({ "renamed_occurrence": new, "id": to }))
            }
            "valuelist" => {
                let id = value_list(d, &arg(2, "value list")?)?;
                let new = arg(3, "new name")?;
                let src =
                    d.db.value_lists
                        .iter()
                        .find(|v| v.id == id)
                        .ok_or("no value list")?
                        .source
                        .clone();
                d.db.set_value_list(&mut d.file, id, &new, &src)
                    .map_err(e)?;
                Ok(json!({ "renamed_valuelist": new, "id": id }))
            }
            x => Err(format!(
                "schema rename: expected table, field, occurrence or valuelist, got {x:?}"
            )),
        },
        "delete" => match arg(1, "what to delete")?.as_str() {
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
            "occurrence" => {
                let name = arg(2, "occurrence")?;
                let to = occurrence(d, &name)?;
                d.db.delete_occurrence(&mut d.file, to).map_err(e)?;
                Ok(json!({ "deleted_occurrence": name }))
            }
            "relationship" => {
                let id = relationship_between(
                    d,
                    &arg(2, "left occurrence")?,
                    &arg(3, "right occurrence")?,
                )?;
                d.db.delete_relationship(&mut d.file, id).map_err(e)?;
                Ok(json!({ "deleted_relationship": id }))
            }
            "valuelist" => {
                let name = arg(2, "value list")?;
                let id = value_list(d, &name)?;
                d.db.delete_value_list(&mut d.file, id).map_err(e)?;
                Ok(json!({ "deleted_valuelist": name }))
            }
            x => Err(format!(
                "schema delete: expected table, field, occurrence, relationship or valuelist, got {x:?}"
            )),
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

    /// Fields are numbered by length of name, occurrences are 200 + length.
    struct Fake;
    impl Names for Fake {
        fn field(&self, name: &str) -> Res<u32> {
            Ok(name.len() as u32)
        }
        fn occurrence(&self, name: &str) -> Res<u32> {
            Ok(200 + name.len() as u32)
        }
        fn to_field(&self, name: &str) -> Res<(u32, u32)> {
            let (o, f) = name.split_once("::").ok_or("no ::")?;
            Ok((self.occurrence(o)?, self.field(f)?))
        }
        fn value_list(&self, name: &str) -> Res<u32> {
            Ok(name.len() as u32)
        }
        fn context(&self) -> Res<u32> {
            Ok(129)
        }
    }

    #[test]
    fn parses_field_options() {
        let mut spec = FieldSpec::new("X", DataType::Text);
        let a = s(&[
            "--type",
            "number",
            "--serial",
            "10",
            "--increment",
            "5",
            "--unique",
            "--range",
            "1:9",
            "--index",
            "all",
            "--global",
            "--repetitions",
            "2",
            "--in-list",
            "abc",
            "--max-chars",
            "8",
            "--message",
            "Say \"no\"",
        ]);
        apply_options(&mut spec, &a, &Fake).unwrap();
        assert_eq!(spec.data_type, DataType::Number);
        assert_eq!(spec.auto_enter.serial.as_deref(), Some("10"));
        assert_eq!(spec.auto_enter.serial_increment, 5);
        assert!(spec.validation.unique && spec.storage.global);
        assert_eq!(spec.validation.range, Some(("1".into(), "9".into())));
        assert_eq!(spec.storage.repetitions, 2);
        assert_eq!(spec.validation.value_list, Some(3));
        assert_eq!(spec.validation.max_chars, Some(8));
        assert_eq!(
            spec.validation.message.as_deref(),
            Some("\"Say \\\"no\\\"\"")
        );
        apply_options(
            &mut spec,
            &s(&["--summary", "count:Yyy", "--running", "--restart-by", "Zz"]),
            &Fake,
        )
        .unwrap();
        assert_eq!(
            spec.field_type,
            FieldType::Summary {
                op: SummaryOp::Count,
                field: 3,
                running: true,
                by: Some(2),
            }
        );
        assert!(apply_options(&mut spec, &s(&["--bogus"]), &Fake).is_err());
        assert!(apply_options(&mut spec, &s(&["--unstored"]), &Fake).is_err());
        // Lookups.
        let mut spec = FieldSpec::new("L", DataType::Text);
        assert!(apply_options(&mut spec, &s(&["--copy-empty"]), &Fake).is_err());
        let a = s(&[
            "--lookup",
            "Abc::Name",
            "--if-no-match",
            "value:none yet",
            "--copy-empty",
        ]);
        apply_options(&mut spec, &a, &Fake).unwrap();
        assert_eq!(
            spec.auto_enter.lookup,
            Some(Lookup {
                context: 129,
                from: 203,
                field: 4,
                no_match: NoMatch::Constant("none yet".into()),
                copy_empty: true,
            })
        );
        assert!(apply_options(&mut spec, &s(&["--if-no-match", "never"]), &Fake).is_err());
    }
}
