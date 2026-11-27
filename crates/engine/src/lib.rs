//! BaseCraft data engine: an in-memory database loaded from a `.fmp12`
//! file, with FileMaker's record, found-set, find, sort and calculation
//! semantics.

pub mod export;
mod find;
mod host;
pub mod import;
pub mod replace;
pub mod summary;
mod xlsx;

use std::collections::BTreeMap;

use calc::Value;
use fmp12::schema::{DataType, FieldKind};
use fmp12::schema_edit::{EditError, Editor, FieldSpec, RelationshipSpec};

pub use find::{Request, find};
pub use fmp12::containers::Stored;
pub use fmp12::schema::{rep_key, split_rep_key};
pub use summary::{groups, summarize, summarize_sorted, summary_at};

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
    /// Stored values as text, by field id ([`rep_key`] for repetitions
    /// after the first). A container value is a store id
    /// ([`fmp12::containers::ref_text`]) or a reference path.
    pub values: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub id: u32,
    pub name: String,
    pub fields: Vec<Field>,
    pub records: Vec<Record>,
    /// Container data by store id (FORMAT.md §13, "Containers").
    pub containers: BTreeMap<u32, Stored>,
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
            containers: BTreeMap::new(),
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
                containers: fmp12::containers::read(file, t.id)?,
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
                r.values.retain(|f, _| ids.contains(&split_rep_key(*f).0));
            }
            t.containers = std::mem::take(&mut old.containers);
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

    /// Adds a table occurrence of `table`; returns its id.
    pub fn add_occurrence(
        &mut self,
        file: &mut fmp12::File,
        table: u32,
        name: &str,
    ) -> Result<u32, EditError> {
        self.edit_schema(file, |ed| ed.add_occurrence(table, name))
    }

    pub fn rename_occurrence(
        &mut self,
        file: &mut fmp12::File,
        to: u32,
        name: &str,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.rename_occurrence(to, name))
    }

    pub fn delete_occurrence(&mut self, file: &mut fmp12::File, to: u32) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.delete_occurrence(to))
    }

    /// Adds a relationship; returns its id.
    pub fn add_relationship(
        &mut self,
        file: &mut fmp12::File,
        spec: &RelationshipSpec,
    ) -> Result<u32, EditError> {
        self.edit_schema(file, |ed| ed.add_relationship(spec))
    }

    pub fn set_relationship(
        &mut self,
        file: &mut fmp12::File,
        id: u32,
        spec: &RelationshipSpec,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.set_relationship(id, spec))
    }

    pub fn delete_relationship(
        &mut self,
        file: &mut fmp12::File,
        id: u32,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.delete_relationship(id))
    }

    /// Adds a value list; returns its id.
    pub fn add_value_list(
        &mut self,
        file: &mut fmp12::File,
        name: &str,
        source: &fmp12::valuelists::Source,
    ) -> Result<u32, EditError> {
        self.edit_schema(file, |ed| ed.add_value_list(name, source))
    }

    pub fn set_value_list(
        &mut self,
        file: &mut fmp12::File,
        id: u32,
        name: &str,
        source: &fmp12::valuelists::Source,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.set_value_list(id, name, source))
    }

    pub fn delete_value_list(&mut self, file: &mut fmp12::File, id: u32) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.delete_value_list(id))
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
                    .filter(|(f, s)| {
                        !s.is_empty() && !calcs.iter().any(|c| c.id == split_rep_key(**f).0)
                    })
                    .map(|(f, s)| (*f, s.clone()))
                    .collect()
            };
            let mut recs = BTreeMap::new();
            for (i, r) in t.records.iter().enumerate() {
                let mut values = r.values.clone();
                if old.get(&r.id).is_none_or(|o| input(o) != input(&values)) {
                    for c in &calcs {
                        for n in 1..=repetitions(c) {
                            let v = stored_text(&self.value_rep(t.id, i, c.id, n), &self.locale);
                            values.insert(rep_key(c.id, n), v);
                        }
                    }
                }
                recs.insert(r.id, values);
            }
            data.insert(t.id, recs);
        }
        let bytes = fmp12::save::save_records(original, &data)?;
        // Container data: files inserted since, use counts, unused files.
        let stores = self
            .tables
            .iter()
            .map(|t| (t.id, t.containers.clone()))
            .collect();
        let bytes = fmp12::containers::save_containers(&fmp12::File::from_bytes(bytes)?, &stores)?;
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
                    let inc = f.def.serial_increment() as u64;
                    ed.set_next_serial(t.id, f.id, &(max + inc).to_string());
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
        self.value_rep(table, record, field, 1)
    }

    /// Typed value of repetition `rep` (1-based) of a field; empty beyond
    /// the field's repetitions.
    pub fn value_rep(&self, table: u32, record: usize, field: u32, rep: u32) -> Value {
        let Some(t) = self.table(table) else {
            return Value::empty();
        };
        let Some(f) = t.field(field) else {
            return Value::empty();
        };
        if rep == 0 || rep > repetitions(f) {
            return Value::empty();
        }
        if f.kind == FieldKind::Summary {
            // ponytail: the engine has no found set, so a summary field
            // summarizes the whole table (Show All Records, unsorted);
            // O(n) per call, O(n²) for a running summary over every record.
            let all: Vec<usize> = (0..t.records.len()).collect();
            return summary_at(self, table, field, record, &all, &[]);
        }
        if let Some(src) = &f.calc {
            let mut ctx = RecordContext {
                db: self,
                table,
                to: f.context,
                record,
                rep,
                vars: BTreeMap::new(),
                env: None,
            };
            return calc::evaluate(src, &mut ctx).unwrap_or_else(|_| Value::text("?"));
        }
        let raw = t
            .records
            .get(record)
            .and_then(|r| r.values.get(&rep_key(field, rep)))
            .map_or("", |s| s.as_str());
        if f.data_type == DataType::Container {
            return container_value(t, raw);
        }
        typed_in(raw, f.data_type, &self.locale)
    }

    /// The text to store for `v` in repetition `rep` of a field: for a
    /// container field given a file, the file joins the table's container
    /// store (once per distinct content, as FileMaker keeps it) and the
    /// text refers to it; anything else as [`stored_text`].
    pub fn store_value(&mut self, table: u32, field: u32, v: &Value) -> String {
        let loc = self.locale.clone();
        let Some(t) = self.tables.iter_mut().find(|t| t.id == table) else {
            return stored_text(v, &loc);
        };
        let container = t
            .field(field)
            .is_some_and(|f| f.data_type == DataType::Container);
        match v {
            Value::Container(c) if container && c.path.is_empty() && !c.data.is_empty() => {
                let s = Stored::new(&c.name, c.data.clone());
                let hash = s.hash();
                let id = match t.containers.iter().find(|(_, o)| o.hash() == hash) {
                    Some((id, _)) => *id,
                    None => {
                        // ponytail: a store id freed by FileMaker above the
                        // highest one kept may be handed out again.
                        let id = t.containers.keys().max().map_or(1, |m| m + 1);
                        t.containers.insert(id, s);
                        id
                    }
                };
                fmp12::containers::ref_text(id)
            }
            Value::Container(c) if container => c.path.clone(),
            _ => stored_text(v, &loc),
        }
    }

    /// Insert Picture / Insert File: stores `data` named `name` in a
    /// container field's repetition.
    pub fn insert_file(
        &mut self,
        table: u32,
        record: usize,
        field: u32,
        rep: u32,
        name: &str,
        data: Vec<u8>,
    ) {
        let v = Value::Container(calc::value::Container {
            name: name.to_string(),
            data,
            ..Default::default()
        });
        let text = self.store_value(table, field, &v);
        self.set_values(table, record, &[(rep_key(field, rep), text)]);
    }
}

