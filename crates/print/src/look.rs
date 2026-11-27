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
