//! `basecraft-cli`: drive BaseCraft from scripts and AI agents.
//!
//! Every command prints one JSON document on stdout. Errors print
//! `{"error": "..."}` and exit with status 1. Nothing is written to disk
//! unless `-o <out.fmp12>` or `--in-place` is given.
//!
//! ```text
//! basecraft-cli info    <file>
//! basecraft-cli records <file> <table> [--find Field=criterion]… [--omit Field=criterion]…
//!                       [--sort Field[:desc]]… [--limit N]
//! basecraft-cli eval    <file> <calculation> [--table T] [--record ID]
//! basecraft-cli new     <file> <table> [Field=value]…        (-o out | --in-place; Container=@file inserts a file)
//! basecraft-cli set     <file> <table> <id> Field=value…      (-o out | --in-place)
//! basecraft-cli delete  <file> <table> <id>                   (-o out | --in-place)
//! basecraft-cli export  <file> <table> [-o out.csv|.tab|.mer|.xml|.xlsx|.json] [--format F]
//!                       [--field F]… [--formatted] [--find…] [--omit…] [--sort…]
//! basecraft-cli import  <file> <table> <source> [--format F] [--names] [--by-name | --map F…]
//!                       [--mode add|update|replace] [--match F]… [--add-remaining]
//!                       [--no-auto-enter] [--find…] (-o out | --in-place)
//! basecraft-cli print-pdf <file> <layout> -o out.pdf [--paper letter|a4] [--landscape]
//!                       [--margin PT] [--find…] [--omit…] [--sort…]
//! basecraft-cli scripts <file>                                 list scripts and their steps
//! basecraft-cli run-script <file> <name|id> [--param P] [--layout L] [--answer N] [-o out | --in-place]
//! basecraft-cli schema add-table|add-field|set-options|add-occurrence|add-relationship|set-relationship|
//!                       add-valuelist|set-valuelist|rename|delete <file> …   (-o out | --in-place)
//! basecraft-cli accounts [add|set|delete] <file> …   privileges [add|set|delete] <file> …
//!                       (File › Manage › Security; see `security.rs`)
//! basecraft-cli functions [add|set|delete] <file> …   (File › Manage › Custom Functions;
//!                       see `functions.rs`)
//! basecraft-cli new-file <out.fmp12> [--table Name]
//! basecraft-cli session <file>     JSON-lines commands on stdin, one reply per line
//! basecraft-cli mcp [file]         Model Context Protocol server on stdio (for AI agents)
//! ```
//!
//! A protected file (one that does not log in by itself) opens with
//! `--account NAME --password PW` anywhere on the line (or the password in
//! `BASECRAFT_PASSWORD`).
//!
//! `--remote https://host/Database --account A --password-env VAR` runs
//! `info`, `records`, `find`, `new`, `set`, `delete` and `run-script` on a
//! file hosted by FileMaker Server, through its Data API (`hosted.rs`).

use std::io::{BufRead, Write};

mod functions;
mod hosted;
mod io;
mod mcp;
mod schema;
mod scripting;
mod security;

use engine::{Database, Order, Request};
use serde_json::{Value as Json, json};

type Res<T> = Result<T, String>;

struct Doc {
    path: String,
    file: fmp12::File,
    db: Database,
    /// Script engine, created on first use; keeps `$$globals` across runs.
    engine: Option<script::Engine>,
}

impl Doc {
    fn open(path: &str, login: Option<&Login>) -> Res<Doc> {
        let file = fmp12::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let db = match login {
            Some(l) => Database::load_as(&file, &l.account, &l.password),
            None => Database::load(&file),
        }
        .map_err(|e| match e {
            fmp12::Error::LoginRequired => {
                format!("{path}: this file is protected: open it with --account NAME --password PW")
            }
            e => format!("{path}: {e}"),
        })?;
        let mut db = db;
        // Container files kept in open external storage next to the file.
        if let Some(dir) = std::path::Path::new(path).parent() {
            db.read_external_containers(dir);
        }
        Ok(Doc {
            path: path.to_string(),
            file,
            db,
            engine: None,
        })
    }

    fn engine(&self) -> Res<script::Engine> {
        script::Engine::load(&self.file).map_err(|e| e.to_string())
    }

