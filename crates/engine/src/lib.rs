//! BaseCraft data engine: an in-memory database loaded from a `.fmp12`
//! file, with FileMaker's record, found-set, find, sort and calculation
//! semantics.

mod access;
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
use fmp12::schema_edit::{CustomFunctionSpec, EditError, Editor, FieldSpec, RelationshipSpec};

pub use find::{Request, find, find_from, related_key};
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
    /// "Validated by calculation" (`calc` source; `Self` is the value).
    pub validation_calc: Option<String>,
    /// "In range" validation: low and high, as text in the file's formats.
    pub range: Option<(String, String)>,
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
    /// Values of the global fields, by value key (`rep_key`): one per
    /// table, whatever the record (FORMAT.md §13, "Global fields").
    pub globals: BTreeMap<u32, String>,
    next_record_id: u32,
}

impl Table {
    fn is_global(&self, key: u32) -> bool {
        let f = split_rep_key(key).0;
        self.fields.iter().any(|x| x.id == f && x.def.global())
    }

    /// Stored text of value key `key` (`rep_key`) as record `record` sees
    /// it: a global field's is the table's.
    pub fn raw(&self, record: usize, key: u32) -> Option<&String> {
        if self.is_global(key) {
            return self.globals.get(&key);
        }
        self.records.get(record)?.values.get(&key)
    }

    /// The stored text to edit in place (created empty).
    pub fn raw_mut(&mut self, record: usize, key: u32) -> Option<&mut String> {
        if self.is_global(key) {
            return Some(self.globals.entry(key).or_default());
        }
        Some(self.records.get_mut(record)?.values.entry(key).or_default())
    }

    /// An empty table (databases built in code, tests).
    pub fn new(id: u32, name: &str, fields: Vec<Field>) -> Table {
        Table {
            id,
            name: name.to_string(),
            fields,
            records: Vec::new(),
            containers: BTreeMap::new(),
            globals: BTreeMap::new(),
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
    /// The logged-in account and the last action its privileges refused.
    pub access: Access,
    /// File › Manage › Custom Functions.
    pub custom_functions: CustomFunctions,
    /// Base directories of external container storage: (id, path relative
    /// to the database's folder), [`fmp12::containers::base_directories`].
    pub base_directories: Vec<(u32, String)>,
    /// Pictures of graphic layout objects, by store id (`[31].[5]`).
    pub pictures: BTreeMap<u32, fmp12::containers::Stored>,
}

/// The file's custom functions, as listed and as the calculation engine
/// calls them.
#[derive(Debug, Clone, Default)]
pub struct CustomFunctions {
    /// Folders included, in display order.
    pub list: Vec<fmp12::custom_functions::CustomFunction>,
    /// Calculation of each function (by id) as FileMaker shows it.
    pub formulas: BTreeMap<u32, String>,
    /// By lower-case name, parsed.
    pub by_name: std::collections::HashMap<String, std::sync::Arc<calc::CustomFunction>>,
}

impl CustomFunctions {
    fn load(file: &fmp12::File, names: &EvalNames) -> Result<Self, fmp12::Error> {
        use fmp12::custom_functions::Kind;
        let list = fmp12::custom_functions::read(file)?;
        let fns: Vec<_> = list.iter().filter(|f| f.kind == Kind::Function).collect();
        let lower: Vec<String> = fns.iter().map(|f| f.name.to_lowercase()).collect();
        let shown = fmp12::calc::FileNames::load(file)?;
        let mut out = CustomFunctions::default();
        for f in fns {
            out.formulas.insert(f.id, f.formula(&shown));
            // The engine's spelling of field names (`${…}`).
            let src = fmp12::calc::decompile(&f.tokens, 0, names).unwrap_or_default();
            let c = calc::CustomFunction::new(&f.name, f.params.clone(), &src, &|n| {
                lower.contains(&n.to_lowercase())
            });
            out.by_name
                .insert(f.name.to_lowercase(), std::sync::Arc::new(c));
        }
        out.list = list;
        Ok(out)
    }
}

pub use access::{
    ADMIN_ACCOUNT, Action, DENIED_MESSAGE, FIELD_DENIED, NO_ACCOUNT, PRIVILEGES, RECORD_DENIED,
};

/// Who is logged in (`None`: a database built in memory, no restrictions)
/// and the error of the last refused action.
#[derive(Debug, Clone, Default)]
pub struct Access {
    pub session: Option<fmp12::security::Session>,
    pub denied: Option<u32>,
    /// The file's accounts and privileges, with the changes script steps
    /// made (Add Account, Change Password…).
    pub security: Option<fmp12::security::Security>,
    /// Those changes, each with the account that made it: FileMaker
    /// commits a step's change as the account running it, so
    /// [`Database::save`] writes them in order, each authorized by its own
    /// session.
    pub pending: Vec<(fmp12::security::Session, fmp12::security::Security)>,
}

/// Names for decompiling calculations into `calc`-parsable source.
struct EvalNames<'a> {
    tables: &'a [fmp12::schema::Table],
    tos: &'a [fmp12::graph::TableOccurrence],
    customs: Vec<(u32, String)>,
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
    fn custom_function(&self, id: u32) -> Option<String> {
        let (_, n) = self.customs.iter().find(|(i, _)| *i == id)?;
        Some(n.clone())
    }
}

impl Database {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, fmp12::Error> {
        Self::load(&fmp12::File::open(path)?)
    }

