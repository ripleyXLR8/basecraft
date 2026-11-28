//! Record storage, checked against files FileMaker Pro 26 wrote.
//!
//! `fixtures/local/inventory_records.fmp12` is a copy of the Inventory
//! starter template (Claris content, so never committed) where we typed
//! records in FileMaker. Tests skip when it is absent.

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fmp12::{File, save, schema, write};

fn local(name: &str) -> Option<File> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/local")
        .join(name);
    p.is_file().then(|| File::open(p).unwrap())
}

/// A tiny file built from scratch: table 130 "T" with text field 1 "Name"
/// (value index) and number field 2 "N".
fn synthetic() -> File {
    use write::{Entry, Item, encode};
    let mut header = vec![0u8; fmp12::SECTOR];
    header[..15].copy_from_slice(&[0, 1, 0, 0, 0, 2, 0, 1, 0, 5, 0, 2, 0, 2, 0xC0]);
    header[15..20].copy_from_slice(b"HBAM7");
    let p = |e: &[&[u8]]| e.iter().map(|x| x.to_vec()).collect::<Vec<_>>();
    let kv = |path: &[&[u8]], key, data: &[u8]| Entry {
        path: p(path),
        item: encode(&Item::Kv {
            key,
            data: data.to_vec(),
        }),
    };
    let node = |path: &[&[u8]]| Entry {
        path: p(path),
        item: vec![0x80],
    };
    let mut opts = vec![0u8; 26];
    opts[..2].copy_from_slice(&[1, 1]);
    opts[8] = 0x80; // index Minimal
    let mut num = opts.clone();
    num[1] = 2;
    num[8] = 0;
    let t: &[u8] = &[0x80, 2];
    let mut es = vec![
        node(&[&[3], &[16], &[5], t]),
        kv(&[&[3], &[16], &[5], t], 16, &save::encode_text("T")),
        kv(&[&[3], &[16], &[5], t], 251, &[0, 0, 0, 1]),
        node(&[t]),
        node(&[t, &[1], &[1]]),
        kv(&[t, &[1], &[1]], 16, &[0; 16]),
        node(&[t, &[3], &[5], &[1]]),
        kv(&[t, &[3], &[5], &[1]], 2, &opts),
        kv(&[t, &[3], &[5], &[1]], 16, &save::encode_text("Name")),
        node(&[t, &[3], &[5], &[2]]),
        kv(&[t, &[3], &[5], &[2]], 2, &num),
        kv(&[t, &[3], &[5], &[2]], 16, &save::encode_text("N")),
    ];
    write::sort(&mut es);
    File::from_bytes(write::build(&header, &es)).unwrap()
}

#[test]
fn save_add_edit_delete_roundtrip() {
    let f = synthetic();
    let table = &schema::tables(&f).unwrap()[0];
    assert_eq!(table.name, "T");
    let rec = |n: &str, v: &str| std::collections::BTreeMap::from([(1, n.into()), (2, v.into())]);
    let long = "é—".repeat(700);
    let first = schema::Records::from([
        (1, rec("Café — crème", "-3.5")),
        (2, rec("Line\rtwo", "12")),
        (3, rec(&long, "1e3")),
    ]);
    let f1 =
        File::from_bytes(save::save_records(&f, &[(130, first.clone())].into()).unwrap()).unwrap();
    assert_eq!(schema::records(&f1, table).unwrap(), first);
    assert_eq!(save::last_record_id(&f1, 130).unwrap(), 3);
    // Delete 2, edit 3, add 4: ids are not reused.
    let mut second = first.clone();
    second.remove(&2);
    second.get_mut(&3).unwrap().insert(2, "7".into());
    second.insert(4, rec("new", ""));
    let f2 = File::from_bytes(save::save_records(&f1, &[(130, second.clone())].into()).unwrap())
        .unwrap();
    second.get_mut(&4).unwrap().remove(&2); // empty values are not stored
    assert_eq!(schema::records(&f2, table).unwrap(), second);
    assert_eq!(save::last_record_id(&f2, 130).unwrap(), 4);
    // The record-id set, the dropped index and the unindexed field.
    let mut ids = None;
    let mut index = false;
    f2.walk(|path, c| match fmp12::int_path(path).as_deref() {
        Some([130, 13]) => ids = Some(save::read_id_set(c)),
        Some([130, 1, 1, ..]) => index = true,
        _ => {}
    })
    .unwrap();
    assert_eq!(ids.unwrap(), [1, 3, 4].into());
    assert!(!index);
    let name = &schema::tables(&f2).unwrap()[0].fields[&1];
    assert_eq!(name.indexing(), schema::Indexing::None);
    assert!(name.auto_index());
}

