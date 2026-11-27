//! Prints index entries of level>=1 sectors next to the first chunk of each child.
use fmp12::{Chunk, File, chunk};

fn show(c: &Chunk) -> String {
    let h = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    match c {
        Chunk::Data(d) => format!("data {}", h(d)),
        Chunk::Kv { key, data } => format!("kv {key}={}", h(data)),
        Chunk::KvLong { key, data } => format!("kvl {}={}", h(key), h(data)),
        Chunk::Push(p) => format!("push {}", h(p)),
        Chunk::Pop => "pop".into(),
    }
}

fn main() {
    let f = File::open(std::env::args().nth(1).unwrap()).unwrap();
    let max: usize = std::env::args().nth(2).map_or(20, |s| s.parse().unwrap());
    for i in 1..=f.sector_count() {
        let s = f.sector(i);
        if s.level == 0 {
            continue;
        }
        println!("== sector {i} level {}", s.level);
        let cs = chunk::parse(s.payload).unwrap();
        let mut path: Vec<String> = vec![];
        for c in cs.iter().take(max * 4) {
            match c {
                Chunk::Push(p) => path.push(p.iter().map(|x| format!("{x:02x}")).collect()),
                Chunk::Pop => {
                    path.pop();
                }
                Chunk::Kv { data, .. } | Chunk::KvLong { data, .. } if data.len() == 4 => {
                    let child = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                    let first = chunk::parse(f.sector(child).payload).unwrap();
                    let fc: Vec<String> = first.iter().take(6).map(show).collect();
                    println!(
                        "[{}] {} -> {child}   | child starts: {}",
                        path.join("."),
                        show(c),
                        fc.join(" ; ")
                    );
                }
                _ => println!("[{}] {}", path.join("."), show(c)),
            }
        }
    }
}
