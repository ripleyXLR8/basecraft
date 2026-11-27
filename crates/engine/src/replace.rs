//! Edit › Find/Replace and Records › Replace Field Contents.
//!
//! Find/Replace looks for text in the stored values of a record's fields
//! (the fields of the layout, in their order), across the current record
//! or every record of the found set, with FileMaker's options: match case,
//! whole words only, and the direction (forward, backward, all = wrap
//! around). Replace Field Contents sets one field of every record of the
//! found set to a value, a serial number or a calculation.

use std::ops::Range;

use fmp12::schema::FieldKind;

use crate::{Database, stored_text};

/// What Find/Replace looks for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FindReplace {
    pub find: String,
    pub replace: String,
    pub match_case: bool,
    pub whole_word: bool,
}

/// Search direction of Find Next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    Forward,
    Backward,
    /// Forward, starting again from the first record after the last one.
    #[default]
    All,
}

/// A match: position in the searched records, field, byte range in its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub pos: usize,
    pub field: u32,
    pub range: Range<usize>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl FindReplace {
    /// Byte ranges of the non-overlapping matches in `text`.
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        let pat: Vec<char> = self.find.chars().collect();
        if pat.is_empty() {
            return Vec::new();
        }
        let same =
            |a: char, b: char| a == b || !self.match_case && a.to_lowercase().eq(b.to_lowercase());
        let mut out = Vec::new();
        let mut from = 0;
        for (i, _) in text.char_indices() {
            if i < from {
                continue;
            }
            let mut end = i;
            let mut chars = text[i..].chars();
            let ok = pat.iter().all(|&p| match chars.next() {
                Some(c) if same(c, p) => {
                    end += c.len_utf8();
                    true
                }
                _ => false,
            });
            if !ok {
                continue;
            }
            if self.whole_word {
                let before = text[..i].chars().next_back().is_some_and(is_word);
                let after = text[end..].chars().next().is_some_and(is_word);
                if before || after {
                    continue;
                }
            }
            out.push(i..end);
            from = end;
        }
        out
    }

    /// `text` with every match replaced, and the number of matches.
    pub fn replace_in(&self, text: &str) -> (String, usize) {
        let m = self.matches(text);
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for r in &m {
            out.push_str(&text[last..r.start]);
            out.push_str(&self.replace);
            last = r.end;
        }
        out.push_str(&text[last..]);
        (out, m.len())
    }
}

/// The fields of `fields` Find/Replace can change: normal (not calculated
/// or summary) fields of `table`.
fn searchable(db: &Database, table: u32, fields: &[u32]) -> Vec<u32> {
    let Some(t) = db.table(table) else {
        return Vec::new();
    };
    fields
        .iter()
        .copied()
        .filter(|f| t.field(*f).is_some_and(|f| f.kind == FieldKind::Normal))
        .collect()
}

fn stored(db: &Database, table: u32, record: usize, field: u32) -> &str {
    db.table(table)
        .and_then(|t| t.records.get(record)?.values.get(&field))
        .map_or("", |s| s.as_str())
}

/// Every match in `records` (indexes into the table), in search order:
/// record, then field, then position.
pub fn hits(
    db: &Database,
    table: u32,
    records: &[usize],
    fields: &[u32],
    fr: &FindReplace,
) -> Vec<Hit> {
    let fields = searchable(db, table, fields);
    let mut out = Vec::new();
    for (pos, &r) in records.iter().enumerate() {
        for &f in &fields {
            for range in fr.matches(stored(db, table, r, f)) {
                out.push(Hit {
                    pos,
                    field: f,
                    range,
                });
            }
        }
    }
    out
}

/// Find Next: the match after `after` (or before it, backward); from the
/// start (or end) when `after` is `None`. `All` wraps around.
/// ponytail: rescans every record per call; fine for found sets of a few
/// thousand records, keep a cursor if that ever shows.
pub fn find_next(
    db: &Database,
    table: u32,
    records: &[usize],
    fields: &[u32],
    fr: &FindReplace,
    after: Option<&Hit>,
    dir: Direction,
) -> Option<Hit> {
    let all = hits(db, table, records, fields, fr);
    let order = |h: &Hit| {
        let fi = fields
            .iter()
            .position(|f| *f == h.field)
            .unwrap_or(fields.len());
        (h.pos, fi, h.range.start)
    };
    let Some(a) = after else {
        return match dir {
            Direction::Backward => all.last().cloned(),
            _ => all.first().cloned(),
        };
    };
    let key = order(a);
    match dir {
        Direction::Backward => all.iter().rev().find(|h| order(h) < key).cloned(),
        Direction::Forward => all.iter().find(|h| order(h) > key).cloned(),
        Direction::All => all.iter().find(|h| order(h) > key).or(all.first()).cloned(),
    }
}

