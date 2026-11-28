//! Script writer on the FileMaker starter templates (skipped when they are
//! not installed; override the directory with FMP12_TEMPLATES).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fmp12::scripts::{
    Calc, DialogButton, FieldRef, LayoutDest, Script, SortOrder, Step, StepParams, id, scripts,
};
use fmp12::scripts_save::{save_scripts, step_entries};
use fmp12::write::{self, Entry};

fn template_files() -> Vec<PathBuf> {
    let dir = std::env::var_os("FMP12_TEMPLATES")
        .map(PathBuf::from)
        .unwrap_or_else(|| r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates".into());
    let mut out = Vec::new();
    for t in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(t.path()).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|e| e == "fmp12") {
                out.push(f.path());
            }
        }
    }
    // Plus the oracle files (FMP12_ORACLE), e.g. a script of 57 pasted steps.
    if let Some(d) = std::env::var_os("FMP12_ORACLE") {
        for f in std::fs::read_dir(d).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|e| e == "fmp12") {
                out.push(f.path());
            }
        }
    }
    out.sort();
    out
}

fn ipath(e: &Entry) -> Option<Vec<u32>> {
    e.path.iter().map(|p| fmp12::path_int(p)).collect()
}

/// Step list and parameter entries of script `s` as stored.
fn stored(entries: &[Entry], s: u32) -> (Vec<Entry>, u32) {
    let mut out = Vec::new();
    let mut last = 0;
    for e in entries {
        let Some(p) = ipath(e) else { continue };
        let list = p == [17, 5, s] && e.item.get(1) == Some(&4) && e.item[0] <= 7;
        let sub = p.len() >= 4 && p[..3] == [17, 5, s] && matches!(p[3], 4 | 5);
        if list || sub {
            if p == [17, 5, s, 5]
                && let Ok(c) = fmp12::chunk::parse(&e.item)
                && let Some(fmp12::Chunk::Kv { key: 0, data }) = c.first()
            {
                last = fmp12::save::read_len_int(data).unwrap() as u32;
            }
            out.push(e.clone());
        }
    }
    (out, last)
}

#[test]
fn template_scripts_reencode_byte_for_byte() {
    let (mut n, mut steps) = (0, 0);
    for path in template_files() {
        let file = fmp12::File::open(&path).unwrap();
        let entries = write::entries(&file).unwrap();
        for s in scripts(&file).unwrap() {
            if s.steps.is_empty() {
                continue;
            }
            let (mut want, last) = stored(&entries, s.id);
            let (mut got, count) = step_entries(&s);
            assert_eq!(count, last, "{} {}", path.display(), s.name);
            write::sort(&mut want);
            write::sort(&mut got);
            if got != want {
                let extra: Vec<_> = got.iter().filter(|e| !want.contains(e)).collect();
                let missing: Vec<_> = want.iter().filter(|e| !got.contains(e)).collect();
                panic!(
                    "{} script {} {}:
 ours {:02x?}
 theirs {:02x?}",
                    path.display(),
                    s.id,
                    s.name,
                    extra,
                    missing
                );
            }
            n += 1;
            steps += s.steps.len();
        }
    }
    eprintln!("{n} scripts ({steps} steps) re-encode byte for byte");
}

