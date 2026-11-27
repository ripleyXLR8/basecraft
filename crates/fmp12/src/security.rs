//! Accounts (`[23].[1]`), privilege sets (`[23].[2]`) and extended
//! privileges (`[23].[3]`): decoding, password checks, the integrity
//! digests FileMaker checks when it opens a file, and writing
//! (`docs/FORMAT.md` §15).
//!
//! Writing goes through [`save`], which takes a [`Session`]: the only way
//! to get one is [`Security::login`] with the right password, and [`save`]
//! refuses a session without full access. There is no way here to open,
//! change or read a protected file's security without its credentials.

use std::collections::{BTreeMap, BTreeSet};

use crate::save::{
    fresh_uuid, ipath, kv_entry, len_int, marker, path_elem, read_len_int, spelled, value_entries,
};
use crate::write::{self, Entry, Item};
use crate::{Chunk, Error, File};

// ---------------------------------------------------------------- model

/// Who may do an action on records: no, yes, or where a calculation is true.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    No,
    Yes,
    Limited,
}

impl Level {
    fn from_bits(b: u8) -> Level {
        match b & 3 {
            1 => Level::Yes,
            2 => Level::Limited,
            _ => Level::No,
        }
    }
    fn bits(self) -> u8 {
        match self {
            Level::No => 0,
            Level::Yes => 1,
            Level::Limited => 2,
        }
    }
}

/// Access to fields, layouts, scripts and value lists (`DBAccessLevel`).
pub const NO_ACCESS: u8 = 1;
pub const VIEW_ONLY: u8 = 2;
/// Modifiable (scripts: modifiable; value lists and layouts: modifiable).
pub const MODIFIABLE: u8 = 3;
/// Scripts: executable only (same value as view only).
pub const EXECUTABLE: u8 = 2;

/// Record privileges of a table (key 02): two bytes, create, view, edit,
/// delete as 2-bit levels from the high bits down, then the access of
/// fields without an entry of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordAccess {
    pub create: Level,
    pub view: Level,
    pub edit: Level,
    pub delete: Level,
    /// [`NO_ACCESS`], [`VIEW_ONLY`] or [`MODIFIABLE`].
    pub fields: u8,
}

impl Default for RecordAccess {
    fn default() -> Self {
        Self::ALL
    }
}

impl RecordAccess {
    pub const ALL: RecordAccess = RecordAccess {
        create: Level::Yes,
        view: Level::Yes,
        edit: Level::Yes,
        delete: Level::Yes,
        fields: MODIFIABLE,
    };
    pub const VIEW: RecordAccess = RecordAccess {
        create: Level::No,
        view: Level::Yes,
        edit: Level::No,
        delete: Level::No,
        fields: MODIFIABLE,
    };
    pub const NONE: RecordAccess = RecordAccess {
        create: Level::No,
        view: Level::No,
        edit: Level::No,
        delete: Level::No,
        fields: NO_ACCESS,
    };

    fn decode(b: &[u8]) -> RecordAccess {
        let (a, f) = (
            b.first().copied().unwrap_or(0),
            b.get(1).copied().unwrap_or(1),
        );
        RecordAccess {
            create: Level::from_bits(a >> 6),
            view: Level::from_bits(a >> 4),
            edit: Level::from_bits(a >> 2),
            delete: Level::from_bits(a),
            fields: f & 3,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let a = self.create.bits() << 6
            | self.view.bits() << 4
            | self.edit.bits() << 2
            | self.delete.bits();
        vec![a, self.fields & 3]
    }

    fn levels(&self) -> [Level; 4] {
        [self.create, self.view, self.edit, self.delete]
    }
}

/// The calculation limiting one action of a table: keys 04 (context), 05
/// (source tokens) and 06 (compiled form) of `[05].[T].[6…9]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limit {
    pub context: Vec<u8>,
    pub tokens: Vec<u8>,
    pub compiled: Vec<u8>,
}

/// Custom privileges of one base table (`[23].[2].[5].[P].[5].[T]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableAccess {
    pub access: RecordAccess,
    /// Field id → [`NO_ACCESS`], [`VIEW_ONLY`] or [`MODIFIABLE`].
    pub fields: BTreeMap<u32, u8>,
    /// Calculations of the create, view, edit and delete actions that are
    /// [`Level::Limited`].
    pub limits: [Option<Limit>; 4],
}

/// Catalog keys of the `09` entries: layouts, scripts, value lists.
pub const LAYOUTS: u8 = 0x04;
pub const SCRIPTS: u8 = 0x11;
pub const VALUE_LISTS: u8 = 0x21;

/// Menu commands a set allows (other privileges bits 0x2000 and 0x4000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menus {
    All,
    EditingOnly,
    Minimum,
}

/// Other privileges (`0a`, stored XOR 0x6000).
pub mod flag {
    pub const CREATE_VALUE_LISTS: u16 = 0x0001;
    /// "Disconnect user from server when idle" is off.
    pub const NO_IDLE_DISCONNECT: u16 = 0x0002;
    pub const CHANGE_PASSWORD: u16 = 0x0004;
    pub const MANAGE_EXTENDED: u16 = 0x0008;
    pub const MANAGE_ACCOUNTS: u16 = 0x0010;
    pub const MANAGE_DATABASE: u16 = 0x0020;
    pub const MANAGE_MENUS: u16 = 0x0080;
    pub const PRINT: u16 = 0x0100;
    pub const EXPORT: u16 = 0x0200;
    pub const OVERRIDE_VALIDATION: u16 = 0x0400;
    pub const CREATE_SCRIPTS: u16 = 0x0800;
    pub const CREATE_LAYOUTS: u16 = 0x1000;
    pub const EDIT_MENUS: u16 = 0x2000;
    pub const OTHER_MENUS: u16 = 0x4000;
    pub const OPEN_QUICKLY: u16 = 0x8000;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivilegeSet {
    pub id: u32,
    /// e.g. "[Full Access]" for the built-in sets.
    pub name: String,
    pub description: String,
    /// Records of tables without an entry of their own (key 02; `None`:
    /// the key is absent, which reads as no access).
    pub records: Option<RecordAccess>,
    pub tables: BTreeMap<u32, TableAccess>,
    /// Layout, script and value list access (`09`): key bytes (catalog key,
    /// then the item's key for an item of its own) → access quad.
    pub items: BTreeMap<Vec<u8>, u8>,
    /// Other privileges (logical bits, [`flag`]).
    pub flags: u16,
    /// Passwords must be changed every N days (`0c`, 0 = never).
    pub expire_days: u16,
    /// Minimum password length (`0d`, 0 = none).
    pub min_password: u16,
}

impl PrivilegeSet {
    /// The built-in [Full Access] set (key 1) passes every check.
    pub fn full_access(&self) -> bool {
        self.id == 1
    }

    pub fn allows(&self, f: u16) -> bool {
        self.full_access() || self.flags & f == f
    }

    pub fn menus(&self) -> Menus {
        match (
            self.flags & flag::EDIT_MENUS != 0,
            self.flags & flag::OTHER_MENUS != 0,
        ) {
            _ if self.full_access() => Menus::All,
            (true, true) => Menus::All,
            (true, false) => Menus::EditingOnly,
            _ => Menus::Minimum,
        }
    }

    pub fn set_menus(&mut self, m: Menus) {
        self.flags &= !(flag::EDIT_MENUS | flag::OTHER_MENUS);
        self.flags |= match m {
            Menus::All => flag::EDIT_MENUS | flag::OTHER_MENUS,
            Menus::EditingOnly => flag::EDIT_MENUS,
            Menus::Minimum => 0,
        };
    }

    pub fn set_flag(&mut self, f: u16, on: bool) {
        if on {
            self.flags |= f;
        } else {
            self.flags &= !f;
        }
    }

    /// The access quad of item `id` of catalog `cat` (its own entry or the
    /// catalog's default; a set without either: no access).
    fn quad(&self, cat: u8, id: Option<u32>) -> u8 {
        let own = id.and_then(|id| {
            let mut k = vec![cat];
            k.extend(path_elem(id));
            self.items.get(&k).copied()
        });
        own.or_else(|| self.items.get(&vec![cat]).copied())
            .unwrap_or(0xD5)
    }

    /// The access of item `id` of catalog `cat` as stored (`None`: the
    /// catalog's default), whatever the set.
    pub fn item_access(&self, cat: u8, id: Option<u32>) -> u8 {
        self.quad(cat, id) & 3
    }

    /// Sets the access of an item (or the catalog's default), keeping its
    /// other fields.
    pub fn set_item_access(&mut self, cat: u8, id: Option<u32>, v: u8) {
        let q = self.quad(cat, id);
        self.set_item(cat, id, v, (q >> 2) & 3);
    }

