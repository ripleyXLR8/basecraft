//! Insert from URL against a server on 127.0.0.1 started by the test (no
//! network), Execute FileMaker Data API, Save Records as JSONL and Save a
//! Copy as XML on the test database.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::datafiles::{FileHost, g, host, run, st};
use super::*;
use crate::net::{DATA_API, INSERT_FROM_URL, SAVE_AS_JSONL, SAVE_AS_XML};

#[derive(Debug, Clone, Default)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(x, _)| x.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

/// Answers one connection per reply (status line, headers, body), in
/// order; returns the base URL and what the server got.
fn serve(
    replies: Vec<(&'static str, &'static str, &'static str)>,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for (status, headers, body) in replies {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let mut seen = Seen {
                method: parts.next().unwrap_or("").into(),
                path: parts.next().unwrap_or("").into(),
                ..Default::default()
            };
            let mut len = 0;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap();
                }
                seen.headers.push((k.trim().into(), v.trim().into()));
            }
            let mut got = vec![0; len];
            r.read_exact(&mut got).unwrap();
            seen.body = String::from_utf8(got).unwrap();
            log.lock().unwrap().push(seen);
            let reply = format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(reply.as_bytes()).unwrap();
        }
    });
    (url, seen)
}

/// Insert from URL as FileMaker 26 stores it: URL at parameter 0, cURL
/// options at 1, select and verify bits, target `$$r`.
fn fetch(url: &str, curl: &str, select: bool) -> Step {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\\\""));
    let opt = 0x1000_0000 | 0x80 | if select { 0x1000 } else { 0 };
    let curl = if curl.is_empty() {
        String::new()
    } else {
        q(curl)
    };
    st(
        INSERT_FROM_URL,
        opt,
        0,
        None,
        Some("$$r"),
        &[&q(url), &curl],
    )
}

fn set_global(e: &mut Engine, name: &str, v: &str) {
    e.globals.insert((name.to_lowercase(), 1), Value::text(v));
}

fn detail(e: &Engine) -> String {
    e.error_detail.clone()
}

#[test]
fn insert_from_url_get_and_dump_header() {
    let (url, seen) = serve(vec![(
        "200 OK",
        "Content-Type: text/plain; charset=utf-8\r\nX-Test: yes\r\n",
        "héllo",
    )]);
    let mut h = host("url-get");
    let mut e = Engine::new(Vec::new());
    let s = fetch(
        &format!("{url}/a b?x=1"),
        r#"--header "X-A: 1" -D $$h -A Tester"#,
        true,
    );
    assert_eq!(run(&mut e, &mut h, s), 0);
    assert_eq!(g(&e, "$$r"), "héllo");
    assert!(
        g(&e, "$$h").starts_with("HTTP/1.1 200 OK\r\n"),
        "{}",
        g(&e, "$$h")
    );
    assert!(g(&e, "$$h").to_lowercase().contains("x-test: yes"));
    let r = seen.lock().unwrap()[0].clone();
    assert_eq!((r.method.as_str(), r.path.as_str()), ("GET", "/a%20b?x=1"));
    assert_eq!(r.header("X-A"), Some("1"));
    assert_eq!(r.header("User-Agent"), Some("Tester"));
}

/// Option `0x100` ("Do not automatically encode URL", FileMaker 26) sends
/// the URL as given.
#[test]
fn insert_from_url_without_encoding() {
    let (url, seen) = serve(vec![("200 OK", "", "a"), ("200 OK", "", "b")]);
    let mut h = host("url-raw");
    let mut e = Engine::new(Vec::new());
    let s = fetch(&format!("{url}/p|q"), "", true);
    assert_eq!(run(&mut e, &mut h, s), 0);
    let mut s = fetch(&format!("{url}/p|q"), "", true);
    s.options |= 0x100;
    assert_eq!(run(&mut e, &mut h, s), 0);
    let paths: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.path.clone())
        .collect();
    assert_eq!(paths, ["/p%7Cq", "/p|q"]);
}

