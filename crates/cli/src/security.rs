//! `accounts` and `privileges`: File › Manage › Security from the command
//! line. Changes need a login with full access (`--account`/`--password`,
//! or the file's own "Log in using").
//!
//! ```text
//! accounts   <file>                                   list accounts
//! accounts   add <file> <name> --new-password P [--set S] [--inactive] [--must-change] [--description D]
//! accounts   set <file> <name> [--rename N] [--new-password P] [--set S] [--active|--inactive]
//!                              [--must-change|--no-must-change] [--description D]
//! accounts   delete <file> <name>
//! privileges <file>                                   list privilege sets
//! privileges add <file> <name> [--copy-of S] [options]
//! privileges set <file> <name> [--rename N] [options]
//! privileges delete <file> <name>
//!   options: --records all|view|none|create,view,edit,delete
//!            --table-access T=create,view,edit,delete   --limit "T:edit=calculation"
//!            --field-access T::F=modifiable|view|none
//!            --layouts modifiable|view|none   --layout-access L=modifiable|view|none
//!            --scripts modifiable|executable|none   --script-access S=…
//!            --value-lists modifiable|view|none   --menus all|editing|minimum
//!            --allow P / --deny P (print, export, change-password, manage-extended,
//!              override-validation, create-layouts, create-scripts, create-value-lists,
//!              manage-accounts, manage-database, manage-menus, open-quickly)
//!            --extended keyword,keyword   --min-password N   --expire-days N
//!            --description D
//! ```

use fmp12::security::{
    self, Account, Level, Menus, PrivilegeSet, RecordAccess, Security, TableAccess, flag,
};
use serde_json::{Value as Json, json};

use crate::{Doc, Res};

/// `--opt value` pairs and bare flags, in order.
struct Args<'a>(&'a [String]);

impl Args<'_> {
    fn value(&self, name: &str) -> Option<&str> {
        let i = self.0.iter().position(|a| a == name)?;
        self.0.get(i + 1).map(String::as_str)
    }
    fn values(&self, name: &str) -> Vec<&str> {
        self.0
            .windows(2)
            .filter(|w| w[0] == name)
            .map(|w| w[1].as_str())
            .collect()
    }
    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|a| a == name)
    }
}

fn level(s: &str) -> Res<u8> {
    Ok(match s {
        "modifiable" | "all" => security::MODIFIABLE,
        "view" | "view-only" | "executable" => security::VIEW_ONLY,
        "none" | "no" => security::NO_ACCESS,
        _ => {
            return Err(format!(
                "unknown access {s:?} (modifiable, view, executable, none)"
            ));
        }
    })
}

fn level_name(v: u8, script: bool) -> &'static str {
    match v {
        security::MODIFIABLE => "modifiable",
        security::VIEW_ONLY if script => "executable",
        security::VIEW_ONLY => "view",
        _ => "none",
    }
}

fn record_access(s: &str) -> Res<RecordAccess> {
    Ok(match s {
        "all" => RecordAccess::ALL,
        "view" => RecordAccess::VIEW,
        "none" => RecordAccess::NONE,
        list => {
            let mut a = RecordAccess {
                fields: security::MODIFIABLE,
                ..RecordAccess::NONE
            };
            for p in list.split(',').filter(|p| !p.is_empty()) {
                match p.trim() {
                    "create" => a.create = Level::Yes,
                    "view" => a.view = Level::Yes,
                    "edit" => a.edit = Level::Yes,
                    "delete" => a.delete = Level::Yes,
                    o => return Err(format!("unknown record action {o:?}")),
                }
            }
            a
        }
    })
}

fn flag_named(n: &str) -> Res<u16> {
    Ok(match n {
        "print" => flag::PRINT,
        "export" => flag::EXPORT,
        "change-password" => flag::CHANGE_PASSWORD,
        "manage-extended" => flag::MANAGE_EXTENDED,
        "override-validation" => flag::OVERRIDE_VALIDATION,
        "create-layouts" => flag::CREATE_LAYOUTS,
        "create-scripts" => flag::CREATE_SCRIPTS,
        "create-value-lists" => flag::CREATE_VALUE_LISTS,
        "manage-accounts" => flag::MANAGE_ACCOUNTS,
        "manage-database" => flag::MANAGE_DATABASE,
        "manage-menus" => flag::MANAGE_MENUS,
        "open-quickly" => flag::OPEN_QUICKLY,
        _ => return Err(format!("unknown privilege {n:?}")),
    })
}

