//! Schema editing ("Manage Database"): tables, their occurrences and
//! layouts, fields and field options, relationships and value lists,
//! written into the file's catalogs (FORMAT.md §14).
//!
//! An [`Editor`] works on the logical entries of a file and rebuilds it with
//! [`Editor::finish`]. Every catalog (`[3].[16]` tables, `[3].[17]` table
//! occurrences, `[4]` layouts, `[T].[3]` fields) keeps a header node `[1]`
//! with the last id (key 0), a name index `[1].[1]` (collation key → id), the
//! display order `[1].[3]` (position → id), the count (key 4) and a change
//! counter (key 252), and one node per object in `[5]`.

use std::collections::{BTreeMap, BTreeSet};

use crate::calc::{self, FileNames};
use crate::collate;
use crate::graph::{JoinOp, Predicate, Relationship, SortKey, TableOccurrence};
use crate::save::{encode_text, entry_kv, fresh_uuid, len_int, path_elem, read_len_int};
use crate::schema::{self, AutoStamp, DataType, Field, FieldKind, Indexing, SummaryOp};
pub use crate::schema::{Lookup, NoMatch};
use crate::valuelists::{Source, ValueList};
use crate::write::{self, Entry, Item};
use crate::{Chunk, Error, File, int_path, path_int};

#[derive(Debug)]
pub enum EditError {
    File(Error),
    /// No table, field… with this name or id.
    NotFound(String),
    /// The name is already used.
    Exists(String),
    Invalid(String),
    Calc(calc::CompileError),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::File(e) => write!(f, "{e}"),
            EditError::NotFound(s) => write!(f, "not found: {s}"),
            EditError::Exists(s) => write!(f, "name already used: {s}"),
            EditError::Invalid(s) => write!(f, "{s}"),
            EditError::Calc(e) => write!(f, "calculation: {e}"),
        }
    }
}

impl std::error::Error for EditError {}

impl From<Error> for EditError {
    fn from(e: Error) -> Self {
        EditError::File(e)
    }
}

impl From<calc::CompileError> for EditError {
    fn from(e: calc::CompileError) -> Self {
        EditError::Calc(e)
    }
}

type Res<T> = Result<T, EditError>;

/// What a field is.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    Normal,
    /// Calculated field: formula, evaluated from the table's occurrence.
    Calculated {
        formula: String,
        stored: bool,
    },
    /// Summary field over another field of the same table.
    Summary {
        op: SummaryOp,
        field: u32,
        /// Running total or count (Total, Count).
        running: bool,
        /// Second field of the same table: the sort field a running
        /// summary restarts on, the weight of an Average, the field a
        /// Fraction of Total is subtotaled by.
        by: Option<u32>,
    },
}

/// Auto-enter options of a normal field.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoEnter {
    pub stamp: Option<AutoStamp>,
    /// Serial number generated on creation: next value.
    pub serial: Option<String>,
    /// Increment of the serial number (0 is written as 1).
    pub serial_increment: u32,
    /// Looked-up value.
    pub lookup: Option<Lookup>,
    pub constant: Option<String>,
    pub calculation: Option<String>,
    /// Auto-enter calculation replaces an existing value.
    pub replace_existing: bool,
    /// Evaluate the calculation even when referenced fields are empty.
    pub always_evaluate: bool,
    pub prohibit_modification: bool,
}

