//! Repeating fields and container data written by `Database::save`, read
//! back. With `BASECRAFT_OUT=<file.fmp12>` the saved file is also written
//! there, to open in FileMaker (FORMAT.md §13).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use calc::Value;
use engine::{Database, rep_key};
use fmp12::schema::DataType;
use fmp12::schema_edit::FieldSpec;

/// A 4×3 red PNG with transparency, as stored by FileMaker in its tests.
const RED_PNG: &str = "89504e470d0a1a0a0000000d4948445200000004000000030806000000b4f4aec6000000017352474200aece1ce90000000467414d410000b18f0bfc6105000000097048597300000ec300000ec301c76fa8640000001549444154185763f80f030c0c60cc80290065c0300090cc1fe1a78d04800000000049454e44ae426082";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn repetitions_and_containers_round_trip() {
    let mut file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Reps").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.tables[0].id;
    let field = |db: &mut Database, file: &mut fmp12::File, name: &str, dt, reps| {
        let mut s = FieldSpec::new(name, dt);
        s.storage.repetitions = reps;
        db.add_field(file, t, &s).unwrap()
    };
    let rep = field(&mut db, &mut file, "Rep", DataType::Text, 5);
    let num = field(&mut db, &mut file, "Num", DataType::Number, 3);
    let pic = field(&mut db, &mut file, "Pic", DataType::Container, 1);
    let doc = field(&mut db, &mut file, "Doc", DataType::Container, 2);
    let mut total = FieldSpec::calculated("Total", DataType::Number, "Num * 2");
    total.storage.repetitions = 3;
    db.add_field(&mut file, t, &total).unwrap();
    // Filled by FileMaker in the manual check (tools/oracle/README.md).
    field(&mut db, &mut file, "Note", DataType::Text, 1);
    let r = db.new_record(t).unwrap();
    let long = "é".repeat(1200);
    db.set_values(
        t,
        r,
        &[
            (rep, "a".into()),
            (rep_key(rep, 2), "b".into()),
            (rep_key(rep, 5), long.clone()),
            (num, "1".into()),
            (rep_key(num, 3), "4".into()),
        ],
    );
    db.insert_file(t, r, pic, 1, "red.png", unhex(RED_PNG));
    db.insert_file(t, r, doc, 2, "hello.txt", b"Hello container".to_vec());
    // The same file again shares the stored copy.
    let r2 = db.new_record(t).unwrap();
    db.insert_file(t, r2, pic, 1, "red.png", unhex(RED_PNG));
    assert_eq!(db.tables[0].containers.len(), 2);
    let check = |db: &Database| {
        let eval = |src: &str| db.eval_source(src, t, 0, Some(0)).to_text();
        assert_eq!(
            eval("Rep[2] & \"|\" & GetRepetition ( Rep ; 5 ) = \"b|\" & Last ( Rep )"),
            "1"
        );
        assert_eq!(eval("Num[3]"), "4");
        assert_eq!(
            eval("List ( Total[1] ; Total[3] ; Sum ( Total ) )"),
            "2\r8\r10"
        );
        assert_eq!(eval("GetAsText ( Pic )"), "red.png");
        assert_eq!(eval("GetContainerAttribute ( Pic ; \"height\" )"), "3");
        assert_eq!(eval("Base64Encode ( Doc[2] )"), "SGVsbG8gY29udGFpbmVy\r\n");
        assert_eq!(eval("IsEmpty ( Doc )"), "1");
        assert_eq!(db.value_rep(t, 0, rep, 5).to_text(), long);
        assert!(matches!(db.value(t, 1, pic), Value::Container(_)));
    };
    check(&db);
    let bytes = db.save(&file).unwrap();
    if let Ok(out) = std::env::var("BASECRAFT_OUT") {
        std::fs::write(out, &bytes).unwrap();
    }
    let db = Database::load(&fmp12::File::from_bytes(bytes).unwrap()).unwrap();
    check(&db);
    assert_eq!(db.tables[0].containers.len(), 2);
}
