//! Accounts and privileges (`[23]`): digests, passwords, writing.
//! Template checks are skipped when FileMaker's starter files are not
//! installed (override the directory with FMP12_TEMPLATES).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fmp12::security::{self, Account, Security};

fn blank() -> fmp12::File {
    fmp12::File::from_bytes(include_bytes!("../assets/blank.fmp12").to_vec()).unwrap()
}

fn template_files() -> Vec<PathBuf> {
    let dir = std::env::var_os("FMP12_TEMPLATES")
        .map(PathBuf::from)
        .unwrap_or_else(|| r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates".into());
    let mut out = Vec::new();
    for t in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(t.path()).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|e| e == "fmp12") {
                out.push(f.path());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn blank_decodes() {
    let s = Security::load(&blank()).unwrap();
    let names: Vec<_> = s.accounts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["[Guest]", "Admin"]);
    let guest = &s.accounts[0];
    assert!(!guest.active && guest.privilege_set == 3);
    let admin = &s.accounts[1];
    assert!(admin.active && !admin.must_change && admin.privilege_set == 1);
    assert!(admin.check_password("") && !admin.check_password("x"));
    // [Guest]'s password is its own name text.
    assert!(guest.check_password("[\u{e002}]"));
    let sets: Vec<_> = s.sets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        sets,
        [
            "[Accès intégral]",
            "[Saisie de données uniquement]",
            "[Accès en lecture seule]"
        ]
    );
    let de = &s.sets[1];
    assert!(de.allows(security::flag::PRINT) && de.allows(security::flag::EXPORT));
    assert!(!de.allows(security::flag::MANAGE_DATABASE));
    assert_eq!(de.layout(5), security::VIEW_ONLY);
    assert_eq!(de.layout_records(5), security::MODIFIABLE);
    assert_eq!(de.script(9), security::EXECUTABLE);
    let ro = &s.sets[2];
    assert_eq!(ro.table(130).edit, security::Level::No);
    assert_eq!(ro.table(130).view, security::Level::Yes);
    let kw: Vec<_> = s.extended.iter().map(|e| e.keyword.as_str()).collect();
    assert!(
        kw.contains(&"fmapp") && kw.contains(&"fmwebdirect"),
        "{kw:?}"
    );
    assert_eq!(s.auto_login, Some(("Admin".into(), String::new())));
    assert!(!s.needs_login());
    assert!(Security::check(&blank()).unwrap().is_empty());
}

#[test]
fn template_digests_recompute() {
    let mut n = 0;
    for f in template_files() {
        let file = fmp12::File::open(&f).unwrap();
        let bad = Security::check(&file).unwrap();
        assert!(bad.is_empty(), "{}: {bad:?}", f.display());
        let s = Security::load(&file).unwrap();
        let admin = s.account_named("Admin").unwrap();
        assert!(admin.check_password(""), "{}", f.display());
        n += 1;
    }
    eprintln!("{n} templates: every account, set and extended privilege digest matches");
}

#[test]
fn login_rules() {
    let s = Security::load(&blank()).unwrap();
    assert!(s.login("admin", "").is_ok(), "names ignore case");
    assert_eq!(s.login("Admin", "nope"), Err(security::LoginError::Invalid));
    assert_eq!(s.login("Nobody", ""), Err(security::LoginError::Invalid));
    assert_eq!(s.login_guest(), Err(security::LoginError::Inactive));
    let sess = s.login("Admin", "").unwrap();
    assert_eq!(sess.account_name(), "Admin");
    assert!(sess.full_access());
}

#[test]
fn accounts_round_trip() {
    let file = blank();
    let s = Security::load(&file).unwrap();
    let admin = s.login("Admin", "").unwrap();
    let mut new = s.clone();
    let alice = new.add_account(Account::new("Alice", "test123", 2));
    let mut bob = Account::new("Bob", "test123", 3);
    bob.must_change = true;
    new.add_account(bob);
    let a = new.accounts.iter_mut().find(|a| a.name == "Admin").unwrap();
    a.set_password("adm1n");
    let ro = new.sets[2].clone();
    let mut custom = ro.clone();
    custom.name = "Viewers".into();
    custom.set_flag(security::flag::EXPORT, false);
    let vid = new.add_set(custom);
    new.extended[0].sets.insert(vid);
    let out = security::save(&file, &admin, &new).unwrap();
    let f2 = fmp12::File::from_bytes(out).unwrap();
    assert!(Security::check(&f2).unwrap().is_empty());
    let s2 = Security::load(&f2).unwrap();
    assert_eq!(s2.account(alice).unwrap().name, "Alice");
    assert!(s2.login("alice", "test123").is_ok());
    assert_eq!(
        s2.login("bob", "test123"),
        Err(security::LoginError::MustChange)
    );
    assert!(s2.login("Admin", "").is_err() && s2.login("Admin", "adm1n").is_ok());
    // "Log in using" Admin no longer logs in: the file asks.
    assert!(s2.needs_login());
    let v = s2.set(vid).unwrap();
    assert_eq!(v.name, "Viewers");
    assert!(!v.allows(security::flag::EXPORT));
    assert!(s2.extended.iter().any(|e| e.sets.contains(&vid)));
    // Untouched content keeps its bytes.
    assert_eq!(
        fmp12::schema::tables(&f2).unwrap(),
        fmp12::schema::tables(&file).unwrap()
    );

    // Delete again, as the new admin.
    let admin2 = s2.login("Admin", "adm1n").unwrap();
    let mut back = s2.clone();
    back.accounts.retain(|a| a.id != alice);
    back.sets.retain(|s| s.id != vid);
    let f3 = fmp12::File::from_bytes(security::save(&f2, &admin2, &back).unwrap()).unwrap();
    assert!(Security::check(&f3).unwrap().is_empty());
    let s3 = Security::load(&f3).unwrap();
    assert!(s3.account(alice).is_none() && s3.set(vid).is_none());
    assert!(s3.extended.iter().all(|e| !e.sets.contains(&vid)));
}