    fn take_engine(&mut self) -> Res<script::Engine> {
        match self.engine.take() {
            Some(e) => Ok(e),
            None => self.engine(),
        }
    }

    fn table(&self, name: &str) -> Res<&engine::Table> {
        self.db
            .tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .or_else(|| {
                let o = self
                    .db
                    .occurrences
                    .iter()
                    .find(|o| o.name.eq_ignore_ascii_case(name))?;
                self.db.table(o.base_table)
            })
            .ok_or_else(|| format!("no table or occurrence named {name:?}"))
    }

    fn field_id(&self, table: &engine::Table, name: &str) -> Res<u32> {
        table
            .field_by_name(name)
            .map(|f| f.id)
            .ok_or_else(|| format!("no field {name:?} in table {:?}", table.name))
    }

    fn index_of(&self, table: u32, id: u32) -> Res<usize> {
        self.db
            .table(table)
            .and_then(|t| t.records.iter().position(|r| r.id == id))
            .ok_or_else(|| format!("no record with id {id}"))
    }

    /// Record `i` of `t`; summary fields over the found set `found`
    /// sorted by fields `sort` (running totals up to the record…).
    fn record_json(&self, t: &engine::Table, i: usize, found: Option<(&[usize], &[u32])>) -> Json {
        let mut fields = serde_json::Map::new();
        for f in &t.fields {
            let v = match found {
                Some((found, sort)) if f.kind == fmp12::schema::FieldKind::Summary => {
                    engine::summary_at(&self.db, t.id, f.id, i, found, sort)
                }
                _ => self.db.value(t.id, i, f.id),
            };
            fields.insert(f.name.clone(), value_json(&v));
        }
        json!({ "id": t.records[i].id, "fields": fields })
    }

    fn save(&mut self, out: &str) -> Res<Json> {
        let bytes = self.db.save(&self.file).map_err(|e| e.to_string())?;
        std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
        if let Some(dir) = std::path::Path::new(out).parent() {
            self.db
                .write_external_containers(dir)
                .map_err(|e| format!("{out}: container files: {e}"))?;
        }
        self.file = fmp12::File::from_bytes(bytes).map_err(|e| e.to_string())?;
        Ok(json!({ "saved": out }))
    }
}

/// Account name and password to open a protected file with.
pub struct Login {
    pub account: String,
    pub password: String,
}

/// Takes `--account A` and `--password P` out of `args`.
fn take_login(args: &mut Vec<String>) -> Res<Option<Login>> {
    let mut take = |name: &str| -> Res<Option<String>> {
        let Some(i) = args.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if i + 1 >= args.len() {
            return Err(format!("{name} needs a value"));
        }
        let v = args.remove(i + 1);
        args.remove(i);
        Ok(Some(v))
    };
    let account = take("--account")?;
    let password = take("--password")?;
    Ok(account.map(|account| Login {
        account,
        password: password
            .or_else(|| std::env::var("BASECRAFT_PASSWORD").ok())
            .unwrap_or_default(),
    }))
}

/// An action the account's privileges refused (FileMaker's error code).
fn denied(d: &mut Doc) -> Res<()> {
    match d.db.last_denied() {
        Some(code) => Err(format!("{} (error {code})", engine::DENIED_MESSAGE)),
        None => Ok(()),
    }
}

fn value_json(v: &calc::Value) -> Json {
    match v {
        calc::Value::Number(n) => {
            let t = v.to_text();
            serde_json::from_str::<Json>(&t)
                .ok()
                .filter(Json::is_number)
                .unwrap_or_else(|| json!(n.to_string()))
        }
        _ => json!(v.to_text()),
    }
}

