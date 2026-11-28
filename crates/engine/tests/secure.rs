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