#[test]
fn typed_parameters_encode_back() {
    let (mut n, mut bad) = (0, 0);
    for path in template_files() {
        let file = fmp12::File::open(&path).unwrap();
        for s in scripts(&file).unwrap() {
            for st in &s.steps {
                if matches!(st.params, StepParams::Other | StepParams::None) {
                    continue;
                }
                let mut a = st.clone();
                a.raw.resize(28, 0);
                let mut b = a.clone();
                b.rewrite();
                n += 1;
                // Text keys may differ by SCSU window choice only.
                let same_text = a.raw_params.kv.len() == b.raw_params.kv.len()
                    && a.raw_params.kv.iter().zip(&b.raw_params.kv).all(|(x, y)| {
                        x.0 == y.0 && (x.1 == y.1 || fmp12::text(x.1) == fmp12::text(y.1))
                    });
                let mut b2 = b.raw_params.clone();
                b2.kv = a.raw_params.kv.clone();
                // Non-Latin text: our SCSU windows differ, so the chunk
                // lengths do too; the text must still read the same.
                let cjk = a.raw_params.kv.keys().eq(b.raw_params.kv.keys())
                    && (0..40).all(|p| a.raw_params.text(p) == b.raw_params.text(p))
                    && a.raw_params.text(0).is_some_and(|t| !t.is_ascii());
                if a.raw != b.raw || !cjk && (!same_text || b2 != a.raw_params) {
                    bad += 1;
                    if bad <= 12 {
                        eprintln!(
                            "{} {} step {:?}
  raw {:02x?}
  got {:02x?}
  kv {:02x?}
  got {:02x?}",
                            path.display(),
                            s.name,
                            st.name(),
                            a.raw,
                            b.raw,
                            a.raw_params.kv,
                            b.raw_params.kv
                        );
                    }
                }
            }
        }
    }
    eprintln!("{n} typed steps, {bad} differ");
    assert_eq!(bad, 0);
}

#[test]
fn unchanged_save_is_a_plain_rewrite() {
    let Some(path) = template_files()
        .into_iter()
        .find(|p| p.ends_with(r"Tasks\en.fmp12"))
    else {
        return;
    };
    let file = fmp12::File::open(&path).unwrap();
    let list = scripts(&file).unwrap();
    assert_eq!(
        save_scripts(&file, &list).unwrap(),
        write::rewrite(&file).unwrap()
    );
}

