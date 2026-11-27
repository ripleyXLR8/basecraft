//! Prints, for every leaf sector, its id and first chunk (path, kind, key).
use fmp12::{Chunk, File, chunk};

fn main() {
    let f = File::open(std::env::args().nth(1).unwrap()).unwrap();
    let h = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    for s in f.leaves() {
        let mut path = Vec::new();
        for c in chunk::parse(s.payload).unwrap() {
            match c {
                Chunk::Push(p) => path.push(h(p)),
                Chunk::Pop => {
                    path.pop();
                }
                Chunk::Marker => {
                    println!("{}\t{}\tnode\t", s.id, path.join("."));
                    break;
                }
                Chunk::Data(d) => {
                    println!("{}\t{}\tdata\t{}", s.id, path.join("."), h(d));
                    break;
                }
                Chunk::Kv { key, .. } => {
                    println!("{}\t{}\tkv\t{key}", s.id, path.join("."));
                    break;
                }
                Chunk::KvLong { key, .. } => {
                    println!("{}\t{}\tkvl\t{}", s.id, path.join("."), h(key));
                    break;
                }
            }
        }
    }
}
