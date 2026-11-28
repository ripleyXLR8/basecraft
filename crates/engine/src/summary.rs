//! Summary fields: Total, Average, Count, Minimum, Maximum, Standard
//! Deviation, Fraction of Total and List, over a found set or over the
//! groups a sorted found set forms on a break field.

use std::ops::Range;

use calc::Value;
use calc::num::{self, Num};
use fmp12::schema::{Summary, SummaryOp};

use crate::Database;

/// A summary field's definition and options.
struct Def {
    s: Summary,
    /// Running total or count.
    running: bool,
    /// Restart a running summary for each sorted group of `by`.
    restart: bool,
    /// Standard deviation by population.
    population: bool,
    /// Sort field (restart, subtotal) or weight (average).
    by: Option<u32>,
}

fn def(db: &Database, table: u32, field: u32) -> Option<Def> {
    let f = db.table(table)?.field(field)?;
    let s = f.def.summary().filter(|s| s.field != field)?;
    let flag = f.def.summary_running();
    let counts = matches!(s.op, SummaryOp::Total | SummaryOp::Count);
    Some(Def {
        running: flag && counts,
        restart: counts && f.def.summary_restart(),
        population: f.def.summary_population(),
        by: f.def.summary_by(),
        s,
    })
}

/// Value of summary field `field` of `table` over the records `group`.
/// `found` is the whole found set, which Fraction of Total divides by.
/// Not a summary field: empty.
pub fn summarize(db: &Database, table: u32, field: u32, group: &[usize], found: &[usize]) -> Value {
    summarize_sorted(db, table, field, group, found, &[])
}

/// Like [`summarize`], for a found set sorted by the fields `sort` (in
/// order): a running summary restarts, and a fraction of total is
/// subtotaled, only when the found set is sorted by its second field. A
/// running summary over a group shows its value at the group's last
/// record, as in a sub-summary part.
pub fn summarize_sorted(
    db: &Database,
    table: u32,
    field: u32,
    group: &[usize],
    found: &[usize],
    sort: &[u32],
) -> Value {
    let Some(d) = def(db, table, field) else {
        return Value::empty();
    };
    if d.running
        && let Some(pos) = group.last().and_then(|l| found.iter().position(|r| r == l))
    {
        let start =
            d.by.filter(|_| d.restart)
                .and_then(|b| sorted_group(db, table, found, sort, b, pos))
                .map_or(0, |g| g.start);
        return plain(db, table, &d, &found[start..=pos], found, sort);
    }
    plain(db, table, &d, group, found, sort)
}

/// What summary field `field` shows on the body of record `record` of a
/// found set sorted by `sort`: the running value up to the record, the
/// record's fraction of the total, or the summary of the found set.
pub fn summary_at(
    db: &Database,
    table: u32,
    field: u32,
    record: usize,
    found: &[usize],
    sort: &[u32],
) -> Value {
    let Some(d) = def(db, table, field) else {
        return Value::empty();
    };
    let own = d.running || d.s.op == SummaryOp::FractionOfTotal;
    if own && found.contains(&record) {
        summarize_sorted(db, table, field, &[record], found, sort)
    } else {
        summarize_sorted(db, table, field, found, found, sort)
    }
}

/// The sorted group of break field `by` holding `found[pos]`, if the
/// found set is sorted by `by`.
fn sorted_group(
    db: &Database,
    table: u32,
    found: &[usize],
    sort: &[u32],
    by: u32,
    pos: usize,
) -> Option<Range<usize>> {
    let level = sort.iter().position(|f| *f == by)?;
    groups(db, table, found, &sort[..=level])
        .into_iter()
        .find(|g| g.contains(&pos))
}

