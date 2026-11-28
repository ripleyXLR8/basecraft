//! Encryption at rest. Fixtures: BaseCraft's blank.fmp12 encrypted by
//! FileMaker Server 26's FMDeveloperTool (`--enableEncryption`) with test
//! passphrases: `abc` (hint "hintxyz") and "TeSt Clé" (NFD check).

use fmp12::{Error, File, schema};

const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ear/");
const BLANK: &[u8] = include_bytes!("../assets/blank.fmp12");

fn names(f: &File) -> Result<Vec<String>, Error> {
    Ok(schema::tables(f)?.into_iter().map(|t| t.name).collect())
}

#[test]
fn opens_with_the_encryption_password() {
    let path = format!("{FIX}blank_abc.fmp12");
    assert!(matches!(File::open(&path), Err(Error::Encrypted)));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(fmp12::ear::hint(&bytes).as_deref(), Some("hintxyz"));
    assert!(matches!(
        File::open_encrypted(&path, "abd"),
        Err(Error::EncryptionPassword)
    ));
    let f = File::open_encrypted(&path, "abc").unwrap();
    assert!(f.is_encrypted());
    assert_eq!(&f.bytes()[15..20], b"HBAM7");
    assert!(f.bytes()[48..512].iter().all(|&b| b == 0));
    // Same schema as the file it was encrypted from; every sector parses.
    let plain = File::from_bytes(BLANK.to_vec()).unwrap();
    assert_eq!(names(&f).unwrap(), names(&plain).unwrap());
    f.walk(|_, _| {}).unwrap();
    // Free-byte counts are those of plain 4096-byte sectors.
    for s in f.leaves() {
        let free = u16::from_be_bytes([
            f.bytes()[s.id as usize * 4096 + 14],
            f.bytes()[s.id as usize * 4096 + 15],
        ]);
        assert!(usize::from(free) <= 4076);
    }
    // A plain file opens as usual through the same call.
    assert!(
        !File::from_encrypted_bytes(BLANK.to_vec(), "x")
            .unwrap()
            .is_encrypted()
    );
}

#[test]
fn password_is_case_sensitive_and_decomposed() {
    let path = format!("{FIX}blank_nfd.fmp12");
    // Precomposed é (as typed) and decomposed e + U+0301 both open it.
    assert!(File::open_encrypted(&path, "TeSt Cl\u{e9}").is_ok());
    assert!(File::open_encrypted(&path, "TeSt Cle\u{301}").is_ok());
    assert!(matches!(
        File::open_encrypted(&path, "test cl\u{e9}"),
        Err(Error::EncryptionPassword)
    ));
}

#[test]
fn tampered_page_is_refused() {
    let mut bytes = std::fs::read(format!("{FIX}blank_abc.fmp12")).unwrap();
    bytes[2 * 4096 + 100] ^= 1;
    assert!(matches!(
        File::from_encrypted_bytes(bytes, "abc"),
        Err(Error::Sector { id: 2, .. })
    ));
}

/// Saving an encrypted file writes it encrypted again: same password,
/// cipher, hint and shared id, a new salt and check value, and the same
/// content as the plain image.
#[test]
fn writes_encrypted_files() {
    let path = format!("{FIX}blank_abc.fmp12");
    let orig = std::fs::read(&path).unwrap();
    let f = File::from_encrypted_bytes(orig.clone(), "abc").unwrap();
    let enc = f.encryption().unwrap();
    assert_eq!(enc.hint(), "hintxyz");
    assert_eq!(enc.shared_id(), "bcshared");
    // The header FileMaker wrote is the one a new encryption gets.
    assert_eq!(
        enc,
        &fmp12::ear::Encryption::new("abc", "hintxyz", "bcshared").unwrap()
    );
    let disk = f.to_disk(f.bytes().to_vec()).unwrap();
    assert!(fmp12::ear::is_encrypted(&disk));
    assert_eq!(disk[..250], orig[..250]);
    assert_ne!(disk[250..266], orig[250..266], "new salt");
    assert_eq!(disk[298..4096], orig[298..4096]);
    assert!(matches!(
        File::from_encrypted_bytes(disk.clone(), "abd"),
        Err(Error::EncryptionPassword)
    ));
    let back = File::from_encrypted_bytes(disk.clone(), "abc").unwrap();
    assert_eq!(
        fmp12::write::entries(&back).unwrap(),
        fmp12::write::entries(&f).unwrap()
    );
    // Fresh IVs: writing twice gives different pages.
    let again = f.to_disk(f.bytes().to_vec()).unwrap();
    assert_ne!(again[4096..8192], disk[4096..8192]);
    // An edit keeps the encryption; removing it gives the plain image.
    let mut g = back.replaced(back.bytes().to_vec()).unwrap();
    assert!(g.is_encrypted());
    g.set_encryption(None);
    assert_eq!(g.to_disk(g.bytes().to_vec()).unwrap(), g.bytes());
}

/// Full sectors (the payloads must shrink by 32 bytes) and a password
/// needing NFD.
#[test]
fn encrypts_full_sectors() {
    let mut es = fmp12::write::entries(&File::from_bytes(BLANK.to_vec()).unwrap()).unwrap();
    for i in 0..400u32 {
        es.push(fmp12::write::Entry {
            path: vec![vec![0x7F], vec![0x05]],
            item: fmp12::write::encode(&fmp12::write::Item::KvLong {
                key: i.to_be_bytes().to_vec(),
                data: vec![i as u8; 60],
            }),
        });
    }
    let plain = fmp12::write::build(&BLANK[..4096], &es);
    let enc = fmp12::ear::Encryption::new("Cl\u{e9}", "", "").unwrap();
    let disk = fmp12::ear::encrypt(&plain, &enc).unwrap();
    assert!(disk.len() >= plain.len());
    let back = File::from_encrypted_bytes(disk, "Cle\u{301}").unwrap();
    assert_eq!(fmp12::write::entries(&back).unwrap(), es);
    back.walk(|_, _| {}).unwrap();
}
