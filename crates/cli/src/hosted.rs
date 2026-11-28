//! Files hosted by FileMaker Server, through its Data API:
//!
//! ```text
//! basecraft-cli <command> --remote https://host/Database --account A
//!               (--password-env VAR | --password-stdin) [--insecure] …
//! info                                     layouts (with their fields) and scripts
//! records    <layout> [--find F=v]… [--omit F=v]… [--sort F[:desc]]… [--limit N] [--offset N] [--portal P]…
//! find       <layout> --find F=v… [--omit F=v]…   (same options)
//! new        <layout> F=v…
//! set        <layout> <recordId> F=v… [--mod-id N]
//! delete     <layout> <recordId>
//! run-script <script> --layout L [--param P]
//! ```
//!
//! Writes go straight to the server (no `-o`/`--in-place`). The password
//! comes from an environment variable or the first line of stdin (or
//! `BASECRAFT_PASSWORD`); it is never printed. `--insecure` accepts a
//! self-signed certificate (test servers only).

use remote::{Client, FindRequest, Options, Query, RemoteRecord, Target};
use serde_json::{Value as Json, json};

use crate::{Login, Res, opts, pairs};

/// The connection options taken out of the command line.
pub struct Conn {
    target: Target,
    account: String,
    password: String,
    options: Options,
}

fn take(args: &mut Vec<String>, name: &str) -> Res<Option<String>> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    if i + 1 >= args.len() {
        return Err(format!("{name} needs a value"));
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Ok(Some(v))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    let n = args.len();
    args.retain(|a| a != name);
    args.len() != n
}

/// `--remote URL` and its login, if given (`login` from `--account`).
pub fn conn(args: &mut Vec<String>, login: Option<&Login>) -> Res<Option<Conn>> {
    let Some(url) = take(args, "--remote")? else {
        return Ok(None);
    };
    let target = Target::parse(&url).map_err(|e| e.to_string())?;
    let insecure = take_flag(args, "--insecure");
    let account = login
        .map(|l| l.account.clone())
        .ok_or("--remote needs --account NAME")?;
    let password = if let Some(var) = take(args, "--password-env")? {
        std::env::var(&var).map_err(|_| format!("environment variable {var} is not set"))?
    } else if take_flag(args, "--password-stdin") {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        line.trim_end_matches(['\r', '\n']).to_string()
    } else {
        login.map(|l| l.password.clone()).unwrap_or_default()
    };
    if insecure {
        eprintln!("warning: {}", remote::INSECURE_WARNING);
    }
    Ok(Some(Conn {
        target,
        account,
        password,
        options: Options { insecure },
    }))
}

fn record_json(r: &RemoteRecord) -> Json {
    let mut o = serde_json::Map::new();
    o.insert("recordId".into(), json!(r.id));
    o.insert("modId".into(), json!(r.mod_id));
    for (k, v) in &r.fields {
        o.insert(k.clone(), json!(v));
    }
    if !r.portals.is_empty() {
        o.insert(
            "portals".into(),
            r.portals
                .iter()
                .map(|(k, rows)| (k.clone(), rows.iter().map(record_json).collect()))
                .collect::<serde_json::Map<_, _>>()
                .into(),
        );
    }
    Json::Object(o)
}

/// Runs `cmd` on the hosted file; `args` are the command's own arguments.
pub fn run(c: &Conn, cmd: &str, args: &[String]) -> Res<Json> {
    let mut args = args.to_vec();
    let offset = take(&mut args, "--offset")?
        .map(|v| v.parse().map_err(|_| "bad --offset"))
        .transpose()?;
    let mod_id = take(&mut args, "--mod-id")?
        .map(|v| v.parse().map_err(|_| "bad --mod-id"))
        .transpose()?;
    let mut portals = Vec::new();
    while let Some(p) = take(&mut args, "--portal")? {
        portals.push(p);
    }
    let o = opts(&args)?;
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
    let err = |e: remote::Error| e.to_string();
    let client = Client::login(&c.target, &c.account, &c.password, c.options).map_err(err)?;
    let q = Query {
        offset: offset.unwrap_or(0),
        limit: o.query.limit,
        sort: o
            .query
            .sort
            .iter()
            .map(|(f, ord)| (f.clone(), *ord == engine::Order::Descending))
            .collect(),
        portals,
    };
    let page_json = |layout: &str, p: remote::Page| {
        json!({ "layout": layout, "found": p.found, "total": p.total,
                "records": p.records.iter().map(record_json).collect::<Vec<_>>() })
    };
    let result = match cmd {
        "info" => {
            let layouts: Vec<Json> = client
                .layouts()
                .map_err(err)?
                .into_iter()
                .map(|l| {
                    let fields = client.layout(&l).map(|m| {
                        m.fields
                            .iter()
                            .map(|f| json!({ "name": f.name, "type": f.kind, "result": f.result }))
                            .collect::<Vec<_>>()
                    });
                    match fields {
                        Ok(f) => json!({ "name": l, "fields": f }),
                        Err(e) => json!({ "name": l, "error": e.to_string() }),
                    }
                })
                .collect();
            json!({ "database": c.target.database, "host": c.target.host,
                    "layouts": layouts, "scripts": client.scripts().map_err(err)? })
        }
        "records" | "find" => {
            let layout = arg(0, "layout")?;
            let mut reqs = Vec::new();
            if !o.query.find.is_empty() {
                reqs.push(FindRequest {
                    criteria: o.query.find.clone(),
                    omit: false,
                });
            }
            if !o.query.omit.is_empty() {
                if reqs.is_empty() {
                    // FileMaker needs a find request before an omit one.
                    reqs.push(FindRequest {
                        criteria: vec![(o.query.omit[0].0.clone(), "*".into())],
                        omit: false,
                    });
                }
                reqs.push(FindRequest {
                    criteria: o.query.omit.clone(),
                    omit: true,
                });
            }
            let page = if reqs.is_empty() {
                if cmd == "find" {
                    return Err("find: give --find Field=criterion".into());
                }
                client.records(&layout, &q)
            } else {
                client.find(&layout, &reqs, &q)
            }
            .map_err(err)?;
            page_json(&layout, page)
        }
        "new" => {
            let layout = arg(0, "layout")?;
            let (id, m) = client
                .create(&layout, &pairs(o.rest.get(1..).unwrap_or(&[]))?)
                .map_err(err)?;
            json!({ "created": id, "modId": m })
        }
        "set" => {
            let layout = arg(0, "layout")?;
            let id = id(1)?;
            let m = client
                .edit(&layout, id, &pairs(o.rest.get(2..).unwrap_or(&[]))?, mod_id)
                .map_err(err)?;
            json!({ "updated": id, "modId": m })
        }
        "delete" => {
            let id = id(1)?;
            client.delete(&arg(0, "layout")?, id).map_err(err)?;
            json!({ "deleted": id })
        }
        "run-script" => {
            let layout = o.layout.clone().ok_or("run-script: give --layout L")?;
            let r = client
                .run_script(&layout, &arg(0, "script name")?, o.param.as_deref())
                .map_err(err)?;
            json!({ "script_error": r.error, "result": r.result })
        }
        _ => {
            return Err(format!(
                "{cmd}: not available with --remote (info, records, find, new, set, delete, run-script)"
            ));
        }
    };
    client.logout().map_err(err)?;
    Ok(result)
}
