//! Execute FileMaker Data API on the current file, as Claris documents the
//! step: a JSON request in, the Data API's JSON reply out (`messages`,
//! `response`). Actions: `read` (all records, `recordId`, `query` with
//! omit requests and related fields, `sort`, `offset`, `limit`,
//! `layout.response`, `portal`, `offset.<portal>`, `limit.<portal>`),
//! `metaData` (table occurrences), `create`, `update`, `delete`,
//! `duplicate` (`fieldData`; `options.entrymode` "script" skips
//! validation). Keys are written in alphabetical order, as in Claris's
//! examples.
//!
//! Not done: `portalData` in writes, `version` differences (v1, v2 and
//! vLatest answer alike), the `modId` check (BaseCraft keeps no
//! modification count: every record answers `"0"`), `prohibitmode`.

use serde_json::{Map, Value as Json, json};

use super::*;

const COMMAND_INVALID: u32 = 3;
const TABLE_MISSING: u32 = 106;
const BAD_VALUE: u32 = 1708;
const BAD_JSON: u32 = 1710;

/// The message FileMaker's Data API gives with a code.
pub fn message(code: u32) -> &'static str {
    match code {
        0 => "OK",
        3 => "Command is unavailable",
        101 => "Record is missing",
        102 => "Field is missing",
        105 => "Layout is missing",
        106 => "Table is missing",
        401 => "No records match the request",
        500..=511 => "Field value does not meet validation entry options",
        1708 => "Parameter value is invalid",
        1710 => "JSON input is not syntactically valid",
        _ => "Unknown error",
    }
}

/// Runs a request; returns the error code and the reply text.
pub fn execute(host: &mut dyn Host, file: &str, request: &str) -> (u32, String) {
    let (code, response) = match run(host, file, request) {
        Ok(r) => (0, r),
        Err(c) => (c, json!({})),
    };
    let out = json!({
        "messages": [{"code": code.to_string(), "message": message(code)}],
        "response": response,
    });
    (code, out.to_string())
}

/// A field as the Data API names it: (name, occurrence, field).
type Named = (String, u32, u32);

/// A layout's fields and portals as the Data API shows them.
struct Shape {
    fields: Vec<Named>,
    /// (portal name, occurrence, its fields).
    portals: Vec<(String, u32, Vec<Named>)>,
}

fn layout<'a>(db: &'a Database, name: &str) -> Result<&'a fmp12::layouts::Layout, u32> {
    db.layouts
        .iter()
        .find(|l| l.name == name && l.table_occurrence != 0)
        .ok_or(error::LAYOUT_MISSING)
}

fn to_name(db: &Database, to: u32) -> String {
    db.occurrences
        .iter()
        .find(|o| o.id == to)
        .map(|o| o.name.clone())
        .unwrap_or_default()
}

fn shape(db: &Database, l: &fmp12::layouts::Layout) -> Shape {
    let home = (l.table_occurrence & 0xFFF) as u32;
    let field_of = |o: &fmp12::layouts::LayoutObject, qualify: bool| {
        let (to, f, rep) = o.field?;
        let to = (to & 0xFFF) as u32;
        if rep > 1 {
            return None;
        }
        let name = db.table_of(to)?.field(f as u32)?.name.clone();
        let name = if qualify || to != home {
            format!("{}::{name}", to_name(db, to))
        } else {
            name
        };
        Some((name, to, f as u32))
    };
    let mut s = Shape {
        fields: Vec::new(),
        portals: Vec::new(),
    };
    let portals: Vec<&fmp12::layouts::LayoutObject> =
        l.objects.iter().filter(|o| o.portal.is_some()).collect();
    for o in &l.objects {
        let in_portal = o.parent.is_some_and(|p| portals.iter().any(|x| x.id == p));
        if !in_portal && let Some(f) = field_of(o, false).filter(|f| !s.fields.contains(f)) {
            s.fields.push(f);
        }
    }
    for p in portals {
        let Some((to, ..)) = p.portal else { continue };
        let to = (to & 0xFFF) as u32;
        let name = if p.name.is_empty() {
            to_name(db, to)
        } else {
            p.name.clone()
        };
        let fields = l
            .objects
            .iter()
            .filter(|o| o.parent == Some(p.id))
            .filter_map(|o| field_of(o, true))
            .collect();
        s.portals.push((name, to, fields));
    }
    s
}