    /// Layout `id` itself: [`NO_ACCESS`], [`VIEW_ONLY`] or [`MODIFIABLE`].
    pub fn layout(&self, id: u32) -> u8 {
        if self.full_access() {
            return MODIFIABLE;
        }
        self.quad(LAYOUTS, Some(id)) & 3
    }

    /// Records through layout `id`.
    pub fn layout_records(&self, id: u32) -> u8 {
        if self.full_access() {
            return MODIFIABLE;
        }
        (self.quad(LAYOUTS, Some(id)) >> 2) & 3
    }

    /// Script `id`: [`MODIFIABLE`], [`EXECUTABLE`] or [`NO_ACCESS`].
    pub fn script(&self, id: u32) -> u8 {
        if self.full_access() {
            return MODIFIABLE;
        }
        self.quad(SCRIPTS, Some(id)) & 3
    }

    pub fn value_list(&self, id: u32) -> u8 {
        if self.full_access() {
            return MODIFIABLE;
        }
        self.quad(VALUE_LISTS, Some(id)) & 3
    }

    /// The default access of a catalog (`id` `None`) or one item's own.
    pub fn set_item(&mut self, cat: u8, id: Option<u32>, first: u8, second: u8) {
        let mut k = vec![cat];
        if let Some(id) = id {
            k.extend(path_elem(id));
        }
        // Third and fourth fields: 1 and 3 in every entry FileMaker writes.
        self.items.insert(k, 0xD0 | (second & 3) << 2 | (first & 3));
    }

    /// Record access of table `t`.
    pub fn table(&self, t: u32) -> RecordAccess {
        if self.full_access() {
            return RecordAccess::ALL;
        }
        match self.tables.get(&t) {
            Some(a) => a.access,
            None => self.records.unwrap_or(RecordAccess::NONE),
        }
    }

    /// Field `f` of table `t`: [`NO_ACCESS`], [`VIEW_ONLY`] or [`MODIFIABLE`].
    pub fn field(&self, t: u32, f: u32) -> u8 {
        if self.full_access() {
            return MODIFIABLE;
        }
        match self.tables.get(&t) {
            Some(a) => a.fields.get(&f).copied().unwrap_or(a.access.fields),
            None => self.records.map_or(NO_ACCESS, |r| r.fields),
        }
    }

    /// The calculation limiting action `i` (0 create, 1 view, 2 edit,
    /// 3 delete) of table `t`.
    pub fn limit(&self, t: u32, i: usize) -> Option<&Limit> {
        self.tables.get(&t)?.limits.get(i)?.as_ref()
    }
}

/// An extended privilege (`[23].[3].[5].[E]`): a keyword such as `fmapp`
/// and the privilege sets that have it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtendedPrivilege {
    pub id: u32,
    pub keyword: String,
    pub description: String,
    pub sets: BTreeSet<u32>,
}

/// How an account is authenticated (key 02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    FileMaker,
    External,
    OAuth(u8),
}

impl AuthType {
    pub fn label(self) -> &'static str {
        match self {
            AuthType::FileMaker => "FileMaker",
            AuthType::External => "External Server",
            AuthType::OAuth(k) => match k >> 3 {
                2 => "Amazon",
                3 => "Google",
                4 => "Microsoft Azure AD",
                5 => "Claris ID",
                6 => "Apple",
                7 => "AD FS",
                _ => "OAuth",
            },
        }
    }
}

/// When a password was last set (status key `0a`): a date and the seconds
/// since midnight; the microseconds carry the account's flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub seconds: u32,
    pub micros: u32,
}

const INACTIVE: u32 = 1 << 19;
const MUST_CHANGE: u32 = 1 << 18;

impl Default for Stamp {
    /// 1/1/1024 at midnight: what an account without the key reads as.
    fn default() -> Self {
        Stamp {
            year: 1024,
            month: 1,
            day: 1,
            seconds: 0,
            micros: 0,
        }
    }
}

impl Stamp {
    /// The stored value: a byte counting the rest, the date
    /// (`0x800000 | year << 10 | month << 6 | day`), then the seconds as a
    /// number key: `80`, the count of integer digits, each digit + 1 as a
    /// nibble (integer digits then six fraction digits, trailing zeros
    /// dropped), a zero nibble when the count is odd.
    pub fn encode(&self) -> Vec<u8> {
        let d = 0x80_0000 | (self.year as u32) << 10 | (self.month as u32) << 6 | self.day as u32;
        let int = if self.seconds == 0 {
            String::new()
        } else {
            self.seconds.to_string()
        };
        let frac = format!("{:06}", self.micros);
        let digits = format!("{int}{}", frac.trim_end_matches('0'));
        let mut v = d.to_be_bytes()[1..].to_vec();
        v.extend([0x80, int.len() as u8]);
        let nib: Vec<u8> = digits.bytes().map(|c| c - b'0' + 1).collect();
        for p in nib.chunks(2) {
            v.push(p[0] << 4 | p.get(1).copied().unwrap_or(0));
        }
        let mut out = vec![v.len() as u8];
        out.extend(v);
        out
    }

    pub fn decode(b: &[u8]) -> Option<Stamp> {
        let (n, v) = b.split_first()?;
        let v = v.get(..*n as usize)?;
        let d = u32::from_be_bytes([0, *v.first()?, *v.get(1)?, *v.get(2)?]) & 0x7F_FFFF;
        if v.get(3) != Some(&0x80) {
            return None;
        }
        let nint = *v.get(4)? as usize;
        let mut digits = Vec::new();
        for b in v.get(5..)? {
            for x in [b >> 4, b & 15] {
                if x == 0 {
                    break;
                }
                digits.push(x - 1);
            }
        }
        let num = |ds: &[u8]| ds.iter().fold(0u32, |a, x| a * 10 + *x as u32);
        let frac = digits.get(nint.min(digits.len())..).unwrap_or(&[]);
        let mut micros = num(frac.get(..frac.len().min(6)).unwrap_or(&[]));
        for _ in frac.len()..6 {
            micros *= 10;
        }
        Some(Stamp {
            year: (d >> 10) as u16,
            month: ((d >> 6) & 15) as u8,
            day: (d & 63) as u8,
            seconds: num(&digits[..nint.min(digits.len())]),
            micros,
        })
    }

    /// Now, in UTC, to the second, keeping `flags` (microseconds).
    pub fn now(flags: u32) -> Stamp {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
        let z = (secs / 86_400) as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + (month <= 2) as i64) as u16;
        Stamp {
            year,
            month,
            day,
            seconds: (secs % 86_400) as u32,
            micros: flags,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub id: u32,
    /// The display name ("[Guest]" for the guest account).
    pub name: String,
    /// Privilege set id.
    pub privilege_set: u32,
    /// Key 02: 0 = an account of the file itself (FileMaker).
    pub kind: u8,
    pub description: String,
    pub active: bool,
    /// "User must change password on next login".
    pub must_change: bool,
    /// When the password was last set.
    pub stamp: Stamp,
    /// Key 06: version, salt, PBKDF2 hash, check bytes.
    password: Vec<u8>,
    /// The stored name text (the guest's is `[` U+E002 `]`).
    raw_name: String,
}

impl Account {
    pub fn auth_type(&self) -> AuthType {
        match self.kind {
            0 => AuthType::FileMaker,
            1 => AuthType::External,
            k => AuthType::OAuth(k),
        }
    }

    /// The built-in [Guest] account (key 1).
    pub fn is_guest(&self) -> bool {
        self.id == 1
    }

    /// Whether `password` is this account's password.
    pub fn check_password(&self, password: &str) -> bool {
        let Some(salt) = self.password.get(1..11) else {
            return false;
        };
        self.kind == 0
            && self.password.first() == Some(&1)
            && self.password.get(11..31) == Some(&password_hash(password, salt)[..])
    }

    /// Sets a new password (new salt), stamped now, flags kept.
    pub fn set_password(&mut self, password: &str) {
        self.password = password_data(password, &fresh_salt(self.id));
        self.stamp = Stamp::now(self.stamp.micros);
    }

    fn status(&self) -> Stamp {
        let flags = (!self.active as u32) * INACTIVE + self.must_change as u32 * MUST_CHANGE;
        Stamp {
            micros: (self.stamp.micros & !(INACTIVE | MUST_CHANGE)) | flags,
            ..self.stamp
        }
    }