/// Record key 252 (modification count) is read, and kept by a save that
/// edits the record.
#[test]
fn modification_counts() {
    use write::{Entry, Item, encode};
    let f = synthetic();
    let rec = |n: &str| std::collections::BTreeMap::from([(1, n.into())]);
    let first = schema::Records::from([(1, rec("a")), (2, rec("b"))]);
    let f1 =
        File::from_bytes(save::save_records(&f, &[(130, first.clone())].into()).unwrap()).unwrap();
    let mut es = write::entries(&f1).unwrap();
    es.push(Entry {
        path: vec![vec![0x80, 2], vec![5], save::path_elem(2)],
        item: encode(&Item::Kv {
            key: 252,
            data: vec![1, 3],
        }),
    });
    write::sort(&mut es);
    let f2 = File::from_bytes(write::build(&f1.bytes()[..fmp12::SECTOR], &es)).unwrap();
    assert_eq!(
        schema::modification_counts(&f2, 130).unwrap(),
        [(2, 3)].into()
    );
    let mut second = first;
    second.insert(2, rec("edited"));
    let f3 = File::from_bytes(save::save_records(&f2, &[(130, second)].into()).unwrap()).unwrap();
    assert_eq!(
        schema::modification_counts(&f3, 130).unwrap(),
        [(2, 3)].into()
    );
}

#[test]
fn decodes_typed_values() {
    let Some(f) = local("inventory_records.fmp12") else {
        eprintln!("no local fixture, skipping");
        return;
    };
    let tables = schema::tables(&f).unwrap();
    let products = schema::records(&f, &tables[0]).unwrap();
    // Record 1 was deleted; ids are never reused.
    assert!(!products.contains_key(&1));
    assert_eq!(products.len(), 39);
    assert_eq!(save::last_record_id(&f, 130).unwrap(), 40);
    let r2 = &products[&2];
    assert_eq!(r2[&3], "Plain name");
    assert_eq!(r2[&5], "1999");
    assert_eq!(r2[&1], "11/27/2026");
    assert_eq!(r2[&4], "Acme\rCorp Été");
    assert_eq!(r2[&6], "p".repeat(1001));
    assert!(r2[&22].ends_with(" PM"));
    let tx = schema::records(&f, &tables[1]).unwrap();
    assert_eq!(tx[&1][&5], "Réception — lot A");
    assert_eq!(tx[&2][&7], "-4");
}

/// Re-encoding every record reproduces FileMaker's bytes.
#[test]
fn reencodes_records_byte_for_byte() {
    let Some(f) = local("inventory_records.fmp12") else {
        return;
    };
    let all = write::entries(&f).unwrap();
    for t in schema::tables(&f).unwrap() {
        let tp = save::path_elem(t.id);
        for (r, values) in schema::records(&f, &t).unwrap() {
            let rec = vec![tp.clone(), vec![5], save::path_elem(r)];
            let mut ours = vec![write::Entry {
                path: rec.clone(),
                item: vec![0x80],
            }];
            for (fid, v) in &values {
                ours.extend(save::value_entries(&rec, *fid, v));
            }
            let mut theirs: Vec<_> = all
                .iter()
                .filter(|e| e.path.starts_with(&rec))
                // Modification count, not a field.
                .filter(|e| !(e.path == rec && e.item.starts_with(&[0x02, 0xFC])))
                .cloned()
                .collect();
            write::sort(&mut ours);
            write::sort(&mut theirs);
            assert_eq!(ours, theirs, "table {} record {r}", t.id);
        }
    }
}
