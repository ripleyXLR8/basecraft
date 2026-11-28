//! Secure container storage is read next to the file (fixtures:
//! `fmp12/tests/secure.rs`).

const FIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fmp12/tests/fixtures/secure/"
);

#[test]
fn reads_secure_container_files() {
    for (dir, pw) in [("plain", ""), ("ear", "test123")] {
        let path = format!("{FIX}{dir}/sec.fmp12");
        let file = fmp12::File::open_encrypted(&path, pw).unwrap();
        let mut db = engine::Database::load(&file).unwrap();
        db.read_external_containers(std::path::Path::new(&format!("{FIX}{dir}")));
        let t = &db.tables[0];
        let hello = t
            .containers
            .values()
            .find(|s| s.name == "hello.txt")
            .unwrap();
        assert_eq!(
            hello.data(),
            b"Hello secure storage, BaseCraft test file number one.\n"
        );
        assert!(t.containers.values().all(|s| s.external_data.is_some()));
    }
}

/// New files in the secure field are encrypted into the folder FileMaker
/// computes (FORMAT.md §13) and read back after saving.
#[test]
fn writes_secure_container_files() {
    for (dir, pw, flag) in [("plain", "", 1), ("ear", "test123", 2)] {
        let out =
            std::env::temp_dir().join(format!("basecraft-secure-{dir}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let file = fmp12::File::open_encrypted(format!("{FIX}{dir}/sec.fmp12"), pw).unwrap();
        let mut db = engine::Database::load(&file).unwrap();
        let (t, photo) = (db.tables[0].id, db.tables[0].fields[0].id);
        let r = db.new_record(t).unwrap();
        db.insert_file(t, r, photo, 1, "note.txt", b"secure note".to_vec());
        let bytes = db.save(&file).unwrap();
        let written = db.write_external_containers(&out).unwrap();
        assert_eq!(written.len(), 1);
        let note = |db: &engine::Database| {
            db.tables[0]
                .containers
                .values()
                .find(|s| s.name == "note.txt")
                .unwrap()
                .clone()
        };
        let s = note(&db);
        assert_eq!(s.streams[b"REMO"][0], flag);
        let rel = s.secure_path().unwrap();
        assert!(written[0].ends_with(&rel), "{rel}");
        let file = file.replaced(bytes).unwrap();
        let mut db = engine::Database::load(&file).unwrap();
        db.read_external_containers(&out);
        let s = note(&db);
        assert_eq!(s.data(), b"secure note");
        assert_eq!(s.secure_path().unwrap(), rel);
        std::fs::remove_dir_all(&out).unwrap();
    }
}
