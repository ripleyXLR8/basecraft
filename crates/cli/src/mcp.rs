//! Model Context Protocol server (`basecraft-cli mcp [file]`).
//!
//! Speaks JSON-RPC 2.0 over stdio, one message per line, as MCP's stdio
//! transport requires. Exposes BaseCraft operations as tools; each tool
//! returns the same JSON the command line prints.

use std::io::{BufRead, Write};

use serde_json::{Value as Json, json};

use crate::{Doc, Res, opts, run};

const PROTOCOL: &str = "2025-06-18";

/// (name, description, JSON schema of the arguments, CLI command)
fn tools() -> Vec<(&'static str, &'static str, Json)> {
    let s = |props: Json, required: &[&str]| json!({ "type": "object", "properties": props, "required": required });
    let table = json!({ "type": "string", "description": "Table or table occurrence name" });
    let fields = json!({ "type": "object", "description": "Field name → value (text, in the file's formats)",
                         "additionalProperties": { "type": "string" } });
    vec![
        (
            "open_file",
            "Open a FileMaker .fmp12 file. Other tools act on the open file.",
            s(json!({ "path": { "type": "string" } }), &["path"]),
        ),
        (
            "describe_database",
            "Tables (fields, types, calculations), table occurrences, relationships, layouts and value lists of the open file.",
            s(json!({}), &[]),
        ),
        (
            "find_records",
            "Find records with FileMaker find syntax (words, =, ==, *, <, >, a...b), optionally omit, sort and limit. Returns records with all field values, calculated fields included.",
            s(
                json!({ "table": table, "find": fields, "omit": fields,
                   "sort": { "type": "array", "items": { "type": "string" }, "description": "Field or Field:desc" },
                   "limit": { "type": "integer" } }),
                &["table"],
            ),
        ),
        (
            "evaluate",
            "Evaluate a FileMaker calculation, optionally in the context of a record (fields with spaces: ${Field Name}).",
            s(
                json!({ "calculation": { "type": "string" }, "table": table, "record_id": { "type": "integer" } }),
                &["calculation"],
            ),
        ),
        (
            "create_record",
            "Create a record (auto-enter options applied) and set field values. Reports validation errors. Changes stay in memory until save_file.",
            s(json!({ "table": table, "fields": fields }), &["table"]),
        ),
        (
            "update_record",
            "Set field values of a record by id. Changes stay in memory until save_file.",
            s(
                json!({ "table": table, "record_id": { "type": "integer" }, "fields": fields }),
                &["table", "record_id", "fields"],
            ),
        ),
        (
            "delete_record",
            "Delete a record by id. Changes stay in memory until save_file.",
            s(
                json!({ "table": table, "record_id": { "type": "integer" } }),
                &["table", "record_id"],
            ),
        ),
        (
            "export_records",
            "Export the found set (find/omit/sort as in find_records) as csv, tab, mer (Merge), xml (FMPXMLRESULT), xlsx or json, with FileMaker's conventions (returns in fields as vertical tab, repetitions and related records joined with GS). `fields`: names or Occurrence::Field (default: every field). Without `path` the text is returned (xlsx needs a path).",
            s(
                json!({ "table": table, "path": { "type": "string" },
                   "format": { "type": "string", "enum": ["csv", "tab", "mer", "xml", "xlsx", "json"] },
                   "fields": { "type": "array", "items": { "type": "string" } },
                   "formatted": { "type": "boolean", "description": "Values in the file's display formats instead of as stored" },
                   "find": fields, "omit": fields,
                   "sort": { "type": "array", "items": { "type": "string" }, "description": "Field or Field:desc" } }),
                &["table"],
            ),
        ),
        (
            "import_records",
            "Import a csv, tab, mer or xlsx file into a table: add records, update matching records of the found set (match_fields), or replace the found set's data. Columns map to the field names of the first row (field_names_row) or, with `map`, to the listed fields in order. Auto-enter applies unless auto_enter is false; validation errors are reported. Changes stay in memory until save_file.",
            s(
                json!({ "table": table, "path": { "type": "string" },
                   "format": { "type": "string", "enum": ["csv", "tab", "mer", "xlsx"] },
                   "field_names_row": { "type": "boolean" },
                   "map": { "type": "array", "items": { "type": "string" }, "description": "Target field per column, in order (\"\" skips a column)" },
                   "mode": { "type": "string", "enum": ["add", "update", "replace"] },
                   "match_fields": { "type": "array", "items": { "type": "string" } },
                   "add_remaining": { "type": "boolean" },
                   "auto_enter": { "type": "boolean" },
                   "find": fields, "omit": fields }),
                &["table", "path"],
            ),
        ),
        (
            "print_pdf",
            "Print a layout to PDF as Preview mode shows it: header and footer on each page, body per record, sub-summaries for the sorted break fields, grand summaries, summary fields computed. Find/omit/sort choose and order the records.",
            s(
                json!({ "layout": { "type": "string" }, "path": { "type": "string" },
                   "paper": { "type": "string", "enum": ["letter", "a4"] },
                   "landscape": { "type": "boolean" }, "margin": { "type": "number", "description": "Margins in points (default 36)" },
                   "find": fields, "omit": fields,
                   "sort": { "type": "array", "items": { "type": "string" } } }),
                &["layout", "path"],
            ),
        ),
        (
            "list_scripts",
            "List the file's FileMaker scripts with their steps.",
            s(json!({}), &[]),
        ),
        (
            "run_script",
            "Run a FileMaker script by name (or id) with an optional parameter, from a layout. Custom dialogs are answered with button `answer` (default 1) and reported. Data changes stay in memory until save_file.",
            s(
                json!({ "name": { "type": "string" }, "parameter": { "type": "string" },
                        "layout": { "type": "string" }, "answer": { "type": "integer" } }),
                &["name"],
            ),
        ),
        (
            "new_file",
            "Create a new, empty FileMaker file at `path` with one table (default name \"Table\") and open it.",
            s(
                json!({ "path": { "type": "string" }, "table": { "type": "string" } }),
                &["path"],
            ),
        ),
        (
            "create_table",
            "Add a table to the open file, with its table occurrence and a layout (Manage Database › Tables). Changes stay in memory until save_file.",
            s(json!({ "name": { "type": "string" } }), &["name"]),
        ),
        (
            "create_field",
            "Add a field to a table (Manage Database › Fields): a normal field of `type`, a calculation (`calculation`, `stored`) or a summary (`summary`: operation and field). Options: auto-enter (serial next value, stamp, auto_calculation, constant), validation (not_empty, unique, existing, range [low, high], validation_calculation, always_validate), storage (global, repetitions, index). Calculations use FileMaker syntax; fields of other tables are written Occurrence::Field. Changes stay in memory until save_file.",
            s(
                json!({ "table": table, "name": { "type": "string" },
                   "type": { "type": "string", "enum": ["text", "number", "date", "time", "timestamp", "container"] },
                   "comment": { "type": "string" },
                   "calculation": { "type": "string" }, "stored": { "type": "boolean" },
                   "summary": { "type": "object", "properties": {
                       "operation": { "type": "string", "enum": ["total", "average", "count", "minimum", "maximum", "stdev", "fraction", "list"] },
                       "field": { "type": "string" } } },
                   "serial": { "type": "string", "description": "Next serial value" },
                   "stamp": { "type": "string", "enum": ["creation-date", "creation-time", "creation-timestamp", "creation-name", "creation-account",
                       "modification-date", "modification-time", "modification-timestamp", "modification-name", "modification-account"] },
                   "auto_calculation": { "type": "string" }, "replace_existing": { "type": "boolean" },
                   "constant": { "type": "string" }, "prohibit_modification": { "type": "boolean" },
                   "not_empty": { "type": "boolean" }, "unique": { "type": "boolean" }, "existing": { "type": "boolean" },
                   "range": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2 },
                   "validation_calculation": { "type": "string" }, "always_validate": { "type": "boolean" },
                   "global": { "type": "boolean" }, "repetitions": { "type": "integer" },
                   "index": { "type": "string", "enum": ["none", "minimal", "all"] } }),
                &["table", "name"],
            ),
        ),
        (
            "save_file",
            "Write the open file with all changes. Give `path` to save a copy; omit it to overwrite the opened file.",
            s(json!({ "path": { "type": "string" } }), &[]),
        ),
    ]
}

