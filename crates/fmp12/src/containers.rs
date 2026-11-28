//! Container data (FORMAT.md §13, "Containers"). A container field's value
//! in a record is text: either a reference to the table's container store
//! (`"0A\u{E000}"`: the store id in hex, then U+E000) or the path of a
//! file stored by reference (`imagewin:/C:/photo.jpg`, `file:report.pdf`).
//!
//! The store `[T].[31]` keeps each distinct content once:
//! `[T].[31].[5].[id]` holds the streams (`FNAM` name, `PNGf`/`JPEG`/
//! `GIFf`/`FILE`… data, `SIZE`, `DPI_`), their directory (key 2), the MD5
//! of the streams (key 4) and the number of record values using it
//! (key 18); `[T].[31].[1]` is the catalog header (last id, count, id set,
//! MD5 index).
//!
//! A field may store its files outside the database, in a folder next to
//! it ([`Storage`]): the store then keeps a reference (`REMO` and the data
//! stream, [`External`]) and the file lies at
//! `<database folder>/<base directory>/<REMO folder>/<file name>`. Secure
//! storage encrypts the files and their folder names; only open storage
//! can be read and written here.

use std::collections::BTreeMap;

use md5::Digest;

use crate::{Chunk, Error, File, path_int};

/// Marker ending a store reference in a record value.
pub const REF_END: char = '\u{E000}';

/// Streams FileMaker stores next to the data, not the data itself.
const META: [&[u8; 4]; 5] = [b"FNAM", b"SIZE", b"DPI_", b"REMO", b"ENCR"];

/// How a container field stores its data outside the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// Encrypted, in folders named by an encrypted id: `Secure/52/97/
    /// ABAB0061/B4723A60/0D3C544B/E855_130` (`<n>/<28 hex>/<4 hex>_<table>`
    /// with fewer folders).
    Secure { fewer_folders: bool },
    /// The file as it is, in a folder given by a calculation
    /// (`"Table/Field/"` by default).
    Open,
}

/// Where a container stored outside the file lies (FORMAT.md §13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct External {
    pub storage: Storage,
    /// Folder under the base directory (open storage), `Table/Field/`.
    pub dir: String,
    /// File name (open storage), or `_<table>` (secure storage).
    pub file: String,
    pub size: u64,
    /// MD5 of the file (open storage) or its encrypted id (secure).
    pub md5: [u8; 16],
}

impl External {
    /// Path of an open-storage file relative to the base directory.
    pub fn path(&self) -> Option<String> {
        (self.storage == Storage::Open).then(|| format!("{}{}", self.dir, self.file))
    }
}

/// The base directories of external container storage (`[38].[5].[id]`
/// key 16, File › Manage › Containers): (id, path). A relative path is
/// relative to the database's folder (`Files/<file name>/` by default).
pub fn base_directories(file: &File) -> Result<Vec<(u32, String)>, Error> {
    let mut out = Vec::new();
    file.walk(|path, c| {
        if let ([cat, five, id], Chunk::Kv { key: 16, data }) = (path, c)
            && path_int(cat) == Some(38)
            && path_int(five) == Some(5)
            && let Ok(id) = <[u8; 4]>::try_from(*id)
        {
            out.push((u32::from_be_bytes(id), crate::text(data)));
        }
    })?;
    Ok(out)
}

/// One stored container: its name and streams.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stored {
    /// `image` for pictures FileMaker shows, `file` for other files.
    pub kind: String,
    pub name: String,
    /// Streams by tag, `FNAM` included (sorted by tag, as stored).
    pub streams: BTreeMap<[u8; 4], Vec<u8>>,
    /// Number of record values using it (key 18), as read from the file.
    pub uses: u32,
    /// The file of a container stored outside the database, once read
    /// from its folder (or given when it was inserted).
    pub external_data: Option<Vec<u8>>,
}