#[test]
fn insert_from_url_post_into_a_field() {
    let (url, seen) = serve(vec![
        ("200 OK", "", "Zed"),
        ("200 OK", "", "!"),
        ("201 Created", "", "{\"ok\":1}"),
    ]);
    let mut h = host("url-post");
    let mut e = Engine::new(Vec::new());
    set_global(&mut e, "$$body", "a=1\r\nb=2");
    // Target People::Name of the first record, select entire contents.
    let into_name = |mut s: Step, select: bool| {
        s.raw_params.kv.remove(&(vec![160], 1));
        s.raw[6..11].copy_from_slice(&[4, 3, 0xD0, 0, (T - 128) as u8]);
        s.raw[12..15].copy_from_slice(&[2, 1, NAME as u8]);
        s.raw[18..20].copy_from_slice(&[0, 1]);
        s.options = (s.options & !0x1004) | 1 | if select { 0x1000 } else { 0 };
        s
    };
    let s = into_name(fetch(&url, "-d @$$body --user ann:pw", true), true);
    assert_eq!(run(&mut e, &mut h, s), 0);
    assert_eq!(h.h.db.tables[0].records[0].values[&NAME], "Zed");
    // Without select: added to the field's text.
    let s = into_name(fetch(&url, "", false), false);
    assert_eq!(run(&mut e, &mut h, s), 0);
    assert_eq!(h.h.db.tables[0].records[0].values[&NAME], "Zed!");
    // -X, --data-binary keeps line ends, a header sets the type.
    let s = fetch(
        &url,
        r#"-X PUT --data-binary @$$body -H "Content-Type: application/json""#,
        true,
    );
    assert_eq!(run(&mut e, &mut h, s), 0);
    assert_eq!(g(&e, "$$r"), "{\"ok\":1}");
    let r = seen.lock().unwrap().clone();
    assert_eq!(r[0].method, "POST");
    assert_eq!(r[0].body, "a=1b=2");
    assert_eq!(
        r[0].header("Content-Type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(r[0].header("Authorization"), Some("Basic YW5uOnB3"));
    assert_eq!(r[1].method, "GET");
    assert_eq!(r[2].method, "PUT");
    assert_eq!(r[2].body, "a=1\r\nb=2");
    assert_eq!(r[2].header("Content-Type"), Some("application/json"));
}

#[test]
fn insert_from_url_errors_as_filemaker() {
    let (url, _) = serve(vec![
        ("401 Unauthorized", "", "no"),
        ("500 Internal Server Error", "", "boom"),
        ("500 Internal Server Error", "", "boom"),
        ("204 No Content", "", ""),
        ("302 Found", "Location: /next\r\n", ""),
        ("200 OK", "", "landed"),
    ]);
    let mut h = host("url-err");
    let mut e = Engine::new(Vec::new());
    // 401: 1627 whatever the options.
    assert_eq!(run(&mut e, &mut h, fetch(&url, "", true)), 1627);
    // 500 without --show-error: no error; with it, 1631 and the code.
    assert_eq!(run(&mut e, &mut h, fetch(&url, "", true)), 0);
    assert_eq!(g(&e, "$$r"), "boom");
    assert_eq!(run(&mut e, &mut h, fetch(&url, "--show-error", true)), 1631);
    assert_eq!(detail(&e), "Response code: 500");
    // No content: 10.
    assert_eq!(run(&mut e, &mut h, fetch(&url, "", true)), 10);
    assert_eq!(detail(&e), "");
    // --location follows the redirect.
    assert_eq!(run(&mut e, &mut h, fetch(&url, "-L", true)), 0);
    assert_eq!(g(&e, "$$r"), "landed");
    // Refused connection: 1631, curl's message only with -S.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dead = format!("http://127.0.0.1:{port}/");
    assert_eq!(run(&mut e, &mut h, fetch(&dead, "", true)), 1631);
    assert_eq!(detail(&e), "");
    assert_eq!(
        run(&mut e, &mut h, fetch(&dead, "-S --connect-timeout 5", true)),
        1631
    );
    assert!(!detail(&e).is_empty());
    // The next step clears the detail.
    assert_eq!(
        run(&mut e, &mut h, fetch("ftp://127.0.0.1/x", "", true)),
        1626
    );
    assert_eq!(detail(&e), "");
    assert_eq!(run(&mut e, &mut h, fetch("no url", "", true)), 1630);
}

/// People with a portal of their Pets on layout 1.
fn data_host(name: &str) -> FileHost {
    let mut h = host(name);
    let db = &mut h.h.db;
    let f = |id: u32, name: &str| {
        let mut x = db.tables[0].fields[0].clone();
        x.id = id;
        x.name = name.into();
        x
    };
    let mut pets = engine::Table::new(131, "Pets", vec![f(1, "Owner"), f(2, "Pet")]);
    for (o, p) in [("Ann", "Cat"), ("Ann", "Dog"), ("Cid", "Fish")] {
        let r = pets.new_record();
        pets.records[r].values = [(1, o.to_string()), (2, p.to_string())].into();
    }
    db.tables.push(pets);
    db.occurrences.push(fmp12::graph::TableOccurrence {
        id: 131,
        name: "Pets".into(),
        base_table: 131,
        ..Default::default()
    });
    db.relationships.push(fmp12::graph::Relationship {
        left: T,
        right: 131,
        predicates: vec![fmp12::graph::Predicate {
            op: fmp12::graph::JoinOp::Equal,
            left_field: NAME,
            right_field: 1,
        }],
        ..Default::default()
    });
    let obj = |id: u64, to: u64, field: u64, parent: Option<u64>| fmp12::layouts::LayoutObject {
        id,
        kind: 1,
        field: Some((0x104000 | to, field, 1)),
        parent,
        ..Default::default()
    };
    let l = &mut db.layouts[0];
    l.objects = vec![
        obj(1, 130, NAME as u64, None),
        obj(2, 130, AGE as u64, None),
        fmp12::layouts::LayoutObject {
            id: 3,
            kind: 9,
            portal: Some((0x104000 | 131, 5, 1)),
            ..Default::default()
        },
        obj(4, 131, 2, Some(3)),
    ];
    h.h.win.host.file_name = "People File".into();
    h
}

fn api(e: &mut Engine, h: &mut FileHost, req: &str) -> (u32, serde_json::Value) {
    set_global(e, "$$req", req);
    let s = st(DATA_API, 0x1000, 0, None, Some("$$out"), &["$$req"]);
    let code = run(e, h, s);
    (code, serde_json::from_str(&g(e, "$$out")).unwrap())
}

#[test]
fn data_api_reads_finds_sorts_and_writes() {
    let mut h = data_host("api");
    let mut e = Engine::new(Vec::new());
    let (code, j) = api(&mut e, &mut h, r#"{"layouts":"People","limit":1}"#);
    assert_eq!(code, 0);
    assert_eq!(
        j.to_string(),
        r#"{"messages":[{"code":"0","message":"OK"}],"response":{"data":[{"fieldData":{"Age":34,"Name":"Ann"},"modId":"0","portalData":{"Pets":[{"Pets::Pet":"Cat","modId":"0","recordId":"1"},{"Pets::Pet":"Dog","modId":"0","recordId":"2"}]},"recordId":"1"}],"dataInfo":{"database":"People File","foundCount":4,"layout":"People","returnedCount":1,"table":"People","totalRecordCount":4}}}"#
    );
    // Find (query as text), omit, sort, offset; portal rows limited.
    let (code, j) = api(
        &mut e,
        &mut h,
        r#"{"layouts":"People","query":"[{\"Age\":\"<40\"},{\"City\":\"Oslo\",\"omit\":\"true\"}]","sort":[{"fieldName":"Name","sortOrder":"descend"}],"offset":1,"limit.Pets":1,"offset.Pets":2}"#,
    );
    assert_eq!(code, 0);
    let names: Vec<&str> = j["response"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["fieldData"]["Name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Dee", "Ann"]);
    assert_eq!(
        j["response"]["data"][1]["portalData"]["Pets"][0]["Pets::Pet"],
        "Dog"
    );
    assert_eq!(j["response"]["dataInfo"]["foundCount"], 2);
    // Related criteria, recordId.
    let (_, j) = api(
        &mut e,
        &mut h,
        r#"{"layouts":"People","query":[{"Pets::Pet":"Fish"}],"portal":[]}"#,
    );
    assert_eq!(j["response"]["data"][0]["fieldData"]["Name"], "Cid");
    assert_eq!(
        j["response"]["data"][0]["portalData"],
        serde_json::json!({})
    );
    let (_, j) = api(&mut e, &mut h, r#"{"layouts":"People","recordId":"2"}"#);
    assert_eq!(j["response"]["data"][0]["fieldData"]["Name"], "Bob");
    // Errors: code and message in the reply, Get ( LastError / LastErrorDetail ).
    for (req, code, msg) in [
        (r#"{"layouts":"Nope"}"#, 105, "Layout is missing"),
        (
            r#"{"layouts":"People","recordId":"99"}"#,
            101,
            "Record is missing",
        ),
        (
            r#"{"layouts":"People","query":[{"Age":"999"}]}"#,
            401,
            "No records match the request",
        ),
        (
            r#"{"layouts":"People","query":[{"Nope":"1"}]}"#,
            102,
            "Field is missing",
        ),
        (
            r#"{"layouts":"People","limit":"x"}"#,
            1708,
            "Parameter value is invalid",
        ),
        ("{not json", 1710, "JSON input is not syntactically valid"),
    ] {
        let (c, j) = api(&mut e, &mut h, req);
        assert_eq!(
            (c, j["messages"][0]["code"].as_str()),
            (code, Some(code.to_string().as_str())),
            "{req}"
        );
        assert_eq!(j["messages"][0]["message"], msg);
        assert_eq!(detail(&e), msg);
    }
    // metaData.
    let (_, j) = api(&mut e, &mut h, r#"{"action":"metaData","tables":""}"#);
    assert_eq!(
        j["response"].to_string(),
        r#"{"tables":[{"baseTable":"People","name":"People"},{"baseTable":"Pets","name":"Pets"}]}"#
    );
    // create, update, duplicate, delete.
    let (c, j) = api(
        &mut e,
        &mut h,
        r#"{"action":"create","layouts":"People","fieldData":{"Name":"Eve","Age":50}}"#,
    );
    assert_eq!(c, 0);
    assert_eq!(j["response"]["recordId"], "5");
    let (c, _) = api(
        &mut e,
        &mut h,
        r#"{"action":"update","layouts":"People","recordId":"5","fieldData":{"City":"Bern"}}"#,
    );
    assert_eq!(c, 0);
    let t = &h.h.db.tables[0];
    assert_eq!(
        (
            t.records[4].values[&NAME].as_str(),
            t.records[4].values[&CITY].as_str()
        ),
        ("Eve", "Bern")
    );
    let (c, j) = api(
        &mut e,
        &mut h,
        r#"{"action":"duplicate","layouts":"People","recordId":"5"}"#,
    );
    assert_eq!((c, j["response"]["recordId"].as_str()), (0, Some("6")));
    let (c, _) = api(
        &mut e,
        &mut h,
        r#"{"action":"delete","layouts":"People","recordId":"1"}"#,
    );
    assert_eq!(c, 0);
    assert_eq!(h.h.db.tables[0].records.len(), 5);
    assert_eq!(h.h.win.found.len(), 3);
}

#[test]
fn save_records_as_jsonl() {
    let mut h = data_host("jsonl");
    let mut e = Engine::new(Vec::new());
    // Off: the completion field (City) inline; the other fields as the user message.
    let mut s = st(SAVE_AS_JSONL, 0, 0, Some("file:out.jsonl"), None, &[]);
    s.raw[6..11].copy_from_slice(&[4, 3, 0xD0, 0, (T - 128) as u8]);
    s.raw[12..15].copy_from_slice(&[2, 1, CITY as u8]);
    s.raw[18..20].copy_from_slice(&[0, 1]);
    s.options |= 1;
    h.h.db.tables[0].records[1].values.remove(&CITY);
    assert_eq!(run(&mut e, &mut h, s), 0);
    let dir = h.disk.folder.clone().unwrap();
    let out = std::fs::read_to_string(dir.join("out.jsonl")).unwrap();
    assert_eq!(
        out,
        "{\"messages\":[{\"content\":\"Name=Ann, Age=34\",\"role\":\"user\"},{\"content\":\"Paris\",\"role\":\"assistant\"}]}\n\
         {\"messages\":[{\"content\":\"Name=Cid, Age=41\",\"role\":\"user\"},{\"content\":\"Rome\",\"role\":\"assistant\"}]}\n\
         {\"messages\":[{\"content\":\"Name=Dee, Age=27\",\"role\":\"user\"},{\"content\":\"Paris\",\"role\":\"assistant\"}]}\n"
    );
    // On: system, user and assistant prompts ([130], [131], [132], as
    // FileMaker 26 writes them) evaluated on each record; [129] is not one.
    let s = st(
        SAVE_AS_JSONL,
        0x1000,
        0,
        Some("file:ft.jsonl"),
        None,
        &[
            "",
            "\"not a prompt\"",
            "\"Be brief\"",
            "People::Name",
            "People::Age",
        ],
    );
    h.h.win.found = vec![2, 0];
    assert_eq!(run(&mut e, &mut h, s), 0);
    let out = std::fs::read_to_string(dir.join("ft.jsonl")).unwrap();
    assert_eq!(
        out,
        "{\"messages\":[{\"content\":\"Be brief\",\"role\":\"system\"},{\"content\":\"Cid\",\"role\":\"user\"},{\"content\":\"41\",\"role\":\"assistant\"}]}\n\
         {\"messages\":[{\"content\":\"Be brief\",\"role\":\"system\"},{\"content\":\"Ann\",\"role\":\"user\"},{\"content\":\"34\",\"role\":\"assistant\"}]}\n"
    );
    assert_eq!(h.h.win.current, 0);
}

#[test]
fn save_a_copy_as_xml() {
    let mut h = data_host("xml");
    let mut e = Engine::new(vec![script(1, vec![var("$x", "1"), plain(id::BEEP)])]);
    let s = st(SAVE_AS_XML, 0, 0, Some("file:copy.xml"), None, &[]);
    assert_eq!(run(&mut e, &mut h, s), 0);
    let dir = h.disk.folder.clone().unwrap();
    let x = std::fs::read_to_string(dir.join("copy.xml")).unwrap();
    for part in [
        "<FMSaveAsXML version=\"2.3.0.0\" Source=\"26.0.1\" File=\"People File.fmp12\"",
        "<BaseTable id=\"130\" comment=\"\" name=\"People\"></BaseTable>",
        "<TableOccurrence id=\"1065090\" name=\"People\" type=\"Local\">",
        "<JoinPredicate type=\"Equal\"><LeftField><FieldReference id=\"1\" name=\"Name\">",
        "<CustomValues><Text><![CDATA[Rome\rParis\rOslo]]></Text></CustomValues>",
        "<Field id=\"2\" name=\"Age\" fieldtype=\"Normal\" datatype=\"Number\" comment=\"\">",
        "<Script id=\"1\" name=\"S1\"><Options>8</Options></Script>",
        "<Layout id=\"1\" name=\"People\" width=\"0\">",
        "<Step index=\"1\" id=\"93\" name=\"Beep\" enable=\"True\"><Options>0</Options></Step>",
    ] {
        assert!(x.contains(part), "{part}\n{x}");
    }
    // Every element closes.
    let opens = x.matches('<').count()
        - x.matches("</").count()
        - x.matches("<?").count()
        - x.matches("<![CDATA[").count();
    assert_eq!(opens, x.matches("</").count());
}

/// A host that runs Insert from URL's request in the background.
struct Background(FileHost);

impl Host for Background {
    fn db(&mut self) -> &mut Database {
        self.0.db()
    }
    fn window(&mut self) -> &mut Window {
        self.0.window()
    }
    fn background_requests(&self) -> bool {
        true
    }
}

/// With a host that asks for it, Insert from URL suspends the run while
/// its request runs on another thread, and the run resumes with the
/// response once it has come.
#[test]
fn insert_from_url_in_the_background() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/x", l.local_addr().unwrap());
    let mut h = Background(host("url-bg"));
    let mut e = Engine::new(Vec::new());
    let mut r = e.start_step(&mut h, &fetch(&url, "", true));
    let st = r.resume(&mut e, &mut h, Answer::Start);
    assert!(
        matches!(&st, Status::Suspended(Suspend::Ui(UiRequest::Fetch { url: u })) if *u == url),
        "{st:?}"
    );
    // The server has not answered yet: the request is still running.
    assert!(!e.fetch_done());
    let (mut s, _) = l.accept().unwrap();
    let mut rd = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    while rd.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
        line.clear();
    }
    s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nlater")
        .unwrap();
    while !e.fetch_done() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let Status::Finished(o) = r.resume(&mut e, &mut h, Answer::Continue) else {
        panic!("still suspended")
    };
    assert_eq!(o.last_error, 0);
    assert_eq!(g(&e, "$$r"), "later");
    assert!(e.fetch_done());
}
