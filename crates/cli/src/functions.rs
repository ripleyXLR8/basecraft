//! `functions`: File › Manage › Custom Functions from the command line.
//!
//! ```text
//! functions <file>                                       list custom functions
//! functions add <file> <Name> --calc F [--params "a;b"] [--full-access-only]
//! functions set <file> <Name> [--name New] [--calc F] [--params "a;b"]
//!                             [--full-access-only | --all-accounts]
//! functions delete <file> <Name>
//! ```

use fmp12::custom_functions::Kind;
use fmp12::schema_edit::CustomFunctionSpec;
use serde_json::{Value as Json, json};

use crate::{Doc, Res};

/// Every custom function, with the folder it is in.
pub fn list(d: &Doc) -> Json {
    let cf = &d.db.custom_functions;
    let mut folder: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for f in &cf.list {
        match f.kind {
            Kind::Folder => folder.push(&f.name),
            Kind::FolderEnd => {
                folder.pop();
            }
            Kind::Function => out.push(json!({
                "id": f.id,
                "name": f.name,
                "parameters": f.params,
                "calculation": cf.formulas.get(&f.id),
                "availability": if f.full_access_only { "full access only" } else { "all accounts" },
                "folder": folder.last(),
            })),
        }
    }
    json!({ "custom_functions": out })
}

fn spec_of(d: &Doc, id: u32) -> Option<CustomFunctionSpec> {
    let cf = &d.db.custom_functions;
    let f = cf.list.iter().find(|f| f.id == id)?;
    Some(CustomFunctionSpec {
        name: f.name.clone(),
        params: f.params.clone(),
        formula: cf.formulas.get(&id).cloned().unwrap_or_default(),
        full_access_only: f.full_access_only,
    })
}

fn id_of(d: &Doc, name: &str) -> Res<u32> {
    d.db.custom_functions
        .list
        .iter()
        .find(|f| f.kind == Kind::Function && f.name.eq_ignore_ascii_case(name))
        .map(|f| f.id)
        .ok_or_else(|| format!("no custom function named {name:?}"))
}

/// `--opt value` arguments after the name.
fn value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

/// `--params "a;b"`: parameter names, as FileMaker lists them.
fn params(args: &[String]) -> Option<Vec<String>> {
    Some(
        value(args, "--params")?
            .split(';')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

pub fn run(d: &mut Doc, args: &[String]) -> Res<Json> {
    let e = |e: fmp12::schema_edit::EditError| e.to_string();
    let name = || {
        args.get(1)
            .cloned()
            .ok_or_else(|| "functions: missing function name".to_string())
    };
    let rest = args.get(2..).unwrap_or_default();
    match args.first().map(String::as_str) {
        None | Some("list") => Ok(list(d)),
        Some("add") => {
            let spec = CustomFunctionSpec {
                name: name()?,
                params: params(rest).unwrap_or_default(),
                formula: value(rest, "--calc")
                    .ok_or("functions add: missing --calc")?
                    .to_string(),
                full_access_only: rest.iter().any(|a| a == "--full-access-only"),
            };
            let id = d.db.add_custom_function(&mut d.file, &spec).map_err(e)?;
            Ok(json!({ "added_function": spec.name, "id": id }))
        }
        Some("set") => {
            let id = id_of(d, &name()?)?;
            let mut spec = spec_of(d, id).ok_or("custom function not found")?;
            if let Some(n) = value(rest, "--name") {
                spec.name = n.to_string();
            }
            if let Some(c) = value(rest, "--calc") {
                spec.formula = c.to_string();
            }
            if let Some(p) = params(rest) {
                spec.params = p;
            }
            if rest.iter().any(|a| a == "--full-access-only") {
                spec.full_access_only = true;
            }
            if rest.iter().any(|a| a == "--all-accounts") {
                spec.full_access_only = false;
            }
            d.db.set_custom_function(&mut d.file, id, &spec)
                .map_err(e)?;
            Ok(json!({ "changed_function": spec.name, "id": id }))
        }
        Some("delete") => {
            let n = name()?;
            let id = id_of(d, &n)?;
            d.db.delete_custom_function(&mut d.file, id).map_err(e)?;
            Ok(json!({ "deleted_function": n }))
        }
        Some(s) => Err(format!("functions: unknown sub-command {s:?}")),
    }
}
