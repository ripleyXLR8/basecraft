//! Data file steps, PDF documents, Go to List of Records, Configure
//! Persistent Data: what FileMaker 26 returned for the same steps
//! (`tools/oracle/datafiles`, FORMAT.md §8), on a temporary folder.

use super::*;
use crate::datafiles::*;
use crate::files::{Disk, encode_path_list};

struct FileHost {
    h: TestHost,
    disk: Disk,
    asked: Vec<String>,
}

impl Host for FileHost {
    fn db(&mut self) -> &mut Database {
        &mut self.h.db
    }
    fn window(&mut self) -> &mut Window {
        &mut self.h.win
    }
    fn file_stat(&mut self, path: &str) -> Result<Option<crate::FileStat>, u32> {
        self.disk.stat(path)
    }
    fn file_read(&mut self, path: &str) -> Result<Vec<u8>, u32> {
        self.disk.read(path)
    }
    fn file_write(&mut self, path: &str, data: &[u8], folders: bool) -> Result<(), u32> {
        self.disk.write(path, data, folders)
    }
    fn file_delete(&mut self, path: &str) -> Result<(), u32> {
        self.disk.delete(path)
    }
    fn file_rename(&mut self, from: &str, to: &str) -> Result<(), u32> {
        self.disk.rename(from, to)
    }
    fn file_confirm(&mut self, path: &str) {
        self.asked.push(path.to_string());
        self.disk.confirm(path);
    }
}

