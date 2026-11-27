//! The account script steps: Add Account, Delete Account, Reset Account
//! Password, Enable Account, Change Password and Re-Login. Each is allowed
//! only as FileMaker allows it (`engine::Database` checks the privileges:
//! full access, or "manage accounts" for accounts outside [Full Access];
//! Change Password needs the set's "change password" privilege), and
//! returns FileMaker's error codes (9, 12, 206, 208, 209, 212, 217…).
//!
//! Parameter storage, as FileMaker 26 writes these steps (`docs/FORMAT.md`
//! §8): the account name (Change Password: the old password) at
//! `[128].[5]`, the password (the new one) at `[129].[5]`, Add Account's
//! privilege set inline at byte 6, and in bytes 22–23 "user must change
//! password" (Add Account, Reset Account Password) or "deactivate"
//! (Enable Account). Option `0x80` = without dialog (Re-Login, Change
//! Password).

use fmp12::scripts::{Calc, Step};

use crate::{Engine, Frame, Host};

pub const CHANGE_PASSWORD: u16 = 83;
pub const ADD_ACCOUNT: u16 = 134;
pub const DELETE_ACCOUNT: u16 = 135;
pub const RESET_ACCOUNT_PASSWORD: u16 = 136;
pub const ENABLE_ACCOUNT: u16 = 137;
pub const RE_LOGIN: u16 = 138;

/// Bytes 22–23 of the step: 1 = must change password / deactivate.
fn flag(s: &Step) -> bool {
    s.raw.get(22..24).is_some_and(|b| b != [0, 0])
}

/// The privilege set of Add Account: inline reference at byte 6
/// (`01` + path integer).
fn inline_set(s: &Step) -> Option<u32> {
    let len = *s.raw.get(6)? as usize;
    let b = s.raw.get(7..7 + len)?;
    match b {
        [1, rest @ ..] => fmp12::path_int(rest),
        _ => None,
    }
}

impl Engine {
    /// Text of calculation parameter `p`.
    fn param_text(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
        p: u32,
    ) -> Result<String, u32> {
        Ok(match s.raw_params.calc(p) {
            Some(Calc(t)) => self.eval(host, frame, &Calc(t))?.to_text(),
            None => String::new(),
        })
    }

    /// Runs an account step.
    pub(crate) fn account_step(
        &mut self,
        host: &mut dyn Host,
        frame: &mut Frame,
        s: &Step,
    ) -> Result<(), u32> {
        let name = self.param_text(host, frame, s, 0)?;
        let password = self.param_text(host, frame, s, 1)?;
        let db = host.db();
        match s.id {
            ADD_ACCOUNT => {
                let set = inline_set(s)
                    .and_then(|id| db.privilege_sets.iter().find(|p| p.id == id))
                    .map(|p| p.name.clone())
                    .ok_or(engine::PRIVILEGES)?;
                db.add_account(&name, &password, &set, flag(s))
            }
            DELETE_ACCOUNT => db.delete_account(&name),
            RESET_ACCOUNT_PASSWORD => db.reset_account_password(&name, &password, flag(s)),
            ENABLE_ACCOUNT => db.enable_account(&name, !flag(s)),
            // Change Password: old password, then the new one.
            CHANGE_PASSWORD => db.change_password(&name, &password),
            // ponytail: "With dialog" logs in with the step's values; no
            // login dialog is shown from a script yet.
            RE_LOGIN => db.relogin(&name, &password),
            _ => Err(crate::error::UNAVAILABLE),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::{Database, Script, Window};
    use fmp12::security::Security;

    struct H {
        db: Database,
        win: Window,
        errors: Vec<u32>,
    }

    impl Host for H {
        fn db(&mut self) -> &mut Database {
            &mut self.db
        }
        fn window(&mut self) -> &mut Window {
            &mut self.win
        }
        fn error(&mut self, _: &Step, code: u32) -> bool {
            self.errors.push(code);
            true
        }
    }

    /// An account step as FileMaker 26 stores it: name and password
    /// calculations, the set inline, the flag in bytes 22–23.
    fn step(id: u16, name: &str, password: Option<&str>, set: u8, flag: bool) -> Step {
        let mut s = Step::new(id);
        for (p, v) in [(128u32, Some(name)), (129, password)] {
            if let Some(v) = v {
                let t = fmp12::save::text_literal_tokens(v);
                s.raw_params.kv.insert((vec![p, 5], 5), t);
            }
        }
        if set != 0 {
            s.raw[6..9].copy_from_slice(&[2, 1, set]);
        }
        s.raw[23] = flag as u8;
        s
    }

    /// The steps FileMaker 26 ran in `tools/oracle/account_steps.xml`, with
    /// the error codes it gave: 212 for a missing account and a wrong
    /// Re-Login, 12 for a duplicate name, 9 for Add Account without the
    /// privilege. The file BaseCraft then writes logs in with the new
    /// passwords and its digests match.
    #[test]
    fn account_steps_as_in_filemaker() {
        let blank =
            fmp12::File::from_bytes(include_bytes!("../../fmp12/assets/blank.fmp12").to_vec())
                .unwrap();
        let db = Database::load(&blank).unwrap();
        let layout = db.layouts.first().map_or(0, |l| l.id);
        let win = Window::new(&db, layout);
        let mut h = H {
            db,
            win,
            errors: Vec::new(),
        };
        let steps = vec![
            step(ADD_ACCOUNT, "vec1", Some("test123"), 2, false),
            step(ADD_ACCOUNT, "vec2", Some("Test-é123"), 3, true),
            step(ADD_ACCOUNT, "vec3", Some("test123"), 2, false),
            step(ENABLE_ACCOUNT, "vec3", None, 0, true),
            step(ADD_ACCOUNT, "vec4", Some("test123"), 2, false),
            step(RESET_ACCOUNT_PASSWORD, "vec4", Some("test456"), 0, false),
            step(ADD_ACCOUNT, "vec5", Some("test123"), 2, false),
            step(DELETE_ACCOUNT, "vec5", None, 0, false),
            step(DELETE_ACCOUNT, "nobody", None, 0, false),
            step(ADD_ACCOUNT, "vec1", Some("test123"), 2, false),
            step(RE_LOGIN, "vec1", Some("wrong1"), 0, false),
            step(RE_LOGIN, "vec1", Some("test123"), 0, false),
            step(ADD_ACCOUNT, "vec6", Some("test123"), 2, false),
            step(CHANGE_PASSWORD, "test123", Some("test789"), 0, false),
        ];
        let mut e = Engine::new(vec![Script {
            id: 1,
            name: "accounts".into(),
            options: 8,
            steps,
        }]);
        e.perform(&mut h, 1, None);
        assert_eq!(h.errors, [212, 12, 212, 9]);
        let s = h.db.session().unwrap();
        assert_eq!(s.account_name(), "vec1");

        let out = fmp12::File::from_bytes(h.db.save(&blank).unwrap()).unwrap();
        assert!(Security::check(&out).unwrap().is_empty());
        let sec = Security::load(&out).unwrap();
        let a = |n: &str| sec.account_named(n).unwrap();
        assert!(a("vec1").check_password("test789"));
        assert!(a("vec2").check_password("Test-é123") && a("vec2").must_change);
        assert!(!a("vec3").active);
        assert!(a("vec4").check_password("test456"));
        assert!(sec.account_named("vec5").is_none() && sec.account_named("vec6").is_none());
    }
}