impl Default for AutoEnter {
    fn default() -> Self {
        AutoEnter {
            stamp: None,
            serial: None,
            serial_increment: 1,
            lookup: None,
            constant: None,
            calculation: None,
            replace_existing: false,
            always_evaluate: false,
            prohibit_modification: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationSpec {
    /// Validate always, not only during data entry.
    pub always: bool,
    pub allow_override: bool,
    pub not_empty: bool,
    pub unique: bool,
    pub existing: bool,
    /// Range, as text in the file's formats.
    pub range: Option<(String, String)>,
    pub calculation: Option<String>,
    /// Member of this value list.
    pub value_list: Option<u32>,
    /// Maximum number of characters.
    pub max_chars: Option<u32>,
    /// Custom message shown when validation fails: a calculation
    /// (`"Text"` for plain text).
    pub message: Option<String>,
}

impl Default for ValidationSpec {
    fn default() -> Self {
        ValidationSpec {
            always: false,
            allow_override: true,
            not_empty: false,
            unique: false,
            existing: false,
            range: None,
            calculation: None,
            value_list: None,
            max_chars: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Storage {
    pub global: bool,
    pub repetitions: u16,
    pub indexing: Indexing,
    pub auto_index: bool,
}

impl Default for Storage {
    fn default() -> Self {
        Storage {
            global: false,
            repetitions: 1,
            indexing: Indexing::None,
            auto_index: true,
        }
    }
}

/// A field definition, for [`Editor::add_field`] and [`Editor::set_field`].
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSpec {
    pub name: String,
    pub comment: String,
    pub field_type: FieldType,
    /// Data type (the result type of a calculation).
    pub data_type: DataType,
    pub auto_enter: AutoEnter,
    pub validation: ValidationSpec,
    pub storage: Storage,
}

impl FieldSpec {
    /// A normal field of `data_type` without options.
    pub fn new(name: &str, data_type: DataType) -> Self {
        FieldSpec {
            name: name.to_string(),
            comment: String::new(),
            field_type: FieldType::Normal,
            data_type,
            auto_enter: AutoEnter::default(),
            validation: ValidationSpec::default(),
            storage: Storage::default(),
        }
    }

    /// A calculated field.
    pub fn calculated(name: &str, result: DataType, formula: &str) -> Self {
        FieldSpec {
            field_type: FieldType::Calculated {
                formula: formula.to_string(),
                stored: true,
            },
            ..FieldSpec::new(name, result)
        }
    }

    /// A summary field over field `of`.
    pub fn summary(name: &str, op: SummaryOp, of: u32) -> Self {
        FieldSpec {
            field_type: FieldType::Summary {
                op,
                field: of,
                running: false,
                by: None,
            },
            ..FieldSpec::new(name, DataType::Number)
        }
    }
}

/// Options of one side of a relationship (FileMaker's Edit Relationship
/// dialog): the records of this side.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelSide {
    /// Allow creation of records in this side's table through the relationship.
    pub create: bool,
    /// Delete this side's records when a related record of the other side
    /// is deleted.
    pub delete: bool,
    /// Sort this side's records: (field of this side's table, descending).
    pub sort: Vec<(u32, bool)>,
}

/// A relationship, for [`Editor::add_relationship`] and
/// [`Editor::set_relationship`].
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipSpec {
    /// Table occurrences.
    pub left: u32,
    pub right: u32,
    /// Match fields: left field of `left`, operator, right field of `right`.
    pub predicates: Vec<Predicate>,
    pub left_options: RelSide,
    pub right_options: RelSide,
}

impl RelationshipSpec {
    /// `left::lf = right::rf`, no options.
    pub fn equal(left: u32, lf: u32, right: u32, rf: u32) -> Self {
        RelationshipSpec {
            left,
            right,
            predicates: vec![Predicate {
                op: JoinOp::Equal,
                left_field: lf,
                right_field: rf,
            }],
            left_options: RelSide::default(),
            right_options: RelSide::default(),
        }
    }
}

/// Path element of the relationship catalog `[3].[251]`: FileMaker spells
/// 251 as one byte there (and in an occurrence's links `[TO].[251]`).
const RELS: u8 = 0xFB;

/// `[3].[251].[…]`.
fn rels_path(rest: &[u32]) -> Vec<Vec<u8>> {
    [vec![vec![3], vec![RELS]], sp(rest)].concat()
}

/// `[T].[3].[1].[14].[251].[F]`: what lookup field F of table T copies
/// through (251 spelled as one byte, like [`RELS`]).
fn lookup_deps(t: u32, f: u32) -> Vec<Vec<u8>> {
    [sp(&[t, 3, 1, 14]), vec![vec![RELS]], sp(&[f])].concat()
}

/// An entry keyed by 3 bytes (an occurrence `D0 hi lo`), as FileMaker
/// writes them: ops 11–15 for values of 1, 2, 4, 6 or 8 bytes, else 16.
fn kv3(path: Vec<Vec<u8>>, key: Vec<u8>, data: Vec<u8>) -> Entry {
    let op = match data.len() {
        1 => 0x11,
        2 => 0x12,
        4 => 0x13,
        6 => 0x14,
        8 => 0x15,
        _ => return kvlong(path, key, data),
    };
    Entry {
        path,
        item: [vec![op], key, data].concat(),
    }
}

/// Path of an entry as integers.
fn ipath(e: &Entry) -> Option<Vec<u32>> {
    int_path(&e.path.iter().map(|x| x.as_slice()).collect::<Vec<_>>())
}

fn type_code(d: DataType) -> u8 {
    match d {
        DataType::Text => 1,
        DataType::Number => 2,
        DataType::Date => 3,
        DataType::Time => 4,
        DataType::Timestamp => 5,
        DataType::Container => 6,
        DataType::Unknown(n) => n,
    }
}

fn stamp_code(s: AutoStamp) -> u8 {
    use AutoStamp::*;
    match s {
        CreationDate => 0,
        CreationTime => 1,
        CreationTimestamp => 2,
        CreationName => 3,
        CreationAccountName => 4,
        ModificationDate => 5,
        ModificationTime => 6,
        ModificationTimestamp => 7,
        ModificationName => 8,
        ModificationAccountName => 9,
    }
}

fn op_code(op: SummaryOp) -> u8 {
    use SummaryOp::*;
    match op {
        Total => 1,
        Average => 2,
        Count => 3,
        Minimum => 4,
        Maximum => 5,
        StandardDeviation => 6,
        FractionOfTotal => 7,
        List => 8,
    }
}

/// The 26-byte option block of a field (FORMAT.md §5).
fn option_block(spec: &FieldSpec, language: u32) -> Vec<u8> {
    let mut o = vec![0u8; 26];
    o[0] = match spec.field_type {
        FieldType::Normal => 1,
        FieldType::Calculated { .. } => 2,
        FieldType::Summary { .. } => 3,
    };
    o[1] = type_code(spec.data_type);
    o[4..8].copy_from_slice(&language.to_be_bytes());
    let s = &spec.storage;
    // A unique value needs an index: FileMaker turns Minimal on.
    let indexing = match s.indexing {
        Indexing::None if spec.validation.unique => Indexing::Minimal,
        i => i,
    };
    o[8] = match indexing {
        Indexing::None => 0,
        Indexing::Minimal => 0x80,
        Indexing::All => 0xA0,
    } | if s.auto_index { 0 } else { 0x40 };
    o[9] = s.global as u8;
    if let FieldType::Calculated { stored: false, .. } = spec.field_type {
        o[9] |= 2;
    }
    if spec.field_type == FieldType::Normal {
        let a = &spec.auto_enter;
        o[10] = a.prohibit_modification as u8 | if a.lookup.is_some() { 0x04 } else { 0 };
        if let Some(st) = a.stamp {
            o[3] = stamp_code(st);
            o[11] |= 0x01;
        }
        if a.serial.is_some() {
            o[11] |= 0x02;
        }
        if a.constant.is_some() {
            o[11] |= 0x04;
        }
        if a.calculation.is_some() {
            o[11] |= 0x08;
            o[11] |= if a.always_evaluate { 0x20 } else { 0 };
            o[11] |= if a.replace_existing { 0x80 } else { 0 };
        }
        let v = &spec.validation;
        o[14] = (v.value_list.is_some() as u8)
            | if v.max_chars.is_some() { 0x02 } else { 0 }
            | if v.always { 0x04 } else { 0 }
            | if v.message.is_some() { 0x80 } else { 0 };
        o[20..24].copy_from_slice(&v.max_chars.unwrap_or(0).to_be_bytes());
        o[15] = (v.calculation.is_some() as u8)
            | if v.message.is_some() { 0x80 } else { 0 }
            | if v.allow_override { 0 } else { 0x04 }
            | if v.not_empty { 0x08 } else { 0 }
            | if v.unique { 0x10 } else { 0 }
            | if v.existing { 0x20 } else { 0 }
            | if v.range.is_some() { 0x40 } else { 0 };
    }
    // Serial increment, 1 in every field FileMaker creates.
    o[16..20].copy_from_slice(&spec.auto_enter.serial_increment.max(1).to_be_bytes());
    o[24..26].copy_from_slice(&s.repetitions.max(1).to_be_bytes());
    o
}

/// A field reference as summaries and lookups store it (14 bytes): a
/// 5-byte occurrence slot (`len 03 D0 hi lo`, or `02 01 00` and padding),
/// a byte (`7F` after a set second reference), a 4-byte field slot (`len 01
/// path-int`, padded), two zero bytes and the repetition (u16).
fn ref14(to: Option<u32>, mark: u8, field: Option<u32>) -> Vec<u8> {
    let mut b = match to {
        Some(t) => crate::graph::len_to(t),
        None => vec![2, 1, 0, 0, 0],
    };
    b.push(mark);
    let mut f = field.map_or(vec![2, 1, 0], crate::graph::fref);
    f.resize(4, 0);
    b.extend(f);
    b.extend([0, 0, 0, 1]);
    b
}

/// Summary definition (key 9, FORMAT.md §5): operation, options (0x04
/// running, 0x10 restart for each sorted group, 0x01 a second field), the
/// summarized field, then the second field: the sort field (on the
/// table's occurrence `ctx`) of a restarting running summary or a
/// subtotaled fraction, the weight of an average.
fn summary_block(op: SummaryOp, field: u32, running: bool, by: Option<u32>, ctx: u32) -> Vec<u8> {
    let flags = if running { 0x04 } else { 0 }
        | match (by, op) {
            (None, _) => 0,
            (Some(_), SummaryOp::Total | SummaryOp::Count) => 0x11,
            (Some(_), _) => 0x01,
        };
    let mut b = vec![op_code(op), flags];
    b.extend(ref14(None, 0, Some(field)));
    b.extend(match by {
        None => ref14(None, 0, None),
        Some(f) if op == SummaryOp::Average => ref14(None, 0x7F, Some(f)),
        Some(f) => ref14(Some(ctx), 0x7F, Some(f)),
    });
    b
}

/// Lookup definition (key 8): no-match option, copy-empty flag, the
/// source field and the starting occurrence (see [`Field::lookup`]).
fn lookup_block(l: &Lookup) -> Vec<u8> {
    let no_match = match l.no_match {
        NoMatch::DontCopy => 0,
        NoMatch::Lower => 1,
        NoMatch::Higher => 2,
        NoMatch::Constant(_) => 3,
    };
    let mut b = vec![no_match, l.copy_empty as u8];
    b.extend(ref14(Some(l.from), 0x7F, Some(l.field)));
    b.extend(crate::graph::len_to(l.context));
    b.push(0);
    b
}

/// Key 7 of "member of value list" validation: the value list, then the
/// bytes FileMaker 26 wrote after it (not decoded).
fn value_list_block(vl: u32) -> Vec<u8> {
    let mut b = crate::graph::fref(vl);
    b.extend([0, 0x90, 1, 2, 1, 0, 0, 0x71, 0xD1, 0xFF]);
    b
}

/// Spelled path from integers.
fn sp(p: &[u32]) -> Vec<Vec<u8>> {
    p.iter().map(|x| path_elem(*x)).collect()
}

/// Path element of a table occurrence node: `D0 hi lo` (id = 128 + hi:lo).
fn to_elem(to: u32) -> Vec<u8> {
    let n = to.saturating_sub(128) as u16;
    let [hi, lo] = n.to_be_bytes();
    vec![0xD0, hi, lo]
}

/// Table occurrence reference `03 D0 hi lo`.
fn to_ref(to: u32) -> Vec<u8> {
    let mut v = vec![3];
    v.extend(to_elem(to));
    v
}

/// Object reference in catalog indexes: length, path integer.
fn id_ref(id: u32) -> Vec<u8> {
    let e = path_elem(id);
    let mut v = vec![e.len() as u8];
    v.extend(e);
    v
}

fn kv(path: Vec<Vec<u8>>, key: u32, data: Vec<u8>) -> Entry {
    let item = if data.is_empty() && key < 0x100 {
        vec![0x00, key as u8]
    } else {
        write::encode(&Item::Kv { key, data })
    };
    Entry { path, item }
}

fn kvlong(path: Vec<Vec<u8>>, key: Vec<u8>, data: Vec<u8>) -> Entry {
    Entry {
        path,
        item: write::encode(&Item::KvLong { key, data }),
    }
}

fn marker(path: Vec<Vec<u8>>) -> Entry {
    Entry {
        path,
        item: vec![0x80],
    }
}

fn entry_kvlong(e: &Entry) -> Option<(Vec<u8>, Vec<u8>)> {
    match crate::chunk::parse(&e.item).ok()?.first()? {
        Chunk::KvLong { key, data } => Some((key.to_vec(), data.to_vec())),
        _ => None,
    }
}

/// How a catalog refers to its objects.
#[derive(Clone, Copy)]
enum Refs {
    /// Length + path integer (tables, layouts, fields).
    Id,
    /// `03 D0 hi lo` (table occurrences).
    Occurrence,
}

impl Refs {
    fn encode(self, id: u32) -> Vec<u8> {
        match self {
            Refs::Id => id_ref(id),
            Refs::Occurrence => to_ref(id),
        }
    }
    fn decode(self, b: &[u8]) -> Option<u32> {
        match self {
            Refs::Id => path_int(b.get(1..)?),
            Refs::Occurrence => crate::graph::to_ref(b),
        }
    }
}

/// A catalog: header path (`[…].[1]`) and its reference form.
#[derive(Clone)]
struct Catalog {
    header: Vec<Vec<u8>>,
    refs: Refs,
}

impl Catalog {
    fn sub(&self, k: u32) -> Vec<Vec<u8>> {
        let mut p = self.header.clone();
        p.push(path_elem(k));
        p
    }
}

/// Edits the schema of a file. See the module documentation.
pub struct Editor {
    header: Vec<u8>,
    entries: Vec<Entry>,
    /// Index language of new fields (the file's language, `[2]` key 13).
    language: u32,
    seed: u64,
}

const TABLES: [u32; 3] = [3, 16, 1];
const OCCURRENCES: [u32; 3] = [3, 17, 1];
const LAYOUTS: [u32; 2] = [4, 1];

impl Editor {
    pub fn open(file: &File) -> Result<Editor, Error> {
        let language = schema::locale(file)?.language;
        Ok(Editor {
            header: file.bytes()[..crate::SECTOR].to_vec(),
            entries: write::entries(file)?,
            language,
            seed: file.bytes().len() as u64,
        })
    }

    /// The edited file.
    pub fn finish(mut self) -> Vec<u8> {
        write::sort(&mut self.entries);
        write::build(&self.header, &self.entries)
    }

    fn uuid(&mut self) -> Vec<u8> {
        self.seed = self.seed.wrapping_add(1);
        fresh_uuid(self.seed)
    }

    // ---- entry access -------------------------------------------------

    fn kvs(&self, path: &[Vec<u8>]) -> Vec<(u32, Vec<u8>)> {
        self.entries
            .iter()
            .filter(|e| e.path == path)
            .filter_map(entry_kv)
            .collect()
    }

    fn get(&self, path: &[Vec<u8>], key: u32) -> Option<Vec<u8>> {
        self.kvs(path)
            .into_iter()
            .find(|(k, _)| *k == key)
            .map(|(_, d)| d)
    }

    fn remove_kv(&mut self, path: &[Vec<u8>], key: u32) {
        self.entries
            .retain(|e| !(e.path == path && entry_kv(e).is_some_and(|(k, _)| k == key)));
    }

    fn set(&mut self, path: Vec<Vec<u8>>, key: u32, data: Vec<u8>) {
        self.remove_kv(&path, key);
        self.ensure(path.clone());
        self.entries.push(kv(path, key, data));
    }

    /// Node marker for `path` and its parents.
    fn ensure(&mut self, path: Vec<Vec<u8>>) {
        for n in 1..=path.len() {
            let p = &path[..n];
            if !self.entries.iter().any(|e| e.path == p && e.item == [0x80]) {
                self.entries.push(marker(p.to_vec()));
            }
        }
    }

    fn remove_tree(&mut self, prefix: &[Vec<u8>]) {
        self.entries.retain(|e| !e.path.starts_with(prefix));
    }

    /// Increments a node's change counter (key 252).
    fn bump(&mut self, path: Vec<Vec<u8>>) {
        let n = self
            .get(&path, 252)
            .and_then(|d| read_len_int(&d))
            .unwrap_or(0);
        self.set(path, 252, len_int(n + 1));
    }

    /// Writes a value inline, or as a long value in `[path].[key]` (key 0 =
    /// length, keys 1… = 1000-byte chunks) when over 1000 bytes.
    fn set_long(&mut self, path: Vec<Vec<u8>>, key: u32, data: Vec<u8>) {
        let mut sub = path.clone();
        sub.push(path_elem(key));
        self.remove_tree(&sub);
        if data.len() <= 1000 {
            self.set(path, key, data);
            return;
        }
        self.remove_kv(&path, key);
        self.ensure(sub.clone());
        self.entries.push(kv(
            sub.clone(),
            0,
            (data.len() as u32).to_be_bytes().to_vec(),
        ));
        for (i, c) in data.chunks(1000).enumerate() {
            self.entries.push(kv(sub.clone(), i as u32 + 1, c.to_vec()));
        }
    }

    // ---- catalogs -------------------------------------------------------

    fn cat_last(&self, c: &Catalog) -> u32 {
        self.get(&c.header, 0)
            .and_then(|d| c.refs.decode(&d))
            .unwrap_or(0)
    }

    /// Ids listed in the display order `[1].[3]`, in order.
    fn cat_order(&self, c: &Catalog) -> Vec<(Vec<u8>, u32)> {
        let p = c.sub(3);
        let mut v: Vec<(Vec<u8>, u32)> = self
            .entries
            .iter()
            .filter(|e| e.path == p)
            .filter_map(entry_kvlong)
            .filter_map(|(k, d)| Some((k, c.refs.decode(&d)?)))
            .collect();
        v.sort();
        v
    }

    fn cat_add(&mut self, c: &Catalog, id: u32, name: &str) {
        self.ensure(c.header.clone());
        if id > self.cat_last(c) {
            self.set(c.header.clone(), 0, c.refs.encode(id));
        }
        self.index_name(c, id, name);
        let n = self.cat_order(c).len() as u32 + 1;
        self.ensure(c.sub(3));
        self.entries.push(kvlong(
            c.sub(3),
            n.to_be_bytes().to_vec(),
            c.refs.encode(id),
        ));
        self.set(c.header.clone(), 4, n.to_be_bytes().to_vec());
        self.bump(c.header.clone());
    }

    /// Name index entry; left out when the name holds a character whose
    /// collation weights are unknown. A name already indexed (layouts may
    /// share a name) gets the object's id appended to its key.
    fn index_name(&mut self, c: &Catalog, id: u32, name: &str) {
        let Some(mut key) = collate::name_key(name) else {
            return;
        };
        let p = c.sub(1);
        let taken = |es: &[Entry], key: &[u8]| {
            es.iter()
                .any(|e| e.path == p && entry_kvlong(e).is_some_and(|(k, _)| k == key))
        };
        if taken(&self.entries, &key) {
            key.extend(path_elem(id));
        }
        if !taken(&self.entries, &key) {
            self.ensure(p.clone());
            self.entries.push(kvlong(p, key, c.refs.encode(id)));
        }
    }

    fn unindex(&mut self, path: &[Vec<u8>], r: &[u8]) {
        self.entries
            .retain(|e| !(e.path == path && entry_kvlong(e).is_some_and(|(_, d)| d == r)));
    }

    fn cat_rename(&mut self, c: &Catalog, id: u32, name: &str) {
        let r = c.refs.encode(id);
        self.unindex(&c.sub(1), &r);
        self.index_name(c, id, name);
        self.bump(c.header.clone());
    }

    fn cat_remove(&mut self, c: &Catalog, id: u32) {
        let r = c.refs.encode(id);
        self.unindex(&c.sub(1), &r);
        // Display order: drop the entry and renumber the ones after it.
        let order: Vec<u32> = self
            .cat_order(c)
            .into_iter()
            .map(|(_, i)| i)
            .filter(|i| *i != id)
            .collect();
        let p = c.sub(3);
        self.entries.retain(|e| e.path != p || e.item == [0x80]);
        for (n, i) in order.iter().enumerate() {
            let key = (n as u32 + 1).to_be_bytes().to_vec();
            self.entries.push(kvlong(p.clone(), key, c.refs.encode(*i)));
        }
        self.set(
            c.header.clone(),
            4,
            (order.len() as u32).to_be_bytes().to_vec(),
        );
        self.bump(c.header.clone());
    }

    fn tables_cat() -> Catalog {
        Catalog {
            header: sp(&TABLES),
            refs: Refs::Id,
        }
    }
    fn occurrences_cat() -> Catalog {
        Catalog {
            header: sp(&OCCURRENCES),
            refs: Refs::Occurrence,
        }
    }
    fn layouts_cat() -> Catalog {
        Catalog {
            header: sp(&LAYOUTS),
            refs: Refs::Id,
        }
    }
    fn fields_cat(t: u32) -> Catalog {
        Catalog {
            header: sp(&[t, 3, 1]),
            refs: Refs::Id,
        }
    }

    // ---- reading the schema back -----------------------------------------

    /// Tables, fields and occurrences as they are now.
    pub fn names(&self) -> FileNames {
        let mut tables: BTreeMap<u32, schema::Table> = BTreeMap::new();
        let mut tos: BTreeMap<u32, TableOccurrence> = BTreeMap::new();
        let mut fields: BTreeMap<(u32, u32), Field> = BTreeMap::new();
        for e in &self.entries {
            let Some(p) = int_path(&e.path.iter().map(|x| x.as_slice()).collect::<Vec<_>>()) else {
                continue;
            };
            let Some((key, data)) = entry_kv(e) else {
                continue;
            };
            match (p.as_slice(), key) {
                ([3, 16, 5, t], 16) => {
                    tables
                        .entry(*t)
                        .or_insert_with(|| schema::Table {
                            id: *t,
                            name: String::new(),
                            fields: BTreeMap::new(),
                        })
                        .name = crate::text(&data);
                }
                ([3, 17, 5, to], 16 | 2) => {
                    let o = tos.entry(*to).or_insert_with(|| TableOccurrence {
                        id: *to,
                        ..Default::default()
                    });
                    if key == 16 {
                        o.name = crate::text(&data);
                    } else {
                        crate::graph::read_occurrence(o, &data);
                    }
                }
                ([t, 3, 5, f], 16 | 2 | 7 | 8 | 9 | 11 | 12 | 24) if *t >= 128 => {
                    let x = fields.entry((*t, *f)).or_default();
                    match key {
                        16 => x.name = crate::text(&data),
                        2 => x.options = data,
                        7 => x.value_list_raw = data,
                        8 => x.lookup_raw = data,
                        11 => x.lookup_constant = crate::text(&data),
                        24 => x.message = crate::text(&data),
                        12 => x.serial_next = crate::text(&data),
                        _ => x.summary_raw = data,
                    }
                }
                _ => {}
            }
        }
        for ((t, f), x) in fields {
            if let Some(tb) = tables.get_mut(&t) {
                tb.fields.insert(f, x);
            }
        }
        FileNames {
            tables: tables.into_values().collect(),
            occurrences: tos.into_values().collect(),
        }
    }

    /// Table id by name (case-insensitive).
    pub fn table_id(&self, name: &str) -> Option<u32> {
        self.names()
            .tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .map(|t| t.id)
    }

    /// Field id by name in table `t` (case-insensitive).
    pub fn field_id(&self, t: u32, name: &str) -> Option<u32> {
        let n = self.names();
        let tb = n.tables.iter().find(|x| x.id == t)?;
        tb.fields
            .iter()
            .find(|(_, f)| f.name.eq_ignore_ascii_case(name))
            .map(|(i, _)| *i)
    }

    /// The occurrence calculations of table `t` are evaluated from: its
    /// first table occurrence.
    fn context(&self, t: u32) -> Res<u32> {
        self.names()
            .occurrences
            .iter()
            .find(|o| o.base_table == t)
            .map(|o| o.id)
            .ok_or_else(|| EditError::Invalid(format!("table {t} has no table occurrence")))
    }

    fn has_records(&self, t: u32) -> bool {
        let p = sp(&[t, 5]);
        self.entries
            .iter()
            .any(|e| e.path.len() > 2 && e.path.starts_with(&p))
    }

    fn check_name(name: &str) -> Res<()> {
        if name.trim().is_empty() {
            return Err(EditError::Invalid("empty name".into()));
        }
        Ok(())
    }

    // ---- tables ---------------------------------------------------------

    /// Adds a base table, a table occurrence of the same name and, when the
    /// file has a layout to model it on, a layout. Returns (table,
    /// occurrence, layout).
    pub fn add_table(&mut self, name: &str) -> Res<(u32, u32, Option<u32>)> {
        Self::check_name(name)?;
        let names = self.names();
        if names
            .tables
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(name))
        {
            return Err(EditError::Exists(name.into()));
        }
        let tc = Self::tables_cat();
        let t = self
            .cat_last(&tc)
            .max(names.tables.iter().map(|x| x.id).max().unwrap_or(0))
            .max(128)
            + 1;
        self.cat_add(&tc, t, name);
        let node = sp(&[3, 16, 5, t]);
        let uuid = self.uuid();
        self.set(node.clone(), 16, encode_text(name));
        self.set(node.clone(), 216, uuid);
        self.set(node, 252, len_int(1));
        // Field catalog of the new table.
        let fc = Self::fields_cat(t);
        self.ensure(sp(&[t, 3, 5]));
        self.ensure(sp(&[t, 3, 1, 14]));
        self.set(fc.header.clone(), 4, 0u32.to_be_bytes().to_vec());
        let uuid = self.uuid();
        self.set(fc.header.clone(), 216, uuid);
        self.set(fc.header, 252, len_int(1));
        // Occurrence, named like the table when that name is free.
        let to_name = if names
            .occurrences
            .iter()
            .any(|o| o.name.eq_ignore_ascii_case(name))
        {
            (2..)
                .map(|i| format!("{name} {i}"))
                .find(|n| {
                    !names
                        .occurrences
                        .iter()
                        .any(|o| o.name.eq_ignore_ascii_case(n))
                })
                .unwrap_or_default()
        } else {
            name.to_string()
        };
        let to = self.add_occurrence(t, &to_name)?;
        let layout = self.add_layout(to, name)?;
        Ok((t, to, layout))
    }

    /// Adds a table occurrence of base table `t`.
    pub fn add_occurrence(&mut self, t: u32, name: &str) -> Res<u32> {
        Self::check_name(name)?;
        let names = self.names();
        if names
            .occurrences
            .iter()
            .any(|o| o.name.eq_ignore_ascii_case(name))
        {
            return Err(EditError::Exists(name.into()));
        }
        let oc = Self::occurrences_cat();
        let to = self
            .cat_last(&oc)
            .max(names.occurrences.iter().map(|x| x.id).max().unwrap_or(0))
            .max(128)
            + 1;
        if !names.tables.iter().any(|x| x.id == t) {
            return Err(EditError::NotFound(format!("table {t}")));
        }
        self.cat_add(&oc, to, name);
        // Key 2: 02 01 00, base table (len, 02, path int), left, top, width
        // and height (f32), u32 80, u32 0, colour. FileMaker lays out
        // occurrences of size 0 when it shows the graph; BaseCraft places a
        // new one to the right of the others (unplaced ones counted in a
        // row, as FileMaker places them), 131 × 111 points.
        let mut unplaced = 0.0;
        let right = names
            .occurrences
            .iter()
            .map(|o| {
                if o.width > 0.0 {
                    o.left + o.width
                } else {
                    unplaced += 188.0;
                    unplaced - 37.0
                }
            })
            .fold(0.0f32, f32::max);
        let b = path_elem(t);
        let mut def = vec![2, 1, 0, 1 + b.len() as u8, 2];
        def.extend(b);
        for v in [right + 57.0, 20.0, 131.0, 111.0] {
            def.extend(v.to_be_bytes());
        }
        def.extend([0, 0, 0, 0x50, 0, 0, 0, 0]);
        def.extend([0x78, 0x78, 0x78, 0xFF]);
        let node = [sp(&[3, 17, 5]), vec![to_elem(to)]].concat();
        let uuid = self.uuid();
        self.set(node.clone(), 2, def);
        self.set(node.clone(), 16, encode_text(name));
        self.set(node, 216, uuid);
        Ok(to)
    }

    /// A layout on occurrence `to`, modelled on the file's first layout that
    /// has only header, body, footer and navigation parts; no objects.
    fn add_layout(&mut self, to: u32, name: &str) -> Res<Option<u32>> {
        let Some(proto) = self.prototype_layout() else {
            return Ok(None);
        };
        let lc = Self::layouts_cat();
        let ids: BTreeSet<u32> = self.layout_ids();
        let l = self
            .cat_last(&lc)
            .max(ids.iter().copied().max().unwrap_or(0))
            + 1;
        let toxml = 0x10_4000 | to as u64;
        let (pp, np) = (sp(&[4, 5, proto]), sp(&[4, 5, l]));
        let mut add = Vec::new();
        for e in &self.entries {
            if !e.path.starts_with(&pp) {
                continue;
            }
            let rest = &e.path[pp.len()..];
            let first = rest.first().and_then(|x| path_int(x));
            let keep = match first {
                None => true,
                // Objects, their list, window state, occurrence node.
                Some(5 | 7 | 13 | 1) => false,
                // Spatial index and tab order: the nodes only.
                Some(10 | 21) => rest.len() == 1 && e.item == [0x80],
                _ => true,
            };
            if !keep {
                continue;
            }
            let path = [np.clone(), rest.to_vec()].concat();
            let item = match (rest.is_empty(), entry_kv(e)) {
                (true, Some((7, _))) => vec![0x00, 0x07],
                (true, Some((2, d))) => match crate::pb::MsgBuf::parse(&d) {
                    Some(mut m) => {
                        m.set(2, crate::pb::Field::Varint(toxml));
                        write::encode(&Item::Kv {
                            key: 2,
                            data: m.encode(),
                        })
                    }
                    None => e.item.clone(),
                },
                (true, Some((16 | 216 | 241 | 252 | 31873..=31875, _))) => continue,
                _ => e.item.clone(),
            };
            add.push(Entry { path, item });
        }
        // Catalog entry [4].[1].[7].[L]: occurrence and name.
        let (cp, cn) = (sp(&[4, 1, 7, proto]), sp(&[4, 1, 7, l]));
        for e in &self.entries {
            if e.path != cp {
                continue;
            }
            let item = match entry_kv(e) {
                Some((2, d)) => match crate::pb::MsgBuf::parse(&d) {
                    Some(mut m) => {
                        m.set(2, crate::pb::Field::Varint(toxml));
                        write::encode(&Item::Kv {
                            key: 2,
                            data: m.encode(),
                        })
                    }
                    None => e.item.clone(),
                },
                Some((241, _)) => write::encode(&Item::Kv {
                    key: 241,
                    data: vec![0; 8],
                }),
                Some((16 | 31873..=31875, _)) => continue,
                _ => e.item.clone(),
            };
            add.push(Entry {
                path: cn.clone(),
                item,
            });
        }
        self.entries.extend(add);
        self.ensure([np.clone(), vec![path_elem(1), to_elem(to)]].concat());
        let uuid = self.uuid();
        self.set(np.clone(), 16, encode_text(name));
        self.set(np.clone(), 216, uuid);
        self.set(np, 252, len_int(1));
        self.set(cn, 16, encode_text(name));
        self.cat_add(&lc, l, name);
        Ok(Some(l))
    }

    fn layout_ids(&self) -> BTreeSet<u32> {
        self.entries
            .iter()
            .filter(|e| e.path.len() == 3 && e.path[..2] == sp(&[4, 5])[..])
            .filter_map(|e| path_int(&e.path[2]))
            .collect()
    }

    /// First layout on an occurrence whose parts are all header, body,
    /// footer or navigation parts.
    fn prototype_layout(&self) -> Option<u32> {
        self.layout_ids().into_iter().find(|l| {
            let lp = sp(&[4, 5, *l]);
            let on_to = self
                .get(&lp, 2)
                .and_then(|d| crate::pb::Msg::parse(&d)?.uint(2))
                .is_some_and(|x| x != 0);
            let parts_ok = self
                .entries
                .iter()
                .filter(|e| e.path.len() == 5 && e.path.starts_with(&lp) && e.path[3] == [3])
                .filter_map(entry_kv)
                .filter(|(k, _)| *k == 2)
                .all(|(_, d)| {
                    crate::pb::Msg::parse(&d)
                        .and_then(|m| m.uint(1))
                        .is_some_and(|k| matches!(k, 1 | 4 | 7 | 8 | 9 | 12 | 13))
                });
            on_to && parts_ok
        })
    }

    /// Renames table `t`, and its occurrences and layouts that had its name.
    pub fn rename_table(&mut self, t: u32, name: &str) -> Res<()> {
        Self::check_name(name)?;
        let names = self.names();
        let old = names
            .tables
            .iter()
            .find(|x| x.id == t)
            .ok_or_else(|| EditError::NotFound(format!("table {t}")))?
            .name
            .clone();
        if names
            .tables
            .iter()
            .any(|x| x.id != t && x.name.eq_ignore_ascii_case(name))
        {
            return Err(EditError::Exists(name.into()));
        }
        let node = sp(&[3, 16, 5, t]);
        self.set(node.clone(), 16, encode_text(name));
        self.bump(node);
        self.cat_rename(&Self::tables_cat(), t, name);
        for o in names
            .occurrences
            .iter()
            .filter(|o| o.base_table == t && o.name == old)
        {
            if names
                .occurrences
                .iter()
                .any(|x| x.name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            let node = [sp(&[3, 17, 5]), vec![to_elem(o.id)]].concat();
            self.set(node.clone(), 16, encode_text(name));
            self.bump(node);
            self.cat_rename(&Self::occurrences_cat(), o.id, name);
            for l in self.layouts_on(o.id) {
                let lp = sp(&[4, 5, l]);
                if self.get(&lp, 16).map(|d| crate::text(&d)) == Some(old.clone()) {
                    self.set(lp.clone(), 16, encode_text(name));
                    self.bump(lp);
                    self.set(sp(&[4, 1, 7, l]), 16, encode_text(name));
                    self.cat_rename(&Self::layouts_cat(), l, name);
                }
            }
        }
        Ok(())
    }

    fn layouts_on(&self, to: u32) -> Vec<u32> {
        let x = 0x10_4000 | to as u64;
        self.layout_ids()
            .into_iter()
            .filter(|l| {
                self.get(&sp(&[4, 5, *l]), 2)
                    .and_then(|d| crate::pb::Msg::parse(&d)?.uint(2))
                    == Some(x)
            })
            .collect()
    }

    /// Deletes table `t` with its fields and records, its table
    /// occurrences and their relationships. Layouts are kept (FileMaker
    /// shows them as based on a missing table).
    pub fn delete_table(&mut self, t: u32) -> Res<()> {
        let names = self.names();
        if !names.tables.iter().any(|x| x.id == t) {
            return Err(EditError::NotFound(format!("table {t}")));
        }
        self.remove_tree(&sp(&[3, 16, 5, t]));
        self.cat_remove(&Self::tables_cat(), t);
        self.remove_tree(&sp(&[t]));
        self.remove_tree(&sp(&[3, 17, 1, 14, t]));
        let tos: Vec<u32> = names
            .occurrences
            .iter()
            .filter(|o| o.base_table == t)
            .map(|o| o.id)
            .collect();
        for to in &tos {
            self.drop_occurrence(*to);
        }
        Ok(())
    }

    // ---- table occurrences ------------------------------------------------

    /// Occurrence id by name (case-insensitive).
    pub fn occurrence_id(&self, name: &str) -> Option<u32> {
        self.names()
            .occurrences
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
            .map(|o| o.id)
    }

    /// Renames table occurrence `to`.
    pub fn rename_occurrence(&mut self, to: u32, name: &str) -> Res<()> {
        Self::check_name(name)?;
        let names = self.names();
        if !names.occurrences.iter().any(|o| o.id == to) {
            return Err(EditError::NotFound(format!("table occurrence {to}")));
        }
        if names
            .occurrences
            .iter()
            .any(|o| o.id != to && o.name.eq_ignore_ascii_case(name))
        {
            return Err(EditError::Exists(name.into()));
        }
        let node = [sp(&[3, 17, 5]), vec![to_elem(to)]].concat();
        self.set(node.clone(), 16, encode_text(name));
        self.bump(node);
        self.cat_rename(&Self::occurrences_cat(), to, name);
        Ok(())
    }

    /// Deletes table occurrence `to` and its relationships. The last
    /// occurrence of a table cannot be deleted: its calculations are
    /// evaluated from it.
    pub fn delete_occurrence(&mut self, to: u32) -> Res<()> {
        let names = self.names();
        let o = names
            .occurrences
            .iter()
            .find(|o| o.id == to)
            .ok_or_else(|| EditError::NotFound(format!("table occurrence {to}")))?;
        if !names
            .occurrences
            .iter()
            .any(|x| x.id != to && x.base_table == o.base_table)
        {
            return Err(EditError::Invalid(format!(
                "{} is the only occurrence of its table",
                o.name
            )));
        }
        self.drop_occurrence(to);
        Ok(())
    }

    fn drop_occurrence(&mut self, to: u32) {
        self.remove_tree(&[sp(&[3, 17, 5]), vec![to_elem(to)]].concat());
        self.remove_tree(&[to_elem(to)]);
        self.cat_remove(&Self::occurrences_cat(), to);
        for r in self
            .relationships()
            .iter()
            .filter(|r| r.left == to || r.right == to)
        {
            self.remove_relationship(r.id);
        }
        self.rebuild_graph();
    }

    // ---- relationships ----------------------------------------------------

    /// Relationships as they are now.
    pub fn relationships(&self) -> Vec<Relationship> {
        let mut rels: BTreeMap<u32, Relationship> = BTreeMap::new();
        let mut preds: BTreeMap<u32, Vec<(u32, Predicate)>> = BTreeMap::new();
        for e in &self.entries {
            let (Some(p), Some((key, data))) = (ipath(e), entry_kv(e)) else {
                continue;
            };
            match p.as_slice() {
                [3, 251, 5, r] if key == 2 => {
                    let x = rels.entry(*r).or_default();
                    x.id = *r;
                    crate::graph::read_relationship(x, &data);
                }
                [3, 251, 5, r, 3] => {
                    if let Some(pr) = crate::graph::parse_predicate(&data) {
                        preds.entry(*r).or_default().push((key, pr));
                    }
                }
                _ => {}
            }
        }
        for (r, mut ps) in preds {
            ps.sort_by_key(|(k, _)| *k);
            if let Some(x) = rels.get_mut(&r) {
                x.predicates = ps.into_iter().map(|(_, p)| p).collect();
            }
        }
        rels.into_values().collect()
    }

    /// The relationship between occurrences `a` and `b` (either side).
    pub fn relationship_between(&self, a: u32, b: u32) -> Option<u32> {
        self.relationships()
            .iter()
            .find(|r| (r.left, r.right) == (a, b) || (r.left, r.right) == (b, a))
            .map(|r| r.id)
    }

    /// Occurrences `a` and `b` are joined by a path of relationships,
    /// leaving out relationship `skip`.
    fn connected(&self, a: u32, b: u32, skip: Option<u32>) -> bool {
        let rels = self.relationships();
        let mut seen = BTreeSet::from([a]);
        let mut todo = vec![a];
        while let Some(x) = todo.pop() {
            if x == b {
                return true;
            }
            for r in rels.iter().filter(|r| Some(r.id) != skip) {
                let y = match (r.left == x, r.right == x) {
                    (true, _) => r.right,
                    (_, true) => r.left,
                    _ => continue,
                };
                if seen.insert(y) {
                    todo.push(y);
                }
            }
        }
        false
    }

    /// The definition of relationship `id`.
    pub fn relationship_spec(&self, id: u32) -> Res<RelationshipSpec> {
        let r = self
            .relationships()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| EditError::NotFound(format!("relationship {id}")))?;
        let side = |create: bool, delete: bool, sort: &[SortKey]| RelSide {
            create,
            delete,
            sort: sort.iter().map(|k| (k.field, k.descending)).collect(),
        };
        Ok(RelationshipSpec {
            left: r.left,
            right: r.right,
            predicates: r.predicates.clone(),
            left_options: side(
                r.left_cascade_create(),
                r.left_cascade_delete(),
                &r.left_sort,
            ),
            right_options: side(
                r.right_cascade_create(),
                r.right_cascade_delete(),
                &r.right_sort,
            ),
        })
    }

    /// Checks a relationship as FileMaker's Edit Relationship dialog does.
    /// `id` is the relationship being changed.
    fn check_relationship(&self, spec: &RelationshipSpec, id: Option<u32>) -> Res<()> {
        let names = self.names();
        let table_of = |to: u32| -> Res<&schema::Table> {
            let o = names
                .occurrences
                .iter()
                .find(|o| o.id == to)
                .ok_or_else(|| EditError::NotFound(format!("table occurrence {to}")))?;
            names
                .tables
                .iter()
                .find(|t| t.id == o.base_table)
                .ok_or_else(|| EditError::NotFound(format!("table of occurrence {}", o.name)))
        };
        let (lt, rt) = (table_of(spec.left)?, table_of(spec.right)?);
        if spec.left == spec.right {
            return Err(EditError::Invalid(
                "a relationship joins two different table occurrences".into(),
            ));
        }
        if spec.predicates.is_empty() {
            return Err(EditError::Invalid(
                "a relationship needs a match field".into(),
            ));
        }
        let field = |t: &schema::Table, f: u32| -> Res<()> {
            t.fields
                .contains_key(&f)
                .then_some(())
                .ok_or_else(|| EditError::NotFound(format!("field {f} of table {}", t.name)))
        };
        for p in &spec.predicates {
            field(lt, p.left_field)?;
            field(rt, p.right_field)?;
            if let JoinOp::Unknown(n) = p.op {
                return Err(EditError::Invalid(format!("unknown operator {n}")));
            }
        }
        for (s, t) in [(&spec.left_options, lt), (&spec.right_options, rt)] {
            for (f, _) in &s.sort {
                field(t, *f)?;
            }
        }
        let equal = spec.predicates.iter().all(|p| p.op == JoinOp::Equal);
        if (spec.left_options.create || spec.right_options.create) && !equal {
            return Err(EditError::Invalid(
                "records can be created only through = relationships".into(),
            ));
        }
        if let Some(r) = self.relationship_between(spec.left, spec.right)
            && Some(r) != id
        {
            return Err(EditError::Exists(format!(
                "relationship between {} and {}",
                spec.left, spec.right
            )));
        }
        if self.connected(spec.left, spec.right, id) {
            // FileMaker's graph has no cycles: a second path needs another occurrence.
            return Err(EditError::Invalid(format!(
                "occurrences {} and {} are already related; add another table occurrence",
                spec.left, spec.right
            )));
        }
        Ok(())
    }

    /// Adds a relationship; returns its id.
    pub fn add_relationship(&mut self, spec: &RelationshipSpec) -> Res<u32> {
        self.check_relationship(spec, None)?;
        let h = rels_path(&[1]);
        let last = self.get(&h, 0).and_then(|d| read_len_int(&d)).unwrap_or(0) as u32;
        let id = self
            .relationships()
            .iter()
            .map(|r| r.id)
            .fold(last, u32::max)
            + 1;
        self.ensure(h.clone());
        let count = self.relationships().len() as u32 + 1;
        self.set(h.clone(), 0, len_int(id as u64));
        self.set(h.clone(), 4, count.to_be_bytes().to_vec());
        if self.get(&h, 216).is_none() {
            let u = self.uuid();
            self.set(h.clone(), 216, u);
        }
        self.bump(h);
        let u = self.uuid();
        self.set(rels_path(&[5, id]), 216, u);
        self.write_relationship(id, spec);
        Ok(id)
    }

    /// Replaces the definition of relationship `id`.
    pub fn set_relationship(&mut self, id: u32, spec: &RelationshipSpec) -> Res<()> {
        if !self.relationships().iter().any(|r| r.id == id) {
            return Err(EditError::NotFound(format!("relationship {id}")));
        }
        self.check_relationship(spec, Some(id))?;
        self.write_relationship(id, spec);
        self.bump(rels_path(&[1]));
        Ok(())
    }

    /// Deletes relationship `id`.
    pub fn delete_relationship(&mut self, id: u32) -> Res<()> {
        if !self.relationships().iter().any(|r| r.id == id) {
            return Err(EditError::NotFound(format!("relationship {id}")));
        }
        self.remove_relationship(id);
        self.rebuild_graph();
        Ok(())
    }

    fn remove_relationship(&mut self, id: u32) {
        self.remove_tree(&rels_path(&[5, id]));
        let h = rels_path(&[1]);
        let n = self.relationships().len() as u32;
        self.set(h.clone(), 4, n.to_be_bytes().to_vec());
        self.bump(h);
    }

    /// Node `[3].[251].[5].[id]`: key 2 (occurrences, sort orders, flags),
    /// predicates in `[3]` keys 1…n; then the occurrences' links.
    fn write_relationship(&mut self, id: u32, spec: &RelationshipSpec) {
        use crate::graph::*;
        let node = rels_path(&[5, id]);
        let sort = |to: u32, s: &RelSide| -> Vec<SortKey> {
            s.sort
                .iter()
                .map(|(field, descending)| SortKey {
                    to,
                    field: *field,
                    descending: *descending,
                })
                .collect()
        };
        let (l, r) = (&spec.left_options, &spec.right_options);
        let flag = |on: bool, f: u8| if on { f } else { 0 };
        let flags = flag(r.delete, RIGHT_DELETE)
            | flag(l.delete, LEFT_DELETE)
            | flag(r.create, RIGHT_CREATE)
            | flag(l.create, LEFT_CREATE)
            | flag(!r.sort.is_empty(), RIGHT_SORT)
            | flag(!l.sort.is_empty(), LEFT_SORT);
        let def = relationship_bytes(
            spec.left,
            spec.right,
            &sort(spec.left, l),
            &sort(spec.right, r),
            flags,
        );
        self.set(node.clone(), 2, def);
        let pp = [node, vec![path_elem(3)]].concat();
        self.remove_tree(&pp);
        self.ensure(pp.clone());
        for (i, p) in spec.predicates.iter().enumerate() {
            self.entries
                .push(kv(pp.clone(), i as u32 + 1, predicate_bytes(p)));
        }
        self.rebuild_graph();
    }

    /// Rewrites what FileMaker derives from the relationships: each
    /// occurrence's links `[3].[17].[5].[TO].[251]` (other occurrence →
    /// relationship), the route table `[3].[17].[1].[8].[TO]` (every
    /// reachable occurrence → the first relationship on the way), and
    /// drops the evaluation plan cache `[3].[17].[1].[14]`.
    fn rebuild_graph(&mut self) {
        let rels = self.relationships();
        let tos: Vec<u32> = self.names().occurrences.iter().map(|o| o.id).collect();
        let links = |to: u32| [sp(&[3, 17, 5]), vec![to_elem(to), vec![RELS]]].concat();
        let routes = sp(&[3, 17, 1, 8]);
        for to in &tos {
            self.remove_tree(&links(*to));
        }
        self.entries
            .retain(|e| !(e.path.starts_with(&routes) && e.path.len() > routes.len()));
        self.remove_tree(&sp(&[3, 17, 1, 14]));
        for r in &rels {
            for (a, b) in [(r.left, r.right), (r.right, r.left)] {
                self.ensure(links(a));
                self.entries
                    .push(kv3(links(a), to_elem(b), len_int(r.id as u64)));
            }
        }
        if rels.is_empty() {
            return;
        }
        self.ensure(routes.clone());
        for a in tos {
            // Breadth-first: the relationship each occurrence is first reached by.
            let mut first: BTreeMap<u32, u32> = BTreeMap::new();
            let mut todo = std::collections::VecDeque::from([(a, 0u32)]);
            while let Some((x, via)) = todo.pop_front() {
                for r in &rels {
                    let y = match (r.left == x, r.right == x) {
                        (true, _) => r.right,
                        (_, true) => r.left,
                        _ => continue,
                    };
                    if y != a && !first.contains_key(&y) {
                        let v = if via == 0 { r.id } else { via };
                        first.insert(y, v);
                        todo.push_back((y, v));
                    }
                }
            }
            if first.is_empty() {
                continue;
            }
            let p = [routes.clone(), vec![to_elem(a)]].concat();
            self.ensure(p.clone());
            for (b, r) in first {
                self.entries
                    .push(kv3(p.clone(), to_elem(b), len_int(r as u64)));
            }
        }
    }

    // ---- value lists --------------------------------------------------------

    fn value_lists_cat() -> Catalog {
        Catalog {
            header: sp(&[33, 1]),
            refs: Refs::Id,
        }
    }

    /// Value lists as they are now.
    pub fn value_lists(&self) -> Vec<ValueList> {
        let mut nodes: BTreeMap<u32, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
        for e in &self.entries {
            if let (Some([33, 5, id]), Some((k, d))) = (ipath(e).as_deref(), entry_kv(e)) {
                nodes.entry(*id).or_default().insert(k, d);
            }
        }
        nodes
            .iter()
            .map(|(id, kv)| crate::valuelists::from_keys(*id, kv))
            .collect()
    }

    /// Value list id by name (case-insensitive).
    pub fn value_list_id(&self, name: &str) -> Option<u32> {
        self.value_lists()
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(name))
            .map(|v| v.id)
    }

    fn check_value_list(&self, id: Option<u32>, name: &str, source: &Source) -> Res<()> {
        Self::check_name(name)?;
        if self
            .value_lists()
            .iter()
            .any(|v| Some(v.id) != id && v.name.eq_ignore_ascii_case(name))
        {
            return Err(EditError::Exists(name.into()));
        }
        let names = self.names();
        let check = |to: u32, f: Option<u32>| -> Res<()> {
            let o = names
                .occurrences
                .iter()
                .find(|o| o.id == to)
                .ok_or_else(|| EditError::NotFound(format!("table occurrence {to}")))?;
            match f {
                Some(f)
                    if !names
                        .tables
                        .iter()
                        .any(|t| t.id == o.base_table && t.fields.contains_key(&f)) =>
                {
                    Err(EditError::NotFound(format!("field {f} of {}", o.name)))
                }
                _ => Ok(()),
            }
        };
        match source {
            Source::Custom(_) => Ok(()),
            Source::Field {
                to,
                field,
                second,
                related_from,
                ..
            } => {
                check(*to, Some(*field))?;
                if let Some((t, f)) = second {
                    check(*t, Some(*f))?;
                }
                if let Some(r) = related_from {
                    check(*r, None)?;
                    if !self.connected(*r, *to, None) || r == to {
                        return Err(EditError::Invalid(format!(
                            "occurrence {to} is not related to {r}"
                        )));
                    }
                }
                Ok(())
            }
            Source::Unknown(n) => Err(EditError::Invalid(format!("unknown value list source {n}"))),
        }
    }

    /// Adds a value list; returns its id.
    pub fn add_value_list(&mut self, name: &str, source: &Source) -> Res<u32> {
        self.check_value_list(None, name, source)?;
        let c = Self::value_lists_cat();
        let id = self
            .value_lists()
            .iter()
            .map(|v| v.id)
            .fold(self.cat_last(&c), u32::max)
            + 1;
        self.cat_add(&c, id, name);
        let ids = c.sub(7);
        self.ensure(ids.clone());
        self.entries.push(kv(ids, id, Vec::new()));
        let u = self.uuid();
        self.set(sp(&[33, 5, id]), 216, u);
        self.write_value_list(id, name, source);
        Ok(id)
    }

    /// Replaces value list `id` (name and values).
    pub fn set_value_list(&mut self, id: u32, name: &str, source: &Source) -> Res<()> {
        let old = self
            .value_lists()
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| EditError::NotFound(format!("value list {id}")))?;
        self.check_value_list(Some(id), name, source)?;
        let c = Self::value_lists_cat();
        if old.name != name {
            self.cat_rename(&c, id, name);
        } else {
            self.bump(c.header);
        }
        self.write_value_list(id, name, source);
        Ok(())
    }

    /// Deletes value list `id`.
    pub fn delete_value_list(&mut self, id: u32) -> Res<()> {
        if !self.value_lists().iter().any(|v| v.id == id) {
            return Err(EditError::NotFound(format!("value list {id}")));
        }
        let c = Self::value_lists_cat();
        self.remove_tree(&sp(&[33, 5, id]));
        self.cat_remove(&c, id);
        self.unindex(&c.sub(6), &id_ref(id));
        let ids = c.sub(7);
        self.entries
            .retain(|e| !(e.path == ids && entry_kv(e).is_some_and(|(k, _)| k == id)));
        Ok(())
    }

    /// Node `[33].[5].[V]`: key 2 definition, key 3 custom values (CR
    /// between values), key 16 name; and the catalog's index by source and
    /// name `[33].[1].[6]`.
    fn write_value_list(&mut self, id: u32, name: &str, source: &Source) {
        let node = sp(&[33, 5, id]);
        self.set(node.clone(), 2, crate::valuelists::definition(source));
        self.remove_kv(&node, 3);
        if let Source::Custom(values) = source {
            self.set(node.clone(), 3, encode_text(values));
        }
        self.set(node.clone(), 16, encode_text(name));
        let tp = Self::value_lists_cat().sub(6);
        self.unindex(&tp, &id_ref(id));
        let kind = matches!(source, Source::Field { .. }) as u8;
        if let Some(key) = collate::index_key(&[0, kind], name) {
            self.ensure(tp.clone());
            self.entries.push(kvlong(tp, key, id_ref(id)));
        }
    }

    // ---- field references -------------------------------------------------

    /// Checks the fields, occurrences and value lists a field definition
    /// of table `t` refers to.
    fn check_field_refs(&self, t: u32, spec: &FieldSpec) -> Res<()> {
        let names = self.names();
        let has = |tb: u32, f: u32| {
            names
                .tables
                .iter()
                .any(|x| x.id == tb && x.fields.contains_key(&f))
        };
        if let FieldType::Summary {
            op,
            field,
            running,
            by,
        } = spec.field_type
        {
            use SummaryOp::*;
            if !has(t, field) {
                return Err(EditError::NotFound(format!("summarized field {field}")));
            }
            if running && !matches!(op, Total | Count) {
                return Err(EditError::Invalid(format!("{op:?} cannot be running")));
            }
            if let Some(b) = by {
                if !has(t, b) {
                    return Err(EditError::NotFound(format!("field {b}")));
                }
                match op {
                    Total | Count if !running => {
                        return Err(EditError::Invalid(
                            "restarting for each sorted group needs a running summary".into(),
                        ));
                    }
                    Total | Count | Average | FractionOfTotal => {}
                    _ => return Err(EditError::Invalid(format!("{op:?} takes no second field"))),
                }
            }
        }
        if let Some(l) = &spec.auto_enter.lookup {
            let to = |id: u32| {
                names
                    .occurrences
                    .iter()
                    .find(|o| o.id == id)
                    .ok_or_else(|| EditError::NotFound(format!("table occurrence {id}")))
            };
            if to(l.context)?.base_table != t {
                return Err(EditError::Invalid(
                    "a lookup starts from an occurrence of the field's table".into(),
                ));
            }
            if !has(to(l.from)?.base_table, l.field) {
                return Err(EditError::NotFound(format!("lookup field {}", l.field)));
            }
            if l.from == l.context || !self.connected(l.context, l.from, None) {
                return Err(EditError::Invalid(format!(
                    "occurrence {} is not related to {}",
                    l.from, l.context
                )));
            }
        }
        if let Some(vl) = spec.validation.value_list
            && !self.value_lists().iter().any(|v| v.id == vl)
        {
            return Err(EditError::NotFound(format!("value list {vl}")));
        }
        Ok(())
    }

    // ---- fields -----------------------------------------------------------

    /// Adds a field to table `t`; returns its id.
    pub fn add_field(&mut self, t: u32, spec: &FieldSpec) -> Res<u32> {
        Self::check_name(&spec.name)?;
        let names = self.names();
        let tb = names
            .tables
            .iter()
            .find(|x| x.id == t)
            .ok_or_else(|| EditError::NotFound(format!("table {t}")))?;
        if tb
            .fields
            .values()
            .any(|f| f.name.eq_ignore_ascii_case(&spec.name))
        {
            return Err(EditError::Exists(spec.name.clone()));
        }
        let fc = Self::fields_cat(t);
        let f = self
            .cat_last(&fc)
            .max(tb.fields.keys().copied().max().unwrap_or(0))
            + 1;
        // Register the name first: a calculation may refer to the field itself.
        self.set(sp(&[t, 3, 5, f]), 16, encode_text(&spec.name));
        if let Err(e) = self.write_field(t, f, spec) {
            self.remove_tree(&sp(&[t, 3, 5, f]));
            return Err(e);
        }
        self.cat_add(&fc, f, &spec.name);
        Ok(f)
    }

    /// Replaces the definition of field `f` (name, type and options).
    pub fn set_field(&mut self, t: u32, f: u32, spec: &FieldSpec) -> Res<()> {
        Self::check_name(&spec.name)?;
        let names = self.names();
        let tb = names
            .tables
            .iter()
            .find(|x| x.id == t)
            .ok_or_else(|| EditError::NotFound(format!("table {t}")))?;
        let old = tb
            .fields
            .get(&f)
            .ok_or_else(|| EditError::NotFound(format!("field {f}")))?;
        if tb
            .fields
            .iter()
            .any(|(i, x)| *i != f && x.name.eq_ignore_ascii_case(&spec.name))
        {
            return Err(EditError::Exists(spec.name.clone()));
        }
        let renamed = old.name != spec.name;
        self.write_field(t, f, spec)?;
        if renamed {
            self.cat_rename(&Self::fields_cat(t), f, &spec.name);
        } else {
            self.bump(Self::fields_cat(t).header);
        }
        Ok(())
    }

    /// Renames field `f` of table `t`.
    pub fn rename_field(&mut self, t: u32, f: u32, name: &str) -> Res<()> {
        let mut spec = self.field_spec(t, f)?;
        spec.name = name.to_string();
        self.set_field(t, f, &spec)
    }

    /// Next value of the serial number of field `f` (key 12), if it has one.
    pub fn next_serial(&self, t: u32, f: u32) -> Option<String> {
        self.get(&sp(&[t, 3, 5, f]), 12).map(|d| crate::text(&d))
    }

    /// Sets the next serial value of field `f` (records created elsewhere
    /// must not reuse a number).
    pub fn set_next_serial(&mut self, t: u32, f: u32, next: &str) {
        let node = sp(&[t, 3, 5, f]);
        self.set(node.clone(), 12, encode_text(next));
        self.bump(node);
    }

    /// Deletes field `f` of table `t` with its values and indexes.
    pub fn delete_field(&mut self, t: u32, f: u32) -> Res<()> {
        let names = self.names();
        if !names
            .tables
            .iter()
            .any(|x| x.id == t && x.fields.contains_key(&f))
        {
            return Err(EditError::NotFound(format!("field {f}")));
        }
        self.remove_tree(&sp(&[t, 3, 5, f]));
        self.cat_remove(&Self::fields_cat(t), f);
        self.unindex(&sp(&[t, 3, 1, 6]), &id_ref(f));
        self.remove_tree(&sp(&[t, 1, f]));
        // Dependencies of the field and on the field.
        self.remove_tree(&lookup_deps(t, f));
        for d in [32880, 32881] {
            self.remove_tree(&sp(&[t, 3, 1, 14, d, f]));
            let dp = sp(&[t, 3, 1, 14, d]);
            self.entries.retain(|e| {
                !(e.path.len() == dp.len() + 1
                    && e.path.starts_with(&dp)
                    && entry_kv(e).is_some_and(|(k, _)| k == f))
            });
        }
        // Values in records: key F of [T].[5].[R], long values [R].[F].
        let rp = sp(&[t, 5]);
        let fe = path_elem(f);
        self.entries.retain(|e| {
            if !e.path.starts_with(&rp) || e.path.len() < 3 {
                return true;
            }
            match e.path.len() {
                3 => !entry_kv(e).is_some_and(|(k, _)| k == f),
                _ => e.path[3] != fe,
            }
        });
        self.remove_tree(&sp(&[3, 17, 1, 14, t]));
        Ok(())
    }

    /// The current definition of field `f` of table `t`.
    pub fn field_spec(&self, t: u32, f: u32) -> Res<FieldSpec> {
        let names = self.names();
        let tb = names
            .tables
            .iter()
            .find(|x| x.id == t)
            .ok_or_else(|| EditError::NotFound(format!("table {t}")))?;
        let x = tb
            .fields
            .get(&f)
            .ok_or_else(|| EditError::NotFound(format!("field {f}")))?;
        let node = sp(&[t, 3, 5, f]);
        let text = |k: u32| self.get(&node, k).map(|d| crate::text(&d));
        let calc_at = |k: u32| -> Option<String> {
            let mut p = node.clone();
            p.push(path_elem(k));
            let k4 = self.get(&p, 4)?;
            let k5 = self.get(&p, 5).or_else(|| {
                let mut q = p.clone();
                q.push(path_elem(5));
                let mut kv = self.kvs(&q);
                kv.sort();
                Some(
                    kv.into_iter()
                        .filter(|(k, _)| *k >= 1)
                        .flat_map(|(_, d)| d)
                        .collect(),
                )
            })?;
            calc::decompile(&k5, calc::context(&k4).unwrap_or(0), &names).ok()
        };
        let field_type = match x.kind() {
            FieldKind::Calculated => FieldType::Calculated {
                formula: calc_at(5).unwrap_or_default(),
                stored: x.stored(),
            },
            FieldKind::Summary => {
                let s = x
                    .summary()
                    .ok_or_else(|| EditError::Invalid("unreadable summary field".into()))?;
                FieldType::Summary {
                    op: s.op,
                    field: s.field,
                    running: x.summary_running(),
                    by: x.summary_by(),
                }
            }
            _ => FieldType::Normal,
        };
        let v = x.validation();
        let normal = field_type == FieldType::Normal;
        Ok(FieldSpec {
            name: x.name.clone(),
            comment: text(3).unwrap_or_default(),
            data_type: x.data_type(),
            auto_enter: AutoEnter {
                stamp: x.auto_stamp(),
                serial: x
                    .auto_serial()
                    .then(|| text(12).unwrap_or_else(|| "1".into())),
                serial_increment: x.serial_increment(),
                lookup: x.lookup(),
                constant: x.auto_constant().then(|| text(13).unwrap_or_default()),
                calculation: if normal && x.auto_calculation() {
                    calc_at(5)
                } else {
                    None
                },
                replace_existing: x.auto_overwrite(),
                always_evaluate: x.auto_always_evaluate(),
                prohibit_modification: x.prohibit_modification(),
            },
            validation: ValidationSpec {
                always: v.always,
                allow_override: v.allow_override,
                not_empty: v.not_empty,
                unique: v.unique,
                existing: v.existing,
                range: v
                    .range
                    .then(|| (text(33).unwrap_or_default(), text(34).unwrap_or_default())),
                calculation: if v.calculation { calc_at(25) } else { None },
                value_list: x.validation_value_list(),
                max_chars: x.max_size(),
                message: if v.custom_message {
                    calc_at(32).or_else(|| text(24))
                } else {
                    None
                },
            },
            storage: Storage {
                global: x.global(),
                repetitions: x.max_repetitions().max(1),
                indexing: x.indexing(),
                auto_index: x.auto_index(),
            },
            field_type,
        })
    }

    /// Writes the node of field `f` and everything that depends on it:
    /// type index, dependencies, index statistics, the graph's dependency
    /// cache (dropped: FileMaker rebuilds it).
    fn write_field(&mut self, t: u32, f: u32, spec: &FieldSpec) -> Res<()> {
        let node = sp(&[t, 3, 5, f]);
        let ctx = self.context(t)?;
        self.check_field_refs(t, spec)?;
        // Compile first: nothing is written when a calculation is wrong.
        let names = self.names();
        let main = match &spec.field_type {
            FieldType::Calculated { formula, .. } => Some(calc::compile(formula, ctx, &names)?),
            FieldType::Normal => match &spec.auto_enter.calculation {
                Some(c) => Some(calc::compile(c, ctx, &names)?),
                None => None,
            },
            FieldType::Summary { .. } => None,
        };
        let check = match (&spec.field_type, &spec.validation.calculation) {
            (FieldType::Normal, Some(c)) => Some(calc::compile(c, ctx, &names)?),
            _ => None,
        };
        let message = match (&spec.field_type, &spec.validation.message) {
            (FieldType::Normal, Some(c)) => Some(calc::compile(c, ctx, &names)?),
            _ => None,
        };
        let records = self.has_records(t);
        let mut options = option_block(spec, self.language);
        // An existing field keeps its index language and the options
        // BaseCraft does not model (lookups, value-list and size
        // validation, strict data types, messages, furigana…).
        if let Some(old) = self.get(&node, 2).filter(|o| o.len() == 26) {
            const MANAGED: [u8; 26] = [
                0xFF, 0xFF, 0, 0xFF, 0, 0, 0, 0, 0xE8, 0x03, 0x05, 0xAF, 0, 0, 0x87, 0xFD, 0xFF,
                0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            ];
            for i in 0..26 {
                options[i] = old[i] & !MANAGED[i] | options[i] & MANAGED[i];
            }
            // A serial generated on commit stays so.
            if spec.auto_enter.serial.is_some() && old[10] & 0x02 != 0 {
                options[11] &= !0x02;
            } else if spec.auto_enter.serial.is_none() {
                options[10] &= !0x02;
            }
        }
        if records && options[8] & 0xA8 != 0 {
            // An index FileMaker would read as empty: let it build one on demand.
            options[8] &= !0xA8;
        }
        // Node: keep the keys BaseCraft does not write (UUIDs, tags,
        // metadata…) and the sub-nodes other than the calculations.
        let uuid = self.get(&node, 216);
        let count = self
            .get(&node, 252)
            .and_then(|d| read_len_int(&d))
            .unwrap_or(0);
        let calcs = [5u32, 25, 32].map(|k| [node.clone(), vec![path_elem(k)]].concat());
        self.entries.retain(|e| {
            if !e.path.starts_with(&node) {
                return true;
            }
            if e.path.len() == node.len() {
                return !entry_kv(e).is_some_and(|(k, _)| {
                    matches!(
                        k,
                        2 | 3 | 7 | 8 | 9 | 11 | 12 | 13 | 16 | 24 | 33 | 34 | 252
                    )
                });
            }
            !calcs.iter().any(|c| e.path.starts_with(c))
        });
        self.ensure(node.clone());
        self.set(node.clone(), 2, options.clone());
        if !spec.comment.is_empty() {
            self.set(node.clone(), 3, encode_text(&spec.comment));
        }
        if let FieldType::Summary {
            op,
            field,
            running,
            by,
        } = spec.field_type
        {
            self.set(node.clone(), 9, summary_block(op, field, running, by, ctx));
        }
        let a = &spec.auto_enter;
        if spec.field_type == FieldType::Normal {
            if let Some(s) = &a.serial {
                self.set(node.clone(), 12, encode_text(s));
            }
            if let Some(c) = &a.constant {
                self.set(node.clone(), 13, encode_text(c));
            }
            if let Some((lo, hi)) = &spec.validation.range {
                self.set(node.clone(), 33, encode_text(lo));
                self.set(node.clone(), 34, encode_text(hi));
            }
            if let Some(l) = &a.lookup {
                self.set(node.clone(), 8, lookup_block(l));
                if let NoMatch::Constant(c) = &l.no_match {
                    self.set(node.clone(), 11, encode_text(c));
                }
            }
            if let Some(vl) = spec.validation.value_list {
                self.set(node.clone(), 7, value_list_block(vl));
            }
            if let Some(m) = &spec.validation.message {
                self.set(node.clone(), 24, encode_text(m));
            }
        }
        self.set(node.clone(), 16, encode_text(&spec.name));
        let uuid = match uuid {
            Some(u) => u,
            None => self.uuid(),
        };
        self.set(node.clone(), 216, uuid);
        self.set(node.clone(), 252, len_int(count + 1));
        for (k, c) in [(5, &main), (25, &check), (32, &message)] {
            if let Some(c) = c {
                let mut p = node.clone();
                p.push(path_elem(k));
                self.set(p.clone(), 4, to_ref(ctx));
                self.set_long(p.clone(), 5, c.tokens.clone());
                self.set_long(p, 6, c.compiled.clone());
            }
        }
        // Type index [T].[3].[1].[6].
        let fc = Self::fields_cat(t);
        let tp = fc.sub(6);
        self.unindex(&tp, &id_ref(f));
        if let Some(key) =
            collate::field_type_key(options[0], options[1], spec.storage.global, &spec.name)
        {
            let taken = self
                .entries
                .iter()
                .any(|e| e.path == tp && entry_kvlong(e).is_some_and(|(k, _)| k == key));
            if !taken {
                self.ensure(tp.clone());
                self.entries.push(kvlong(tp, key, id_ref(f)));
            }
        }
        // Dependencies [T].[3].[1].[14]: [FFF0].[F] fields the value depends
        // on (01; 00 for the field a summary summarizes), FF00 = auto-enter,
        // FF02 = modification stamp; [FFF1].[F] fields its validation uses.
        let same_table = |c: &calc::Compiled| -> Vec<u32> {
            c.fields
                .iter()
                .filter(|(to, _)| {
                    names
                        .occurrences
                        .iter()
                        .any(|o| o.id == *to && o.base_table == t)
                })
                .map(|(_, fid)| *fid)
                .collect()
        };
        let mut deps: Vec<(u32, u8)> = Vec::new();
        // Keys naming a field through an occurrence: `00 TOref len field`.
        let mut long_deps: Vec<(Vec<u8>, u8)> = Vec::new();
        if spec.field_type == FieldType::Normal {
            let auto = a.stamp.is_some()
                || a.serial.is_some()
                || a.constant.is_some()
                || a.calculation.is_some();
            if auto {
                deps.push((32640, 1));
            }
            if a.stamp.is_some_and(|s| stamp_code(s) >= 5) {
                deps.push((32642, 1));
            }
        }
        if let Some(c) = &main {
            deps.extend(same_table(c).into_iter().map(|x| (x, 1)));
        }
        if let FieldType::Summary { field, op, by, .. } = spec.field_type {
            match by {
                Some(b) if op == SummaryOp::Average => {
                    deps.push((b, 2));
                    if b != field {
                        deps.push((field, 0));
                    }
                }
                Some(b) => {
                    deps.push((field, 0));
                    long_deps.push(([vec![0], to_ref(ctx), id_ref(b)].concat(), 2));
                }
                None => deps.push((field, 0)),
            }
        }
        let checks: Vec<(u32, u8)> = check
            .as_ref()
            .map(|c| same_table(c).into_iter().map(|x| (x, 1)).collect())
            .unwrap_or_default();
        // A lookup: empty node [14].[251].[F].[context TO].[lookup TO]
        // (FileMaker runs lookups from it; without it they never fire).
        let lp = lookup_deps(t, f);
        self.remove_tree(&lp);
        if let (FieldType::Normal, Some(l)) = (&spec.field_type, &a.lookup) {
            self.ensure([lp, vec![to_elem(l.context), to_elem(l.from)]].concat());
        }
        for (d, list, long) in [(32880, deps, long_deps), (32881, checks, Vec::new())] {
            let p = sp(&[t, 3, 1, 14, d, f]);
            self.remove_tree(&p);
            if !list.is_empty() || !long.is_empty() {
                self.ensure(p.clone());
                for (k, v) in list {
                    self.entries.push(kv(p.clone(), k, vec![v]));
                }
                for (k, v) in long {
                    self.entries.push(kvlong(p.clone(), k, vec![v]));
                }
            }
        }
        // Index statistics of an empty table: no entries, no distinct keys.
        let ip = sp(&[t, 1, f]);
        self.remove_tree(&ip);
        if !records && options[8] & 0xA8 != 0 {
            self.ensure(ip.clone());
            self.entries.push(kv(ip.clone(), 16, vec![0; 16]));
            if options[8] & 0x20 != 0 {
                self.entries.push(kv(ip, 17, vec![0; 16]));
            }
        }
        // The graph's dependency cache of the table: FileMaker rebuilds it.
        self.remove_tree(&sp(&[3, 17, 1, 14, t]));
        Ok(())
    }
}

/// A new, empty database: one table `table` (with its occurrence and an
/// empty layout) and no fields. Built from a blank file made with FileMaker
/// Pro 26 (`assets/blank.fmp12`, see ATTRIBUTION.md).
pub fn new_file(table: &str) -> Result<Vec<u8>, EditError> {
    let file = File::from_bytes(BLANK.to_vec())?;
    let mut ed = Editor::open(&file)?;
    let t = ed
        .table_id("Blank")
        .ok_or_else(|| EditError::NotFound("Blank".into()))?;
    if table != "Blank" {
        ed.rename_table(t, table)?;
    }
    Ok(ed.finish())
}

static BLANK: &[u8] = include_bytes!("../assets/blank.fmp12");
