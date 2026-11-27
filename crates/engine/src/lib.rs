//! BaseCraft data engine: an in-memory database loaded from a `.fmp12`
//! file, with FileMaker's record, found-set, find, sort and calculation
//! semantics.

pub mod export;
mod find;
mod host;
pub mod import;
pub mod summary;
mod xlsx;

use std::collections::BTreeMap;

use calc::Value;
use fmp12::schema::{DataType, FieldKind};
use fmp12::schema_edit::{EditError, Editor, FieldSpec};

pub use find::{Request, find};
pub use summary::{groups, summarize};

#[derive(Debug, Clone)]
pub struct Field {
    pub id: u32,
    pub name: String,
    pub kind: FieldKind,
    pub data_type: DataType,
    /// Calculation source for calculated fields, in a form `calc` parses
    /// (field names wrapped as `${…}`).
    pub calc: Option<String>,
    /// Auto-enter calculation of a normal field.
    pub auto_calc: Option<String>,
    /// Table occurrence a calculation is evaluated from (0 if none).
    pub context: u32,
    pub def: fmp12::schema::Field,
}

#[derive(Debug, Clone, Default)]
pub struct Record {
    pub id: u32,
    /// Stored values as text, by field id.
    pub values: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub id: u32,
    pub name: String,
    pub fields: Vec<Field>,
    pub records: Vec<Record>,
    next_record_id: u32,
}

impl Table {
    /// An empty table (databases built in code, tests).
    pub fn new(id: u32, name: &str, fields: Vec<Field>) -> Table {
        Table {
            id,
            name: name.to_string(),
            fields,
            records: Vec::new(),
            next_record_id: 0,
        }
    }

    pub fn field(&self, id: u32) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// New Record: returns its index.
    pub fn new_record(&mut self) -> usize {
        self.next_record_id += 1;
        self.records.push(Record {
            id: self.next_record_id,
            values: BTreeMap::new(),
        });
        self.records.len() - 1
    }
}

#[derive(Debug, Clone)]
pub struct Database {
    pub tables: Vec<Table>,
    pub occurrences: Vec<fmp12::graph::TableOccurrence>,
    pub relationships: Vec<fmp12::graph::Relationship>,
    pub layouts: Vec<fmp12::layouts::Layout>,
    pub themes: Vec<fmp12::theme::Theme>,
    pub value_lists: Vec<fmp12::valuelists::ValueList>,
    pub accounts: Vec<fmp12::security::Account>,
    pub privilege_sets: Vec<fmp12::security::PrivilegeSet>,
    /// Formats the file stores dates, times and numbers in.
    pub locale: fmp12::schema::Locale,
}

/// Names for decompiling calculations into `calc`-parsable source.
struct EvalNames<'a> {
    tables: &'a [fmp12::schema::Table],
    tos: &'a [fmp12::graph::TableOccurrence],
}

impl fmp12::calc::Names for EvalNames<'_> {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        let bt = self.tos.iter().find(|t| t.id == to)?.base_table;
        let t = self.tables.iter().find(|t| t.id == bt)?;
        Some(format!("${{{}}}", t.fields.get(&field)?.name))
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(format!(
            "${{{}}}",
            self.tos.iter().find(|t| t.id == to)?.name
        ))
    }
}

impl Database {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, fmp12::Error> {
        Self::load(&fmp12::File::open(path)?)
    }

    pub fn load(file: &fmp12::File) -> Result<Self, fmp12::Error> {
        let schema = fmp12::schema::tables(file)?;
        let occurrences = fmp12::graph::table_occurrences(file)?;
        // Calculation token streams of calculated fields: (table, field) -> (key 4, key 5).
        let mut streams: BTreeMap<(u32, u32), (Vec<u8>, Vec<u8>)> = BTreeMap::new();
        file.walk(|path, c| {
            let (Some(p), fmp12::Chunk::Kv { key, data }) = (fmp12::int_path(path), c) else {
                return;
            };
            if let [t, 3, 5, f, 5] = p.as_slice() {
                let e = streams.entry((*t, *f)).or_default();
                match key {
                    4 => e.0 = data.to_vec(),
                    5 => e.1.extend_from_slice(data),
                    _ => {}
                }
            }
        })?;
        let names = EvalNames {
            tables: &schema,
            tos: &occurrences,
        };
        let mut tables = Vec::new();
        for t in &schema {
            let fields = t
                .fields
                .iter()
                .map(|(id, f)| {
                    let stream = streams.get(&(t.id, *id));
                    let context = stream
                        .and_then(|(k4, _)| fmp12::calc::context(k4))
                        .unwrap_or(0);
                    let source =
                        stream.and_then(|(_, k5)| fmp12::calc::decompile(k5, context, &names).ok());
                    // The same node holds a calculated field's formula or an auto-enter calculation.
                    let (calc, auto_calc) = if f.kind() == FieldKind::Calculated {
                        (source, None)
                    } else {
                        (None, source.filter(|_| f.auto_calculation()))
                    };
                    Field {
                        id: *id,
                        name: f.name.clone(),
                        kind: f.kind(),
                        data_type: f.data_type(),
                        calc,
                        auto_calc,
                        context,
                        def: f.clone(),
                    }
                })
                .collect();
            let records: Vec<Record> = fmp12::schema::records(file, t)?
                .into_iter()
                .map(|(id, values)| Record { id, values })
                .collect();
            let last = fmp12::save::last_record_id(file, t.id)?;
            tables.push(Table {
                id: t.id,
                name: t.name.clone(),
                // FileMaker never reuses the id of a deleted record.
                next_record_id: records.iter().map(|r| r.id).fold(last, u32::max),
                fields,
                records,
            });
        }
        Ok(Database {
            tables,
            relationships: fmp12::graph::relationships(file)?,
            layouts: fmp12::layouts::layouts(file)?,
            themes: fmp12::theme::themes(file)?,
            value_lists: fmp12::valuelists::value_lists(file)?,
            accounts: fmp12::security::accounts(file)?,
            privilege_sets: fmp12::security::privilege_sets(file)?,
            locale: fmp12::schema::locale(file)?,
            occurrences,
        })
    }

