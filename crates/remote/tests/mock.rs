//! The Data API client against a mock server that replays replies
//! recorded from FileMaker Server 26 (the BCRemote test file of the VM,
//! tools/oracle/README.md) and checks the requests it gets.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use remote::{Client, FindRequest, Kind, Options, Query, Target};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    auth: String,
    body: String,
}

/// Serves `routes` (method, path prefix, status, body, extra headers) on
/// 127.0.0.1; returns its URL and the requests it got.
fn serve(
    routes: Vec<(&'static str, &'static str, u16, String)>,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let (mut len, mut auth) = (0, String::new());
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => len = v.trim().parse().unwrap(),
                    "authorization" => auth = v.trim().to_string(),
                    _ => {}
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            log.lock().unwrap().push(Seen {
                method: method.clone(),
                path: path.clone(),
                auth,
                body: String::from_utf8(body).unwrap(),
            });
            let (status, reply) = routes
                .iter()
                .find(|(m, p, _, _)| *m == method && path.starts_with(p))
                .map_or((404, "{}".to_string()), |(_, _, s, b)| (*s, b.clone()));
            let head = if reply.starts_with("REDIRECT ") {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: {}\r\nSet-Cookie: X-FMS-Session-Key=abc; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    &reply[9..]
                )
            } else {
                format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
            };
            s.write_all(head.as_bytes()).unwrap();
        }
    });
    (url, seen)
}

fn ok(response: &str) -> String {
    format!(r#"{{"response":{response},"messages":[{{"code":"0","message":"OK"}}]}}"#)
}

fn error(code: u32, message: &str) -> String {
    format!(r#"{{"messages":[{{"code":"{code}","message":"{message}"}}],"response":{{}}}}"#)
}

fn login_route() -> (&'static str, &'static str, u16, String) {
    (
        "POST",
        "/fmi/data/vLatest/databases/BC%20Remote/sessions",
        200,
        ok(r#"{"token":"tok123"}"#),
    )
}

fn target(url: &str) -> Target {
    Target::parse(&format!("{url}/BC%20Remote")).unwrap()
}

#[test]
fn login_layouts_records_portals() {
    let (url, seen) = serve(vec![
        login_route(),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records",
            200,
            ok(
                r#"{"dataInfo":{"database":"BCRemote","layout":"Contacts","table":"Contacts","totalRecordCount":3,"foundCount":3,"returnedCount":2},"data":[{"fieldData":{"Age":85,"City":"Arlington","Name":"Grace"},"portalData":{"Pets":[{"recordId":"7","Pets::Name":"Rex","modId":"2"}]},"recordId":"2","modId":"0"},{"fieldData":{"Age":29,"City":"Helsinki","Name":"Linus"},"portalData":{},"recordId":"3","modId":"4"}]}"#,
            ),
        ),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts",
            200,
            ok(
                r#"{"fieldMetaData":[{"name":"Age","type":"normal","displayType":"editText","result":"number","global":false,"autoEnter":false,"fourDigitYear":false,"maxRepeat":1,"maxCharacters":0,"notEmpty":false,"numeric":false,"timeOfDay":false,"repetitionStart":1,"repetitionEnd":1},{"name":"Size","type":"normal","result":"text","global":false,"maxRepeat":1,"valueList":"Sizes"}],"portalMetaData":{"Pets":[{"name":"Pets::Name","type":"normal","result":"text","global":false,"maxRepeat":1}]},"valueLists":[{"name":"Sizes","type":"customList","values":[{"value":"S","displayValue":"S"},{"value":"M","displayValue":"M"}]}]}"#,
            ),
        ),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts",
            200,
            ok(
                r#"{"layouts":[{"name":"Contacts","table":"Contacts"},{"name":"Folder","isFolder":true,"folderLayoutNames":[{"name":"Inner","table":"T"}]}]}"#,
            ),
        ),
        (
            "DELETE",
            "/fmi/data/vLatest/databases/BC%20Remote/sessions/tok123",
            200,
            ok("{}"),
        ),
    ]);
    let c = Client::login(&target(&url), "api", "test123", Options::default()).unwrap();
    assert_eq!(c.layouts().unwrap(), ["Contacts", "Inner"]);
    let meta = c.layout("Contacts").unwrap();
    assert_eq!(meta.fields[0].result, "number");
    assert_eq!(meta.fields[1].value_list.as_deref(), Some("Sizes"));
    assert_eq!(meta.portals["Pets"][0].name, "Pets::Name");
    assert_eq!(meta.value_lists["Sizes"], ["S", "M"]);
    let page = c
        .records(
            "Contacts",
            &Query {
                offset: 2,
                limit: Some(2),
                sort: vec![("Age".into(), true)],
                portals: vec!["Pets".into()],
            },
        )
        .unwrap();
    assert_eq!((page.total, page.found, page.records.len()), (3, 3, 2));
    let g = &page.records[0];
    assert_eq!((g.id, g.mod_id), (2, 0));
    assert_eq!(g.fields["Age"], "85");
    assert_eq!(g.fields["Name"], "Grace");
    let pet = &g.portals["Pets"][0];
    assert_eq!((pet.id, pet.mod_id), (7, 2));
    assert_eq!(pet.fields["Pets::Name"], "Rex");
    assert!(!pet.fields.contains_key("recordId"));
    c.logout().unwrap();

    let seen = seen.lock().unwrap().clone();
    // Basic auth for the login only ("api:test123"), the token after.
    assert_eq!(seen[0].auth, "Basic YXBpOnRlc3QxMjM=");
    assert!(
        seen[1..].iter().all(|s| s.auth == "Bearer tok123"),
        "{seen:?}"
    );
    let rec = seen.iter().find(|s| s.path.contains("/records")).unwrap();
    assert!(rec.path.ends_with(
        "?_offset=2&_limit=2&_sort=%5B%7B%22fieldName%22%3A%22Age%22%2C%22sortOrder%22%3A%22descend%22%7D%5D&portal=%5B%22Pets%22%5D"
    ), "{}", rec.path);
    assert_eq!(seen.last().unwrap().method, "DELETE");
}