    /// Loads a file, logged in by its File Options ("Log in using").
    /// A protected file gives [`fmp12::Error::LoginRequired`]: open it with
    /// [`Database::load_as`].
    pub fn load(file: &fmp12::File) -> Result<Self, fmp12::Error> {
        let sec = fmp12::security::Security::load(file)?;
        if sec.accounts.is_empty() {
            return Self::load_with(file, None);
        }
        let session = sec.login_auto().ok_or(fmp12::Error::LoginRequired)?;
        Self::load_with(file, Some(session))
    }

    fn load_with(
        file: &fmp12::File,
        session: Option<fmp12::security::Session>,
    ) -> Result<Self, fmp12::Error> {
        let schema = fmp12::schema::tables(file)?;
        let occurrences = fmp12::graph::table_occurrences(file)?;
        // Calculation token streams of calculated fields: (table, field) -> (key 4, key 5).
        let mut streams: BTreeMap<(u32, u32), (Vec<u8>, Vec<u8>)> = BTreeMap::new();
        // Validation calculations (sub-node 25) and range bounds (keys 33, 34).
        let mut checks: BTreeMap<(u32, u32), (Vec<u8>, Vec<u8>)> = BTreeMap::new();
        let mut ranges: BTreeMap<(u32, u32), (String, String)> = BTreeMap::new();
        file.walk(|path, c| {
            let (Some(p), fmp12::Chunk::Kv { key, data }) = (fmp12::int_path(path), c) else {
                return;
            };
            let e = match p.as_slice() {
                [t, 3, 5, f, 5] => streams.entry((*t, *f)).or_default(),
                [t, 3, 5, f, 25] => checks.entry((*t, *f)).or_default(),
                [t, 3, 5, f] => {
                    let r = ranges.entry((*t, *f)).or_default();
                    match key {
                        33 => r.0 = fmp12::text(data),
                        34 => r.1 = fmp12::text(data),
                        _ => {}
                    }
                    return;
                }
                _ => return,
            };
            match key {
                4 => e.0 = data.to_vec(),
                5 => e.1.extend_from_slice(data),
                _ => {}
            }
        })?;
        let names = EvalNames {
            tables: &schema,
            tos: &occurrences,
            customs: fmp12::custom_functions::read(file)?
                .into_iter()
                .map(|f| (f.id, f.name))
                .collect(),
        };
        let custom_functions = CustomFunctions::load(file, &names)?;
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
                    let v = f.validation();
                    let validation_calc = checks
                        .get(&(t.id, *id))
                        .filter(|_| v.calculation)
                        .and_then(|(k4, k5)| {
                            let to = fmp12::calc::context(k4).unwrap_or(context);
                            fmp12::calc::decompile(k5, to, &names).ok()
                        });
                    Field {
                        id: *id,
                        name: f.name.clone(),
                        kind: f.kind(),
                        data_type: f.data_type(),
                        calc,
                        auto_calc,
                        validation_calc,
                        range: ranges.get(&(t.id, *id)).filter(|_| v.range).cloned(),
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
                globals: fmp12::schema::globals(file, t)?,
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
            custom_functions,
            base_directories: fmp12::containers::base_directories(file)?,
            pictures: fmp12::containers::layout_pictures(file)?,
            access: Access {
                session,
                denied: None,
                security: Some(fmp12::security::Security::load(file)?),
                pending: Vec::new(),
            },
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
        let mut fresh = Database::load_with(&new, self.access.session.clone())?;
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

    /// Adds a custom function; returns its id.
    pub fn add_custom_function(
        &mut self,
        file: &mut fmp12::File,
        spec: &CustomFunctionSpec,
    ) -> Result<u32, EditError> {
        self.edit_schema(file, |ed| ed.add_custom_function(spec))
    }

    /// Replaces custom function `id`.
    pub fn set_custom_function(
        &mut self,
        file: &mut fmp12::File,
        id: u32,
        spec: &CustomFunctionSpec,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.set_custom_function(id, spec))
    }

    /// Deletes custom function `id`.
    pub fn delete_custom_function(
        &mut self,
        file: &mut fmp12::File,
        id: u32,
    ) -> Result<(), EditError> {
        self.edit_schema(file, |ed| ed.delete_custom_function(id))
    }

    /// Reads the files of containers kept in open external storage, from
    /// the folder `dir` of the database (FORMAT.md §13). Files that are
    /// missing stay unread (FileMaker then shows the file name only);
    /// secure storage is encrypted and is not read.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_external_containers(&mut self, dir: &std::path::Path) {
        let base = self.external_base(dir);
        for t in self.tables.iter_mut() {
            for s in t.containers.values_mut() {
                if let Some(p) = s.external().and_then(|x| x.path()) {
                    s.external_data = std::fs::read(base.join(p)).ok();
                }
            }
        }
    }

