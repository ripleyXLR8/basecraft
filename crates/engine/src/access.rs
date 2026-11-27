//! Logging in and the privileges of the logged-in account: which records,
//! fields, layouts and scripts it may use, as FileMaker enforces them.

use fmp12::security::{self, Level, PrivilegeSet, Security, Session};

use crate::{Database, Value};

/// FileMaker's "record access is denied".
pub const RECORD_DENIED: u32 = 200;
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

    /// The account logged in, `None` for a database built in memory.
    pub fn session(&self) -> Option<&Session> {
        self.access.session.as_ref()
    }

    /// Re-Login: a new account for the open file.
    pub fn relogin(
        &mut self,
        file: &fmp12::File,
        account: &str,
        password: &str,
    ) -> Result<(), security::LoginError> {
        let sec = Security::load(file).map_err(|_| security::LoginError::Invalid)?;
        self.access.session = Some(sec.login(account, password)?);
        Ok(())
    }

    /// Reloads accounts and privilege sets after File › Manage › Security
    /// changed `file`; the logged-in account keeps its session with its
    /// privileges as they are now.
    pub fn refresh_security(&mut self, file: &fmp12::File) -> Result<(), fmp12::Error> {
        let sec = Security::load(file)?;
        if let Some(s) = self.access.session.as_ref().and_then(|s| sec.resume(s)) {
            self.access.session = Some(s);
        }
        self.accounts = sec.accounts;
        self.privilege_sets = sec.sets;
        Ok(())
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
        self.set().map_or(security::MODIFIABLE, |s| s.field(table, field))
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
        self.set().map_or(security::MODIFIABLE, |s| s.layout(layout))
    }

    /// Records through a layout (view only: no editing on it).
    pub fn layout_records(&self, layout: u32) -> u8 {
        self.set()
            .map_or(security::MODIFIABLE, |s| s.layout_records(layout))
    }

    /// Script access: a script with no access is not listed or run.
    pub fn script_access(&self, script: u32) -> u8 {
        self.set().map_or(security::MODIFIABLE, |s| s.script(script))
    }

    /// The error of the last action the privileges refused (and forgets it).
    pub fn last_denied(&mut self) -> Option<u32> {
        self.access.denied.take()
    }

    /// Why setting `values` on `record` is refused, if it is.
    pub(crate) fn edit_denied(&self, table: u32, record: usize, values: &[(u32, String)]) -> Option<u32> {
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
            "accountprivilegesetname" | "currentprivilegesetname" | "privilegesetname" => {
                Value::text(s.privileges().name.clone())
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
            db.eval_source("Get ( AccountName ) & \"|\" & Get ( AccountPrivilegeSetName )", t, 0, Some(r))
                .to_text(),
            "clerk|Clerks"
        );

        let mut ro = Database::load_as(&f, "ro", "test123").unwrap();
        assert!(ro.new_record(t).is_none());
        assert_eq!(ro.last_denied(), Some(RECORD_DENIED));
        assert_eq!(
            ro.eval_source("Get ( AccountPrivilegeSetName )", t, 0, None).to_text(),
            "[Accès en lecture seule]"
        );
    }
}