#[test]
fn only_full_access_writes() {
    let file = blank();
    let s = Security::load(&file).unwrap();
    let admin = s.login("Admin", "").unwrap();
    let mut new = s.clone();
    new.add_account(Account::new("clerk", "test123", 2));
    let f2 = fmp12::File::from_bytes(security::save(&file, &admin, &new).unwrap()).unwrap();
    let s2 = Security::load(&f2).unwrap();
    let clerk = s2.login("clerk", "test123").unwrap();
    // Without full access (or "manage accounts"): no new account, no
    // change to a set, no other account's password.
    let mut more = s2.clone();
    more.add_account(Account::new("x", "test123", 3));
    let mut set = s2.clone();
    set.sets[2].set_flag(security::flag::PRINT, false);
    let mut other = s2.clone();
    for a in other.accounts.iter_mut().filter(|a| a.name == "Admin") {
        a.set_password("test123");
    }
    for bad in [more, set, other] {
        assert!(matches!(
            security::save(&f2, &clerk, &bad),
            Err(security::SaveError::NotFullAccess)
        ));
    }
    // Its own password (Change Password): allowed.
    let mut own = s2.clone();
    own.change_password("clerk", "test123", "test456").unwrap();
    let f3 = fmp12::File::from_bytes(security::save(&f2, &clerk, &own).unwrap()).unwrap();
    assert!(Security::check(&f3).unwrap().is_empty());
    assert!(
        Security::load(&f3)
            .unwrap()
            .login("clerk", "test456")
            .is_ok()
    );
    // The last full access account cannot go.
    let mut bad = s2.clone();
    bad.accounts.retain(|a| a.name != "Admin");
    assert!(security::save(&f2, &admin_of(&s2), &bad).is_err());
}

fn admin_of(s: &Security) -> security::Session {
    s.login("Admin", "").unwrap()
}

#[test]
fn password_vectors() {
    // Admin of blank.fmp12: the empty password.
    let s = Security::load(&blank()).unwrap();
    assert!(s.account_named("Admin").unwrap().check_password(""));
    // Precomposed and decomposed text hash alike.
    let salt = [1u8; 10];
    assert_eq!(
        security::password_hash("caf\u{e9}", &salt),
        security::password_hash("cafe\u{301}", &salt)
    );

    // Account key 6 as FileMaker Pro 26 wrote it for test passwords set by
    // its own script steps (tools/oracle/account_steps.xml: Add Account,
    // Reset Account Password, Change Password): version 1, salt, hash,
    // then `hash[1] & 0x1F` check bytes.
    let hex = |s: &str| -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    };
    for (password, data) in [
        (
            "test789",
            "018144805194b21fd5ab449adc82493af2f0db77e0b392ceb475d55f4b0b941b980218ae40ef0edca4294e4cfd4f27af907c74a80accacdb95b045",
        ),
        (
            "Test-\u{e9}123",
            "0190ccf619a8cc5d4ea0e61110e8b8bdaa1e276a1ec66c309ff4cc4617bc2681dc1ea115664369caf8d77cd8274966",
        ),
        (
            "test123",
            "011047c37758352c2355f514954a581fcbe458ab9a5614e71cc78f3fc3496604d2892f47fec87bfe6f4281ad44d844db9be2fc52",
        ),
        (
            "test456",
            "01da12b9f01cb391dd85c0659790fb65b7e58bbadb85012e56652d9fa4b204bf85290b790474563f1be096bead009a7a2f08df3a8407",
        ),
    ] {
        let d = hex(data);
        assert_eq!(d[0], 1);
        assert_eq!(
            security::password_hash(password, &d[1..11])[..],
            d[11..31],
            "{password}"
        );
        assert_eq!(d.len(), 31 + (d[12] & 0x1F) as usize, "{password}");
        for i in 31..d.len() {
            assert_eq!(d[i], d[i - 20] ^ d[i - 30], "{password} check byte {i}");
        }
        assert_ne!(security::password_hash("test", &d[1..11])[..], d[11..31]);
    }
}