impl Database {
    /// Values of value list `id`: custom values, or the distinct values of
    /// its field (the values stored, sorted by the first or the second
    /// field).
    pub fn value_list_values(&self, id: u32) -> Vec<String> {
        self.value_list_values_from(id, None)
    }

    /// Like [`Database::value_list_values`]; a list of related values only
    /// takes the records related to `record` (occurrence, record index),
    /// none without it.
    pub fn value_list_values_from(&self, id: u32, record: Option<(u32, usize)>) -> Vec<String> {
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
            Source::Field {
                to,
                field,
                second,
                sort_by_second,
                related_from,
                ..
            } => {
                let Some(t) = self.table_of(*to) else {
                    return Vec::new();
                };
                let records: Vec<usize> = match (related_from, record) {
                    (None, _) => (0..t.records.len()).collect(),
                    (Some(from), Some((rto, r))) if *from == rto => self.related(rto, r, *to),
                    _ => Vec::new(),
                };
                // ponytail: the second field is read from the same record
                // (FileMaker allows it on a related occurrence).
                let by = second.filter(|_| *sort_by_second).map(|(_, f)| f);
                let mut v: Vec<(String, String)> = records
                    .iter()
                    .map(|&r| {
                        let first = self.value(t.id, r, *field).to_text();
                        let key =
                            by.map_or_else(|| first.clone(), |f| self.value(t.id, r, f).to_text());
                        (key.to_lowercase(), first)
                    })
                    .filter(|(_, s)| !s.is_empty())
                    .collect();
                v.sort();
                let mut out: Vec<String> = Vec::new();
                for (_, s) in v {
                    if !out.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
                        out.push(s);
                    }
                }
                out
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
            rep: 1,
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
            rep: 1,
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
        self.apply_lookups(table, r, None);
        self.touch(table, r);
        Some(r)
    }