    /// A new FileMaker account (not yet in a file: see [`Security::add_account`]).
    pub fn new(name: &str, password: &str, privilege_set: u32) -> Account {
        let mut a = Account {
            name: name.to_string(),
            raw_name: name.to_string(),
            privilege_set,
            active: true,
            ..Default::default()
        };
        a.set_password(password);
        a
    }
}

/// Everything in `[23]` that BaseCraft reads and writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Security {
    pub accounts: Vec<Account>,
    pub sets: Vec<PrivilegeSet>,
    pub extended: Vec<ExtendedPrivilege>,
    /// File Options › "Log in using": account name and password (`[23].[1].[1]`
    /// keys 16 and 6); `None` when the option is off.
    pub auto_login: Option<(String, String)>,
    /// `[2]` key 254, which the digests mix in.
    internal_creation: [u8; 4],
    /// `[23].[4].[1]` key 16, when the file version is above 10.
    file_access_uuid: Option<Vec<u8>>,
}

// ---------------------------------------------------------------- passwords

/// FileMaker's fixed 30-byte pepper.
const PEPPER: &[u8; 30] =
    b"FileMaker, Inc.\0\xca\x3c\x23\x90\xa5\x5b\x71\x74\xb6\x1d\xf5\xb2\x6d\x00";

/// Precomposed Latin letters (U+00C0–U+017F) and their decomposition: a
/// password's text is decomposed before it is hashed.
/// ponytail: Latin-1 and Latin Extended-A only; other text is hashed as
/// typed (right for characters FileMaker does not decompose).
#[rustfmt::skip]
const DECOMPOSE: &[(u16, u16, u16)] = &[
    (0x00C0, 0x0041, 0x0300), (0x00C1, 0x0041, 0x0301), (0x00C2, 0x0041, 0x0302), (0x00C3, 0x0041, 0x0303),
    (0x00C4, 0x0041, 0x0308), (0x00C5, 0x0041, 0x030A), (0x00C7, 0x0043, 0x0327), (0x00C8, 0x0045, 0x0300),
    (0x00C9, 0x0045, 0x0301), (0x00CA, 0x0045, 0x0302), (0x00CB, 0x0045, 0x0308), (0x00CC, 0x0049, 0x0300),
    (0x00CD, 0x0049, 0x0301), (0x00CE, 0x0049, 0x0302), (0x00CF, 0x0049, 0x0308), (0x00D1, 0x004E, 0x0303),
    (0x00D2, 0x004F, 0x0300), (0x00D3, 0x004F, 0x0301), (0x00D4, 0x004F, 0x0302), (0x00D5, 0x004F, 0x0303),
    (0x00D6, 0x004F, 0x0308), (0x00D9, 0x0055, 0x0300), (0x00DA, 0x0055, 0x0301), (0x00DB, 0x0055, 0x0302),
    (0x00DC, 0x0055, 0x0308), (0x00DD, 0x0059, 0x0301), (0x00E0, 0x0061, 0x0300), (0x00E1, 0x0061, 0x0301),
    (0x00E2, 0x0061, 0x0302), (0x00E3, 0x0061, 0x0303), (0x00E4, 0x0061, 0x0308), (0x00E5, 0x0061, 0x030A),
    (0x00E7, 0x0063, 0x0327), (0x00E8, 0x0065, 0x0300), (0x00E9, 0x0065, 0x0301), (0x00EA, 0x0065, 0x0302),
    (0x00EB, 0x0065, 0x0308), (0x00EC, 0x0069, 0x0300), (0x00ED, 0x0069, 0x0301), (0x00EE, 0x0069, 0x0302),
    (0x00EF, 0x0069, 0x0308), (0x00F1, 0x006E, 0x0303), (0x00F2, 0x006F, 0x0300), (0x00F3, 0x006F, 0x0301),
    (0x00F4, 0x006F, 0x0302), (0x00F5, 0x006F, 0x0303), (0x00F6, 0x006F, 0x0308), (0x00F9, 0x0075, 0x0300),
    (0x00FA, 0x0075, 0x0301), (0x00FB, 0x0075, 0x0302), (0x00FC, 0x0075, 0x0308), (0x00FD, 0x0079, 0x0301),
    (0x00FF, 0x0079, 0x0308), (0x0100, 0x0041, 0x0304), (0x0101, 0x0061, 0x0304), (0x0102, 0x0041, 0x0306),
    (0x0103, 0x0061, 0x0306), (0x0104, 0x0041, 0x0328), (0x0105, 0x0061, 0x0328), (0x0106, 0x0043, 0x0301),
    (0x0107, 0x0063, 0x0301), (0x0108, 0x0043, 0x0302), (0x0109, 0x0063, 0x0302), (0x010A, 0x0043, 0x0307),
    (0x010B, 0x0063, 0x0307), (0x010C, 0x0043, 0x030C), (0x010D, 0x0063, 0x030C), (0x010E, 0x0044, 0x030C),
    (0x010F, 0x0064, 0x030C), (0x0112, 0x0045, 0x0304), (0x0113, 0x0065, 0x0304), (0x0114, 0x0045, 0x0306),
    (0x0115, 0x0065, 0x0306), (0x0116, 0x0045, 0x0307), (0x0117, 0x0065, 0x0307), (0x0118, 0x0045, 0x0328),
    (0x0119, 0x0065, 0x0328), (0x011A, 0x0045, 0x030C), (0x011B, 0x0065, 0x030C), (0x011C, 0x0047, 0x0302),
    (0x011D, 0x0067, 0x0302), (0x011E, 0x0047, 0x0306), (0x011F, 0x0067, 0x0306), (0x0120, 0x0047, 0x0307),
    (0x0121, 0x0067, 0x0307), (0x0122, 0x0047, 0x0327), (0x0123, 0x0067, 0x0327), (0x0124, 0x0048, 0x0302),
    (0x0125, 0x0068, 0x0302), (0x0128, 0x0049, 0x0303), (0x0129, 0x0069, 0x0303), (0x012A, 0x0049, 0x0304),
    (0x012B, 0x0069, 0x0304), (0x012C, 0x0049, 0x0306), (0x012D, 0x0069, 0x0306), (0x012E, 0x0049, 0x0328),
    (0x012F, 0x0069, 0x0328), (0x0130, 0x0049, 0x0307), (0x0134, 0x004A, 0x0302), (0x0135, 0x006A, 0x0302),
    (0x0136, 0x004B, 0x0327), (0x0137, 0x006B, 0x0327), (0x0139, 0x004C, 0x0301), (0x013A, 0x006C, 0x0301),
    (0x013B, 0x004C, 0x0327), (0x013C, 0x006C, 0x0327), (0x013D, 0x004C, 0x030C), (0x013E, 0x006C, 0x030C),
    (0x0143, 0x004E, 0x0301), (0x0144, 0x006E, 0x0301), (0x0145, 0x004E, 0x0327), (0x0146, 0x006E, 0x0327),
    (0x0147, 0x004E, 0x030C), (0x0148, 0x006E, 0x030C), (0x014C, 0x004F, 0x0304), (0x014D, 0x006F, 0x0304),
    (0x014E, 0x004F, 0x0306), (0x014F, 0x006F, 0x0306), (0x0150, 0x004F, 0x030B), (0x0151, 0x006F, 0x030B),
    (0x0154, 0x0052, 0x0301), (0x0155, 0x0072, 0x0301), (0x0156, 0x0052, 0x0327), (0x0157, 0x0072, 0x0327),
    (0x0158, 0x0052, 0x030C), (0x0159, 0x0072, 0x030C), (0x015A, 0x0053, 0x0301), (0x015B, 0x0073, 0x0301),
    (0x015C, 0x0053, 0x0302), (0x015D, 0x0073, 0x0302), (0x015E, 0x0053, 0x0327), (0x015F, 0x0073, 0x0327),
    (0x0160, 0x0053, 0x030C), (0x0161, 0x0073, 0x030C), (0x0162, 0x0054, 0x0327), (0x0163, 0x0074, 0x0327),
    (0x0164, 0x0054, 0x030C), (0x0165, 0x0074, 0x030C), (0x0168, 0x0055, 0x0303), (0x0169, 0x0075, 0x0303),
    (0x016A, 0x0055, 0x0304), (0x016B, 0x0075, 0x0304), (0x016C, 0x0055, 0x0306), (0x016D, 0x0075, 0x0306),
    (0x016E, 0x0055, 0x030A), (0x016F, 0x0075, 0x030A), (0x0170, 0x0055, 0x030B), (0x0171, 0x0075, 0x030B),
    (0x0172, 0x0055, 0x0328), (0x0173, 0x0075, 0x0328), (0x0174, 0x0057, 0x0302), (0x0175, 0x0077, 0x0302),
    (0x0176, 0x0059, 0x0302), (0x0177, 0x0079, 0x0302), (0x0178, 0x0059, 0x0308), (0x0179, 0x005A, 0x0301),
    (0x017A, 0x007A, 0x0301), (0x017B, 0x005A, 0x0307), (0x017C, 0x007A, 0x0307), (0x017D, 0x005A, 0x030C),
    (0x017E, 0x007A, 0x030C),
];

