//! Prints a file's script triggers and the raw File Options record
//! (`[2]` key 24): `cargo run -p fmp12 --example triggersdump -- file.fmp12`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: triggersdump <file.fmp12>")?;
    let file = fmp12::File::open(&path)?;
    let names: std::collections::HashMap<u32, String> = fmp12::scripts::scripts(&file)?
        .into_iter()
        .map(|s| (s.id, s.name))
        .collect();
    file.walk(|p, c| {
        if let (Some([2]), fmp12::chunk::Chunk::Kv { key: 24, data }) =
            (fmp12::int_path(p).as_deref(), c)
        {
            println!("[2] key 24 ({} bytes): {:02x?}", data.len(), data);
        }
    })?;
    let t = fmp12::triggers::triggers(&file)?;
    for tr in &t.file {
        let name = names.get(&tr.script).map_or("?", String::as_str);
        println!("file trigger {} -> script {} {name:?}", tr.event, tr.script);
    }
    if t.file.is_empty() {
        println!("no file triggers decoded");
    }
    let layouts = fmp12::layouts::layouts(&file)?;
    for (id, list) in &t.layouts {
        let lname = layouts
            .iter()
            .find(|l| l.id == *id)
            .map_or("?", |l| l.name.as_str());
        for tr in list {
            let name = names.get(&tr.script).map_or("?", String::as_str);
            println!(
                "layout {id} {lname:?}: event {} -> script {} {name:?}",
                tr.event, tr.script
            );
        }
    }
    let first = layouts.iter().find(|l| l.table_occurrence != 0);
    println!("first layout: {:?}", first.map(|l| (l.id, &l.name)));
    if let Some(l) = layouts.iter().find(|l| l.id == 100) {
        println!("layout 100: {:?}", l.name);
    }
    Ok(())
}
