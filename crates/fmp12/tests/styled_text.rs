//! Styled text objects (FORMAT.md §10, "Style runs"): run decoding on
//! byte blocks shaped like FileMaker Pro 26's, and writing them back.
#![allow(clippy::unwrap_used)]

use fmp12::layouts::{
    Bounds, CharStyle, LayoutObject, char_runs, face, merge_ref, merge_ref_text, paragraph_aligns,
    paragraph_runs, styled_pieces,
};

/// A `.[1].[255]` block: runs (start, style index), then the styles.
fn block(runs: &[(u32, u32)], styles: &[Vec<u8>]) -> Vec<u8> {
    let mut b = (runs.len() as u32).to_be_bytes().to_vec();
    for (s, i) in runs {
        b.extend(s.to_be_bytes());
        b.extend(i.to_be_bytes());
    }
    b.extend((styles.len() as u32).to_be_bytes());
    for s in styles {
        b.extend(1u32.to_be_bytes());
        b.extend((s.len() as u32).to_be_bytes());
        b.extend(s);
    }
    b
}

/// A character style: mask, font id − 128, face, size, RGBA, 0.
fn cs(mask: u16, font: u16, face: u16, size: u16, rgba: [u8; 4]) -> Vec<u8> {
    let mut v: Vec<u8> = [mask, font, face, size]
        .iter()
        .flat_map(|x| x.to_be_bytes())
        .collect();
    v.extend(rgba);
    v.push(0);
    v
}

fn para(mask: u16, align: u16) -> Vec<u8> {
    let mut v = vec![0u8; 36];
    v[..2].copy_from_slice(&mask.to_be_bytes());
    v[14..18].copy_from_slice(&1f32.to_be_bytes());
    v[28..30].copy_from_slice(&align.to_be_bytes());
    v
}

/// "C <<Name>> end" made in FileMaker: "C " red (Format › Text Colour),
/// "end" bold 24 pt (Ctrl+Shift+B, Format › Size): offsets count the
/// stored reference's characters.
fn sample() -> (String, Vec<u8>) {
    let text = format!("C <<{}>> end", merge_ref_text(130, 2, false));
    let b = block(
        &[(0, 2), (2, 0), (24, 1)],
        &[
            cs(0, 0, 0, 12, [0, 0, 0, 255]),
            cs(0x13, 5, face::BOLD, 24, [0, 0, 0, 255]),
            cs(0x04, 0, 0, 12, [255, 39, 18, 255]),
        ],
    );
    (text, b)
}

#[test]
fn runs_decode() {
    let (text, b) = sample();
    assert_eq!(merge_ref(&text[4..text.len() - 6]), Some((130, 2)));
    let runs = char_runs(&b);
    let red = CharStyle {
        color: Some([255, 39, 18, 255]),
        ..Default::default()
    };
    let bold = CharStyle {
        font: Some(133),
        size: Some(24),
        face: Some(face::BOLD),
        color: None,
    };
    assert_eq!(runs, vec![(0, red), (2, CharStyle::default()), (24, bold)]);
    let pieces = styled_pieces(&text, &runs);
    let texts: Vec<&str> = pieces.iter().map(|p| p.0).collect();
    assert_eq!(texts, ["C ", &text[2..text.len() - 3], "end"]);
    // A run starting inside a merge field starts the whole field.
    let cut = styled_pieces(&text, &[(0, red), (6, bold)]);
    assert_eq!(cut[0].0, "C ");
    assert!(cut[1].0.starts_with("<<"));
}

#[test]
fn paragraph_alignment() {
    let text = "Left\rCenter\rRight";
    let b = block(
        &[(0, 0), (5, 1), (12, 2)],
        &[para(0, 1), para(0x80, 2), para(0x80, 3)],
    );
    let runs = paragraph_runs(&b);
    assert_eq!(runs, vec![(0, 0), (5, 2), (12, 3)]);
    assert_eq!(paragraph_aligns(text, &runs), vec![0, 2, 3]);
    assert_eq!(paragraph_aligns("One", &[]), vec![0]);
}