    /// Writes the files of containers kept in open external storage into
    /// the folder `dir` of the database the records were saved to, in
    /// FileMaker's layout (`<base directory>/<Table>/<Field>/<name>`).
    /// Files already there are left alone. Returns the files written.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_external_containers(
        &self,
        dir: &std::path::Path,
    ) -> std::io::Result<Vec<std::path::PathBuf>> {
        let base = self.external_base(dir);
        let mut out = Vec::new();
        for t in &self.tables {
            for s in t.containers.values() {
                let (Some(p), Some(data)) = (s.external().and_then(|x| x.path()), &s.external_data)
                else {
                    continue;
                };
                let path = base.join(p);
                if path.exists() {
                    continue;
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, data)?;
                out.push(path);
            }
        }
        Ok(out)
    }

    /// Folder of the first base directory ("[database location]/…").
    // ponytail: other base directories and absolute ones are not used.
    #[cfg(not(target_arch = "wasm32"))]
    fn external_base(&self, dir: &std::path::Path) -> std::path::PathBuf {
        let rel = self
            .base_directories
            .iter()
            .min_by_key(|(id, _)| *id)
            .map_or("", |(_, p)| p.as_str());
        dir.join(rel)
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
        let globals = self
            .tables
            .iter()
            .map(|t| (t.id, t.globals.clone()))
            .collect();
        let bytes = fmp12::save::save_globals(&fmp12::File::from_bytes(bytes)?, &globals)?;
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
                // The value in memory (Set Next Serial Value, Replace
                // Field Contents), moved past the numbers given here.
                let inc = f.def.serial_increment() as u64;
                let want = match (f.def.serial_next.trim().parse::<u64>().ok(), max) {
                    (Some(w), Some(m)) if m >= w => Some(m + inc),
                    (Some(w), _) => Some(w),
                    (None, m) => m.map(|m| m + inc),
                };
                if let (Some(want), Some(next)) = (want, next)
                    && want != next
                {
                    ed.set_next_serial(t.id, f.id, &want.to_string());
                    moved = true;
                }
            }
        }
        let out = if moved {
            ed.finish()
        } else {
            saved.bytes().to_vec()
        };
        self.save_security(out)
    }

    /// Occurrence and field id of the merge field `<<name>>` on a layout
    /// of occurrence `layout_to`: `TO::Field`, `Field`, or the reference
    /// FileMaker stores ([`fmp12::layouts::merge_ref`]).
    pub fn merge_field(&self, name: &str, layout_to: u32) -> Option<(u32, u32)> {
        // By reference: two spellings of the same `TO::Field` bytes.
        if let Some(r) =
            fmp12::layouts::merge_ref(name).or_else(|| fmp12::layouts::merge_field_ref(name))
        {
            return Some(r);
        }
        let (to_name, field) = name.split_once("::").unwrap_or(("", name));
        let to = self
            .occurrences
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(to_name))
            .map_or(layout_to, |o| o.id);
        Some((to, self.table_of(to)?.field_by_name(field)?.id))
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
        self.value_in(table, record, field, rep, None)
    }

    /// Like [`Database::value_rep`], as a window showing `found` sees it:
    /// a summary field summarizes the found set in its sort order (running
    /// totals, subtotaled fractions), and calculations see the found set
    /// (GetSummary, GetNthRecord, `Get ( RecordNumber )`…). `None`, or a
    /// found set of another table: every record, unsorted.
    pub fn value_in(
        &self,
        table: u32,
        record: usize,
        field: u32,
        rep: u32,
        found: Option<FoundSet>,
    ) -> Value {
        let found = found.filter(|f| f.table == table);
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
            // ponytail: O(n) per call, O(n²) for a running summary over
            // every record of a found set.
            return match found {
                Some(fs) => summary_at(self, table, field, record, fs.records, fs.sort),
                None => {
                    let all: Vec<usize> = (0..t.records.len()).collect();
                    summary_at(self, table, field, record, &all, &[])
                }
            };
        }
        if let Some(src) = &f.calc {
            let scope = Scope {
                found,
                self_value: None,
            };
            let mut ctx = RecordContext::new(self, table, f.context, Some(record), &scope, None);
            ctx.rep = rep;
            return calc::evaluate(src, &mut ctx)
                .map(Value::plain)
                .unwrap_or_else(|_| Value::text("?"));
        }
        let raw = t
            .raw(record, rep_key(field, rep))
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
                let open = t.field(field).is_some_and(|f| {
                    f.def.external_storage() == Some(fmp12::containers::Storage::Open)
                });
                let s = if open {
                    // ponytail: FileMaker's default folder ("Table/Field/");
                    // a folder calculation of the field is not evaluated.
                    let fname = t.field(field).map(|f| f.name.clone()).unwrap_or_default();
                    let dir = format!("{}/{}/", t.name, fname);
                    let file = external_name(t, &dir, &c.name, &c.data);
                    Stored::new_open(&c.name, &file, &dir, c.data.clone())
                } else {
                    Stored::new(&c.name, c.data.clone())
                };
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
    /// Auto-complete using previously entered values: the distinct values
    /// of `field` in `table` that start with `prefix` (ignoring case),
    /// sorted, `prefix` itself excluded.
    // ponytail: scans every record; read the field's index if tables get large.
    pub fn completions(&self, table: u32, field: u32, prefix: &str) -> Vec<String> {
        let p = prefix.to_lowercase();
        let n = self.table(table).map_or(0, |t| t.records.len());
        let mut v: Vec<String> = (0..n)
            .map(|r| self.value(table, r, field).to_text())
            .filter(|s| {
                let l = s.to_lowercase();
                !p.is_empty() && l.starts_with(&p) && l != p
            })
            .collect();
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup_by_key(|s| s.to_lowercase());
        v
    }

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

    /// What fields showing value list `id` display for their stored values
    /// when the list shows only its second field's values: stored value
    /// (first field) → second field's value, from the same record. `None`
    /// for any other list.
    pub fn value_list_labels(&self, id: u32) -> Option<BTreeMap<String, String>> {
        let vl = self.value_lists.iter().find(|v| v.id == id)?;
        let fmp12::valuelists::Source::Field {
            to,
            field,
            second: Some((_, second)),
            second_only: true,
            ..
        } = &vl.source
        else {
            return None;
        };
        // ponytail: the second field is read from the same record (as in
        // `value_list_values_from`).
        let t = self.table_of(*to)?;
        let mut out = BTreeMap::new();
        for r in 0..t.records.len() {
            let v = self.value(t.id, r, *field).to_text();
            if !v.is_empty() {
                out.entry(v)
                    .or_insert_with(|| self.value(t.id, r, *second).to_text());
            }
        }
        Some(out)
    }

    /// Evaluates calculation source text for `record` of `table`, from
    /// occurrence `to` (Data Viewer style).
    pub fn eval_source(&self, src: &str, table: u32, to: u32, record: Option<usize>) -> Value {
        self.eval_source_in(src, table, to, record, &Scope::default())
            .unwrap_or_else(|_| Value::text("?"))
    }

    /// Like [`Database::eval_source`], in a window's found set and with
    /// `Self` ([`Scope`]), and the error FileMaker would report.
    pub fn eval_source_in(
        &self,
        src: &str,
        table: u32,
        to: u32,
        record: Option<usize>,
        scope: &Scope,
    ) -> Result<Value, calc::EvalError> {
        let mut ctx = RecordContext::new(self, table, to, record, scope, None);
        calc::evaluate(src, &mut ctx)
    }

    /// A layout object's calculation where `Self` is the object's value:
    /// conditional formatting (the formula FileMaker stores for a "Value
    /// is…" condition tests `Self`), "hide object when", tooltips.
    /// `found`: the window's found set (GetSummary, `Get ( RecordNumber )`).
    pub fn eval_with_self(
        &self,
        tokens: &[u8],
        table: u32,
        to: u32,
        record: Option<usize>,
        self_value: &Value,
        found: Option<FoundSet>,
    ) -> Value {
        let scope = Scope {
            found,
            self_value: Some(self_value.clone()),
        };
        self.eval_tokens_scoped(tokens, table, to, record, &scope, None)
            .unwrap_or_else(|_| Value::text("?"))
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
        self.eval_tokens_scoped(tokens, table, to, record, &Scope::default(), env)
    }

    /// The general form of the token evaluations: in a window's found set
    /// and with `Self` ([`Scope`]), with a script's variables and flags.
    pub fn eval_tokens_scoped(
        &self,
        tokens: &[u8],
        table: u32,
        to: u32,
        record: Option<usize>,
        scope: &Scope,
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
            customs: self
                .custom_functions
                .list
                .iter()
                .map(|f| (f.id, f.name.clone()))
                .collect(),
        };
        let src = fmp12::calc::decompile(tokens, to, &names)
            .map_err(|e| calc::EvalError::new(calc::error::code::GENERIC, e.to_string()))?;
        let mut ctx = RecordContext::new(self, table, to, record, scope, env);
        calc::evaluate(&src, &mut ctx)
    }

    /// New Record with FileMaker's auto-enter options applied (creation
    /// stamps, serial numbers, auto-enter calculations). Returns its index.
    pub fn new_record(&mut self, table: u32) -> Option<usize> {
        if !self.can(Action::Create, table, None) {
            self.access.denied = Some(RECORD_DENIED);
            return None;
        }
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
        if let Some(code) = self.edit_denied(table, record, values) {
            self.access.denied = Some(code);
            return;
        }
        let Some(ti) = self.tables.iter().position(|t| t.id == table) else {
            return;
        };
        // Global fields keep one value for the table, records or not.
        let t = &mut self.tables[ti];
        let (globals, values): (Vec<_>, Vec<_>) = values.iter().partition(|(f, _)| t.is_global(*f));
        for (f, v) in globals {
            t.globals.insert(*f, v.clone());
        }
        let Some(rec) = t.records.get_mut(record) else {
            return;
        };
        for (f, v) in &values {
            rec.values.insert(*f, v.clone());
        }
        let changed: Vec<u32> = values.iter().map(|(f, _)| *f).collect();
        if changed.is_empty() {
            return;
        }
        self.apply_lookups(table, record, Some(&changed));
        self.reevaluate_auto(table, record, &changed);
        self.touch(table, record);
    }

    /// Auto-enter calculations that use a changed field evaluate again, as
    /// in FileMaker: into an empty field, or over its value when "Do not
    /// replace existing value" is unticked.
    // ponytail: a field is "used" when its `${Name}` appears in the formula
    // (a same-named related field counts too); one pass, no cascade.
    fn reevaluate_auto(&mut self, table: u32, record: usize, changed: &[u32]) {
        let Some(t) = self.table(table) else {
            return;
        };
        let names: Vec<String> = changed
            .iter()
            .filter_map(|c| t.field(split_rep_key(*c).0))
            .map(|f| format!("${{{}}}", f.name.to_lowercase()))
            .collect();
        let empty = |f: &Field| {
            t.records
                .get(record)
                .is_none_or(|r| r.values.get(&f.id).is_none_or(|v| v.is_empty()))
        };
        let updates: Vec<(u32, String)> = t
            .fields
            .iter()
            .filter(|f| !changed.contains(&f.id))
            .filter(|f| {
                f.auto_calc.as_ref().is_some_and(|s| {
                    let s = s.to_lowercase();
                    names.iter().any(|n| s.contains(n.as_str()))
                })
            })
            .filter(|f| f.def.auto_overwrite() || empty(f))
            .filter_map(|f| Some((f.id, self.auto_calc_value(table, record, f)?)))
            .collect();
        if let Some(r) = self
            .tables
            .iter_mut()
            .find(|t| t.id == table)
            .and_then(|t| t.records.get_mut(record))
        {
            r.values.extend(updates);
        }
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
            Some(f) if f.kind == FieldKind::Normal => {
                t.raw(record, field).cloned().unwrap_or_default()
            }
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
        if !self.can(Action::Delete, table, Some(record)) {
            self.access.denied = Some(RECORD_DENIED);
            return Vec::new();
        }
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
                CreationName | ModificationName | CreationAccountName | ModificationAccountName => {
                    self.session()
                        .map_or("Admin", |s| s.account_name())
                        .to_string()
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
        self.auto_calc_value(table, record, f)
    }

    /// The auto-enter calculation of `f` for `record`; `Self` is the
    /// field's current value.
    fn auto_calc_value(&self, table: u32, record: usize, f: &Field) -> Option<String> {
        let src = f.auto_calc.as_ref()?;
        let scope = Scope {
            found: None,
            self_value: Some(self.value(table, record, f.id)),
        };
        let v = self
            .eval_source_in(src, table, f.context, Some(record), &scope)
            .unwrap_or_else(|_| Value::text("?"));
        Some(v.to_text())
    }

    /// Checks a record against its fields' validation options, as FileMaker
    /// does when the record is committed. Returns the failing fields.
    pub fn validate(&self, table: u32, record: usize) -> Vec<Invalid> {
        self.validate_with(table, record, true)
    }

    /// [`Database::validate`] for a record a script changed (Set Field…,
    /// then a commit): FileMaker 26 only checks not empty, unique, existing
    /// and calculations then; value lists, ranges and maximum characters
    /// are checked on data entry only (whatever "validate always" says).
    /// The first failing field (by field id) is the error FileMaker reports.
    pub fn validate_script(&self, table: u32, record: usize) -> Vec<Invalid> {
        self.validate_with(table, record, false)
    }

    fn validate_with(&self, table: u32, record: usize, entry: bool) -> Vec<Invalid> {
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
            if let Some(rule) = self.failed_rule(t, f, record, text, entry) {
                out.push(Invalid {
                    field: f.id,
                    rule,
                    can_override: v.allow_override,
                    always: v.always,
                });
            }
        }
        out
    }

    /// The first validation rule `text` (the stored value of field `f` in
    /// `record`) fails, in the order of FileMaker's Field Options dialog:
    /// strict data type, not empty, unique, existing, value list, range,
    /// calculation, maximum characters. Empty values only fail "not empty".
    /// Without `entry`, only the rules a script's commit
    /// checks ([`Database::validate_script`]).
    fn failed_rule(
        &self,
        t: &Table,
        f: &Field,
        record: usize,
        text: &str,
        entry: bool,
    ) -> Option<Rule> {
        let v = f.def.validation();
        let typed = typed_in(text, f.data_type, &self.locale);
        let set = !text.trim().is_empty();
        let same = |o: &Record| {
            o.values
                .get(&f.id)
                .is_some_and(|x| x.eq_ignore_ascii_case(text))
        };
        let others = || t.records.iter().enumerate().filter(|(i, _)| *i != record);
        let in_range = |(lo, hi): &(String, String)| {
            // Typed values compare by type, text fields as text.
            let bound = |s: &str| typed_in(s, f.data_type, &self.locale);
            calc::eval::compare(&typed, &bound(lo)).is_ge()
                && calc::eval::compare(&typed, &bound(hi)).is_le()
        };
        let checks: [(bool, Rule); 10] = [
            (
                v.strict_numeric && calc::value::text_to_num(text).is_none(),
                Rule::Numeric,
            ),
            (
                v.strict_four_digit_year && !four_digit_date(text, &typed),
                Rule::Date,
            ),
            (
                v.strict_time
                    && !match &typed {
                        Value::Time(s) => time_of_day(s),
                        _ => calc::value::parse_time(text).is_some_and(|s| time_of_day(&s)),
                    },
                Rule::Time,
            ),
            (v.not_empty && !set, Rule::NotEmpty),
            (v.unique && others().any(|(_, o)| same(o)), Rule::Unique),
            (
                v.existing && !others().any(|(_, o)| same(o)),
                Rule::Existing,
            ),
            (
                f.def.validation_value_list().is_some_and(|vl| {
                    !self
                        .value_list_values(vl)
                        .iter()
                        .any(|x| x.eq_ignore_ascii_case(text))
                }),
                Rule::ValueList,
            ),
            (f.range.as_ref().is_some_and(|r| !in_range(r)), Rule::Range),
            (false, Rule::Calculation),
            (
                f.def
                    .max_size()
                    .is_some_and(|n| text.chars().count() > n as usize),
                Rule::MaxChars,
            ),
        ];
        for (failed, rule) in checks {
            if !entry && matches!(rule, Rule::ValueList | Rule::Range | Rule::MaxChars) {
                continue;
            }
            let failed = match rule {
                // Only evaluated when the earlier rules pass, and not on an
                // empty value (FileMaker 26).
                Rule::Calculation => {
                    set && f.validation_calc.as_ref().is_some_and(|src| {
                        let scope = Scope {
                            found: None,
                            self_value: Some(typed.clone()),
                        };
                        !self
                            .eval_source_in(src, t.id, f.context, Some(record), &scope)
                            .is_ok_and(|r| r.truth())
                    })
                }
                Rule::NotEmpty => failed,
                _ => failed && set,
            };
            if failed {
                return Some(rule);
            }
        }
        None
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

    /// Whether a relationship path leads from occurrence `from` to `to`.
    pub fn is_related(&self, from: u32, to: u32) -> bool {
        from != to && self.path(from, to).is_some()
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
        let fields = |p: &fmp12::graph::Predicate| {
            if forward {
                (p.left_field, p.right_field)
            } else {
                (p.right_field, p.left_field)
            }
        };
        // This side's values once per record, not once per candidate.
        let sources: Vec<Vec<Value>> = records
            .iter()
            .map(|&r| {
                rel.predicates
                    .iter()
                    .map(|p| self.value(ta.id, r, fields(p).0))
                    .collect()
            })
            .filter(|vals: &Vec<Value>| {
                // An empty key matches nothing (except through ×).
                rel.predicates.iter().zip(vals).all(|(p, v)| {
                    p.op == fmp12::graph::JoinOp::Cartesian || !v.to_text().is_empty()
                })
            })
            .collect();
        if sources.is_empty() {
            return Vec::new();
        }
        (0..tb.records.len())
            .filter(|&j| {
                // The other side's values once per candidate.
                let mut theirs: Vec<Option<Value>> = vec![None; rel.predicates.len()];
                sources.iter().any(|vals| {
                    rel.predicates.iter().enumerate().all(|(k, p)| {
                        let vb = theirs[k].get_or_insert_with(|| self.value(tb.id, j, fields(p).1));
                        // The predicate reads left op right.
                        let (l, r) = if forward {
                            (&vals[k], &*vb)
                        } else {
                            (&*vb, &vals[k])
                        };
                        joins(p.op, l, r)
                    })
                })
            })
            .collect()
    }
}

