//! Custom functions (File › Manage › Custom Functions): catalog `[16]`
//! (FORMAT.md §16). Written by [`crate::schema_edit::Editor`].
//!
//! * `[16].[1]`: catalog header (§4): last id, count, name index `[1]`,
//!   display order `[3]`, and one member `[1].[7].[id]` per entry (key 16
//!   name, key 240 = 1 folder, 2 end of folder);
//! * `[16].[5].[id]`: key 16 name, key 32 = `02` + u16 number of
//!   parameters, key 33 = the parameter names joined by `;`, key 36 =
//!   availability (1 all accounts, 0 full access only), `[35]` keys 5 and 6
//!   = the calculation (§11; a parameter is written like a Let variable).

use std::collections::BTreeMap;

use crate::calc::{self, Names};
use crate::{Chunk, Error, File, int_path};

/// What a catalog entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Function,
    /// A folder: the entries after it, up to its [`Kind::FolderEnd`], are in it.
    Folder,
    FolderEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomFunction {
    pub id: u32,
    pub name: String,
    pub kind: Kind,
    pub params: Vec<String>,
    /// The calculation's token stream (key 5).
    pub tokens: Vec<u8>,
    /// Available to accounts with full access only.
    pub full_access_only: bool,
}

impl CustomFunction {
    /// The calculation as text.
    pub fn formula(&self, names: &dyn Names) -> String {
        calc::decompile(&self.tokens, 0, names).unwrap_or_default()
    }
}

/// The custom functions of a file, folders included, in display order.
pub fn read(file: &File) -> Result<Vec<CustomFunction>, Error> {
    let mut items = Vec::new();
    file.walk(|path, c| {
        if let Some(p) = int_path(path).filter(|p| p.first() == Some(&16)) {
            items.push((p, *c));
        }
    })?;
    Ok(collect(items))
}

/// [`read`] over the chunks of catalog `[16]`, with their paths.
pub(crate) fn collect<'a>(
    items: impl IntoIterator<Item = (Vec<u32>, Chunk<'a>)>,
) -> Vec<CustomFunction> {
    let mut by_id: BTreeMap<u32, CustomFunction> = BTreeMap::new();
    let mut order: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
    for (p, c) in items {
        match (p.as_slice(), c) {
            ([16, 1, 3], Chunk::KvLong { key, data }) => {
                if let Some(id) = crate::save::read_len_int(data) {
                    order.insert(key.to_vec(), id as u32);
                }
            }
            ([16, 1, 7, id], Chunk::Kv { key: 240, data }) => {
                let f = by_id.entry(*id).or_default();
                f.kind = match data.last() {
                    Some(1) => Kind::Folder,
                    Some(2) => Kind::FolderEnd,
                    _ => Kind::Function,
                };
            }
            ([16, 5, id], Chunk::Kv { key, data }) => {
                let f = by_id.entry(*id).or_default();
                match key {
                    16 => f.name = crate::text(data),
                    33 => {
                        f.params = crate::text(data)
                            .split(';')
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect()
                    }
                    36 => f.full_access_only = data.first() == Some(&0),
                    _ => {}
                }
            }
            ([16, 1, 7, id], Chunk::Kv { key: 16, data }) => {
                let f = by_id.entry(*id).or_default();
                if f.name.is_empty() {
                    f.name = crate::text(data);
                }
            }
            ([16, 5, id, 35], Chunk::Kv { key: 5, data }) => {
                by_id.entry(*id).or_default().tokens = data.to_vec();
            }
            _ => {}
        }
    }
    for (id, f) in by_id.iter_mut() {
        f.id = *id;
    }
    let mut out: Vec<CustomFunction> = order.values().filter_map(|id| by_id.remove(id)).collect();
    // Entries missing from the display order go last.
    out.extend(by_id.into_values().filter(|f| !f.name.is_empty()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::{FileNames, compile_function};

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn names() -> FileNames {
        FileNames {
            custom_functions: vec![
                (1, "Fact".into()),
                (3, "Hello".into()),
                (6, "Tail".into()),
                (8, "Sq".into()),
            ],
            ..Default::default()
        }
    }

    // Compiled forms FileMaker Pro 26 stored for custom functions pasted
    // into Manage Custom Functions (FORMAT.md §16).
    #[test]
    fn function_bodies_match_filemaker() {
        let n = names();
        let p = |s: &str| s.split(';').map(str::to_string).collect::<Vec<_>>();
        let fact = compile_function("If ( n <= 1 ; 1 ; n * Fact ( n - 1 ) )", &p("n"), &n).unwrap();
        assert_eq!(
            hex(&fact.compiled),
            "4b42180000100200010010000000010000000000000000100200010010000000010000000000000000271800001b00011402010126180000100200010010000000010000000000000000"
        );
        let k5 = hex(&fact.tokens);
        assert!(k5.contains("0b01340000"), "{k5}");
        assert!(k5.contains("1b14020101"), "{k5}");
        let tail = compile_function(
            "If ( n <= 0 ; acc ; Tail ( n - 1 ; acc + 1 ) )",
            &p("n;acc"),
            &n,
        )
        .unwrap();
        assert_eq!(
            hex(&tail.compiled),
            "4b421800001002000100100000000000000000000000001800011c0002140201062518000110020001001000000001000000000000000026180000100200010010000000010000000000000000"
        );
        let sq = compile_function("x * x // square", &p("x"), &n).unwrap();
        assert_eq!(hex(&sq.compiled), "27180000180000");
        assert!(hex(&sq.tokens).ends_with("0e13077a292b2f3b283f00"));
    }

    // Set Variable steps pasted into FileMaker Pro 26: a call that gives
    // the calculation's result is 1C, any other 1B.
    #[test]
    fn calls_match_filemaker() {
        let n = names();
        let c = |s: &str| hex(&calc::compile(s, 0, &n).unwrap().compiled);
        let one = "100200010010000000010000000000000000";
        let two = "100200010010000000020000000000000000";
        assert_eq!(c("Hello"), "1c000014020103");
        assert_eq!(c("Fact ( 2 )"), format!("1c000114020101{two}"));
        assert_eq!(c("Hello & 1"), format!("501b000014020103{one}"));
        assert_eq!(
            c("Sq ( Sq ( 2 ) )"),
            format!("1c0001140201081b000114020108{two}")
        );
        assert_eq!(
            c("Case ( 1 ; Hello ; Fact ( 2 ) )"),
            format!("4c0002{one}1c000014020103{one}1c000114020101{two}")
        );
        assert_eq!(
            c("Let ( x = Hello ; Fact ( x ) )"),
            "df00011b0000140201031c000114020101180000"
        );
        assert_eq!(
            c("If ( Hello ; 1 ; Hello )"),
            format!("4b1b000014020103{one}1c000014020103")
        );
        assert_eq!(c("( Hello )"), "1c000014020103");
        assert_eq!(c("GetAsText ( Hello )"), "301b000014020103");
        assert_eq!(c("not Hello"), "1e1b000014020103");
        assert_eq!(c("Hello and Hello"), "1f1b0000140201031b000014020103");
        // Key 5 and the decompiler.
        let k5 = calc::compile("Fact ( 5 )", 0, &n).unwrap().tokens;
        assert!(hex(&k5).starts_with("1b14020101"));
        assert_eq!(calc::decompile(&k5, 0, &n).unwrap(), "Fact ( 5 )");
        assert_eq!(
            calc::compiled(&k5).unwrap(),
            calc::compile("Fact ( 5 )", 0, &n).unwrap().compiled
        );
    }
}