impl Stored {
    /// A new container from a file's bytes, as Insert File / Insert
    /// Picture or `Base64Decode` store it: PNG, JPEG and GIF data become
    /// pictures (with their pixel size and resolution), anything else a
    /// file.
    pub fn new(name: &str, data: Vec<u8>) -> Stored {
        let mut streams = BTreeMap::new();
        let image = image_tag(&data);
        let kind = if image.is_some() { "image" } else { "file" };
        if let Some(tag) = image {
            if let Some((w, h)) = image_size(&data) {
                let mut s = (w as u16).to_be_bytes().to_vec();
                s.extend((h as u16).to_be_bytes());
                streams.insert(*b"SIZE", s);
            }
            if let Some(dpi) = image_dpi(&data) {
                let mut s = dpi.to_be_bytes().to_vec();
                s.extend(dpi.to_be_bytes());
                streams.insert(*b"DPI_", s);
            }
            streams.insert(tag, data);
        } else {
            streams.insert(*b"FILE", data);
        }
        streams.insert(*b"FNAM", fnam(kind, name));
        Stored {
            kind: kind.into(),
            name: name.into(),
            streams,
            uses: 0,
            external_data: None,
        }
    }

    /// The content: the first stream that is not metadata.
    pub fn data(&self) -> &[u8] {
        if self.streams.contains_key(b"REMO") {
            return self.external_data.as_deref().unwrap_or(&[]);
        }
        self.streams
            .iter()
            .find(|(t, _)| !META.contains(t))
            .map_or(&[], |(_, d)| d.as_slice())
    }

    /// A new container of a field with open external storage: the file
    /// goes to `dir` (under the base directory, `Table/Field/`) as
    /// `file_name`; the store keeps a reference, as FileMaker Pro 26 writes
    /// it: `REMO` = 8 zero bytes then the folder and `00`; the data stream
    /// (the embedded tag with its case swapped: `pngF`, `jpeg`, `gifF`, `file`) = u64 LE size,
    /// the file's MD5, the file name and `00`.
    pub fn new_open(name: &str, file_name: &str, dir: &str, data: Vec<u8>) -> Stored {
        let mut s = Stored::new(name, Vec::new());
        let image = image_tag(&data);
        s.streams.clear();
        s.kind = if image.is_some() { "image" } else { "file" }.into();
        if image.is_some() {
            if let Some((w, h)) = image_size(&data) {
                let mut x = (w as u16).to_be_bytes().to_vec();
                x.extend((h as u16).to_be_bytes());
                s.streams.insert(*b"SIZE", x);
            }
            if let Some(dpi) = image_dpi(&data) {
                let mut x = dpi.to_be_bytes().to_vec();
                x.extend(dpi.to_be_bytes());
                s.streams.insert(*b"DPI_", x);
            }
        }
        let mut remo = vec![0; 8];
        remo.extend(dir.as_bytes());
        remo.push(0);
        s.streams.insert(*b"REMO", remo);
        let mut r = (data.len() as u64).to_le_bytes().to_vec();
        r.extend(md5::Md5::digest(&data));
        r.extend(file_name.as_bytes());
        r.push(0);
        // The tag of the embedded data with its case swapped: pngF, file.
        let tag = image.unwrap_or(*b"FILE").map(|c| c ^ 0x20);
        s.streams.insert(tag, r);
        s.streams.insert(*b"FNAM", fnam(&s.kind, name));
        s.external_data = Some(data);
        s
    }