#[test]
fn edited_and_new_scripts_read_back() {
    let Some(path) = template_files()
        .into_iter()
        .find(|p| p.ends_with(r"Tasks\en.fmp12"))
    else {
        return;
    };
    let file = fmp12::File::open(&path).unwrap();
    let names = fmp12::calc::FileNames::load(&file).unwrap();
    let calc = |s: &str| Calc(fmp12::calc::compile(s, 0, &names).unwrap().tokens);
    let mut list = scripts(&file).unwrap();
    // Edit "Sort by Name": a comment, an If block around a Set Variable,
    // a disabled Beep; descending sort.
    let i = list.iter().position(|s| s.name == "Sort by Name").unwrap();
    let mut comment = Step::new(id::COMMENT);
    comment.params = StepParams::Comment("Edited by BaseCraft".into());
    let mut if_ = Step::new(id::IF);
    if_.params = StepParams::Condition(Some(calc("Get ( FoundCount ) > 0")));
    let mut set = Step::new(id::SET_VARIABLE);
    set.params = StepParams::SetVariable {
        name: "$count".into(),
        value: Some(calc("Get ( FoundCount ) & \" tasks\"")),
        repetition: None,
    };
    let end = Step::new(id::END_IF);
    let mut beep = Step::new(id::BEEP);
    beep.enabled = false;
    let s = &mut list[i];
    s.steps.insert(0, comment);
    s.steps.extend([if_, set, end, beep]);
    for st in &mut s.steps {
        if let StepParams::Sort { keys, .. } = &mut st.params {
            keys[0].1 = SortOrder::Descending;
        }
    }
    s.name = "Sort by Name (edited)".into();
    // A new script with a dialog and a layout change.
    let mut dialog = Step::new(id::SHOW_CUSTOM_DIALOG);
    dialog.params = StepParams::ShowCustomDialog {
        title: Some(calc("\"Hello\"")),
        message: Some(calc("\"Count: \" & $count")),
        buttons: vec![
            DialogButton {
                label: Some(calc("\"OK\"")),
                commit: true,
            },
            DialogButton {
                label: Some(calc("\"Cancel\"")),
                commit: false,
            },
        ],
        input: None,
    };
    let mut go = Step::new(id::GO_TO_LAYOUT);
    go.params = StepParams::GoToLayout {
        dest: LayoutDest::Layout(2),
        animation: 0,
    };
    let new_id = list.iter().map(|s| s.id).max().unwrap() + 1;
    list.push(Script {
        id: new_id,
        name: "New Script".into(),
        options: 8,
        steps: vec![dialog, go],
    });
    // Delete "Sort by Status".
    list.retain(|s| s.name != "Sort by Status");

    let bytes = save_scripts(&file, &list).unwrap();
    let back = scripts(&fmp12::File::from_bytes(bytes).unwrap()).unwrap();
    assert_eq!(back.len(), list.len());
    assert!(back.iter().all(|s| s.name != "Sort by Status"));
    let ed = back
        .iter()
        .find(|s| s.name == "Sort by Name (edited)")
        .unwrap();
    let ids: Vec<(u16, bool)> = ed.steps.iter().map(|s| (s.id, s.enabled)).collect();
    assert_eq!(ids.first(), Some(&(id::COMMENT, true)));
    assert_eq!(ids.last(), Some(&(id::BEEP, false)));
    assert_eq!(
        ed.steps[0].params,
        StepParams::Comment("Edited by BaseCraft".into())
    );
    let n = ed.steps.len();
    // If → End If link.
    assert_eq!(ed.steps[n - 4].arg as usize, n - 2);
    let StepParams::SetVariable { name, value, .. } = &ed.steps[n - 3].params else {
        panic!()
    };
    assert_eq!(name, "$count");

    assert_eq!(
        value.as_ref().unwrap().text(&names).unwrap(),
        "Get ( FoundCount ) & \" tasks\""
    );
    assert!(ed.steps.iter().any(|s| matches!(&s.params,
        StepParams::Sort { keys, .. } if keys[0].1 == SortOrder::Descending)));
    let new = back.iter().find(|s| s.name == "New Script").unwrap();
    assert_eq!(new.id, new_id);
    let StepParams::ShowCustomDialog { buttons, title, .. } = &new.steps[0].params else {
        panic!()
    };
    assert_eq!(buttons.len(), 3);
    assert!(buttons[0].commit && !buttons[1].commit);
    assert_eq!(title.as_ref().unwrap().text(&names).unwrap(), "\"Hello\"");
    assert_eq!(
        new.steps[1].params,
        StepParams::GoToLayout {
            dest: LayoutDest::Layout(2),
            animation: 0
        }
    );
    let _ = FieldRef::default();
}

/// Stored `[17].[1]` key 8.
fn key8(f: &fmp12::File) -> Option<[u8; 16]> {
    let mut out = None;
    f.walk(|path, c| {
        if let fmp12::Chunk::Kv { key: 8, data } = c
            && fmp12::int_path(path).as_deref() == Some(&[17, 1])
        {
            out = (*data).try_into().ok();
        }
    })
    .unwrap();
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The script catalog digest reproduces key 8 of every committed fixture
/// (none has scripts, hence none has a key 8) and of every template.
#[test]
fn catalog_digest_matches_stored_key8() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("assets/blank.fmp12")];
    for f in std::fs::read_dir(root.join("tests/fixtures"))
        .unwrap()
        .flatten()
    {
        if f.path().extension().is_some_and(|e| e == "fmp12") {
            files.push(f.path());
        }
    }
    files.extend(template_files());
    for p in files {
        let f = fmp12::File::open(&p).unwrap();
        let d = fmp12::scripts_save::catalog_digest(&write::entries(&f).unwrap());
        assert_eq!(d, key8(&f), "{}", p.display());
    }
}