/// Replace: replaces the text of one match (if it still matches) and
/// returns whether it did.
pub fn replace_hit(
    db: &mut Database,
    table: u32,
    records: &[usize],
    fr: &FindReplace,
    hit: &Hit,
) -> bool {
    let Some(&r) = records.get(hit.pos) else {
        return false;
    };
    let text = stored(db, table, r, hit.field).to_string();
    if !fr.matches(&text).contains(&hit.range) {
        return false;
    }
    let mut new = text[..hit.range.start].to_string();
    new.push_str(&fr.replace);
    new.push_str(&text[hit.range.end..]);
    db.set_values(table, r, &[(hit.field, new)]);
    true
}

/// Replace All over `records`: returns the number of replacements.
pub fn replace_all(
    db: &mut Database,
    table: u32,
    records: &[usize],
    fields: &[u32],
    fr: &FindReplace,
) -> usize {
    let fields = searchable(db, table, fields);
    let mut n = 0;
    for &r in records {
        let changes: Vec<(u32, String)> = fields
            .iter()
            .filter_map(|&f| {
                let (new, k) = fr.replace_in(stored(db, table, r, f));
                n += k;
                (k > 0).then_some((f, new))
            })
            .collect();
        if !changes.is_empty() {
            db.set_values(table, r, &changes);
        }
    }
    n
}

/// What Replace Field Contents puts in the field.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplaceWith {
    /// "Replace with current contents": the same text in every record.
    Value(String),
    /// "Replace with serial numbers": `initial` (its trailing number is
    /// counted up, prefixes and leading zeros kept), then `increment`.
    /// `update_entry` also sets the field's next auto-enter serial.
    Serial {
        initial: String,
        increment: i64,
        update_entry: bool,
    },
    /// "Replace with calculated result": calculation source text evaluated
    /// for each record from occurrence `to`.
    Calc { source: String, to: u32 },
}

/// `initial` counted up by `n`: the last run of digits changes, keeping
/// its width (`INV009` + 1 = `INV010`); text without digits gets the
/// number appended.
pub fn serial_add(initial: &str, n: i64) -> String {
    let b = initial.as_bytes();
    let Some(last) = b.iter().rposition(u8::is_ascii_digit) else {
        return format!("{initial}{n}");
    };
    let end = last + 1;
    let start = b[..end]
        .iter()
        .rposition(|c| !c.is_ascii_digit())
        .map_or(0, |i| i + 1);
    let digits = end - start;
    let base: i64 = initial[start..end].parse().unwrap_or(0);
    let v = base + n;
    let width = if v < 0 { digits + 1 } else { digits };
    format!(
        "{}{:0width$}{}",
        &initial[..start],
        v,
        &initial[end..],
        width = width
    )
}

