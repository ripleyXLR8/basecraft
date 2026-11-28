//! Secure container storage. Fixtures: a BaseCraft file with an open
//! storage container field (hello.txt, a 4 × 3 red PNG, 771 bytes of
//! 00..FF then `xyz`), encrypted by FileMaker Server 26's FMDeveloperTool
//! `--enableEncryption` with test password `test123` (which moves open
//! storage to secure storage: `ear/`), then `--removeEncryption` (`plain/`).
#![allow(clippy::unwrap_used)]

use fmp12::containers::{Storage, read, secure_files};
use md5::Digest;

const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/secure/");

/// MD5s of the three files as inserted (open storage, before encryption).
const FILES: [(&str, u64, &str); 3] = [
    ("hello.txt", 54, "c34789f5e91b9a04daf1214cbf6e59b0"),
    ("red.png", 73, "9a866eecd60b4fb878738e16d4053441"),
    ("rand.bin", 771, "87daa51868fc838745ddada9d6ed6201"),
];

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn check(dir: &str, file: fmp12::File) {
    let found = secure_files(std::path::Path::new(&format!(
        "{FIX}{dir}/Files/sec/Secure"
    )));
    assert_eq!(found.len(), 3);
    let stores = read(&file, 129).unwrap();
    // Key 4 of each store entry (the dedup MD5).
    let mut key4 = std::collections::BTreeMap::new();
    file.walk(|path, c| {
        if let ([t, s, five, id], fmp12::Chunk::Kv { key: 4, data }) = (path, c)
            && (
                fmp12::path_int(t),
                fmp12::path_int(s),
                fmp12::path_int(five),
            ) == (Some(129), Some(31), Some(5))
        {
            key4.insert(fmp12::path_int(id).unwrap(), data.to_vec());
        }
    })
    .unwrap();
    let mut names = Vec::new();
    for (id, s) in &stores {
        let mut with = s.clone();
        with.external_data = s
            .external()
            .and_then(|x| s.decrypt_secure(&std::fs::read(&found[&x.md5]).ok()?));
        // As for open storage, computed as if embedded (`ENCR` left out).
        assert_eq!(with.hash().to_vec(), key4[id]);
        let x = s.external().unwrap();
        assert!(matches!(x.storage, Storage::Secure { .. }));
        assert_eq!(x.file, "_129");
        let raw = std::fs::read(&found[&x.md5]).unwrap();
        let data = s.decrypt_secure(&raw).unwrap();
        let (name, size, md5) = FILES.iter().find(|f| f.0 == s.name).unwrap();
        assert_eq!((data.len() as u64, x.size), (*size, *size));
        assert_eq!(hex(&md5::Md5::digest(&data)), *md5);
        // PKCS #7: the file is the content padded to the next block.
        assert_eq!(raw.len(), (data.len() / 16 + 1) * 16);
        // The folder FileMaker chose, and the very bytes it wrote.
        let rel = found[&x.md5]
            .strip_prefix(format!("{FIX}{dir}/Files/sec/"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(with.secure_path().unwrap(), rel);
        assert_eq!(with.encrypt_secure().unwrap(), raw);
        names.push(name.to_string());
    }
    names.sort();
    assert_eq!(names, ["hello.txt", "rand.bin", "red.png"]);
}

#[test]
fn reads_secure_storage() {
    check(
        "plain",
        fmp12::File::open(format!("{FIX}plain/sec.fmp12")).unwrap(),
    );
}

#[test]
fn reads_secure_storage_with_encryption_at_rest() {
    check(
        "ear",
        fmp12::File::open_encrypted(format!("{FIX}ear/sec.fmp12"), "test123").unwrap(),
    );
}

#[test]
fn new_secure_round_trips() {
    for (at_rest, fewer) in [(false, false), (true, false), (false, true)] {
        let s = fmp12::containers::Stored::new_secure("a.txt", 130, fewer, at_rest, b"hi".to_vec())
            .unwrap();
        let x = s.external().unwrap();
        assert_eq!(
            x.storage,
            Storage::Secure {
                fewer_folders: fewer
            }
        );
        assert_eq!((x.file.as_str(), x.size), ("_130", 2));
        let enc = s.encrypt_secure().unwrap();
        assert_eq!(enc.len(), 16);
        assert_eq!(x.md5, <[u8; 16]>::from(md5::Md5::digest(&enc)));
        assert_eq!(s.decrypt_secure(&enc).unwrap(), b"hi");
        let p = s.secure_path().unwrap();
        let parts: Vec<_> = p.split('/').map(str::len).collect();
        let want: &[usize] = if fewer {
            &[6, 1, 28, 8]
        } else {
            &[6, 2, 2, 8, 8, 8, 8]
        };
        assert_eq!(parts, want, "{p}");
        assert_eq!(
            s.streams[b"REMO"][0],
            [1, 2][at_rest as usize] | [0, 8][fewer as usize]
        );
    }
}