/// Text objects round-trip through [`fmp12::save::save_layouts`]; retexting
/// one replaces its `.[1]` node instead of leaving it stale.
#[test]
fn styled_text_saves() {
    let f = fmp12::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blank.fmp12")).unwrap();
    let mut ls = fmp12::layouts::layouts(&f).unwrap();
    let (text, b) = sample();
    let o = LayoutObject {
        id: 1,
        kind: 2,
        bounds: Bounds {
            top: 130.0,
            left: 20.0,
            bottom: 180.0,
            right: 420.0,
        },
        text: Some(text.clone()),
        text_runs: vec![(1, b.clone())],
        ..Default::default()
    };
    let plain = LayoutObject {
        id: 2,
        text: Some("Plain".into()),
        text_runs: Vec::new(),
        ..o.clone()
    };
    ls[0].objects = vec![o, plain];
    let g = fmp12::File::from_bytes(fmp12::save::save_layouts(&f, &ls).unwrap()).unwrap();
    let back = fmp12::layouts::layouts(&g).unwrap();
    let got = &back[0].objects;
    assert_eq!(got[0].text.as_deref(), Some(text.as_str()));
    assert_eq!(got[0].text_runs, vec![(1, b)]);
    assert_eq!(got[1].text.as_deref(), Some("Plain"));
    assert!(got[1].text_runs.is_empty());
    // FileMaker spells `.[255]` under `.[1]` in one byte (its store order
    // puts it after the text).
    let mut spelled = Vec::new();
    g.walk(|path, _| {
        if fmp12::int_path(path).is_some_and(|p| p.len() == 7 && p[6] == 255) {
            spelled.push(path[6].to_vec());
        }
    })
    .unwrap();
    assert!(!spelled.is_empty() && spelled.iter().all(|s| s == &[0xFF]));

    // Retexted in Layout mode: plain text again, no runs left behind.
    let mut ls = back.clone();
    ls[0].objects[0].text = Some("Bye".into());
    ls[0].objects[0].text_runs.clear();
    let h = fmp12::File::from_bytes(fmp12::save::save_layouts(&g, &ls).unwrap()).unwrap();
    let again = fmp12::layouts::layouts(&h).unwrap();
    assert_eq!(again[0].objects[0].text.as_deref(), Some("Bye"));
    assert!(again[0].objects[0].text_runs.is_empty());
    let mut stale = false;
    h.walk(|path, _| {
        if let Some(p) = fmp12::int_path(path) {
            stale |= p.len() > 5 && p[..5] == [4, 5, 1, 5, 1] && p[5] == 1;
        }
    })
    .unwrap();
    assert!(!stale, "the `.[1]` node of a retexted object is removed");
}

/// For FileMaker Pro to check: `STYLED_TEXT_OUT=<in.fmp12>|<layout>|<out.fmp12>`
/// retexts the layout's first styled text object (its runs dropped) and
/// adds a copy of it holding [`sample`] (a merge field by reference to
/// field 1 of the layout's occurrence, red and bold 24 pt runs).
#[test]
fn styled_text_oracle_file() {
    let Some(spec) = std::env::var_os("STYLED_TEXT_OUT") else {
        return;
    };
    let spec = spec.to_string_lossy().to_string();
    let [input, layout, out] = spec.split('|').collect::<Vec<_>>()[..] else {
        return;
    };
    let f = fmp12::File::open(input).unwrap();
    let mut ls = fmp12::layouts::layouts(&f).unwrap();
    let l = ls.iter_mut().find(|l| l.name == layout).unwrap();
    let to = (l.table_occurrence & 0xFFF) as u32;
    let i = l
        .objects
        .iter()
        .position(|o| !o.text_runs.is_empty())
        .unwrap();
    let mut copy = l.objects[i].clone();
    l.objects[i].text = Some("Retexted by BaseCraft".into());
    l.objects[i].text_runs.clear();
    let (_, b) = sample();
    copy.id = l.objects.iter().map(|o| o.id).max().unwrap() + 1;
    copy.text = Some(format!("C <<{}>> end", merge_ref_text(to, 1, false)));
    copy.text_runs = vec![(1, b)];
    copy.bounds.left += 450.0;
    copy.bounds.right += 450.0;
    l.objects.push(copy);
    std::fs::write(out, fmp12::save::save_layouts(&f, &ls).unwrap()).unwrap();
}