/// FileMaker's password hash: the text's UTF-16 units (decomposed; a space
/// before a leading combining mark), each XOR-ed with two bytes of the
/// pepper, then PBKDF2-HMAC-SHA1 with the 10-byte salt, 1 iteration, 20
/// bytes. One iteration of PBKDF2 for 20 bytes is HMAC(P, salt ‖ 00000001).
pub fn password_hash(password: &str, salt: &[u8]) -> [u8; 20] {
    use hmac::{KeyInit, Mac};
    let mut units: Vec<u16> = Vec::new();
    for u in password.encode_utf16() {
        if units.is_empty() && (0x300..0x370).contains(&u) {
            units.push(0x20);
        }
        match DECOMPOSE.binary_search_by_key(&u, |d| d.0) {
            Ok(i) => units.extend([DECOMPOSE[i].1, DECOMPOSE[i].2]),
            Err(_) => units.push(u),
        }
    }
    let p: Vec<u8> = units
        .iter()
        .enumerate()
        .flat_map(|(i, u)| {
            let k = (PEPPER[2 * i % 30] as u16) << 8 | PEPPER[(2 * i + 1) % 30] as u16;
            (u ^ k).to_be_bytes()
        })
        .collect();
    let mut out = [0; 20];
    if let Ok(mut m) = hmac::Hmac::<sha1::Sha1>::new_from_slice(&p) {
        m.update(salt);
        m.update(&[0, 0, 0, 1]);
        out.copy_from_slice(&m.finalize().into_bytes());
    }
    out
}

/// Key 06: version 1, salt, hash, then `hash[1] & 0x1f` check bytes, byte
/// `31 + i` = byte `11 + i` XOR byte `1 + i` of the value being written.
fn password_data(password: &str, salt: &[u8; 10]) -> Vec<u8> {
    let mut v = vec![1];
    v.extend(salt);
    v.extend(password_hash(password, salt));
    for i in 0..(v[12] & 0x1F) as usize {
        v.push(v[11 + i] ^ v[1 + i]);
    }
    v
}

fn fresh_salt(seed: u32) -> [u8; 10] {
    let mut s = [0; 10];
    s.copy_from_slice(&fresh_uuid(seed as u64 ^ 0x5A17)[..10]);
    s
}

// ---------------------------------------------------------------- login

/// A logged-in account. Only [`Security::login`] makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    account: u32,
    name: String,
    set: PrivilegeSet,
    extended: Vec<String>,
}

impl Session {
    pub fn account_id(&self) -> u32 {
        self.account
    }
    /// `Get ( AccountName )`.
    pub fn account_name(&self) -> &str {
        &self.name
    }
    /// The account's privilege set (`Get ( AccountPrivilegeSetName )`: its name).
    pub fn privileges(&self) -> &PrivilegeSet {
        &self.set
    }
    /// `Get ( AccountExtendedPrivileges )`: keywords, in name order.
    pub fn extended(&self) -> &[String] {
        &self.extended
    }
    pub fn full_access(&self) -> bool {
        self.set.full_access()
    }
}

/// Why a login was refused, with FileMaker's error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginError {
    /// 212: no such account, or the wrong password.
    Invalid,
    /// 210: the account is inactive.
    Inactive,
    /// 211: the password must be changed (or has expired).
    MustChange,
}

impl LoginError {
    /// FileMaker Pro's alert (English).
    pub fn message(self) -> &'static str {
        match self {
            LoginError::Invalid => {
                "The account name and password you entered do not match. Please try again."
            }
            LoginError::Inactive => "The account is inactive.",
            LoginError::MustChange => "The password for this account must be changed.",
        }
    }

    pub fn code(self) -> u32 {
        match self {
            LoginError::Invalid => 212,
            LoginError::Inactive => 210,
            LoginError::MustChange => 211,
        }
    }
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for LoginError {}

impl Security {
    /// Reads `[23]` (and the two `[2]` values the digests mix in).
    pub fn load(file: &File) -> Result<Security, Error> {
        let entries = write::entries(file)?;
        Ok(parse(&entries))
    }

    pub fn account(&self, id: u32) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn set(&self, id: u32) -> Option<&PrivilegeSet> {
        self.sets.iter().find(|s| s.id == id)
    }

    /// Account names are not case-sensitive.
    pub fn account_named(&self, name: &str) -> Option<&Account> {
        let n = name.to_lowercase();
        self.accounts
            .iter()
            .find(|a| !a.is_guest() && a.name.to_lowercase() == n)
    }

    /// Logs in with an account name and password, as FileMaker does: the
    /// inactive check comes before the password, the password-change check
    /// after it.
    pub fn login(&self, name: &str, password: &str) -> Result<Session, LoginError> {
        let a = self.account_named(name).ok_or(LoginError::Invalid)?;
        if a.kind != 0 {
            return Err(LoginError::Invalid);
        }
        if !a.active {
            return Err(LoginError::Inactive);
        }
        if !a.check_password(password) {
            return Err(LoginError::Invalid);
        }
        if a.must_change || self.expired(a) {
            return Err(LoginError::MustChange);
        }
        Ok(self.session(a))
    }

    /// Logs in as [Guest] (when it is active).
    pub fn login_guest(&self) -> Result<Session, LoginError> {
        match self.account(1) {
            Some(g) if g.active => Ok(self.session(g)),
            Some(_) => Err(LoginError::Inactive),
            None => Err(LoginError::Invalid),
        }
    }

    /// Logs in with "Log in using" of File Options, if it is on and works.
    pub fn login_auto(&self) -> Option<Session> {
        let (name, pw) = self.auto_login.as_ref()?;
        if name == GUEST_TEXT {
            return self.login_guest().ok();
        }
        self.login(name, pw).ok()
    }

    /// Changes a password at a forced change (error 211) or through
    /// File › Change Password: `old` must be the current password.
    pub fn change_password(
        &mut self,
        name: &str,
        old: &str,
        new: &str,
    ) -> Result<Session, ChangeError> {
        let id = self
            .account_named(name)
            .ok_or(ChangeError::Login(LoginError::Invalid))?
            .id;
        match self.login(name, old) {
            Ok(_) | Err(LoginError::MustChange) => {}
            Err(e) => return Err(ChangeError::Login(e)),
        }
        let a = self
            .account(id)
            .ok_or(ChangeError::Login(LoginError::Invalid))?;
        let set = self.set(a.privilege_set).cloned().unwrap_or_default();
        if !set.allows(flag::CHANGE_PASSWORD) && !(a.must_change || self.expired(a)) {
            return Err(ChangeError::NotAllowed);
        }
        if (new.chars().count() as u16) < set.min_password {
            return Err(ChangeError::TooShort(set.min_password));
        }
        if a.check_password(new) {
            return Err(ChangeError::Same);
        }
        if let Some(a) = self.accounts.iter_mut().find(|a| a.id == id) {
            a.set_password(new);
            a.must_change = false;
        }
        self.login(name, new).map_err(ChangeError::Login)
    }

    /// Whether the set's "change password every N days" has run out.
    fn expired(&self, a: &Account) -> bool {
        let days = self.set(a.privilege_set).map_or(0, |s| s.expire_days);
        if days == 0 {
            return false;
        }
        let n = Stamp::now(0);
        let day = |s: &Stamp| s.year as i64 * 372 + s.month as i64 * 31 + s.day as i64;
        // ponytail: calendar days approximated as 31-day months.
        day(&n) - day(&a.stamp) >= days as i64
    }

    fn session(&self, a: &Account) -> Session {
        let set = self.set(a.privilege_set).cloned().unwrap_or_default();
        let extended = self
            .extended
            .iter()
            .filter(|e| e.sets.contains(&a.privilege_set))
            .map(|e| e.keyword.clone())
            .collect();
        Session {
            account: a.id,
            name: a.name.clone(),
            set,
            extended,
        }
    }

    /// The session of an account already logged in, from this (newer)
    /// state of the file: the same account, with its privileges as they are
    /// now (an account made inactive keeps its session, as in FileMaker).
    /// `None` when the account is gone.
    pub fn resume(&self, s: &Session) -> Option<Session> {
        Some(self.session(self.account(s.account)?))
    }

    pub fn set_named(&self, name: &str) -> Option<&PrivilegeSet> {
        let n = name.to_lowercase();
        self.sets.iter().find(|s| s.name.to_lowercase() == n)
    }

    /// Whether opening the file needs a login dialog: "Log in using" is
    /// off or does not log in.
    pub fn needs_login(&self) -> bool {
        self.login_auto().is_none()
    }

