//! Minimal Excel `.xlsx` (OOXML) writer and reader: one worksheet, inline
//! strings on write, shared and inline strings on read. The zip container
//! is written by hand; deflate comes from `miniz_oxide`, XML parsing on
//! read from `roxmltree`.

use std::collections::BTreeMap;

use calc::Value;

pub enum Cell {
    Text(String),
    /// A number, as text (`12.5`).
    Number(String),
    /// Excel serial date (days since 1899-12-30) and the cell style:
    /// 1 date, 2 time, 3 timestamp.
    Serial(f64, u8),
}

fn excel_epoch() -> i64 {
    calc::value::day_number(1899, 12, 30)
}

impl Cell {
    pub fn of(v: &Value) -> Cell {
        let secs = |t: &calc::num::Num| calc::num::to_f64(t) / 86400.0;
        match v {
            Value::Number(_) => Cell::Number(v.to_text()),
            Value::Date(d) => Cell::Serial((d - excel_epoch()) as f64, 1),
            Value::Time(t) => Cell::Serial(secs(t), 2),
            Value::Timestamp(ts) => {
                let day = calc::value::ts_date(ts) - excel_epoch();
                Cell::Serial(day as f64 + secs(&calc::value::ts_time(ts)), 3)
            }
            _ => Cell::Text(v.to_text()),
        }
    }
}

/// Text of an Excel serial for a Date, Time or Timestamp field, in US formats.
pub fn serial_to_us(serial: f64, dt: fmp12::schema::DataType) -> Option<String> {
    use fmp12::schema::DataType;
    let day = serial.floor() as i64 + excel_epoch();
    let secs = ((serial - serial.floor()) * 86400.0).round() as i64;
    let (y, m, d) = calc::value::civil(day);
    let time = format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    Some(match dt {
        DataType::Date => format!("{m}/{d}/{y}"),
        DataType::Time => time,
        DataType::Timestamp => format!("{m}/{d}/{y} {time}"),
        _ => return None,
    })
}

fn col_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn col_index(r: &str) -> Option<usize> {
    let letters: String = r.chars().take_while(char::is_ascii_alphabetic).collect();
    if letters.is_empty() {
        return None;
    }
    Some(
        letters.bytes().fold(0, |a, b| {
            a * 26 + (b.to_ascii_uppercase() - b'A') as usize + 1
        }) - 1,
    )
}

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

