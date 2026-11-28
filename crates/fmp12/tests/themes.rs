//! Decodes the themes of the FileMaker starter templates and checks values
//! verified by hand against FileMaker's theme CSS. Skips when the templates
//! are absent (e.g. on CI). Override the location with FMP12_TEMPLATES.

use std::path::PathBuf;

use fmp12::theme::{self, ICON, INNER_BORDER, NORMAL, ObjectStyle, PropsExt, SELF, Unit, prop};

fn template_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FMP12_TEMPLATES")
        .map(PathBuf::from)
        .unwrap_or_else(|| r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates".into());
    dir.is_dir().then_some(dir)
}

#[test]
fn every_template_theme_decodes() {
    let Some(dir) = template_dir() else {
        eprintln!("no FileMaker templates found, skipping");
        return;
    };
    let mut n = 0;
    for t in std::fs::read_dir(dir).unwrap() {
        let Ok(files) = std::fs::read_dir(t.unwrap().path()) else {
            continue;
        };
        for f in files {
            let f = f.unwrap().path();
            if f.extension().is_none_or(|e| e != "fmp12") {
                continue;
            }
            let file = fmp12::File::open(&f).unwrap();
            let themes = theme::themes(&file).unwrap();
            assert!(!themes.is_empty(), "{}", f.display());
            for th in &themes {
                assert!(th.id.starts_with("com.filemaker.theme."), "{}", th.id);
                assert!(th.styles.len() >= 30, "{}: {}", f.display(), th.name);
            }
            for l in fmp12::layouts::layouts(&file).unwrap() {
                if l.theme == 0 {
                    continue;
                }
                let th = themes.iter().find(|t| t.number == l.theme).unwrap();
                assert_eq!(ObjectStyle::of_layout(&l).object_type, 20);
                for o in &l.objects {
                    let s = ObjectStyle::parse(&o.style_raw);
                    assert!(
                        !th.resolve(&s, SELF, NORMAL).is_empty(),
                        "{}: {} object {} type {}",
                        f.display(),
                        l.name,
                        o.id,
                        s.object_type
                    );
                }
            }
            n += 1;
        }
    }
    assert!(n > 0);
}