    pub fn table(&self, id: u32) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
    }

    /// Applies a schema edit ("Manage Database") to `file`, which must be
    /// the file this database was loaded from, and reloads the schema
    /// (tables, fields, occurrences, layouts). Records not saved yet are
    /// kept; values of deleted fields and tables are dropped.
    pub fn edit_schema<T>(
        &mut self,
        file: &mut fmp12::File,
        edit: impl FnOnce(&mut Editor) -> Result<T, EditError>,
    ) -> Result<T, EditError> {
        let mut ed = Editor::open(file)?;
        let out = edit(&mut ed)?;
        let new = fmp12::File::from_bytes(ed.finish())?;
        let mut fresh = Database::load(&new)?;
        for t in fresh.tables.iter_mut() {
            let Some(old) = self.tables.iter_mut().find(|o| o.id == t.id) else {
                continue;
            };
            let ids: std::collections::BTreeSet<u32> = t.fields.iter().map(|f| f.id).collect();
            t.records = std::mem::take(&mut old.records);
            for r in t.records.iter_mut() {
                r.values.retain(|f, _| ids.contains(f));
            }
            t.next_record_id = t.next_record_id.max(old.next_record_id);
        }
        *self = fresh;
        *file = new;
        Ok(out)
    }

    /// Adds a table (with its occurrence and layout); returns its id.
    pub fn add_table(&mut self, file: &mut fmp12::File, name: &str) -> Result<u32, EditError> {
        self.edit_schema(file, |ed| ed.add_table(name).map(|(t, _, _)| t))
    }

    /// Adds a field to table `table`; returns its id.
    pub fn add_field(
        &mut self,
        file: &mut fmp12::File,
        table: u32,
        spec: &FieldSpec,
    ) -> Result<u32, EditError> {
        self.edit_schema(file, |ed| ed.add_field(table, spec))
    }

    /// Replaces a field's definition (name, type, options).
    pub fn set_field(
        &mut self,
        file: &mut fmp12::File,
        table: u32,
        field: u32,
        spec: &FieldSpec,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.set_field(table, field, spec))
    }

    pub fn rename_table(
        &mut self,
        file: &mut fmp12::File,
        table: u32,
        name: &str,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.rename_table(table, name))
    }

    pub fn delete_table(&mut self, file: &mut fmp12::File, table: u32) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.delete_table(table))
    }

    pub fn rename_field(
        &mut self,
        file: &mut fmp12::File,
        table: u32,
        field: u32,
        name: &str,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.rename_field(table, field, name))
    }

    pub fn delete_field(
        &mut self,
        file: &mut fmp12::File,
        table: u32,
        field: u32,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.delete_field(table, field))
    }

    /// The bytes of `original` (the file this database was loaded from)
    /// with the current records of every table: new, edited and deleted
    /// records are written, everything else is kept. Stored calculations
    /// of new and edited records are evaluated first, as FileMaker does on
    /// commit (it does not recompute them when opening a file).
    /// ponytail: results are written in US formats (`Value::to_text`), and
    /// auto-enter options (serials, UUIDs, stamps) are not applied yet.
    pub fn save(&self, original: &fmp12::File) -> Result<Vec<u8>, fmp12::Error> {
        let schema = fmp12::schema::tables(original)?;
        let mut data = BTreeMap::new();
        for t in &self.tables {
            let old = match schema.iter().find(|s| s.id == t.id) {
                Some(s) => fmp12::schema::records(original, s)?,
                None => Default::default(),
            };
            let calcs: Vec<&Field> = t
                .fields
                .iter()
                .filter(|f| f.calc.is_some() && f.def.stored() && !f.def.global())
                .collect();
            let input = |v: &BTreeMap<u32, String>| -> BTreeMap<u32, String> {
                v.iter()
                    .filter(|(f, s)| !s.is_empty() && !calcs.iter().any(|c| c.id == **f))
                    .map(|(f, s)| (*f, s.clone()))
                    .collect()
            };
            let mut recs = BTreeMap::new();
            for (i, r) in t.records.iter().enumerate() {
                let mut values = r.values.clone();
                if old.get(&r.id).is_none_or(|o| input(o) != input(&values)) {
                    for c in &calcs {
                        values.insert(c.id, stored_text(&self.value(t.id, i, c.id), &self.locale));
                    }
                }
                recs.insert(r.id, values);
            }
            data.insert(t.id, recs);
        }
        let bytes = fmp12::save::save_records(original, &data)?;
        // Serial numbers: the file's next value moves past the numbers
        // given here, so that FileMaker does not hand them out again.
        let saved = fmp12::File::from_bytes(bytes)?;
        let mut ed = Editor::open(&saved)?;
        let mut moved = false;
        for t in &self.tables {
            for f in t.fields.iter().filter(|f| f.def.auto_serial()) {
                let max = t
                    .records
                    .iter()
                    .filter_map(|r| r.values.get(&f.id)?.trim().parse::<u64>().ok())
                    .max();
                let next = ed
                    .next_serial(t.id, f.id)
                    .and_then(|s| s.trim().parse::<u64>().ok());
                if let (Some(max), Some(next)) = (max, next)
                    && max >= next
                {
                    ed.set_next_serial(t.id, f.id, &(max + 1).to_string());
                    moved = true;
                }
            }
        }
        Ok(if moved {
            ed.finish()
        } else {
            saved.bytes().to_vec()
        })
    }

    /// Base table of a table occurrence.
    pub fn table_of(&self, to: u32) -> Option<&Table> {
        self.table(self.occurrences.iter().find(|o| o.id == to)?.base_table)
    }

    /// Typed value of a field in a record (calculated fields are evaluated).
    pub fn value(&self, table: u32, record: usize, field: u32) -> Value {
        let Some(t) = self.table(table) else {
            return Value::empty();
        };
        let Some(f) = t.field(field) else {
            return Value::empty();
        };
        if f.kind == FieldKind::Summary {
            // ponytail: the engine has no found set, so a summary field
            // summarizes the whole table (Show All Records); O(n) per call.
            let all: Vec<usize> = (0..t.records.len()).collect();
            return summarize(self, table, field, &all, &all);
        }
        if let Some(src) = &f.calc {
            let mut ctx = RecordContext {
                db: self,
                table,
                to: f.context,
                record,
                vars: BTreeMap::new(),
                env: None,
            };
            return calc::evaluate(src, &mut ctx).unwrap_or_else(|_| Value::text("?"));
        }
        let raw = t
            .records
            .get(record)
            .and_then(|r| r.values.get(&field))
            .map_or("", |s| s.as_str());
        typed_in(raw, f.data_type, &self.locale)
    }
}