#[test]
fn find_edit_delete_scripts_errors() {
    let (url, seen) = serve(vec![
        login_route(),
        (
            "POST",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/_find",
            200,
            ok(
                r#"{"dataInfo":{"database":"BCRemote","layout":"Contacts","table":"Contacts","totalRecordCount":3,"foundCount":1,"returnedCount":1},"data":[{"fieldData":{"Age":36,"City":"London","Name":"Ada"},"portalData":{},"recordId":"1","modId":"0"}]}"#,
            ),
        ),
        (
            "PATCH",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records/1",
            200,
            ok(r#"{"modId":"1"}"#),
        ),
        (
            "PATCH",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records/2",
            500,
            error(306, "Record modification ID does not match"),
        ),
        (
            "POST",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records",
            200,
            ok(r#"{"recordId":"4","modId":"0"}"#),
        ),
        (
            "DELETE",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records/99",
            500,
            error(101, "Record is missing"),
        ),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/script/Echo",
            200,
            ok(r#"{"scriptResult":"echo:hi there","scriptError":"0"}"#),
        ),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Nope",
            500,
            error(105, "Layout is missing"),
        ),
        (
            "DELETE",
            "/fmi/data/vLatest/databases/BC%20Remote/sessions/",
            200,
            ok("{}"),
        ),
    ]);
    let c = Client::login(&target(&url), "api", "test123", Options::default()).unwrap();
    let page = c
        .find(
            "Contacts",
            &[
                FindRequest {
                    criteria: vec![("City".into(), "L*".into()), ("Age".into(), "<50".into())],
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
    assert_eq!(page.records[0].fields["Name"], "Ada");
    assert_eq!(
        c.edit("Contacts", 1, &[("City".into(), "Paris".into())], Some(0))
            .unwrap(),
        1
    );
    let e = c
        .edit("Contacts", 2, &[("City".into(), "Paris".into())], Some(0))
        .unwrap_err();
    assert_eq!((e.code(), e.kind()), (Some(306), Kind::Conflict));
    assert_eq!(
        c.create("Contacts", &[("Name".into(), "Alan".into())])
            .unwrap(),
        (4, 0)
    );
    let e = c.delete("Contacts", 99).unwrap_err();
    assert_eq!(e.kind(), Kind::Missing);
    let r = c.run_script("Contacts", "Echo", Some("hi there")).unwrap();
    assert_eq!((r.error, r.result.as_deref()), (0, Some("echo:hi there")));
    assert_eq!(c.layout("Nope").unwrap_err().code(), Some(105));
    drop(c); // logs out

    let seen = seen.lock().unwrap().clone();
    let find = seen.iter().find(|s| s.path.ends_with("/_find")).unwrap();
    let body: serde_json::Value = serde_json::from_str(&find.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"query": [{"City": "L*", "Age": "<50"}, {"Name": "==Linus", "omit": "true"}]})
    );
    let patch = seen.iter().find(|s| s.method == "PATCH").unwrap();
    let body: serde_json::Value = serde_json::from_str(&patch.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"fieldData": {"City": "Paris"}, "modId": "0"})
    );
    let script = seen.iter().find(|s| s.path.contains("/script/")).unwrap();
    assert!(
        script
            .path
            .ends_with("/script/Echo?script.param=hi%20there")
    );
    assert_eq!(seen.last().unwrap().method, "DELETE");
    assert!(seen.last().unwrap().path.ends_with("/sessions/tok123"));
}