fn plain(
    db: &Database,
    table: u32,
    d: &Def,
    group: &[usize],
    found: &[usize],
    sort: &[u32],
) -> Value {
    let s = d.s;
    let values = |set: &[usize]| -> Vec<Value> {
        set.iter()
            .map(|&r| db.value(table, r, s.field))
            .filter(|v| !v.is_empty())
            .collect()
    };
    let nums =
        |set: &[usize]| -> Vec<Num> { values(set).iter().filter_map(Value::to_num).collect() };
    let sum = |v: &[Num]| v.iter().fold(num::int(0), |a, b| a + b);
    let vals = values(group);
    match s.op {
        SummaryOp::Total => Value::Number(num::fix(sum(&nums(group)))),
        SummaryOp::Count => Value::Number(num::int(vals.len() as i64)),
        SummaryOp::Average => {
            // Weighted: sum(x × w) / sum(w) over the records with both.
            let (top, weight) = match d.by {
                Some(w) => group
                    .iter()
                    .filter_map(|&r| {
                        let x = db.value(table, r, s.field).to_num()?;
                        let w = db.value(table, r, w).to_num()?;
                        Some((&x * &w, w))
                    })
                    .fold((num::int(0), num::int(0)), |(a, b), (xw, w)| {
                        (a + xw, b + w)
                    }),
                None => {
                    let n = nums(group);
                    (sum(&n), num::int(n.len() as i64))
                }
            };
            num::div(&top, &weight, num::DEFAULT_SCALE).map_or_else(Value::empty, Value::Number)
        }
        SummaryOp::Minimum | SummaryOp::Maximum => {
            let pick = vals.into_iter().reduce(|a, b| {
                let less = calc::eval::compare(&b, &a) == std::cmp::Ordering::Less;
                if less == (s.op == SummaryOp::Minimum) {
                    b
                } else {
                    a
                }
            });
            pick.unwrap_or_else(Value::empty)
        }
        // Sample standard deviation, or by population (divided by n).
        SummaryOp::StandardDeviation => {
            let n = nums(group);
            let div = n.len() as i64 - if d.population { 0 } else { 1 };
            if div < 1 || n.len() < 2 {
                return Value::Number(num::int(0));
            }
            let scale = 2 * num::DEFAULT_SCALE;
            let len = num::int(n.len() as i64);
            let mean = num::div(&sum(&n), &len, scale).unwrap_or_default();
            let squares: Vec<Num> = n.iter().map(|x| (x - &mean) * (x - &mean)).collect();
            num::div(&sum(&squares), &num::int(div), scale)
                .and_then(|var| num::sqrt(&var, num::DEFAULT_SCALE))
                .map_or_else(Value::empty, Value::Number)
        }
        SummaryOp::FractionOfTotal => {
            // Subtotaled: divided by the total of the sorted groups of
            // `by` the records belong to; empty when the found set is not
            // sorted by `by`, or with no value (as FileMaker shows them).
            let n = nums(group);
            if n.is_empty() {
                return Value::empty();
            }
            let total: Vec<usize> = match d.by {
                Some(b) => {
                    let Some(level) = sort.iter().position(|f| *f == b) else {
                        return Value::empty();
                    };
                    let pos: Vec<usize> = group
                        .iter()
                        .filter_map(|r| found.iter().position(|x| x == r))
                        .collect();
                    groups(db, table, found, &sort[..=level])
                        .into_iter()
                        .filter(|g| pos.iter().any(|p| g.contains(p)))
                        .flat_map(|g| found[g].to_vec())
                        .collect()
                }
                None => found.to_vec(),
            };
            num::div(&sum(&n), &sum(&nums(&total)), num::DEFAULT_SCALE)
                .map_or_else(Value::empty, Value::Number)
        }
        SummaryOp::List => Value::text(
            vals.iter()
                .map(Value::to_text)
                .collect::<Vec<_>>()
                .join("\r"),
        ),
    }
}