    /// Duplicate Record: a new record (auto-enter applied) with the values
    /// of `record`, except the auto-entered ones (serials, stamps,
    /// auto-enter calculations), which stay those of the new record.
    /// Returns its index.
    pub fn duplicate_record(&mut self, table: u32, record: usize) -> Option<usize> {
        let src = self.table(table)?.records.get(record)?.values.clone();
        let r = self.new_record(table)?;
        let t = self.tables.iter_mut().find(|t| t.id == table)?;
        let auto: Vec<u32> = t
            .fields
            .iter()
            .filter(|f| {
                f.def.auto_serial() || f.def.auto_stamp().is_some() || f.def.auto_calculation()
            })
            .map(|f| f.id)
            .collect();
        t.records[r]
            .values
            .extend(src.into_iter().filter(|(f, _)| !auto.contains(f)));
        Some(r)
    }

    /// Sets field values of a record as data entry does: lookups whose
    /// match field changed copy their value again, then modification
    /// stamps are applied.
    pub fn set_values(&mut self, table: u32, record: usize, values: &[(u32, String)]) {
        let Some(ti) = self.tables.iter().position(|t| t.id == table) else {
            return;
        };
        let Some(rec) = self.tables[ti].records.get_mut(record) else {
            return;
        };
        for (f, v) in values {
            rec.values.insert(*f, v.clone());
        }
        let changed: Vec<u32> = values.iter().map(|(f, _)| *f).collect();
        self.apply_lookups(table, record, Some(&changed));
        self.touch(table, record);
    }

    /// Relookup Field Contents: the lookups matching on `field` copy their
    /// value again in each of `records`; records that change get their
    /// modification stamps.
    pub fn relookup(&mut self, table: u32, records: &[usize], field: u32) {
        for &r in records {
            let before = self.table(table).and_then(|t| t.records.get(r)).cloned();
            self.apply_lookups(table, r, Some(&[field]));
            let after = self.table(table).and_then(|t| t.records.get(r));
            if before.map(|b| b.values) != after.map(|a| a.values.clone()) {
                self.touch(table, r);
            }
        }
    }