/// Whether a predicate `left op right` holds, as FileMaker matches
/// relationships: an empty value matches nothing, except through ×. A
/// value of several lines is a multi-key: each line is a key (as the
/// field's index has it), and the predicate holds when one line of the
/// left holds with one line of the right (each predicate on its own: the
/// "type-ahead" range `low¶high ≥ x` and `low¶high ≤ x`).
fn joins(op: fmp12::graph::JoinOp, left: &Value, right: &Value) -> bool {
    if op == fmp12::graph::JoinOp::Cartesian {
        return true;
    }
    let lines = |v: &Value| -> Vec<Value> {
        let t = v.to_text();
        if t.contains(['\r', '\n']) {
            t.split(['\r', '\n'])
                .filter(|l| !l.is_empty())
                .map(Value::text)
                .collect()
        } else {
            vec![v.clone()]
        }
    };
    let rights = lines(right);
    lines(left)
        .iter()
        .any(|l| rights.iter().any(|r| joins_key(op, l, r)))
}

fn joins_key(op: fmp12::graph::JoinOp, left: &Value, right: &Value) -> bool {
    use fmp12::graph::JoinOp as J;
    use std::cmp::Ordering as O;
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

/// File name for `data` in folder `dir` of open external storage: its own
/// name, or `name_1.ext`, `name_2.ext`… when another file has it (as
/// FileMaker Pro 26 names a second red.png red_1.png).
fn external_name(t: &Table, dir: &str, name: &str, data: &[u8]) -> String {
    let taken = |f: &str| {
        t.containers.values().any(|s| {
            s.external().is_some_and(|x| {
                x.dir == dir
                    && x.file.eq_ignore_ascii_case(f)
                    && s.external_data.as_deref() != Some(data)
            })
        })
    };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut n = name.to_string();
    let mut i = 1;
    while taken(&n) {
        n = format!("{stem}_{i}{ext}");
        i += 1;
    }
    n
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

/// A window's found set: record indexes of `table` in the order shown, and
/// the fields it is sorted by (empty when unsorted).
#[derive(Debug, Clone, Copy)]
pub struct FoundSet<'a> {
    pub table: u32,
    pub records: &'a [usize],
    pub sort: &'a [u32],
}

/// What a calculation sees besides its record: the window's found set
/// (`None`: every record, unsorted) and `Self` (the value of the field
/// being validated or auto-entered, or of the layout object).
#[derive(Debug, Clone, Default)]
pub struct Scope<'a> {
    pub found: Option<FoundSet<'a>>,
    pub self_value: Option<Value>,
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
    found: Option<FoundSet<'a>>,
    self_value: Option<Value>,
}

