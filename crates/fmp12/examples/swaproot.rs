//! Experiment: replace the root index of a 2-level file with BaseCraft's index.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = fmp12::File::open(&a[1]).unwrap();
    let leaves: Vec<u32> = f.leaves().iter().map(|s| s.id).collect();
    let p = fmp12::write::index_payload(&f, &leaves).unwrap();
    let mut b = f.bytes().to_vec();
    let s = &mut b[4096..8192];
    s[20..].fill(0);
    s[20..20 + p.len()].copy_from_slice(&p);
    s[14..16].copy_from_slice(&((4076 - p.len()) as u16).to_be_bytes());
    std::fs::write(&a[2], b).unwrap();
    println!("root payload {} bytes for {} leaves", p.len(), leaves.len());
}