    /// Adds an account (with a new id); returns the id.
    pub fn add_account(&mut self, mut a: Account) -> u32 {
        a.id = self.accounts.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        a.raw_name = a.name.clone();
        let id = a.id;
        self.accounts.push(a);
        id
    }

    /// Adds a privilege set (with a new id); returns the id.
    pub fn add_set(&mut self, mut s: PrivilegeSet) -> u32 {
        s.id = self.sets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let id = s.id;
        self.sets.push(s);
        id
    }
}

/// Why a password change was refused (FileMaker's codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeError {
    Login(LoginError),
    /// 206: the privilege set does not allow changing the password.
    NotAllowed,
    /// 208: shorter than the set's minimum.
    TooShort(u16),
    /// 209: the new password must differ from the old one.
    Same,
}

impl ChangeError {
    pub fn code(self) -> u32 {
        match self {
            ChangeError::Login(e) => e.code(),
            ChangeError::NotAllowed => 206,
            ChangeError::TooShort(_) => 208,
            ChangeError::Same => 209,
        }
    }
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeError::Login(e) => e.fmt(f),
            ChangeError::NotAllowed => {
                f.write_str("Your privileges do not allow changing the password.")
            }
            ChangeError::TooShort(n) => {
                write!(f, "The password must be at least {n} characters long.")
            }
            ChangeError::Same => {
                f.write_str("The new password must be different from the old one.")
            }
        }
    }
}

impl std::error::Error for ChangeError {}

/// The guest account's stored name, also its password and its "Log in
/// using" value.
const GUEST_TEXT: &str = "[\u{e002}]";

/// Built-in names are stored as private-use characters that FileMaker
/// translates on display (U+E002 = "Guest", as in the "[Guest]" account).
pub fn localize(s: &str) -> String {
    // ponytail: only the Guest token has been observed.
    s.replace('\u{e002}', "Guest")
}

pub fn accounts(file: &File) -> Result<Vec<Account>, Error> {
    Ok(Security::load(file)?.accounts)
}

pub fn privilege_sets(file: &File) -> Result<Vec<PrivilegeSet>, Error> {
    Ok(Security::load(file)?.sets)
}

// ---------------------------------------------------------------- reading

/// Raw key bytes and value of a key/value item (1- and 2-byte keys keep
/// their spelling; byte-string keys as they are).
fn raw_kv(item: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let op = *item.first()?;
    match op {
        0x00 => Some((vec![*item.get(1)?], Vec::new())),
        0x08 => Some((item.get(1..3)?.to_vec(), Vec::new())),
        _ => match crate::chunk::parse(item).ok()?.into_iter().next()? {
            Chunk::Kv { data, .. } if (0x01..=0x07).contains(&op) => {
                Some((vec![*item.get(1)?], data.to_vec()))
            }
            Chunk::Kv { data, .. } if (0x09..=0x0F).contains(&op) => {
                Some((item.get(1..3)?.to_vec(), data.to_vec()))
            }
            Chunk::KvLong { key, data } => Some((key.to_vec(), data.to_vec())),
            _ => None,
        },
    }
}

fn int_key(k: &[u8]) -> Option<u32> {
    match k {
        [a] => Some(*a as u32),
        _ => crate::path_int(k),
    }
}

/// One member's keys: (sub-path, raw key) → value.
type Member = BTreeMap<(Vec<u32>, Vec<u8>), Vec<u8>>;

fn members(entries: &[Entry], cat: &[u32]) -> BTreeMap<u32, Member> {
    let mut out: BTreeMap<u32, Member> = BTreeMap::new();
    for e in entries {
        let Some(p) = ipath(e) else { continue };
        if p.len() <= cat.len() || p[..cat.len()] != *cat {
            continue;
        }
        let m = out.entry(p[cat.len()]).or_default();
        if let Some((k, v)) = raw_kv(&e.item) {
            m.insert((p[cat.len() + 1..].to_vec(), k), v);
        }
    }
    out
}

fn get<'a>(m: &'a Member, sub: &[u32], key: u8) -> Option<&'a Vec<u8>> {
    m.get(&(sub.to_vec(), vec![key]))
}

/// Text at key `key`, inline or in the chunks of sub-node `[key]`.
fn text_at(m: &Member, sub: &[u32], key: u8) -> String {
    let mut long = sub.to_vec();
    long.push(key as u32);
    let chunks: Vec<u8> = m
        .iter()
        .filter(|((p, k), _)| *p == long && int_key(k).is_some_and(|k| k >= 1))
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    if !chunks.is_empty() {
        return crate::text(&chunks);
    }
    get(m, sub, key).map_or_else(String::new, |v| crate::text(v))
}

fn header(entries: &[Entry], path: &[u32]) -> Member {
    let mut m = Member::new();
    for e in entries {
        if let Some(p) = ipath(e)
            && p.starts_with(path)
            && let Some((k, v)) = raw_kv(&e.item)
        {
            m.insert((p[path.len()..].to_vec(), k), v);
        }
    }
    m
}