    /// Copies the looked-up values of a record: every lookup (`changed` =
    /// `None`, a new record) or those whose first relationship matches on
    /// a field of `changed`, as FileMaker does when a match field changes.
    pub fn apply_lookups(&mut self, table: u32, record: usize, changed: Option<&[u32]>) {
        use fmp12::schema::NoMatch;
        let Some(ti) = self.tables.iter().position(|t| t.id == table) else {
            return;
        };
        let fields = self.tables[ti].fields.clone();
        for f in &fields {
            let Some(l) = f.def.lookup() else {
                continue;
            };
            let Some(hops) = self.path(l.context, l.from) else {
                continue;
            };
            let Some(&(a, _, i)) = hops.first() else {
                continue;
            };
            let rel = &self.relationships[i];
            // Match fields on this record's side of the first relationship.
            let keys: Vec<u32> = rel
                .predicates
                .iter()
                .map(|p| {
                    if rel.left == a {
                        p.left_field
                    } else {
                        p.right_field
                    }
                })
                .collect();
            if changed.is_some_and(|c| !keys.iter().any(|k| c.contains(k))) {
                continue;
            }
            let Some(src) = self.table_of(l.from).map(|t| t.id) else {
                continue;
            };
            let found = self.related(l.context, record, l.from);
            let value = match found.first() {
                Some(&r) => Some(self.stored(src, r, l.field)),
                None => match &l.no_match {
                    NoMatch::DontCopy => None,
                    NoMatch::Constant(c) => Some(c.clone()),
                    NoMatch::Lower | NoMatch::Higher => {
                        self.nearest(&hops, record, l.field, l.no_match == NoMatch::Lower)
                    }
                },
            };
            if let Some(v) = value.filter(|v| !v.is_empty() || l.copy_empty)
                && let Some(rec) = self.tables[ti].records.get_mut(record)
            {
                rec.values.insert(f.id, v);
            }
        }
    }

    /// A field's value as stored: the text of a normal field, the result
    /// of a calculation in the file's formats.
    fn stored(&self, table: u32, record: usize, field: u32) -> String {
        let Some(t) = self.table(table) else {
            return String::new();
        };
        match t.field(field) {
            Some(f) if f.kind == FieldKind::Normal => t
                .records
                .get(record)
                .and_then(|r| r.values.get(&field))
                .cloned()
                .unwrap_or_default(),
            _ => stored_text(&self.value(table, record, field), &self.locale),
        }
    }

    /// "Copy next lower / higher value": `field` of the record of a
    /// one-relationship lookup whose match value is closest below (above)
    /// this record's, on the first match field.
    /// ponytail: one hop and the first predicate only.
    fn nearest(
        &self,
        hops: &[(u32, u32, usize)],
        record: usize,
        field: u32,
        lower: bool,
    ) -> Option<String> {
        let [(a, b, i)] = hops else {
            return None;
        };
        let rel = &self.relationships[*i];
        let p = rel.predicates.first()?;
        let (fa, fb) = if rel.left == *a {
            (p.left_field, p.right_field)
        } else {
            (p.right_field, p.left_field)
        };
        let (ta, tb) = (self.table_of(*a)?.id, self.table_of(*b)?);
        let key = self.value(ta, record, fa);
        if key.is_empty() {
            return None;
        }
        let mut best: Option<(Value, usize)> = None;
        for j in 0..tb.records.len() {
            let v = self.value(tb.id, j, fb);
            let side = calc::eval::compare(&v, &key);
            let ok = if lower {
                side == std::cmp::Ordering::Less
            } else {
                side == std::cmp::Ordering::Greater
            };
            if v.is_empty() || !ok {
                continue;
            }
            let better = best.as_ref().is_none_or(|(b, _)| {
                let c = calc::eval::compare(&v, b);
                if lower {
                    c == std::cmp::Ordering::Greater
                } else {
                    c == std::cmp::Ordering::Less
                }
            });
            if better {
                best = Some((v, j));
            }
        }
        best.map(|(_, j)| self.stored(tb.id, j, field))
    }

