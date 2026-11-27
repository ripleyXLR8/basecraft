//! Design functions (schema, layouts, scripts, value lists) on top of
//! [`Context::schema`](crate::Context::schema), and the record functions
//! that need the host (GetNthRecord, Lookup, GetSummary, Self...).

use super::txt;
use crate::ast::Expr;
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R, to_index};
use crate::schema::{DataType, Field, Layout, LayoutObject, Occurrence, Schema, Storage};
use crate::value::Value;
use serde_json::{Map, Value as J};
use std::sync::Arc;

fn schema(ev: &mut Evaluator, file: &str) -> Result<Arc<Schema>, EvalError> {
    ev.ctx
        .schema(file)
        .ok_or_else(|| EvalError::new(code::FILE_MISSING, format!("file {file} is not open")))
}

fn lines<I: IntoIterator<Item = String>>(items: I) -> R {
    Ok(Value::Text(
        items.into_iter().collect::<Vec<_>>().join("\r"),
    ))
}

fn layout<'a>(s: &'a Schema, name: &str) -> Result<&'a Layout, EvalError> {
    s.layout(name)
        .ok_or_else(|| EvalError::new(code::LAYOUT_MISSING, format!("layout {name} is missing")))
}

fn missing_field(name: &str) -> EvalError {
    EvalError::new(code::FIELD_MISSING, format!("field {name} is missing"))
}

fn field<'a>(
    ev: &mut Evaluator,
    s: &'a Schema,
    name: &str,
) -> Result<(&'a Occurrence, &'a Field), EvalError> {
    let current = ev.ctx.current_occurrence();
    s.field(name, current.as_deref())
        .ok_or_else(|| missing_field(name))
}

// ---- lists of names and IDs --------------------------------------------------

pub fn base_table_names(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .base_tables
            .iter()
            .map(|t| t.name.clone()),
    )
}

pub fn base_table_ids(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .base_tables
            .iter()
            .map(|t| t.id.to_string()),
    )
}

pub fn table_names(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .occurrences
            .iter()
            .map(|t| t.name.clone()),
    )
}

pub fn table_ids(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .occurrences
            .iter()
            .map(|t| t.id.to_string()),
    )
}

pub fn layout_names(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .layouts
            .iter()
            .map(|l| l.name.clone()),
    )
}

pub fn layout_ids(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .layouts
            .iter()
            .map(|l| l.id.to_string()),
    )
}

pub fn script_names(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .scripts
            .iter()
            .map(|s| s.name.clone()),
    )
}

pub fn script_ids(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .scripts
            .iter()
            .map(|s| s.id.to_string()),
    )
}

pub fn value_list_names(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .value_lists
            .iter()
            .map(|v| v.name.clone()),
    )
}

pub fn value_list_ids(ev: &mut Evaluator, a: &[Value]) -> R {
    lines(
        schema(ev, &txt(a, 0))?
            .value_lists
            .iter()
            .map(|v| v.id.to_string()),
    )
}

pub fn value_list_items(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let name = txt(a, 1);
    let vl = s.value_list(&name).ok_or_else(|| {
        EvalError::new(code::FIELD_MISSING, format!("value list {name} is missing"))
    })?;
    lines(vl.items.iter().cloned())
}

pub fn base_table_comment(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let name = txt(a, 1);
    let t = s
        .base_table(&name)
        .ok_or_else(|| EvalError::new(code::TABLE_MISSING, format!("table {name} is missing")))?;
    Ok(Value::text(t.comment.clone()))
}

// ---- fields ------------------------------------------------------------------

pub fn field_type(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let (_, f) = field(ev, &s, &txt(a, 1))?;
    let storage = match f.storage {
        Storage::Standard => "Standard",
        Storage::StoredCalc => "StoredCalc",
        Storage::UnstoredCalc => "UnstoredCalc",
        Storage::Summary => "Summary",
        Storage::Global => "Global",
        Storage::ExternalSecure => "External(Secure)",
        Storage::ExternalOpen => "External(Open)",
    };
    let ty = match f.data_type {
        DataType::Text => "Text",
        DataType::Number => "Number",
        DataType::Date => "Date",
        DataType::Time => "Time",
        DataType::Timestamp => "Timestamp",
        DataType::Container => "Container",
    };
    let indexed = if f.indexed { "Indexed" } else { "Unindexed" };
    Ok(Value::Text(format!(
        "{storage} {ty} {indexed} {}",
        f.repetitions.max(1)
    )))
}

pub fn field_comment(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    Ok(Value::text(field(ev, &s, &txt(a, 1))?.1.comment.clone()))
}