/// A fresh folder under the system's temporary folder.
fn folder(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("basecraft-df-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn host(name: &str) -> FileHost {
    FileHost {
        h: TestHost::new(PEOPLE),
        disk: Disk {
            folder: Some(folder(name)),
            confirmed: Vec::new(),
        },
        asked: Vec::new(),
    }
}

fn compile(src: &str) -> Calc {
    text::Names {
        db: &TestHost::new(PEOPLE).db,
        scripts: &[],
    }
    .compile(src)
    .unwrap()
}

/// A step as FileMaker 26 stores it: option bits, bytes 22–23, a path
/// list (`[128]` key 130, option 0x20), a target variable (`[160]`) and
/// calculations at `[128 + i]`.
fn st(
    id: u16,
    opt: u32,
    arg: u16,
    path: Option<&str>,
    target: Option<&str>,
    calcs: &[&str],
) -> Step {
    let mut raw = vec![0u8; 28];
    let mut p = fmp12::scripts::RawParams::default();
    let mut options = opt;
    if let Some(path) = path {
        p.kv.insert((vec![128], 130), encode_path_list(&[path]));
        options |= 0x20;
    }
    if let Some(t) = target {
        p.kv.insert((vec![160], 1), t.bytes().map(|c| c ^ 0x5A).collect());
        raw[6..9].copy_from_slice(&[2, 1, 0]);
        raw[12..15].copy_from_slice(&[2, 1, 0x20]);
        raw[18..20].copy_from_slice(&[0, 1]);
        options |= 0x5;
    }
    for (i, c) in calcs.iter().enumerate() {
        if !c.is_empty() {
            p.kv.insert((vec![128 + i as u32, 5], 5), compile(c).0);
        }
    }
    Step {
        id,
        enabled: true,
        options,
        arg,
        raw,
        raw_params: p,
        params: StepParams::Other,
    }
}

/// Runs one step; its error.
fn run(e: &mut Engine, h: &mut FileHost, s: Step) -> u32 {
    e.run_step(h, &s).last_error
}

fn g(e: &Engine, name: &str) -> String {
    e.globals
        .get(&(name.to_lowercase(), 1))
        .map(|v| v.to_text())
        .unwrap_or_default()
}

#[test]
fn data_files_as_filemaker() {
    let mut h = host("seq");
    let mut e = Engine::new(Vec::new());
    let f = "file:bc.txt";
    // Missing file: Get File Exists 0, Get File Size and Open 100.
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(GET_FILE_EXISTS, 0, 0, Some(f), Some("$$x"), &[])
        ),
        0
    );
    assert_eq!(g(&e, "$$x"), "0");
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(GET_FILE_SIZE, 0, 0, Some(f), Some("$$x"), &[])
        ),
        100
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(OPEN_DATA_FILE, 0, 0, Some(f), Some("$$id"), &[])
        ),
        100
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(CREATE_DATA_FILE, 0, 0, Some(f), None, &[])
        ),
        0
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(GET_FILE_EXISTS, 0, 0, Some(f), Some("$$x"), &[])
        ),
        0
    );
    assert_eq!(g(&e, "$$x"), "1");
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(OPEN_DATA_FILE, 0, 0, Some(f), Some("$$id"), &[])
        ),
        0
    );
    assert_eq!(g(&e, "$$id"), "1");
    // Already open: 300.
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(OPEN_DATA_FILE, 0, 0, Some(f), Some("$$id2"), &[])
        ),
        300
    );
    let pos = |e: &mut Engine, h: &mut FileHost| {
        let s = st(
            GET_DATA_FILE_POSITION,
            0x4000,
            0,
            None,
            Some("$$pos"),
            &["$$id"],
        );
        assert_eq!(run(e, h, s), 0);
        g(e, "$$pos")
    };
    // Write UTF-8 "héllo" + line feed: 7 bytes; UTF-16 "ab": 4 more.
    e.globals.insert(("$$d".into(), 1), Value::text("héllo"));
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(WRITE_DATA_FILE, 0x4200, 2, None, Some("$$d"), &["$$id"])
        ),
        0
    );
    assert_eq!(pos(&mut e, &mut h), "7");
    e.globals.insert(("$$d".into(), 1), Value::text("ab"));
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(WRITE_DATA_FILE, 0x4000, 1, None, Some("$$d"), &["$$id"])
        ),
        0
    );
    assert_eq!(pos(&mut e, &mut h), "11");
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(CLOSE_DATA_FILE, 0x4000, 0, None, None, &["$$id"])
        ),
        0
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(GET_FILE_SIZE, 0, 0, Some(f), Some("$$x"), &[])
        ),
        0
    );
    assert_eq!(g(&e, "$$x"), "11");
    let disk = std::fs::read(h.disk.folder.as_ref().unwrap().join("bc.txt")).unwrap();
    assert_eq!(disk, b"h\xc3\xa9llo\na\0b\0");
    // A new file id; closing twice, unknown ids: 100.
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(CLOSE_DATA_FILE, 0x4000, 0, None, None, &["$$id"])
        ),
        100
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(OPEN_DATA_FILE, 0, 0, Some(f), Some("$$id"), &[])
        ),
        0
    );
    assert_eq!(g(&e, "$$id"), "2");
    // Overwrite "abcdefghij", then positions and counts (sem5).
    std::fs::write(
        h.disk.folder.as_ref().unwrap().join("bc.txt"),
        b"abcdefghij",
    )
    .unwrap();
    let set = |e: &mut Engine, h: &mut FileHost, n: &str| {
        let s = st(SET_DATA_FILE_POSITION, 0xC000, 0, None, None, &["$$id", n]);
        run(e, h, s)
    };
    let read = |e: &mut Engine, h: &mut FileHost, arg: u16, n: Option<&str>| {
        let s = match n {
            Some(n) => st(READ_DATA_FILE, 0xC000, arg, None, Some("$$r"), &["$$id", n]),
            None => st(READ_DATA_FILE, 0x4000, arg, None, Some("$$r"), &["$$id"]),
        };
        let err = run(e, h, s);
        (err, g(e, "$$r"))
    };
    assert_eq!(set(&mut e, &mut h, "3"), 0);
    assert_eq!(read(&mut e, &mut h, 2, Some("2")), (0, "de".into()));
    assert_eq!(pos(&mut e, &mut h), "5");
    assert_eq!(set(&mut e, &mut h, "100"), 0);
    assert_eq!(pos(&mut e, &mut h), "100");
    assert_eq!(set(&mut e, &mut h, "-2"), 14);
    assert_eq!(pos(&mut e, &mut h), "100");
    assert_eq!(set(&mut e, &mut h, "10"), 0);
    assert_eq!(read(&mut e, &mut h, 2, Some("1")), (10, "".into()));
    // No count: the whole file, whatever the position.
    assert_eq!(read(&mut e, &mut h, 2, None), (0, "abcdefghij".into()));
    assert_eq!(set(&mut e, &mut h, "0"), 0);
    assert_eq!(read(&mut e, &mut h, 2, Some("0")), (0, "".into()));
    assert_eq!(
        read(&mut e, &mut h, 2, Some("100")),
        (10, "abcdefghij".into())
    );
    // UTF-16: the count is in characters, little-endian.
    assert_eq!(set(&mut e, &mut h, "0"), 0);
    let (err, r) = read(&mut e, &mut h, 1, Some("2"));
    assert_eq!(
        (err, r.encode_utf16().collect::<Vec<_>>()),
        (0, vec![0x6261, 0x6463])
    );
    // Bytes: a container named after the file.
    assert_eq!(set(&mut e, &mut h, "0"), 0);
    run(
        &mut e,
        &mut h,
        st(READ_DATA_FILE, 0xC000, 3, None, Some("$$r"), &["$$id", "4"]),
    );
    match e.globals.get(&("$$r".into(), 1)) {
        Some(Value::Container(c)) => assert_eq!(
            (c.name.as_str(), c.data.as_slice()),
            ("bc.txt", &b"abcd"[..])
        ),
        v => panic!("{v:?}"),
    }
    // Writes overwrite at the position: "XY" + LF at 2.
    assert_eq!(set(&mut e, &mut h, "2"), 0);
    e.globals.insert(("$$d".into(), 1), Value::text("XY"));
    run(
        &mut e,
        &mut h,
        st(WRITE_DATA_FILE, 0x4200, 2, None, Some("$$d"), &["$$id"]),
    );
    assert_eq!(read(&mut e, &mut h, 2, None), (0, "abXY\nfghij".into()));
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(CLOSE_DATA_FILE, 0x4000, 0, None, None, &["$$id"])
        ),
        0
    );
    // Create Data File empties an existing file.
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(CREATE_DATA_FILE, 0, 0, Some(f), None, &[])
        ),
        0
    );
    run(
        &mut e,
        &mut h,
        st(GET_FILE_SIZE, 0, 0, Some(f), Some("$$x"), &[]),
    );
    assert_eq!(g(&e, "$$x"), "0");
    // Rename, delete; missing files 100.
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(RENAME_FILE, 0x4000, 0, Some(f), None, &["\"bc2.txt\""])
        ),
        0
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(RENAME_FILE, 0x4000, 0, Some(f), None, &["\"bc3.txt\""])
        ),
        100
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(DELETE_FILE, 0, 0, Some("file:bc2.txt"), None, &[])
        ),
        0
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(DELETE_FILE, 0, 0, Some("file:bc2.txt"), None, &[])
        ),
        100
    );
    // A folder is not a file: exists 0, size 100.
    std::fs::create_dir(h.disk.folder.as_ref().unwrap().join("sub")).unwrap();
    run(
        &mut e,
        &mut h,
        st(GET_FILE_EXISTS, 0, 0, Some("file:sub"), Some("$$x"), &[]),
    );
    assert_eq!(g(&e, "$$x"), "0");
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(GET_FILE_SIZE, 0, 0, Some("file:sub"), Some("$$x"), &[])
        ),
        100
    );
    // A path list in a variable: Open takes the first file that exists.
    run(
        &mut e,
        &mut h,
        st(CREATE_DATA_FILE, 0, 0, Some("file:b3.txt"), None, &[]),
    );
    e.globals
        .insert(("$$p".into(), 1), Value::text("file:nope.txt\rfile:b3.txt"));
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(OPEN_DATA_FILE, 0, 0, Some("$$p"), Some("$$id"), &[])
        ),
        0
    );
    assert_eq!(g(&e, "$$id"), "3");
    // A step that finds no file system: 3.
    let mut plain = TestHost::new(PEOPLE);
    let s = st(GET_FILE_EXISTS, 0, 0, Some(f), Some("$$x"), &[]);
    assert_eq!(e.run_step(&mut plain, &s).last_error, 3);
}