impl Database {
    /// Values of value list `id`: custom values, or the distinct sorted
    /// values of a field.
    pub fn value_list_values(&self, id: u32) -> Vec<String> {
        use fmp12::valuelists::Source;
        let Some(vl) = self.value_lists.iter().find(|v| v.id == id) else {
            return Vec::new();
        };
        match &vl.source {
            Source::Custom(text) => text
                .split('\r')
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect(),
            Source::Field { to, field } => {
                let Some(t) = self.table_of(*to) else {
                    return Vec::new();
                };
                let mut v: Vec<String> = (0..t.records.len())
                    .map(|r| self.value(t.id, r, *field).to_text())
                    .filter(|s| !s.is_empty())
                    .collect();
                v.sort_by_key(|s| s.to_lowercase());
                v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                v
            }
            Source::Unknown(_) => Vec::new(),
        }
    }

    /// Evaluates calculation source text for `record` of `table`, from
    /// occurrence `to` (Data Viewer style).
    pub fn eval_source(&self, src: &str, table: u32, to: u32, record: Option<usize>) -> Value {
        let mut ctx = RecordContext {
            db: self,
            table,
            to,
            record: record.unwrap_or(usize::MAX),
            vars: BTreeMap::new(),
            env: None,
        };
        calc::evaluate(src, &mut ctx).unwrap_or_else(|_| Value::text("?"))
    }