impl<'a, 'e> RecordContext<'a, 'e> {
    fn new(
        db: &'a Database,
        table: u32,
        to: u32,
        record: Option<usize>,
        scope: &Scope<'a>,
        env: Option<&'e mut dyn Env>,
    ) -> Self {
        RecordContext {
            db,
            table,
            to,
            record: record.unwrap_or(usize::MAX),
            rep: 1,
            vars: BTreeMap::new(),
            env,
            found: scope.found.filter(|f| f.table == table),
            self_value: scope.self_value.clone(),
        }
    }

    /// The found set of this calculation's table, in order.
    fn found_records(&self) -> Vec<usize> {
        match self.found {
            Some(f) => f.records.to_vec(),
            None => (0..self.db.table(self.table).map_or(0, |t| t.records.len())).collect(),
        }
    }

    /// `Get` flags of the window's found set and record.
    fn found_flag(&self, flag: &str) -> Option<Value> {
        let t = self.db.table(self.table)?;
        let pos = || match self.found {
            Some(f) => f.records.iter().position(|r| *r == self.record),
            None => (self.record < t.records.len()).then_some(self.record),
        };
        let count = self.found.map_or(t.records.len(), |f| f.records.len());
        let int = |n: usize| Some(Value::int(n as i64));
        match flag.to_ascii_lowercase().as_str() {
            "foundcount" => int(count),
            "totalrecordcount" => int(t.records.len()),
            "recordnumber" => int(pos().map_or(0, |p| p + 1)),
            "recordid" => int(t.records.get(self.record).map_or(0, |r| r.id as usize)),
            // 0 unsorted, 1 sorted (2, semi-sorted, after records were added).
            "sortstate" => int(self.found.is_some_and(|f| !f.sort.is_empty()) as usize),
            "layoutcount" => int(self.db.layouts.len()),
            // Outside a script.
            "lasterror"
            | "errorcapturestate"
            | "recordopenstate"
            | "recordopencount"
            | "transactionopenstate"
            | "windowmode" => int(0),
            "allowabortstate" => int(1),
            "scriptname" | "scriptparameter" | "scriptresult" | "lasterrordetail"
            | "lasterrorlocation" => Some(Value::empty()),
            _ => None,
        }
    }
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
        let (t, records, f, own) = self.resolve(table, name)?;
        let rep = match (repetition, self.rep) {
            (1, n) if n > 1 && repetitions(f) > 1 => n,
            (1, n) if n > 1 && !extended => return Ok(Value::empty()),
            (r, _) => r as u32,
        };
        // A related summary field summarizes the related records.
        let found = if own {
            self.found
        } else {
            Some(FoundSet {
                table: t,
                records: &records,
                sort: &[],
            })
        };
        Ok(records
            .first()
            .map_or_else(Value::empty, |r| self.db.value_in(t, *r, f.id, rep, found)))
    }
}