fn actions(a: &RecordAccess) -> String {
    let lv = |l: Level| match l {
        Level::Yes => "yes",
        Level::No => "no",
        Level::Limited => "limited",
    };
    format!(
        "create {}, view {}, edit {}, delete {}",
        lv(a.create),
        lv(a.view),
        lv(a.edit),
        lv(a.delete)
    )
}

pub fn set_json(d: &Doc, sec: &Security, s: &PrivilegeSet) -> Json {
    let db = &d.db;
    let flags: Vec<&str> = [
        ("print", flag::PRINT),
        ("export", flag::EXPORT),
        ("change-password", flag::CHANGE_PASSWORD),
        ("manage-extended", flag::MANAGE_EXTENDED),
        ("override-validation", flag::OVERRIDE_VALIDATION),
        ("create-layouts", flag::CREATE_LAYOUTS),
        ("create-scripts", flag::CREATE_SCRIPTS),
        ("create-value-lists", flag::CREATE_VALUE_LISTS),
        ("manage-accounts", flag::MANAGE_ACCOUNTS),
        ("manage-database", flag::MANAGE_DATABASE),
        ("manage-menus", flag::MANAGE_MENUS),
        ("open-quickly", flag::OPEN_QUICKLY),
    ]
    .into_iter()
    .filter(|(_, f)| s.allows(*f))
    .map(|(n, _)| n)
    .collect();
    let names = fmp12::calc::FileNames {
        tables: db
            .tables
            .iter()
            .map(|t| fmp12::schema::Table {
                id: t.id,
                name: t.name.clone(),
                fields: t.fields.iter().map(|f| (f.id, f.def.clone())).collect(),
            })
            .collect(),
        occurrences: db.occurrences.clone(),
        ..Default::default()
    };
    let tables: Vec<Json> = s
        .tables
        .iter()
        .map(|(t, ta)| {
            let tbl = db.table(*t);
            let limits: serde_json::Map<String, Json> = ta
                .limits
                .iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    let l = l.as_ref()?;
                    let to = fmp12::calc::context(&l.context).unwrap_or(0);
                    let src = fmp12::calc::decompile(&l.tokens, to, &names).ok()?;
                    Some((["create", "view", "edit", "delete"][i].to_string(), json!(src)))
                })
                .collect();
            json!({
                "table": tbl.map_or_else(|| t.to_string(), |t| t.name.clone()),
                "records": actions(&ta.access),
                "limits": limits,
                "fields": ta.fields.iter().map(|(f, v)| json!({
                    "field": tbl.and_then(|t| t.field(*f)).map_or_else(|| f.to_string(), |f| f.name.clone()),
                    "access": level_name(*v, false),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let item_name = |cat: u8, k: &[u8]| -> String {
        let id = fmp12::path_int(&k[1..]).unwrap_or(0);
        let n = match cat {
            security::LAYOUTS => db
                .layouts
                .iter()
                .find(|l| l.id == id)
                .map(|l| l.name.clone()),
            _ => None,
        };
        n.unwrap_or_else(|| id.to_string())
    };
    let items = |cat: u8| -> Vec<Json> {
        s.items
            .iter()
            .filter(|(k, _)| k.len() > 1 && k[0] == cat)
            .map(|(k, v)| json!({ "item": item_name(cat, k), "access": level_name(v & 3, cat == security::SCRIPTS) }))
            .collect()
    };
    let default = |cat: u8| {
        s.items
            .get(&vec![cat])
            .map(|v| level_name(v & 3, cat == security::SCRIPTS))
    };
    json!({
        "name": s.name,
        "description": s.description,
        "full_access": s.full_access(),
        "records": s.records.as_ref().map(actions),
        "tables": tables,
        "layouts": default(security::LAYOUTS), "layout_access": items(security::LAYOUTS),
        "scripts": default(security::SCRIPTS), "script_access": items(security::SCRIPTS),
        "value_lists": default(security::VALUE_LISTS), "value_list_access": items(security::VALUE_LISTS),
        "menus": format!("{:?}", s.menus()),
        "privileges": flags,
        "extended": sec.extended.iter().filter(|e| e.sets.contains(&s.id)).map(|e| e.keyword.clone()).collect::<Vec<_>>(),
        "min_password": s.min_password,
        "expire_days": s.expire_days,
    })
}

pub fn account_json(sec: &Security, a: &Account) -> Json {
    json!({
        "name": a.name,
        "privilege_set": sec.set(a.privilege_set).map(|s| s.name.clone()),
        "active": a.active,
        "auth": a.auth_type().label(),
        "must_change_password": a.must_change,
        "description": a.description,
    })
}

fn session(d: &Doc) -> Res<security::Session> {
    let s =
        d.db.session()
            .ok_or("this file is not open with an account")?;
    if !s.full_access() {
        return Err(
            "Manage Security needs an account with full access (--account/--password)".into(),
        );
    }
    Ok(s.clone())
}

/// Writes `new` into the open document (in memory; saved like any change).
fn apply(d: &mut Doc, new: &Security) -> Res<()> {
    let s = session(d)?;
    let bytes = security::save(&d.file, &s, new).map_err(|e| e.to_string())?;
    let file = d.file.replaced(bytes).map_err(|e| e.to_string())?;
    d.db.refresh_security(&file).map_err(|e| e.to_string())?;
    d.file = file;
    Ok(())
}

fn load(d: &Doc) -> Res<Security> {
    Security::load(&d.file).map_err(|e| e.to_string())
}

pub fn accounts(d: &mut Doc, rest: &[String]) -> Res<Json> {
    let sec = load(d)?;
    let sub = rest.first().map(String::as_str);
    if sub.is_none() || sub == Some("list") {
        session(d)?;
        return Ok(
            json!({ "accounts": sec.accounts.iter().map(|a| account_json(&sec, a)).collect::<Vec<_>>() }),
        );
    }
    let name = rest.get(1).ok_or("missing account name")?;
    let a = Args(&rest[2..]);
    let mut new = sec.clone();
    let set_id = |n: &str| -> Res<u32> {
        sec.set_named(n)
            .map(|s| s.id)
            .ok_or_else(|| format!("no privilege set named {n:?}"))
    };
    match sub {
        Some("add") => {
            let pw = a
                .value("--new-password")
                .ok_or("--new-password is required")?;
            let set = set_id(a.value("--set").unwrap_or("[Data Entry Only]"))
                .or_else(|_| set_id_of(&sec, 2))?;
            let mut acc = Account::new(name, pw, set);
            acc.active = !a.has("--inactive");
            acc.must_change = a.has("--must-change");
            acc.description = a.value("--description").unwrap_or_default().to_string();
            new.add_account(acc);
        }
        Some("set") => {
            let id = sec
                .account_named(name)
                .ok_or_else(|| format!("no account named {name:?}"))?
                .id;
            let acc = new
                .accounts
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or("no account")?;
            if let Some(n) = a.value("--rename") {
                acc.name = n.to_string();
            }
            if let Some(p) = a.value("--new-password") {
                acc.set_password(p);
            }
            if let Some(s) = a.value("--set") {
                acc.privilege_set = set_id(s)?;
            }
            if a.has("--active") {
                acc.active = true;
            }
            if a.has("--inactive") {
                acc.active = false;
            }
            if a.has("--must-change") {
                acc.must_change = true;
            }
            if a.has("--no-must-change") {
                acc.must_change = false;
            }
            if let Some(t) = a.value("--description") {
                acc.description = t.to_string();
            }
        }
        Some("delete") => {
            let id = sec
                .account_named(name)
                .ok_or_else(|| format!("no account named {name:?}"))?
                .id;
            new.accounts.retain(|x| x.id != id);
        }
        Some(o) => {
            return Err(format!(
                "accounts: unknown sub-command {o:?} (add, set, delete)"
            ));
        }
        None => {}
    }
    apply(d, &new)?;
    let sec = load(d)?;
    Ok(
        json!({ "accounts": sec.accounts.iter().map(|a| account_json(&sec, a)).collect::<Vec<_>>() }),
    )
}

fn set_id_of(sec: &Security, id: u32) -> Res<u32> {
    sec.set(id)
        .map(|s| s.id)
        .ok_or_else(|| "no privilege set".into())
}

pub fn privileges(d: &mut Doc, rest: &[String]) -> Res<Json> {
    let sec = load(d)?;
    let sub = rest.first().map(String::as_str);
    if sub.is_none() || sub == Some("list") {
        session(d)?;
        return Ok(
            json!({ "privilege_sets": sec.sets.iter().map(|s| set_json(d, &sec, s)).collect::<Vec<_>>() }),
        );
    }
    let name = rest.get(1).ok_or("missing privilege set name")?;
    let a = Args(&rest[2..]);
    let mut new = sec.clone();
    let id = match sub {
        Some("add") => {
            let base = match a.value("--copy-of") {
                Some(n) => sec
                    .set_named(n)
                    .cloned()
                    .ok_or_else(|| format!("no privilege set named {n:?}"))?,
                // FileMaker's New privilege set: view records, view layouts,
                // run scripts, view value lists, all menus.
                None => {
                    let mut s = PrivilegeSet {
                        records: Some(RecordAccess::NONE),
                        ..Default::default()
                    };
                    s.set_item(
                        security::LAYOUTS,
                        None,
                        security::NO_ACCESS,
                        security::NO_ACCESS,
                    );
                    s.set_item(security::SCRIPTS, None, security::NO_ACCESS, 0);
                    s.set_item(security::VALUE_LISTS, None, security::NO_ACCESS, 0);
                    s.set_menus(Menus::Minimum);
                    s
                }
            };
            let s = PrivilegeSet {
                name: name.clone(),
                ..base
            };
            let id = new.add_set(s);
            // A copy keeps its extended privileges.
            if let Some(from) = a.value("--copy-of").and_then(|n| sec.set_named(n)) {
                for e in new.extended.iter_mut() {
                    if e.sets.contains(&from.id) {
                        e.sets.insert(id);
                    }
                }
            }
            id
        }
        Some("set") | Some("delete") => {
            sec.set_named(name)
                .ok_or_else(|| format!("no privilege set named {name:?}"))?
                .id
        }
        Some(o) => {
            return Err(format!(
                "privileges: unknown sub-command {o:?} (add, set, delete)"
            ));
        }
        None => 0,
    };
    if sub == Some("delete") {
        if id <= 3 {
            return Err("the built-in privilege sets cannot be deleted".into());
        }
        if sec.accounts.iter().any(|x| x.privilege_set == id) {
            return Err(format!("privilege set {name:?} is used by an account"));
        }
        new.sets.retain(|s| s.id != id);
    } else {
        if id == 1 {
            return Err("[Full Access] cannot be changed".into());
        }
        edit_set(d, &mut new, id, &a)?;
    }
    apply(d, &new)?;
    let sec = load(d)?;
    let out: Vec<Json> = sec
        .sets
        .iter()
        .filter(|s| s.id == id)
        .map(|s| set_json(d, &sec, s))
        .collect();
    Ok(json!({ "privilege_sets": out }))
}

fn edit_set(d: &Doc, new: &mut Security, id: u32, a: &Args) -> Res<()> {
    let db = &d.db;
    let table = |n: &str| -> Res<u32> {
        db.tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(n))
            .map(|t| t.id)
            .ok_or_else(|| format!("no table named {n:?}"))
    };
    let names = fmp12::calc::FileNames::load(&d.file).map_err(|e| e.to_string())?;
    let extended = a.value("--extended").map(|v| {
        v.split(',')
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect::<Vec<_>>()
    });
    let s = new.sets.iter_mut().find(|s| s.id == id).ok_or("no set")?;
    if let Some(n) = a.value("--rename") {
        s.name = n.to_string();
    }
    if let Some(t) = a.value("--description") {
        s.description = t.to_string();
    }
    if let Some(r) = a.value("--records") {
        s.records = Some(record_access(r)?);
    }
    for v in a.values("--table-access") {
        let (t, acts) = v.split_once('=').ok_or("--table-access Table=actions")?;
        let t = table(t)?;
        let ta = s.tables.entry(t).or_insert_with(|| TableAccess {
            access: RecordAccess::ALL,
            ..Default::default()
        });
        let fields = ta.access.fields;
        ta.access = RecordAccess {
            fields,
            ..record_access(acts)?
        };
    }
    for v in a.values("--limit") {
        let (t, rest) = v
            .split_once(':')
            .ok_or("--limit \"Table:action=calculation\"")?;
        let (act, calc) = rest
            .split_once('=')
            .ok_or("--limit \"Table:action=calculation\"")?;
        let i = ["create", "view", "edit", "delete"]
            .iter()
            .position(|x| *x == act.trim())
            .ok_or_else(|| format!("unknown action {act:?}"))?;
        let tid = table(t)?;
        let to = db
            .occurrences
            .iter()
            .find(|o| o.base_table == tid)
            .map(|o| o.id)
            .ok_or("table has no occurrence")?;
        let l = security::limit(calc, to, &names).map_err(|e| e.to_string())?;
        let ta = s.tables.entry(tid).or_insert_with(|| TableAccess {
            access: RecordAccess::ALL,
            ..Default::default()
        });
        match i {
            0 => ta.access.create = Level::Limited,
            1 => ta.access.view = Level::Limited,
            2 => ta.access.edit = Level::Limited,
            _ => ta.access.delete = Level::Limited,
        }
        ta.limits[i] = Some(l);
    }
    for v in a.values("--field-access") {
        let (tf, acc) = v
            .split_once('=')
            .ok_or("--field-access Table::Field=access")?;
        let (t, f) = tf
            .split_once("::")
            .ok_or("--field-access Table::Field=access")?;
        let tid = table(t)?;
        let fid = db
            .table(tid)
            .and_then(|x| x.field_by_name(f))
            .map(|x| x.id)
            .ok_or_else(|| format!("no field {f:?}"))?;
        let base = s.records.unwrap_or(RecordAccess::ALL);
        s.tables
            .entry(tid)
            .or_insert_with(|| TableAccess {
                access: base,
                ..Default::default()
            })
            .fields
            .insert(fid, level(acc)?);
    }
    for (opt, cat) in [
        ("--layouts", security::LAYOUTS),
        ("--scripts", security::SCRIPTS),
        ("--value-lists", security::VALUE_LISTS),
    ] {
        if let Some(v) = a.value(opt) {
            let second = if cat == security::LAYOUTS {
                security::MODIFIABLE
            } else {
                0
            };
            s.set_item(cat, None, level(v)?, second);
        }
    }
    for v in a.values("--layout-access") {
        let (l, acc) = v.split_once('=').ok_or("--layout-access Layout=access")?;
        let lid = db
            .layouts
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(l))
            .map(|x| x.id)
            .ok_or_else(|| format!("no layout named {l:?}"))?;
        s.set_item(
            security::LAYOUTS,
            Some(lid),
            level(acc)?,
            security::MODIFIABLE,
        );
    }
    let scripts = fmp12::scripts::scripts(&d.file).map_err(|e| e.to_string())?;
    for v in a.values("--script-access") {
        let (n, acc) = v.split_once('=').ok_or("--script-access Script=access")?;
        let sid = scripts
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(n))
            .map(|x| x.id)
            .ok_or_else(|| format!("no script named {n:?}"))?;
        s.set_item(security::SCRIPTS, Some(sid), level(acc)?, 0);
    }
    if let Some(m) = a.value("--menus") {
        s.set_menus(match m {
            "all" => Menus::All,
            "editing" => Menus::EditingOnly,
            "minimum" => Menus::Minimum,
            _ => return Err(format!("unknown menus {m:?} (all, editing, minimum)")),
        });
    }
    for v in a.values("--allow") {
        s.set_flag(flag_named(v)?, true);
    }
    for v in a.values("--deny") {
        s.set_flag(flag_named(v)?, false);
    }
    if let Some(n) = a.value("--min-password") {
        s.min_password = n.parse().map_err(|_| "bad --min-password")?;
    }
    if let Some(n) = a.value("--expire-days") {
        s.expire_days = n.parse().map_err(|_| "bad --expire-days")?;
    }
    if let Some(kws) = extended {
        for e in new.extended.iter_mut() {
            if kws.iter().any(|k| k.eq_ignore_ascii_case(&e.keyword)) {
                e.sets.insert(id);
            } else {
                e.sets.remove(&id);
            }
        }
        for k in &kws {
            if !new
                .extended
                .iter()
                .any(|e| e.keyword.eq_ignore_ascii_case(k))
            {
                return Err(format!("no extended privilege {k:?}"));
            }
        }
    }
    Ok(())
}