pub fn write(rows: &[Vec<Cell>]) -> Vec<u8> {
    let esc = crate::export::xml_escape;
    let mut sheet = format!("{HEAD}<worksheet xmlns=\"{MAIN_NS}\"><sheetData>");
    for (ri, row) in rows.iter().enumerate() {
        sheet.push_str(&format!("<row r=\"{}\">", ri + 1));
        for (ci, c) in row.iter().enumerate() {
            let r = format!("{}{}", col_name(ci), ri + 1);
            match c {
                Cell::Text(s) if s.is_empty() => {}
                Cell::Text(s) => sheet.push_str(&format!(
                    "<c r=\"{r}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    esc(&s.replace('\r', "\n"))
                )),
                Cell::Number(n) => sheet.push_str(&format!("<c r=\"{r}\"><v>{n}</v></c>")),
                Cell::Serial(v, style) => {
                    sheet.push_str(&format!("<c r=\"{r}\" s=\"{style}\"><v>{v}</v></c>"))
                }
            }
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    let xf = |fmt: u32| {
        format!(
            "<xf numFmtId=\"{fmt}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>"
        )
    };
    let styles = format!(
        "{HEAD}<styleSheet xmlns=\"{MAIN_NS}\"><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"4\">{}{}{}{}</cellXfs></styleSheet>",
        xf(0),
        xf(14),
        xf(21),
        xf(22)
    );
    let files: [(&str, String); 6] = [
        (
            "[Content_Types].xml",
            format!(
                "{HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>"
            ),
        ),
        (
            "_rels/.rels",
            format!(
                "{HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{REL_NS}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                "{HEAD}<workbook xmlns=\"{MAIN_NS}\" xmlns:r=\"{REL_NS}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                "{HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{REL_NS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"{REL_NS}/styles\" Target=\"styles.xml\"/></Relationships>"
            ),
        ),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/styles.xml", styles),
    ];
    zip(&files
        .iter()
        .map(|(n, s)| (*n, s.as_bytes()))
        .collect::<Vec<_>>())
}

/// Rows of the first worksheet, as text (numbers as Excel stores them).
pub fn read(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let files = unzip(bytes)?;
    let xml = |name: &str| -> Result<String, String> {
        let b = files.get(name).ok_or_else(|| format!("xlsx: no {name}"))?;
        String::from_utf8(b.clone()).map_err(|e| format!("xlsx: {name}: {e}"))
    };
    fn parse(text: &str) -> Result<roxmltree::Document<'_>, String> {
        roxmltree::Document::parse(text).map_err(|e| format!("xlsx: bad XML: {e}"))
    }
    // First sheet of the workbook, through its relationship.
    let sheet_path = (|| -> Option<String> {
        let wb = xml("xl/workbook.xml").ok()?;
        let rels = xml("xl/_rels/workbook.xml.rels").ok()?;
        let wb = roxmltree::Document::parse(&wb).ok()?;
        let rels = roxmltree::Document::parse(&rels).ok()?;
        let sheet = wb
            .descendants()
            .find(|n| n.has_tag_name((MAIN_NS, "sheet")))?;
        let id = sheet.attribute((REL_NS, "id"))?;
        let target = rels
            .descendants()
            .find(|n| n.attribute("Id") == Some(id))?
            .attribute("Target")?;
        Some(match target.strip_prefix('/') {
            Some(abs) => abs.to_string(),
            None => format!("xl/{target}"),
        })
    })()
    .unwrap_or_else(|| "xl/worksheets/sheet1.xml".into());
    let text_of = |n: roxmltree::Node| -> String {
        n.descendants()
            .filter(|t| t.tag_name().name() == "t")
            .filter(|t| !t.ancestors().any(|a| a.tag_name().name() == "rPh"))
            .filter_map(|t| t.text())
            .collect()
    };
    let shared: Vec<String> = match xml("xl/sharedStrings.xml") {
        Ok(s) => parse(&s)?
            .root_element()
            .children()
            .filter(|n| n.tag_name().name() == "si")
            .map(text_of)
            .collect(),
        Err(_) => Vec::new(),
    };
    let sheet = xml(&sheet_path)?;
    let doc = parse(&sheet)?;
    let mut rows = Vec::new();
    for row in doc.descendants().filter(|n| n.tag_name().name() == "row") {
        let mut cells: Vec<String> = Vec::new();
        for c in row.children().filter(|n| n.tag_name().name() == "c") {
            let i = c.attribute("r").and_then(col_index).unwrap_or(cells.len());
            let v = || {
                c.children()
                    .find(|n| n.tag_name().name() == "v")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_string()
            };
            let value = match c.attribute("t") {
                Some("s") => v()
                    .parse::<usize>()
                    .ok()
                    .and_then(|k| shared.get(k).cloned())
                    .unwrap_or_default(),
                Some("inlineStr") => c
                    .children()
                    .find(|n| n.tag_name().name() == "is")
                    .map(text_of)
                    .unwrap_or_default(),
                _ => v(),
            };
            if cells.len() <= i {
                cells.resize(i + 1, String::new());
            }
            cells[i] = value.replace("\r\n", "\r").replace('\n', "\r");
        }
        rows.push(cells);
    }
    Ok(rows)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Zip archive with deflated entries (dates set to 1980-01-01).
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let u16le = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let u32le = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let packed = miniz_oxide::deflate::compress_to_vec(data, 6);
        let crc = crc32(data);
        let offset = out.len() as u32;
        // Fields shared by the local and central headers, from "version needed".
        let mut common = Vec::new();
        u16le(&mut common, 20);
        u16le(&mut common, 0); // flags
        u16le(&mut common, 8); // deflate
        u16le(&mut common, 0); // time
        u16le(&mut common, 0x21); // date: 1980-01-01
        u32le(&mut common, crc);
        u32le(&mut common, packed.len() as u32);
        u32le(&mut common, data.len() as u32);
        u16le(&mut common, name.len() as u16);
        u16le(&mut common, 0); // extra length
        u32le(&mut out, 0x0403_4B50);
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        u32le(&mut central, 0x0201_4B50);
        u16le(&mut central, 20); // version made by
        central.extend_from_slice(&common);
        u16le(&mut central, 0); // comment length
        u16le(&mut central, 0); // disk
        u16le(&mut central, 0); // internal attributes
        u32le(&mut central, 0); // external attributes
        u32le(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    u32le(&mut out, 0x0605_4B50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, files.len() as u16);
    u16le(&mut out, files.len() as u16);
    u32le(&mut out, central.len() as u32);
    u32le(&mut out, cd_offset);
    u16le(&mut out, 0);
    out
}

/// Entries of a zip archive (stored or deflated), by name.
fn unzip(b: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let bad = || "xlsx: not a zip archive".to_string();
    let u16at = |i: usize| -> Result<usize, String> {
        Ok(u16::from_le_bytes(
            b.get(i..i + 2)
                .ok_or_else(bad)?
                .try_into()
                .map_err(|_| bad())?,
        ) as usize)
    };
    let u32at = |i: usize| -> Result<usize, String> {
        Ok(u32::from_le_bytes(
            b.get(i..i + 4)
                .ok_or_else(bad)?
                .try_into()
                .map_err(|_| bad())?,
        ) as usize)
    };
    let eocd = (0..b.len().saturating_sub(21))
        .rev()
        .find(|&i| b[i..].starts_with(&[0x50, 0x4B, 0x05, 0x06]))
        .ok_or_else(bad)?;
    let count = u16at(eocd + 10)?;
    let mut p = u32at(eocd + 16)?;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        if u32at(p)? != 0x0201_4B50 {
            return Err(bad());
        }
        let method = u16at(p + 10)?;
        let size = u32at(p + 20)?;
        let (n, extra, comment) = (u16at(p + 28)?, u16at(p + 30)?, u16at(p + 32)?);
        let local = u32at(p + 42)?;
        let name = String::from_utf8_lossy(b.get(p + 46..p + 46 + n).ok_or_else(bad)?).into_owned();
        p += 46 + n + extra + comment;
        let start = local + 30 + u16at(local + 26)? + u16at(local + 28)?;
        let data = b.get(start..start + size).ok_or_else(bad)?;
        let data = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec(data)
                .map_err(|e| format!("xlsx: {name}: {e:?}"))?,
            m => {
                return Err(format!(
                    "xlsx: {name}: compression method {m} not supported"
                ));
            }
        };
        out.insert(name, data);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let rows = vec![
            vec![Cell::Text("Name".into()), Cell::Text("Born".into())],
            vec![
                Cell::Text("Ada & <co>\rnext".into()),
                Cell::of(&Value::Date(calc::value::day_number(1815, 12, 10))),
            ],
            vec![Cell::Text(String::new()), Cell::Number("12.5".into())],
        ];
        let bytes = write(&rows);
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let back = read(&bytes).unwrap();
        assert_eq!(back[0], ["Name", "Born"]);
        assert_eq!(back[1][0], "Ada & <co>\rnext");
        assert_eq!(
            serial_to_us(back[1][1].parse().unwrap(), fmp12::schema::DataType::Date).unwrap(),
            "12/10/1815"
        );
        assert_eq!(back[2], ["", "12.5"]);
        assert_eq!(col_name(27), "AB");
        assert_eq!(col_index("AB3"), Some(27));
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