/// Replace Field Contents over `records` (indexes into `table`). With
/// `auto_enter` ("Perform auto-enter options while replacing"), lookups
/// and modification stamps follow the change, as in data entry. Returns
/// the number of records changed, or why nothing was done.
pub fn replace_field_contents(
    db: &mut Database,
    table: u32,
    records: &[usize],
    field: u32,
    with: &ReplaceWith,
    auto_enter: bool,
) -> Result<usize, String> {
    let f = db
        .table(table)
        .and_then(|t| t.field(field))
        .ok_or("no such field")?;
    if f.kind != FieldKind::Normal {
        return Err(format!("{:?} is not a field you can replace", f.name));
    }
    for (i, &r) in records.iter().enumerate() {
        let v = match with {
            ReplaceWith::Value(s) => s.clone(),
            ReplaceWith::Serial {
                initial, increment, ..
            } => serial_add(initial, increment * i as i64),
            ReplaceWith::Calc { source, to } => {
                let v = db.eval_source(source, table, *to, Some(r));
                stored_text(&v, &db.locale)
            }
        };
        if auto_enter {
            db.set_values(table, r, &[(field, v)]);
        } else if let Some(rec) = db
            .tables
            .iter_mut()
            .find(|t| t.id == table)
            .and_then(|t| t.records.get_mut(r))
        {
            rec.values.insert(field, v);
        }
    }
    if let ReplaceWith::Serial {
        initial,
        increment,
        update_entry: true,
    } = with
        && let Some(f) = db
            .tables
            .iter_mut()
            .find(|t| t.id == table)
            .and_then(|t| t.fields.iter_mut().find(|f| f.id == field))
        && f.def.auto_serial()
    {
        f.def.serial_next = serial_add(initial, increment * records.len() as i64);
    }
    Ok(records.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{AMOUNT, CITY, NAME, NOTE, T, sample};

    fn fr(find: &str, replace: &str) -> FindReplace {
        FindReplace {
            find: find.into(),
            replace: replace.into(),
            ..Default::default()
        }
    }

    #[test]
    fn matching_options() {
        let mut f = fr("rome", "");
        assert_eq!(f.matches("Rome, rome, Romeo"), vec![0..4, 6..10, 12..16]);
        f.whole_word = true;
        assert_eq!(f.matches("Rome, rome, Romeo"), vec![0..4, 6..10]);
        f.match_case = true;
        assert_eq!(f.matches("Rome, rome, Romeo"), vec![6..10]);
        assert_eq!(fr("aa", "").matches("aaaa"), vec![0..2, 2..4]);
        assert_eq!(fr("é", "").matches("ÉTÉ"), vec![0..2, 3..5]);
        assert_eq!(fr("o", "0").replace_in("foo bar"), ("f00 bar".into(), 2));
    }

    #[test]
    fn find_next_and_replace() {
        let mut db = sample();
        let recs: Vec<usize> = (0..5).collect();
        let fields = [NAME, CITY, NOTE];
        let q = fr("rome", "Milan");
        let h = find_next(&db, T, &recs, &fields, &q, None, Direction::All);
        assert_eq!(h.as_ref().map(|h| (h.pos, h.field)), Some((0, CITY)));
        let h2 = find_next(&db, T, &recs, &fields, &q, h.as_ref(), Direction::All);
        assert_eq!(h2.as_ref().map(|h| h.pos), Some(2));
        // Backward from the second match is the first; All wraps around.
        let b = find_next(&db, T, &recs, &fields, &q, h2.as_ref(), Direction::Backward);
        assert_eq!(b, h);
        let last = Hit {
            pos: 4,
            field: CITY,
            range: 0..4,
        };
        assert_eq!(
            find_next(&db, T, &recs, &fields, &q, Some(&last), Direction::Forward),
            None
        );
        assert_eq!(
            find_next(&db, T, &recs, &fields, &q, Some(&last), Direction::All),
            h
        );
        let Some(h) = h else { panic!("no hit") };
        assert!(replace_hit(&mut db, T, &recs, &q, &h));
        assert!(!replace_hit(&mut db, T, &recs, &q, &h));
        assert_eq!(db.value(T, 0, CITY).to_text(), "Milan");
        // Replace All over the found set (records 2 and 4 still say Rome).
        assert_eq!(replace_all(&mut db, T, &recs, &fields, &q), 2);
        assert_eq!(db.value(T, 4, CITY).to_text(), "Milan");
        // Current record only.
        let q = fr("line", "row");
        assert_eq!(replace_all(&mut db, T, &[0], &fields, &q), 2);
        assert_eq!(db.value(T, 0, NOTE).to_text(), "row 1\rrow 2");
    }

    #[test]
    fn serial_numbers() {
        assert_eq!(serial_add("1", 4), "5");
        assert_eq!(serial_add("INV009", 1), "INV010");
        assert_eq!(serial_add("A-07-x", 3), "A-10-x");
        assert_eq!(serial_add("abc", 2), "abc2");
    }

    #[test]
    fn replace_field_contents_modes() {
        let mut db = sample();
        let recs = [0, 2, 4];
        let n = replace_field_contents(
            &mut db,
            T,
            &recs,
            AMOUNT,
            &ReplaceWith::Serial {
                initial: "100".into(),
                increment: 10,
                update_entry: true,
            },
            true,
        );
        assert_eq!(n, Ok(3));
        let got: Vec<String> = (0..5).map(|r| db.value(T, r, AMOUNT).to_text()).collect();
        assert_eq!(got, ["100", "20", "110", "40", "120"]);
        replace_field_contents(
            &mut db,
            T,
            &[1],
            CITY,
            &ReplaceWith::Value("Nice".into()),
            true,
        )
        .ok();
        assert_eq!(db.value(T, 1, CITY).to_text(), "Nice");
        replace_field_contents(
            &mut db,
            T,
            &[0, 1],
            CITY,
            &ReplaceWith::Calc {
                source: "Upper ( Name ) & \"!\"".into(),
                to: T,
            },
            false,
        )
        .ok();
        assert_eq!(db.value(T, 0, CITY).to_text(), "ADA!");
    }
}