fn parse(entries: &[Entry]) -> Security {
    let mut sec = Security::default();
    let file2 = header(entries, &[2]);
    if let Some(v) = get(&file2, &[], 254) {
        sec.internal_creation
            .copy_from_slice(&[v.as_slice(), &[0; 4]].concat()[..4]);
    }
    let version = get(&file2, &[], 5).map_or(0, |v| v.iter().fold(0u32, |a, b| a << 8 | *b as u32));
    if version > 10 {
        sec.file_access_uuid = get(&header(entries, &[23, 4, 1]), &[], 16)
            .filter(|u| u.len() == 16)
            .cloned();
    }
    let acc_head = header(entries, &[23, 1, 1]);
    let login = text_at(&acc_head, &[], 16);
    if !login.is_empty() {
        sec.auto_login = Some((login, text_at(&acc_head, &[], 6)));
    }
    for (id, m) in members(entries, &[23, 1, 5]) {
        let raw_name = text_at(&m, &[], 16);
        let stamp = get(&m, &[], 0x0A)
            .and_then(|v| Stamp::decode(v))
            .unwrap_or_default();
        sec.accounts.push(Account {
            id,
            name: localize(&raw_name),
            raw_name,
            privilege_set: get(&m, &[], 0x0B)
                .and_then(|v| read_len_int(v))
                .unwrap_or(0) as u32,
            kind: get(&m, &[], 2)
                .and_then(|v| v.first().copied())
                .unwrap_or(0),
            description: text_at(&m, &[], 3),
            active: stamp.micros & INACTIVE == 0,
            must_change: stamp.micros & MUST_CHANGE != 0,
            // The flags live in `active` and `must_change` ([`Account::status`]
            // puts them back), so equal accounts compare equal.
            stamp: Stamp {
                micros: stamp.micros & !(INACTIVE | MUST_CHANGE),
                ..stamp
            },
            password: get(&m, &[], 6).cloned().unwrap_or_default(),
        });
    }
    for (id, m) in members(entries, &[23, 2, 5]) {
        let mut s = PrivilegeSet {
            id,
            name: localize(&text_at(&m, &[], 0x10)),
            description: text_at(&m, &[], 3),
            records: get(&m, &[], 2).map(|v| RecordAccess::decode(v)),
            flags: get(&m, &[], 0x0A)
                .map_or(0, |v| u16::from_be_bytes([v[0], *v.get(1).unwrap_or(&0)]))
                ^ 0x6000,
            expire_days: get(&m, &[], 0x0C).map_or(0, |v| be16(v)),
            min_password: get(&m, &[], 0x0D).map_or(0, |v| be16(v)),
            ..Default::default()
        };
        for ((p, k), v) in &m {
            match p.as_slice() {
                [9] => {
                    if let Some(b) = v.first() {
                        s.items.insert(k.clone(), *b);
                    }
                }
                [5, t] => {
                    let ta = s.tables.entry(*t).or_default();
                    if k.as_slice() == [2] {
                        ta.access = RecordAccess::decode(v);
                    }
                }
                [5, t, 5] => {
                    if let (Some(f), Some(b)) = (int_key(k), v.first()) {
                        s.tables.entry(*t).or_default().fields.insert(f, *b);
                    }
                }
                [5, t, a @ 6..=9] => {
                    let ta = s.tables.entry(*t).or_default();
                    let l = ta.limits[*a as usize - 6].get_or_insert_with(Limit::default);
                    match k.as_slice() {
                        [4] => l.context = v.clone(),
                        [5] => l.tokens = v.clone(),
                        [6] => l.compiled = v.clone(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        sec.sets.push(s);
    }
    for (id, m) in members(entries, &[23, 3, 5]) {
        sec.extended.push(ExtendedPrivilege {
            id,
            keyword: text_at(&m, &[], 0x10),
            description: text_at(&m, &[], 3),
            sets: m
                .keys()
                .filter(|(p, _)| p.as_slice() == [5])
                .filter_map(|(_, k)| int_key(k))
                .collect(),
        });
    }
    // Extended privileges in the order of their name index (keywords'
    // collation keys), as Get ( AccountExtendedPrivileges ) lists them.
    sec.extended
        .sort_by_key(|e| crate::collate::name_key(&e.keyword).unwrap_or_default());
    sec
}

fn be16(v: &[u8]) -> u16 {
    u16::from_be_bytes([*v.first().unwrap_or(&0), *v.get(1).unwrap_or(&0)])
}

// ---------------------------------------------------------------- digests

/// A one-segment path key: its length + 1, its length, its bytes.
fn path_key(k: &[u8]) -> Vec<u8> {
    let mut v = vec![k.len() as u8 + 1, k.len() as u8];
    v.extend(k);
    v
}

fn utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// Key spelling of an id in a key/value item (1 byte below 256).
fn key_spelling(k: u32) -> Vec<u8> {
    if k < 0x100 {
        vec![k as u8]
    } else {
        path_elem(k)
    }
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    for p in parts {
        h.update(p);
    }
    let mut out = [0; 16];
    out.copy_from_slice(&h.finalize());
    out
}

impl Security {
    /// Account key 04: MD5 of the file access catalog's UUID and `[2]`
    /// key 254, the account's key, kind, name (UTF-16BE), the first 31
    /// bytes of its password data, description, status and privilege set
    /// key (each of the last two after a byte holding its length).
    pub fn account_digest(&self, a: &Account) -> [u8; 16] {
        let mut b = Vec::new();
        if let Some(u) = &self.file_access_uuid {
            b.extend(u);
            b.extend(self.internal_creation);
        }
        b.extend(path_key(&path_elem(a.id)));
        b.push(a.kind);
        b.extend(utf16be(&a.raw_name));
        b.extend(a.password.iter().take(31));
        b.extend(utf16be(&a.description));
        b.extend(path_key(&a.status().encode()).get(1..).unwrap_or(&[]));
        b.extend(
            path_key(&len_int(a.privilege_set as u64))
                .get(1..)
                .unwrap_or(&[]),
        );
        md5(&[&b])
    }

    /// Privilege set key 04.
    pub fn set_digest(&self, s: &PrivilegeSet) -> [u8; 16] {
        let access = |b: &mut Vec<u8>, a: Option<RecordAccess>, limits: &[Option<Limit>; 4]| {
            let Some(a) = a else {
                b.extend([0, 0, 0, 0, 1]);
                return;
            };
            let mut lv = a.levels();
            for (i, l) in lv.iter_mut().enumerate() {
                if *l == Level::Limited && limits[i].as_ref().is_none_or(|x| x.compiled.is_empty())
                {
                    *l = Level::No;
                }
            }
            b.extend(lv.iter().map(|l| l.bits()));
            b.push(a.fields);
            for (i, l) in lv.iter().enumerate() {
                if let (Level::Limited, Some(x)) = (l, &limits[i]) {
                    b.extend(&x.compiled);
                    b.push(x.context.len() as u8);
                    b.extend(&x.context);
                }
            }
        };
        let mut b = Vec::new();
        access(&mut b, s.records, &Default::default());
        let ic = u32::from_be_bytes(self.internal_creation);
        b.extend((s.flags as u32).wrapping_add(ic).to_be_bytes());
        b.extend(utf16be(&s.description));
        b.extend(s.expire_days.to_be_bytes());
        b.extend(s.min_password.to_be_bytes());
        for (t, ta) in &s.tables {
            b.extend(path_key(&path_elem(*t)));
            access(&mut b, Some(ta.access), &ta.limits);
            for (f, v) in &ta.fields {
                b.extend(path_key(&key_spelling(*f)));
                b.push(*v);
            }
        }
        for (k, v) in &s.items {
            b.extend(path_key(k));
            b.push(*v);
        }
        md5(&[&b])
    }

    /// Extended privilege key 04: keyword, description, the sharing byte
    /// (2), then each set's key.
    /// ponytail: the sharing byte is 1 when key 02 is `01 01` (fmreauthenticate10
    /// in every file we have); BaseCraft keeps key 02 as read.
    fn extended_digest(e: &ExtendedPrivilege, sharing: u8) -> [u8; 16] {
        let mut b = utf16be(&e.keyword);
        b.extend(utf16be(&e.description));
        b.push(sharing);
        for s in &e.sets {
            b.extend(path_key(&key_spelling(*s)));
        }
        md5(&[&b])
    }

    /// Stored digests that do not match their member (empty for a file
    /// FileMaker opens without "access privileges damaged").
    pub fn check(file: &File) -> Result<Vec<String>, Error> {
        let entries = write::entries(file)?;
        let sec = parse(&entries);
        let mut bad = Vec::new();
        let stored = |cat: u32, id: u32| {
            members(&entries, &[23, cat, 5])
                .get(&id)
                .and_then(|m| get(m, &[], 4).cloned())
        };
        for a in &sec.accounts {
            if stored(1, a.id).as_deref() != Some(&sec.account_digest(a)[..]) {
                bad.push(format!("account {} ({})", a.id, a.name));
            }
        }
        for s in &sec.sets {
            if stored(2, s.id).as_deref() != Some(&sec.set_digest(s)[..]) {
                bad.push(format!("privilege set {} ({})", s.id, s.name));
            }
        }
        let ext = members(&entries, &[23, 3, 5]);
        for e in &sec.extended {
            let sharing = ext.get(&e.id).map_or(2, sharing_byte);
            if stored(3, e.id).as_deref() != Some(&Self::extended_digest(e, sharing)[..]) {
                bad.push(format!("extended privilege {} ({})", e.id, e.keyword));
            }
        }
        Ok(bad)
    }
}

fn sharing_byte(m: &Member) -> u8 {
    match get(m, &[], 2).map(|v| v.as_slice()) {
        Some([1, x]) => *x,
        _ => 2,
    }
}

// ---------------------------------------------------------------- writing

/// Why [`save`] refused.
#[derive(Debug)]
pub enum SaveError {
    /// The session is not a full access login (FileMaker's 9).
    NotFullAccess,
    /// A rule FileMaker enforces in Manage Security (message).
    Rule(String),
    File(Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NotFullAccess => {
                f.write_str("Only an account with full access can change accounts and privileges.")
            }
            SaveError::Rule(m) => f.write_str(m),
            SaveError::File(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<Error> for SaveError {
    fn from(e: Error) -> Self {
        SaveError::File(e)
    }
}

/// Who may write `new` over `old`: a full access account anything; an
/// account whose set may manage accounts (script steps Add Account, Delete
/// Account, Reset Account Password, Enable Account) only accounts outside
/// [Full Access]; any account its own password (Change Password, checked
/// by [`Security::change_password`]). Privilege sets and extended
/// privileges need full access. The session must be one of `old`'s
/// accounts, with the privilege set it logged in with.
fn authorize(old: &Security, new: &Security, session: &Session) -> Result<(), SaveError> {
    let me = old
        .account(session.account)
        .filter(|a| a.privilege_set == session.set.id)
        .ok_or(SaveError::NotFullAccess)?;
    if me.privilege_set == 1 && session.full_access() {
        return Ok(());
    }
    if old.sets != new.sets || old.extended != new.extended || old.auto_login != new.auto_login {
        return Err(SaveError::NotFullAccess);
    }
    let manage = session.set.allows(flag::MANAGE_ACCOUNTS);
    let ids: BTreeSet<u32> = old
        .accounts
        .iter()
        .chain(&new.accounts)
        .map(|a| a.id)
        .collect();
    for id in ids {
        let (o, n) = (old.account(id), new.account(id));
        if o == n {
            continue;
        }
        let own_password = id == session.account
            && matches!((o, n), (Some(o), Some(n)) if Account {
                password: o.password.clone(),
                stamp: o.stamp,
                must_change: o.must_change,
                ..n.clone()
            } == *o);
        if own_password {
            continue;
        }
        let full = |a: Option<&Account>| a.is_some_and(|a| a.privilege_set == 1);
        if !manage || full(o) || full(n) {
            return Err(SaveError::NotFullAccess);
        }
    }
    Ok(())
}

/// FileMaker's rules for a valid security setup (Manage Security refuses
/// the rest).
fn validate(new: &Security, session: &Session) -> Result<(), SaveError> {
    let rule = |m: &str| Err(SaveError::Rule(m.to_string()));
    if !new
        .accounts
        .iter()
        .any(|a| a.privilege_set == 1 && a.active && a.kind == 0 && !a.is_guest())
    {
        return rule("At least one active account must have the [Full Access] privilege set.");
    }
    let mut names = BTreeSet::new();
    for a in &new.accounts {
        if a.name.trim().is_empty() {
            return rule("An account name cannot be empty.");
        }
        if !names.insert(a.name.to_lowercase()) {
            return rule(&format!("An account named \"{}\" already exists.", a.name));
        }
        if new.set(a.privilege_set).is_none() {
            return rule(&format!("Account \"{}\" has no privilege set.", a.name));
        }
    }
    if new.account(session.account).is_none() {
        return rule("The account you are logged in with cannot be deleted.");
    }
    let mut names = BTreeSet::new();
    for s in &new.sets {
        if s.name.trim().is_empty() || !names.insert(s.name.to_lowercase()) {
            return rule(&format!(
                "Privilege set name \"{}\" is empty or taken.",
                s.name
            ));
        }
    }
    for id in 1..=3 {
        if new.set(id).is_none() {
            return rule("The built-in privilege sets cannot be deleted.");
        }
    }
    Ok(())
}

/// Rebuilds `file` with the accounts, privilege sets and extended
/// privileges of `new`, logged in as `session` (full access required).
/// Changed and new members are rewritten with their digests (key 04),
/// deleted ones removed with their index entries; untouched members keep
/// their bytes. Password data comes from [`Account::new`] and
/// [`Account::set_password`] only.
pub fn save(file: &File, session: &Session, new: &Security) -> Result<Vec<u8>, SaveError> {
    let mut entries = write::entries(file)?;
    let old = parse(&entries);
    authorize(&old, new, session)?;
    validate(new, session)?;
    let mut add: Vec<Entry> = Vec::new();

    // Accounts.
    let olds: BTreeMap<u32, &Account> = old.accounts.iter().map(|a| (a.id, a)).collect();
    let ids: Vec<u32> = new.accounts.iter().map(|a| a.id).collect();
    for o in &old.accounts {
        if !ids.contains(&o.id) {
            drop_member(&mut entries, 1, o.id);
        }
    }
    let stamps = catalog_stamps(&entries, 1);
    for a in &new.accounts {
        let base = [23, 1, 5, a.id];
        match olds.get(&a.id) {
            Some(o) if **o == *a => continue,
            Some(o) => {
                drop_keys(&mut entries, &base, &[2, 3, 4, 6, 0x0A, 0x0B]);
                if o.name != a.name && !a.is_guest() {
                    drop_name(&mut entries, &base);
                    add.extend(value_entries(&spelled(&base), 16, &a.name));
                    rename_index(&mut entries, &mut add, 1, a.id, &a.name);
                }
            }
            None => {
                add.push(marker(&base));
                add.extend(value_entries(&spelled(&base), 16, &a.name));
                add.push(kv_entry(&base, 216, fresh_uuid(a.id as u64)));
                add.extend(stamps.iter().map(|s| with_path(s, &base)));
                add_index(&mut add, 1, a.id, &a.name);
            }
        }
        let mut a = a.clone();
        if a.is_guest() {
            a.raw_name = GUEST_TEXT.to_string();
        } else {
            a.raw_name = a.name.clone();
        }
        if a.kind != 0 {
            add.push(kv_entry(&base, 2, vec![a.kind]));
        }
        if !a.description.is_empty() {
            add.extend(value_entries(&spelled(&base), 3, &a.description));
        }
        add.push(kv_entry(&base, 6, a.password.clone()));
        add.push(kv_entry(&base, 0x0A, a.status().encode()));
        add.push(kv_entry(&base, 0x0B, len_int(a.privilege_set as u64)));
        // The digest of what is written (`new` with the stored name text).
        let mut probe = new.clone();
        probe.internal_creation = old.internal_creation;
        probe.file_access_uuid = old.file_access_uuid.clone();
        add.push(kv_entry(&base, 4, probe.account_digest(&a).to_vec()));
    }
    reindex(
        &mut entries,
        &mut add,
        1,
        &ids,
        &old.accounts.iter().map(|a| a.id).collect::<Vec<_>>(),
    );

    // Privilege sets.
    let olds: BTreeMap<u32, &PrivilegeSet> = old.sets.iter().map(|s| (s.id, s)).collect();
    let sids: Vec<u32> = new.sets.iter().map(|s| s.id).collect();
    for o in &old.sets {
        if !sids.contains(&o.id) {
            drop_member(&mut entries, 2, o.id);
        }
    }
    let stamps = catalog_stamps(&entries, 2);
    let mut digests = old.clone();
    digests.sets = new.sets.clone();
    for s in &new.sets {
        let base = [23, 2, 5, s.id];
        match olds.get(&s.id) {
            Some(o) if **o == *s => continue,
            Some(o) => {
                entries.retain(|e| {
                    let Some(p) = ipath(e) else { return true };
                    if p.len() > 4 && p[..4] == base && matches!(p[4], 5 | 9) {
                        return false;
                    }
                    !(p == base
                        && raw_kv(&e.item).is_some_and(|(k, _)| {
                            matches!(k.as_slice(), [2 | 3 | 4 | 0x0A | 0x0C | 0x0D])
                        }))
                });
                if o.name != s.name {
                    drop_name(&mut entries, &base);
                    add.extend(value_entries(&spelled(&base), 16, &s.name));
                    rename_index(&mut entries, &mut add, 2, s.id, &s.name);
                }
            }
            None => {
                add.push(marker(&base));
                add.extend(value_entries(&spelled(&base), 16, &s.name));
                add.push(kv_entry(&base, 216, fresh_uuid(s.id as u64 + 0x100)));
                add.extend(stamps.iter().map(|x| with_path(x, &base)));
                add_index(&mut add, 2, s.id, &s.name);
            }
        }
        add.extend(set_entries(s, &base));
        add.push(kv_entry(&base, 4, digests.set_digest(s).to_vec()));
    }
    reindex(
        &mut entries,
        &mut add,
        2,
        &sids,
        &old.sets.iter().map(|s| s.id).collect::<Vec<_>>(),
    );
    // The set digest index `[23].[2].[1].[8].[digest]`: one empty key per set.
    entries.retain(|e| {
        !(e.path.len() >= 4
            && ipath(&Entry {
                path: e.path[..4].to_vec(),
                item: vec![],
            })
            .as_deref()
                == Some(&[23, 2, 1, 8])
            && e.path.len() > 4)
    });
    for s in &new.sets {
        let d = match olds.get(&s.id) {
            Some(o) if **o == *s => old.set_digest(o),
            _ => digests.set_digest(s),
        };
        let mut p = spelled(&[23, 2, 1, 8]);
        p.push(d.to_vec());
        add.push(Entry {
            path: p.clone(),
            item: vec![0x80],
        });
        add.push(Entry {
            path: p,
            item: kv_entry(&[], s.id, Vec::new()).item,
        });
    }

    // Extended privileges: set lists (a deleted set leaves every list).
    let ext = members(&entries, &[23, 3, 5]);
    for e in &new.extended {
        let mut e = e.clone();
        e.sets.retain(|s| sids.contains(s));
        let Some(o) = old.extended.iter().find(|o| o.id == e.id) else {
            continue;
        };
        if *o == e {
            continue;
        }
        let base = [23, 3, 5, e.id];
        drop_keys(&mut entries, &base, &[4]);
        entries.retain(|x| ipath(x).is_none_or(|p| !(p.len() == 5 && p[..4] == base && p[4] == 5)));
        if !e.sets.is_empty() {
            add.push(marker(&[23, 3, 5, e.id, 5]));
            for s in &e.sets {
                add.push(kv_entry(&[23, 3, 5, e.id, 5], *s, Vec::new()));
            }
        }
        let sharing = ext.get(&e.id).map_or(2, sharing_byte);
        add.push(kv_entry(
            &base,
            4,
            Security::extended_digest(&e, sharing).to_vec(),
        ));
    }

    // "Log in using" naming an account whose password changed or that is
    // gone no longer logs in: turn it off, as the file then asks.
    if let Some((name, _)) = &old.auto_login
        && name != GUEST_TEXT
    {
        let still = old.account_named(name).and_then(|o| {
            new.accounts
                .iter()
                .find(|a| a.id == o.id && a.password == o.password && a.name == o.name)
        });
        if still.is_none() {
            drop_keys(&mut entries, &[23, 1, 1], &[6, 16]);
            entries.retain(|e| {
                ipath(e).is_none_or(|p| {
                    !(p.len() == 4 && p[..3] == [23, 1, 1] && matches!(p[3], 6 | 16))
                })
            });
            add.push(kv_entry(&[23, 1, 1], 6, Vec::new()));
        }
    }

    entries.extend(add);
    write::sort(&mut entries);
    let bytes = write::build(&file.bytes()[..crate::SECTOR], &entries);
    Ok(bytes)
}

/// Entries of a privilege set's managed keys.
fn set_entries(s: &PrivilegeSet, base: &[u32]) -> Vec<Entry> {
    let mut out = Vec::new();
    if let Some(r) = s.records {
        out.push(kv_entry(base, 2, r.encode()));
    }
    if !s.description.is_empty() {
        out.extend(value_entries(&spelled(base), 3, &s.description));
    }
    out.push(kv_entry(
        base,
        0x0A,
        (s.flags ^ 0x6000).to_be_bytes().to_vec(),
    ));
    if s.expire_days != 0 {
        out.push(kv_entry(base, 0x0C, s.expire_days.to_be_bytes().to_vec()));
    }
    if s.min_password != 0 {
        out.push(kv_entry(base, 0x0D, s.min_password.to_be_bytes().to_vec()));
    }
    let sub = |rest: &[u32]| [base, rest].concat();
    if !s.tables.is_empty() {
        out.push(marker(&sub(&[5])));
    }
    for (t, ta) in &s.tables {
        let tp = sub(&[5, *t]);
        out.push(marker(&tp));
        out.push(kv_entry(&tp, 2, ta.access.encode()));
        if !ta.fields.is_empty() {
            out.push(marker(&sub(&[5, *t, 5])));
            for (f, v) in &ta.fields {
                out.push(kv_entry(&sub(&[5, *t, 5]), *f, vec![*v]));
            }
        }
        for (i, l) in ta.limits.iter().enumerate() {
            if let Some(l) = l {
                let lp = sub(&[5, *t, 6 + i as u32]);
                out.push(marker(&lp));
                if !l.context.is_empty() {
                    out.push(kv_entry(&lp, 4, l.context.clone()));
                }
                out.push(kv_entry(&lp, 5, l.tokens.clone()));
                out.push(kv_entry(&lp, 6, l.compiled.clone()));
            }
        }
    }
    if !s.items.is_empty() {
        out.push(marker(&sub(&[9])));
    }
    for (k, v) in &s.items {
        let item = match k.as_slice() {
            [c] => write::encode(&Item::Kv {
                key: *c as u32,
                data: vec![*v],
            }),
            // ponytail: an item entry's key is the catalog key then the
            // item's key, written as a byte-string key.
            _ => write::encode(&Item::KvLong {
                key: k.clone(),
                data: vec![*v],
            }),
        };
        out.push(Entry {
            path: spelled(&sub(&[9])),
            item,
        });
    }
    out
}

fn with_path(e: &Entry, base: &[u32]) -> Entry {
    Entry {
        path: spelled(base),
        item: e.item.clone(),
    }
}

/// Modification stamps of the catalog index (keys 31873–31875, `FC xx`),
/// copied onto new members as FileMaker does.
fn catalog_stamps(entries: &[Entry], cat: u32) -> Vec<Entry> {
    entries
        .iter()
        .filter(|e| ipath(e).as_deref() == Some(&[23, cat, 1]))
        .filter(|e| raw_kv(&e.item).is_some_and(|(k, _)| k.len() == 2 && k[0] == 0xFC))
        .cloned()
        .collect()
}

fn drop_keys(entries: &mut Vec<Entry>, base: &[u32], keys: &[u8]) {
    entries.retain(|e| {
        !(ipath(e).as_deref() == Some(base)
            && raw_kv(&e.item).is_some_and(|(k, _)| k.len() == 1 && keys.contains(&k[0])))
    });
    // Long text values of those keys.
    entries.retain(|e| {
        ipath(e).is_none_or(|p| {
            !(p.len() == base.len() + 1
                && p.starts_with(base)
                && keys.contains(&(p[base.len()] as u8))
                && p[base.len()] < 0x100)
        })
    });
}

fn drop_name(entries: &mut Vec<Entry>, base: &[u32]) {
    drop_keys(entries, base, &[16]);
}

fn drop_member(entries: &mut Vec<Entry>, cat: u32, id: u32) {
    let idv = len_int(id as u64);
    entries.retain(|e| {
        let Some(p) = ipath(e) else {
            return true;
        };
        if p.starts_with(&[23, cat, 5, id]) || p.starts_with(&[23, cat, 1, 7, id]) {
            return false;
        }
        if p == [23, cat, 1, 7] && raw_kv(&e.item).is_some_and(|(k, _)| int_key(&k) == Some(id)) {
            return false;
        }
        !(p == [23, cat, 1, 1] && raw_kv(&e.item).is_some_and(|(_, v)| v == idv))
    });
}

fn add_index(add: &mut Vec<Entry>, cat: u32, id: u32, name: &str) {
    add.push(kv_entry(&[23, cat, 1, 7], id, Vec::new()));
    if let Some(k) = crate::collate::name_key(name) {
        add.push(Entry {
            path: spelled(&[23, cat, 1, 1]),
            item: write::encode(&Item::KvLong {
                key: k,
                data: len_int(id as u64),
            }),
        });
    }
}

fn rename_index(entries: &mut Vec<Entry>, add: &mut Vec<Entry>, cat: u32, id: u32, name: &str) {
    let idv = len_int(id as u64);
    entries.retain(|e| {
        !(ipath(e).as_deref() == Some(&[23, cat, 1, 1])
            && raw_kv(&e.item).is_some_and(|(_, v)| v == idv))
    });
    if let Some(k) = crate::collate::name_key(name) {
        add.push(Entry {
            path: spelled(&[23, cat, 1, 1]),
            item: write::encode(&Item::KvLong { key: k, data: idv }),
        });
    }
}

/// Display order `[23].[cat].[1].[3]`, count (key 4) and last id (key 0)
/// when the member list changed.
fn reindex(entries: &mut Vec<Entry>, add: &mut Vec<Entry>, cat: u32, ids: &[u32], old: &[u32]) {
    if ids == old {
        return;
    }
    let mut last = 0;
    entries.retain(|e| {
        let p = ipath(e);
        if p.as_deref() == Some(&[23, cat, 1])
            && let Some((k, v)) = raw_kv(&e.item)
            && matches!(k.as_slice(), [0] | [4])
        {
            if k == [0] {
                last = read_len_int(&v).unwrap_or(0) as u32;
            }
            return false;
        }
        !(p.as_deref() == Some(&[23, cat, 1, 3]) && e.item.first() != Some(&0x80))
    });
    if !entries
        .iter()
        .any(|e| ipath(e).as_deref() == Some(&[23, cat, 1, 3]))
    {
        add.push(marker(&[23, cat, 1, 3]));
    }
    for (i, id) in ids.iter().enumerate() {
        add.push(Entry {
            path: spelled(&[23, cat, 1, 3]),
            item: write::encode(&Item::KvLong {
                key: (i as u32 + 1).to_be_bytes().to_vec(),
                data: len_int(*id as u64),
            }),
        });
    }
    let max = ids.iter().copied().max().unwrap_or(0).max(last);
    add.push(kv_entry(&[23, cat, 1], 0, len_int(max as u64)));
    add.push(kv_entry(
        &[23, cat, 1],
        4,
        (ids.len() as u32).to_be_bytes().to_vec(),
    ));
}

/// Calculation of a limited record action, compiled for table occurrence
/// `to` (its context, `03 D0 hi lo` with hi lo = `to` - 128).
pub fn limit(
    src: &str,
    to: u32,
    names: &crate::calc::FileNames,
) -> Result<Limit, crate::calc::CompileError> {
    let c = crate::calc::compile(src, to, names)?;
    let mut context = vec![3, 0xD0];
    context.extend((to.saturating_sub(128) as u16).to_be_bytes());
    Ok(Limit {
        context,
        tokens: c.tokens,
        compiled: c.compiled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_round_trip() {
        for hex in [
            "089000418000635399",
            "089f910380055217 30",
            "079faad480043416",
            "0a9faad4800434166353 99",
            "0a9faad480043659373255",
            "089faad380057834 10",
        ] {
            let b: Vec<u8> = (0..hex.replace(' ', "").len() / 2)
                .map(|i| u8::from_str_radix(&hex.replace(' ', "")[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            let s = Stamp::decode(&b).unwrap();
            assert_eq!(s.encode(), b, "{hex} {s:?}");
        }
        let s = Stamp::decode(&[8, 0x90, 0, 0x41, 0x80, 0, 0x63, 0x53, 0x99]).unwrap();
        assert_eq!(
            (s.year, s.month, s.day, s.seconds, s.micros),
            (1024, 1, 1, 0, 524_288)
        );
    }

    #[test]
    fn check_bytes() {
        let v = password_data("test123", &[7; 10]);
        assert_eq!(v.len(), 31 + (v[12] & 0x1F) as usize);
        assert_eq!(&v[11..31], &password_hash("test123", &[7; 10]));
    }
}
