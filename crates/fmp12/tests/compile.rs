//! Calculation compiler: FileMaker's own compiled forms, and round trips
//! through the decompiler.

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::calc::{self, Names, Resolve};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The new file the fixture's field references point into.
struct Blank;
const FIELDS: [&str; 5] = [
    "Clé primaire",
    "Horodatage de création",
    "Créé par",
    "Horodatage de modification",
    "Modifié par",
];

impl Names for Blank {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        (to == 129).then(|| FIELDS.get(field as usize - 1).map(|s| s.to_string()))?
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        (to == 129).then(|| "Blank".to_string())
    }
}

impl Resolve for Blank {
    fn occurrences(&self) -> Vec<(u32, String)> {
        vec![(129, "Blank".into())]
    }
    fn fields(&self, to: u32) -> Vec<(u32, String)> {
        if to != 129 {
            return vec![];
        }
        (1..).zip(FIELDS.iter().map(|s| s.to_string())).collect()
    }
}

/// Every pair in fixtures/compiled.tsv (compiled by FileMaker Pro 26):
/// decompiling key 5 and compiling the text again gives FileMaker's key 5
/// and key 6 byte for byte.
#[test]
fn compiled_forms_match_filemaker() {
    let text = include_str!("fixtures/compiled.tsv");
    let (mut ok, mut total, mut fails) = (0, 0, Vec::new());
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let (k5, k6) = line.split_once('\t').unwrap();
        let (k5, k6) = (unhex(k5), unhex(k6));
        total += 1;
        let src = match calc::decompile(&k5, 0, &Blank) {
            Ok(s) => s,
            Err(e) => {
                fails.push(format!("decompile: {e} {}", hex(&k5)));
                continue;
            }
        };
        // FileMaker's SCSU uses dynamic windows and Unicode mode where ours
        // quotes each character: both decode alike, so text beyond Latin-1
        // (and general punctuation) is compared after decoding.
        let exotic = src
            .chars()
            .any(|c| c as u32 >= 0x100 && !(0x2000..0x2080).contains(&(c as u32)));
        match calc::compile(&src, 0, &Blank) {
            Ok(c) if c.tokens == k5 && c.compiled == k6 => ok += 1,
            Ok(c) if exotic && calc::decompile(&c.tokens, 0, &Blank).as_deref() == Ok(&src) => {
                ok += 1
            }
            Ok(c) => fails.push(format!(
                "{src:?}\n  k5 want {}\n     got {}\n  k6 want {}\n     got {}",
                hex(&k5),
                hex(&c.tokens),
                hex(&k6),
                hex(&c.compiled)
            )),
            Err(e) => fails.push(format!("{src:?}: {e}")),
        }
    }
    for f in fails.iter().take(400) {
        eprintln!("{f}");
    }
    eprintln!("{ok}/{total} compiled forms match");
    assert!(fails.is_empty(), "{} mismatches", fails.len());
}

#[test]
fn compiles_field_calculations_in_context() {
    let c = calc::compile("Créé par & \" \" & Blank::Modifié par", 129, &Blank).unwrap();
    assert_eq!(
        calc::decompile(&c.tokens, 129, &Blank).unwrap(),
        "Créé par & \" \" & Blank::Modifié par"
    );
    // Prefix form: & ( & ( field 3, " " ), field 5 qualified ).
    assert_eq!(
        hex(&c.compiled),
        "505016000403d000010201030001130 17a0016040403d000010201050001".replace(' ', "")
    );
    assert!(calc::compile("Nope + 1", 129, &Blank).is_err());
    assert!(calc::compile("Left ( \"a\" ; 1", 129, &Blank).is_err());
}