    /// Where the file lies when it is stored outside the database.
    pub fn external(&self) -> Option<External> {
        let remo = self.streams.get(b"REMO")?;
        let flags = *remo.first()?;
        let storage = if flags & 1 != 0 {
            Storage::Secure {
                fewer_folders: flags & 8 != 0,
            }
        } else {
            Storage::Open
        };
        let z = |b: &[u8]| {
            let end = b.iter().position(|x| *x == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..end]).into_owned()
        };
        let d = self
            .streams
            .iter()
            .find(|(t, _)| !META.contains(t))
            .map(|(_, d)| d)?;
        Some(External {
            storage,
            dir: z(remo.get(8..)?),
            file: z(d.get(24..)?),
            size: u64::from_le_bytes(d.get(..8)?.try_into().ok()?),
            md5: d.get(8..24)?.try_into().ok()?,
        })
    }

    /// Tag of the content stream (`PNGf`, `JPEG`, `FILE`…).
    pub fn tag(&self) -> Option<[u8; 4]> {
        self.streams.keys().find(|t| !META.contains(t)).copied()
    }

    /// Key 4: MD5 over every stream's tag and bytes, in tag order.
    /// For a file stored outside, the reference stream counts as the
    /// embedded stream would (`PNGf` + the file's bytes).
    pub fn hash(&self) -> [u8; 16] {
        let mut streams: BTreeMap<[u8; 4], &[u8]> = self
            .streams
            .iter()
            .map(|(t, d)| (*t, d.as_slice()))
            .collect();
        if let (Some(data), Some(tag)) = (&self.external_data, self.tag()) {
            streams.remove(&tag);
            streams.insert(tag.map(|c| c ^ 0x20), data);
        }
        let mut h = md5::Md5::new();
        for (t, d) in streams {
            h.update(t);
            h.update(d);
        }
        h.finalize().into()
    }

    /// Key 2: u16 stream count, then each tag and u32 length.
    pub fn directory(&self) -> Vec<u8> {
        let mut out = (self.streams.len() as u16).to_be_bytes().to_vec();
        for (t, d) in &self.streams {
            out.extend(t);
            out.extend((d.len() as u32).to_be_bytes());
        }
        out
    }

    /// Size FileMaker reports as "Internal Size": the streams plus 8 bytes each.
    pub fn internal_size(&self) -> usize {
        self.streams.values().map(|d| d.len() + 8).sum()
    }

    /// Resolution from the `DPI_` stream, 72 without one.
    pub fn dpi(&self) -> u16 {
        self.streams
            .get(b"DPI_")
            .and_then(|d| Some(u16::from_be_bytes([*d.first()?, *d.get(1)?])))
            .unwrap_or(72)
    }
}

/// `FNAM`: u32 1, kind (length byte + XOR-ed text), u16 length + XOR-ed name.
fn fnam(kind: &str, name: &str) -> Vec<u8> {
    let (k, n) = (
        crate::save::encode_text(kind),
        crate::save::encode_text(name),
    );
    let mut out = 1u32.to_be_bytes().to_vec();
    out.push(k.len() as u8);
    out.extend(k);
    out.extend((n.len() as u16).to_be_bytes());
    out.extend(n);
    out
}

fn parse_fnam(b: &[u8]) -> Option<(String, String)> {
    let k = *b.get(4)? as usize;
    let kind = crate::text(b.get(5..5 + k)?);
    let n = u16::from_be_bytes([*b.get(5 + k)?, *b.get(6 + k)?]) as usize;
    let name = crate::text(b.get(7 + k..7 + k + n)?);
    Some((kind, name))
}

/// Stream tag of picture data FileMaker shows: PNG, JPEG, GIF.
pub fn image_tag(d: &[u8]) -> Option<[u8; 4]> {
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(*b"PNGf")
    } else if d.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(*b"JPEG")
    } else if d.starts_with(b"GIF8") {
        Some(*b"GIFf")
    } else {
        None
    }
}

/// Pixel size of PNG, GIF and JPEG data.
pub fn image_size(d: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?));
    let le16 = |i: usize| Some(u16::from_le_bytes([*d.get(i)?, *d.get(i + 1)?]) as u32);
    match image_tag(d)? {
        [b'P', ..] => Some((be32(16)?, be32(20)?)),
        [b'G', ..] => Some((le16(6)?, le16(8)?)),
        _ => {
            let mut i = 2;
            while i + 9 < d.len() {
                let marker = *d.get(i + 1)?;
                // SOF0–SOF15, except DHT (C4), JPG (C8) and DAC (CC).
                if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                    return Some((be16(i + 7)?, be16(i + 5)?));
                }
                i += 2 + be16(i + 2)? as usize;
            }
            None
        }
    }
}

