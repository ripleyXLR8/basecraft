//! Schema editing ("Manage Database"): tables, their occurrences and
//! layouts, fields and field options, written into the file's catalogs
//! (FORMAT.md §14).
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
use crate::graph::TableOccurrence;
use crate::save::{encode_text, entry_kv, fresh_uuid, len_int, path_elem, read_len_int};
use crate::schema::{self, AutoStamp, DataType, Field, FieldKind, Indexing, SummaryOp};
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
    },
}

/// Auto-enter options of a normal field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutoEnter {
    pub stamp: Option<AutoStamp>,
    /// Serial number generated on creation: next value.
    /// ponytail: increment is always 1 (its storage is not decoded).
    pub serial: Option<String>,
    pub constant: Option<String>,
    pub calculation: Option<String>,
    /// Auto-enter calculation replaces an existing value.
    pub replace_existing: bool,
    /// Evaluate the calculation even when referenced fields are empty.
    pub always_evaluate: bool,
    pub prohibit_modification: bool,
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
            field_type: FieldType::Summary { op, field: of },
            ..FieldSpec::new(name, DataType::Number)
        }
    }
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
        o[10] = a.prohibit_modification as u8;
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
        o[14] = if v.always { 0x04 } else { 0 };
        o[15] = (v.calculation.is_some() as u8)
            | if v.allow_override { 0 } else { 0x04 }
            | if v.not_empty { 0x08 } else { 0 }
            | if v.unique { 0x10 } else { 0 }
            | if v.existing { 0x20 } else { 0 }
            | if v.range.is_some() { 0x40 } else { 0 };
    }
    o[19] = 1;
    o[24..26].copy_from_slice(&s.repetitions.max(1).to_be_bytes());
    o
}

/// Summary definition (key 9): operation, then the summarized field as a
/// reference `len 01 path-int` (FORMAT.md §5); the other bytes as FileMaker
/// writes them for a plain summary.
fn summary_block(op: SummaryOp, field: u32) -> Vec<u8> {
    let f = path_elem(field);
    let mut b = vec![op_code(op), 0, 2, 1, 0, 0, 0, 0, 1 + f.len() as u8, 1];
    b.extend(f);
    b.extend([0, 0, 0, 0, 1, 2, 1, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 1]);
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
                        let n = data.get(3).copied().unwrap_or(0) as usize;
                        o.base_table = data.get(5..4 + n).and_then(path_int).unwrap_or(0);
                        o.raw = data;
                    }
                }
                ([t, 3, 5, f], 16 | 2 | 9) if *t >= 128 => {
                    let x = fields.entry((*t, *f)).or_default();
                    match key {
                        16 => x.name = crate::text(&data),
                        2 => x.options = data,
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
        self.cat_add(&oc, to, name);
        // Key 2: 02 01 00, base table (len, 02, path int), position and size
        // (0: FileMaker places it), collapsed height, colour.
        let b = path_elem(t);
        let mut def = vec![2, 1, 0, 1 + b.len() as u8, 2];
        def.extend(b);
        def.extend([0u8; 24]);
        def.extend([0x77, 0x77, 0x77, 0xFF]);
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
            self.delete_occurrence(*to);
        }
        Ok(())
    }

    fn delete_occurrence(&mut self, to: u32) {
        self.remove_tree(&[sp(&[3, 17, 5]), vec![to_elem(to)]].concat());
        self.remove_tree(&[to_elem(to)]);
        self.cat_remove(&Self::occurrences_cat(), to);
        // Relationships that use it.
        let rels: Vec<u32> = self
            .entries
            .iter()
            .filter(|e| e.path.len() == 4 && e.path[..3] == sp(&[3, 251, 5])[..])
            .filter_map(|e| {
                let (k, d) = entry_kv(e)?;
                let r = path_int(&e.path[3])?;
                // Key 2: 04 TOref (left), 04 TOref (right), …
                let uses = |i: usize| d.get(i..i + 4).and_then(crate::graph::to_ref) == Some(to);
                (k == 2 && (uses(1) || uses(6))).then_some(r)
            })
            .collect();
        for r in rels {
            self.remove_tree(&sp(&[3, 251, 5, r]));
            let h = sp(&[3, 251, 1]);
            let n = self
                .get(&h, 4)
                .and_then(|d| d.try_into().ok().map(u32::from_be_bytes))
                .unwrap_or(1);
            self.set(h.clone(), 4, n.saturating_sub(1).to_be_bytes().to_vec());
            self.bump(h);
        }
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
        if let FieldType::Summary { field, .. } = spec.field_type
            && self.field_id_exists(t, field).is_none()
        {
            return Err(EditError::NotFound(format!("summarized field {field}")));
        }
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
        let records = self.has_records(t);
        let mut options = option_block(spec, self.language);
        // An existing field keeps its index language and the options
        // BaseCraft does not model (lookups, value-list and size
        // validation, strict data types, messages, furigana…).
        if let Some(old) = self.get(&node, 2).filter(|o| o.len() == 26) {
            const MANAGED: [u8; 26] = [
                0xFF, 0xFF, 0, 0xFF, 0, 0, 0, 0, 0xE8, 0x03, 0x01, 0xAF, 0, 0, 0x04, 0x7D, 0, 0, 0,
                0, 0, 0, 0, 0, 0xFF, 0xFF,
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
        let calcs = [5u32, 25].map(|k| [node.clone(), vec![path_elem(k)]].concat());
        self.entries.retain(|e| {
            if !e.path.starts_with(&node) {
                return true;
            }
            if e.path.len() == node.len() {
                return !entry_kv(e)
                    .is_some_and(|(k, _)| matches!(k, 2 | 3 | 9 | 12 | 13 | 16 | 33 | 34 | 252));
            }
            !calcs.iter().any(|c| e.path.starts_with(c))
        });
        self.ensure(node.clone());
        self.set(node.clone(), 2, options.clone());
        if !spec.comment.is_empty() {
            self.set(node.clone(), 3, encode_text(&spec.comment));
        }
        if let FieldType::Summary { op, field } = spec.field_type {
            self.set(node.clone(), 9, summary_block(op, field));
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
        }
        self.set(node.clone(), 16, encode_text(&spec.name));
        let uuid = match uuid {
            Some(u) => u,
            None => self.uuid(),
        };
        self.set(node.clone(), 216, uuid);
        self.set(node.clone(), 252, len_int(count + 1));
        for (k, c) in [(5, &main), (25, &check)] {
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
        if let FieldType::Summary { field, .. } = spec.field_type {
            deps.push((field, 0));
        }
        let checks: Vec<(u32, u8)> = check
            .as_ref()
            .map(|c| same_table(c).into_iter().map(|x| (x, 1)).collect())
            .unwrap_or_default();
        for (d, list) in [(32880, deps), (32881, checks)] {
            let p = sp(&[t, 3, 1, 14, d, f]);
            self.remove_tree(&p);
            if !list.is_empty() {
                self.ensure(p.clone());
                for (k, v) in list {
                    self.entries.push(kv(p.clone(), k, vec![v]));
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

    fn field_id_exists(&self, t: u32, f: u32) -> Option<()> {
        self.names()
            .tables
            .iter()
            .find(|x| x.id == t)?
            .fields
            .contains_key(&f)
            .then_some(())
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
