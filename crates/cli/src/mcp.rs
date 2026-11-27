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
            "save_file",
            "Write the open file with all changes. Give `path` to save a copy; omit it to overwrite the opened file.",
            s(json!({ "path": { "type": "string" } }), &[]),
        ),
    ]
}

/// Turns tool arguments into CLI-style arguments and runs the command.
fn call(doc: &mut Option<Doc>, name: &str, a: &Json) -> Res<Json> {
    let str_of = |k: &str| a.get(k).and_then(Json::as_str).map(str::to_string);
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