#[test]
fn no_match_login_and_no_access() {
    let (url, _) = serve(vec![
        login_route(),
        (
            "POST",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/_find",
            500,
            r#"{"response":{"dataInfo":{"database":"BCRemote","layout":"Contacts","table":"Contacts","totalRecordCount":3,"foundCount":0,"returnedCount":0},"data":[]},"messages":[{"code":"401","message":"No records match the request"}]}"#.into(),
        ),
        ("DELETE", "/fmi/data/vLatest/databases/BC%20Remote/sessions/", 200, ok("{}")),
    ]);
    let c = Client::login(&target(&url), "api", "test123", Options::default()).unwrap();
    let found = c
        .find(
            "Contacts",
            &[FindRequest {
                criteria: vec![("Age".into(), ">100".into())],
                omit: false,
            }],
            &Query::default(),
        )
        .unwrap();
    assert!(found.records.is_empty());

    // Replies FileMaker Server 26 gave: a wrong password, an account whose
    // set lacks fmrest, a file no set of which has fmrest.
    for (code, msg, kind) in [
        (
            212,
            "Invalid user account and/or password; please try again",
            Kind::Login,
        ),
        (9, "Insufficient privileges", Kind::Denied),
        (802, "Unable to open file", Kind::Missing),
    ] {
        let (url, _) = serve(vec![(
            "POST",
            "/fmi/data/vLatest/databases/BC%20Remote/sessions",
            401,
            error(code, msg),
        )]);
        let e = Client::login(&target(&url), "api", "nope", Options::default()).unwrap_err();
        assert_eq!((e.code(), e.kind()), (Some(code), kind));
        // The message never echoes the password.
        assert!(!e.to_string().contains("nope"));
    }
}

#[test]
fn container_follows_the_cookie_redirect() {
    let (url, seen) = serve(vec![
        login_route(),
        (
            "GET",
            "/Streaming_SSL/MainDB/ABC.png?RCType=EmbeddedRCFileProcessor",
            200,
            "PNGDATA".into(),
        ),
        (
            "GET",
            "/Streaming_SSL/MainDB/ABC.png",
            302,
            "REDIRECT /Streaming_SSL/MainDB/ABC.png?RCType=EmbeddedRCFileProcessor".into(),
        ),
        (
            "DELETE",
            "/fmi/data/vLatest/databases/BC%20Remote/sessions/",
            200,
            ok("{}"),
        ),
    ]);
    let c = Client::login(&target(&url), "api", "test123", Options::default()).unwrap();
    let data = c
        .container("https://127.0.0.1:443/Streaming_SSL/MainDB/ABC.png")
        .unwrap();
    assert_eq!(data, b"PNGDATA");
    assert!(c.container("https://elsewhere.example/x.png").is_err());
    drop(c);
    assert_eq!(seen.lock().unwrap().len(), 4);
}

