//! Prints the parts and objects (absolute bounds, parent) of a layout.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let [_, file, name, ..] = a.as_slice() else {
        return Err("usage: objects <file> <layout>".into());
    };
    let f = fmp12::File::open(file)?;
    for l in fmp12::layouts::layouts(&f)? {
        if l.name != *name {
            continue;
        }
        for p in &l.parts {
            println!("part kind {} top {} height {}", p.kind, p.top, p.height);
        }
        for o in &l.objects {
            let b = l.absolute(o);
            println!(
                "obj {:3} kind {:2} parent {:?} abs ({}, {}, {}, {}) field {:?} text {:?}",
                o.id, o.kind, o.parent, b.top, b.left, b.bottom, b.right, o.field, o.text
            );
        }
    }
    Ok(())
}
