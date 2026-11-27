//! Platform, service and container functions. Those that need the device,
//! the window or a model go through [`Context::host_function`]
//! (crate::Context::host_function) and fail with "not available" (error 3)
//! by default; the rest are computed here.

use super::{int, txt};
use crate::error::EvalError;
use crate::eval::{Evaluator, R, unavailable};
use crate::num;
use crate::value::{Container, Value};

fn host(ev: &mut Evaluator, name: &str, a: &[Value]) -> R {
    ev.ctx
        .host_function(name, a)
        .unwrap_or_else(|| Err(unavailable(name)))
}

macro_rules! host_fns {
    ($($f:ident => $name:literal),* $(,)?) => {
        $(pub fn $f(ev: &mut Evaluator, a: &[Value]) -> R {
            host(ev, $name, a)
        })*
    };
}

host_fns! {
    location => "Location",
    location_values => "LocationValues",
    get_sensor => "GetSensor",
    range_beacons => "RangeBeacons",
    furigana => "Furigana",
    get_live_text => "GetLiveText",
    get_live_text_as_json => "GetLiveTextAsJSON",
    read_qr_code => "ReadQRCode",
    get_text_from_pdf => "GetTextFromPDF",
    get_thumbnail => "GetThumbnail",
    get_av_player_attribute => "GetAVPlayerAttribute",
    get_addon_info => "GetAddonInfo",
    get_persistent_data => "GetPersistentData",
    list_persistent_data_ids => "ListPersistentDataIDs",
    get_layout_object_attribute => "GetLayoutObjectAttribute",
    get_layout_object_owner_info => "GetLayoutObjectOwnerInfo",
    get_embedding => "GetEmbedding",
    get_token_count => "GetTokenCount",
    compute_model => "ComputeModel",
    predict_from_model => "PredictFromModel",
    get_model_attributes => "GetModelAttributes",
    get_rag_space_info => "GetRAGSpaceInfo",
}

// ---- embeddings ----------------------------------------------------------------

/// An embedding is a JSON array of numbers as text, or a container of
/// little-endian 32-bit floats.
// ponytail: the float32 container layout is assumed; FileMaker does not
// document its embedding file format.
fn vector(v: &Value) -> Result<(Vec<f64>, bool), EvalError> {
    let bad = || EvalError::generic("not an embedding");
    match v {
        Value::Container(c) => {
            let (chunks, rest) = c.data.as_chunks::<4>();
            if !rest.is_empty() {
                return Err(bad());
            }
            Ok((
                chunks
                    .iter()
                    .map(|b| f32::from_le_bytes(*b) as f64)
                    .collect(),
                true,
            ))
        }
        v => match serde_json::from_str::<serde_json::Value>(&v.to_text()) {
            Ok(serde_json::Value::Array(a)) => Ok((
                a.iter()
                    .map(|x| x.as_f64().ok_or_else(bad))
                    .collect::<Result<_, _>>()?,
                false,
            )),
            _ => Err(bad()),
        },
    }
}

fn as_text(v: &[f64]) -> String {
    let items: Vec<String> = v.iter().map(|x| format!("{}", *x as f32)).collect();
    format!("[{}]", items.join(","))
}

fn as_file(v: &[f64], name: &str) -> Value {
    Value::Container(Container {
        name: name.into(),
        data: v.iter().flat_map(|x| (*x as f32).to_le_bytes()).collect(),
    })
}

fn same_kind(v: &[f64], binary: bool) -> Value {
    if binary {
        as_file(v, "embedding.fve")
    } else {
        Value::Text(as_text(v))
    }
}

fn pair(a: &[Value]) -> Result<(Vec<f64>, Vec<f64>, bool), EvalError> {
    let (x, binary) = vector(&a[0])?;
    let (y, _) = vector(&a[1])?;
    if x.len() != y.len() {
        return Err(EvalError::generic("embeddings differ in dimension"));
    }
    Ok((x, y, binary))
}