#[test]
fn writes_outside_the_folder_ask_first() {
    let mut h = host("ask");
    let outside = folder("ask-out").join("x.txt");
    let fm = format!("/{}", outside.to_string_lossy().replace('\\', "/"));
    let fm = fm.replacen("//", "/", 1);
    let mut e = Engine::new(Vec::new());
    let s = st(CREATE_DATA_FILE, 0, 0, Some(&fm), None, &[]);
    let mut run = e.start_step(&mut h, &s);
    match run.resume(&mut e, &mut h, Answer::Start) {
        Status::Suspended(Suspend::Ui(UiRequest::ConfirmFile { path })) => assert_eq!(path, fm),
        st => panic!("{st:?}"),
    }
    assert!(!outside.exists());
    // Allowed: the step runs again and writes.
    match run.resume(&mut e, &mut h, Answer::Continue) {
        Status::Finished(o) => assert_eq!(o.last_error, 0),
        st => panic!("{st:?}"),
    }
    assert!(outside.exists());
    assert_eq!(h.asked, [fm.clone()]);
    // Refused: error 1, nothing written.
    let other = fm.replace("x.txt", "y.txt");
    let s = st(CREATE_DATA_FILE, 0, 0, Some(&other), None, &[]);
    let mut run = e.start_step(&mut h, &s);
    assert!(matches!(
        run.resume(&mut e, &mut h, Answer::Start),
        Status::Suspended(_)
    ));
    match run.resume(&mut e, &mut h, Answer::Cancel) {
        Status::Finished(o) => assert_eq!(o.last_error, 1),
        st => panic!("{st:?}"),
    }
    assert!(!outside.with_file_name("y.txt").exists());
}

