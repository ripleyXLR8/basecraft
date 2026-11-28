//! Coverage of the FileMaker Pro 26 inventories in docs/reference/ that
//! no single crate owns (docs/COVERAGE.md): script triggers, layout object
//! kinds, field control styles and error codes. `cargo test -p basecraft
//! coverage -- --nocapture` prints the lists; the counts are asserted so
//! COVERAGE.md stays honest.

use std::path::{Path, PathBuf};

const TRIGGERS: &str = include_str!("../../../docs/reference/script_triggers.txt");
const OBJECTS: &str = include_str!("../../../docs/reference/layout_objects.txt");
const CONTROLS: &str = include_str!("../../../docs/reference/control_styles.txt");
const ERRORS: &str = include_str!("../../../docs/reference/error_codes.txt");

/// `id<TAB>name` lines, comments skipped.
fn entries(list: &str) -> Vec<(&str, &str)> {
    list.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split_once('\t'))
        .collect()
}

/// The non-test source of a crate's `src` (each file cut at its first
/// `#[cfg(test)]`; files named `*tests*` skipped).
fn source(krate: &str) -> String {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "tests") {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs")
                && !p.to_string_lossy().contains("tests")
            {
                out.push(p);
            }
        }
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(krate)
        .join("src");
    let mut files = Vec::new();
    walk(&dir, &mut files);
    assert!(!files.is_empty(), "no source under {}", dir.display());
    files
        .iter()
        .map(|p| {
            let s = std::fs::read_to_string(p).unwrap();
            // Cut at the first inline test module.
            let cut = s
                .match_indices("#[cfg(test)]")
                .map(|(i, _)| i)
                .find(|&i| {
                    let next = s[i..].lines().nth(1).unwrap_or("");
                    next.contains("mod ") && next.trim_end().ends_with('{')
                })
                .unwrap_or(s.len());
            s[..cut].to_string()
        })
        .collect()
}

/// `OnObjectAVPlayerChange` -> `OBJECT_AVPLAYER_CHANGE` (fmp12::triggers::event).
fn const_name(trigger: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in trigger.trim_start_matches("On").chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase();
        out.push(c.to_ascii_uppercase());
    }
    out
}

/// A trigger is implemented when the application fires it.
#[test]
fn trigger_coverage() {
    let app = source("app");
    let (mut fired, mut missing) = (Vec::new(), Vec::new());
    for (id, name) in entries(TRIGGERS) {
        let id: u16 = id.parse().unwrap();
        assert_eq!(fmp12::triggers::event_name(id), Some(name), "event {id}");
        if app.contains(&format!("event::{}", const_name(name))) {
            fired.push(format!("{id} {name}"));
        } else {
            missing.push(format!("{id} {name}"));
        }
    }
    println!("FIRED {}: {}", fired.len(), fired.join(", "));
    println!("NOT FIRED {}: {}", missing.len(), missing.join(", "));
    assert_eq!((fired.len(), missing.len()), (22, 4));
}

/// Layout object kinds: implemented = read, drawn in Browse mode and made
/// in Layout mode; partial as noted.
const PARTIAL_OBJECTS: [(&str, &str); 4] = [
    ("Graphic", "drawn; no Insert › Picture in Layout mode"),
    ("Rounded Rectangle", "drawn as read; no tool makes one"),
    (
        "Web Viewer",
        "placeholder box with its address; no web content",
    ),
    ("Chart", "drawn; no Chart tool in Layout mode"),
];

#[test]
fn layout_object_coverage() {
    let all = entries(OBJECTS);
    for (name, _) in PARTIAL_OBJECTS {
        assert!(all.iter().any(|e| e.1 == name), "{name} not in the list");
    }
    let done: Vec<&str> = all
        .iter()
        .map(|e| e.1)
        .filter(|n| PARTIAL_OBJECTS.iter().all(|p| p.0 != *n))
        .collect();
    println!("IMPLEMENTED {}: {}", done.len(), done.join(", "));
    println!("PARTIAL {}: {PARTIAL_OBJECTS:?}", PARTIAL_OBJECTS.len());
    assert_eq!((all.len(), done.len()), (19, 15));
}

/// Field control styles: the Inspector's list (`design::CONTROLS`) offers
/// them and Browse mode draws them; partial = drawn without some of their
/// options (checkbox and radio button sets: no "Other…" item, no "Allow
/// editing").
const PARTIAL_CONTROLS: [&str; 2] = ["Checkbox Set", "Radio Button Set"];

#[test]
fn control_style_coverage() {
    let (mut done, mut partial, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for (_, name) in entries(CONTROLS) {
        let offered = crate::design::CONTROLS
            .iter()
            .any(|c| c.2.eq_ignore_ascii_case(name));
        match (offered, PARTIAL_CONTROLS.contains(&name)) {
            (true, false) => done.push(name),
            (true, true) => partial.push(name),
            _ => missing.push(name),
        }
    }
    println!("IMPLEMENTED {}: {}", done.len(), done.join(", "));
    println!("PARTIAL {}: {}", partial.len(), partial.join(", "));
    println!("MISSING {}: {}", missing.len(), missing.join(", "));
    assert_eq!((done.len(), partial.len(), missing.len()), (5, 2, 0));
}

/// An error code is raised when its number appears in an error position of
/// the calc, engine, script or app source: a `u32` constant, `Err(n)`,
/// `EvalError::new(n` or a validation rule's code. A heuristic: it can
/// count a `u32` constant that is not an error.
#[test]
fn error_code_coverage() {
    let src: String = ["calc", "engine", "script", "app"]
        .iter()
        .map(|k| source(k))
        .collect();
    let mut raised = std::collections::BTreeSet::new();
    for pat in [": u32 = ", "Err(", "EvalError::new(", " => "] {
        for (i, _) in src.match_indices(pat) {
            let rest = &src[i + pat.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let after = rest[digits.len()..].chars().next();
            // `=> n` counts only inside `Rule::… => n` arms.
            let rule = pat != " => "
                || src[..i]
                    .rsplit('\n')
                    .next()
                    .is_some_and(|l| l.trim_start().starts_with("Rule::"));
            if !digits.is_empty() && rule && matches!(after, Some(';' | ')' | ',')) {
                raised.insert(digits.parse::<i64>().unwrap());
            }
        }
    }
    let codes: Vec<i64> = ERRORS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.trim().parse().unwrap())
        .collect();
    let (done, missing): (Vec<i64>, Vec<i64>) =
        codes.iter().partition(|c| **c == 0 || raised.contains(c));
    println!("RAISED {}: {done:?}", done.len());
    println!("NEVER RAISED {}: {missing:?}", missing.len());
    assert_eq!(codes.len(), 300);
    assert_eq!((done.len(), missing.len()), (59, 241));
}