    /// Evaluates a calculation stored as a token stream (layout labels,
    /// script parameters…) for `record` of `table`, from occurrence `to`.
    pub fn eval_tokens(&self, tokens: &[u8], table: u32, to: u32, record: Option<usize>) -> Value {
        self.eval_tokens_in(tokens, table, to, record, None)
            .unwrap_or_else(|_| Value::text("?"))
    }

    /// Like [`Database::eval_tokens`], with a script's variables and `Get`
    /// flags (`env`), and the error FileMaker would report.
    pub fn eval_tokens_in(
        &self,
        tokens: &[u8],
        table: u32,
        to: u32,
        record: Option<usize>,
        env: Option<&mut dyn Env>,
    ) -> Result<Value, calc::EvalError> {
        let schema: Vec<fmp12::schema::Table> = self
            .tables
            .iter()
            .map(|t| fmp12::schema::Table {
                id: t.id,
                name: t.name.clone(),
                fields: t.fields.iter().map(|f| (f.id, f.def.clone())).collect(),
            })
            .collect();
        let names = EvalNames {
            tables: &schema,
            tos: &self.occurrences,
        };
        let src = fmp12::calc::decompile(tokens, to, &names)
            .map_err(|e| calc::EvalError::new(calc::error::code::GENERIC, e.to_string()))?;
        let mut ctx = RecordContext {
            db: self,
            table,
            to,
            record: record.unwrap_or(usize::MAX),
            vars: BTreeMap::new(),
            env,
        };
        calc::evaluate(&src, &mut ctx)
    }

    /// New Record with FileMaker's auto-enter options applied (creation
    /// stamps, serial numbers, auto-enter calculations). Returns its index.
    pub fn new_record(&mut self, table: u32) -> Option<usize> {
        let ti = self.tables.iter().position(|t| t.id == table)?;
        let r = self.tables[ti].new_record();
        let fields = self.tables[ti].fields.clone();
        for f in &fields {
            if let Some(v) = self.auto_value(table, r, f, true) {
                self.tables[ti].records[r].values.insert(f.id, v);
            }
        }
        self.touch(table, r);
        Some(r)
    }

    /// Applies modification stamps after a record changed.
    pub fn touch(&mut self, table: u32, record: usize) {
        let Some(ti) = self.tables.iter().position(|t| t.id == table) else {
            return;
        };
        let fields = self.tables[ti].fields.clone();
        for f in fields.iter().filter(|f| {
            f.def
                .auto_stamp()
                .is_some_and(|s| (s as u8) >= fmp12::schema::AutoStamp::ModificationDate as u8)
        }) {
            if let Some(v) = self.auto_value(table, record, f, false) {
                self.tables[ti].records[record].values.insert(f.id, v);
            }
        }
    }

    fn auto_value(&self, table: u32, record: usize, f: &Field, creating: bool) -> Option<String> {
        use fmp12::schema::AutoStamp::*;
        let now = |what: &str| {
            let v = self.eval_source(&format!("Get ( {what} )"), table, f.context, Some(record));
            stored_text(&v, &self.locale)
        };
        if let Some(stamp) = f.def.auto_stamp() {
            let is_creation = (stamp as u8) < ModificationDate as u8;
            if is_creation != creating {
                return None;
            }
            return Some(match stamp {
                CreationDate | ModificationDate => now("CurrentDate"),
                CreationTime | ModificationTime => now("CurrentTime"),
                CreationTimestamp | ModificationTimestamp => now("CurrentTimestamp"),
                // ponytail: no accounts yet; FileMaker's default account is "Admin".
                CreationName | ModificationName | CreationAccountName | ModificationAccountName => {
                    "Admin".to_string()
                }
            });
        }
        if !creating {
            return None;
        }
        if f.def.auto_serial() {
            // ponytail: next serial = highest existing value + 1 (stored next value not decoded).
            let t = self.table(table)?;
            let max = t
                .records
                .iter()
                .filter_map(|r| r.values.get(&f.id)?.trim().parse::<i64>().ok())
                .max()
                .unwrap_or(0);
            return Some((max + 1).to_string());
        }
        let src = f.auto_calc.as_ref()?;
        let v = self
            .eval_source(src, table, f.context, Some(record))
            .to_text();
        Some(v)
    }

