//! Reader for the FileMaker `.fmp12` (HBAM7) file format.
//!
//! Layout: a 4096-byte header sector followed by 4096-byte body sectors that
//! form a B-tree. Leaf sectors hold a byte-code stream ([`chunk`]) that builds
//! a hierarchical key/value store addressed by paths. See `docs/FORMAT.md`.

pub mod calc;
pub mod calc_compile;
mod calc_tables;
pub mod chunk;
pub mod graph;
pub mod layouts;
pub mod pb;
pub mod save;
pub mod schema;
pub mod scripts;
pub mod scripts_save;
pub mod scsu;
pub mod security;
pub mod theme;
pub mod valuelists;
pub mod write;

pub use chunk::{Chunk, path_int};

pub const SECTOR: usize = 4096;
const HEADER_LEN: usize = 20;
const MAGIC: [u8; 15] = [0, 1, 0, 0, 0, 2, 0, 1, 0, 5, 0, 2, 0, 2, 0xC0];
/// Most byte strings are obfuscated by XOR with this mask.
pub const XOR: u8 = 0x5A;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotFmp12,
    Encrypted,
    BadSize,
    Truncated(usize),
    UnknownOpcode { op: u8, at: usize },
    Sector { id: u32, source: Box<Error> },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NotFmp12 => f.write_str("not an .fmp12 file"),
            Error::Encrypted => f.write_str("file uses encryption at rest (HBAMe)"),
            Error::BadSize => f.write_str("file size does not match sector count"),
            Error::Truncated(at) => write!(f, "chunk truncated at payload offset {at}"),
            Error::UnknownOpcode { op, at } => {
                write!(f, "unknown opcode 0x{op:02X} at offset {at}")
            }
            Error::Sector { id, source } => write!(f, "sector {id}: {source}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Sector<'a> {
    pub id: u32,
    pub deleted: bool,
    pub level: u16,
    pub prev: u32,
    pub next: u32,
    pub payload: &'a [u8],
}

pub struct File {
    bytes: Vec<u8>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl File {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        Self::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() < 2 * SECTOR || bytes[..15] != MAGIC {
            return Err(Error::NotFmp12);
        }
        match &bytes[15..20] {
            b"HBAM7" => {}
            b"HBAMe" => return Err(Error::Encrypted),
            _ => return Err(Error::NotFmp12),
        }
        let f = File { bytes };
        // Sector 1's "next" field holds the number of body sectors.
        if !f.bytes.len().is_multiple_of(SECTOR)
            || (f.sector(1).next as usize + 1) * SECTOR != f.bytes.len()
        {
            return Err(Error::BadSize);
        }
        Ok(f)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// File offset of a slice borrowed from this file (e.g. a chunk value).
    pub fn offset_of(&self, part: &[u8]) -> usize {
        part.as_ptr() as usize - self.bytes.as_ptr() as usize
    }

    /// Name of the creating application, e.g. "Pro 12.0".
    pub fn creator(&self) -> String {
        let n = self.bytes[541] as usize;
        String::from_utf8_lossy(&self.bytes[542..542 + n]).into_owned()
    }

    pub fn sector_count(&self) -> u32 {
        (self.bytes.len() / SECTOR) as u32 - 1
    }

    /// Body sector `id` (1-based; sector 0 is the file header).
    pub fn sector(&self, id: u32) -> Sector<'_> {
        let s = &self.bytes[id as usize * SECTOR..][..SECTOR];
        Sector {
            id,
            deleted: s[0] != 0,
            level: u16::from_be_bytes([s[12], s[13]]),
            prev: be32(&s[4..]),
            next: be32(&s[8..]),
            payload: &s[HEADER_LEN..],
        }
    }

    /// Leaf sectors in logical order, following the linked list.
    pub fn leaves(&self) -> Vec<Sector<'_>> {
        let n = self.sector_count();
        let first = (2..=n)
            .map(|i| self.sector(i))
            .find(|s| s.level == 0 && s.prev == 0)
            .map_or(2, |s| s.id);
        let mut seen = vec![false; n as usize + 1];
        let mut out = Vec::new();
        let mut id = first;
        while id != 0 && id <= n && !std::mem::replace(&mut seen[id as usize], true) {
            let s = self.sector(id);
            out.push(s);
            id = s.next;
        }
        out
    }

    /// Calls `f(path, chunk)` for every non-path chunk of every leaf.
    pub fn walk<'a>(&'a self, mut f: impl FnMut(&[&'a [u8]], &Chunk<'a>)) -> Result<(), Error> {
        let mut path: Vec<&[u8]> = Vec::new();
        for s in self.leaves() {
            let chunks = chunk::parse(s.payload).map_err(|e| Error::Sector {
                id: s.id,
                source: Box::new(e),
            })?;
            // Each sector restarts its path from scratch.
            path.clear();
            for c in &chunks {
                match c {
                    Chunk::Push(p) => path.push(p),
                    Chunk::Pop => {
                        path.pop();
                    }
                    _ => f(&path, c),
                }
            }
        }
        Ok(())
    }
}

/// XOR-decodes then SCSU-decodes a FileMaker string.
pub fn text(data: &[u8]) -> String {
    let plain: Vec<u8> = data.iter().map(|b| b ^ XOR).collect();
    scsu::decode(&plain)
}

/// Path as integers, `None` for any non-integer element.
pub fn int_path(path: &[&[u8]]) -> Option<Vec<u32>> {
    path.iter().map(|p| path_int(p)).collect()
}
