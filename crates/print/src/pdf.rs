//! A small PDF 1.4 writer: pages with filled rectangles, lines, pictures
//! and text in the standard Helvetica fonts (WinAnsiEncoding). Content
//! streams are not compressed; JPEG pictures are embedded as they are,
//! others as raw RGB samples (and a soft mask when they have transparency). Coordinates are in points from the page's top-left corner.

use crate::look::Color;

#[derive(Default)]
pub struct Pdf {
    /// (width, height, content stream) per page.
    pages: Vec<(f64, f64, Vec<u8>)>,
    /// Image XObjects `/Im<i>`, shared by every page: (key, image).
    images: Vec<(u64, Image)>,
}

/// An image XObject: its dictionary entries and samples, and the 8-bit
/// alpha samples of its soft mask.
struct Image {
    width: u32,
    height: u32,
    dict: &'static str,
    data: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

impl Image {
    fn new(data: &[u8]) -> Option<Image> {
        if let Some((width, height, comps)) = jpeg_info(data) {
            return Some(Image {
                width,
                height,
                dict: if comps == 1 {
                    "/ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode"
                } else {
                    "/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"
                },
                data: data.to_vec(),
                alpha: None,
            });
        }
        let img = crate::picture::decode(data)?;
        let alpha: Vec<u8> = img.pixels().map(|p| p.0[3]).collect();
        Some(Image {
            width: img.width(),
            height: img.height(),
            dict: "/ColorSpace /DeviceRGB /BitsPerComponent 8",
            data: img
                .pixels()
                .flat_map(|p| [p.0[0], p.0[1], p.0[2]])
                .collect(),
            alpha: alpha.iter().any(|a| *a < 255).then_some(alpha),
        })
    }
}

/// Size and colour components of JPEG data in grey or RGB (others, such
/// as CMYK, are decoded instead).
fn jpeg_info(d: &[u8]) -> Option<(u32, u32, u8)> {
    if !d.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let be16 = |i: usize| Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]) as u32);
    let mut i = 2;
    while i + 9 < d.len() {
        let marker = *d.get(i + 1)?;
        // SOF0–SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let comps = *d.get(i + 9)?;
            return matches!(comps, 1 | 3).then_some((be16(i + 7)?, be16(i + 5)?, comps));
        }
        i += 2 + be16(i + 2)? as usize;
    }
    None
}

/// Helvetica advance widths (1/1000 em) for ' ' to '~', from the standard
/// font metrics.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Width of `s` in points at `size`.
/// ponytail: bold uses the regular widths plus 5 %; non-ASCII letters use
/// an average width.
pub fn text_width(s: &str, size: f32, bold: bool) -> f32 {
    let w: u32 = s
        .chars()
        .map(|c| match c as u32 {
            32..=126 => HELVETICA[c as usize - 32] as u32,
            _ => 556,
        })
        .sum();
    w as f32 * size / 1000.0 * if bold { 1.05 } else { 1.0 }
}

/// Windows-1252 byte of a character ('?' when it has none).
fn cp1252(c: char) -> u8 {
    const HIGH: [(char, u8); 27] = [
        ('€', 0x80),
        ('‚', 0x82),
        ('ƒ', 0x83),
        ('„', 0x84),
        ('…', 0x85),
        ('†', 0x86),
        ('‡', 0x87),
        ('ˆ', 0x88),
        ('‰', 0x89),
        ('Š', 0x8A),
        ('‹', 0x8B),
        ('Œ', 0x8C),
        ('Ž', 0x8E),
        ('‘', 0x91),
        ('’', 0x92),
        ('“', 0x93),
        ('”', 0x94),
        ('•', 0x95),
        ('–', 0x96),
        ('—', 0x97),
        ('˜', 0x98),
        ('™', 0x99),
        ('š', 0x9A),
        ('›', 0x9B),
        ('œ', 0x9C),
        ('ž', 0x9E),
        ('Ÿ', 0x9F),
    ];
    match c as u32 {
        0x20..=0x7E | 0xA0..=0xFF => c as u8,
        _ => HIGH.iter().find(|(h, _)| *h == c).map_or(b'?', |(_, b)| *b),
    }
}

