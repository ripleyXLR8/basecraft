//! Reverse-oracle helper: sets keys of layout object nodes and rebuilds the
//! file. `nodeset <in> <out> <spec>`; each spec line is
//! `layout obj key hex` (`[4].[5].[layout].[5].[obj]` key = hex value), or
//! `layout obj key -` to delete the key.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::write::{Entry, Item};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f = fmp12::File::open(&a[0]).unwrap();
    let mut es = fmp12::write::entries(&f).unwrap();
    let spec = std::fs::read_to_string(&a[2]).unwrap();
    for line in spec.lines().filter(|l| !l.trim().is_empty()) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let (l, o, k): (u32, u32, u32) = (
            w[0].parse().unwrap(),
            w[1].parse().unwrap(),
            w[2].parse().unwrap(),
        );
        let path: Vec<Vec<u8>> = [4, 5, l, 5, o]
            .iter()
            .map(|x| fmp12::save::path_elem(*x))
            .collect();
        es.retain(|e| {
            if e.path != path {
                return true;
            }
            match fmp12::chunk::parse(&e.item).ok().as_deref() {
                Some([fmp12::Chunk::Kv { key, .. }]) => *key != k,
                Some([fmp12::Chunk::Data([x])]) if e.item[0] == 0 => *x as u32 != k,
                _ => true,
            }
        });
        if w[3] != "-" {
            let data: Vec<u8> = (0..w[3].len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&w[3][i..i + 2], 16).unwrap())
                .collect();
            es.push(Entry {
                path,
                item: fmp12::write::encode(&Item::Kv { key: k, data }),
            });
        }
    }
    fmp12::write::sort(&mut es);
    std::fs::write(&a[1], fmp12::write::build(&f.bytes()[..fmp12::SECTOR], &es)).unwrap();
}