    /// Deletes record `record` of `table` and, as FileMaker does, the
    /// records related through relationships set to delete them (and so
    /// on). Returns the deleted records as (table, record id).
    pub fn delete_record(&mut self, table: u32, record: usize) -> Vec<(u32, u32)> {
        let Some(id) = self
            .table(table)
            .and_then(|t| t.records.get(record))
            .map(|r| r.id)
        else {
            return Vec::new();
        };
        let mut gone: Vec<(u32, u32)> = vec![(table, id)];
        let mut i = 0;
        while let Some(&(t, rid)) = gone.get(i) {
            i += 1;
            let Some(idx) = self
                .table(t)
                .and_then(|x| x.records.iter().position(|r| r.id == rid))
            else {
                continue;
            };
            for rel in &self.relationships {
                for (a, b, delete) in [
                    (rel.left, rel.right, rel.right_cascade_delete()),
                    (rel.right, rel.left, rel.left_cascade_delete()),
                ] {
                    if !delete || self.table_of(a).map(|x| x.id) != Some(t) {
                        continue;
                    }
                    let Some(tb) = self.table_of(b) else {
                        continue;
                    };
                    for j in self.hop(a, &[idx], b, rel) {
                        let k = (tb.id, tb.records[j].id);
                        if !gone.contains(&k) {
                            gone.push(k);
                        }
                    }
                }
            }
        }
        for t in self.tables.iter_mut() {
            t.records.retain(|r| !gone.contains(&(t.id, r.id)));
        }
        gone
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
            // The file's next value, or past the highest value given here.
            // ponytail: numeric serials only.
            let t = self.table(table)?;
            let inc = f.def.serial_increment() as i64;
            let max = t
                .records
                .iter()
                .filter_map(|r| r.values.get(&f.id)?.trim().parse::<i64>().ok())
                .max();
            let stored = f.def.serial_next.trim().parse::<i64>().ok();
            let next = match (max, stored) {
                (Some(m), Some(s)) => s.max(m + inc),
                (Some(m), None) => m + inc,
                (None, s) => s.unwrap_or(1),
            };
            return Some(next.to_string());
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
            } else if let Some(vl) = f.def.validation_value_list()
                && !self
                    .value_list_values(vl)
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(text))
            {
                out.push(Invalid {
                    field: f.id,
                    rule: Rule::ValueList,
                    can_override: v.allow_override,
                });
            } else if f
                .def
                .max_size()
                .is_some_and(|n| text.chars().count() > n as usize)
            {
                out.push(Invalid {
                    field: f.id,
                    rule: Rule::MaxChars,
                    can_override: v.allow_override,
                });
            }
        }
        out
    }

    /// The custom message of a field's validation ("Display custom message
    /// if validation fails"), evaluated for `record`; `None` without one.
    pub fn validation_message(&self, table: u32, record: usize, field: u32) -> Option<String> {
        let f = self.table(table)?.field(field)?;
        if !f.def.validation().custom_message || f.def.message.is_empty() {
            return None;
        }
        Some(
            self.eval_source(&f.def.message, table, f.context, Some(record))
                .to_text(),
        )
    }

    fn to_named(&self, name: &str) -> Option<&fmp12::graph::TableOccurrence> {
        self.occurrences
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
    }

    /// Records related to record `record` of table occurrence `from`, seen
    /// through table occurrence `to` (following the shortest relationship
    /// path), in the relationship's sort order when it has one.
    pub fn related(&self, from: u32, record: usize, to: u32) -> Vec<usize> {
        let Some(hops) = self.path(from, to) else {
            return Vec::new();
        };
        let mut set = vec![record];
        for &(a, b, i) in &hops {
            set = self.hop(a, &set, b, &self.relationships[i]);
        }
        if let (Some(&(_, b, i)), Some(t)) = (hops.last(), self.table_of(to)) {
            let r = &self.relationships[i];
            let keys = if r.right == b {
                &r.right_sort
            } else {
                &r.left_sort
            };
            let keys: Vec<(u32, Order)> = keys
                .iter()
                .map(|k| {
                    let o = if k.descending {
                        Order::Descending
                    } else {
                        Order::Ascending
                    };
                    (k.field, o)
                })
                .collect();
            if !keys.is_empty() {
                sort(self, t.id, &mut set, &keys);
            }
        }
        set
    }

    /// The relationships from occurrence `from` to `to`: (a, b, index in
    /// `relationships`) per hop. `None` when they are not related.
    fn path(&self, from: u32, to: u32) -> Option<Vec<(u32, u32, usize)>> {
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
            let &(a, i) = prev.get(&cur)?;
            hops.push((a, cur, i));
            cur = a;
        }
        hops.reverse();
        Some(hops)
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
        let forward = rel.left == a;
        (0..tb.records.len())
            .filter(|&j| {
                records.iter().any(|&r| {
                    rel.predicates.iter().all(|p| {
                        let (fa, fb) = if forward {
                            (p.left_field, p.right_field)
                        } else {
                            (p.right_field, p.left_field)
                        };
                        let (va, vb) = (self.value(ta.id, r, fa), self.value(tb.id, j, fb));
                        // The predicate reads left op right.
                        let (l, r) = if forward { (&va, &vb) } else { (&vb, &va) };
                        joins(p.op, l, r)
                    })
                })
            })
            .collect()
    }
}

