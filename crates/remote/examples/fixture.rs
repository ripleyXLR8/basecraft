//! Finishes the Data API test file (`tools/oracle/remote_fixture.sh`):
//! puts every field of `Contacts` on its layout (the Data API returns only
//! the fields a layout shows) and adds the script `Echo` (Exit Script
//! [ "echo:" & Get ( ScriptParameter ) ]).
//! `cargo run -p remote --example fixture -- <in.fmp12> <out.fmp12>`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::scripts::{Calc, Script, Step, StepParams, id};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = fmp12::File::open(&a[1]).unwrap();
    let tables = fmp12::schema::tables(&f).unwrap();
    let t = tables.iter().find(|t| t.name == "Contacts").unwrap();
    let mut ls = fmp12::layouts::layouts(&f).unwrap();
    let l = ls.iter_mut().find(|l| l.name == "Contacts").unwrap();
    let body = l.parts.iter().find(|p| p.kind == 4).unwrap().clone();
    let first = l.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
    for (i, (id, fid)) in (first..).zip(t.fields.keys()).enumerate() {
        let x = 10.0 + 114.0 * i as f64;
        l.objects.push(fmp12::layouts::LayoutObject {
            id,
            kind: 1,
            bounds: fmp12::layouts::Bounds {
                top: body.top + 4.0,
                left: x,
                bottom: body.top + 24.0,
                right: x + 110.0,
            },
            field: Some((l.table_occurrence, *fid as u64, 1)),
            ..Default::default()
        });
        l.width = l.width.max(x + 120.0);
    }
    let bytes = fmp12::save::save_layouts(&f, &ls).unwrap();
    if a.get(3).is_some_and(|s| s == "--no-script") {
        std::fs::write(&a[2], bytes).unwrap();
        return;
    }
    let f = fmp12::File::from_bytes(bytes).unwrap();
    let names = fmp12::calc::FileNames::load(&f).unwrap();
    let calc = fmp12::calc::compile("\"echo:\" & Get ( ScriptParameter )", 0, &names)
        .unwrap()
        .tokens;
    let mut exit = Step::new(id::EXIT_SCRIPT);
    exit.params = StepParams::ExitScript(Some(Calc(calc)));
    let mut list = fmp12::scripts::scripts(&f).unwrap();
    list.push(Script {
        id: list.iter().map(|s| s.id).max().unwrap_or(0) + 1,
        name: "Echo".into(),
        options: 8,
        steps: vec![exit],
    });
    std::fs::write(&a[2], fmp12::scripts_save::save_scripts(&f, &list).unwrap()).unwrap();
}