    /// Checks a record against its fields' validation options, as FileMaker
    /// does when the record is committed. Returns the failing fields.
    pub fn validate(&self, table: u32, record: usize) -> Vec<Invalid> {
        let Some(t) = self.table(table) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for f in &t.fields {
            if f.kind != FieldKind::Normal {
                continue;
            }
            let v = f.def.validation();
            let raw = t.records.get(record).and_then(|r| r.values.get(&f.id));
            let text = raw.map_or("", |s| s.as_str());
            if v.not_empty && text.trim().is_empty() {
                out.push(Invalid {
                    field: f.id,
                    rule: Rule::NotEmpty,
                    can_override: v.allow_override,
                });
                continue;
            }
            if text.is_empty() {
                continue;
            }
            let same = |o: &Record| {
                o.values
                    .get(&f.id)
                    .is_some_and(|x| x.eq_ignore_ascii_case(text))
            };
            let others = t.records.iter().enumerate().filter(|(i, _)| *i != record);
            if v.unique && others.clone().any(|(_, o)| same(o)) {
                out.push(Invalid {
                    field: f.id,
                    rule: Rule::Unique,
                    can_override: v.allow_override,
                });
            } else if v.existing && !others.clone().any(|(_, o)| same(o)) {
                out.push(Invalid {
                    field: f.id,
                    rule: Rule::Existing,
                    can_override: v.allow_override,
                });
            } else if v.strict_numeric && calc::value::text_to_num(text).is_none() {
                out.push(Invalid {
                    field: f.id,
                    rule: Rule::Numeric,
                    can_override: v.allow_override,
                });
            }
        }
        out
    }

    fn to_named(&self, name: &str) -> Option<&fmp12::graph::TableOccurrence> {
        self.occurrences
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
    }

    /// Records related to record `record` of table occurrence `from`, seen
    /// through table occurrence `to` (following the shortest relationship path).
    pub fn related(&self, from: u32, record: usize, to: u32) -> Vec<usize> {
        // Breadth-first search over the relationship graph.
        let mut prev: BTreeMap<u32, (u32, usize)> = BTreeMap::new();
        let mut queue = std::collections::VecDeque::from([from]);
        while let Some(a) = queue.pop_front() {
            if a == to {
                break;
            }
            for (i, r) in self.relationships.iter().enumerate() {
                let b = if r.left == a {
                    r.right
                } else if r.right == a {
                    r.left
                } else {
                    continue;
                };
                if b != from && !prev.contains_key(&b) {
                    prev.insert(b, (a, i));
                    queue.push_back(b);
                }
            }
        }
        let mut hops = Vec::new();
        let mut cur = to;
        while cur != from {
            let Some(&(a, i)) = prev.get(&cur) else {
                return Vec::new();
            };
            hops.push((a, cur, i));
            cur = a;
        }
        let mut set = vec![record];
        for (a, b, i) in hops.into_iter().rev() {
            set = self.hop(a, &set, b, &self.relationships[i]);
        }
        set
    }

    fn hop(
        &self,
        a: u32,
        records: &[usize],
        b: u32,
        rel: &fmp12::graph::Relationship,
    ) -> Vec<usize> {
        let (Some(ta), Some(tb)) = (self.table_of(a), self.table_of(b)) else {
            return Vec::new();
        };
        // Predicate fields oriented from a to b.
        let preds: Vec<(u32, u32)> = rel
            .predicates
            .iter()
            .map(|p| {
                if rel.left == a {
                    (p.left_field, p.right_field)
                } else {
                    (p.right_field, p.left_field)
                }
            })
            .collect();
        (0..tb.records.len())
            .filter(|&j| {
                records.iter().any(|&r| {
                    preds.iter().all(|(fa, fb)| {
                        let (va, vb) = (self.value(ta.id, r, *fa), self.value(tb.id, j, *fb));
                        // ponytail: only `=` joins; other operators need relationship option decoding.
                        !va.is_empty()
                            && !vb.is_empty()
                            && calc::eval::compare(&va, &vb) == std::cmp::Ordering::Equal
                    })
                })
            })
            .collect()
    }
}

/// Converts text stored in a file with the given formats to a typed value:
/// FileMaker stores `14/07/2025` in a French file, `7/14/2025` in an
/// English one.
/// Formats a value the way FileMaker stores it in this file's formats
/// (`2/1/2026 2:13:33 PM` in an English file, `01/02/2026 14:35:26` in a
/// French one).
pub fn stored_text(v: &Value, loc: &fmp12::schema::Locale) -> String {
    use fmp12::schema::DateOrder;
    // ponytail: zero padding follows the observed samples (US unpadded, D/M/Y padded).
    let pad = loc.date_order != DateOrder::Mdy;
    let date = |day: i64| {
        let (y, m, d) = calc::value::civil(day);
        let (m, d) = if pad {
            (format!("{m:02}"), format!("{d:02}"))
        } else {
            (m.to_string(), d.to_string())
        };
        let sep = loc.date_separator;
        match loc.date_order {
            DateOrder::Mdy => format!("{m}{sep}{d}{sep}{y}"),
            DateOrder::Dmy => format!("{d}{sep}{m}{sep}{y}"),
            DateOrder::Ymd => format!("{y}{sep}{m}{sep}{d}"),
        }
    };
    let time = |t: &calc::num::Num| {
        use bigdecimal::ToPrimitive;
        let secs = t.to_f64().unwrap_or(0.0).round() as i64;
        let (h, mi, se) = (secs / 3600, secs / 60 % 60, secs % 60);
        let sep = loc.time_separator;
        if loc.am.is_empty() {
            format!("{h:02}{sep}{mi:02}{sep}{se:02}")
        } else {
            let (h12, suffix) = match h {
                0 => (12, &loc.am),
                1..=11 => (h, &loc.am),
                12 => (12, &loc.pm),
                _ => (h - 12, &loc.pm),
            };
            format!("{h12}{sep}{mi:02}{sep}{se:02}{suffix}")
        }
    };
    match v {
        Value::Date(d) => date(*d),
        Value::Time(t) => time(t),
        Value::Timestamp(ts) => format!(
            "{} {}",
            date(calc::value::ts_date(ts)),
            time(&calc::value::ts_time(ts))
        ),
        Value::Number(_) if loc.decimal != '.' => {
            v.to_text().replace('.', &loc.decimal.to_string())
        }
        _ => v.to_text(),
    }
}

