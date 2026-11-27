//! Layout script triggers against the XML export of the oracle files
//! (directory in FMP12_ORACLE: `<name>.fmp12` + `<name>.xml` pairs).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[test]
fn layout_triggers_match_xml() {
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
        let t = fmp12::triggers::triggers(&fmp12::File::open(&p).unwrap()).unwrap();
        for l in doc.descendants().filter(|x| x.has_tag_name("Layout")) {
            let Some(id) = l.attribute("id").and_then(|v| v.parse::<u32>().ok()) else {
                continue;
            };
            let want: Vec<(u16, u32)> = l
                .children()
                .filter(|c| c.has_tag_name("ScriptTriggers"))
                .flat_map(|c| c.children().filter(|x| x.has_tag_name("ScriptTrigger")))
                .map(|x| {
                    let s = x
                        .children()
                        .find(|c| c.has_tag_name("ScriptReference"))
                        .and_then(|c| c.attribute("id"))
                        .unwrap();
                    (
                        x.attribute("id").unwrap().parse().unwrap(),
                        s.parse().unwrap(),
                    )
                })
                .collect();
            if want.is_empty() && !t.layouts.contains_key(&id) {
                continue;
            }
            let got: Vec<(u16, u32)> = t
                .layouts
                .get(&id)
                .map(|v| v.iter().map(|t| (t.event, t.script)).collect())
                .unwrap_or_default();
            assert_eq!(got, want, "{} layout {id}", p.display());
            n += want.len();
        }
    }
    eprintln!("{n} layout triggers match");
}