/// FileMaker 26 writes `786c087f…62da` for one script "Nouveau script"
/// (in a blank file and in an emptied Tasks); a new file gets the same.
#[test]
fn new_script_key8_as_filemaker() {
    let blank = fmp12::File::from_bytes(fmp12::schema_edit::new_file("T").unwrap()).unwrap();
    let list = vec![Script {
        id: 1,
        name: "Nouveau script".into(),
        options: 8,
        steps: vec![Step::new(id::BEEP)],
    }];
    let out = fmp12::File::from_bytes(save_scripts(&blank, &list).unwrap()).unwrap();
    assert_eq!(
        key8(&out).map(|k| hex(&k)).as_deref(),
        Some("786c087fff6b2b8133833d37f78462da")
    );
    // Deleting it again leaves no key 8, as in a file without scripts.
    let empty = fmp12::File::from_bytes(save_scripts(&out, &[]).unwrap()).unwrap();
    assert_eq!(key8(&empty), None);
}

/// Adding, renaming (also to a non-Latin name) and deleting scripts on the
/// templates: key 8 follows the new list and the names read back.
#[test]
fn script_list_changes_update_key8() {
    for p in template_files() {
        let f = fmp12::File::open(&p).unwrap();
        let mut list = scripts(&f).unwrap();
        let next = list.iter().map(|s| s.id).max().unwrap() + 1;
        let first = list.iter().position(|s| s.options == 8).unwrap();
        list[first].name = "Renommé 改名".into();
        let gone = list.iter().rposition(|s| s.options == 8).unwrap();
        list.remove(gone);
        list.push(Script {
            id: next,
            name: "BC Added".into(),
            options: 8,
            steps: vec![Step::new(id::BEEP)],
        });
        let out = fmp12::File::from_bytes(save_scripts(&f, &list).unwrap()).unwrap();
        let d = fmp12::scripts_save::catalog_digest(&write::entries(&out).unwrap());
        assert!(d.is_some() && d != key8(&f), "{}", p.display());
        assert_eq!(d, key8(&out), "{}", p.display());
        let back: Vec<String> = scripts(&out).unwrap().into_iter().map(|s| s.name).collect();
        let want: Vec<String> = list.into_iter().map(|s| s.name).collect();
        assert_eq!(back, want, "{}", p.display());
    }
}

/// The first script of a file with none (the blank asset): every node of
/// the catalog gets its marker, as FileMaker 26 writes them; without the
/// markers of `[17].[1].[1]` and `[17].[1].[7]` FileMaker Server's
/// consistency check reports a damaged block (error 805).
#[test]
fn first_script_of_a_blank_file_marks_its_nodes() {
    let blank = fmp12::File::from_bytes(include_bytes!("../assets/blank.fmp12").to_vec()).unwrap();
    assert!(scripts(&blank).unwrap().is_empty());
    let list = vec![Script {
        id: 1,
        name: "Nouveau script".into(),
        options: 8,
        steps: vec![Step::new(id::BEEP)],
    }];
    let out = fmp12::File::from_bytes(save_scripts(&blank, &list).unwrap()).unwrap();
    let es = write::entries(&out).unwrap();
    for node in [
        vec![0x11u8],
        vec![0x11, 0x01],
        vec![0x11, 0x01, 0x01],
        vec![0x11, 0x01, 0x07],
        vec![0x11, 0x01, 0x07, 0x01],
    ] {
        let path: Vec<Vec<u8>> = node.iter().map(|b| vec![*b]).collect();
        assert!(
            es.iter()
                .any(|e: &Entry| e.path == path && e.item == [0x80]),
            "no marker for {node:02x?}"
        );
    }
    assert_eq!(scripts(&out).unwrap()[0].name, "Nouveau script");
}

