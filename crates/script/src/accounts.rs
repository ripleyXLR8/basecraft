//! The account script steps: Add Account, Delete Account, Reset Account
//! Password, Enable Account, Change Password and Re-Login. Each is allowed
//! only as FileMaker allows it (`engine::Database` checks the privileges:
//! full access, or "manage accounts" for accounts outside [Full Access];
//! Change Password needs the set's "change password" privilege), and
//! returns FileMaker's error codes (9, 206, 208, 209, 212, 213, 217…).
//!
//! Parameter storage (seen in steps FileMaker 26 wrote, `docs/FORMAT.md`
//! §8): calculations at `[128 + p].[5]`, the privilege set inline at byte
//! 6, flags in the step's option bits.

use fmp12::scripts::{Calc, Step};

use crate::{Engine, Frame, Host};

pub const CHANGE_PASSWORD: u16 = 83;
pub const ADD_ACCOUNT: u16 = 134;
pub const DELETE_ACCOUNT: u16 = 135;
pub const RESET_ACCOUNT_PASSWORD: u16 = 136;
pub const ENABLE_ACCOUNT: u16 = 137;
pub const RE_LOGIN: u16 = 138;

/// Option bit: "User must change password on next login" (Add Account,
/// Reset Account Password).
const EXPIRE: u32 = 0x1;
/// Option bit of Enable Account: deactivate.
const DEACTIVATE: u32 = 0x1;

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
                db.add_account(&name, &password, &set, s.options & EXPIRE != 0)
            }
            DELETE_ACCOUNT => db.delete_account(&name),
            RESET_ACCOUNT_PASSWORD => {
                db.reset_account_password(&name, &password, s.options & EXPIRE != 0)
            }
            ENABLE_ACCOUNT => db.enable_account(&name, s.options & DEACTIVATE == 0),
            // Change Password: old password, then the new one.
            CHANGE_PASSWORD => db.change_password(&name, &password),
            // ponytail: "With dialog" logs in with the step's values; no
            // login dialog is shown from a script yet.
            RE_LOGIN => db.relogin(&name, &password),
            _ => Err(crate::error::UNAVAILABLE),
        }
    }
}