/// Whether a predicate `left op right` holds, as FileMaker matches
/// relationships: an empty value matches nothing, except through ×.
fn joins(op: fmp12::graph::JoinOp, left: &Value, right: &Value) -> bool {
    use fmp12::graph::JoinOp as J;
    use std::cmp::Ordering as O;
    if op == J::Cartesian {
        return true;
    }
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let c = calc::eval::compare(left, right);
    match op {
        J::Equal => c == O::Equal,
        J::NotEqual => c != O::Equal,
        J::Less => c == O::Less,
        J::LessOrEqual => c != O::Greater,
        J::Greater => c == O::Greater,
        J::GreaterOrEqual => c != O::Less,
        J::Cartesian => true,
        J::Unknown(_) => false,
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

/// Number of repetitions of a field (1 for a non-repeating one).
pub fn repetitions(f: &Field) -> u32 {
    (f.def.max_repetitions() as u32).max(1)
}

/// A container field's stored text as a value: the file it refers to in
/// the table's store, or the path of a file stored by reference.
// ponytail: referenced files are not read (no bytes, picture or MD5).
fn container_value(t: &Table, raw: &str) -> Value {
    if raw.is_empty() {
        return Value::empty();
    }
    let c = match fmp12::containers::ref_id(raw).and_then(|id| t.containers.get(&id)) {
        Some(s) => calc::value::Container {
            name: s.name.clone(),
            data: s.data().to_vec(),
            path: String::new(),
        },
        None => calc::value::Container {
            path: raw.to_string(),
            ..Default::default()
        },
    };
    Value::Container(c)
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
    /// Repetition of a repeating calculation being evaluated (1 otherwise).
    rep: u32,
    vars: BTreeMap<(String, usize), Value>,
    env: Option<&'e mut dyn Env>,
}

impl RecordContext<'_, '_> {
    /// (table, records, field, own record) a field reference stands for:
    /// this record, or the related records.
    fn resolve(
        &self,
        table: Option<&str>,
        name: &str,
    ) -> Result<(u32, Vec<usize>, &Field, bool), calc::EvalError> {
        let missing = |what: &str| calc::EvalError::new(102, format!("{what} missing"));
        let (t, records, own) = match table {
            Some(to_name) => {
                let to = self.db.to_named(to_name).ok_or_else(|| missing(to_name))?;
                if to.id == self.to || (self.to == 0 && to.base_table == self.table) {
                    (to.base_table, vec![self.record], true)
                } else {
                    let rel = self.db.related(self.to, self.record, to.id);
                    (to.base_table, rel, false)
                }
            }
            None => (self.table, vec![self.record], true),
        };
        let tbl = self.db.table(t).ok_or_else(|| missing("table"))?;
        let f = tbl.field_by_name(name).ok_or_else(|| missing(name))?;
        Ok((t, records, f, own))
    }

    /// Repetition `repetition` of a field; in repetition n of a repeating
    /// calculation, a plain reference (`repetition` 1) means repetition n
    /// of a repeating field, and nothing for a non-repeating one unless
    /// `extended`.
    // ponytail: an explicit `Field[1]` inside repetition n reads as plain.
    fn rep_value(
        &self,
        table: Option<&str>,
        name: &str,
        repetition: usize,
        extended: bool,
    ) -> Result<Value, calc::EvalError> {
        let (t, records, f, _) = self.resolve(table, name)?;
        let rep = match (repetition, self.rep) {
            (1, n) if n > 1 && repetitions(f) > 1 => n,
            (1, n) if n > 1 && !extended => return Ok(Value::empty()),
            (r, _) => r as u32,
        };
        Ok(records
            .first()
            .map_or_else(Value::empty, |r| self.db.value_rep(t, *r, f.id, rep)))
    }
}

impl calc::Context for RecordContext<'_, '_> {
    fn field(
        &mut self,
        table: Option<&str>,
        name: &str,
        repetition: usize,
    ) -> Result<Value, calc::EvalError> {
        self.rep_value(table, name, repetition, false)
    }

    fn extended(&mut self, table: Option<&str>, name: &str) -> Result<Value, calc::EvalError> {
        self.rep_value(table, name, 1, true)
    }

    /// The related records' values, or a repeating field's repetitions.
    fn field_values(
        &mut self,
        table: Option<&str>,
        name: &str,
    ) -> Result<Vec<Value>, calc::EvalError> {
        let (t, records, f, own) = self.resolve(table, name)?;
        if own && repetitions(f) > 1 {
            return Ok((1..=repetitions(f))
                .map(|n| self.db.value_rep(t, self.record, f.id, n))
                .collect());
        }
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
        let mut found: Vec<usize> = (0..t.records.len()).collect();
        if sf == bf {
            return Ok(summary::summarize(self.db, t.id, sf, &found, &found));
        }
        // The found set sorted by the break field, and the records sharing
        // this record's break value.
        sort(self.db, t.id, &mut found, &[(bf, Order::Ascending)]);
        let mine = self.db.value(t.id, self.record, bf);
        let group: Vec<usize> = found
            .iter()
            .copied()
            .filter(|&r| {
                calc::eval::compare(&self.db.value(t.id, r, bf), &mine) == std::cmp::Ordering::Equal
            })
            .collect();
        Ok(summary::summarize_sorted(
            self.db,
            t.id,
            sf,
            &group,
            &found,
            &[bf],
        ))
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
        if flag.eq_ignore_ascii_case("CalculationRepetitionNumber") {
            return Some(Value::int(self.rep as i64));
        }
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
    /// Not a value of the field's "member of value list".
    ValueList,
    /// More characters than the field's maximum.
    MaxChars,
}

impl Rule {
    /// FileMaker's error code (`Get ( LastError )` after a commit fails).
    pub fn code(self) -> u32 {
        match self {
            Rule::Numeric => 502,
            Rule::Unique => 504,
            Rule::Existing => 505,
            Rule::ValueList => 506,
            Rule::NotEmpty => 509,
            Rule::MaxChars => 511,
        }
    }
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