/// Resolution of PNG (`pHYs`) and JPEG (JFIF density) data, `None` when
/// the data does not say (FileMaker then writes no `DPI_` stream).
pub fn image_dpi(d: &[u8]) -> Option<u16> {
    if d.starts_with(b"\x89PNG") {
        let at = d.windows(4).position(|w| w == b"pHYs")?;
        let ppm = u32::from_be_bytes(d.get(at + 4..at + 8)?.try_into().ok()?);
        (*d.get(at + 12)? == 1).then(|| (ppm as f64 * 0.0254).round() as u16)
    } else if d.starts_with(&[0xFF, 0xD8]) && d.get(6..11) == Some(b"JFIF\0") {
        let x = u16::from_be_bytes([*d.get(14)?, *d.get(15)?]);
        (*d.get(13)? == 1).then_some(x)
    } else {
        None
    }
}

/// Store id of a record value that refers to the container store.
pub fn ref_id(value: &str) -> Option<u32> {
    let hex = value.strip_suffix(REF_END)?;
    u32::from_str_radix(hex, 16).ok()
}

/// Record value referring to store id `id`.
// ponytail: ids above 255 not observed; assumed to grow in hex digits.
pub fn ref_text(id: u32) -> String {
    format!("{id:02X}{REF_END}")
}

/// Every stored container of table `table`, by store id.
pub fn read(file: &File, table: u32) -> Result<BTreeMap<u32, Stored>, Error> {
    let mut out: BTreeMap<u32, Stored> = BTreeMap::new();
    // Streams over 1000 bytes: [id].[tag] key 1… chunks.
    file.walk(|path, c| {
        let [tp, store, five, id, rest @ ..] = path else {
            return;
        };
        if tp.len() > 2
            || path_int(tp) != Some(table)
            || path_int(store) != Some(31)
            || path_int(five) != Some(5)
        {
            return;
        }
        let Some(id) = path_int(id) else { return };
        let s = out.entry(id).or_default();
        match (rest, c) {
            ([], Chunk::Kv { key: 18, data }) if data.len() == 4 => {
                s.uses = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            }
            ([], Chunk::KvLong { key, data }) => {
                if let Ok(tag) = <[u8; 4]>::try_from(*key) {
                    s.streams.insert(tag, data.to_vec());
                }
            }
            ([tag], Chunk::Kv { key, data }) if *key >= 1 => {
                if let Ok(tag) = <[u8; 4]>::try_from(*tag) {
                    let e = s.streams.entry(tag).or_default();
                    if *key == 1 {
                        e.clear();
                    }
                    e.extend_from_slice(data);
                }
            }
            _ => {}
        }
    })?;
    for s in out.values_mut() {
        if let Some((kind, name)) = s.streams.get(b"FNAM").and_then(|f| parse_fnam(f)) {
            (s.kind, s.name) = (kind, name);
        }
    }
    out.retain(|_, s| !s.streams.is_empty());
    Ok(out)
}

/// File name at the end of a reference path (`imagewin:/C:/a/b.jpg` → `b.jpg`).
pub fn ref_name(path: &str) -> &str {
    path.rsplit(['/', '\\', ':']).next().unwrap_or(path)
}

/// Streams longer than this move to a sub-path `[id].[tag]` in chunks.
const INLINE_MAX: usize = 1000;

