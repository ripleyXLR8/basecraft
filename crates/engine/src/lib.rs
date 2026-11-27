//! OpenMaker data engine: an in-memory database loaded from a `.fmp12`
//! file, with FileMaker's record, found-set, find, sort and calculation
//! semantics.

mod find;

use std::collections::BTreeMap;

use calc::Value;
use fmp12::schema::{DataType, FieldKind};

pub use find::{Request, find};

#[derive(Debug, Clone)]
pub struct Field {
    pub id: u32,
    pub name: String,
    pub kind: FieldKind,
    pub data_type: DataType,
    /// Calculation source for calculated fields, in a form `calc` parses
    /// (field names wrapped as `${…}`).
    pub calc: Option<String>,
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
                    let stream = (f.kind() == FieldKind::Calculated)
                        .then(|| streams.get(&(t.id, *id)))
                        .flatten();
                    let context = stream
                        .and_then(|(k4, _)| fmp12::calc::context(k4))
                        .unwrap_or(0);
                    let calc =
                        stream.and_then(|(_, k5)| fmp12::calc::decompile(k5, context, &names).ok());
                    Field {
                        id: *id,
                        name: f.name.clone(),
                        kind: f.kind(),
                        data_type: f.data_type(),
                        calc,
                        context,
                        def: f.clone(),
                    }
                })
                .collect();
            let records: Vec<Record> = fmp12::schema::records(file, t)?
                .into_iter()
                .map(|(id, values)| Record { id, values })
                .collect();
            tables.push(Table {
                id: t.id,
                name: t.name.clone(),
                next_record_id: records.iter().map(|r| r.id).max().unwrap_or(0),
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
            occurrences,
        })
    }

    pub fn table(&self, id: u32) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
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
        if let Some(src) = &f.calc {
            let mut ctx = RecordContext {
                db: self,
                table,
                to: f.context,
                record,
                vars: BTreeMap::new(),
            };
            return calc::evaluate(src, &mut ctx).unwrap_or_else(|_| Value::text("?"));
        }
        let raw = t
            .records
            .get(record)
            .and_then(|r| r.values.get(&field))
            .map_or("", |s| s.as_str());
        typed(raw, f.data_type)
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

    /// Evaluates a calculation stored as a token stream (layout labels,
    /// script parameters…) for `record` of `table`, from occurrence `to`.
    pub fn eval_tokens(&self, tokens: &[u8], table: u32, to: u32, record: Option<usize>) -> Value {
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
        let Ok(src) = fmp12::calc::decompile(tokens, to, &names) else {
            return Value::text("?");
        };
        let mut ctx = RecordContext {
            db: self,
            table,
            to,
            record: record.unwrap_or(usize::MAX),
            vars: BTreeMap::new(),
        };
        calc::evaluate(&src, &mut ctx).unwrap_or_else(|_| Value::text("?"))
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

/// Converts stored text to a typed value.
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

/// Evaluation context bound to one record.
struct RecordContext<'a> {
    db: &'a Database,
    table: u32,
    /// Context table occurrence of the calculation.
    to: u32,
    record: usize,
    vars: BTreeMap<(String, usize), Value>,
}

impl calc::Context for RecordContext<'_> {
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

    fn variable(&mut self, name: &str, repetition: usize) -> Value {
        self.vars
            .get(&(name.to_string(), repetition))
            .cloned()
            .unwrap_or_else(Value::empty)
    }

    fn set_variable(&mut self, name: &str, repetition: usize, value: Value) {
        self.vars.insert((name.to_string(), repetition), value);
    }
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
