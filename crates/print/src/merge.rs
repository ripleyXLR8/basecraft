//! Joins PDF documents page by page (the PDF steps: Open / Append / Close
//! PDF). Objects are found by scanning for `n g obj … endobj`, so files
//! with a classic layout work (BaseCraft's own, most writers); ponytail:
//! compressed object streams (`/ObjStm`, PDF 1.5) are refused, a full
//! parser would lift that.

use std::collections::BTreeMap;

type Objects = BTreeMap<u32, Vec<u8>>;

/// The objects of a PDF: number → bytes between `obj` and `endobj`.
fn objects(pdf: &[u8]) -> Result<Objects, String> {
    let mut out = Objects::new();
    let mut i = 0;
    while let Some(at) = find(pdf, b" obj", i) {
        // The object number and generation before " obj".
        let head = &pdf[pdf[..at]
            .iter()
            .rposition(|c| *c == b'\n' || *c == b'\r')
            .map_or(0, |x| x + 1)..at];
        let mut it = std::str::from_utf8(head).unwrap_or("").split_whitespace();
        let num = it.next().and_then(|n| n.parse::<u32>().ok());
        let body_start = at + 4;
        let Some(end) = object_end(pdf, body_start) else {
            return Err("unterminated object".into());
        };
        if let Some(n) = num {
            out.insert(n, pdf[body_start..end].to_vec());
        }
        i = end + 6;
    }
    if out.values().any(|o| find(o, b"/ObjStm", 0).is_some()) {
        return Err("compressed object streams are not supported".into());
    }
    if out.is_empty() {
        return Err("not a PDF".into());
    }
    Ok(out)
}

/// End (index of `endobj`) of the object starting at `from`, skipping
/// stream data.
fn object_end(pdf: &[u8], from: usize) -> Option<usize> {
    let e = find(pdf, b"endobj", from)?;
    match find(pdf[..e].as_ref(), b"stream", from) {
        Some(s) if pdf.get(s.wrapping_sub(3)..s) != Some(b"end") => {
            let es = find(pdf, b"endstream", s + 6)?;
            find(pdf, b"endobj", es)
        }
        _ => Some(e),
    }
}

fn find(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    h.get(from..)?
        .windows(n.len())
        .position(|w| w == n)
        .map(|p| p + from)
}

/// `/Key value` of a dictionary as text (value up to the next key or `>>`).
fn entry(dict: &str, key: &str) -> Option<String> {
    let i = dict.find(&format!("/{key}"))? + key.len() + 1;
    let rest = &dict[i..];
    let rest = rest.trim_start();
    if let Some(r) = rest.strip_prefix('[') {
        return Some(format!("[{}]", &r[..r.find(']')?]));
    }
    if rest.starts_with("<<") {
        let mut depth = 0;
        let b = rest.as_bytes();
        for k in 0..b.len().saturating_sub(1) {
            if &b[k..k + 2] == b"<<" {
                depth += 1;
            } else if &b[k..k + 2] == b">>" {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..k + 2].to_string());
                }
            }
        }
        return None;
    }
    let end = rest.find(['/', '>']).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

/// `n g R` references in text.
fn refs(text: &str) -> Vec<u32> {
    let w: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || "[]<>/".contains(c))
        .filter(|x| !x.is_empty())
        .collect();
    (2..w.len())
        .filter(|&k| w[k] == "R")
        .filter_map(|k| {
            w[k - 1].parse::<u32>().ok()?;
            w[k - 2].parse().ok()
        })
        .collect()
}

/// The dictionary part of an object (before any stream).
fn dict_of(o: &[u8]) -> String {
    let end = find(o, b"stream", 0).unwrap_or(o.len());
    String::from_utf8_lossy(&o[..end]).into_owned()
}

/// The page objects of a PDF, in order, with inherited attributes.
fn page_list(objs: &Objects) -> Result<Vec<(u32, String)>, String> {
    let catalog = objs
        .iter()
        .find(|(_, o)| dict_of(o).contains("/Catalog"))
        .ok_or("no catalog")?;
    let pages = entry(&dict_of(catalog.1), "Pages")
        .and_then(|v| refs(&v).first().copied())
        .ok_or("no page tree")?;
    let mut out = Vec::new();
    walk(objs, pages, &mut Vec::new(), &mut out, 0)?;
    Ok(out)
}

const INHERITED: [&str; 4] = ["Resources", "MediaBox", "CropBox", "Rotate"];

fn walk(
    objs: &Objects,
    n: u32,
    inherited: &mut Vec<(&'static str, String)>,
    out: &mut Vec<(u32, String)>,
    depth: usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err("page tree too deep".into());
    }
    let d = dict_of(objs.get(&n).ok_or("missing page object")?);
    if d.contains("/Type /Pages") || d.contains("/Type/Pages") {
        let saved = inherited.len();
        for k in INHERITED {
            if let Some(v) = entry(&d, k) {
                inherited.push((k, v));
            }
        }
        for kid in refs(&entry(&d, "Kids").unwrap_or_default()) {
            walk(objs, kid, inherited, out, depth + 1)?;
        }
        inherited.truncate(saved);
        return Ok(());
    }
    // A page: add what it inherits and does not set itself.
    let mut extra = String::new();
    for k in INHERITED {
        if entry(&d, k).is_none()
            && let Some((_, v)) = inherited.iter().rev().find(|(x, _)| *x == k)
        {
            extra.push_str(&format!(" /{k} {v}"));
        }
    }
    out.push((n, extra));
    Ok(())
}

/// The number of pages of a PDF (an error when it cannot be read).
pub fn pages(pdf: &[u8]) -> Result<usize, String> {
    Ok(page_list(&objects(pdf)?)?.len())
}

