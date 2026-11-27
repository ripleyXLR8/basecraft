//! fmdump — inspect .fmp12 files.
//!
//! fmdump <file> sectors        sector table
//! fmdump <file> chunks         every chunk with its path (text-decoded)
//! fmdump <file> raw            tab-separated path, kind, key, hex value, file offset
//! fmdump <file> tables         tables and fields
//! fmdump <file> graph          table occurrences and relationships
//! fmdump <file> records <tbl>  records of a table (by name)

use fmp12::{Chunk, File, schema};

fn show(b: &[u8]) -> String {
    let t = fmp12::text(b);
    if t.chars().all(|c| !c.is_control() || c == '\r') && !t.is_empty() {
        format!("{t:?}")
    } else {
        b.iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn path(p: &[&[u8]]) -> String {
    p.iter()
        .map(|e| match fmp12::path_int(e) {
            Some(n) => n.to_string(),
            None => format!(
                "<{}>",
                e.iter().map(|x| format!("{x:02x}")).collect::<String>()
            ),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, file, cmd, rest @ ..] = args.as_slice() else {
        eprintln!("usage: fmdump <file.fmp12> sectors|chunks|raw|tables|graph|records <table>");
        std::process::exit(2);
    };
    let f = File::open(file)?;
    match cmd.as_str() {
        "sectors" => {
            println!("creator: {}  sectors: {}", f.creator(), f.sector_count());
            for i in 1..=f.sector_count() {
                let s = f.sector(i);
                println!(
                    "{:5} lvl={} prev={:5} next={:5} del={}",
                    s.id, s.level, s.prev, s.next, s.deleted
                );
            }
        }
        "chunks" => f.walk(|p, c| {
            let p = path(p);
            match c {
                Chunk::Data(d) => println!("[{p}] data {}", show(d)),
                Chunk::Segment { index, data } => println!("[{p}] seg#{index} {}", show(data)),
                Chunk::Kv { key, data } => println!("[{p}] {key} = {}", show(data)),
                Chunk::KvLong { key, data } => println!("[{p}] <{}> = {}", show(key), show(data)),
                Chunk::Push(_) | Chunk::Pop => {}
            }
        })?,
        "raw" => f.walk(|p, c| {
            let p = path(p);
            let h = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            match c {
                Chunk::Data(d) => println!("{p}	data		{}	{}", h(d), f.offset_of(d)),
                Chunk::Segment { index, data } => {
                    println!("{p}	seg	{index}	{}	{}", h(data), f.offset_of(data))
                }
                Chunk::Kv { key, data } => {
                    println!("{p}	kv	{key}	{}	{}", h(data), f.offset_of(data))
                }
                Chunk::KvLong { key, data } => {
                    println!("{p}	kvl	{}	{}	{}", h(key), h(data), f.offset_of(data))
                }
                Chunk::Push(_) | Chunk::Pop => {}
            }
        })?,
        "tables" => {
            for t in schema::tables(&f)? {
                println!("{} (id {})", t.name, t.id);
                for (id, f) in &t.fields {
                    let mut flags = Vec::new();
                    if f.global() {
                        flags.push("global".to_string());
                    }
                    if f.max_repetitions() > 1 {
                        flags.push(format!("reps={}", f.max_repetitions()));
                    }
                    if let Some(a) = f.auto_stamp() {
                        flags.push(format!("{a:?}"));
                    }
                    if f.auto_calculation() {
                        flags.push("auto-calc".into());
                    }
                    if f.auto_serial() {
                        flags.push("serial".into());
                    }
                    let v = f.validation();
                    if v.not_empty {
                        flags.push("not-empty".into());
                    }
                    if v.unique {
                        flags.push("unique".into());
                    }
                    println!(
                        "  {id:4} {:<28} {:?} {:?} {:?} {}",
                        f.name,
                        f.kind(),
                        f.data_type(),
                        f.indexing(),
                        flags.join(" ")
                    );
                }
            }
        }
        "graph" => {
            for to in fmp12::graph::table_occurrences(&f)? {
                println!(
                    "TO {} {:?} → table {} at ({}, {}) {}×{}",
                    to.id, to.name, to.base_table, to.left, to.top, to.width, to.height
                );
            }
            for r in fmp12::graph::relationships(&f)? {
                let p: Vec<String> = r
                    .predicates
                    .iter()
                    .map(|p| format!("{} {:?} {}", p.left_field, p.op, p.right_field))
                    .collect();
                println!(
                    "REL {}: TO {} ⇄ TO {} [{}]",
                    r.id,
                    r.left,
                    r.right,
                    p.join(", ")
                );
            }
        }
        "records" => {
            let name = rest.first().ok_or("table name required")?;
            let tables = schema::tables(&f)?;
            let t = tables
                .iter()
                .find(|t| &t.name == name)
                .ok_or("no such table")?;
            for (r, fields) in schema::records(&f, t)? {
                let cells: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}={v:?}", t.fields.get(k).map_or("?", |f| &f.name)))
                    .collect();
                println!("#{r} {}", cells.join(" "));
            }
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