pub fn typed_in(raw: &str, dt: DataType, loc: &fmp12::schema::Locale) -> Value {
    typed(&to_us(raw, dt, loc), dt)
}

/// Rewrites a stored value from the file's formats into the US formats
/// `typed` reads (M/D/Y dates, `.` decimal point, AM/PM).
fn to_us(raw: &str, dt: DataType, loc: &fmp12::schema::Locale) -> String {
    use fmp12::schema::DateOrder;
    let date = |d: &str| {
        let g: Vec<&str> = d
            .split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .collect();
        match (loc.date_order, g.as_slice()) {
            (DateOrder::Dmy, [d, m, y]) => format!("{m}/{d}/{y}"),
            (DateOrder::Ymd, [y, m, d]) => format!("{m}/{d}/{y}"),
            _ => d.to_string(),
        }
    };
    let time = |t: &str| {
        let t = t.trim();
        for (suffix, us) in [(&loc.am, "AM"), (&loc.pm, "PM")] {
            let s = suffix.trim();
            if let Some(rest) = t.strip_suffix(s).filter(|_| !s.is_empty()) {
                return format!("{} {us}", rest.trim_end());
            }
        }
        t.to_string()
    };
    match dt {
        DataType::Number if loc.decimal != '.' => raw
            .chars()
            .filter(|c| *c != '.')
            .map(|c| if c == loc.decimal { '.' } else { c })
            .collect(),
        DataType::Date => date(raw),
        DataType::Time => time(raw),
        DataType::Timestamp => match raw.trim().split_once(' ') {
            Some((d, t)) => format!("{} {}", date(d), time(t)),
            None => date(raw),
        },
        _ => raw.to_string(),
    }
}

/// Converts stored text in US formats to a typed value.
pub fn typed(raw: &str, dt: DataType) -> Value {
    if raw.is_empty() {
        return Value::empty();
    }
    match dt {
        DataType::Number => {
            calc::value::text_to_num(raw).map_or_else(|| Value::text(raw), Value::Number)
        }
        DataType::Date => {
            calc::value::parse_date(raw).map_or_else(|| Value::text(raw), Value::Date)
        }
        DataType::Time => {
            calc::value::parse_time(raw).map_or_else(|| Value::text(raw), Value::Time)
        }
        DataType::Timestamp => {
            calc::value::parse_timestamp(raw).map_or_else(|| Value::text(raw), Value::Timestamp)
        }
        _ => Value::text(raw),
    }
}

/// What a running script adds to calculations: its `$local` and `$$global`
/// variables and `Get` flags (ScriptParameter, LastError, FoundCount…).
pub trait Env {
    fn variable(&mut self, name: &str, repetition: usize) -> Value;
    fn set_variable(&mut self, name: &str, repetition: usize, value: Value);
    /// `None` falls back to the calculation engine's built-in flags.
    fn get(&mut self, flag: &str) -> Option<Value>;
}

/// Evaluation context bound to one record.
struct RecordContext<'a, 'e> {
    db: &'a Database,
    table: u32,
    /// Context table occurrence of the calculation.
    to: u32,
    record: usize,
    vars: BTreeMap<(String, usize), Value>,
    env: Option<&'e mut dyn Env>,
}

impl calc::Context for RecordContext<'_, '_> {
    fn field(
        &mut self,
        table: Option<&str>,
        name: &str,
        _repetition: usize,
    ) -> Result<Value, calc::EvalError> {
        Ok(self
            .field_values(table, name)?
            .into_iter()
            .next()
            .unwrap_or_else(Value::empty))
    }