/// Rebuilds `file`'s container stores so that they hold what its records
/// refer to: containers of `stores` (table id → store id → container) that
/// the file lacks are added, the use counts (key 18) are recounted from
/// the record values, and containers no record uses any more are removed.
/// Containers already in the file keep their bytes.
pub fn save_containers(
    file: &File,
    stores: &BTreeMap<u32, BTreeMap<u32, Stored>>,
) -> Result<Vec<u8>, Error> {
    use crate::save::{item_raw_key, kv_raw, len_int, path_elem, spelled};
    use crate::write::{self, Entry};
    let tables = crate::schema::tables(file)?;
    let mut entries = write::entries(file)?;
    let mut changed = false;
    for table in &tables {
        let t = table.id;
        let old = read(file, t)?;
        let empty = BTreeMap::new();
        let given = stores.get(&t).unwrap_or(&empty);
        let mut uses: BTreeMap<u32, u32> = BTreeMap::new();
        for values in crate::schema::records(file, table)?.values() {
            for v in values.values() {
                if let Some(id) = ref_id(v) {
                    *uses.entry(id).or_default() += 1;
                }
            }
        }
        let live: BTreeMap<u32, &Stored> = uses
            .keys()
            .filter_map(|id| Some((*id, old.get(id).or_else(|| given.get(id))?)))
            .collect();
        let same = live.len() == old.len()
            && live
                .iter()
                .all(|(id, _)| old.get(id).is_some_and(|o| o.uses == uses[id]));
        if same {
            continue;
        }
        changed = true;
        let tp = path_elem(t);
        let at = |rest: &[u32]| -> Vec<Vec<u8>> {
            let mut p = vec![tp.clone(), vec![31]];
            p.extend(spelled(rest));
            p
        };
        let id_of = |e: &Entry| e.path.get(3).and_then(|x| path_int(x));
        let in_store = |e: &Entry| e.path.first() == Some(&tp) && e.path.get(1) == Some(&vec![31]);
        let mut header_uuid = None;
        let mut counter = 0;
        entries.retain(|e| {
            if !in_store(e) {
                return true;
            }
            let key = item_raw_key(&e.item).and_then(path_int);
            match e.path.get(2).and_then(|x| path_int(x)) {
                // Header: rewritten below (UUID and counter kept).
                Some(1) if e.path.len() == 3 => {
                    let v = crate::save::entry_kv(e).map(|(_, v)| v);
                    match key {
                        Some(216) => header_uuid = v,
                        Some(252) => {
                            counter = v.and_then(|v| crate::save::read_len_int(&v)).unwrap_or(0)
                        }
                        _ => {}
                    }
                    !matches!(key, Some(0 | 4 | 216 | 252))
                }
                // Id set and MD5 index: rewritten below.
                Some(1) => !matches!(e.path.get(3).and_then(|x| path_int(x)), Some(7 | 8)),
                // A container: kept while used, its use count rewritten.
                Some(5) => match id_of(e) {
                    Some(id) if old.contains_key(&id) && live.contains_key(&id) => {
                        !(e.path.len() == 4 && key == Some(18))
                    }
                    Some(_) => false,
                    None => true,
                },
                _ => true,
            }
        });
        for p in [&[][..], &[1], &[1, 6], &[1, 7], &[1, 8], &[5]] {
            let path = at(p);
            if !entries.iter().any(|e| e.path == path && e.item == [0x80]) {
                entries.push(Entry {
                    path,
                    item: vec![0x80],
                });
            }
        }
        let mut add = |path: Vec<Vec<u8>>, item: Vec<u8>| entries.push(Entry { path, item });
        let last = live
            .keys()
            .copied()
            .chain(old.keys().copied())
            .max()
            .unwrap_or(0);
        add(at(&[1]), kv_raw(&[0], &len_int(last as u64)));
        add(at(&[1]), kv_raw(&[4], &(live.len() as u32).to_be_bytes()));
        let uuid = header_uuid.unwrap_or_else(|| crate::save::fresh_uuid(t as u64));
        add(at(&[1]), kv_raw(&[216], &uuid));
        add(at(&[1]), kv_raw(&[252], &len_int(counter + 1)));
        for (id, s) in &live {
            add(at(&[1, 7]), empty_item(&path_elem(*id)));
            let mut hp = at(&[1, 8]);
            hp.push(s.hash().to_vec());
            add(hp.clone(), vec![0x80]);
            add(hp, empty_item(&path_elem(*id)));
            let node = at(&[5, *id]);
            add(node.clone(), kv_raw(&[18], &uses[id].to_be_bytes()));
            if old.contains_key(id) {
                continue;
            }
            add(node.clone(), vec![0x80]);
            add(node.clone(), kv_raw(&[2], &s.directory()));
            add(node.clone(), kv_raw(&[4], &s.hash()));
            add(
                node.clone(),
                kv_raw(&[216], &crate::save::fresh_uuid(*id as u64)),
            );
            for (tag, d) in &s.streams {
                if d.len() <= INLINE_MAX {
                    add(
                        node.clone(),
                        write::encode(&write::Item::KvLong {
                            key: tag.to_vec(),
                            data: d.clone(),
                        }),
                    );
                    continue;
                }
                let mut sp = node.clone();
                sp.push(tag.to_vec());
                add(sp.clone(), vec![0x80]);
                add(sp.clone(), kv_raw(&[0], &(d.len() as u32).to_be_bytes()));
                for (i, c) in d.chunks(INLINE_MAX).enumerate() {
                    let k = path_elem(i as u32 + 1);
                    add(sp.clone(), kv_raw(&k, c));
                }
            }
        }
    }
    if !changed {
        return Ok(file.bytes().to_vec());
    }
    write::sort(&mut entries);
    Ok(write::build(&file.bytes()[..crate::SECTOR], &entries))
}