pub fn field_annotation(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    Ok(Value::text(field(ev, &s, &txt(a, 1))?.1.annotation.clone()))
}

pub fn field_display_names(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let (_, f) = field(ev, &s, &txt(a, 1))?;
    let m: Map<String, J> = f
        .display_names
        .iter()
        .map(|(k, v)| (k.clone(), J::String(v.clone())))
        .collect();
    Ok(Value::Text(super::json::compact(&J::Object(m))))
}

pub fn get_next_serial_value(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    Ok(Value::text(
        field(ev, &s, &txt(a, 1))?
            .1
            .next_serial
            .clone()
            .unwrap_or_default(),
    ))
}

/// GetBaseTableName ( field ): the base table behind the field's occurrence.
pub fn get_base_table_name(ev: &mut Evaluator, a: &[Expr]) -> R {
    let Expr::Field { table, name, .. } = &a[0] else {
        return Err(EvalError::new(
            code::NAME_MISSING,
            "GetBaseTableName needs a field",
        ));
    };
    let s = schema(ev, "")?;
    let qualified = match table {
        Some(t) => format!("{t}::{name}"),
        None => name.clone(),
    };
    let (o, _) = field(ev, &s, &qualified)?;
    Ok(Value::text(o.base_table.clone()))
}

// ---- layouts -----------------------------------------------------------------

/// Every object, depth first, in layout order.
fn walk<'a>(objects: &'a [LayoutObject], out: &mut Vec<&'a LayoutObject>) {
    for o in objects {
        out.push(o);
        walk(&o.children, out);
    }
}

fn layout_fields(l: &Layout) -> Vec<&str> {
    let mut all = vec![];
    walk(&l.objects, &mut all);
    let mut out: Vec<&str> = vec![];
    for f in all.iter().filter_map(|o| o.field.as_deref()) {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(f)) {
            out.push(f);
        }
    }
    out
}

/// `TO::Field` written as FileMaker lists it on `l`: bare for the layout's
/// own occurrence.
fn short<'a>(l: &Layout, f: &'a str) -> &'a str {
    match f.split_once("::") {
        Some((t, name)) if t.eq_ignore_ascii_case(&l.occurrence) => name,
        _ => f,
    }
}

/// FieldNames / FieldIDs: fields on a layout, or all fields of a table
/// occurrence when no layout has that name.
fn field_list(ev: &mut Evaluator, a: &[Value], ids: bool) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let mut name = txt(a, 1);
    if name.is_empty() {
        name = ev.ctx.current_occurrence().unwrap_or_default();
    }
    if let Some(l) = s.layout(&name) {
        let mut out = vec![];
        for f in layout_fields(l) {
            if !ids {
                out.push(short(l, f).to_string());
                continue;
            }
            let Some((o, fd)) = s.field(f, Some(&l.occurrence)) else {
                continue;
            };
            out.push(if o.name.eq_ignore_ascii_case(&l.occurrence) {
                fd.id.to_string()
            } else {
                format!("{}::{}", o.id, fd.id)
            });
        }
        return lines(out);
    }
    let t = s
        .occurrence_table(&name)
        .ok_or_else(|| EvalError::new(code::LAYOUT_MISSING, format!("{name} is missing")))?;
    lines(t.fields.iter().map(|f| {
        if ids {
            f.id.to_string()
        } else {
            f.name.clone()
        }
    }))
}

pub fn field_names(ev: &mut Evaluator, a: &[Value]) -> R {
    field_list(ev, a, false)
}

pub fn field_ids(ev: &mut Evaluator, a: &[Value]) -> R {
    field_list(ev, a, true)
}

/// The first object on the layout showing `field` (`Field` or `TO::Field`).
fn field_object(ev: &mut Evaluator, a: &[Value]) -> Result<LayoutObject, EvalError> {
    let s = schema(ev, &txt(a, 0))?;
    let l = layout(&s, &txt(a, 1))?;
    let want = txt(a, 2);
    let mut all = vec![];
    walk(&l.objects, &mut all);
    all.into_iter()
        .find(|o| {
            o.field.as_deref().is_some_and(|f| {
                f.eq_ignore_ascii_case(&want) || short(l, f).eq_ignore_ascii_case(&want)
            })
        })
        .cloned()
        .ok_or_else(|| missing_field(&want))
}

pub fn field_bounds(ev: &mut Evaluator, a: &[Value]) -> R {
    let o = field_object(ev, a)?;
    let [l, t, r, b] = o.bounds;
    Ok(Value::Text(format!("{l} {t} {r} {b} {}", o.rotation)))
}

