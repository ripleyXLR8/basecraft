//! Builds the file `tools/oracle/security_check.ps1` checks in FileMaker:
//! a copy of a template (Tasks) with accounts and a custom privilege set.
//!
//!   cargo run -p fmp12 --example secdemo -- <Tasks en.fmp12> <out.fmp12> [admin password]
//!
//! Accounts (test passwords only): Admin (password from the command
//! line, "test123" by default), alice "test123" in "Clerks", bob
//! "test123" inactive, carol "test123" who must change her password.
//! Clerks: Tasks records view, create, edit only where
//! `Status ≠ "Done"`, no delete; field Description view only, Priority
//! no access; other tables view only; layouts view only, "Task Details"
//! no access; scripts executable only, the first one no access; printing
//! yes, exporting no, menus editing only, fmapp.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::security::{self, Account, Level, PrivilegeSet, RecordAccess, Security};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = fmp12::File::open(&args[1]).unwrap();
    let admin_pw = args.get(3).map_or("test123", |s| s.as_str());
    let s = Security::load(&file).unwrap();
    let session = s
        .login("Admin", "")
        .expect("template Admin with no password");
    let mut new = s.clone();

    let tables = fmp12::schema::tables(&file).unwrap();
    let tasks = tables.iter().find(|t| t.name == "Tasks").unwrap();
    let fid = |n: &str| *tasks.fields.iter().find(|(_, f)| f.name == n).unwrap().0;
    let names = fmp12::calc::FileNames::load(&file).unwrap();
    let to = names
        .occurrences
        .iter()
        .find(|o| o.base_table == tasks.id)
        .unwrap()
        .id;
    let layouts = fmp12::layouts::layouts(&file).unwrap();
    let details = layouts
        .iter()
        .find(|l| l.name.contains("Details"))
        .map(|l| l.id);
    let scripts = fmp12::scripts::scripts(&file).unwrap();

    let mut clerks = PrivilegeSet {
        name: "Clerks".into(),
        description: "BaseCraft test set".into(),
        records: Some(RecordAccess::VIEW),
        ..Default::default()
    };
    let mut ta = security::TableAccess {
        access: RecordAccess {
            create: Level::Yes,
            view: Level::Yes,
            edit: Level::Limited,
            delete: Level::No,
            fields: security::MODIFIABLE,
        },
        ..Default::default()
    };
    ta.limits[2] = Some(security::limit("Tasks::Status ≠ \"Done\"", to, &names).unwrap());
    ta.fields.insert(fid("Description"), security::VIEW_ONLY);
    ta.fields.insert(fid("Priority"), security::NO_ACCESS);
    clerks.tables.insert(tasks.id, ta);
    clerks.set_item(
        security::LAYOUTS,
        None,
        security::VIEW_ONLY,
        security::MODIFIABLE,
    );
    if let Some(d) = details {
        clerks.set_item(
            security::LAYOUTS,
            Some(d),
            security::NO_ACCESS,
            security::MODIFIABLE,
        );
    }
    clerks.set_item(security::SCRIPTS, None, security::EXECUTABLE, 0);
    if let Some(first) = scripts.iter().find(|s| s.options == 8) {
        clerks.set_item(security::SCRIPTS, Some(first.id), security::NO_ACCESS, 0);
    }
    clerks.set_item(security::VALUE_LISTS, None, security::VIEW_ONLY, 0);
    clerks.set_flag(security::flag::PRINT, true);
    clerks.set_flag(security::flag::CHANGE_PASSWORD, true);
    clerks.set_menus(security::Menus::EditingOnly);
    let cid = new.add_set(clerks);
    if let Some(e) = new.extended.iter_mut().find(|e| e.keyword == "fmapp") {
        e.sets.insert(cid);
    }

    new.add_account(Account::new("alice", "test123", cid));
    let mut bob = Account::new("bob", "test123", 2);
    bob.active = false;
    new.add_account(bob);
    let mut carol = Account::new("carol", "test123", 2);
    carol.must_change = true;
    new.add_account(carol);
    if let Some(a) = new.accounts.iter_mut().find(|a| a.name == "Admin") {
        a.set_password(admin_pw);
    }
    let out = security::save(&file, &session, &new).unwrap();
    let check = Security::check(&fmp12::File::from_bytes(out.clone()).unwrap()).unwrap();
    assert!(check.is_empty(), "{check:?}");
    std::fs::write(&args[2], out).unwrap();
    println!("wrote {} (set Clerks = {cid})", args[2]);
}