#[test]
fn pdf_documents_as_filemaker() {
    let mut h = host("pdf");
    let mut e = Engine::new(Vec::new());
    let out = "file:out.pdf";
    // Nothing open: Cancel 0, Close 5, Append 829.
    assert_eq!(
        run(&mut e, &mut h, st(CANCEL_PDF, 0, 0, None, None, &[])),
        0
    );
    assert_eq!(
        run(&mut e, &mut h, st(CLOSE_PDF, 0, 0, Some(out), None, &[])),
        5
    );
    assert_eq!(
        run(&mut e, &mut h, st(APPEND_PDF, 0, 0, Some(out), None, &[])),
        829
    );
    assert_eq!(
        run(&mut e, &mut h, st(CREATE_PDF, 0, 0, None, None, &[])),
        0
    );
    assert_eq!(
        run(&mut e, &mut h, st(CREATE_PDF, 0, 0, None, None, &[])),
        833
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(APPEND_PDF, 0, 0, Some("file:none.pdf"), None, &[])
        ),
        100
    );
    // Two documents of 1 and 2 pages.
    let dir = h.disk.folder.clone().unwrap();
    for (name, n) in [("a.pdf", 1), ("b.pdf", 2)] {
        let mut p = print::pdf::Pdf::new();
        for _ in 0..n {
            p.page(612.0, 792.0);
        }
        std::fs::write(dir.join(name), p.finish()).unwrap();
    }
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(APPEND_PDF, 0, 0, Some("file:a.pdf"), None, &[])
        ),
        0
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(APPEND_PDF, 0, 0, Some("file:b.pdf"), None, &[])
        ),
        0
    );
    assert_eq!(
        run(&mut e, &mut h, st(CLOSE_PDF, 0, 0, Some(out), None, &[])),
        0
    );
    let merged = std::fs::read(dir.join("out.pdf")).unwrap();
    assert_eq!(print::merge::pages(&merged), Ok(3));
    // Open PDF, append, cancel: the file stays as it was.
    assert_eq!(
        run(&mut e, &mut h, st(OPEN_PDF, 0, 0, Some(out), None, &[])),
        0
    );
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(APPEND_PDF, 0, 0, Some("file:a.pdf"), None, &[])
        ),
        0
    );
    assert_eq!(
        run(&mut e, &mut h, st(CANCEL_PDF, 0, 0, None, None, &[])),
        0
    );
    assert_eq!(std::fs::read(dir.join("out.pdf")).unwrap(), merged);
    // Print PDF of a missing file: 100.
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(PRINT_PDF, 0x80, 0, Some("file:nope.pdf"), None, &[])
        ),
        100
    );
    // An empty document saves a PDF with no pages.
    run(&mut e, &mut h, st(CREATE_PDF, 0, 0, None, None, &[]));
    assert_eq!(
        run(
            &mut e,
            &mut h,
            st(CLOSE_PDF, 0, 0, Some("file:e.pdf"), None, &[])
        ),
        0
    );
    assert_eq!(
        print::merge::pages(&std::fs::read(dir.join("e.pdf")).unwrap()),
        Ok(0)
    );
}

