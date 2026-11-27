//! Dumps every chunk under `[4].[5]` (layouts), or under another path
//! prefix: path, key and hex value. `layoutdump <file> [layout id]` or
//! `layoutdump <file> 17.5` (dotted prefix).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let Some(file) = a.get(1) else {
        return Err("usage: layoutdump <file> [layout]".into());
    };
    let prefix: Vec<u32> = match a.get(2) {
        Some(s) if s.contains('.') => s.split('.').filter_map(|x| x.parse().ok()).collect(),
        Some(s) => [4, 5].into_iter().chain(s.parse().ok()).collect(),
        None => vec![4, 5],
    };
    let f = fmp12::File::open(file)?;
    let hex = |b: &[u8]| {
        b.iter()
            .map(|x| format!("{x:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    f.walk(|path, c| {
        let Some(p) = fmp12::int_path(path) else {
            return;
        };
        if !p.starts_with(&prefix) {
            return;
        }
        match c {
            fmp12::Chunk::Kv { key, data } => println!("{p:?} k{key} = {}", hex(data)),
            fmp12::Chunk::KvLong { key, data } => {
                println!("{p:?} K[{}] = {}", hex(key), hex(data))
            }
            fmp12::Chunk::Data(d) => println!("{p:?} data = {}", hex(d)),
            _ => {}
        }
    })?;
    Ok(())
}
