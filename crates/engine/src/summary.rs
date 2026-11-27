//! Summary fields: Total, Average, Count, Minimum, Maximum, Standard
//! Deviation, Fraction of Total and List, over a found set or over the
//! groups a sorted found set forms on a break field.

use std::ops::Range;

use calc::Value;
use calc::num::{self, Num};
use fmp12::schema::SummaryOp;

use crate::Database;

/// Value of summary field `field` of `table` over the records `group`.
/// `found` is the whole found set, which Fraction of Total divides by.
/// Not a summary field: empty.
pub fn summarize(db: &Database, table: u32, field: u32, group: &[usize], found: &[usize]) -> Value {
    let Some(s) = db
        .table(table)
        .and_then(|t| t.field(field))
        .and_then(|f| f.def.summary())
    else {
        return Value::empty();
    };
    if s.field == field {
        return Value::empty();
    }
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
            let n = nums(group);
            num::div(&sum(&n), &num::int(n.len() as i64), num::DEFAULT_SCALE)
                .map_or_else(Value::empty, Value::Number)
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
        // Sample standard deviation (FileMaker's default; "by population" is not decoded).
        SummaryOp::StandardDeviation => {
            let n = nums(group);
            if n.len() < 2 {
                return Value::Number(num::int(0));
            }
            let scale = 2 * num::DEFAULT_SCALE;
            let len = num::int(n.len() as i64);
            let mean = num::div(&sum(&n), &len, scale).unwrap_or_default();
            let squares: Vec<Num> = n.iter().map(|x| (x - &mean) * (x - &mean)).collect();
            num::div(&sum(&squares), &num::int(n.len() as i64 - 1), scale)
                .and_then(|var| num::sqrt(&var, num::DEFAULT_SCALE))
                .map_or_else(Value::empty, Value::Number)
        }
        SummaryOp::FractionOfTotal => {
            num::div(&sum(&nums(group)), &sum(&nums(found)), num::DEFAULT_SCALE)
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
}