/// A field's value as the Data API gives it: numbers as JSON numbers,
/// containers as their file name, the rest as text.
fn json_value(db: &Database, table: u32, record: usize, field: u32) -> Json {
    let v = db.value(table, record, field);
    let dt = db
        .table(table)
        .and_then(|t| t.field(field))
        .map(|f| f.data_type);
    match (dt, &v) {
        (_, Value::Container(c)) => Json::String(c.name.clone()),
        (Some(fmp12::schema::DataType::Number), _) => {
            let t = v.to_text();
            match serde_json::from_str::<Json>(&t) {
                Ok(n @ Json::Number(_)) => n,
                _ => Json::String(t),
            }
        }
        _ => Json::String(v.to_text()),
    }
}

fn record_id(db: &Database, table: u32, r: usize) -> String {
    db.table(table)
        .and_then(|t| t.records.get(r))
        .map_or(String::new(), |x| x.id.to_string())
}

/// A number in the request: JSON number or numeric text.
fn number(j: &Json, key: &str, default: usize) -> Result<usize, u32> {
    match j.get(key) {
        None | Some(Json::Null) => Ok(default),
        Some(Json::Number(n)) => n.as_u64().map(|n| n as usize).ok_or(BAD_VALUE),
        Some(Json::String(s)) => s.trim().parse().map_err(|_| BAD_VALUE),
        _ => Err(BAD_VALUE),
    }
}

/// The text of a JSON value as a criterion or field value.
fn text(j: &Json) -> String {
    match j {
        Json::String(s) => s.clone(),
        Json::Null => String::new(),
        j => j.to_string(),
    }
}

/// A field name of a request: `Field` of the layout's table or
/// `Occurrence::Field`; (occurrence, field).
fn field_ref(db: &Database, home: u32, name: &str) -> Result<(u32, u32), u32> {
    let (to, fname) = match name.split_once("::") {
        Some((t, f)) => (
            db.occurrences
                .iter()
                .find(|o| o.name == t)
                .map(|o| o.id)
                .ok_or(error::FIELD_MISSING)?,
            f,
        ),
        None => (home, name),
    };
    let f = db
        .table_of(to)
        .and_then(|t| t.field_by_name(fname))
        .ok_or(error::FIELD_MISSING)?;
    Ok((to, f.id))
}

/// A request's value: a JSON object, or text holding one (`query` and
/// `sort` are often passed as text).
fn object_or_text(j: Option<&Json>) -> Result<Option<Json>, u32> {
    match j {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Json::String(s)) => serde_json::from_str(s).map(Some).map_err(|_| BAD_JSON),
        Some(j) => Ok(Some(j.clone())),
    }
}

