//! Edits records of a .fmp12 file and saves a copy with `Database::save`.
//!
//! edit <in> <out> OP...
//!   new:<Table>                       new record (later `set`s use id `+`)
//!   set:<Table>:<id|+>:<Field>=<value>   `\r` in value = paragraph break
//!   del:<Table>:<id>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, ops @ ..] = args.as_slice() else {
        return Err("usage: edit <in> <out> OP...".into());
    };
    let file = fmp12::File::open(input)?;
    let mut db = engine::Database::load(&file)?;
    let mut last_new = 0;
    for op in ops {
        let (kind, rest) = op.split_once(':').ok_or("bad op")?;
        let (tname, rest) = rest.split_once(':').unwrap_or((rest, ""));
        if kind == "new" {
            // Database::new_record applies auto-enter options.
            let tid = db
                .tables
                .iter()
                .find(|t| t.name == tname)
                .ok_or("no such table")?
                .id;
            let i = db.new_record(tid).ok_or("no such table")?;
            last_new = db.table(tid).unwrap().records[i].id;
            continue;
        }
        let t = db
            .tables
            .iter_mut()
            .find(|t| t.name == tname)
            .ok_or("no such table")?;
        match kind {
            "set" => {
                let (id, rest) = rest.split_once(':').ok_or("bad set")?;
                let (field, value) = rest.split_once('=').ok_or("bad set")?;
                let id = if id == "+" { last_new } else { id.parse()? };
                let f = t.field_by_name(field).ok_or("no such field")?.id;
                let r = t
                    .records
                    .iter_mut()
                    .find(|r| r.id == id)
                    .ok_or("no record")?;
                r.values.insert(f, value.replace("\\r", "\r"));
            }
            "del" => {
                let id: u32 = rest.parse()?;
                t.records.retain(|r| r.id != id);
            }
            _ => return Err(format!("unknown op {kind}").into()),
        }
    }
    std::fs::write(output, db.save(&file)?)?;
    Ok(())
}
