//! The fonts a file asks for, in egui. Each font catalog family list
//! (theme and local styles, styled-text runs) becomes an egui font family
//! named after it, filled on first use with the installed face
//! ([`print::fonts::find`], which substitutes the default sans-serif font
//! like FileMaker), then egui's own fonts for characters that face lacks.
//! The web build has no system fonts and keeps egui's.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontInsert};

const PREFIX: &str = "basecraft-font:";

/// Set once [`install`] has run: the named families can be used.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Lets `ctx` draw the files' fonts (from its next pass on).
pub fn install(ctx: &egui::Context) {
    ctx.add_font_provider(Arc::new(Provider));
    INSTALLED.store(true, Ordering::Relaxed);
}

/// The egui family for font family list `family` in the given weight and
/// slant, and whether bold and italic must still be faked (the family has
/// no such face, or the text falls back to egui's font).
pub fn family(family: Option<&str>, bold: bool, italic: bool) -> (FontFamily, bool, bool) {
    if !INSTALLED.load(Ordering::Relaxed) {
        return (FontFamily::Proportional, bold, italic);
    }
    named(family, bold, italic)
}

fn named(family: Option<&str>, bold: bool, italic: bool) -> (FontFamily, bool, bool) {
    let Some((list, face)) = family.and_then(|l| Some((l, print::fonts::find(l, bold, italic)?)))
    else {
        return (FontFamily::Proportional, bold, italic);
    };
    let name = format!("{PREFIX}{}{}{list}", u8::from(bold), u8::from(italic));
    (
        FontFamily::Name(name.into()),
        face.fake_bold,
        face.fake_italic,
    )
}

/// Fills the families named by [`family`].
struct Provider;

impl egui::FontProvider for Provider {
    fn fonts_for_family(&self, family: &FontFamily) -> Vec<FontInsert> {
        let FontFamily::Name(name) = family else {
            return Vec::new();
        };
        let Some(rest) = name.strip_prefix(PREFIX) else {
            return Vec::new();
        };
        let (Some(flags), Some(list)) = (rest.get(..2), rest.get(2..)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let (bold, italic) = (flags.starts_with('1'), flags.ends_with('1'));
        if let Some(f) = print::fonts::find(list, bold, italic) {
            out.push(FontInsert {
                name: name.to_string(),
                data: FontData::from_blob(f.data, f.index),
                families: Vec::new(),
            });
        }
        // egui's own fonts for the characters the face lacks.
        let defs = FontDefinitions::default();
        for n in defs
            .families
            .get(&FontFamily::Proportional)
            .into_iter()
            .flatten()
        {
            if let Some(d) = defs.font_data.get(n) {
                out.push(FontInsert {
                    name: n.clone(),
                    data: (**d).clone(),
                    families: Vec::new(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::FontProvider as _;

    /// Arial (shipped with Windows) gets its own egui family with a real
    /// bold face, drawn in Arial; no family list keeps egui's font.
    /// Skipped where Arial is not installed.
    #[test]
    fn families_for_installed_fonts() {
        assert_eq!(
            named(None, true, false),
            (FontFamily::Proportional, true, false)
        );
        if !std::path::Path::new("C:/Windows/Fonts/arial.ttf").exists() {
            return;
        }
        let (f, fake_bold, _) = named(Some("arial"), true, false);
        assert_eq!(f, FontFamily::Name("basecraft-font:10arial".into()));
        assert!(!fake_bold);
        assert!(Provider.fonts_for_family(&f).len() > 1);
        let ctx = egui::Context::default();
        ctx.add_font_provider(Arc::new(Provider));
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        // Arial Bold's H is 722/1000 em.
        let w = ctx.fonts_mut(|fonts| fonts.glyph_width(&egui::FontId::new(10.0, f), 'H'));
        assert!((w - 7.22).abs() < 0.05, "{w}");
    }
}
