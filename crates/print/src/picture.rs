//! Pictures of container fields and graphic objects: PNG, JPEG and BMP
//! data decoded for Browse, Preview and PDF output.

/// Pixels of picture data the `image` crate decodes (PNG, JPEG, BMP).
/// GIF and other files give `None`.
pub fn decode(data: &[u8]) -> Option<image::RgbaImage> {
    if let Ok(img) = image::load_from_memory(data) {
        return Some(img.to_rgba8());
    }
    let bmp = with_file_header(data)?;
    image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
        .ok()
        .map(|i| i.to_rgba8())
}

/// A device-independent bitmap without its 14-byte `BM` file header (as on
/// the Windows clipboard) with one put in front. ponytail: no `BMPf` stream
/// seen yet, so whether FileMaker stores the header is unverified; both work.
fn with_file_header(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let u16_at = |i: usize| Some(u16::from_le_bytes(dib.get(i..i + 2)?.try_into().ok()?));
    let header = u32_at(0)?;
    if ![12, 40, 52, 56, 108, 124].contains(&header) {
        return None;
    }
    let (bits, entry) = if header == 12 {
        (u16_at(10)?, 3)
    } else {
        (u16_at(14)?, 4)
    };
    let used = if header >= 40 { u32_at(32)? } else { 0 };
    let colors = match (used, bits) {
        (0, 1..=8) => 1u32 << bits,
        (n, _) => n,
    };
    // BI_BITFIELDS masks follow a 40-byte header.
    let masks = if header == 40 && matches!(u32_at(16)?, 3 | 6) {
        12
    } else {
        0
    };
    let offset = 14 + header + masks + colors.checked_mul(entry)?;
    let mut out = b"BM".to_vec();
    out.extend(u32::try_from(dib.len() + 14).ok()?.to_le_bytes());
    out.extend([0; 4]);
    out.extend(offset.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 24-bit BMP: blue then red pixel (BGR, rows padded to 4 bytes).
    fn dib() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(40u32.to_le_bytes());
        d.extend(2i32.to_le_bytes());
        d.extend(1i32.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(24u16.to_le_bytes());
        d.extend([0; 24]);
        d.extend([255, 0, 0, 0, 0, 255, 0, 0]);
        d
    }

    #[test]
    fn decodes_bmp_with_and_without_file_header() {
        let full = with_file_header(&dib()).unwrap();
        assert_eq!(&full[..2], b"BM");
        for data in [full, dib()] {
            let img = decode(&data).unwrap();
            assert_eq!(img.dimensions(), (2, 1));
            assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255]);
            assert_eq!(img.get_pixel(1, 0).0, [255, 0, 0, 255]);
        }
        assert!(decode(b"GIF89a\x01\x00\x01\x00").is_none());
        assert!(decode(b"hello").is_none());
    }
}