impl calc::Context for RecordContext<'_, '_> {
    fn custom_function(&self, name: &str) -> Option<std::sync::Arc<calc::CustomFunction>> {
        self.db
            .custom_functions
            .by_name
            .get(&name.to_lowercase())
            .cloned()
    }

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
        // A related field: the n-th related record; otherwise the n-th
        // record of the found set.
        if table.is_some()
            && let Ok((t, records, f, false)) = self.resolve(table, name)
        {
            return Ok(records
                .get(n.wrapping_sub(1))
                .map_or_else(Value::empty, |r| self.db.value(t, *r, f.id)));
        }
        let (t, _, f, _) = self.resolve(table, name)?;
        Ok(self
            .found_records()
            .get(n.wrapping_sub(1))
            .map_or_else(Value::empty, |r| {
                self.db.value_in(t, *r, f.id, 1, self.found)
            }))
    }

    /// The summary of the found set (break field = summary field), or of
    /// the record's group when the found set is sorted by the break field;
    /// empty otherwise, as in FileMaker.
    fn summary(
        &mut self,
        summary: (Option<&str>, &str),
        break_field: (Option<&str>, &str),
    ) -> Result<Value, calc::EvalError> {
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
        let found = self.found_records();
        let sort = self.found.map_or(&[][..], |f| f.sort);
        if sf == bf {
            return Ok(summary::summarize_sorted(
                self.db, t.id, sf, &found, &found, sort,
            ));
        }
        let (Some(level), Some(pos)) = (
            sort.iter().position(|f| *f == bf),
            found.iter().position(|r| *r == self.record),
        ) else {
            return Ok(Value::empty());
        };
        let group = summary::groups(self.db, t.id, &found, &sort[..=level])
            .into_iter()
            .find(|g| g.contains(&pos))
            .map_or_else(Vec::new, |g| found[g].to_vec());
        Ok(summary::summarize_sorted(
            self.db, t.id, sf, &group, &found, sort,
        ))
    }

    fn self_value(&mut self) -> Result<Value, calc::EvalError> {
        self.self_value
            .clone()
            .ok_or_else(|| calc::eval::unavailable("Self"))
    }

    fn record_ids(&mut self, _table: Option<&str>) -> Result<Vec<u64>, calc::EvalError> {
        let Some(t) = self.db.table(self.table) else {
            return Ok(Vec::new());
        };
        Ok(self
            .found_records()
            .iter()
            .filter_map(|r| t.records.get(*r).map(|r| r.id as u64))
            .collect())
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
        self.env
            .as_mut()
            .and_then(|e| e.get(flag))
            .or_else(|| self.db.account_flag(flag))
            .or_else(|| self.found_flag(flag))
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

/// A date (or timestamp) with a four-digit year ("Strict data type: 4-digit
/// year date").
fn four_digit_date(text: &str, typed: &Value) -> bool {
    let date = text.split_whitespace().next().unwrap_or("");
    (matches!(typed, Value::Date(_) | Value::Timestamp(_))
        || calc::value::parse_date(date).is_some())
        && date
            .split(|c: char| !c.is_ascii_digit())
            .any(|p| p.len() == 4)
}

/// A time of day, 0:00:00 to 23:59:59 ("Strict data type: time of day").
fn time_of_day(secs: &calc::num::Num) -> bool {
    *secs >= calc::num::int(0) && *secs < calc::num::int(86_400)
}

/// A failed validation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    NotEmpty,
    Unique,
    Existing,
    /// Strict data type: numeric only.
    Numeric,
    /// Strict data type: 4-digit year date.
    Date,
    /// Strict data type: time of day.
    Time,
    /// Not a value of the field's "member of value list".
    ValueList,
    /// Outside the field's "in range" bounds.
    Range,
    /// The field's validation calculation is false.
    Calculation,
    /// More characters than the field's maximum.
    MaxChars,
}

