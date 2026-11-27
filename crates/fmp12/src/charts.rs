//! Charts (`docs/FORMAT.md` §10, "Charts"), decoded from charts made in
//! FileMaker Pro 26 (Insert › Chart, Chart Setup, and layout objects pasted
//! from the clipboard) and diffed.
//!
//! A chart is a layout object of kind 13 (an external object, like a web
//! viewer) whose field 11 is `{1: 'CHRT' (0x43485254), 2: 0, 3: 19}`. Its
//! settings live in the object's node `[4].[5].[L].[5].[obj]`:
//!
//! * calculations (keys 4, 5, 6 of `.[K].[5]`, as for button labels):
//!   K = 128 chart title, 129 X axis title, 130 Y axis title, 131 X data,
//!   then for Y series *i* (from 0) 133 + 2*i data and 134 + 2*i series
//!   name; a bubble chart's size data follows the last series (a data key
//!   without a name);
//! * `.[139].[138]`: a binary record of big-endian u32 words, key 0 = its
//!   length, keys 1… = 1000-byte chunks. The first 120 bytes have a fixed
//!   layout ([`Chart`] fields); then the data source, after which the
//!   record has variable-length parts (fonts, colours, axis formats) not
//!   decoded here.

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path};

/// Chart types (record word at byte 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Column,
    Bar,
    Line,
    Area,
    Pie,
    PositiveNegative,
    StackedColumn,
    StackedBar,
    Bubble,
    Scatter,
}

impl Kind {
    fn from_code(c: u32) -> Kind {
        match c {
            1 => Kind::Bar,
            2 => Kind::Line,
            3 => Kind::Area,
            4 => Kind::Pie,
            5 => Kind::PositiveNegative,
            6 => Kind::StackedColumn,
            7 => Kind::StackedBar,
            8 => Kind::Bubble,
            9 => Kind::Scatter,
            _ => Kind::Column,
        }
    }

    /// Bars along the vertical axis.
    pub fn horizontal(self) -> bool {
        matches!(self, Kind::Bar | Kind::StackedBar)
    }

    pub fn stacked(self) -> bool {
        matches!(self, Kind::StackedColumn | Kind::StackedBar)
    }
}

/// Where the chart's values come from (record byte 120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// One point per record of the found set (2).
    #[default]
    FoundSet,
    /// Return-separated values of the current record (1, "delimited data").
    CurrentRecord,
    /// One point per related record of this table occurrence (3).
    Related(u32),
}

/// One Y series: its value calculation and its name (token streams, key 5).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Series {
    pub value: Vec<u8>,
    pub name: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    pub kind: Kind,
    pub source: Source,
    /// Calculations (token streams, see [`crate::calc::decompile`]).
    pub title: Option<Vec<u8>>,
    pub x_title: Option<Vec<u8>>,
    pub y_title: Option<Vec<u8>>,
    pub x: Option<Vec<u8>>,
    pub y: Vec<Series>,
    /// Bubble size (bubble charts).
    pub size: Option<Vec<u8>>,
    /// "Show legend" (byte 59).
    pub legend: bool,
    /// "Show data points on chart" (byte 7): values printed on the marks.
    pub data_points: bool,
    /// Colour scheme (byte 23): 0 Bright, 3 Earth (the default)…
    pub scheme: u32,
}

