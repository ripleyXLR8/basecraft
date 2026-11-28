//! Prints each chart of a file: type, series count, settings record (hex).
//! `cargo run -p fmp12 --example chartdump -- <file>`
#![allow(clippy::unwrap_used)]

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = fmp12::File::open(&a[0]).unwrap();
    for ((l, o), c) in fmp12::charts::charts(&file).unwrap() {
        println!(
            "layout {l} object {o}: {:?} {} series{} record {} bytes",
            c.kind,
            c.y.len(),
            if c.size.is_some() { " + size" } else { "" },
            c.record.len()
        );
        for (i, w) in c.record.chunks(25).enumerate() {
            let h: Vec<String> = w.iter().map(|b| format!("{b:02x}")).collect();
            println!("  {:4} {}", i * 25, h.join(" "));
        }
    }
}
