//! Against a real FileMaker Server hosting `BCRemote` as
//! `tools/oracle/remote_fixture.sh` builds it (the VM of
//! tools/oracle/README.md). Skipped unless `BASECRAFT_FMS_URL`
//! (`https://host:port`), `BASECRAFT_FMS_ACCOUNT` and
//! `BASECRAFT_FMS_PASSWORD` are set; `BASECRAFT_FMS_INSECURE=1` accepts
//! the VM's self-signed certificate.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use remote::{Client, FindRequest, Kind, Options, Query, Target};

fn connect() -> Option<(Target, String, String, Options)> {
    let url = std::env::var("BASECRAFT_FMS_URL").ok()?;
    let account = std::env::var("BASECRAFT_FMS_ACCOUNT").ok()?;
    let password = std::env::var("BASECRAFT_FMS_PASSWORD").ok()?;
    let options = Options {
        insecure: std::env::var("BASECRAFT_FMS_INSECURE").is_ok_and(|v| v == "1"),
    };
    let target = Target::parse(&format!("{}/BCRemote", url.trim_end_matches('/'))).unwrap();
    Some((target, account, password, options))
}

#[test]
fn hosted_file_round_trip() {
    let Some((target, account, password, options)) = connect() else {
        eprintln!("BASECRAFT_FMS_URL/ACCOUNT/PASSWORD not set: skipped");
        return;
    };
    // Certificate checks are on by default: the VM's self-signed one fails.
    if options.insecure {
        let strict = Client::login(&target, &account, &password, Options::default());
        assert!(
            matches!(strict, Err(remote::Error::Transport(_))),
            "{strict:?}"
        );
    }
    let wrong = Client::login(&target, &account, "wrong password", options).unwrap_err();
    assert_eq!(wrong.kind(), Kind::Login, "{wrong}");

    let dbs = remote::databases(&target.host, &account, &password, options).unwrap();
    assert!(dbs.iter().any(|d| d == "BCRemote"), "{dbs:?}");
    let c = Client::login(&target, &account, &password, options).unwrap();
    assert!(c.layouts().unwrap().contains(&"Contacts".to_string()));
    let meta = c.layout("Contacts").unwrap();
    let names: Vec<&str> = meta.fields.iter().map(|f| f.name.as_str()).collect();
    for f in ["Name", "City", "Age", "Photo"] {
        assert!(names.contains(&f), "{names:?}");
    }

    let sorted = c
        .records(
            "Contacts",
            &Query {
                sort: vec![("Age".into(), true)],
                ..Default::default()
            },
        )
        .unwrap();
    assert!(sorted.found >= 3);
    assert_eq!(sorted.records[0].fields["Name"], "Grace");
    let page = c
        .records(
            "Contacts",
            &Query {
                offset: 2,
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.records.len(), 1);

    // Find: Age < 50, omitting Linus.
    let found = c
        .find(
            "Contacts",
            &[
                FindRequest {
                    criteria: vec![("Age".into(), "<50".into())],
                    omit: false,
                },
                FindRequest {
                    criteria: vec![("Name".into(), "==Linus".into())],
                    omit: true,
                },
            ],
            &Query::default(),
        )
        .unwrap();
    let names: Vec<&str> = found
        .records
        .iter()
        .map(|r| r.fields["Name"].as_str())
        .collect();
    assert!(
        names.contains(&"Ada") && !names.contains(&"Linus"),
        "{names:?}"
    );
    let none = c
        .find(
            "Contacts",
            &[FindRequest {
                criteria: vec![("Age".into(), ">1000".into())],
                omit: false,
            }],
            &Query::default(),
        )
        .unwrap();
    assert!(none.records.is_empty());

    // Create, edit with modId, a stale modId, delete.
    let (id, m0) = c
        .create(
            "Contacts",
            &[
                ("Name".into(), "BC test".into()),
                ("Age".into(), "1".into()),
            ],
        )
        .unwrap();
    let m1 = c
        .edit(
            "Contacts",
            id,
            &[("City".into(), "Remote".into())],
            Some(m0),
        )
        .unwrap();
    assert!(m1 > m0);
    let stale = c
        .edit("Contacts", id, &[("City".into(), "Stale".into())], Some(m0))
        .unwrap_err();
    assert_eq!(stale.code(), Some(306));
    assert_eq!(c.record("Contacts", id).unwrap().fields["City"], "Remote");
    c.delete("Contacts", id).unwrap();
    assert_eq!(c.record("Contacts", id).unwrap_err().code(), Some(101));

    // Scripts on the server.
    if c.scripts().unwrap().contains(&"Echo".to_string()) {
        let r = c.run_script("Contacts", "Echo", Some("hi é")).unwrap();
        assert_eq!((r.error, r.result.as_deref()), (0, Some("echo:hi é")));
    }
    assert_eq!(
        c.run_script("Contacts", "No such script", None)
            .unwrap_err()
            .code(),
        Some(104)
    );

    // A container, if one was uploaded to record 1 (README: VM section).
    let ada = c.record("Contacts", 1).unwrap();
    if let Some(url) = ada.fields.get("Photo").filter(|u| !u.is_empty()) {
        let data = c.container(url).unwrap();
        assert!(data.starts_with(b"\x89PNG"), "{} bytes", data.len());
    }
    c.logout().unwrap();

    // The engine view: every layout, its records.
    let c = Client::login(&target, &account, &password, options).unwrap();
    let (_remote, db) = remote::source::Remote::open(c).unwrap();
    let t = db.tables.iter().find(|t| t.name == "Contacts").unwrap();
    assert!(t.records.len() >= 3);
}
