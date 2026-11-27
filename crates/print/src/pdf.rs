//! A small PDF 1.4 writer: pages with filled rectangles, lines and text in
//! the standard Helvetica fonts (WinAnsiEncoding). Content streams are not
//! compressed. Coordinates are in points from the page's top-left corner.

use crate::look::Color;

#[derive(Default)]
pub struct Pdf {
    /// (width, height, content stream) per page.
    pages: Vec<(f64, f64, Vec<u8>)>,
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
        let (s, ph) = self.out();
        let (cx, cy, cw, chh) = clip;
        let head = format!(
            "q {} {} {} {} re W n BT /{} {} Tf {} {} {} Td (",
            num(cx),
            num(ph - cy - chh),
            num(cw),
            num(chh),
            if bold { "F2" } else { "F1" },
            num(size as f64),
            Self::rgb(c, "rg"),
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

    pub fn finish(self) -> Vec<u8> {
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        let n = self.pages.len();
        // 1 catalog, 2 page tree, 3 and 4 fonts, then (page, contents) pairs.
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
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
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
}
