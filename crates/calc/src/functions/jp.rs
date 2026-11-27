//! Japanese functions: kana width and script, kanji numerals, eras.

use super::{int, txt};
use crate::error::EvalError;
use crate::eval::{Evaluator, R};
use crate::num;
use crate::value::{Value, civil, day_number};
use bigdecimal::{Signed, Zero};

/// Half-width katakana (U+FF65..U+FF9F) and their full-width forms,
/// position for position. FileMaker leaves the punctuation 。「」、 alone.
const HALF: &str = "･ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ";
const FULL: &str = "・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";
/// Kana that take a voiced mark (ガ = カ + 1); the ハ row also takes the
/// semi-voiced mark (パ = ハ + 2).
const VOICED: &str = "カキクケコサシスセソタチツテトハヒフヘホ";
const SEMI: &str = "ハヒフヘホ";

fn half(c: char) -> Option<char> {
    FULL.chars()
        .position(|f| f == c)
        .and_then(|i| HALF.chars().nth(i))
}

fn full(c: char) -> Option<char> {
    HALF.chars()
        .position(|h| h == c)
        .and_then(|i| FULL.chars().nth(i))
}

fn offset(c: char, by: u32) -> Option<char> {
    char::from_u32(c as u32 + by)
}

/// Full-width katakana to half-width; voiced kana become two characters.
// ponytail: ヷ ヸ ヹ ヺ and the small ヵ ヶ have no half-width form and stay.
pub fn to_hankaku(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if let Some(h) = half(c) {
            out.push(h);
        } else if c == 'ヴ' {
            out.push_str("ｳﾞ");
        } else if let Some(h) = VOICED
            .chars()
            .find(|&b| offset(b, 1) == Some(c))
            .and_then(half)
        {
            out.extend([h, 'ﾞ']);
        } else if let Some(h) = SEMI
            .chars()
            .find(|&b| offset(b, 2) == Some(c))
            .and_then(half)
        {
            out.extend([h, 'ﾟ']);
        } else {
            out.push(c);
        }
    }
    out
}

/// Half-width katakana to full-width, joining a following voiced mark.
pub fn to_zenkaku(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        let Some(f) = full(c) else {
            out.push(c);
            continue;
        };
        let joined = match it.peek() {
            Some('ﾞ') if f == 'ウ' => Some('ヴ'),
            Some('ﾞ') if VOICED.contains(f) => offset(f, 1),
            Some('ﾟ') if SEMI.contains(f) => offset(f, 2),
            _ => None,
        };
        match joined {
            Some(j) => {
                it.next();
                out.push(j);
            }
            None => out.push(f),
        }
    }
    out
}

fn map(a: &[Value], f: impl Fn(&str) -> String) -> R {
    Ok(Value::Text(f(&txt(a, 0))))
}

pub fn kana_hankaku(_: &mut Evaluator, a: &[Value]) -> R {
    map(a, to_hankaku)
}

pub fn kana_zenkaku(_: &mut Evaluator, a: &[Value]) -> R {
    map(a, to_zenkaku)
}

fn shift(c: char, from: std::ops::RangeInclusive<u32>, delta: i64) -> char {
    if from.contains(&(c as u32)) {
        char::from_u32((c as i64 + delta) as u32).unwrap_or(c)
    } else {
        c
    }
}

/// Katakana, full- or half-width, to hiragana.
pub fn hiragana(_: &mut Evaluator, a: &[Value]) -> R {
    map(a, |s| {
        to_zenkaku(s)
            .chars()
            .map(|c| shift(c, 0x30A1..=0x30F6, -0x60))
            .collect()
    })
}

/// Hiragana to full-width katakana.
pub fn katakana(_: &mut Evaluator, a: &[Value]) -> R {
    map(a, |s| {
        s.chars().map(|c| shift(c, 0x3041..=0x3096, 0x60)).collect()
    })
}

const KANJI_DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// Value of an ASCII or full-width digit.
fn digit(c: char) -> Option<usize> {
    c.to_digit(10)
        .filter(|_| c.is_ascii_digit())
        .or_else(|| ('０'..='９').contains(&c).then(|| c as u32 - '０' as u32))
        .map(|d| d as usize)
}

/// Arabic digits, half- or full-width, to kanji digits one for one.
pub fn kanji_numeral(_: &mut Evaluator, a: &[Value]) -> R {
    map(a, |s| {
        s.chars()
            .map(|c| digit(c).map_or(c, |d| KANJI_DIGITS[d]))
            .collect()
    })
}

