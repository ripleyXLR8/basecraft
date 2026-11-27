//! Experiment helper: edits the logical entries of a file (as printed by
//! `fmdump <file> entries`) and rebuilds it.
//! `cargo run -p fmp12 --example entedit -- <in> <out> [del:<path>:<item prefix>]… [add:<path>:<item>]…`
//! Paths are dot-separated hex elements, items hex.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn path(s: &str) -> Vec<Vec<u8>> {
    if s.is_empty() {
        return vec![];
    }
    s.split('.').map(unhex).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let f = fmp12::File::open(&a[0]).unwrap();
    let mut es = fmp12::write::entries(&f).unwrap();
    for op in &a[2..] {
        let parts: Vec<&str> = op.splitn(3, ':').collect();
        match parts[0] {
            "del" => {
                let (p, it) = (path(parts[1]), unhex(parts[2]));
                // A trailing '*' on the path deletes the whole subtree.
                let n = es.len();
                es.retain(|e| !(e.path == p && e.item.starts_with(&it)));
                eprintln!("{op}: {} removed", n - es.len());
            }
            "deltree" => {
                let p = path(parts[1]);
                let n = es.len();
                es.retain(|e| !e.path.starts_with(&p));
                eprintln!("{op}: {} removed", n - es.len());
            }
            "add" => es.push(fmp12::write::Entry {
                path: path(parts[1]),
                item: unhex(parts[2]),
            }),
            _ => panic!("bad op {op}"),
        }
    }
    fmp12::write::sort(&mut es);
    std::fs::write(&a[1], fmp12::write::build(&f.bytes()[..fmp12::SECTOR], &es)).unwrap();
}
