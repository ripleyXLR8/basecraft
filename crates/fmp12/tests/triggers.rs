//! Script triggers (layouts, objects, file) against the XML export of the
//! oracle files (directory in FMP12_ORACLE: `<name>.fmp12` + `<name>.xml`
//! pairs): event, script, modes and parameter text.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::triggers::mode;

type Key = (u16, u32, u32, Option<String>);

#[test]
fn triggers_match_xml() {
    let Some(dir) = std::env::var_os("FMP12_ORACLE") else {
        return;
    };
    let mut n = 0;
    for f in std::fs::read_dir(dir).unwrap().flatten() {
        let p = f.path();
        if p.extension().is_none_or(|e| e != "fmp12") {
            continue;
        }
        let Ok(xml) = std::fs::read_to_string(p.with_extension("xml")) else {
            continue;
        };
        let doc = roxmltree::Document::parse_with_options(
            &xml,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        let file = fmp12::File::open(&p).unwrap();
        let names = fmp12::calc::FileNames::load(&file).unwrap();
        let t = fmp12::triggers::triggers(&file).unwrap();
        let mut want: Vec<Key> = doc
            .descendants()
            .filter(|x| x.has_tag_name("ScriptTrigger"))
            .map(|x| {
                let r = x
                    .children()
                    .find(|c| c.has_tag_name("ScriptReference"))
                    .unwrap();
                let event: u16 = x.attribute("id").unwrap().parse().unwrap();
                let on = |a: &str, bit: u32| {
                    if x.attribute(a) == Some("True") {
                        bit
                    } else {
                        0
                    }
                };
                let modes = if event > 200 {
                    0
                } else {
                    on("browseMode", mode::BROWSE)
                        | on("findMode", mode::FIND)
                        | on("previewMode", mode::PREVIEW)
                };
                let param = r
                    .descendants()
                    .find(|c| c.has_tag_name("Text"))
                    .and_then(|c| c.text())
                    .map(str::to_string);
                (
                    event,
                    r.attribute("id").unwrap().parse().unwrap(),
                    modes,
                    param,
                )
            })
            .collect();
        let mut got: Vec<Key> = t
            .layouts
            .values()
            .chain(t.objects.values())
            .flatten()
            .chain(&t.file)
            .map(|t| {
                (
                    t.event,
                    t.script,
                    t.modes,
                    t.parameter.as_ref().map(|c| c.text(&names).unwrap()),
                )
            })
            .collect();
        want.sort();
        got.sort();
        assert_eq!(got, want, "{}", p.display());
        n += want.len();
    }
    eprintln!("{n} triggers match");
}
