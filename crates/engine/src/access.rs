//! Logging in and the privileges of the logged-in account: which records,
//! fields, layouts and scripts it may use, as FileMaker enforces them.

use fmp12::security::{self, Level, PrivilegeSet, Security, Session};

use crate::{Database, Value};

/// "Command is invalid" (an empty account name).
pub const INVALID: u32 = 5;
/// FileMaker's "insufficient privileges".
pub const PRIVILEGES: u32 = 9;
/// FileMaker's "record access is denied".
pub const RECORD_DENIED: u32 = 200;
/// No such account (FileMaker 26 gives 212 to Delete Account, Reset
/// Account Password and Enable Account).
pub const NO_ACCOUNT: u32 = 212;
/// An account of that name exists (Add Account).
pub const NAME_EXISTS: u32 = 12;
/// "User does not have sufficient privileges to modify administrator account".
pub const ADMIN_ACCOUNT: u32 = 217;
/// "Field access is denied".
pub const FIELD_DENIED: u32 = 201;
/// FileMaker's alert when a privilege does not allow an action.
pub const DENIED_MESSAGE: &str = "Your access privileges do not allow you to perform this action.";

/// The actions of a record privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create = 0,
    View = 1,
    Edit = 2,
    Delete = 3,
}

impl Database {
    /// Opens a protected file with an account name and password (the login
    /// dialog's). The file's own "Log in using" is not tried.
    pub fn load_as(
        file: &fmp12::File,
        account: &str,
        password: &str,
    ) -> Result<Self, fmp12::Error> {
        let sec = Security::load(file)?;
        let session = sec.login(account, password).map_err(fmp12::Error::Login)?;
        Self::load_with(file, Some(session))
    }

    /// Opens a file with an account that must change its password (or
    /// whose password expired): logs in with the old password and sets the
    /// new one, saved with the file.
    pub fn load_changing_password(
        file: &fmp12::File,
        account: &str,
        old: &str,
        new: &str,
    ) -> Result<Self, fmp12::Error> {
        let mut sec = Security::load(file)?;
        let session = sec
            .change_password(account, old, new)
            .map_err(|e| match e {
                security::ChangeError::Login(e) => fmp12::Error::Login(e),
                e => fmp12::Error::Security(e.to_string()),
            })?;
        let mut db = Self::load_with(file, Some(session))?;
        db.record(sec);
        Ok(db)
    }

    /// The account logged in, `None` for a database built in memory.
    pub fn session(&self) -> Option<&Session> {
        self.access.session.as_ref()
    }

    /// Reloads accounts and privilege sets after File › Manage › Security
    /// changed `file`; the logged-in account keeps its session with its
    /// privileges as they are now.
    pub fn refresh_security(&mut self, file: &fmp12::File) -> Result<(), fmp12::Error> {
        let sec = Security::load(file)?;
        self.set_security(sec);
        self.access.pending.clear();
        Ok(())
    }

    fn set_security(&mut self, sec: Security) {
        if let Some(s) = self.access.session.as_ref().and_then(|s| sec.resume(s)) {
            self.access.session = Some(s);
        }
        self.accounts = sec.accounts.clone();
        self.privilege_sets = sec.sets.clone();
        self.access.security = Some(sec);
    }

    /// The file's accounts and privileges, with the changes not saved yet.
    pub fn security(&self) -> Option<&Security> {
        self.access.security.as_ref()
    }

    /// Re-Login: another account of the file. On failure the account
    /// logged in stays (FileMaker's 212, 210 or 211).
    pub fn relogin(&mut self, account: &str, password: &str) -> Result<(), u32> {
        let sec = self.access.security.as_ref().ok_or(PRIVILEGES)?;
        let s = sec.login(account, password).map_err(|e| e.code())?;
        self.access.session = Some(s);
        Ok(())
    }

    /// Change Password of the logged-in account (206, 208, 209, 212).
    pub fn change_password(&mut self, old: &str, new: &str) -> Result<(), u32> {
        let name = self.session().ok_or(PRIVILEGES)?.account_name().to_string();
        let mut sec = self.access.security.clone().ok_or(PRIVILEGES)?;
        let s = sec.change_password(&name, old, new).map_err(|e| e.code())?;
        self.record(sec);
        self.access.session = Some(s);
        Ok(())
    }