pub fn cosine_similarity(_: &mut Evaluator, a: &[Value]) -> R {
    let (x, y, _) = pair(a)?;
    let dot: f64 = x.iter().zip(&y).map(|(p, q)| p * q).sum();
    let norm = |v: &[f64]| v.iter().map(|p| p * p).sum::<f64>().sqrt();
    let d = norm(&x) * norm(&y);
    if d == 0.0 {
        return Err(EvalError::generic("zero vector"));
    }
    num::from_f64(dot / d, 15)
        .map(Value::num)
        .ok_or_else(|| EvalError::generic("invalid similarity"))
}

pub fn add_embeddings(_: &mut Evaluator, a: &[Value]) -> R {
    let (x, y, binary) = pair(a)?;
    let v: Vec<f64> = x.iter().zip(&y).map(|(p, q)| p + q).collect();
    Ok(same_kind(&v, binary))
}

pub fn subtract_embeddings(_: &mut Evaluator, a: &[Value]) -> R {
    let (x, y, binary) = pair(a)?;
    let v: Vec<f64> = x.iter().zip(&y).map(|(p, q)| p - q).collect();
    Ok(same_kind(&v, binary))
}

/// NormalizeEmbedding ( data {; dimension } ): truncated to `dimension`,
/// then scaled to unit length.
pub fn normalize_embedding(_: &mut Evaluator, a: &[Value]) -> R {
    let (mut v, binary) = vector(&a[0])?;
    if a.len() > 1 && !a[1].is_empty() {
        let d = int(a, 1)?;
        if d < 1 {
            return Err(EvalError::generic("dimension must be positive"));
        }
        v.truncate(d as usize);
    }
    let n = v.iter().map(|p| p * p).sum::<f64>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|p| *p /= n);
    }
    Ok(same_kind(&v, binary))
}

pub fn get_embedding_as_text(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(as_text(&vector(&a[0])?.0)))
}

// ponytail: "embedding.fve" as the default file name is a guess.
pub fn get_embedding_as_file(_: &mut Evaluator, a: &[Value]) -> R {
    let name = match txt(a, 1) {
        n if n.is_empty() => "embedding.fve".to_string(),
        n => n,
    };
    Ok(as_file(&vector(&a[0])?.0, &name))
}

// ---- containers ------------------------------------------------------------------

/// (width, height) from a PNG, GIF, BMP or JPEG header.
fn image_size(d: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?));
    let le16 = |i: usize| Some(u16::from_le_bytes(d.get(i..i + 2)?.try_into().ok()?) as u32);
    let le32 = |i: usize| Some(i32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?));
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if d.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if d.starts_with(b"BM") {
        return Some((le32(18)?.unsigned_abs(), le32(22)?.unsigned_abs()));
    }
    if d.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < d.len() {
            if d[i] != 0xFF {
                return None;
            }
            let marker = d[i + 1];
            let len = be16(i + 2)? as usize;
            // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC)
            if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + len;
        }
    }
    None
}

fn image(a: &[Value]) -> Result<Option<(u32, u32)>, EvalError> {
    match &a[0] {
        Value::Container(c) if !c.data.is_empty() => image_size(&c.data)
            .map(Some)
            .ok_or_else(|| EvalError::generic("not an image")),
        _ => Ok(None),
    }
}

pub fn get_width(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(image(a)?.map_or_else(Value::empty, |(w, _)| Value::int(w as i64)))
}

pub fn get_height(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(image(a)?.map_or_else(Value::empty, |(_, h)| Value::int(h as i64)))
}

/// GetContainerAttribute ( field ; attributeName ) for what an in-memory
/// container knows: filename, fileSize, MD5, width, height.
// ponytail: storage, timestamps, media and signature attributes and "all"
// need container metadata we do not keep; they return empty.
pub fn get_container_attribute(_: &mut Evaluator, a: &[Value]) -> R {
    let Value::Container(c) = &a[0] else {
        return Ok(Value::empty());
    };
    let size = image_size(&c.data);
    Ok(match txt(a, 1).to_ascii_lowercase().as_str() {
        "filename" => Value::text(c.name.clone()),
        "filesize" => Value::int(c.data.len() as i64),
        "md5" => {
            use md5::Digest;
            Value::Text(
                md5::Md5::digest(&c.data)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            )
        }
        "width" => size.map_or_else(Value::empty, |s| Value::int(s.0 as i64)),
        "height" => size.map_or_else(Value::empty, |s| Value::int(s.1 as i64)),
        _ => Value::empty(),
    })
}