pub fn field_repetitions(ev: &mut Evaluator, a: &[Value]) -> R {
    let o = field_object(ev, a)?;
    Ok(Value::Text(format!(
        "{} {}",
        o.repetitions_shown.max(1),
        if o.vertical { "vertical" } else { "horizontal" }
    )))
}

pub fn field_style(ev: &mut Evaluator, a: &[Value]) -> R {
    let o = field_object(ev, a)?;
    let style = if o.style.is_empty() {
        "Standard"
    } else {
        &o.style
    };
    Ok(Value::Text(if o.value_list.is_empty() {
        style.to_string()
    } else {
        format!("{style} {}", o.value_list)
    }))
}

/// Named objects; a named group's named children follow it between `<` and
/// `>` lines. Unnamed groups are transparent.
fn object_names(objects: &[LayoutObject], out: &mut Vec<String>) {
    for o in objects {
        if o.name.is_empty() {
            object_names(&o.children, out);
            continue;
        }
        out.push(o.name.clone());
        if o.is_group || !o.children.is_empty() {
            out.push("<".into());
            object_names(&o.children, out);
            out.push(">".into());
        }
    }
}

pub fn layout_object_names(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let name = txt(a, 1);
    if name.is_empty() {
        return Ok(Value::empty());
    }
    let mut out = vec![];
    object_names(&layout(&s, &name)?.objects, &mut out);
    lines(out)
}

/// GetFieldsOnLayout: JSON describing the find-able fields of a layout in
/// the current file.
// ponytail: find-mode exclusions (outside the layout, "allow entry in
// Find mode" off, Quick Find, privileges) need data the schema does not
// carry; only summary, global and container fields are left out.
pub fn get_fields_on_layout(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, "")?;
    let l = layout(&s, &txt(a, 0))?;
    let mut fields = Map::new();
    for f in layout_fields(l) {
        let Some((o, fd)) = s.field(f, Some(&l.occurrence)) else {
            continue;
        };
        if matches!(fd.storage, Storage::Summary | Storage::Global)
            || fd.data_type == DataType::Container
        {
            continue;
        }
        let mut entry = Map::new();
        let note = if fd.annotation.is_empty() {
            &fd.comment
        } else {
            &fd.annotation
        };
        let note = note.strip_prefix("[LLM]").unwrap_or(note).trim_start();
        if !note.is_empty() {
            entry.insert("description".into(), J::String(note.into()));
        }
        let ty = if fd.data_type == DataType::Number {
            "number"
        } else {
            "string"
        };
        entry.insert("type".into(), J::String(ty.into()));
        fields.insert(format!("{}::{}", o.name, fd.name), J::Object(entry));
    }
    let mut root = Map::new();
    root.insert("fields".into(), J::Object(fields));
    root.insert("layout_name".into(), J::String(l.name.clone()));
    Ok(Value::Text(super::json::spaced(&J::Object(root))))
}

// ---- relationships -----------------------------------------------------------

fn flip(op: &str) -> &str {
    match op {
        "<" => ">",
        ">" => "<",
        "<=" => ">=",
        ">=" => "<=",
        "≤" => "≥",
        "≥" => "≤",
        op => op,
    }
}

/// One block per relationship of `tableName`: `Source:file`, `Table:other`,
/// `Options:` (Delete, Create, Sorted of the other side), then one line per
/// criterion, the other occurrence first. Blocks are separated by an empty
/// line (checked against FileMaker on the Tasks template).
pub fn relation_info(ev: &mut Evaluator, a: &[Value]) -> R {
    let s = schema(ev, &txt(a, 0))?;
    let me = txt(a, 1);
    if s.occurrence(&me).is_none() {
        return Err(EvalError::new(
            code::TABLE_MISSING,
            format!("table {me} is missing"),
        ));
    }
    let mut blocks = vec![];
    for r in &s.relationships {
        let mine_left = r.left.eq_ignore_ascii_case(&me);
        if !mine_left && !r.right.eq_ignore_ascii_case(&me) {
            continue;
        }
        let (other, opts) = if mine_left {
            (&r.right, r.right_options)
        } else {
            (&r.left, r.left_options)
        };
        let source = s
            .occurrence(other)
            .map(|o| o.source.clone())
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| s.file.clone());
        let words: Vec<&str> = [
            (opts.delete, "Delete"),
            (opts.create, "Create"),
            (opts.sorted, "Sorted"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, w)| *w)
        .collect();
        let mut b = vec![
            format!("Source:{source}"),
            format!("Table:{other}"),
            format!("Options:{}", words.join(" ")),
        ];
        for (lf, op, rf) in &r.criteria {
            b.push(if mine_left {
                format!("{}::{rf} {} {}::{lf}", r.right, flip(op), r.left)
            } else {
                format!("{}::{lf} {op} {}::{rf}", r.left, r.right)
            });
        }
        blocks.push(b.join("\r"));
    }
    Ok(Value::Text(blocks.join("\r\r")))
}