fn word(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .map_or(0, |x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

/// Decodes the settings record `.[139].[138]` (chunks joined) into `c`.
fn read_record(c: &mut Chart, b: &[u8]) {
    c.data_points = word(b, 4) != 0;
    c.kind = Kind::from_code(word(b, 8));
    c.scheme = word(b, 20);
    c.legend = word(b, 56) != 0;
    c.source = match b.get(120) {
        Some(1) => Source::CurrentRecord,
        // Then a length-prefixed table occurrence reference `03 D0 hi lo`.
        Some(3) => b
            .get(122..126)
            .and_then(crate::graph::to_ref)
            .map_or(Source::FoundSet, Source::Related),
        _ => Source::FoundSet,
    };
}

#[derive(Default)]
struct Raw {
    record: BTreeMap<u32, Vec<u8>>,
    calcs: BTreeMap<u32, Vec<u8>>,
}

/// Every chart of the file, by (layout id, object id).
pub fn charts(file: &File) -> Result<BTreeMap<(u32, u64), Chart>, Error> {
    let mut raw: BTreeMap<(u32, u64), Raw> = BTreeMap::new();
    file.walk(|path, c| {
        let Chunk::Kv { key, data } = c else { return };
        let Some(p) = int_path(path) else { return };
        match p.as_slice() {
            [4, 5, l, 5, obj, 139, 138] if *key >= 1 => {
                let r = raw.entry((*l, *obj as u64)).or_default();
                r.record.insert(*key, data.to_vec());
            }
            [4, 5, l, 5, obj, k, 5] if *key == 5 && (128..=200).contains(k) => {
                raw.entry((*l, *obj as u64))
                    .or_default()
                    .calcs
                    .entry(*k)
                    .or_default()
                    .extend_from_slice(data);
            }
            _ => {}
        }
    })?;
    Ok(raw
        .into_iter()
        .filter(|(_, r)| !r.record.is_empty())
        .map(|(id, mut r)| {
            let mut c = Chart::default();
            let rec: Vec<u8> = r.record.values().flatten().copied().collect();
            read_record(&mut c, &rec);
            c.title = r.calcs.remove(&128);
            c.x_title = r.calcs.remove(&129);
            c.y_title = r.calcs.remove(&130);
            c.x = r.calcs.remove(&131);
            let mut k = 133;
            while let Some(value) = r.calcs.remove(&k) {
                let name = r.calcs.remove(&(k + 1));
                c.y.push(Series { value, name });
                k += 2;
            }
            // ponytail: seen with two series only (key 137); a bubble chart's
            // unnamed last "series" is taken as its size.
            if c.kind == Kind::Bubble && c.y.last().is_some_and(|s| s.name.is_none()) {
                c.size = c.y.pop().map(|s| s.value);
            }
            (id, c)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    /// First 128 bytes of records written by FileMaker Pro 26.
    const DELIMITED: &str = "00 00 00 02 00 00 00 00 00 00 00 00 00 00 00 00
        00 00 00 00 00 00 00 03 00 00 00 02 00 00 00 00 00 00 00 01 00 00 00 00
        00 00 00 02 FF FF FF FF FF FF FF FF 00 00 01 00 00 00 00 01 00 00 00 0D
        3F B7 00 00 00 00 00 0B 33 33 33 FF 00 00 00 00 00 33 33 33 FF 00 00 00
        02 00 00 00 00 00 00 00 01 00 00 00 00 00 00 00 02 FF FF FF FF FF FF FF
        FF 00 00 00 03 00 00 00 01 02 01 00 00 00 00 00";

    #[test]
    fn record() {
        let mut c = Chart::default();
        read_record(&mut c, &hex(DELIMITED));
        assert_eq!(c.kind, Kind::Column);
        assert_eq!(c.source, Source::CurrentRecord);
        assert!(c.legend && !c.data_points);
        assert_eq!(c.scheme, 3);
        // Two series with data points, found set.
        let mut b = hex(DELIMITED);
        b[7] = 1;
        b[120] = 2;
        b[11] = 6;
        read_record(&mut c, &b);
        assert_eq!(c.kind, Kind::StackedColumn);
        assert_eq!(c.source, Source::FoundSet);
        assert!(c.data_points);
        // Related records of occurrence 2 (`04 03 D0 00 02`).
        b[120..126].copy_from_slice(&[3, 4, 3, 0xD0, 0, 2]);
        read_record(&mut c, &b);
        assert_eq!(c.source, Source::Related(130));
    }
}
