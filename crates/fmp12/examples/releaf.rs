//! Experiment: re-encode every leaf of a file in place (same sector boundaries,
//! original index), reporting the sectors whose bytes change.
use fmp12::{File, write};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = File::open(&a[1]).unwrap();
    let mut b = f.bytes().to_vec();
    let mut changed = 0;
    for s in f.leaves() {
        let es = write::leaf_entries(s.payload).unwrap();
        let pages = write::pack(&es);
        assert_eq!(pages.len(), 1, "sector {} grew", s.id);
        let p = &pages[0].0;
        let o = s.id as usize * 4096;
        let free = u16::from_be_bytes([b[o + 14], b[o + 15]]) as usize;
        let old = &b[o + 20..o + 4096 - free];
        // Trailing pops depend on the next sector, which `pack` does not see.
        let same = old.starts_with(p) && old[p.len()..].iter().all(|x| *x == 0x40);
        if !same {
            changed += 1;
            let at = old.iter().zip(p).position(|(x, y)| x != y).unwrap_or(0);
            println!(
                "sector {}: {} -> {} bytes, first diff at {at}: {:02x?} vs {:02x?}",
                s.id,
                old.len(),
                p.len(),
                &old[at.saturating_sub(4)..(at + 8).min(old.len())],
                &p[at.saturating_sub(4)..(at + 8).min(p.len())]
            );
        }
        let s = &mut b[o..o + 4096];
        s[20..].fill(0);
        s[20..20 + p.len()].copy_from_slice(p);
        s[14..16].copy_from_slice(&((4076 - p.len()) as u16).to_be_bytes());
    }
    println!("{changed} leaves changed (ignoring trailing pops)");
    if let Some(out) = a.get(2) {
        std::fs::write(out, b).unwrap();
    }
}
