//! Prints the separators of every index sector next to the first entry of
//! each child: `index <file> [max entries per sector]`.
use fmp12::{File, write};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = File::open(&a[1]).unwrap();
    let max: usize = a.get(2).map_or(usize::MAX, |s| s.parse().unwrap());
    let h = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let path = |p: &[Vec<u8>]| p.iter().map(|e| h(e)).collect::<Vec<_>>().join(".");
    for i in 1..=f.sector_count() {
        let s = f.sector(i);
        if s.level == 0 || s.deleted {
            continue;
        }
        println!("== sector {i} level {}", s.level);
        for sep in write::decode_index(s.payload).unwrap().iter().take(max) {
            let child = f.sector(sep.child);
            let first = if child.level == 0 {
                let e = &write::leaf_entries(child.payload).unwrap()[0];
                format!("[{}] {}", path(&e.path), h(&e.item[..e.item.len().min(8)]))
            } else {
                "index".into()
            };
            println!(
                "[{}] {} -> {:5} | child starts: {first}",
                path(&sep.path),
                h(&sep.key),
                sep.child
            );
        }
    }
}
