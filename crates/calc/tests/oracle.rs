//! Expressions evaluated by FileMaker Pro 26 (tools/oracle/calc_oracle.ps1)
//! and checked against `calc`. oracle.tsv: expression TAB result [TAB
//! EvaluationError when not 0, `?` when EvaluationError itself failed];
//! results escape \\ \t \r \n. The results come from a copy of the English
//! Tasks starter template (en_US formats). Rows after the "# known
//! differences" line are reported, not failed.

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

#[test]
fn filemaker_oracle() {
    let tsv = include_str!("oracle.tsv");
    let (mut n, mut known, mut failures, mut still_differ) = (0, false, vec![], 0);
    for line in tsv.lines() {
        if line.starts_with("# known differences") {
            known = true;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let src = cols.next().unwrap();
        let want = unescape(cols.next().unwrap_or(""));
        let want_err = cols.next().unwrap_or("0");
        n += 1;
        let mut ctx = calc::MemoryContext::default();
        let (got, err) = match calc::evaluate(src, &mut ctx) {
            Ok(v) => (v.to_text(), 0),
            Err(e) => ("?".to_string(), e.code),
        };
        let err_ok = err.to_string() == want_err || (want_err == "?" && err != 0);
        if got == want && err_ok {
            continue;
        }
        if known {
            still_differ += 1;
        } else {
            failures.push(format!(
                "{src}\n    FileMaker {want:?} (error {want_err})\n    calc      {got:?} (error {err})"
            ));
        }
    }
    println!("{n} oracle rows; {still_differ} known differences");
    assert!(
        failures.is_empty(),
        "{} of {n} oracle rows differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