    fn field_values(
        &mut self,
        table: Option<&str>,
        name: &str,
    ) -> Result<Vec<Value>, calc::EvalError> {
        let missing = |what: &str| calc::EvalError::new(102, format!("{what} missing"));
        let (t, records) = match table {
            Some(to_name) => {
                let to = self.db.to_named(to_name).ok_or_else(|| missing(to_name))?;
                if to.id == self.to || (self.to == 0 && to.base_table == self.table) {
                    (to.base_table, vec![self.record])
                } else {
                    (to.base_table, self.db.related(self.to, self.record, to.id))
                }
            }
            None => (self.table, vec![self.record]),
        };
        let tbl = self.db.table(t).ok_or_else(|| missing("table"))?;
        let f = tbl.field_by_name(name).ok_or_else(|| missing(name))?;
        Ok(records
            .into_iter()
            .map(|r| self.db.value(t, r, f.id))
            .collect())
    }

    fn schema(&mut self, file: &str) -> Option<std::sync::Arc<calc::schema::Schema>> {
        // ponytail: only the current file is known; rebuilt on each call.
        (file.is_empty()).then(|| std::sync::Arc::new(self.db.calc_schema("")))
    }

    fn current_occurrence(&mut self) -> Option<String> {
        self.db
            .occurrences
            .iter()
            .find(|o| o.id == self.to)
            .map(|o| o.name.clone())
    }

    fn nth_record(
        &mut self,
        table: Option<&str>,
        name: &str,
        _repetition: usize,
        n: usize,
    ) -> Result<Value, calc::EvalError> {
        // ponytail: n-th record of the whole table (no found sets in the engine yet).
        Ok(self
            .field_values(table, name)
            .ok()
            .filter(|_| table.is_some())
            .and_then(|v| v.into_iter().nth(n.saturating_sub(1)))
            .or_else(|| {
                let t = self.db.table(self.table)?;
                let f = t.field_by_name(name)?;
                (n >= 1 && n <= t.records.len()).then(|| self.db.value(t.id, n - 1, f.id))
            })
            .unwrap_or_else(Value::empty))
    }

    fn summary(
        &mut self,
        summary: (Option<&str>, &str),
        break_field: (Option<&str>, &str),
    ) -> Result<Value, calc::EvalError> {
        // ponytail: the found set is the whole table (no found set in the engine).
        let t = self
            .db
            .table(self.table)
            .ok_or_else(|| calc::EvalError::new(106, "table missing"))?;
        let field = |n: &str| {
            t.field_by_name(n)
                .map(|f| f.id)
                .ok_or_else(|| calc::EvalError::new(102, format!("field {n} missing")))
        };
        let (sf, bf) = (field(summary.1)?, field(break_field.1)?);
        let found: Vec<usize> = (0..t.records.len()).collect();
        if sf == bf {
            return Ok(summary::summarize(self.db, t.id, sf, &found, &found));
        }
        // Records sharing this record's break value, in table order.
        let mine = self.db.value(t.id, self.record, bf);
        let group: Vec<usize> = found
            .iter()
            .copied()
            .filter(|&r| {
                calc::eval::compare(&self.db.value(t.id, r, bf), &mine) == std::cmp::Ordering::Equal
            })
            .collect();
        Ok(summary::summarize(self.db, t.id, sf, &group, &found))
    }

    fn record_ids(&mut self, _table: Option<&str>) -> Result<Vec<u64>, calc::EvalError> {
        Ok(self
            .db
            .table(self.table)
            .map(|t| t.records.iter().map(|r| r.id as u64).collect())
            .unwrap_or_default())
    }

    fn rows(&mut self, table: &str) -> Result<calc::schema::Rows, calc::EvalError> {
        self.db
            .calc_rows(table)
            .ok_or_else(|| calc::EvalError::new(106, format!("table {table} missing")))
    }

    fn get(&mut self, flag: &str) -> Option<Value> {
        self.env.as_mut()?.get(flag)
    }

    fn variable(&mut self, name: &str, repetition: usize) -> Value {
        if let Some(e) = self.env.as_mut() {
            return e.variable(name, repetition);
        }
        self.vars
            .get(&(name.to_string(), repetition))
            .cloned()
            .unwrap_or_else(Value::empty)
    }

    fn set_variable(&mut self, name: &str, repetition: usize, value: Value) {
        if let Some(e) = self.env.as_mut() {
            return e.set_variable(name, repetition, value);
        }
        self.vars.insert((name.to_string(), repetition), value);
    }
}

/// A failed validation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    NotEmpty,
    Unique,
    Existing,
    Numeric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid {
    pub field: u32,
    pub rule: Rule,
    /// The user may accept the value anyway ("Allow user to override").
    pub can_override: bool,
}

/// FileMaker's sort order for one field: ascending or descending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Ascending,
    Descending,
}

