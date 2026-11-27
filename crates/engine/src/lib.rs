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
                    let calc = (f.kind() == FieldKind::Calculated)
                        .then(|| streams.get(&(t.id, *id)))
                        .flatten()
                        .and_then(|(k4, k5)| {
                            let ctx = fmp12::calc::context(k4).unwrap_or(0);
                            fmp12::calc::decompile(k5, ctx, &names).ok()
                        });
                    Field {
                        id: *id,
                        name: f.name.clone(),
                        kind: f.kind(),
                        data_type: f.data_type(),
                        calc,
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
        // ponytail: related fields (other TOs) resolve to the first related record only.
        let t = match table {
            Some(to_name) => {
                let to = self
                    .db
                    .occurrences
                    .iter()
                    .find(|o| o.name.eq_ignore_ascii_case(to_name))
                    .ok_or_else(|| calc::EvalError::new(102, "table missing"))?;
                to.base_table
            }
            None => self.table,
        };
        let tbl = self
            .db
            .table(t)
            .ok_or_else(|| calc::EvalError::new(102, "table missing"))?;
        let f = tbl
            .field_by_name(name)
            .ok_or_else(|| calc::EvalError::new(102, format!("field {name} missing")))?;
        let rec = if t == self.table { self.record } else { 0 };
        Ok(self.db.value(t, rec, f.id))
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