/// Turns tool arguments into CLI-style arguments and runs the command.
fn call(doc: &mut Option<Doc>, name: &str, a: &Json) -> Res<Json> {
    let str_of = |k: &str| a.get(k).and_then(Json::as_str).map(str::to_string);
    if name == "new_file" {
        let path = str_of("path").ok_or("path is required")?;
        let table = str_of("table").unwrap_or_else(|| "Table".into());
        let r = crate::schema::new_file(&path, &table)?;
        *doc = Some(Doc::open(&path)?);
        return Ok(r);
    }
    if name == "open_file" {
        let path = str_of("path").ok_or("path is required")?;
        let d = Doc::open(&path)?;
        let summary = json!({ "opened": path, "tables": d.db.tables.iter()
            .map(|t| json!({ "name": t.name, "records": t.records.len() })).collect::<Vec<_>>() });
        *doc = Some(d);
        return Ok(summary);
    }
    let d = doc
        .as_mut()
        .ok_or("no file is open: call open_file first")?;
    let pairs = |k: &str| -> Vec<String> {
        a.get(k)
            .and_then(Json::as_object)
            .map(|m| {
                m.iter()
                    .map(|(f, v)| {
                        format!(
                            "{f}={}",
                            v.as_str().map_or_else(|| v.to_string(), str::to_string)
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut args: Vec<String> = Vec::new();
    let list = |k: &str| -> Vec<String> {
        a.get(k)
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect()
    };
    let flag = |k: &str| a.get(k).and_then(Json::as_bool);
    // --find / --omit / --sort, shared by the found-set tools.
    let mut query = Vec::new();
    for p in pairs("find") {
        query.extend(["--find".into(), p]);
    }
    for p in pairs("omit") {
        query.extend(["--omit".into(), p]);
    }
    for s in list("sort") {
        query.extend(["--sort".into(), s]);
    }
    let cmd = match name {
        "describe_database" => "info",
        "find_records" => {
            args.push(str_of("table").ok_or("table is required")?);
            args.extend(query);
            if let Some(l) = a.get("limit").and_then(Json::as_u64) {
                args.extend(["--limit".into(), l.to_string()]);
            }
            "records"
        }
        "export_records" => {
            args.push(str_of("table").ok_or("table is required")?);
            args.extend(query);
            if let Some(p) = str_of("path") {
                args.extend(["-o".into(), p]);
            }
            if let Some(f) = str_of("format") {
                args.extend(["--format".into(), f]);
            }
            for f in list("fields") {
                args.extend(["--field".into(), f]);
            }
            if flag("formatted") == Some(true) {
                args.push("--formatted".into());
            }
            "export"
        }
        "import_records" => {
            args.push(str_of("table").ok_or("table is required")?);
            args.push(str_of("path").ok_or("path is required")?);
            args.extend(query);
            if let Some(f) = str_of("format") {
                args.extend(["--format".into(), f]);
            }
            if flag("field_names_row") == Some(true) {
                args.push("--names".into());
            }
            for f in list("map") {
                args.extend(["--map".into(), f]);
            }
            if let Some(m) = str_of("mode") {
                args.extend(["--mode".into(), m]);
            }
            for f in list("match_fields") {
                args.extend(["--match".into(), f]);
            }
            if flag("add_remaining") == Some(true) {
                args.push("--add-remaining".into());
            }
            if flag("auto_enter") == Some(false) {
                args.push("--no-auto-enter".into());
            }
            "import"
        }
        "print_pdf" => {
            args.push(str_of("layout").ok_or("layout is required")?);
            args.extend(["-o".into(), str_of("path").ok_or("path is required")?]);
            args.extend(query);
            if let Some(p) = str_of("paper") {
                args.extend(["--paper".into(), p]);
            }
            if flag("landscape") == Some(true) {
                args.push("--landscape".into());
            }
            if let Some(m) = a.get("margin").and_then(Json::as_f64) {
                args.extend(["--margin".into(), m.to_string()]);
            }
            "print-pdf"
        }
        "evaluate" => {
            args.push(str_of("calculation").ok_or("calculation is required")?);
            if let Some(t) = str_of("table") {
                args.extend(["--table".into(), t]);
            }
            if let Some(r) = a.get("record_id").and_then(Json::as_u64) {
                args.extend(["--record".into(), r.to_string()]);
            }
            "eval"
        }
        "create_record" | "update_record" | "delete_record" => {
            args.push(str_of("table").ok_or("table is required")?);
            if name != "create_record" {
                let id = a
                    .get("record_id")
                    .and_then(Json::as_u64)
                    .ok_or("record_id is required")?;
                args.push(id.to_string());
            }
            if name != "delete_record" {
                args.extend(pairs("fields"));
            }
            match name {
                "create_record" => "new",
                "update_record" => "set",
                _ => "delete",
            }
        }
        "create_table" => {
            args.extend([
                "add-table".into(),
                str_of("name").ok_or("name is required")?,
            ]);
            "schema"
        }
        "create_field" => {
            args.push("add-field".into());
            args.push(str_of("table").ok_or("table is required")?);
            args.push(str_of("name").ok_or("name is required")?);
            for (k, opt) in [
                ("type", "--type"),
                ("comment", "--comment"),
                ("calculation", "--calc"),
                ("serial", "--serial"),
                ("stamp", "--stamp"),
                ("auto_calculation", "--auto-calc"),
                ("constant", "--constant"),
                ("validation_calculation", "--validate"),
                ("index", "--index"),
            ] {
                if let Some(v) = str_of(k) {
                    args.extend([opt.into(), v]);
                }
            }
            for (k, opt) in [
                ("replace_existing", "--replace"),
                ("prohibit_modification", "--prohibit-modification"),
                ("not_empty", "--not-empty"),
                ("unique", "--unique"),
                ("existing", "--existing"),
                ("always_validate", "--always-validate"),
                ("global", "--global"),
            ] {
                if flag(k) == Some(true) {
                    args.push(opt.into());
                }
            }
            if flag("stored") == Some(false) {
                args.push("--unstored".into());
            }
            if let Some(s) = a.get("summary") {
                let op = s
                    .get("operation")
                    .and_then(Json::as_str)
                    .ok_or("summary.operation is required")?;
                let f = s
                    .get("field")
                    .and_then(Json::as_str)
                    .ok_or("summary.field is required")?;
                args.extend(["--summary".into(), format!("{op}:{f}")]);
            }
            let r = list("range");
            if let [lo, hi] = r.as_slice() {
                args.extend(["--range".into(), format!("{lo}:{hi}")]);
            }
            if let Some(n) = a.get("repetitions").and_then(Json::as_u64) {
                args.extend(["--repetitions".into(), n.to_string()]);
            }
            "schema"
        }
        "list_scripts" => "scripts",
        "run_script" => {
            args.push(str_of("name").ok_or("name is required")?);
            if let Some(p) = str_of("parameter") {
                args.extend(["--param".into(), p]);
            }
            if let Some(l) = str_of("layout") {
                args.extend(["--layout".into(), l]);
            }
            if let Some(n) = a.get("answer").and_then(Json::as_u64) {
                args.extend(["--answer".into(), n.to_string()]);
            }
            "run-script"
        }
        "save_file" => {
            if let Some(p) = str_of("path") {
                args.extend(["-o".into(), p]);
            }
            "save"
        }
        _ => return Err(format!("unknown tool {name:?}")),
    };
    run(d, cmd, &opts(&args)?)
}

fn handle(doc: &mut Option<Doc>, msg: &Json) -> Option<Json> {
    let id = msg.get("id")?.clone(); // notifications get no reply
    let method = msg["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "basecraft", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "BaseCraft reads and writes FileMaker .fmp12 files. Call open_file first, then use the other tools. Writes stay in memory until save_file.",
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools().into_iter().map(|(n, d, s)|
            json!({ "name": n, "description": d, "inputSchema": s })).collect::<Vec<_>>() }),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
            match call(doc, name, &args) {
                Ok(v) => {
                    json!({ "content": [{ "type": "text", "text": v.to_string() }], "isError": false })
                }
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            }
        }
        _ => {
            return Some(json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("method not found: {method}") } }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Runs the server until stdin closes. `file` is opened at start if given.
pub fn serve(file: Option<&str>) -> Res<Json> {
    let mut doc = file.map(Doc::open).transpose()?;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Json>(&line) {
            Ok(msg) => handle(&mut doc, &msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") } })),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}").map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_file_a_table_and_fields() {
        let dir = std::env::temp_dir().join(format!("basecraft-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("new.fmp12").to_string_lossy().to_string();
        let mut doc = None;
        let mut call_ok = |name: &str, args: Json| {
            let r = handle(
                &mut doc,
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                        "params":{"name": name, "arguments": args}}),
            )
            .unwrap();
            assert_eq!(r["result"]["isError"], false, "{name}: {r}");
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        call_ok("new_file", json!({"path": path, "table": "Notes"}));
        call_ok("create_table", json!({"name": "People"}));
        call_ok(
            "create_field",
            json!({"table": "People", "name": "Id", "type": "number", "serial": "1", "unique": true}),
        );
        call_ok(
            "create_field",
            json!({"table": "People", "name": "Label", "type": "text", "calculation": "\"#\" & Id"}),
        );
        call_ok(
            "create_field",
            json!({"table": "People", "name": "Count", "summary": {"operation": "count", "field": "Id"}}),
        );
        call_ok("create_record", json!({"table": "People"}));
        call_ok("save_file", json!({}));
        let db = engine::Database::open(&path).unwrap();
        let t = db.tables.iter().find(|t| t.name == "People").unwrap();
        let label = t.field_by_name("Label").unwrap().id;
        assert_eq!(t.records[0].values[&label], "#1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lists_tools_and_requires_open_file() {
        let mut doc = None;
        let r = handle(
            &mut doc,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .unwrap();
        assert!(r["result"]["tools"].as_array().unwrap().len() >= 10);
        let r = handle(
            &mut doc,
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"describe_database"}}),
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(
            handle(
                &mut doc,
                &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .is_none()
        );
    }
}
