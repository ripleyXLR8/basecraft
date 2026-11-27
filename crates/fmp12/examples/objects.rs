//! Prints the parts and objects (absolute bounds, parent) of a layout.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = fmp12::File::open(&a[1]).unwrap();
    for l in fmp12::layouts::layouts(&f).unwrap() {
        if l.name != a[2] {
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
}