/// NumToJText ( number ; separator ; characterType ).
/// separator: 0 none, 1 commas every three digits, 2 万/億 units, 3 also
/// 十/百/千 within each four-digit group (千2百3十4: no 1 before a unit,
/// except in traditional kanji, 壱阡). characterType: 0 half-width digits,
/// 1 full-width, 2 kanji, 3 traditional kanji (壱弐参四伍...).
/// Empty four-digit groups are dropped (一億五); fractions are kept with
/// separators 0 and 1 (一.五) and dropped with 2 and 3.
pub fn num_to_j_text(_: &mut Evaluator, a: &[Value]) -> R {
    let n = num::shown(&a[0].num_or_zero());
    let opt = |i: usize| -> Result<i64, EvalError> {
        let v = if a[i].is_empty() { 0 } else { int(a, i)? };
        Ok(if (0..=3).contains(&v) { v } else { 0 })
    };
    let (sep, ty) = (opt(1)?, opt(2)?);
    let whole = num::trunc(&n, 0).abs();
    let digits = num::plain(&if sep <= 1 { n.abs() } else { whole.clone() });
    let traditional = ['〇', '壱', '弐', '参', '四', '伍', '六', '七', '八', '九'];
    let d = |c: char| -> char {
        let Some(v) = c.to_digit(10).map(|v| v as usize) else {
            return c;
        };
        match ty {
            1 => char::from_u32('０' as u32 + v as u32).unwrap_or(c),
            2 => KANJI_DIGITS[v],
            3 => traditional[v],
            _ => c,
        }
    };
    let small = if ty == 3 {
        ["", "拾", "陌", "阡"]
    } else {
        ["", "十", "百", "千"]
    };
    let big = if ty == 3 {
        ["", "萬", "億", "兆", "京"]
    } else {
        ["", "万", "億", "兆", "京"]
    };
    let body: String = match sep {
        1 => {
            let comma = if ty == 1 { '，' } else { ',' };
            let c: Vec<char> = digits.chars().collect();
            let mut out = String::new();
            for (i, &ch) in c.iter().enumerate() {
                if i > 0 && (c.len() - i).is_multiple_of(3) {
                    out.push(comma);
                }
                out.push(d(ch));
            }
            out
        }
        2 | 3 => {
            let c: Vec<char> = digits.chars().collect();
            let groups: Vec<&[char]> = c.rchunks(4).collect();
            if groups.len() > big.len() || whole.is_zero() {
                c.iter().map(|&x| d(x)).collect()
            } else {
                let mut out = String::new();
                for (gi, g) in groups.iter().enumerate().rev() {
                    let g: Vec<char> = g.iter().copied().skip_while(|&x| x == '0').collect();
                    if g.is_empty() {
                        continue;
                    }
                    if sep == 2 {
                        out.extend(g.iter().map(|&x| d(x)));
                    } else {
                        for (i, &x) in g.iter().enumerate() {
                            let unit = small[g.len() - 1 - i];
                            if x != '0' {
                                if x != '1' || unit.is_empty() || ty == 3 {
                                    out.push(d(x));
                                }
                                out.push_str(unit);
                            }
                        }
                    }
                    out.push_str(big[gi]);
                }
                out
            }
        }
        _ => digits.chars().map(d).collect(),
    };
    let minus = if n.is_negative() { "-" } else { "" };
    Ok(Value::Text(format!("{minus}{body}")))
}

/// (start year, month, day, name, short name, letter) of the modern eras.
const ERAS: [(i64, i64, i64, &str, &str, &str); 5] = [
    (2019, 5, 1, "令和", "令", "R"),
    (1989, 1, 8, "平成", "平", "H"),
    (1926, 12, 25, "昭和", "昭", "S"),
    (1912, 7, 30, "大正", "大", "T"),
    (1868, 9, 8, "明治", "明", "M"),
];

/// YearName ( date ; format ): 0 long (平成20), 1 short ((平)20), 2 letter
/// (H20). The first year of an era is 元 in the kanji forms. Dates before
/// Meiji give the Western year: 西暦1850, (西)1850, 1850 CE.
/// (All forms checked against FileMaker; other formats act as 0.)
pub fn year_name(_: &mut Evaluator, a: &[Value]) -> R {
    let format = if a[1].is_empty() { 0 } else { int(a, 1)? };
    let format = if (0..=2).contains(&format) { format } else { 0 };
    super::datetime::on_date(a, "YearName", |d| {
        let y = civil(d).0;
        let era = ERAS
            .iter()
            .find(|(ey, em, ed, ..)| d >= day_number(*ey, *em, *ed));
        Value::Text(match era {
            Some((ey, _, _, long, short, letter)) => {
                let n = y - ey + 1;
                let num = if n == 1 && format < 2 {
                    "元".to_string()
                } else {
                    n.to_string()
                };
                match format {
                    1 => format!("({short}){num}"),
                    2 => format!("{letter}{num}"),
                    _ => format!("{long}{num}"),
                }
            }
            None => match format {
                1 => format!("(西){y}"),
                2 => format!("{y} CE"),
                _ => format!("西暦{y}"),
            },
        })
    })
}

/// MonthNameJ: 1月 ... 12月 (half-width digits, as in FileMaker).
pub fn month_name_j(_: &mut Evaluator, a: &[Value]) -> R {
    super::datetime::on_date(a, "MonthNameJ", |d| {
        Value::Text(format!("{}月", civil(d).1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_tables_align() {
        assert_eq!(HALF.chars().count(), FULL.chars().count());
        assert_eq!(to_hankaku("ガッコウ・パン。"), "ｶﾞｯｺｳ･ﾊﾟﾝ。");
        assert_eq!(to_zenkaku("ｶﾞｯｺｳ･ﾊﾟﾝｳﾞ｡"), "ガッコウ・パンヴ｡");
    }
}
