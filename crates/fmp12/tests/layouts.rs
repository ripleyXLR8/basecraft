//! Layout writing (M6) on the FileMaker starter templates (skipped when
//! they are absent; override the folder with FMP12_TEMPLATES).

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use fmp12::layouts::{Bounds, Layout, LayoutObject};
use fmp12::pb::Msg;

fn template_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FMP12_TEMPLATES")
        .map(PathBuf::from)
        .unwrap_or_else(|| r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates".into());
    dir.is_dir().then_some(dir)
}

fn template_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for t in std::fs::read_dir(dir).unwrap() {
        let Ok(files) = std::fs::read_dir(t.unwrap().path()) else {
            continue;
        };
        for f in files {
            let f = f.unwrap().path();
            if f.extension().is_some_and(|e| e == "fmp12") {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

/// Raw protobuf messages of each layout: header, parts, object list.
#[derive(Default)]
struct RawLayout {
    header: Vec<u8>,
    parts: Vec<Vec<u8>>,
    list: BTreeMap<u32, Vec<u8>>,
}

fn raw_layouts(f: &fmp12::File) -> BTreeMap<u32, RawLayout> {
    let mut out: BTreeMap<u32, RawLayout> = BTreeMap::new();
    f.walk(|p, c| {
        let (Some(p), fmp12::Chunk::Kv { key, data }) = (fmp12::int_path(p), c) else {
            return;
        };
        match p.as_slice() {
            [4, 5, l] if *key == 2 => out.entry(*l).or_default().header = data.to_vec(),
            [4, 5, l] if *key == 7 => {
                out.entry(*l).or_default().list.insert(0, data.to_vec());
            }
            [4, 5, l, 7] if *key >= 1 => {
                out.entry(*l).or_default().list.insert(*key, data.to_vec());
            }
            [4, 5, l, 3, _] if *key == 2 => out.entry(*l).or_default().parts.push(data.to_vec()),
            _ => {}
        }
    })
    .unwrap();
    out
}

fn same_bytes(b: &[u8]) -> bool {
    Msg::parse(b).is_some_and(|m| m.encode() == b)
}

/// Decode then encode gives the same bytes for every layout message of
/// every template (header, parts, object list, each object and its style),
/// and re-encoding a layout's object list from its decoded form is exact.
#[test]
fn layout_messages_roundtrip_byte_for_byte() {
    let Some(dir) = template_dir() else {
        eprintln!("no FileMaker templates found, skipping");
        return;
    };
    let (mut layouts, mut objects) = (0, 0);
    for path in template_files(&dir) {
        let f = fmp12::File::open(&path).unwrap();
        let decoded = fmp12::layouts::layouts(&f).unwrap();
        for (id, raw) in raw_layouts(&f) {
            let name = format!("{} layout {id}", path.display());
            assert!(same_bytes(&raw.header), "{name}: header");
            for p in &raw.parts {
                assert!(same_bytes(p), "{name}: part");
            }
            let blob: Vec<u8> = raw.list.into_values().flatten().collect();
            assert!(same_bytes(&blob), "{name}: object list");
            for o in Msg::parse(&blob).unwrap().msgs(2) {
                assert_eq!(o.encode(), Msg::parse(&o.encode()).unwrap().encode());
                if let Some(s) = o.bytes(5) {
                    assert!(same_bytes(s), "{name}: style");
                }
                objects += 1;
            }
            let l = decoded.iter().find(|l| l.id == id).unwrap();
            let again = fmp12::layouts::encode_objects(&blob, l, l, &|_| None);
            assert!(again == blob, "{name}: object list rebuilt from the model");
            layouts += 1;
        }
    }
    assert!(layouts > 0);
    eprintln!("{layouts} layouts, {objects} objects re-encoded byte for byte");
}

/// Saving unchanged layouts leaves the file's content untouched.
#[test]
fn unchanged_layouts_save_to_same_content() {
    let Some(dir) = template_dir() else { return };
    let f = fmp12::File::open(dir.join("Tasks").join("en.fmp12")).unwrap();
    let ls = fmp12::layouts::layouts(&f).unwrap();
    let bytes = fmp12::save::save_layouts(&f, &ls).unwrap();
    let again = fmp12::File::from_bytes(bytes).unwrap();
    let mut a = fmp12::write::entries(&f).unwrap();
    fmp12::write::drop_leftovers(&mut a);
    let mut b = fmp12::write::entries(&again).unwrap();
    fmp12::write::drop_leftovers(&mut b);
    assert!(a == b);
}

fn object(l: &Layout, id: u64) -> &LayoutObject {
    l.objects.iter().find(|o| o.id == id).unwrap()
}

/// An edited layout re-decodes with its edits; other layouts do not change.
#[test]
fn edited_layout_redecodes() {
    let Some(dir) = template_dir() else { return };
    let f = fmp12::File::open(dir.join("Contacts").join("en.fmp12")).unwrap();
    let before = fmp12::layouts::layouts(&f).unwrap();
    let mut ls = before.clone();
    let l = ls.iter_mut().find(|l| l.name == "Contact List").unwrap();
    // Move field 9 (Last Name) by (10, 20) and widen it.
    let o = l.objects.iter_mut().find(|o| o.id == 9).unwrap();
    o.bounds = Bounds {
        top: 122.0,
        left: 386.0,
        bottom: 166.0,
        right: 700.0,
    };
    o.name = "LastNameField".into();
    use fmp12::layouts::field_flags::{AUTO_COMPLETE, NO_QUICK_FIND};
    assert_eq!(o.field_flags & (AUTO_COMPLETE | NO_QUICK_FIND), 0);
    o.field_flags |= AUTO_COMPLETE | NO_QUICK_FIND;
    let flags = o.field_flags;
    // Retext the "Company" label, delete object 6, grow the body.
    l.objects.iter_mut().find(|o| o.id == 3).unwrap().text = Some("Société".into());
    l.objects.retain(|o| o.id != 6);
    let body = l.parts.iter_mut().find(|p| p.kind == 4).unwrap();
    body.height += 15.0;
    // New field (copy of field 10 at another place), text and button.
    let mut nf = object(l, 10).clone();
    nf.id = 13;
    nf.bounds = Bounds {
        top: 120.0,
        left: 700.0,
        bottom: 140.0,
        right: 800.0,
    };
    nf.name = String::new();
    let mut nt = object(l, 3).clone();
    nt.id = 14;
    nt.text = Some("New text".into());
    nt.bounds.top += 100.0;
    nt.bounds.bottom += 100.0;
    let mut nb = object(l, 7).clone();
    nb.id = 15;
    nb.button = Some(fmp12::scripts::targeted_step(
        fmp12::scripts::id::PERFORM_SCRIPT,
        2,
    ));
    nb.label = Some(fmp12::save::text_literal_tokens("Go"));
    nb.bounds.left -= 100.0;
    nb.bounds.right -= 100.0;
    l.objects.insert(0, nb.clone());
    l.objects.push(nf.clone());
    l.objects.push(nt.clone());
    let edited = l.clone();

    let bytes = fmp12::save::save_layouts(&f, &ls).unwrap();
    let g = fmp12::File::from_bytes(bytes).unwrap();
    let after = fmp12::layouts::layouts(&g).unwrap();
    let l2 = after.iter().find(|l| l.name == "Contact List").unwrap();
    assert_eq!(l2.parts, edited.parts);
    assert_eq!(object(l2, 9).bounds, object(&edited, 9).bounds);
    assert_eq!(object(l2, 9).name, "LastNameField");
    assert_eq!(object(l2, 9).field_flags, flags);
    assert_eq!(object(l2, 3).text.as_deref(), Some("Société"));
    assert!(l2.objects.iter().all(|o| o.id != 6));
    assert_eq!(object(l2, 13).field, nf.field);
    assert_eq!(object(l2, 13).bounds, nf.bounds);
    assert_eq!(object(l2, 14).text.as_deref(), Some("New text"));
    assert_eq!(object(l2, 15).button, nb.button);
    assert_eq!(object(l2, 15).label, nb.label);
    assert_eq!(l2.objects[0].id, 15, "z-order kept");
    let ids = |l: &Layout| l.objects.iter().map(|o| o.id).collect::<Vec<_>>();
    assert_eq!(ids(l2), ids(&edited));
    for (a, b) in before.iter().zip(&after) {
        if a.id != edited.id {
            assert_eq!(a, b, "layout {} changed", a.name);
        }
    }
    // The script engine still reads the file.
    fmp12::scripts::scripts(&g).unwrap();
}

#[test]
fn targeted_steps_decode() {
    let s = fmp12::scripts::targeted_step(fmp12::scripts::id::GO_TO_LAYOUT, 3);
    assert_eq!(
        s.params,
        fmp12::scripts::StepParams::GoToLayout {
            dest: fmp12::scripts::LayoutDest::Layout(3),
            animation: 0
        }
    );
    let s = fmp12::scripts::targeted_step(fmp12::scripts::id::PERFORM_SCRIPT, 300);
    assert!(matches!(
        s.params,
        fmp12::scripts::StepParams::PerformScript {
            script: Some(300),
            ..
        }
    ));
}

/// The file's picture store reads (the templates keep pictures there but
/// place no graphic object), and any graphic object's picture is in it.
#[test]
fn graphic_objects_resolve_to_pictures() {
    let Some(dir) = template_dir() else { return };
    let mut seen = 0;
    for f in template_files(&dir) {
        let file = fmp12::File::open(&f).unwrap();
        let pictures = fmp12::containers::layout_pictures(&file).unwrap();
        seen += pictures.len();
        for l in fmp12::layouts::layouts(&file).unwrap() {
            for o in l.objects.iter().filter(|o| o.kind == 3) {
                let id = o
                    .picture
                    .unwrap_or_else(|| panic!("{f:?} {} obj {}", l.name, o.id));
                let p = &pictures[&id];
                assert!(
                    fmp12::containers::image_tag(p.data()).is_some(),
                    "{f:?} picture {id}"
                );
            }
        }
    }
    assert!(seen > 0);
}

/// Key `key` of `path` as stored (layout settings live in two places).
fn kv_at(f: &fmp12::File, path: &[u32], key: u32) -> Option<Vec<u8>> {
    let mut out = None;
    f.walk(|p, c| {
        if let (Some(p), fmp12::Chunk::Kv { key: k, data }) = (fmp12::int_path(p), c)
            && p == path
            && *k == key
        {
            out = Some(data.to_vec());
        }
    })
    .unwrap();
    out
}

type Edit = fn(&mut Layout);

/// Layout Setup settings round-trip one at a time, written where
/// FileMaker Pro 26 writes them: the options are the values FileMaker
/// wrote for the same change on Contacts "Détails des contacts"
/// (0x10F3080D before).
#[test]
fn layout_setup_settings_roundtrip() {
    use fmp12::layouts::setup;
    use fmp12::triggers::{Trigger, mode};
    let Some(dir) = template_dir() else { return };
    let f = fmp12::File::open(dir.join("Contacts").join("fr.fmp12")).unwrap();
    let before = fmp12::layouts::layouts(&f).unwrap();
    let l = before.iter().position(|l| l.id == 3).unwrap();
    assert_eq!(before[l].options, 0x10F3_080D);
    assert_eq!(before[l].views(), [true, false, false]);
    assert_eq!((before[l].menu_set, before[l].columns), (0, 2));
    // Liste des contacts: list only, default list, OnLayoutEnter.
    let list = before.iter().find(|l| l.id == 2).unwrap();
    assert_eq!(
        (list.views(), list.default_view()),
        ([false, true, false], 1)
    );
    assert_eq!(list.triggers.len(), 1);
    let edits: [(&str, Edit, u64); 12] = [
        ("save", |l| l.options |= setup::NO_AUTO_SAVE, 0x10F3_081D),
        ("frames", |l| l.options |= setup::FIELD_FRAMES, 0x10F3_082D),
        ("delineate", |l| l.options &= !setup::DELINEATE, 0x10F3_080C),
        (
            "indicator",
            |l| l.options &= !setup::NO_RECORD_INDICATOR,
            0x00F3_080D,
        ),
        (
            "quick find",
            |l| l.options |= setup::NO_QUICK_FIND,
            0x10F3_880D,
        ),
        ("table", |l| l.options &= !setup::NO_TABLE, 0x10F3_0805),
        (
            "list default",
            |l| {
                l.options &= !(setup::NO_LIST | setup::NO_TABLE);
                l.set_default_view(1);
            },
            0x10F3_0A01,
        ),
        (
            "table default",
            |l| {
                l.options &= !(setup::NO_LIST | setup::NO_TABLE);
                l.set_default_view(2);
            },
            0x10F3_4A01,
        ),
        (
            "columns",
            |l| {
                l.options |= setup::COLUMNS | setup::COLUMNS_DOWN;
                l.columns = 3;
            },
            0x10F3_0D0D,
        ),
        (
            "margins",
            |l| {
                l.options |= setup::FIXED_MARGINS | setup::FACING_PAGES;
                l.margins = [11.0, 33.0, 22.0, 44.0];
            },
            0x10F3_08CD,
        ),
        ("menu set", |l| l.menu_set = 1, 0x10F3_080D),
        ("hidden", |l| l.flags |= setup::HIDDEN, 0x10F3_080D),
    ];
    for (what, edit, options) in edits {
        let mut ls = before.clone();
        edit(&mut ls[l]);
        let f2 = fmp12::File::from_bytes(fmp12::save::save_layouts(&f, &ls).unwrap()).unwrap();
        let after = fmp12::layouts::layouts(&f2).unwrap();
        assert_eq!(after[l], ls[l], "{what}");
        assert_eq!(after[l].options, options, "{what}");
        for p in [&[4, 5, 3][..], &[4, 1, 7, 3]] {
            assert_eq!(kv_at(&f2, p, 241).unwrap(), options.to_be_bytes(), "{what}");
            let menu = Msg::parse(&kv_at(&f2, p, 2).unwrap()).unwrap().uint(3);
            assert_eq!(menu, Some(ls[l].menu_set), "{what}");
        }
        if what == "hidden" {
            for p in [&[4, 5, 3][..], &[4, 1, 7, 3]] {
                assert_eq!(kv_at(&f2, p, 240).unwrap(), 8u64.to_be_bytes(), "{what}");
            }
        }
        for (a, b) in before.iter().zip(&after).filter(|(a, _)| a.id != 3) {
            assert_eq!(a, b, "{what}: other layouts");
        }
    }
    // Triggers: two added (FileMaker wrote count 2 in both places and
    // these records), then removed again.
    let mut ls = before.clone();
    ls[l].triggers = vec![
        Trigger {
            event: 103,
            script: 3,
            modes: mode::BROWSE,
            parameter: None,
        },
        Trigger {
            event: 104,
            script: 3,
            modes: mode::BROWSE | mode::FIND,
            parameter: None,
        },
    ];
    let bytes = fmp12::save::save_layouts(&f, &ls).unwrap();
    // FMP12_SETUP_DEMO=<out.fmp12>: every setting changed at once, to open
    // in FileMaker (Layout Setup of "Détails des contacts").
    if let Some(out) = std::env::var_os("FMP12_SETUP_DEMO") {
        let mut all = ls.clone();
        for (_, edit, _) in &edits[..11] {
            edit(&mut all[l]);
        }
        // Hidden from the layout menus: the list layout (2).
        all.iter_mut().find(|l| l.id == 2).unwrap().flags |= setup::HIDDEN;
        std::fs::write(out, fmp12::save::save_layouts(&f, &all).unwrap()).unwrap();
    }
    let f2 = fmp12::File::from_bytes(bytes).unwrap();
    assert_eq!(
        fmp12::layouts::layouts(&f2).unwrap()[l].triggers,
        ls[l].triggers
    );
    assert_eq!(kv_at(&f2, &[4, 5, 3, 48], 4).unwrap(), [0, 2]);
    assert_eq!(kv_at(&f2, &[4, 1, 7, 3], 48).unwrap(), [0, 0, 0, 2]);
    assert_eq!(
        kv_at(&f2, &[4, 5, 3, 48, 5, 232], 1).unwrap(),
        [0, 0x68, 2, 1, 3, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0]
    );
    let mut ls2 = fmp12::layouts::layouts(&f2).unwrap();
    ls2[l].triggers.clear();
    let f3 = fmp12::File::from_bytes(fmp12::save::save_layouts(&f2, &ls2).unwrap()).unwrap();
    assert!(fmp12::layouts::layouts(&f3).unwrap()[l].triggers.is_empty());
    assert_eq!(kv_at(&f3, &[4, 1, 7, 3], 48).unwrap(), [0, 0, 0, 0]);
}

/// A merge field stored by reference (files from older versions): the
/// occurrence and field of a part's break field reference, in hex.
#[test]
fn merge_field_by_reference() {
    let r = fmp12::layouts::merge_field_ref;
    assert_eq!(r("\u{1}\u{2}\u{e}0003D000020108"), Some((130, 8)));
    // A field id of two bytes (path-integer spelling).
    assert_eq!(r("\u{1}\u{2}\u{10}0003D0000202800C"), Some((130, 140)));
    assert_eq!(r("Contacts::Name"), None);
    // Round trip with the break field writer.
    let b = fmp12::layouts::break_field_bytes(131, 12);
    let hex: String = b[2..].iter().map(|x| format!("{x:02X}")).collect();
    assert_eq!(r(&format!("\u{1}\u{2}\u{e}{hex}")), Some((131, 12)));
}