#[test]
fn list_of_records_persistent_data_and_inserts() {
    let mut h = host("misc");
    let mut e = Engine::new(Vec::new());
    let ids: Vec<u32> = h.h.db.tables[0].records.iter().map(|r| r.id).collect();
    // The list at parameter 7, in its order; ids that match nothing: 101.
    let list = |l: &str| {
        let mut calcs = vec![""; 7];
        calcs.push(l);
        st(GO_TO_LIST_OF_RECORDS, 0x4000, 1, None, None, &calcs)
    };
    let l = format!("\"{}¶{}\"", ids[2], ids[0]);
    assert_eq!(run(&mut e, &mut h, list(&l)), 0);
    assert_eq!(h.h.win.found, [2, 0]);
    let l = format!("\"{}¶999¶{}\"", ids[1], ids[1]);
    assert_eq!(run(&mut e, &mut h, list(&l)), 101);
    assert_eq!(h.h.win.found, [1]);
    assert_eq!(run(&mut e, &mut h, list("\"999\"")), 401);
    assert!(h.h.win.found.is_empty());
    assert_eq!(run(&mut e, &mut h, list("\"\"")), 401);
    // Configure Persistent Data: the name is required.
    let mut s = st(
        CONFIGURE_PERSISTENT_DATA,
        0xC004,
        0,
        None,
        None,
        &["", "\"v1\""],
    );
    assert_eq!(run(&mut e, &mut h, s.clone()), 5);
    s.raw_params
        .kv
        .insert((vec![128], 1), b"key".iter().map(|c| c ^ 0x5A).collect());
    assert_eq!(run(&mut e, &mut h, s), 0);
    assert_eq!(e.more.persistent["key"], "v1");
    // Insert Object is obsolete (4); Insert PDF / Audio-Video with no field 3.
    assert_eq!(run(&mut e, &mut h, Step::new(INSERT_OBJECT)), 4);
    assert_eq!(run(&mut e, &mut h, Step::new(INSERT_PDF)), 3);
    assert_eq!(run(&mut e, &mut h, Step::new(INSERT_AUDIO_VIDEO)), 3);
    // Configure Local Notification / AI Account with no parameters: 5.
    assert_eq!(run(&mut e, &mut h, Step::new(187)), 5);
    assert_eq!(run(&mut e, &mut h, Step::new(212)), 5);
}