/// Rewrites `n g R` references in a dictionary with `map`.
fn renumber(dict: &str, map: &BTreeMap<u32, u32>) -> String {
    let toks: Vec<&str> = dict
        .split_inclusive(|c: char| c.is_whitespace() || "[]<>/".contains(c))
        .collect();
    let words: Vec<&str> = toks
        .iter()
        .map(|t| t.trim_end_matches(|c: char| c.is_whitespace() || "[]<>/".contains(c)))
        .collect();
    let mut out = String::new();
    for k in 0..toks.len() {
        let is_ref_num = k + 2 < toks.len()
            && words[k + 2] == "R"
            && words[k + 1].parse::<u32>().is_ok()
            && words[k].parse::<u32>().is_ok();
        match (
            is_ref_num,
            words[k].parse::<u32>().ok().and_then(|n| map.get(&n)),
        ) {
            (true, Some(m)) => {
                out.push_str(&m.to_string());
                out.push_str(&toks[k][words[k].len()..]);
            }
            _ => out.push_str(toks[k]),
        }
    }
    out
}

/// One PDF made of every page of `docs`, in order. No documents: an empty
/// PDF (no pages), as Close PDF after Create PDF saves.
pub fn merge(docs: &[&[u8]]) -> Result<Vec<u8>, String> {
    // Objects of the result: 1 catalog, 2 page tree, then each document's.
    let mut body: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let mut kids = Vec::new();
    for pdf in docs {
        let objs = objects(pdf)?;
        let pages = page_list(&objs)?;
        // Everything the pages reach, except their parents.
        let mut keep: Vec<u32> = Vec::new();
        let mut todo: Vec<u32> = pages.iter().map(|(n, _)| *n).collect();
        let extra: String = pages.iter().map(|(_, e)| e.as_str()).collect();
        todo.extend(refs(&extra));
        while let Some(n) = todo.pop() {
            if keep.contains(&n) {
                continue;
            }
            let Some(o) = objs.get(&n) else { continue };
            keep.push(n);
            let d = dict_of(o);
            let d = match entry(&d, "Parent") {
                Some(p) => d.replace(&format!("/Parent {p}"), ""),
                None => d,
            };
            todo.extend(refs(&d));
        }
        keep.sort_unstable();
        let base = body.len() as u32;
        let map: BTreeMap<u32, u32> = keep
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, base + 1 + i as u32))
            .collect();
        for n in &keep {
            let o = &objs[n];
            let split = find(o, b"stream", 0).unwrap_or(o.len());
            let mut d = String::from_utf8_lossy(&o[..split]).into_owned();
            if let Some((_, extra)) = pages.iter().find(|(p, _)| p == n)
                && let Some(i) = d.rfind(">>")
            {
                d.insert_str(i, extra);
            }
            let mut b = renumber(&d, &map).into_bytes();
            // Pages point at the new tree (object 2, never renumbered away).
            if pages.iter().any(|(p, _)| p == n) {
                let s = String::from_utf8_lossy(&b).into_owned();
                let fixed = match entry(&s, "Parent") {
                    Some(p) => s.replace(&format!("/Parent {p}"), "/Parent 2 0 R"),
                    None => s,
                };
                b = fixed.into_bytes();
            }
            b.extend_from_slice(&o[split..]);
            body.push(b);
        }
        kids.extend(pages.iter().map(|(n, _)| map[n]));
    }
    body[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    let k: Vec<String> = kids.iter().map(|n| format!("{n} 0 R")).collect();
    body[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        k.join(" "),
        kids.len()
    )
    .into_bytes();
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in body.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o.strip_prefix(b"\n").unwrap_or(o));
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", body.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            body.len() + 1
        )
        .as_bytes(),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::Pdf;

    fn doc(n: usize) -> Vec<u8> {
        let mut p = Pdf::new();
        for i in 0..n {
            p.page(612.0, 792.0);
            p.text(
                10.0,
                20.0,
                12.0,
                false,
                [0.0; 4],
                &format!("page {i}"),
                (0.0, 0.0, 600.0, 700.0),
            );
        }
        p.finish()
    }

    #[test]
    fn joins_pages() {
        let (a, b) = (doc(2), doc(3));
        assert_eq!(pages(&a), Ok(2));
        let m = merge(&[&a, &b]).unwrap();
        assert_eq!(pages(&m), Ok(5));
        // A merged file merges again, and its pages keep their fonts.
        let m2 = merge(&[&m, &a]).unwrap();
        assert_eq!(pages(&m2), Ok(7));
        let objs = objects(&m2).unwrap();
        let first = page_list(&objs).unwrap()[0].0;
        let d = dict_of(&objs[&first]);
        assert!(d.contains("/Parent 2 0 R"), "{d}");
        for r in refs(&entry(&d, "Resources").unwrap()) {
            assert!(objs.contains_key(&r));
        }
        assert_eq!(pages(&merge(&[]).unwrap()), Ok(0));
        assert!(pages(b"not a pdf").is_err());
    }

    #[test]
    fn inherited_attributes_reach_the_pages() {
        let src = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 200] /Resources << /Font << /F1 4 0 R >> >> >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\n4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n";
        let m = merge(&[src]).unwrap();
        let objs = objects(&m).unwrap();
        let (p, _) = page_list(&objs).unwrap()[0];
        let d = dict_of(&objs[&p]);
        assert!(d.contains("/MediaBox [0 0 100 200]"), "{d}");
        let font = refs(&entry(&d, "Resources").unwrap())[0];
        assert!(dict_of(&objs[&font]).contains("Helvetica"));
    }
}