/// Item of key `key` with an empty value: `00 k`, `08 k k`, `10 k k k`.
fn empty_item(key: &[u8]) -> Vec<u8> {
    let mut out = vec![[0x00, 0x08, 0x10][key.len().clamp(1, 3) - 1]];
    out.extend_from_slice(key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs() {
        assert_eq!(ref_id("0A\u{E000}"), Some(10));
        assert_eq!(ref_text(10), "0A\u{E000}");
        assert_eq!(ref_id("imagewin:/C:/x.jpg"), None);
        assert_eq!(ref_name("imagewin:/C:/x/blue.jpg"), "blue.jpg");
    }

    // red.png (4 × 3) and hello.txt set by Base64Decode in a field with
    // open external storage in FileMaker Pro 26 (Tasks, field ExtOpen).
    #[test]
    fn open_storage_matches_filemaker() {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let red = (0..)
            .step_by(2)
            .take_while(|i| *i < 146)
            .map(|i| u8::from_str_radix(&"89504e470d0a1a0a0000000d49484452000000040000000308020000003b9639910000001049444154789c63f8cfc000470c383900f5310bf5357bfb820000000049454e44ae426082"[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>();
        let s = Stored::new_open("red.png", "red.png", "Tasks/ExtOpen/", red.clone());
        assert_eq!(
            hex(&s.directory()),
            "0004464e414d0000001352454d4f0000001753495a4500000004706e674600000020"
        );
        assert_eq!(hex(&s.hash()), "bdaa7438230af90639571f8b304e0d53");
        assert_eq!(s.data(), red.as_slice());
        let x = s.external().unwrap();
        assert_eq!(x.path().as_deref(), Some("Tasks/ExtOpen/red.png"));
        assert_eq!(x.size, 73);
        let t = Stored::new_open(
            "hello.txt",
            "hello.txt",
            "Tasks/ExtOpen/",
            b"Hello BaseCraft".to_vec(),
        );
        assert_eq!(
            hex(&t.directory()),
            "0003464e414d0000001452454d4f0000001766696c6500000022"
        );
        assert_eq!(hex(&t.hash()), "9adc3a3aa97ed944589b65bab0c5f920");
    }

    #[test]
    fn new_container_matches_filemaker() {
        // hello.txt as FileMaker 26 stored it from Base64Decode.
        let s = Stored::new("hello.txt", b"Hello container".to_vec());
        assert_eq!(s.kind, "file");
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(&s.directory()), "000246494c450000000f464e414d00000014");
        assert_eq!(
            hex(&s.streams[b"FNAM"]),
            "00000001043c33363f0009323f363635742e222e"
        );
        assert_eq!(hex(&s.hash()), "99ff941bb8e4d3f0952310dd9a33f3b5");
        assert_eq!(s.internal_size(), 51);
    }
}
