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
            "Open a FileMaker .fmp12 file. Other tools act on the open file. A protected file (one that does not log in by itself) needs `account` and `password`; privileges of that account then apply.",
            s(
                json!({ "path": { "type": "string" }, "account": { "type": "string" }, "password": { "type": "string" } }),
                &["path"],
            ),
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
            "list_security",
            "Accounts and privilege sets of the open file (File › Manage › Security): record, field, layout, script and value list access, other privileges, extended privileges. Needs full access.",
            s(json!({}), &[]),
        ),
        (
            "add_account",
            "Add a FileMaker account with a password and a privilege set (default [Data Entry Only]). Needs full access. Changes stay in memory until save_file.",
            s(
                json!({ "name": { "type": "string" }, "password": { "type": "string" },
                   "privilege_set": { "type": "string" }, "active": { "type": "boolean" },
                   "must_change_password": { "type": "boolean" }, "description": { "type": "string" } }),
                &["name", "password"],
            ),
        ),
        (
            "update_account",
            "Change an account: rename, new password, privilege set, active, must change password at next login, description. Needs full access. Changes stay in memory until save_file.",
            s(
                json!({ "name": { "type": "string" }, "rename": { "type": "string" }, "password": { "type": "string" },
                   "privilege_set": { "type": "string" }, "active": { "type": "boolean" },
                   "must_change_password": { "type": "boolean" }, "description": { "type": "string" } }),
                &["name"],
            ),
        ),
        (
            "delete_account",
            "Delete an account. Needs full access; at least one active [Full Access] account stays. Changes stay in memory until save_file.",
            s(json!({ "name": { "type": "string" } }), &["name"]),
        ),
        (
            "set_privileges",
            "Create (`create`: true, optionally `copy_of` a set) or change a privilege set. `records`: all, view, none or a list like \"create,view,edit\"; `tables`: {Table: actions}; `limits`: [\"Table:edit=calculation\"]; `fields`: {\"Table::Field\": modifiable|view|none}; `layouts`/`scripts`/`value_lists`: default access (modifiable, view or executable, none); `layout_access`/`script_access`: {name: access}; `menus`: all, editing, minimum; `allow`/`deny`: print, export, change-password, manage-extended, override-validation, create-layouts, create-scripts, create-value-lists; `extended`: keywords such as fmapp. Needs full access. Changes stay in memory until save_file.",
            s(
                json!({ "name": { "type": "string" }, "create": { "type": "boolean" }, "copy_of": { "type": "string" },
                   "rename": { "type": "string" }, "description": { "type": "string" },
                   "records": { "type": "string" },
                   "tables": { "type": "object", "additionalProperties": { "type": "string" } },
                   "limits": { "type": "array", "items": { "type": "string" } },
                   "fields": { "type": "object", "additionalProperties": { "type": "string" } },
                   "layouts": { "type": "string" }, "scripts": { "type": "string" }, "value_lists": { "type": "string" },
                   "layout_access": { "type": "object", "additionalProperties": { "type": "string" } },
                   "script_access": { "type": "object", "additionalProperties": { "type": "string" } },
                   "menus": { "type": "string", "enum": ["all", "editing", "minimum"] },
                   "allow": { "type": "array", "items": { "type": "string" } },
                   "deny": { "type": "array", "items": { "type": "string" } },
                   "extended": { "type": "array", "items": { "type": "string" } },
                   "min_password": { "type": "integer" }, "expire_days": { "type": "integer" } }),
                &["name"],
            ),
        ),
        (
            "delete_privilege_set",
            "Delete a privilege set no account uses (not a built-in one). Needs full access. Changes stay in memory until save_file.",
            s(json!({ "name": { "type": "string" } }), &["name"]),
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
            "Add a field to a table (Manage Database › Fields): a normal field of `type`, a calculation (`calculation`, `stored`) or a summary (`summary`: operation, field, running, by = restart/weight/subtotal field). Options: auto-enter (serial next value and increment, stamp, auto_calculation, constant, lookup from Occurrence::Field through a relationship), validation (not_empty, unique, existing, range [low, high], value_list, max_chars, validation_calculation, message, always_validate), storage (global, repetitions, index). Calculations use FileMaker syntax; fields of other tables are written Occurrence::Field. Changes stay in memory until save_file.",
            s(
                json!({ "table": table, "name": { "type": "string" },
                   "type": { "type": "string", "enum": ["text", "number", "date", "time", "timestamp", "container"] },
                   "comment": { "type": "string" },
                   "calculation": { "type": "string" }, "stored": { "type": "boolean" },
                   "summary": { "type": "object", "properties": {
                       "operation": { "type": "string", "enum": ["total", "average", "count", "minimum", "maximum", "stdev", "fraction", "list"] },
                       "field": { "type": "string" },
                       "running": { "type": "boolean", "description": "Running total or count; by population for a standard deviation" },
                       "by": { "type": "string", "description": "Field a running summary restarts on, the weight of an average, or the field a fraction is subtotaled by" } } },
                   "serial": { "type": "string", "description": "Next serial value" },
                   "serial_increment": { "type": "integer" },
                   "lookup": { "type": "object", "description": "Copy a related field when the match field changes",
                       "properties": { "field": { "type": "string", "description": "Occurrence::Field to copy" },
                       "start": { "type": "string", "description": "Occurrence the lookup starts from (default: the table's first)" },
                       "if_no_match": { "type": "string", "description": "none, lower, higher or value:TEXT" },
                       "copy_empty": { "type": "boolean" } }, "required": ["field"] },
                   "value_list": { "type": "string", "description": "Validate: member of this value list" },
                   "max_chars": { "type": "integer" },
                   "message": { "type": "string", "description": "Text shown when validation fails" },
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
            "create_occurrence",
            "Add a table occurrence of a table to the relationship graph (Manage Database › Relationships), under its own name. Changes stay in memory until save_file.",
            s(
                json!({ "table": { "type": "string" }, "name": { "type": "string" } }),
                &["table", "name"],
            ),
        ),
        (
            "create_relationship",
            "Relate two table occurrences (Manage Database › Relationships). `predicates`: \"LeftField op RightField\" with op =, ≠, <, ≤, >, ≥ or × (or !=, <=, >=, x). Options per side: allow creation of records, delete them with the other side's record, sort them (\"Field\" or \"Field:desc\"). The graph has no cycles: relate a new occurrence instead. Changes stay in memory until save_file.",
            s(
                json!({ "left": { "type": "string" }, "right": { "type": "string" },
                   "predicates": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                   "create_left": { "type": "boolean" }, "create_right": { "type": "boolean" },
                   "delete_left": { "type": "boolean" }, "delete_right": { "type": "boolean" },
                   "sort_left": { "type": "array", "items": { "type": "string" } },
                   "sort_right": { "type": "array", "items": { "type": "string" } } }),
                &["left", "right", "predicates"],
            ),
        ),
        (
            "create_valuelist",
            "Add a value list (File › Manage › Value Lists): custom `values`, or the values of `field` (Occurrence::Field), optionally with a `second_field` shown next to it (`second_only`, `sort_by_second`) and only values related from `related_from`. Changes stay in memory until save_file.",
            s(
                json!({ "name": { "type": "string" },
                   "values": { "type": "array", "items": { "type": "string" } },
                   "field": { "type": "string" }, "second_field": { "type": "string" },
                   "second_only": { "type": "boolean" }, "sort_by_second": { "type": "boolean" },
                   "related_from": { "type": "string" } }),
                &["name"],
            ),
        ),
        (
            "save_file",
            "Write the open file with all changes. Give `path` to save a copy; omit it to overwrite the opened file.",
            s(json!({ "path": { "type": "string" } }), &[]),
        ),
        (
            "remote_command",
            "Run a command on a file hosted by FileMaker Server (Data API), without opening a local file: info, records, find, new, set, delete or run-script, with the command line's arguments (e.g. [\"Contacts\", \"--find\", \"City=Paris\"], [\"Contacts\", \"12\", \"City=Rome\", \"--mod-id\", \"3\"], [\"Echo\", \"--layout\", \"Contacts\", \"--param\", \"x\"]). The password is read from the environment variable named by `password_env`, never passed here. Writes go to the server at once.",
            s(
                json!({ "url": { "type": "string", "description": "https://host/Database" },
                        "account": { "type": "string" },
                        "password_env": { "type": "string", "description": "Name of the environment variable holding the password" },
                        "insecure": { "type": "boolean", "description": "Accept a self-signed certificate (test servers only)" },
                        "command": { "type": "string", "enum": ["info", "records", "find", "new", "set", "delete", "run-script"] },
                        "args": { "type": "array", "items": { "type": "string" } } }),
                &["url", "account", "password_env", "command"],
            ),
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
        *doc = Some(Doc::open(&path, None)?);
        return Ok(r);
    }
    if name == "remote_command" {
        let mut args: Vec<String> = vec![
            "--remote".into(),
            str_of("url").ok_or("url is required")?,
            "--password-env".into(),
            str_of("password_env").ok_or("password_env is required")?,
        ];
        if a.get("insecure").and_then(Json::as_bool) == Some(true) {
            args.push("--insecure".into());
        }
        let login = crate::Login {
            account: str_of("account").ok_or("account is required")?,
            password: String::new(),
        };
        let c = crate::hosted::conn(&mut args, Some(&login))?.ok_or("url is required")?;
        let rest: Vec<String> = a
            .get("args")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
            .collect();
        let cmd = str_of("command").ok_or("command is required")?;
        return crate::hosted::run(&c, &cmd, &rest);
    }
    if name == "open_file" {
        let path = str_of("path").ok_or("path is required")?;
        let login = str_of("account").map(|account| crate::Login {
            account,
            password: str_of("password").unwrap_or_default(),
        });
        let d = Doc::open(&path, login.as_ref())?;
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
                ("value_list", "--in-list"),
                ("message", "--message"),
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
            for (k, opt) in [
                ("repetitions", "--repetitions"),
                ("serial_increment", "--increment"),
                ("max_chars", "--max-chars"),
            ] {
                if let Some(n) = a.get(k).and_then(Json::as_u64) {
                    args.extend([opt.into(), n.to_string()]);
                }
            }
            if let Some(s) = a.get("summary") {
                if s.get("running").and_then(Json::as_bool) == Some(true) {
                    args.push("--running".into());
                }
                if let Some(by) = s.get("by").and_then(Json::as_str) {
                    args.extend(["--subtotal-by".into(), by.to_string()]);
                }
            }
            if let Some(l) = a.get("lookup") {
                let get = |k: &str| l.get(k).and_then(Json::as_str).map(str::to_string);
                args.extend([
                    "--lookup".into(),
                    get("field").ok_or("lookup.field is required")?,
                ]);
                if let Some(v) = get("start") {
                    args.extend(["--lookup-from".into(), v]);
                }
                if let Some(v) = get("if_no_match") {
                    args.extend(["--if-no-match".into(), v]);
                }
                if l.get("copy_empty").and_then(Json::as_bool) == Some(true) {
                    args.push("--copy-empty".into());
                }
            }
            "schema"
        }
        "create_occurrence" => {
            args.push("add-occurrence".into());
            args.push(str_of("table").ok_or("table is required")?);
            args.push(str_of("name").ok_or("name is required")?);
            "schema"
        }
        "create_relationship" => {
            args.push("add-relationship".into());
            args.push(str_of("left").ok_or("left is required")?);
            args.push(str_of("right").ok_or("right is required")?);
            let preds = list("predicates");
            if preds.is_empty() {
                return Err("predicates are required".into());
            }
            args.extend(preds);
            for k in ["create_left", "create_right", "delete_left", "delete_right"] {
                if flag(k) == Some(true) {
                    args.push(format!("--{}", k.replace('_', "-")));
                }
            }
            for k in ["sort_left", "sort_right"] {
                for f in list(k) {
                    args.extend([format!("--{}", k.replace('_', "-")), f]);
                }
            }
            "schema"
        }
        "create_valuelist" => {
            args.push("add-valuelist".into());
            args.push(str_of("name").ok_or("name is required")?);
            args.extend(list("values"));
            for (k, opt) in [
                ("field", "--from-field"),
                ("second_field", "--second"),
                ("related_from", "--related-from"),
            ] {
                if let Some(v) = str_of(k) {
                    args.extend([opt.into(), v]);
                }
            }
            for (k, opt) in [
                ("second_only", "--second-only"),
                ("sort_by_second", "--sort-second"),
            ] {
                if flag(k) == Some(true) {
                    args.push(opt.into());
                }
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
        "list_security" => {
            let mut a = crate::security::accounts(d, &[])?;
            a["privilege_sets"] = crate::security::privileges(d, &[])?["privilege_sets"].take();
            return Ok(a);
        }
        "add_account" | "update_account" | "delete_account" => {
            args.push(
                match name {
                    "add_account" => "add",
                    "update_account" => "set",
                    _ => "delete",
                }
                .into(),
            );
            args.push(str_of("name").ok_or("name is required")?);
            if let Some(p) = str_of("password") {
                args.extend(["--new-password".into(), p]);
            }
            if let Some(v) = str_of("privilege_set") {
                args.extend(["--set".into(), v]);
            }
            if let Some(v) = str_of("rename") {
                args.extend(["--rename".into(), v]);
            }
            if let Some(v) = str_of("description") {
                args.extend(["--description".into(), v]);
            }
            match flag("active") {
                Some(true) => args.push("--active".into()),
                Some(false) => args.push("--inactive".into()),
                None => {}
            }
            match flag("must_change_password") {
                Some(true) => args.push("--must-change".into()),
                Some(false) => args.push("--no-must-change".into()),
                None => {}
            }
            return crate::security::accounts(d, &args);
        }
        "set_privileges" | "delete_privilege_set" => {
            let sub = match (name, flag("create")) {
                ("delete_privilege_set", _) => "delete",
                (_, Some(true)) => "add",
                _ => "set",
            };
            args.push(sub.into());
            args.push(str_of("name").ok_or("name is required")?);
            for (k, opt) in [
                ("copy_of", "--copy-of"),
                ("rename", "--rename"),
                ("description", "--description"),
                ("records", "--records"),
                ("layouts", "--layouts"),
                ("scripts", "--scripts"),
                ("value_lists", "--value-lists"),
                ("menus", "--menus"),
            ] {
                if let Some(v) = str_of(k) {
                    args.extend([opt.into(), v]);
                }
            }
            for (k, opt) in [
                ("tables", "--table-access"),
                ("fields", "--field-access"),
                ("layout_access", "--layout-access"),
                ("script_access", "--script-access"),
            ] {
                for p in pairs(k) {
                    args.extend([opt.into(), p]);
                }
            }
            for (k, opt) in [
                ("limits", "--limit"),
                ("allow", "--allow"),
                ("deny", "--deny"),
            ] {
                for v in list(k) {
                    args.extend([opt.into(), v]);
                }
            }
            if a.get("extended").is_some() {
                args.extend(["--extended".into(), list("extended").join(",")]);
            }
            for (k, opt) in [
                ("min_password", "--min-password"),
                ("expire_days", "--expire-days"),
            ] {
                if let Some(n) = a.get(k).and_then(Json::as_u64) {
                    args.extend([opt.into(), n.to_string()]);
                }
            }
            return crate::security::privileges(d, &args);
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
pub fn serve(file: Option<&str>, login: Option<&crate::Login>) -> Res<Json> {
    let mut doc = file.map(|f| Doc::open(f, login)).transpose()?;
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
    fn creates_relationships_lookups_and_value_lists() {
        let dir = std::env::temp_dir().join(format!("basecraft-mcp-rel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shop.fmp12").to_string_lossy().to_string();
        let mut doc = None;
        let call = |doc: &mut Option<Doc>, name: &str, args: Json| {
            let r = handle(
                doc,
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                        "params":{"name": name, "arguments": args}}),
            )
            .unwrap();
            let text = r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string();
            (r["result"]["isError"] == false, text)
        };
        let ok = |doc: &mut Option<Doc>, name: &str, args: Json| {
            let (good, text) = call(doc, name, args);
            assert!(good, "{name}: {text}");
            text
        };
        ok(
            &mut doc,
            "new_file",
            json!({"path": path, "table": "Clients"}),
        );
        for (t, f, ty) in [("Clients", "Id", "number"), ("Clients", "Name", "text")] {
            ok(
                &mut doc,
                "create_field",
                json!({"table": t, "name": f, "type": ty}),
            );
        }
        ok(&mut doc, "create_table", json!({"name": "Invoices"}));
        for f in ["ClientId", "Amount"] {
            ok(
                &mut doc,
                "create_field",
                json!({"table": "Invoices", "name": f, "type": "number"}),
            );
        }
        let rel = ok(
            &mut doc,
            "create_relationship",
            json!({"left": "Clients", "right": "Invoices", "predicates": ["Id = ClientId"],
                   "create_right": true, "delete_right": true, "sort_right": ["Amount:desc"]}),
        );
        assert!(rel.contains("Id = ClientId"), "{rel}");
        ok(
            &mut doc,
            "create_occurrence",
            json!({"table": "Invoices", "name": "Big invoices"}),
        );
        ok(
            &mut doc,
            "create_relationship",
            json!({"left": "Clients", "right": "Big invoices", "predicates": ["Id = ClientId", "Id < Amount"]}),
        );
        ok(
            &mut doc,
            "create_field",
            json!({"table": "Invoices", "name": "ClientName", "type": "text",
                   "lookup": {"field": "Clients::Name", "if_no_match": "value:?"}}),
        );
        ok(
            &mut doc,
            "create_field",
            json!({"table": "Invoices", "name": "Running", "type": "number",
                   "summary": {"operation": "total", "field": "Amount", "running": true, "by": "ClientId"}}),
        );
        let vl = ok(
            &mut doc,
            "create_valuelist",
            json!({"name": "Cities", "values": ["Paris", "Lyon"]}),
        );
        assert!(vl.contains("Lyon"), "{vl}");
        ok(
            &mut doc,
            "create_valuelist",
            json!({"name": "Names", "field": "Clients::Id", "second_field": "Clients::Name", "second_only": true}),
        );
        ok(
            &mut doc,
            "create_field",
            json!({"table": "Clients", "name": "City", "type": "text",
                   "value_list": "Cities", "max_chars": 10, "message": "Pick a city"}),
        );
        // Errors come back as tool errors.
        let (good, text) = call(
            &mut doc,
            "create_relationship",
            json!({"left": "Clients", "right": "Invoices", "predicates": ["Id = ClientId"]}),
        );
        assert!(!good, "{text}");
        ok(
            &mut doc,
            "create_record",
            json!({"table": "Clients", "fields": {"Id": "1", "Name": "Ada"}}),
        );
        let inv = ok(
            &mut doc,
            "create_record",
            json!({"table": "Invoices", "fields": {"ClientId": "1", "Amount": "5"}}),
        );
        assert!(inv.contains("\"ClientName\":\"Ada\""), "{inv}");
        let n = ok(
            &mut doc,
            "evaluate",
            json!({"calculation": "Count ( Invoices::Amount ) & \"/\" & Count ( ${Big invoices}::Amount )", "table": "Clients", "record_id": 1}),
        );
        assert!(n.contains("1/1"), "{n}");
        ok(&mut doc, "save_file", json!({}));
        let db = engine::Database::open(&path).unwrap();
        assert_eq!(db.relationships.len(), 2);
        assert_eq!(db.value_lists.len(), 2);
        let t = db.tables.iter().find(|t| t.name == "Invoices").unwrap();
        let f = t.field_by_name("ClientName").unwrap();
        assert!(f.def.lookup().is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Accounts and privilege sets through MCP: a protected file opens only
    /// with the right password, and the account's privileges apply.
    #[test]
    fn accounts_and_privileges() {
        let dir = std::env::temp_dir().join(format!("basecraft-mcp-sec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sec.fmp12").to_string_lossy().to_string();
        let mut doc = None;
        let call = |doc: &mut Option<Doc>, name: &str, args: Json| {
            let r = handle(
                doc,
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                        "params":{"name": name, "arguments": args}}),
            )
            .unwrap();
            let text = r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string();
            (r["result"]["isError"] == true, text)
        };
        let ok = |doc: &mut Option<Doc>, name: &str, args: Json| {
            let (err, text) = call(doc, name, args);
            assert!(!err, "{name}: {text}");
            text
        };
        ok(
            &mut doc,
            "new_file",
            json!({"path": path, "table": "Notes"}),
        );
        ok(&mut doc, "create_record", json!({"table": "Notes"}));
        ok(
            &mut doc,
            "set_privileges",
            json!({"name": "Readers", "create": true, "records": "view", "deny": ["export"]}),
        );
        ok(
            &mut doc,
            "add_account",
            json!({"name": "reader", "password": "test123", "privilege_set": "Readers"}),
        );
        ok(
            &mut doc,
            "update_account",
            json!({"name": "Admin", "password": "test123"}),
        );
        ok(&mut doc, "save_file", json!({}));

        // No credentials, then a wrong password: refused.
        let (err, text) = call(&mut doc, "open_file", json!({"path": path}));
        assert!(err && text.contains("password"), "{text}");
        let (err, text) = call(
            &mut doc,
            "open_file",
            json!({"path": path, "account": "reader", "password": "nope"}),
        );
        assert!(err && text.contains("do not match"), "{text}");

        // The reader sees records but cannot change them, export, or manage security.
        ok(
            &mut doc,
            "open_file",
            json!({"path": path, "account": "READER", "password": "test123"}),
        );
        let (err, text) = call(&mut doc, "create_record", json!({"table": "Notes"}));
        assert!(err && text.contains("200"), "{text}");
        let (err, _) = call(
            &mut doc,
            "export_records",
            json!({"table": "Notes", "format": "csv"}),
        );
        assert!(err);
        let (err, _) = call(&mut doc, "list_security", json!({}));
        assert!(err);
        let (err, _) = call(
            &mut doc,
            "add_account",
            json!({"name": "x", "password": "test123"}),
        );
        assert!(err);
        let who = ok(
            &mut doc,
            "evaluate",
            json!({"table": "Notes", "calculation": "Get ( AccountName ) & \"|\" & Get ( AccountPrivilegeSetName )"}),
        );
        assert!(who.contains("reader|Readers"), "{who}");

        // Admin lists both and may delete the reader.
        ok(
            &mut doc,
            "open_file",
            json!({"path": path, "account": "Admin", "password": "test123"}),
        );
        let list = ok(&mut doc, "list_security", json!({}));
        assert!(
            list.contains("reader") && list.contains("Readers"),
            "{list}"
        );
        ok(&mut doc, "delete_account", json!({"name": "reader"}));
        ok(&mut doc, "save_file", json!({}));
        let f = fmp12::File::open(&path).unwrap();
        assert!(fmp12::security::Security::check(&f).unwrap().is_empty());
        let sec = fmp12::security::Security::load(&f).unwrap();
        assert!(sec.account_named("reader").is_none());
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