fn run(host: &mut dyn Host, file: &str, request: &str) -> Result<Json, u32> {
    let req: Json = serde_json::from_str(request).map_err(|_| BAD_JSON)?;
    if !req.is_object() {
        return Err(BAD_JSON);
    }
    let action = match req.get("action") {
        None => "read".to_string(),
        Some(Json::String(a)) => a.to_ascii_lowercase(),
        Some(_) => return Err(BAD_VALUE),
    };
    if action == "metadata" {
        return metadata(host.db(), &req);
    }
    let lname = match req.get("layouts") {
        Some(Json::String(s)) if !s.is_empty() => s.clone(),
        None => return Err(COMMAND_INVALID),
        _ => return Err(BAD_VALUE),
    };
    let db = host.db();
    let l = layout(db, &lname)?;
    let home = (l.table_occurrence & 0xFFF) as u32;
    let table = db.table_of(home).ok_or(TABLE_MISSING)?.id;
    let by_id = |db: &Database| -> Result<usize, u32> {
        let id = text(req.get("recordId").ok_or(error::RECORD_MISSING)?);
        let id: u32 = id.trim().parse().map_err(|_| BAD_VALUE)?;
        db.table(table)
            .and_then(|t| t.records.iter().position(|r| r.id == id))
            .ok_or(error::RECORD_MISSING)
    };
    match action.as_str() {
        "read" => read(db, file, &req, l, home, table),
        "create" | "update" => {
            let fields = match req.get("fieldData") {
                Some(Json::Object(m)) => m.clone(),
                None => Map::new(),
                _ => return Err(BAD_VALUE),
            };
            let mut values = Vec::new();
            for (k, v) in &fields {
                let (to, f) = field_ref(db, home, k)?;
                if to != home {
                    return Err(error::FIELD_MISSING); // ponytail: related fields not written
                }
                values.push((engine::rep_key(f, 1), text(v)));
            }
            let (r, created) = if action == "create" {
                (db.new_record(table).ok_or(TABLE_MISSING)?, true)
            } else {
                (by_id(db)?, false)
            };
            let old = db.table(table).and_then(|t| t.records.get(r)).cloned();
            db.set_values(table, r, &values);
            let script = req
                .pointer("/options/entrymode")
                .and_then(Json::as_str)
                .is_some_and(|m| m.eq_ignore_ascii_case("script"));
            if !script {
                let bad = db.validate(table, r).first().map(|b| b.rule.code());
                if let Some(code) = bad {
                    if created {
                        delete_records(host, table, &[r]);
                    } else if let (Some(t), Some(old)) =
                        (host.db().tables.iter_mut().find(|t| t.id == table), old)
                    {
                        t.records[r] = old;
                    }
                    return Err(code);
                }
            }
            let mut out = json!({"modId": "0"});
            if created {
                out["recordId"] = json!(record_id(host.db(), table, r));
            }
            Ok(out)
        }
        "duplicate" => {
            let r = by_id(db)?;
            let n = db.duplicate_record(table, r).ok_or(error::RECORD_MISSING)?;
            Ok(json!({"modId": "0", "recordId": record_id(db, table, n)}))
        }
        "delete" => {
            let r = by_id(db)?;
            delete_records(host, table, &[r]);
            Ok(json!({}))
        }
        _ => Err(BAD_VALUE),
    }
}

fn metadata(db: &Database, req: &Json) -> Result<Json, u32> {
    let only = req.get("tables").map(text).filter(|t| !t.is_empty());
    let tables: Vec<Json> = db
        .occurrences
        .iter()
        .filter(|o| only.as_ref().is_none_or(|n| *n == o.name))
        .map(|o| {
            let base = db.table(o.base_table).map_or("", |t| t.name.as_str());
            json!({"baseTable": base, "name": o.name})
        })
        .collect();
    if tables.is_empty() && only.is_some() {
        return Err(TABLE_MISSING);
    }
    Ok(json!({ "tables": tables }))
}