/// A script of 140 steps: step node 128 and up is spelled `80 xx`, its
/// parameters `00 xx`, and the step record says `03 02 80 xx` (length 3,
/// a path integer of 2 bytes), as FileMaker 26 writes them; `[S].[5]`
/// key 0 is the count as an id (`02 80 0C`). With `03 01 80 00` FileMaker
/// Server's XML export showed no parameters from step 129 on.
/// `BASECRAFT_STEPS140=<out.fmp12>` keeps the file (checked with
/// `tools/oracle/fms_check.sh`).
#[test]
fn scripts_of_128_steps_and_more_keep_their_parameters() {
    let blank = fmp12::File::from_bytes(include_bytes!("../assets/blank.fmp12").to_vec()).unwrap();
    let names = fmp12::calc::FileNames::load(&blank).unwrap();
    let steps: Vec<Step> = (1..=140)
        .map(|i| {
            let mut s = Step::new(id::SET_VARIABLE);
            s.params = StepParams::SetVariable {
                name: format!("$v{i}"),
                value: Some(Calc(
                    fmp12::calc::compile(&i.to_string(), 0, &names)
                        .unwrap()
                        .tokens,
                )),
                repetition: None,
            };
            s
        })
        .collect();
    let list = vec![Script {
        id: 1,
        name: "Long".into(),
        options: 8,
        steps,
    }];
    let bytes = save_scripts(&blank, &list).unwrap();
    if let Some(p) = std::env::var_os("BASECRAFT_STEPS140") {
        std::fs::write(p, &bytes).unwrap();
    }
    let out = fmp12::File::from_bytes(bytes).unwrap();
    let back = &scripts(&out).unwrap()[0];
    assert_eq!(back.steps.len(), 140);
    for (i, s) in back.steps.iter().enumerate() {
        let StepParams::SetVariable { name, value, .. } = &s.params else {
            panic!("step {i}: {:?}", s.params)
        };
        assert_eq!(name, &format!("$v{}", i + 1));
        assert!(value.is_some(), "step {i}");
    }
    assert_eq!(back.steps[126].raw[..4], [2, 1, 0x7F, 0]);
    assert_eq!(back.steps[127].raw[..4], [3, 2, 0x80, 0]);
    assert_eq!(back.steps[139].raw[..4], [3, 2, 0x80, 0x0C]);
    let es = write::entries(&out).unwrap();
    let count = es
        .iter()
        .find(|e| {
            e.path == [vec![0x11], vec![5], vec![1], vec![5]] && e.item.starts_with(&[0x06, 0])
        })
        .unwrap();
    assert_eq!(count.item, [0x06, 0, 3, 2, 0x80, 0x0C]);
}

/// Script ids from 128 on are written in the catalog as FileMaker writes
/// ids in its catalogs (`02 80 xx`, as in the base table catalog): the
/// display order `[17].[1].[3]`, the name index and the last id. With
/// len-ints (`01 80`) FileMaker Server's XML export listed them as
/// `<unknown>` scripts.
#[test]
fn script_ids_from_128_in_the_catalog() {
    let blank = fmp12::File::from_bytes(include_bytes!("../assets/blank.fmp12").to_vec()).unwrap();
    let list: Vec<Script> = (1..=130)
        .map(|n| Script {
            id: n,
            name: format!("S{n}"),
            options: 8,
            steps: vec![Step::new(id::BEEP)],
        })
        .collect();
    let out = fmp12::File::from_bytes(save_scripts(&blank, &list).unwrap()).unwrap();
    let es = write::entries(&out).unwrap();
    let order: Vec<&Entry> = es
        .iter()
        .filter(|e| e.path == [vec![0x11], vec![1], vec![3]] && e.item != [0x80])
        .collect();
    assert!(order.last().unwrap().item.ends_with(&[3, 2, 0x80, 2]));
    let last = es
        .iter()
        .find(|e| e.path == [vec![0x11], vec![1]] && e.item.starts_with(&[0x06, 0]))
        .unwrap();
    assert_eq!(last.item, [0x06, 0, 3, 2, 0x80, 2]);
    let back = scripts(&out).unwrap();
    assert_eq!(back.len(), 130);
    assert_eq!((back[129].id, back[129].name.as_str()), (130, "S130"));
}
