//! Scripts (`[17]`): catalog entries and their step lists.

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, text};

/// Size of one step record in key 4 of a script node. The last record of
/// the list has its trailing zero bytes trimmed.
const STEP_LEN: usize = 28;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Step {
    /// Script step type id (e.g. 39 = Sort Records).
    pub id: u16,
    pub enabled: bool,
    /// Step option bits as shown in FileMaker's XML `<Options>`.
    pub options: u32,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub id: u32,
    pub name: String,
    /// Raw value of key 240 (XML `<Options>` text: 2 = separator, 8 = script, 9 = folder…).
    pub options: u64,
    pub steps: Vec<Step>,
}

impl Script {
    pub fn is_folder(&self) -> bool {
        self.options & 1 != 0
    }
    pub fn is_separator(&self) -> bool {
        self.options == 2
    }
}

fn parse_steps(b: &[u8]) -> Vec<Step> {
    b.chunks(STEP_LEN)
        .map(|c| {
            let mut r = c.to_vec();
            r.resize(STEP_LEN, 0);
            Step {
                id: u16::from_be_bytes([r[20], r[21]]),
                // ponytail: 0xBEBE observed on every enabled step; disabled marker unconfirmed.
                enabled: r[4] == 0xBE,
                options: u32::from_be_bytes([r[24], r[25], r[26], r[27]]),
                raw: c.to_vec(),
            }
        })
        .collect()
}

pub fn scripts(file: &File) -> Result<Vec<Script>, Error> {
    let mut out: BTreeMap<u32, Script> = BTreeMap::new();
    let mut step_bytes: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        if let ([17, 5, id], Chunk::Kv { key, data }) = (p.as_slice(), c) {
            let s = out.entry(*id).or_insert_with(|| Script {
                id: *id,
                ..Default::default()
            });
            match key {
                16 => s.name = text(data),
                240 => s.options = data.iter().fold(0, |a, b| (a << 8) | *b as u64),
                4 => step_bytes.entry(*id).or_default().extend_from_slice(data),
                _ => {}
            }
        }
    })?;
    for (id, b) in step_bytes {
        if let Some(s) = out.get_mut(&id) {
            s.steps = parse_steps(&b);
        }
    }
    Ok(out.into_values().collect())
}