/// The engine view of a hosted file: Browse/Find/sort work on the loaded
/// records, and `sync` sends creates, edits (with modId) and deletes.
#[test]
fn source_loads_and_syncs() {
    let (url, seen) = serve(vec![
        login_route(),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records",
            200,
            ok(
                r#"{"dataInfo":{"database":"BCRemote","layout":"Contacts","table":"Contacts","totalRecordCount":3,"foundCount":3,"returnedCount":3},"data":[{"fieldData":{"Age":36,"City":"London","Name":"Ada","Total":1},"portalData":{},"recordId":"1","modId":"0"},{"fieldData":{"Age":85,"City":"Arlington","Name":"Grace","Total":1},"portalData":{},"recordId":"2","modId":"3"},{"fieldData":{"Age":29,"City":"Helsinki","Name":"Linus","Total":1},"portalData":{},"recordId":"3","modId":"0"}]}"#,
            ),
        ),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts",
            200,
            ok(
                r#"{"fieldMetaData":[{"name":"Age","type":"normal","result":"number","global":false,"maxRepeat":1},{"name":"City","type":"normal","result":"text","global":false,"maxRepeat":1},{"name":"Name","type":"normal","result":"text","global":false,"maxRepeat":1},{"name":"Total","type":"calculation","result":"number","global":false,"maxRepeat":1}],"portalMetaData":{}}"#,
            ),
        ),
        (
            "GET",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts",
            200,
            ok(r#"{"layouts":[{"name":"Contacts","table":"Contacts"}]}"#),
        ),
        (
            "PATCH",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records/2",
            200,
            ok(r#"{"modId":"4"}"#),
        ),
        (
            "POST",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records",
            200,
            ok(r#"{"recordId":"9","modId":"0"}"#),
        ),
        (
            "DELETE",
            "/fmi/data/vLatest/databases/BC%20Remote/layouts/Contacts/records/3",
            200,
            ok("{}"),
        ),
        (
            "DELETE",
            "/fmi/data/vLatest/databases/BC%20Remote/sessions/",
            200,
            ok("{}"),
        ),
    ]);
    let c = Client::login(&target(&url), "api", "test123", Options::default()).unwrap();
    let (mut remote, mut db) = remote::source::Remote::open(c).unwrap();
    let t = db.tables[0].id;
    assert_eq!(db.tables[0].records.len(), 3);
    assert_eq!(db.layouts[0].name, "Contacts");
    // Find and sort run on the engine, with FileMaker's semantics.
    let age = db.tables[0].field_by_name("Age").unwrap().id;
    let city = db.tables[0].field_by_name("City").unwrap().id;
    let found = engine::find(
        &db,
        t,
        &[engine::Request {
            criteria: vec![(age, "<50".into())],
            omit: false,
        }],
    );
    assert_eq!(found.len(), 2);
    let mut all: Vec<usize> = (0..3).collect();
    engine::sort(&db, t, &mut all, &[(age, engine::Order::Descending)]);
    assert_eq!(db.value(t, all[0], age).to_text(), "85");

    // Edit Grace, add Alan, delete Linus; the calculation is not sent.
    db.set_values(t, 1, &[(city, "Washington".into())]);
    let total = db.tables[0].field_by_name("Total").unwrap().id;
    db.tables[0].records[1].values.insert(total, "7".into());
    let r = db.new_record(t).unwrap();
    db.set_values(
        t,
        r,
        &[(
            db.tables[0].field_by_name("Name").unwrap().id,
            "Alan".into(),
        )],
    );
    db.delete_record(t, 2);
    assert!(remote.sync(&db).is_empty());
    // Nothing more to send.
    assert!(remote.sync(&db).is_empty());
    drop(remote);

    let seen = seen.lock().unwrap().clone();
    let writes: Vec<(String, String, String)> = seen
        .iter()
        .filter(|s| s.method != "GET" && !s.path.contains("/sessions"))
        .map(|s| {
            (
                s.method.clone(),
                s.path.rsplit('/').next().unwrap().to_string(),
                s.body.clone(),
            )
        })
        .collect();
    assert_eq!(
        writes,
        [
            ("DELETE".into(), "3".into(), String::new()),
            (
                "PATCH".into(),
                "2".into(),
                r#"{"fieldData":{"City":"Washington"},"modId":"3"}"#.into()
            ),
            (
                "POST".into(),
                "records".into(),
                r#"{"fieldData":{"Name":"Alan"}}"#.into()
            ),
        ]
    );
}