fn read(
    db: &Database,
    file: &str,
    req: &Json,
    l: &fmp12::layouts::Layout,
    home: u32,
    table: u32,
) -> Result<Json, u32> {
    let total = db.table(table).map_or(0, |t| t.records.len());
    let mut found: Vec<usize> = if req.get("recordId").is_some() {
        if req.get("query").is_some() {
            return Err(BAD_VALUE);
        }
        let id: u32 = text(&req["recordId"])
            .trim()
            .parse()
            .map_err(|_| BAD_VALUE)?;
        let r = db
            .table(table)
            .and_then(|t| t.records.iter().position(|r| r.id == id))
            .ok_or(error::RECORD_MISSING)?;
        vec![r]
    } else if let Some(q) = object_or_text(req.get("query"))? {
        let Json::Array(q) = q else {
            return Err(BAD_VALUE);
        };
        let mut reqs = Vec::new();
        for item in &q {
            let Json::Object(m) = item else {
                return Err(BAD_VALUE);
            };
            let mut r = Request {
                criteria: Vec::new(),
                omit: false,
            };
            for (k, v) in m {
                if k.eq_ignore_ascii_case("omit") {
                    r.omit = text(v).eq_ignore_ascii_case("true");
                    continue;
                }
                let (to, f) = field_ref(db, home, k)?;
                let key = if to == home {
                    f
                } else {
                    engine::related_key(to, f)
                };
                r.criteria.push((key, text(v)));
            }
            reqs.push(r);
        }
        let hits = engine::find_from(db, home, &reqs);
        if hits.is_empty() {
            return Err(error::NO_MATCH);
        }
        hits
    } else {
        (0..total).collect()
    };
    if let Some(sort) = object_or_text(req.get("sort"))? {
        let Json::Array(sort) = sort else {
            return Err(BAD_VALUE);
        };
        let mut keys = Vec::new();
        for k in &sort {
            let name = k.get("fieldName").map(text).ok_or(BAD_VALUE)?;
            let (to, f) = field_ref(db, home, &name)?;
            if to != home {
                return Err(BAD_VALUE);
            }
            let order = match k.get("sortOrder").map(text).as_deref() {
                None | Some("ascend") => engine::Order::Ascending,
                Some("descend") => engine::Order::Descending,
                Some(_) => return Err(BAD_VALUE),
            };
            keys.push((f, order));
        }
        engine::sort(db, table, &mut found, &keys);
    }
    let offset = number(req, "offset", 1)?.max(1);
    let limit = number(req, "limit", 100)?;
    let out_layout = match req.get("layout.response").map(text) {
        Some(n) if !n.is_empty() => layout(db, &n)?,
        _ => l,
    };
    let sh = shape(db, out_layout);
    let wanted: Option<Vec<String>> = match object_or_text(req.get("portal"))? {
        Some(Json::Array(a)) => Some(a.iter().map(text).collect()),
        Some(_) => return Err(BAD_VALUE),
        None => None,
    };
    let mut data = Vec::new();
    for &r in found.iter().skip(offset - 1).take(limit) {
        let mut fd = Map::new();
        for (name, to, f) in &sh.fields {
            let v = if *to == home {
                json_value(db, table, r, *f)
            } else {
                match (db.table_of(*to), db.related(home, r, *to).first()) {
                    (Some(t), Some(&x)) => json_value(db, t.id, x, *f),
                    _ => Json::String(String::new()),
                }
            };
            fd.insert(name.clone(), v);
        }
        let mut pd = Map::new();
        for (pname, to, fields) in &sh.portals {
            if wanted.as_ref().is_some_and(|w| !w.contains(pname)) {
                continue;
            }
            let Some(pt) = db.table_of(*to).map(|t| t.id) else {
                continue;
            };
            let po = number(req, &format!("offset.{pname}"), 1)?.max(1);
            let pl = number(req, &format!("limit.{pname}"), 50)?;
            let rows: Vec<Json> = db
                .related(home, r, *to)
                .into_iter()
                .skip(po - 1)
                .take(pl)
                .map(|x| {
                    let mut row = Map::new();
                    for (n, _, f) in fields {
                        row.insert(n.clone(), json_value(db, pt, x, *f));
                    }
                    row.insert("modId".into(), json!("0"));
                    row.insert("recordId".into(), json!(record_id(db, pt, x)));
                    Json::Object(row)
                })
                .collect();
            pd.insert(pname.clone(), Json::Array(rows));
        }
        data.push(json!({
            "fieldData": fd,
            "modId": "0",
            "portalData": pd,
            "recordId": record_id(db, table, r),
        }));
    }
    Ok(json!({
        "data": data,
        "dataInfo": {
            "database": file,
            "foundCount": found.len(),
            "layout": l.name,
            "returnedCount": data.len(),
            "table": to_name(db, home),
            "totalRecordCount": total,
        }
    }))
}