fn info(d: &Doc) -> Json {
    let db = &d.db;
    let to_name = |id: u32| {
        db.occurrences
            .iter()
            .find(|o| o.id == id)
            .map_or(String::new(), |o| o.name.clone())
    };
    json!({
        "file": d.path,
        "tables": db.tables.iter().map(|t| json!({
            "name": t.name,
            "records": t.records.len(),
            "fields": t.fields.iter().map(|f| json!({
                "name": f.name,
                "type": format!("{:?}", f.data_type),
                "kind": format!("{:?}", f.kind),
                "calculation": f.calc,
                "auto_enter": f.auto_calc,
                "comment": f.def.comment,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "occurrences": db.occurrences.iter().map(|o| json!({
            "name": o.name,
            "table": db.table(o.base_table).map(|t| t.name.clone()),
        })).collect::<Vec<_>>(),
        "relationships": db.relationships.iter().map(|r| schema::relationship_json(db, r)).collect::<Vec<_>>(),
        "layouts": db.layouts.iter().filter(|l| l.table_occurrence != 0 && db.layout_access(l.id) != fmp12::security::NO_ACCESS).map(|l| json!({
            "name": l.name,
            "occurrence": to_name((l.table_occurrence & 0xFFF) as u32),
        })).collect::<Vec<_>>(),
        "accounts": if db.full_access() { db.accounts.iter().map(|a| json!({
            "name": a.name,
            "privilege_set": db.privilege_sets.iter().find(|p| p.id == a.privilege_set).map(|p| p.name.clone()),
        })).collect::<Vec<_>>() } else { Vec::new() },
        "logged_in": db.session().map(|s| json!({
            "account": s.account_name(),
            "privilege_set": s.privileges().name,
            "extended_privileges": s.extended(),
        })),
        "value_lists": db.value_lists.iter().map(|v| schema::value_list_json(db, v)).collect::<Vec<_>>(),
    })
}

/// `Field=value` pairs.
fn pairs(args: &[String]) -> Res<Vec<(String, String)>> {
    args.iter()
        .map(|a| {
            a.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("expected Field=value, got {a:?}"))
        })
        .collect()
}

/// Options shared by the record commands.
struct Query {
    find: Vec<(String, String)>,
    omit: Vec<(String, String)>,
    sort: Vec<(String, Order)>,
    limit: Option<usize>,
}

fn records(d: &Doc, table: &str, q: &Query) -> Res<Json> {
    let t = d.table(table)?;
    let found = found_set(d, t, q)?;
    let sort: Vec<u32> = sort_keys(d, t, q)?.iter().map(|(f, _)| *f).collect();
    let shown = &found[..found.len().min(q.limit.unwrap_or(usize::MAX))];
    Ok(json!({
        "table": t.name,
        "found": found.len(),
        "records": shown.iter().map(|&i| d.record_json(t, i, Some((&found, &sort)))).collect::<Vec<_>>(),
    }))
}

/// Sort keys of `--sort` options.
fn sort_keys(d: &Doc, t: &engine::Table, q: &Query) -> Res<Vec<(u32, Order)>> {
    q.sort
        .iter()
        .map(|(f, o)| Ok((d.field_id(t, f)?, *o)))
        .collect()
}

/// Records matching `--find`/`--omit` (all records without them), in `--sort` order.
fn found_set(d: &Doc, t: &engine::Table, q: &Query) -> Res<Vec<usize>> {
    let crit = |list: &[(String, String)]| -> Res<Vec<(u32, String)>> {
        list.iter()
            .map(|(f, c)| Ok((d.field_id(t, f)?, c.clone())))
            .collect()
    };
    let mut found: Vec<usize> = if q.find.is_empty() && q.omit.is_empty() {
        (0..t.records.len()).collect()
    } else {
        let mut reqs = vec![Request {
            criteria: crit(&q.find)?,
            omit: false,
        }];
        if !q.omit.is_empty() {
            reqs.push(Request {
                criteria: crit(&q.omit)?,
                omit: true,
            });
        }
        engine::find(&d.db, t.id, &reqs)
    };
    let keys = sort_keys(d, t, q)?;
    if !keys.is_empty() {
        engine::sort(&d.db, t.id, &mut found, &keys);
    }
    Ok(found)
}

fn eval(d: &Doc, src: &str, table: Option<&str>, record: Option<u32>) -> Res<Json> {
    let t = match table {
        Some(n) => d.table(n)?,
        None => d.db.tables.first().ok_or("file has no tables")?,
    };
    let to =
        d.db.occurrences
            .iter()
            .find(|o| table.is_some_and(|n| o.name.eq_ignore_ascii_case(n)))
            .or_else(|| d.db.occurrences.iter().find(|o| o.base_table == t.id))
            .map_or(0, |o| o.id);
    let rec = match record {
        Some(id) => Some(d.index_of(t.id, id)?),
        None => (!t.records.is_empty()).then_some(0),
    };
    let v = d.db.eval_source(src, t.id, to, rec);
    Ok(json!({ "result": value_json(&v) }))
}

fn set_fields(d: &mut Doc, table: u32, idx: usize, values: &[(String, String)]) -> Res<()> {
    let t = d.db.table(table).ok_or("no table")?;
    let mut ids: Vec<(u32, String)> = Vec::new();
    // `Container=@path`: inserts that file, as Insert File does.
    let mut files: Vec<(u32, String)> = Vec::new();
    for (f, v) in values {
        let id = d.field_id(t, f)?;
        let container = t
            .field(id)
            .is_some_and(|x| x.data_type == fmp12::schema::DataType::Container);
        match v.strip_prefix('@') {
            Some(p) if container => files.push((id, p.to_string())),
            _ => ids.push((id, v.clone())),
        }
    }
    d.db.set_values(table, idx, &ids);
    for (id, p) in files {
        let data = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
        let name = std::path::Path::new(&p)
            .file_name()
            .map_or(p.clone(), |n| n.to_string_lossy().into_owned());
        d.db.insert_file(table, idx, id, 1, &name, data);
    }
    denied(d)
}

fn new_record(d: &mut Doc, table: &str, values: &[(String, String)]) -> Res<Json> {
    let tid = d.table(table)?.id;
    let idx = match d.db.new_record(tid) {
        Some(i) => i,
        None => return denied(d).and(Err("no table".into())),
    };
    set_fields(d, tid, idx, values)?;
    let errs = d.db.validate(tid, idx);
    let t = d.db.table(tid).ok_or("no table")?;
    Ok(
        json!({ "record": d.record_json(t, idx, None), "validation_errors": invalid_json(t, &errs) }),
    )
}

fn invalid_json(t: &engine::Table, errs: &[engine::Invalid]) -> Json {
    errs.iter()
        .map(|e| {
            json!({
                "field": t.field(e.field).map(|f| f.name.clone()),
                "rule": format!("{:?}", e.rule),
            })
        })
        .collect()
}

fn set(d: &mut Doc, table: &str, id: u32, values: &[(String, String)]) -> Res<Json> {
    let tid = d.table(table)?.id;
    let idx = d.index_of(tid, id)?;
    set_fields(d, tid, idx, values)?;
    let errs = d.db.validate(tid, idx);
    let t = d.db.table(tid).ok_or("no table")?;
    Ok(
        json!({ "record": d.record_json(t, idx, None), "validation_errors": invalid_json(t, &errs) }),
    )
}

fn delete(d: &mut Doc, table: &str, id: u32) -> Res<Json> {
    let tid = d.table(table)?.id;
    let idx = d.index_of(tid, id)?;
    let gone = d.db.delete_record(tid, idx);
    denied(d)?;
    let also: Vec<Json> = gone
        .iter()
        .skip(1)
        .map(|(t, r)| json!({ "table": d.db.table(*t).map(|t| t.name.clone()), "id": r }))
        .collect();
    Ok(json!({ "deleted": id, "related_deleted": also }))
}

/// Parses `records`/`eval` options and the output target out of `args`.
struct Opts {
    rest: Vec<String>,
    query: Query,
    table: Option<String>,
    record: Option<u32>,
    param: Option<String>,
    layout: Option<String>,
    answer: usize,
    out: Option<String>,
    in_place: bool,
    // export / import / print-pdf
    format: Option<String>,
    fields: Vec<String>,
    formatted: bool,
    names: bool,
    by_name: bool,
    map: Vec<String>,
    mode: Option<String>,
    matches: Vec<String>,
    add_remaining: bool,
    no_auto_enter: bool,
    paper: Option<String>,
    landscape: bool,
    margin: Option<f64>,
}

fn opts(args: &[String]) -> Res<Opts> {
    let mut o = Opts {
        rest: Vec::new(),
        query: Query {
            find: Vec::new(),
            omit: Vec::new(),
            sort: Vec::new(),
            limit: None,
        },
        table: None,
        record: None,
        param: None,
        layout: None,
        answer: 1,
        out: None,
        in_place: false,
        format: None,
        fields: Vec::new(),
        formatted: false,
        names: false,
        by_name: false,
        map: Vec::new(),
        mode: None,
        matches: Vec::new(),
        add_remaining: false,
        no_auto_enter: false,
        paper: None,
        landscape: false,
        margin: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--find" => o.query.find.extend(pairs(&[val("--find")?])?),
            "--omit" => o.query.omit.extend(pairs(&[val("--omit")?])?),
            "--sort" => {
                let v = val("--sort")?;
                let (f, ord) = match v.rsplit_once(':') {
                    Some((f, "desc")) => (f.to_string(), Order::Descending),
                    Some((f, "asc")) => (f.to_string(), Order::Ascending),
                    _ => (v, Order::Ascending),
                };
                o.query.sort.push((f, ord));
            }
            "--limit" => o.query.limit = Some(val("--limit")?.parse().map_err(|_| "bad --limit")?),
            "--table" => o.table = Some(val("--table")?),
            "--record" => o.record = Some(val("--record")?.parse().map_err(|_| "bad --record")?),
            "-o" | "--out" => o.out = Some(val("-o")?),
            "--param" => o.param = Some(val("--param")?),
            "--layout" => o.layout = Some(val("--layout")?),
            "--answer" => o.answer = val("--answer")?.parse().map_err(|_| "bad --answer")?,
            "--in-place" => o.in_place = true,
            "--format" => o.format = Some(val("--format")?),
            "--field" => o.fields.push(val("--field")?),
            "--formatted" => o.formatted = true,
            "--names" => o.names = true,
            "--by-name" => o.by_name = true,
            "--map" => o.map.push(val("--map")?),
            "--mode" => o.mode = Some(val("--mode")?),
            "--match" => o.matches.push(val("--match")?),
            "--add-remaining" => o.add_remaining = true,
            "--no-auto-enter" => o.no_auto_enter = true,
            "--paper" => o.paper = Some(val("--paper")?.to_ascii_lowercase()),
            "--landscape" => o.landscape = true,
            "--margin" => o.margin = Some(val("--margin")?.parse().map_err(|_| "bad --margin")?),
            _ => o.rest.push(a.clone()),
        }
    }
    Ok(o)
}

/// Runs one command against an open document. `save_to` decides writes.
fn run(d: &mut Doc, cmd: &str, o: &Opts) -> Res<Json> {
    let arg = |i: usize, what: &str| {
        o.rest
            .get(i)
            .cloned()
            .ok_or_else(|| format!("{cmd}: missing {what}"))
    };
    let id = |i: usize| -> Res<u32> {
        arg(i, "record id")?
            .parse()
            .map_err(|_| "bad record id".into())
    };
    match cmd {
        "info" => Ok(info(d)),
        "records" => records(d, &arg(0, "table")?, &o.query),
        "eval" => eval(d, &arg(0, "calculation")?, o.table.as_deref(), o.record),
        "new" => new_record(d, &arg(0, "table")?, &pairs(&o.rest[1..])?),
        "set" => set(
            d,
            &arg(0, "table")?,
            id(1)?,
            &pairs(o.rest.get(2..).unwrap_or(&[]))?,
        ),
        "delete" => delete(d, &arg(0, "table")?, id(1)?),
        "export" => io::export(d, &arg(0, "table")?, o),
        "import" => io::import_file(d, &arg(0, "table")?, &arg(1, "source file")?, o),
        "print-pdf" => io::print_pdf(d, &arg(0, "layout")?, o),
        "scripts" => scripting::list(d),
        "schema" => schema::run(d, &o.rest),
        "functions" => functions::run(d, &o.rest),
        "accounts" => security::accounts(d, &o.rest),
        "privileges" => security::privileges(d, &o.rest),
        "run-script" => scripting::run(
            d,
            &arg(0, "script name or id")?,
            o.param.as_deref(),
            o.layout.as_deref(),
            o.answer,
        ),
        "save" => {
            let out = o.out.clone().unwrap_or_else(|| d.path.clone());
            d.save(&out)
        }
        _ => Err(format!("unknown command {cmd:?}")),
    }
}

/// JSON-lines session: each input line is `{"cmd": "...", "args": [...]}`
/// with the same arguments as the command line (without the file).
fn session(d: &mut Doc) -> Res<Json> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = (|| -> Res<Json> {
            let req: Json = serde_json::from_str(&line).map_err(|e| format!("bad JSON: {e}"))?;
            let cmd = req["cmd"].as_str().ok_or("missing \"cmd\"")?;
            if cmd == "quit" {
                return Ok(json!({ "bye": true }));
            }
            let args: Vec<String> = req["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            run(d, cmd, &opts(&args)?)
        })();
        let text = match &reply {
            Ok(v) => v.to_string(),
            Err(e) => json!({ "error": e }).to_string(),
        };
        writeln!(out, "{text}").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        if reply.is_ok_and(|v| v.get("bye").is_some()) {
            break;
        }
    }
    Ok(json!({ "session": "closed" }))
}

fn main_inner() -> Res<Json> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let login = take_login(&mut args)?;
    // A file hosted by FileMaker Server (`hosted.rs`).
    if let Some(c) = hosted::conn(&mut args, login.as_ref())? {
        let (cmd, rest) = args.split_first().ok_or("usage: basecraft-cli <command> --remote https://host/Database --account A --password-env VAR …")?;
        return hosted::run(&c, cmd, rest);
    }
    if args.first().map(String::as_str) == Some("mcp") {
        mcp::serve(args.get(1).map(String::as_str), login.as_ref())?;
        std::process::exit(0);
    }
    if args.first().map(String::as_str) == Some("new-file") {
        let o = opts(&args[1..])?;
        let out = o
            .rest
            .first()
            .or(o.out.as_ref())
            .ok_or("usage: basecraft-cli new-file <out.fmp12> [--table Name]")?;
        return schema::new_file(out, o.table.as_deref().unwrap_or("Table"));
    }
    // `schema <sub> <file> …`: the file comes after the sub-command (also
    // `accounts` and `privileges`, whose list form is `accounts <file>`).
    let sec_sub = |s: &str| matches!(s, "add" | "set" | "delete" | "list");
    let args = match args.as_slice() {
        [s, sub, file, rest @ ..]
            if s == "schema"
                || (matches!(s.as_str(), "accounts" | "privileges" | "functions")
                    && sec_sub(sub)) =>
        {
            [vec![s.clone(), file.clone(), sub.clone()], rest.to_vec()].concat()
        }
        _ => args,
    };
    let [cmd, file, rest @ ..] = args.as_slice() else {
        return Err("usage: basecraft-cli <info|records|eval|new|set|delete|export|import|print-pdf|scripts|run-script|schema|session|mcp> <file.fmp12> [args] (see README)".into());
    };
    let mut d = Doc::open(file, login.as_ref())?;
    if cmd == "session" {
        return session(&mut d);
    }
    let o = opts(rest)?;
    let result = run(&mut d, cmd, &o)?;
    let security_write = matches!(cmd.as_str(), "accounts" | "privileges" | "functions")
        && o.rest.first().is_some_and(|s| s != "list");
    if security_write
        || matches!(
            cmd.as_str(),
            "new" | "set" | "delete" | "run-script" | "import" | "schema"
        )
    {
        let target = match (&o.out, o.in_place) {
            (Some(p), _) => p.clone(),
            (None, true) => file.clone(),
            (None, false) => {
                return Ok(json!({ "result": result, "saved": null,
                    "note": "dry run: add -o <out.fmp12> or --in-place to write" }));
            }
        };
        d.save(&target)?;
        return Ok(json!({ "result": result, "saved": target }));
    }
    Ok(result)
}

fn main() {
    match main_inner() {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => {
            println!("{}", json!({ "error": e }));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_options() {
        let a: Vec<String> = [
            "T",
            "--find",
            "Name=ad",
            "--sort",
            "Name:desc",
            "--limit",
            "5",
            "-o",
            "x.fmp12",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let o = opts(&a).unwrap();
        assert_eq!(o.rest, ["T"]);
        assert_eq!(o.query.find, [("Name".to_string(), "ad".to_string())]);
        assert_eq!(o.query.sort[0].1, Order::Descending);
        assert_eq!(o.query.limit, Some(5));
        assert_eq!(o.out.as_deref(), Some("x.fmp12"));
    }

    #[test]
    fn rejects_bad_pairs() {
        assert!(pairs(&["novalue".to_string()]).is_err());
    }
}
