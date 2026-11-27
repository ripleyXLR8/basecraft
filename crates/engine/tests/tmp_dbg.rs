#[test]
fn tmp_dbg() {
    let f = fmp12::File::open(std::env::var("F").unwrap()).unwrap();
    for t in fmp12::schema::tables(&f).unwrap() {
        for (id, fd) in &t.fields {
            println!("T {} {} {id} {}", t.id, t.name, fd.name);
        }
    }
    for l in fmp12::layouts::layouts(&f).unwrap() {
        println!("L {} {}", l.id, l.name);
    }
}