    /// Whether the account may add, delete, reset or enable accounts of
    /// privilege set `set`: full access, or "manage accounts" for sets
    /// other than [Full Access] (FileMaker's 9 and 217).
    fn may_manage(&self, set: u32) -> Result<(), u32> {
        if self.full_access() {
            return Ok(());
        }
        if !self.allows(security::flag::MANAGE_ACCOUNTS) {
            return Err(PRIVILEGES);
        }
        if set == 1 {
            return Err(ADMIN_ACCOUNT);
        }
        Ok(())
    }

    /// Changes the account named `name` with `f`, after the privilege check.
    fn edit_account(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut security::Account),
    ) -> Result<(), u32> {
        let mut sec = self.access.security.clone().ok_or(PRIVILEGES)?;
        let a = sec.account_named(name).ok_or(NO_ACCOUNT)?;
        self.may_manage(a.privilege_set)?;
        let id = a.id;
        if let Some(a) = sec.accounts.iter_mut().find(|a| a.id == id) {
            f(a);
        }
        self.store(sec)
    }

    /// Keeps `sec` if FileMaker's rules allow it (one active full access
    /// account stays…); written by [`Database::save`].
    fn store(&mut self, sec: Security) -> Result<(), u32> {
        let ok = sec.accounts.iter().any(|a| {
            a.privilege_set == 1
                && a.active
                && !a.is_guest()
                && a.auth_type() == security::AuthType::FileMaker
        });
        if !ok {
            return Err(ADMIN_ACCOUNT);
        }
        self.record(sec);
        Ok(())
    }

    /// Keeps a change made by the logged-in account, for [`Database::save`].
    fn record(&mut self, sec: Security) {
        if let Some(s) = self.access.session.clone() {
            match self.access.pending.last_mut() {
                Some((by, last)) if *by == s => *last = sec.clone(),
                _ => self.access.pending.push((s, sec.clone())),
            }
        }
        self.set_security(sec);
    }

    /// Add Account: a FileMaker account with privilege set `set` (by name).
    pub fn add_account(
        &mut self,
        name: &str,
        password: &str,
        set: &str,
        must_change: bool,
    ) -> Result<(), u32> {
        let mut sec = self.access.security.clone().ok_or(PRIVILEGES)?;
        let set = sec.set_named(set).ok_or(PRIVILEGES)?.id;
        self.may_manage(set)?;
        if sec.account_named(name).is_some() {
            return Err(NAME_EXISTS);
        }
        if name.trim().is_empty() {
            return Err(crate::access::INVALID);
        }
        let mut a = security::Account::new(name, password, set);
        a.must_change = must_change;
        sec.add_account(a);
        self.store(sec)
    }

    /// Delete Account (not the account logged in).
    pub fn delete_account(&mut self, name: &str) -> Result<(), u32> {
        let mut sec = self.access.security.clone().ok_or(PRIVILEGES)?;
        let a = sec.account_named(name).ok_or(NO_ACCOUNT)?;
        self.may_manage(a.privilege_set)?;
        let id = a.id;
        if self.session().is_some_and(|s| s.account_id() == id) {
            return Err(PRIVILEGES);
        }
        sec.accounts.retain(|a| a.id != id);
        self.store(sec)
    }

    /// Reset Account Password.
    pub fn reset_account_password(
        &mut self,
        name: &str,
        password: &str,
        must_change: bool,
    ) -> Result<(), u32> {
        self.edit_account(name, |a| {
            a.set_password(password);
            a.must_change = must_change;
        })
    }

    /// Enable Account: activate or deactivate.
    pub fn enable_account(&mut self, name: &str, active: bool) -> Result<(), u32> {
        self.edit_account(name, |a| a.active = active)
    }

    /// Writes the accounts changed by script steps into `bytes` (the rest
    /// of [`Database::save`]), as the logged-in account.
    pub(crate) fn save_security(&self, mut bytes: Vec<u8>) -> Result<Vec<u8>, fmp12::Error> {
        for (s, sec) in &self.access.pending {
            let file = fmp12::File::from_bytes(bytes)?;
            bytes =
                security::save(&file, s, sec).map_err(|e| fmp12::Error::Security(e.to_string()))?;
        }
        Ok(bytes)
    }

    fn set(&self) -> Option<&PrivilegeSet> {
        self.access.session.as_ref().map(|s| s.privileges())
    }

    pub fn full_access(&self) -> bool {
        self.set().is_none_or(|s| s.full_access())
    }

    /// One of the set's other privileges ([`security::flag`]).
    pub fn allows(&self, flag: u16) -> bool {
        self.set().is_none_or(|s| s.allows(flag))
    }

    pub fn can_print(&self) -> bool {
        self.allows(security::flag::PRINT)
    }

    pub fn can_export(&self) -> bool {
        self.allows(security::flag::EXPORT)
    }

    /// Whether the account may do `action` on `record` of `table` (`None`
    /// for a record not created yet). An action limited by a calculation
    /// is allowed where the calculation is true for the record.
    pub fn can(&self, action: Action, table: u32, record: Option<usize>) -> bool {
        let Some(set) = self.set() else {
            return true;
        };
        let a = set.table(table);
        let level = [a.create, a.view, a.edit, a.delete][action as usize];
        match level {
            Level::Yes => true,
            Level::No => false,
            Level::Limited => {
                let Some(l) = set.limit(table, action as usize) else {
                    return false;
                };
                let to = fmp12::calc::context(&l.context).unwrap_or(0);
                self.eval_tokens_in(&l.tokens, table, to, record, None)
                    .is_ok_and(|v: Value| v.truth())
            }
        }
    }

    /// Field access: [`security::MODIFIABLE`], [`security::VIEW_ONLY`] or
    /// [`security::NO_ACCESS`].
    pub fn field_access(&self, table: u32, field: u32) -> u8 {
        self.set()
            .map_or(security::MODIFIABLE, |s| s.field(table, field))
    }

    /// Whether the account may type into `field` of `record`.
    pub fn can_edit_field(&self, table: u32, record: usize, field: u32) -> bool {
        self.field_access(table, field) == security::MODIFIABLE
            && self.can(Action::Edit, table, Some(record))
    }

    /// Whether the value of `field` of `record` may be shown (FileMaker
    /// draws `<No Access>` otherwise).
    pub fn can_see(&self, table: u32, record: usize, field: u32) -> bool {
        self.field_access(table, field) != security::NO_ACCESS
            && self.can(Action::View, table, Some(record))
    }

    /// Layout access: a layout with no access is not listed.
    pub fn layout_access(&self, layout: u32) -> u8 {
        self.set()
            .map_or(security::MODIFIABLE, |s| s.layout(layout))
    }

    /// Records through a layout (view only: no editing on it).
    pub fn layout_records(&self, layout: u32) -> u8 {
        self.set()
            .map_or(security::MODIFIABLE, |s| s.layout_records(layout))
    }

    /// Script access: a script with no access is not listed or run.
    pub fn script_access(&self, script: u32) -> u8 {
        self.set()
            .map_or(security::MODIFIABLE, |s| s.script(script))
    }

    /// The error of the last action the privileges refused (and forgets it).
    pub fn last_denied(&mut self) -> Option<u32> {
        self.access.denied.take()
    }

    /// Why setting `values` on `record` is refused, if it is.
    pub(crate) fn edit_denied(
        &self,
        table: u32,
        record: usize,
        values: &[(u32, String)],
    ) -> Option<u32> {
        if values
            .iter()
            .any(|(f, _)| self.field_access(table, *f) != security::MODIFIABLE)
        {
            return Some(FIELD_DENIED);
        }
        (!self.can(Action::Edit, table, Some(record))).then_some(RECORD_DENIED)
    }

    /// `Get` flags of the logged-in account.
    pub(crate) fn account_flag(&self, flag: &str) -> Option<Value> {
        let s = self.access.session.as_ref()?;
        let f = flag.to_ascii_lowercase();
        Some(match f.as_str() {
            "accountname" => Value::text(s.account_name()),
            // FileMaker gives the built-in sets' English names, whatever
            // the file's language ("[Data Entry Only]" in a French file).
            "accountprivilegesetname" | "currentprivilegesetname" | "privilegesetname" => {
                Value::text(match s.privileges().id {
                    1 => "[Full Access]".to_string(),
                    2 => "[Data Entry Only]".to_string(),
                    3 => "[Read-Only Access]".to_string(),
                    _ => s.privileges().name.clone(),
                })
            }
            "accountextendedprivileges" | "currentextendedprivileges" | "extendedprivileges" => {
                Value::text(s.extended().join("\r"))
            }
            "accounttype" => Value::text("FileMaker"),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> fmp12::File {
        fmp12::File::from_bytes(include_bytes!("../../fmp12/assets/blank.fmp12").to_vec()).unwrap()
    }

    /// A blank file with "clerk" (view only, no export) and "ro" ([Read-Only Access]).
    fn protected() -> fmp12::File {
        let f = blank();
        let s = Security::load(&f).unwrap();
        let admin = s.login("Admin", "").unwrap();
        let mut new = s.clone();
        let mut set = new.sets[1].clone();
        set.name = "Clerks".into();
        set.set_flag(security::flag::EXPORT, false);
        set.records = Some(security::RecordAccess {
            delete: Level::No,
            ..security::RecordAccess::ALL
        });
        let t = fmp12::schema::tables(&f).unwrap()[0].id;
        let mut ta = security::TableAccess {
            access: security::RecordAccess {
                delete: Level::No,
                ..security::RecordAccess::ALL
            },
            ..Default::default()
        };
        ta.fields.insert(1, security::VIEW_ONLY);
        set.tables.insert(t, ta);
        let id = new.add_set(set);
        new.add_account(security::Account::new("clerk", "test123", id));
        new.add_account(security::Account::new("ro", "test123", 3));
        let a = new.accounts.iter_mut().find(|a| a.name == "Admin").unwrap();
        a.set_password("test123");
        fmp12::File::from_bytes(security::save(&f, &admin, &new).unwrap()).unwrap()
    }

    #[test]
    fn protected_files_need_credentials() {
        let f = protected();
        assert!(matches!(
            Database::load(&f),
            Err(fmp12::Error::LoginRequired)
        ));
        assert!(matches!(
            Database::load_as(&f, "clerk", "wrong"),
            Err(fmp12::Error::Login(security::LoginError::Invalid))
        ));
        let db = Database::load_as(&f, "Admin", "test123").unwrap();
        assert!(db.full_access());
        // The blank file logs in by itself, as Admin.
        let db = Database::load(&blank()).unwrap();
        assert_eq!(db.session().unwrap().account_name(), "Admin");
    }

    /// An account that must change its password opens the file only with
    /// a new one, which the saved file keeps (FileMaker's change dialog).
    #[test]
    fn must_change_password_at_login() {
        let f = blank();
        let s = Security::load(&f).unwrap();
        let admin = s.login("Admin", "").unwrap();
        let mut new = s.clone();
        let mut carol = security::Account::new("carol", "test123", 2);
        carol.must_change = true;
        new.add_account(carol);
        let f = fmp12::File::from_bytes(security::save(&f, &admin, &new).unwrap()).unwrap();
        assert!(matches!(
            Database::load_as(&f, "carol", "test123"),
            Err(fmp12::Error::Login(security::LoginError::MustChange))
        ));
        assert!(Database::load_changing_password(&f, "carol", "wrong", "test456").is_err());
        let db = Database::load_changing_password(&f, "carol", "test123", "test456").unwrap();
        let out = fmp12::File::from_bytes(db.save(&f).unwrap()).unwrap();
        assert!(Security::check(&out).unwrap().is_empty());
        assert!(Database::load_as(&out, "carol", "test456").is_ok());
    }

    #[test]
    fn privileges_are_enforced() {
        let f = protected();
        let mut db = Database::load_as(&f, "clerk", "test123").unwrap();
        let t = db.tables[0].id;
        assert!(!db.can_export() && db.can_print());
        assert!(db.can(Action::Create, t, None));
        let r = db.new_record(t).unwrap();
        assert!(!db.can(Action::Delete, t, Some(r)));
        assert!(db.delete_record(t, r).is_empty(), "delete denied");
        assert_eq!(db.last_denied(), Some(RECORD_DENIED));
        assert!(!db.can_edit_field(t, r, 1) && db.can_edit_field(t, r, 2));
        assert_eq!(
            db.eval_source(
                "Get ( AccountName ) & \"|\" & Get ( AccountPrivilegeSetName )",
                t,
                0,
                Some(r)
            )
            .to_text(),
            "clerk|Clerks"
        );

        let mut ro = Database::load_as(&f, "ro", "test123").unwrap();
        assert!(ro.new_record(t).is_none());
        assert_eq!(ro.last_denied(), Some(RECORD_DENIED));
        assert_eq!(
            ro.eval_source("Get ( AccountPrivilegeSetName )", t, 0, None)
                .to_text(),
            "[Read-Only Access]"
        );
    }
}
