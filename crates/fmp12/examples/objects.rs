//! Prints the parts and objects (absolute bounds, parent, option bits) of a
//! layout (`*` = every layout).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let [_, file, name, ..] = a.as_slice() else {
        return Err("usage: objects <file> <layout|*>".into());
    };
    let f = fmp12::File::open(file)?;
    for l in fmp12::layouts::layouts(&f)? {
        if l.name != *name && name != "*" {
            continue;
        }
        println!("layout {} {:?}", l.id, l.name);
        for p in &l.parts {
            println!("part kind {} top {} height {}", p.kind, p.top, p.height);
        }
        for o in &l.objects {
            let b = l.absolute(o);
            println!(
                "obj {:3} kind {:2} options {:#x} parent {:?} abs ({}, {}, {}, {}) field {:?} text {:?}",
                o.id,
                o.kind,
                o.options,
                o.parent,
                b.top,
                b.left,
                b.bottom,
                b.right,
                o.field,
                o.text
            );
            if let Some(p) = o.picture {
                println!("    picture {p}");
            }
            if o.button_raw.is_some() || o.button.is_some() {
                println!(
                    "    button {:?} raw {:02x?}",
                    o.button.as_ref().map(|s| (s.id, s.inline_id())),
                    o.button_raw
                );
            }
        }
    }
    Ok(())
}