fn num(x: f64) -> String {
    let s = format!("{x:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

impl Pdf {
    pub fn new() -> Pdf {
        Pdf::default()
    }

    pub fn page(&mut self, width: f64, height: f64) {
        self.pages.push((width, height, Vec::new()));
    }

    /// The current page's content stream. Drawing before any `page()` call
    /// starts a US Letter page.
    fn out(&mut self) -> (&mut Vec<u8>, f64) {
        if self.pages.is_empty() {
            self.page(612.0, 792.0);
        }
        let last = self.pages.len() - 1;
        let (_, h, s) = &mut self.pages[last];
        (s, *h)
    }

    fn rgb(c: Color, op: &str) -> String {
        format!(
            "{} {} {} {op}",
            num(c[0] as f64),
            num(c[1] as f64),
            num(c[2] as f64)
        )
    }

    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Color) {
        let (s, ph) = self.out();
        let cmd = format!(
            "{} {} {} {} {} re f\n",
            Self::rgb(c, "rg"),
            num(x),
            num(ph - y - h),
            num(w),
            num(h)
        );
        s.extend_from_slice(cmd.as_bytes());
    }

    pub fn line(&mut self, a: (f64, f64), b: (f64, f64), width: f64, c: Color) {
        let (s, ph) = self.out();
        let cmd = format!(
            "{} {} w {} {} m {} {} l S\n",
            Self::rgb(c, "RG"),
            num(width),
            num(a.0),
            num(ph - a.1),
            num(b.0),
            num(ph - b.1)
        );
        s.extend_from_slice(cmd.as_bytes());
    }

    /// A filled polygon (points from the page's top-left corner).
    pub fn polygon(&mut self, points: &[(f64, f64)], c: Color) {
        let (s, ph) = self.out();
        let mut cmd = Self::rgb(c, "rg");
        for (i, (x, y)) in points.iter().enumerate() {
            let op = if i == 0 { "m" } else { "l" };
            cmd.push_str(&format!(" {} {} {op}", num(*x), num(ph - y)));
        }
        cmd.push_str(
            " h f
",
        );
        s.extend_from_slice(cmd.as_bytes());
    }

    /// Text with its baseline at `y`, clipped to the rectangle `clip`
    /// (x, y, w, h).
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        x: f64,
        y: f64,
        size: f32,
        bold: bool,
        c: Color,
        text: &str,
        clip: (f64, f64, f64, f64),
    ) {
        self.text_slanted(x, y, size, bold, false, c, text, clip);
    }

    /// [`Pdf::text`], slanted by 12° when `italic` (Helvetica-Oblique's
    /// angle, without embedding that font).
    #[allow(clippy::too_many_arguments)]
    pub fn text_slanted(
        &mut self,
        x: f64,
        y: f64,
        size: f32,
        bold: bool,
        italic: bool,
        c: Color,
        text: &str,
        clip: (f64, f64, f64, f64),
    ) {
        let (s, ph) = self.out();
        let (cx, cy, cw, chh) = clip;
        let head = format!(
            "q {} {} {} {} re W n BT /{} {} Tf {} 1 0 {} 1 {} {} Tm (",
            num(cx),
            num(ph - cy - chh),
            num(cw),
            num(chh),
            if bold { "F2" } else { "F1" },
            num(size as f64),
            Self::rgb(c, "rg"),
            if italic { "0.21" } else { "0" },
            num(x),
            num(ph - y)
        );
        s.extend_from_slice(head.as_bytes());
        for ch in text.chars() {
            let b = cp1252(ch);
            if matches!(b, b'(' | b')' | b'\\') {
                s.push(b'\\');
            }
            s.push(b);
        }
        s.extend_from_slice(b") Tj ET Q\n");
    }

    /// Draws picture `data` stretched over (x, y, w, h); `key` identifies
    /// the picture so that it is embedded once. Returns false for data that
    /// is not a picture this writer can decode.
    pub fn image(&mut self, key: u64, data: &[u8], r: (f64, f64, f64, f64)) -> bool {
        let i = match self.images.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let Some(img) = Image::new(data) else {
                    return false;
                };
                self.images.push((key, img));
                self.images.len() - 1
            }
        };
        let (s, ph) = self.out();
        let (x, y, w, h) = r;
        let cmd = format!(
            "q {} 0 0 {} {} {} cm /Im{i} Do Q\n",
            num(w),
            num(h),
            num(x),
            num(ph - y - h)
        );
        s.extend_from_slice(cmd.as_bytes());
        true
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        let n = self.pages.len();
        // 1 catalog, 2 page tree, 3 and 4 fonts, then (page, contents)
        // pairs, then the images (each followed by its soft mask, if any).
        let stream = |dict: String, data: &[u8]| {
            let mut o = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
            o.extend_from_slice(data);
            o.extend_from_slice(b"\nendstream");
            o
        };
        let mut next = 5 + 2 * n;
        let mut xobjects = String::new();
        let mut image_objects = Vec::new();
        for (i, (_, img)) in self.images.iter().enumerate() {
            xobjects.push_str(&format!(" /Im{i} {next} 0 R"));
            let size = format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {}",
                img.width, img.height
            );
            let mask = match &img.alpha {
                Some(_) => format!(" /SMask {} 0 R", next + 1),
                None => String::new(),
            };
            image_objects.push(stream(format!("{size} {}{mask}", img.dict), &img.data));
            next += 1;
            if let Some(a) = &img.alpha {
                let dict = format!("{size} /ColorSpace /DeviceGray /BitsPerComponent 8");
                image_objects.push(stream(dict, a));
                next += 1;
            }
        }
        let xobjects = if xobjects.is_empty() {
            String::new()
        } else {
            format!(" /XObject <<{xobjects} >>")
        };
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {n} >>",
                kids.join(" ")
            )
            .into_bytes(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
                .to_vec(),
        ];
        for (i, (w, h, content)) in self.pages.into_iter().enumerate() {
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >>{xobjects} >> /Contents {} 0 R >>",
                    num(w),
                    num(h),
                    6 + 2 * i
                )
                .into_bytes(),
            );
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(&content);
            stream.extend_from_slice(b"\nendstream");
            objects.push(stream);
        }
        objects.extend(image_objects);
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_valid_skeleton() {
        let mut p = Pdf::new();
        p.page(612.0, 792.0);
        p.fill_rect(10.0, 10.0, 100.0, 20.0, [1.0, 0.0, 0.0, 1.0]);
        p.text(
            12.0,
            25.0,
            12.0,
            false,
            [0.0; 4],
            "Caf\u{e9} (1) \\",
            (10.0, 10.0, 100.0, 20.0),
        );
        let b = p.finish();
        assert!(b.starts_with(b"%PDF-1.4"));
        assert!(b.ends_with(b"%%EOF\n"));
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("1 0 0 rg 10 762 100 20 re f"));
        let want = b"(Caf\xE9 \\(1\\) \\\\) Tj";
        assert!(b.windows(want.len()).any(|w| w == want));
        // xref offsets point at the objects.
        let xref = b.windows(5).position(|w| w == b"xref\n").unwrap();
        let entry = &b[xref + 5 + "0 7\n".len() + 20..][..10];
        let first: usize = std::str::from_utf8(entry).unwrap().parse().unwrap();
        assert!(b[first..].starts_with(b"1 0 obj"));
        assert!((text_width("Hi", 10.0, false) - 9.44).abs() < 1e-4);
    }

    #[test]
    fn embeds_pictures_once() {
        let mut p = Pdf::new();
        p.page(100.0, 100.0);
        // JPEG: passed through with its SOF size and components.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 3, 0, 4, 3, 0, 0, 0, 0, 0, 0, 0xFF, 0xD9,
        ];
        assert!(p.image(1, &jpeg, (10.0, 10.0, 40.0, 30.0)));
        assert!(p.image(1, &jpeg, (10.0, 50.0, 40.0, 30.0)));
        assert!(!p.image(2, b"not a picture", (0.0, 0.0, 1.0, 1.0)));
        // A 1×1 24-bit BMP, one red pixel: raw RGB, opaque (no soft mask).
        let mut bmp = b"BM".to_vec();
        bmp.extend(58u32.to_le_bytes());
        bmp.extend([0; 4]);
        bmp.extend(54u32.to_le_bytes());
        bmp.extend(40u32.to_le_bytes());
        bmp.extend(1i32.to_le_bytes());
        bmp.extend(1i32.to_le_bytes());
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(24u16.to_le_bytes());
        bmp.extend([0; 24]);
        bmp.extend([0, 0, 255, 0]);
        assert!(p.image(3, &bmp, (0.0, 0.0, 10.0, 10.0)));
        let b = p.finish();
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("q 40 0 0 30 10 60 cm /Im0 Do Q"));
        assert!(s.contains("q 40 0 0 30 10 20 cm /Im0 Do Q"));
        assert!(s.contains("/XObject << /Im0 7 0 R /Im1 8 0 R >>"));
        assert!(s.contains(
            "/Width 4 /Height 3 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"
        ));
        let want: &[u8] = b"/Length 3 >>\nstream\n\xFF\x00\x00\nendstream";
        assert!(b.windows(want.len()).any(|w| w == want));
        assert!(!s.contains("/SMask"));
        assert_eq!(crate::merge::pages(&b), Ok(1));
    }
}
