//! Prints the compiled styles of a file's themes (default and named styles,
//! rules by element and state, properties by id).
//! `cargo run -p fmp12 --example themedump -- <file> [object type]`
#![allow(clippy::unwrap_used)]

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let only: Option<u32> = a.get(1).and_then(|s| s.parse().ok());
    let file = fmp12::File::open(&a[0]).unwrap();
    for th in fmp12::theme::themes(&file).unwrap() {
        println!("theme {} {} ({})", th.number, th.id, th.name);
        for ((ty, name), rules) in &th.styles {
            if only.is_some_and(|o| o != *ty) {
                continue;
            }
            println!("  type {ty} {name:?}");
            for ((el, st), props) in rules {
                println!("    el {el} st {st}: {props:?}");
            }
        }
    }
}