/// Runs of consecutive records of a sorted found set with the same values
/// of `fields` (a break field and the sort fields before it), as index
/// ranges into `found`.
pub fn groups(db: &Database, table: u32, found: &[usize], fields: &[u32]) -> Vec<Range<usize>> {
    let key = |r: usize| -> Vec<Value> { fields.iter().map(|f| db.value(table, r, *f)).collect() };
    let same = |a: &[Value], b: &[Value]| {
        a.iter()
            .zip(b)
            .all(|(x, y)| calc::eval::compare(x, y) == std::cmp::Ordering::Equal)
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut prev: Option<Vec<Value>> = None;
    for (i, &r) in found.iter().enumerate() {
        let k = key(r);
        match (&prev, out.last_mut()) {
            (Some(p), Some(last)) if same(p, &k) => last.end = i + 1,
            _ => out.push(i..i + 1),
        }
        prev = Some(k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{AMOUNT, CITY, T, sample, summary_field};

    #[test]
    fn summary_operations() {
        // Amounts 10, 20, 30, 40, empty; cities Rome, Paris, Rome, Oslo, Rome.
        let cases = [
            (SummaryOp::Total, "100"),
            (SummaryOp::Count, "4"),
            (SummaryOp::Average, "25"),
            (SummaryOp::Minimum, "10"),
            (SummaryOp::Maximum, "40"),
            (SummaryOp::StandardDeviation, "12.9099444873580563"),
            (SummaryOp::FractionOfTotal, "1"),
            (SummaryOp::List, "10\r20\r30\r40"),
        ];
        for (op, want) in cases {
            let mut db = sample();
            let f = summary_field(&mut db, op, AMOUNT);
            let all: Vec<usize> = (0..5).collect();
            let got = summarize(&db, T, f, &all, &all).to_text();
            assert_eq!(got, want, "{op:?}");
        }
        let mut db = sample();
        let f = summary_field(&mut db, SummaryOp::FractionOfTotal, AMOUNT);
        let all: Vec<usize> = (0..5).collect();
        assert_eq!(summarize(&db, T, f, &[1], &all).to_text(), ".2"); // FileMaker shows no leading zero
    }

    #[test]
    fn groups_follow_break_values() {
        let db = sample();
        let mut found: Vec<usize> = (0..5).collect();
        crate::sort(&db, T, &mut found, &[(CITY, crate::Order::Ascending)]);
        let g = groups(&db, T, &found, &[CITY]);
        assert_eq!(g, vec![0..1, 1..2, 2..5]);
    }

    /// Summary field of `op` over `field` with option byte `flags` and
    /// second field `by`, encoded as FileMaker stores them (FORMAT.md §5).
    fn option_summary(
        db: &mut Database,
        name: &str,
        op: u8,
        field: u32,
        flags: u8,
        by: u32,
    ) -> u32 {
        let t = &mut db.tables[0];
        let id = t.fields.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        let mut f = crate::tests::field(id, name, 3, 2);
        let mut raw = vec![
            op,
            flags,
            2,
            1,
            0,
            0,
            0,
            0,
            2,
            1,
            field as u8,
            0,
            0,
            0,
            0,
            1,
        ];
        raw.extend([2, 1, 0, 0, 0, 0x7F, 2, 1, by as u8, 0, 0, 0, 0, 1]);
        f.def.summary_raw = raw;
        t.fields.push(f);
        id
    }

    /// The file of `tools/oracle/summary_demo.sh`, and the values FileMaker
    /// Pro 26 showed in its Table view (unsorted, then sorted by Region),
    /// GetSummary by Region giving the sub-summary values.
    #[test]
    fn summary_options_match_filemaker() {
        use crate::tests::field;
        let (region, amount, weight) = (1, 2, 3);
        let mut db = sample();
        let mut t = crate::Table::new(
            T,
            "Sales",
            vec![
                field(region, "Region", 1, 1),
                field(amount, "Amount", 1, 2),
                field(weight, "Weight", 1, 2),
            ],
        );
        let rows = [
            ("A", "10", "1"),
            ("B", "5", "4"),
            ("A", "30", "2"),
            ("C", "40", "2"),
            ("B", "15", "1"),
            ("C", "", "1"),
            ("A", "20", "3"),
        ];
        for (g, a, w) in rows {
            let r = t.new_record();
            t.records[r].values = [(region, g), (amount, a), (weight, w)]
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| (k, v.to_string()))
                .collect();
        }
        db.tables[0] = t;
        let run_tot = option_summary(&mut db, "RunTot", 1, amount, 0x04, 0);
        let run_tot_r = option_summary(&mut db, "RunTotR", 1, amount, 0x15, region);
        let run_cnt = option_summary(&mut db, "RunCnt", 3, amount, 0x04, 0);
        let run_cnt_r = option_summary(&mut db, "RunCntR", 3, amount, 0x15, region);
        let w_avg = option_summary(&mut db, "WAvg", 2, amount, 0x01, weight);
        let frac = option_summary(&mut db, "Frac", 7, amount, 0, 0);
        let frac_sub = option_summary(&mut db, "FracSub", 7, amount, 0x01, region);
        let sd = option_summary(&mut db, "SD", 6, amount, 0, 0);
        let sdp = option_summary(&mut db, "SDP", 6, amount, 0x01, 0);
        let num = |v: Value| {
            v.to_num()
                .map(|n| n.to_string().parse::<f64>().unwrap_or(f64::NAN))
        };
        let check = |got: Value, want: Option<f64>, what: &str| match (num(got.clone()), want) {
            (Some(g), Some(w)) => assert!((g - w).abs() < 1e-7, "{what}: {g} ≠ {w}"),
            (None, None) => {}
            _ => panic!("{what}: {} ≠ {want:?}", got.to_text()),
        };
        // Unsorted: no restart, no subtotal.
        let all: Vec<usize> = (0..7).collect();
        for (r, (tot, frac_v)) in [10.0, 15.0, 45.0, 85.0, 100.0, 100.0, 120.0]
            .iter()
            .zip([
                Some(10.0 / 120.0),
                Some(5.0 / 120.0),
                Some(0.25),
                Some(1.0 / 3.0),
                Some(0.125),
                None,
                Some(1.0 / 6.0),
            ])
            .enumerate()
        {
            check(
                summary_at(&db, T, run_tot, r, &all, &[]),
                Some(*tot),
                "RunTot",
            );
            check(
                summary_at(&db, T, run_tot_r, r, &all, &[]),
                Some(*tot),
                "RunTotR",
            );
            check(summary_at(&db, T, frac, r, &all, &[]), frac_v, "Frac");
            check(summary_at(&db, T, frac_sub, r, &all, &[]), None, "FracSub");
            check(
                summary_at(&db, T, w_avg, r, &all, &[]),
                Some(245.0 / 13.0),
                "WAvg",
            );
            check(
                summary_at(&db, T, sd, r, &all, &[]),
                Some(13.0384048104),
                "SD",
            );
            check(
                summary_at(&db, T, sdp, r, &all, &[]),
                Some(11.9023807142),
                "SDP",
            );
        }
        // Sorted by Region: A10 A30 A20 B5 B15 C40 C(empty).
        let mut found = all.clone();
        crate::sort(&db, T, &mut found, &[(region, crate::Order::Ascending)]);
        assert_eq!(found, vec![0, 2, 6, 1, 4, 3, 5]);
        let sort = [region];
        let body = [
            (10.0, 10.0, 1.0, 1.0, Some(1.0 / 6.0)),
            (40.0, 40.0, 2.0, 2.0, Some(0.5)),
            (60.0, 60.0, 3.0, 3.0, Some(1.0 / 3.0)),
            (65.0, 5.0, 4.0, 1.0, Some(0.25)),
            (80.0, 20.0, 5.0, 2.0, Some(0.75)),
            (120.0, 40.0, 6.0, 1.0, Some(1.0)),
            (120.0, 40.0, 6.0, 1.0, None),
        ];
        for (&r, (tot, tot_r, cnt, cnt_r, sub)) in found.iter().zip(body) {
            check(
                summary_at(&db, T, run_tot, r, &found, &sort),
                Some(tot),
                "RunTot",
            );
            check(
                summary_at(&db, T, run_tot_r, r, &found, &sort),
                Some(tot_r),
                "RunTotR",
            );
            check(
                summary_at(&db, T, run_cnt, r, &found, &sort),
                Some(cnt),
                "RunCnt",
            );
            check(
                summary_at(&db, T, run_cnt_r, r, &found, &sort),
                Some(cnt_r),
                "RunCntR",
            );
            check(
                summary_at(&db, T, frac_sub, r, &found, &sort),
                sub,
                "FracSub",
            );
        }
        // Sub-summaries by Region (GetSummary): A, B, C.
        let sub = [
            (0..3, 60.0, 60.0, 3.0, 130.0 / 6.0, 0.5, 8.16496581),
            (3..5, 80.0, 20.0, 2.0, 7.0, 1.0 / 6.0, 5.0),
            (5..7, 120.0, 40.0, 1.0, 40.0, 1.0 / 3.0, 0.0),
        ];
        for (g, tot, tot_r, cnt_r, wa, fr, sp) in sub {
            let group = &found[g];
            let s = |f: u32| summarize_sorted(&db, T, f, group, &found, &sort);
            check(s(run_tot), Some(tot), "GetSummary RunTot");
            check(s(run_tot_r), Some(tot_r), "GetSummary RunTotR");
            check(s(run_cnt_r), Some(cnt_r), "GetSummary RunCntR");
            check(s(w_avg), Some(wa), "GetSummary WAvg");
            check(s(frac), Some(fr), "GetSummary Frac");
            check(s(frac_sub), Some(1.0), "GetSummary FracSub");
            check(s(sdp), Some(sp), "GetSummary SDP");
        }
        // In a calculation: over the window's found set, by the break
        // field only when sorted by it (empty otherwise, as in FileMaker).
        let v = db.eval_source("GetSummary ( RunTotR ; Region )", T, T, Some(4));
        check(v, None, "GetSummary unsorted");
        let scope = crate::Scope {
            found: Some(crate::FoundSet {
                table: T,
                records: &found,
                sort: &sort,
            }),
            self_value: None,
        };
        let ev = |src: &str, r: usize| {
            db.eval_source_in(src, T, T, Some(r), &scope)
                .unwrap_or_else(|e| Value::text(format!("error {}", e.code)))
        };
        check(
            ev("GetSummary ( RunTotR ; Region )", 4),
            Some(20.0),
            "by Region",
        );
        check(
            ev("GetSummary ( RunTot ; RunTot )", 4),
            Some(120.0),
            "grand",
        );
        // Record 4 (B15) is the 5th of the sorted found set.
        check(ev("Get ( RecordNumber )", 4), Some(5.0), "RecordNumber");
        check(ev("RunTot", 4), Some(80.0), "running total");
        check(
            ev("GetNthRecord ( Amount ; 2 )", 4),
            Some(30.0),
            "GetNthRecord",
        );
        let few = [6, 0];
        let scope = crate::Scope {
            found: Some(crate::FoundSet {
                table: T,
                records: &few,
                sort: &[],
            }),
            self_value: None,
        };
        let ev = |src: &str| db.eval_source_in(src, T, T, Some(0), &scope);
        assert_eq!(ev("Get ( FoundCount )").unwrap().to_text(), "2");
        assert_eq!(ev("Get ( TotalRecordCount )").unwrap().to_text(), "7");
        assert_eq!(
            ev("GetSummary ( RunTot ; RunTot )").unwrap().to_text(),
            "30"
        );
        assert_eq!(
            ev("GetRecordIDsFromFoundSet ( 0 )").unwrap().to_text(),
            "7\r1"
        );
    }
}
