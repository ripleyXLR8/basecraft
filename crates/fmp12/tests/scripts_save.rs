//! Script writer on the FileMaker starter templates (skipped when they are
//! not installed; override the directory with FMP12_TEMPLATES).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use fmp12::script_calc::{Resolve, compile};
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
            let (mut got, _) = step_entries(&s, last);
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

struct Names(
    Vec<fmp12::schema::Table>,
    Vec<fmp12::graph::TableOccurrence>,
);

impl Resolve for Names {
    fn field(&self, to: Option<&str>, name: &str) -> Option<(u32, u32)> {
        let o = self.1.iter().find(|t| Some(t.name.as_str()) == to)?;
        let t = self.0.iter().find(|t| t.id == o.base_table)?;
        let (f, _) = t.fields.iter().find(|(_, f)| f.name == name)?;
        Some((o.id, *f))
    }
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
    let names = Names(
        fmp12::schema::tables(&file).unwrap(),
        fmp12::graph::table_occurrences(&file).unwrap(),
    );
    let calc = |s: &str| Calc(compile(s, 0, &names).unwrap().tokens);
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
    let fnames = Names(names.0.clone(), names.1.clone());
    struct N<'a>(&'a Names);
    impl fmp12::calc::Names for N<'_> {
        fn field(&self, to: u32, f: u32) -> Option<String> {
            let bt = self.0.1.iter().find(|t| t.id == to)?.base_table;
            Some(
                self.0
                    .0
                    .iter()
                    .find(|t| t.id == bt)?
                    .fields
                    .get(&f)?
                    .name
                    .clone(),
            )
        }
        fn occurrence(&self, to: u32) -> Option<String> {
            Some(self.0.1.iter().find(|t| t.id == to)?.name.clone())
        }
    }
    assert_eq!(
        value.as_ref().unwrap().text(&N(&fnames)).unwrap(),
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
    assert_eq!(
        title.as_ref().unwrap().text(&N(&fnames)).unwrap(),
        "\"Hello\""
    );
    assert_eq!(
        new.steps[1].params,
        StepParams::GoToLayout {
            dest: LayoutDest::Layout(2),
            animation: 0
        }
    );
    let _ = FieldRef::default();
}
