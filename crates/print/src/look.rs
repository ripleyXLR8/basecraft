//! Theme styles resolved into paint attributes. Same resolution rules as
//! the app's `look.rs` (theme default, named style, local style), without
//! the egui types.

use fmp12::theme::{INNER_BORDER, NORMAL, ObjectStyle, PropsExt, SELF, TEXT, Theme, prop};

/// RGBA, 0–1.
pub type Color = [f32; 4];

#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub background: Option<Color>,
    /// Border width and colour per side: top, right, bottom, left.
    pub borders: [(f32, Color); 4],
    pub color: Color,
    pub font_size: f32,
    pub bold: bool,
    /// `font-style: italic`.
    pub italic: bool,
    /// Font family list from the font catalog, e.g. "arial,sans-serif"
    /// (`None`: Helvetica).
    pub family: Option<String>,
    /// The file's font catalog, for the fonts of styled-text runs.
    pub fonts: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    /// 0 left, 1 centre, 2 right.
    pub align: u8,
    /// 0 top, 1 centre, 2 bottom.
    pub valign: u8,
    /// Padding: top, right, bottom, left.
    pub padding: [f32; 4],
}

impl Default for Look {
    fn default() -> Self {
        Look {
            background: None,
            borders: [(0.0, [0.0; 4]); 4],
            color: [0.0, 0.0, 0.0, 1.0],
            font_size: 12.0,
            bold: false,
            italic: false,
            family: None,
            fonts: Default::default(),
            align: 0,
            valign: 1,
            padding: [2.0; 4],
        }
    }
}

fn color(c: fmp12::theme::Rgba) -> Color {
    [c.r, c.g, c.b, c.a]
}

/// Effective look of an object style under `theme`, in the normal state.
pub fn resolve(theme: Option<&Theme>, style: &ObjectStyle) -> Look {
    let Some(th) = theme else {
        return Look::default();
    };
    let p = th.resolve(style, SELF, NORMAL);
    let inner = th.resolve(style, INNER_BORDER, NORMAL);
    let text = th.resolve(style, TEXT, NORMAL);
    let mut l = Look::default();
    let font_size = p.length(prop::FONT_SIZE).map_or(12.0, |x| x.pt(12.0, 12.0));
    l.font_size = font_size as f32;
    l.background = p
        .color(prop::BACKGROUND_COLOR)
        .map(color)
        .filter(|c| c[3] > 0.0);
    for i in 0..4u32 {
        // Border style 1 = none.
        let none = p.enum_value(prop::BORDER_STYLE + i) == Some(1);
        let w = p
            .length(prop::BORDER_WIDTH + i)
            .map_or(0.0, |x| x.pt(font_size, 0.0) as f32);
        let c = p.color(prop::BORDER_COLOR + i).map_or([0.0; 4], color);
        l.borders[i as usize] = if none { (0.0, c) } else { (w, c) };
        l.padding[i as usize] = inner
            .length(prop::PADDING + i)
            .map_or(2.0, |x| x.pt(font_size, 0.0) as f32);
    }
    // Text colour and weight may live on the `.text` element or on `.self`.
    if let Some(c) = text
        .color(prop::COLOR)
        .or_else(|| p.color(prop::COLOR))
        .filter(|c| c.a > 0.0)
    {
        l.color = color(c);
    }
    l.bold = text
        .enum_value(prop::FONT_WEIGHT)
        .or_else(|| p.enum_value(prop::FONT_WEIGHT))
        == Some(2);
    l.italic = text
        .enum_value(prop::FONT_STYLE)
        .or_else(|| p.enum_value(prop::FONT_STYLE))
        == Some(2);
    l.family = text
        .enum_value(prop::FONT_FAMILY)
        .or_else(|| p.enum_value(prop::FONT_FAMILY))
        .and_then(|i| family(&th.fonts, i + 128));
    l.fonts = th.fonts.clone();
    l.align = match p.enum_value(prop::TEXT_ALIGN) {
        Some(2) => 1,
        Some(3) => 2,
        _ => 0,
    };
    l.valign = p
        .enum_value(prop::VERTICAL_ALIGN)
        .map_or(1, |v| v.clamp(0, 2) as u8);
    l
}

/// The family list of font catalog id `id`.
pub fn family(fonts: &std::collections::BTreeMap<u32, String>, id: i64) -> Option<String> {
    fonts
        .get(&u32::try_from(id).ok()?)
        .filter(|f| !f.is_empty())
        .cloned()
}

/// An RGB colour (bytes) as a paint colour.
pub fn rgb(c: [u8; 3]) -> Color {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fmp12::theme::Value;

    /// font-family 64 = catalog id − 128, font-style 67 = 2 italic.
    #[test]
    fn family_and_italic() {
        let mut style = ObjectStyle::default();
        let own = style.local.entry((SELF, NORMAL)).or_default();
        own.insert(prop::FONT_FAMILY, Value::Enum(1));
        own.insert(prop::FONT_STYLE, Value::Enum(2));
        let theme = Theme {
            fonts: std::sync::Arc::new(
                [
                    (128, "Helvetica Neue".into()),
                    (129, "arial,sans-serif".into()),
                ]
                .into(),
            ),
            ..Default::default()
        };
        let l = resolve(Some(&theme), &style);
        assert_eq!(l.family.as_deref(), Some("arial,sans-serif"));
        assert!(l.italic);
        assert_eq!(family(&l.fonts, 128).as_deref(), Some("Helvetica Neue"));
        assert_eq!(family(&l.fonts, -1), None);
    }
}