/// VerifyContainer: 1 for embedded data, empty for an empty field.
// ponytail: external storage is not modelled, so there is no checksum to
// compare; embedded data always verifies.
pub fn verify_container(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(if a[0].is_empty() {
        Value::empty()
    } else {
        Value::int(1)
    })
}

// ---- paths -------------------------------------------------------------------------

const PREFIXES: [&str; 8] = [
    "filewin:",
    "filemac:",
    "filelinux:",
    "file:",
    "imagewin:",
    "imagemac:",
    "image:",
    "movie:",
];

/// ConvertFromFileMakerPath ( path ; PosixPath | WinPath | URLPath ).
// ponytail: filemac paths become /Volumes/<volume>/...; FileMaker drops the
// volume for the boot disk, which only the host knows.
pub fn convert_from_filemaker_path(_: &mut Evaluator, a: &[Value]) -> R {
    let p = txt(a, 0);
    let lower = p.to_ascii_lowercase();
    let Some(prefix) = PREFIXES.iter().find(|x| lower.starts_with(*x)) else {
        return Ok(Value::empty());
    };
    let rest = &p[prefix.len()..];
    let mac = prefix.ends_with("mac:");
    // /C:/dir/file, //server/share/file or /Volume/dir/file
    let posix = if mac {
        format!("/Volumes{rest}")
    } else {
        rest.to_string()
    };
    Ok(Value::Text(match int(a, 1)? {
        2 => match rest.strip_prefix("//") {
            Some(unc) => format!("\\\\{}", unc.replace('/', "\\")),
            None => rest.trim_start_matches('/').replace('/', "\\"),
        },
        3 => {
            let path = if rest.starts_with("//") {
                rest.to_string()
            } else {
                format!("//{posix}")
            };
            let enc: String = path
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            format!("file:{enc}")
        }
        1 => posix,
        _ => return Err(EvalError::generic("invalid path format")),
    }))
}

/// ConvertToFileMakerPath ( path ; PosixPath | WinPath | URLPath ).
pub fn convert_to_filemaker_path(_: &mut Evaluator, a: &[Value]) -> R {
    let p = txt(a, 0);
    Ok(Value::Text(match int(a, 1)? {
        2 => {
            let p = p.replace('\\', "/");
            match p.strip_prefix("//") {
                Some(unc) => format!("filewin://{unc}"),
                None => format!("filewin:/{}", p.trim_start_matches('/')),
            }
        }
        3 => {
            let body = p.strip_prefix("file://").unwrap_or(&p);
            let b = body.as_bytes();
            let mut bytes = vec![];
            let mut i = 0;
            while i < b.len() {
                let hex = (b[i] == b'%')
                    .then(|| body.get(i + 1..i + 3))
                    .flatten()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(x) => {
                        bytes.push(x);
                        i += 3;
                    }
                    None => {
                        bytes.push(b[i]);
                        i += 1;
                    }
                }
            }
            let path = String::from_utf8_lossy(&bytes);
            format!("file:{path}")
        }
        1 => match p.strip_prefix("/Volumes/") {
            Some(rest) => format!("filemac:/{rest}"),
            None => format!("file:{p}"),
        },
        _ => return Err(EvalError::generic("invalid path format")),
    }))
}

#[cfg(test)]
mod tests {
    use super::image_size;

    #[test]
    fn png_and_gif_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(&[0, 0, 1, 0, 0, 0, 0, 64]);
        assert_eq!(image_size(&png), Some((256, 64)));
        assert_eq!(image_size(b"GIF89a\x0a\x00\x05\x00"), Some((10, 5)));
        assert_eq!(image_size(b"hello"), None);
    }
}
