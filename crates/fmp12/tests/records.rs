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
    synthetic_in(0)
}

/// [`synthetic`] with index language `lang` (option bytes 4–7).
fn synthetic_in(lang: u32) -> File {
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
    opts[4..8].copy_from_slice(&lang.to_be_bytes());
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

/// Record key 252 (modification count) is read, and a save that edits the
/// record counts one commit more (or the count the caller kept).
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
    second.insert(1, rec("a2"));
    second.insert(3, rec("new"));
    let f3 = File::from_bytes(save::save_records(&f2, &[(130, second.clone())].into()).unwrap())
        .unwrap();
    // Edited: 3 → 4 and 0 → 1; the new record has none.
    assert_eq!(
        schema::modification_counts(&f3, 130).unwrap(),
        [(1, 1), (2, 4)].into()
    );
    // Counts kept by the caller (commits in the app) win when higher.
    second.insert(2, rec("again"));
    let given = [(130, [(2, 9), (3, 2)].into())].into();
    let f4 =
        File::from_bytes(save::save_records_counted(&f3, &[(130, second)].into(), &given).unwrap())
            .unwrap();
    assert_eq!(
        schema::modification_counts(&f4, 130).unwrap(),
        [(1, 1), (2, 9)].into()
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

/// An indexed text field (English) keeps an index written as FileMaker
/// builds it (keys checked against FileMaker Pro 26 in `index::tests`),
/// and stays marked indexed; postings follow edits.
#[test]
fn value_index_is_rebuilt() {
    let f = synthetic_in(21);
    let rec = |n: &str| std::collections::BTreeMap::from([(1, n.into())]);
    let index = |f: &File| {
        let mut keys = std::collections::BTreeMap::new();
        let mut stats = None;
        f.walk(|path, c| {
            let head: Option<Vec<u32>> = path.iter().take(4).map(|p| fmp12::path_int(p)).collect();
            match (head.as_deref(), path.len(), c) {
                (Some([130, 1, 1, 0]), 5, c) if !matches!(c, fmp12::Chunk::Marker) => {
                    let k: String = path[4].iter().map(|b| format!("{b:02x}")).collect();
                    keys.insert(k, save::read_id_set(c));
                }
                (Some([130, 1, 1]), 3, fmp12::Chunk::Kv { key: 16, data }) => {
                    stats = Some(data.to_vec())
                }
                _ => {}
            }
        })
        .unwrap();
        (keys, stats)
    };
    let first = schema::Records::from([
        (1, rec("Café — crème")),
        (2, rec("Line\rtwo")),
        (3, rec("x")),
        (4, rec("")),
    ]);
    let f1 =
        File::from_bytes(save::save_records(&f, &[(130, first.clone())].into()).unwrap()).unwrap();
    let (keys, stats) = index(&f1);
    let want = std::collections::BTreeMap::from([
        ("01".to_string(), [4].into()),
        (
            "01123d120f12a3126b020a022c020a123d13da126b135f126b".into(),
            [1].into(),
        ),
        ("01133012ec136d126b".into(), [2].into()),
        ("011433148d138e".into(), [2].into()),
        ("011497".into(), [3].into()),
    ]);
    assert_eq!(keys, want);
    // 5 entries, 5 keys.
    assert_eq!(
        stats.unwrap(),
        [[0, 0, 0, 0, 0, 0, 0, 5], [0, 0, 0, 0, 0, 0, 0, 5]].concat()
    );
    let name = &schema::tables(&f1).unwrap()[0].fields[&1];
    assert_eq!(name.indexing(), schema::Indexing::Minimal);
    // Record 3 now shares record 1's key; record 2 is gone.
    let mut second = first;
    second.insert(3, rec("café — CRÈME"));
    second.remove(&2);
    let f2 = File::from_bytes(save::save_records(&f1, &[(130, second)].into()).unwrap()).unwrap();
    let (keys, _) = index(&f2);
    assert_eq!(keys.len(), 2);
    assert_eq!(
        keys["01123d120f12a3126b020a022c020a123d13da126b135f126b"],
        [1, 3].into()
    );
}