// ---- GetTableDDL -----------------------------------------------------------------

fn sql_type(t: DataType) -> &'static str {
    match t {
        DataType::Text => "varchar(255)",
        DataType::Number => "int",
        DataType::Date | DataType::Time | DataType::Timestamp => "datetime",
        DataType::Container => "varbinary(4096)",
    }
}

// ponytail: FileMaker's rule for the primary key is undocumented; the
// field named PrimaryKey (the default in new tables) is used.
fn primary_key(fields: &[Field]) -> Option<&Field> {
    fields
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case("PrimaryKey"))
}

fn ddl(s: &Schema, to: &str) -> Option<String> {
    let o = s.occurrence(to)?;
    let t = s.base_table(&o.base_table)?;
    let annotated = t.fields.iter().any(|f| !f.annotation.is_empty());
    let cols: Vec<(&Field, &str)> = t
        .fields
        .iter()
        .filter(|f| !annotated || !f.annotation.is_empty())
        .map(|f| {
            let note = if annotated { &f.annotation } else { &f.comment };
            (f, note.as_str())
        })
        .collect();
    // FileMaker writes no FOREIGN KEY for the template's ForeignKey fields.
    let keys: Vec<String> = primary_key(&t.fields)
        .map(|pk| format!("PRIMARY KEY ({})", pk.name))
        .into_iter()
        .collect();
    // One line per column, CR-separated, comments after the comma; a blank
    // line after each table:
    // CREATE TABLE "T" (\r"a" varchar(255), /*note*/\rPRIMARY KEY (a)\r);\r\r
    let mut out = format!("CREATE TABLE \"{}\" (\r", o.name);
    let n = cols.len() + keys.len();
    let mut i = 0;
    for (f, note) in cols {
        i += 1;
        out.push_str(&format!("\"{}\" {}", f.name, sql_type(f.data_type)));
        if i < n {
            out.push(',');
        }
        if !note.is_empty() {
            out.push_str(&format!(" /*{note}*/"));
        }
        out.push('\r');
    }
    for k in keys {
        out.push_str(&k);
        out.push('\r');
    }
    out.push_str(");\r\r");
    Some(out)
}

/// GetTableDDL ( tableOccurrenceNames ; ignoreError ): CREATE TABLE text for
/// a JSON array of occurrence names.
pub fn get_table_ddl(ev: &mut Evaluator, a: &[Value]) -> R {
    let names: Vec<String> = match serde_json::from_str::<J>(&txt(a, 0)) {
        Ok(J::Array(v)) => v
            .into_iter()
            .map(|n| match n {
                J::String(s) => s,
                n => n.to_string(),
            })
            .collect(),
        _ => return Err(EvalError::generic("GetTableDDL needs a JSON array")),
    };
    let ignore = a[1].truth();
    let s = schema(ev, "")?;
    let mut out = String::new();
    for n in &names {
        match ddl(&s, n) {
            Some(d) => out.push_str(&d),
            None if ignore => {}
            None => return Ok(Value::text("?")),
        }
    }
    if out.is_empty() {
        return Ok(Value::text("?"));
    }
    Ok(Value::Text(out))
}

// ---- records -----------------------------------------------------------------

fn field_ref(e: &Expr, what: &str) -> Result<(Option<String>, String), EvalError> {
    match e {
        Expr::Field { table, name, .. } => Ok((table.clone(), name.clone())),
        _ => Err(EvalError::new(
            code::NAME_MISSING,
            format!("{what} needs a field"),
        )),
    }
}

pub fn get_nth_record(ev: &mut Evaluator, a: &[Expr]) -> R {
    let Expr::Field { table, name, rep } = &a[0] else {
        return Err(EvalError::new(
            code::NAME_MISSING,
            "GetNthRecord needs a field",
        ));
    };
    let rep = match rep {
        Some(r) => to_index(&ev.eval(r)?)?.max(0) as usize,
        None => 1,
    };
    let n = to_index(&ev.eval(&a[1])?)?;
    if n < 1 {
        return Err(EvalError::generic("record number must be at least 1"));
    }
    ev.ctx.nth_record(table.as_deref(), name, rep, n as usize)
}