impl Rule {
    /// FileMaker's error code (`Get ( LastError )` after a commit fails).
    pub fn code(self) -> u32 {
        match self {
            Rule::Date => 500,
            Rule::Time => 501,
            Rule::Numeric => 502,
            Rule::Range => 503,
            Rule::Unique => 504,
            Rule::Existing => 505,
            Rule::ValueList => 506,
            Rule::Calculation => 507,
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
    /// Validated always, not only during data entry: imports and Commit
    /// Records' "skip data entry validation" still check it.
    pub always: bool,
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
            validation_calc: None,
            range: None,
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
            access: Default::default(),
            privilege_sets: vec![],
            locale: Locale::default(),
            custom_functions: Default::default(),
            base_directories: vec![],
            pictures: BTreeMap::new(),
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

    #[test]
    fn completions_offer_entered_values() {
        let db = sample();
        assert_eq!(db.completions(T, CITY, "r"), ["Rome"]);
        assert_eq!(db.completions(T, CITY, "O"), ["Oslo"]);
        assert!(db.completions(T, CITY, "rome").is_empty());
        assert!(db.completions(T, CITY, "").is_empty());
    }

    struct NoNames;
    impl fmp12::calc::Resolve for NoNames {
        fn occurrences(&self) -> Vec<(u32, String)> {
            Vec::new()
        }
        fn fields(&self, _: u32) -> Vec<(u32, String)> {
            Vec::new()
        }
    }

    /// A layout object's stored formula sees the object's value as `Self`
    /// (conditional formats, hide conditions, tooltips) and the window's
    /// found set.
    #[test]
    fn eval_with_self() {
        let db = sample();
        let compile = |s: &str| fmp12::calc::compile(s, 0, &NoNames).unwrap().tokens;
        let tokens = compile("Self > 40 and Self < 100");
        let ev = |v: i64| db.eval_with_self(&tokens, T, 0, Some(0), &Value::int(v), None);
        assert_eq!(ev(50).to_text(), "1");
        assert_eq!(ev(30).to_text(), "0");
        // Without a value, Self is not available.
        assert_eq!(db.eval_source("Self", T, 0, Some(0)).to_text(), "?");
        let found = [2, 0];
        let fs = FoundSet {
            table: T,
            records: &found,
            sort: &[NAME],
        };
        let tokens = compile("Get ( RecordNumber ) & \"/\" & Get ( FoundCount )");
        let v = db.eval_with_self(&tokens, T, 0, Some(0), &Value::empty(), Some(fs));
        assert_eq!(v.to_text(), "2/2");
    }
}