/// Sorts a found set (record indexes) by fields of `table`.
pub fn sort(db: &Database, table: u32, found: &mut [usize], keys: &[(u32, Order)]) {
    found.sort_by(|&a, &b| {
        for (f, o) in keys {
            let (va, vb) = (db.value(table, a, *f), db.value(table, b, *f));
            // Empty values sort first in ascending order.
            let c = match (va.is_empty(), vb.is_empty()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => calc::eval::compare(&va, &vb),
            };
            let c = if *o == Order::Descending {
                c.reverse()
            } else {
                c
            };
            if c != std::cmp::Ordering::Equal {
                return c;
            }
        }
        std::cmp::Ordering::Equal
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use fmp12::schema::{DateOrder, Locale};

    pub const T: u32 = 130;
    pub const PETS: u32 = 131;
    pub const NAME: u32 = 1;
    pub const AMOUNT: u32 = 2;
    pub const CITY: u32 = 3;
    pub const BORN: u32 = 4;
    pub const NOTE: u32 = 5;

    /// `options` byte 0 = kind, 1 = data type.
    pub fn field(id: u32, name: &str, kind: u8, dt: u8) -> Field {
        let def = fmp12::schema::Field {
            name: name.into(),
            options: vec![kind, dt],
            ..Default::default()
        };
        Field {
            id,
            name: name.into(),
            kind: def.kind(),
            data_type: def.data_type(),
            calc: None,
            auto_calc: None,
            context: 0,
            def,
        }
    }

    /// People (Name, Amount, City, Born, Note) and their Pets, related by name.
    pub fn sample() -> Database {
        let mut t = Table::new(
            T,
            "People",
            vec![
                field(NAME, "Name", 1, 1),
                field(AMOUNT, "Amount", 1, 2),
                field(CITY, "City", 1, 1),
                field(BORN, "Born", 1, 3),
                field(NOTE, "Note", 1, 1),
            ],
        );
        let rows = [
            ["Ada", "10", "Rome", "12/10/1815", "line 1\rline 2"],
            ["Bob \"B\"", "20", "Paris", "", ""],
            ["Cy", "30", "Rome", "", "a,b"],
            ["Di", "40", "Oslo", "", "tab\there"],
            ["Ed", "", "Rome", "", ""],
        ];
        for row in rows {
            let r = t.new_record();
            for (i, v) in row.iter().enumerate() {
                if !v.is_empty() {
                    t.records[r].values.insert(i as u32 + 1, v.to_string());
                }
            }
        }
        let mut pets = Table::new(
            PETS,
            "Pets",
            vec![field(1, "Owner", 1, 1), field(2, "Pet", 1, 1)],
        );
        for (o, p) in [("Ada", "Cat"), ("Ada", "Dog"), ("Cy", "Fish")] {
            let r = pets.new_record();
            pets.records[r].values = [(1, o.to_string()), (2, p.to_string())].into();
        }
        let to = |id: u32, name: &str| fmp12::graph::TableOccurrence {
            id,
            name: name.into(),
            base_table: id,
            ..Default::default()
        };
        Database {
            tables: vec![t, pets],
            occurrences: vec![to(T, "People"), to(PETS, "Pets")],
            relationships: vec![fmp12::graph::Relationship {
                left: T,
                right: PETS,
                predicates: vec![fmp12::graph::Predicate {
                    op: fmp12::graph::JoinOp::Equal,
                    left_field: NAME,
                    right_field: 1,
                }],
                ..Default::default()
            }],
            layouts: vec![],
            themes: vec![],
            value_lists: vec![],
            accounts: vec![],
            privilege_sets: vec![],
            locale: Locale::default(),
        }
    }

    /// Adds a summary field of `op` over `target` to People; returns its id.
    pub fn summary_field(db: &mut Database, op: fmp12::schema::SummaryOp, target: u32) -> u32 {
        let t = &mut db.tables[0];
        let id = t.fields.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        let mut f = field(id, &format!("{op:?}"), 3, 2);
        f.def.summary_raw =
            fmp12::schema::Field::summary_bytes(fmp12::schema::Summary { op, field: target });
        t.fields.push(f);
        id
    }

    #[test]
    fn stored_text_follows_file_formats() {
        let ts = calc::value::make_ts(
            calc::value::day_number(2026, 2, 1),
            &calc::num::int(14 * 3600 + 5 * 60 + 9),
        );
        let en = Locale::default();
        assert_eq!(
            stored_text(&Value::Timestamp(ts.clone()), &en),
            "2/1/2026 2:05:09 PM"
        );
        let fr = Locale {
            date_order: DateOrder::Dmy,
            decimal: ',',
            am: String::new(),
            pm: String::new(),
            ..Locale::default()
        };
        assert_eq!(
            stored_text(&Value::Timestamp(ts), &fr),
            "01/02/2026 14:05:09"
        );
        assert_eq!(
            stored_text(&Value::Number(calc::num::parse("12.75").unwrap()), &fr),
            "12,75"
        );
    }
}