#[test]
fn universal_touch_matches_css() {
    let Some(dir) = template_dir() else { return };
    let file = fmp12::File::open(dir.join("Contacts").join("en.fmp12")).unwrap();
    let themes = theme::themes(&file).unwrap();
    let ut = &themes[0];
    assert_eq!((ut.number, ut.name.as_str()), (1, "Universal Touch"));
    assert_eq!(ut.id, "com.filemaker.theme.universal_touch");
    let fonts = theme::fonts(&file).unwrap();

    // button:normal .self — #007aff, 16pt, centred both ways, no border.
    let button = ObjectStyle {
        object_type: 80,
        ..Default::default()
    };
    let p = ut.resolve(&button, SELF, NORMAL);
    assert_eq!(p.color(prop::COLOR).unwrap().hex(), "#007aff");
    let size = p.length(prop::FONT_SIZE).unwrap();
    assert_eq!((size.value, size.unit), (16.0, Unit::Pt));
    assert_eq!(p.enum_value(prop::TEXT_ALIGN), Some(2));
    assert_eq!(p.enum_value(prop::VERTICAL_ALIGN), Some(1));
    assert_eq!(p.enum_value(prop::BORDER_STYLE), Some(1));
    assert_eq!(p.color(prop::BACKGROUND_COLOR).unwrap().a, 0.0);
    assert_eq!(p.length(prop::LINE_HEIGHT).unwrap().unit, Unit::Line);
    let family = p.enum_value(prop::FONT_FAMILY).unwrap() as u32 + 128;
    assert_eq!(fonts[&family], "arial,sans-serif");
    // button:normal .inner_border — padding 0.88em 0.75em 0.63em 0.75em.
    let pad = ut.resolve(&button, INNER_BORDER, NORMAL);
    let em: Vec<f64> = (0..4)
        .map(|i| pad.length(prop::PADDING + i).unwrap().value)
        .collect();
    assert_eq!(em, [0.88, 0.75, 0.63, 0.75]);
    // button:pressed .icon — -fm-icon-color rgba(50%, 74.17%, 100%).
    let icon = ut.resolve(&button, ICON, 3);
    assert_eq!(icon.color(514).unwrap().hex(), "#80bdff");

    // edit_box: bottom border only, #c8c7cc 1pt; :focus #0070cf.
    let edit = ObjectStyle {
        object_type: 61,
        ..Default::default()
    };
    let p = ut.resolve(&edit, SELF, NORMAL);
    let styles: Vec<_> = (0..4)
        .map(|i| p.enum_value(prop::BORDER_STYLE + i).unwrap())
        .collect();
    assert_eq!(styles, [1, 1, 5, 1]);
    assert_eq!(p.color(prop::BORDER_COLOR + 2).unwrap().hex(), "#c8c7cc");
    assert_eq!(p.length(prop::BORDER_WIDTH + 2).unwrap().value, 1.0);
    let focus = ut.resolve(&edit, SELF, 4);
    assert_eq!(focus.color(prop::BORDER_COLOR).unwrap().hex(), "#0070cf");

    // button_bar: 4pt corner radius.
    let bar = ObjectStyle {
        object_type: 78,
        ..Default::default()
    };
    let p = ut.resolve(&bar, SELF, NORMAL);
    let theme::Value::Pair(h, _) = &p[&prop::BORDER_RADIUS] else {
        panic!("radius")
    };
    assert_eq!((h.value, h.unit), (4.0, Unit::Pt));

    // A text object styled "small-primary-text" (13pt) on "Contact List".
    let layouts = fmp12::layouts::layouts(&file).unwrap();
    let list = layouts.iter().find(|l| l.name == "Contact List").unwrap();
    assert_eq!(list.theme, 1);
    let o = list
        .objects
        .iter()
        .find(|o| o.style == "small-primary-text")
        .unwrap();
    let s = ObjectStyle::parse(&o.style_raw);
    assert_eq!(s.object_type, 70);
    let p = ut.resolve(&s, SELF, NORMAL);
    assert_eq!(p.length(prop::FONT_SIZE).unwrap().value, 13.0);
    assert_eq!(p.color(prop::COLOR).unwrap().hex(), "#000000");
    // The layout background is white.
    let bg = ut.resolve(&ObjectStyle::of_layout(list), SELF, NORMAL);
    assert!(bg.color(prop::BACKGROUND_COLOR).is_some());
}

/// BaseCraft's own theme (new files): FileMaker Pro 26 draws no text in a
/// text object whose `.text` element has no height; the `.text` rule
/// `height: 100%` makes text, edit boxes and buttons show (checked in
/// FileMaker, Browse and Layout modes).
#[test]
fn plain_theme_gives_text_a_box() {
    let file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("T").unwrap()).unwrap();
    let themes = theme::themes(&file).unwrap();
    assert_eq!(themes[0].id, "org.basecraft.theme.plain");
    let fonts = theme::fonts(&file).unwrap();
    for ty in [61, 70, 80] {
        let s = ObjectStyle {
            object_type: ty,
            ..Default::default()
        };
        let text = themes[0].resolve(&s, theme::TEXT, NORMAL);
        let h = text.length(110).unwrap();
        assert_eq!((h.value, h.unit), (100.0, Unit::Percent), "type {ty}");
        let p = themes[0].resolve(&s, SELF, NORMAL);
        let family = p.enum_value(prop::FONT_FAMILY).unwrap() as u32 + 128;
        assert!(fonts.contains_key(&family), "type {ty}");
        assert_eq!(p.color(prop::COLOR).unwrap().hex(), "#000000");
    }
}