/// Lookup ( sourceField {; failExpression} ): the first related value, or
/// failExpression when there is no related record.
pub fn lookup(ev: &mut Evaluator, a: &[Expr]) -> R {
    let (table, name) = field_ref(&a[0], "Lookup")?;
    let vals = ev.ctx.field_values(table.as_deref(), &name)?;
    match vals.into_iter().next() {
        Some(v) => Ok(v),
        None => a.get(1).map_or(Ok(Value::empty()), |e| ev.eval(e)),
    }
}

/// LookupNext ( sourceField ; Lower | Higher ).
pub fn lookup_next(ev: &mut Evaluator, a: &[Expr]) -> R {
    let (table, name) = field_ref(&a[0], "LookupNext")?;
    let flag = match &a[1] {
        Expr::Field {
            table: None, name, ..
        } if ev.local(name).is_none() => name.clone(),
        e => ev.eval(e)?.to_text(),
    };
    let higher = match flag.to_ascii_lowercase().as_str() {
        "higher" => true,
        "lower" => false,
        _ => return Err(EvalError::generic("LookupNext needs Lower or Higher")),
    };
    ev.ctx.lookup_next(table.as_deref(), &name, higher)
}

pub fn get_summary(ev: &mut Evaluator, a: &[Expr]) -> R {
    let (st, sf) = field_ref(&a[0], "GetSummary")?;
    let (bt, bf) = field_ref(&a[1], "GetSummary")?;
    ev.ctx.summary((st.as_deref(), &sf), (bt.as_deref(), &bf))
}

pub fn self_(ev: &mut Evaluator, _: &[Value]) -> R {
    ev.ctx.self_value()
}

/// Status ( CurrentX ): the pre-7.0 spelling of Get ( X ).
pub fn status(ev: &mut Evaluator, a: &[Expr]) -> R {
    let Expr::Field {
        table: None, name, ..
    } = &a[0]
    else {
        return Err(EvalError::new(
            code::BAD_GET_PARAM,
            "invalid Status parameter",
        ));
    };
    let get = |flag: &str| Expr::Call {
        name: "Get".into(),
        args: vec![Expr::Field {
            table: None,
            name: flag.into(),
            rep: None,
        }],
    };
    if name.eq_ignore_ascii_case("CurrentGroups") {
        return ev.eval(&get("PrivilegeSetName"));
    }
    // Status ( CurrentDate ) is Get ( CurrentDate ); Status ( CurrentRecordID )
    // is Get ( RecordID ).
    match ev.eval(&get(name)) {
        Err(e) if e.code == code::BAD_GET_PARAM => match (name.get(..7), name.get(7..)) {
            (Some(p), Some(short)) if p.eq_ignore_ascii_case("current") && !short.is_empty() => {
                ev.eval(&get(short))
            }
            _ => Err(e),
        },
        r => r,
    }
}

/// Consecutive IDs as `first-last`.
fn ranges(ids: &[u64]) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    while i < ids.len() {
        let mut j = i;
        while j + 1 < ids.len() && ids[j + 1] == ids[j] + 1 {
            j += 1;
        }
        out.push(if j > i {
            format!("{}-{}", ids[i], ids[j])
        } else {
            ids[i].to_string()
        });
        i = j + 1;
    }
    out
}

pub fn get_record_ids_from_found_set(ev: &mut Evaluator, a: &[Value]) -> R {
    let ty = super::int(a, 0)?;
    let table = a.get(1).map(Value::to_text).filter(|t| !t.is_empty());
    let ids = ev.ctx.record_ids(table.as_deref())?;
    let strs = |v: Vec<String>| J::Array(v.into_iter().map(J::String).collect()).to_string();
    Ok(Value::Text(match ty {
        0 => ids
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("\r"),
        1 => strs(ids.iter().map(u64::to_string).collect()),
        2 => J::Array(ids.iter().map(|i| J::from(*i)).collect()).to_string(),
        3 => ranges(&ids).join("\r"),
        4 => strs(ranges(&ids)),
        _ => return Err(EvalError::generic("invalid GetRecordIDsFromFoundSet type")),
    }))
}

#[cfg(test)]
mod tests {
    use super::ranges;

    #[test]
    fn id_ranges() {
        assert_eq!(ranges(&[1, 5, 21, 22, 23, 7]), ["1", "5", "21-23", "7"]);
        assert!(ranges(&[]).is_empty());
    }
}
