//! Design data a host hands to the design functions (FieldNames,
//! FieldType, RelationInfo, GetTableDDL…) through [`crate::Context::schema`].
//! The functions format it the way FileMaker does; the host only describes
//! the file.

use crate::value::Value;

#[derive(Clone, Debug, Default)]
pub struct Schema {
    /// File name without extension, as `Get ( FileName )` returns it.
    pub file: String,
    pub base_tables: Vec<BaseTable>,
    pub occurrences: Vec<Occurrence>,
    pub relationships: Vec<Relationship>,
    pub layouts: Vec<Layout>,
    /// Scripts in catalog order. Folders are listed by name and separators
    /// as `-`, as the host's script catalog has them.
    pub scripts: Vec<Named>,
    pub value_lists: Vec<ValueList>,
}

#[derive(Clone, Debug, Default)]
pub struct Named {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct BaseTable {
    pub id: u64,
    pub name: String,
    pub comment: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Storage {
    #[default]
    Standard,
    StoredCalc,
    UnstoredCalc,
    Summary,
    Global,
    ExternalSecure,
    ExternalOpen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataType {
    #[default]
    Text,
    Number,
    Date,
    Time,
    Timestamp,
    Container,
}

#[derive(Clone, Debug, Default)]
pub struct Field {
    pub id: u64,
    pub name: String,
    pub storage: Storage,
    pub data_type: DataType,
    pub indexed: bool,
    /// Maximum repetitions; 1 for a non-repeating field.
    pub repetitions: u32,
    pub comment: String,
    /// The AI annotation (FieldAnnotation, GetTableDDL, GetFieldsOnLayout).
    pub annotation: String,
    /// "Customize field display names": (key, name) pairs.
    pub display_names: Vec<(String, String)>,
    /// Next auto-enter serial value, when the field has one.
    pub next_serial: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Occurrence {
    pub id: u64,
    pub name: String,
    pub base_table: String,
    /// Data source name of the file the base table lives in.
    pub source: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelOptions {
    /// Allow creation of records in this side's table via the relationship.
    pub create: bool,
    /// Delete this side's records when a record on the other side is deleted.
    pub delete: bool,
    /// This side's records are sorted.
    pub sorted: bool,
}

/// One relationship between two table occurrences. Each criterion reads
/// `left::field op right::field`.
#[derive(Clone, Debug, Default)]
pub struct Relationship {
    pub left: String,
    pub right: String,
    /// (left field, operator as FileMaker writes it: `=`, `≠`, `<`, `≤`,
    /// `>`, `≥`, `>=`… or `x` for cartesian, right field).
    pub criteria: Vec<(String, String, String)>,
    pub left_options: RelOptions,
    pub right_options: RelOptions,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub id: u64,
    pub name: String,
    /// The layout's table occurrence.
    pub occurrence: String,
    pub objects: Vec<LayoutObject>,
}

#[derive(Clone, Debug, Default)]
pub struct LayoutObject {
    /// Object name; empty when unnamed.
    pub name: String,
    /// `TO::Field` for field objects.
    pub field: Option<String>,
    /// left, top, right, bottom in points from the layout's top-left corner.
    pub bounds: [f64; 4],
    /// Degrees, counter-clockwise.
    pub rotation: f64,
    /// FieldStyle word: Standard, Scrolling, Popuplist, Popupmenu, Checkbox,
    /// RadioButton or Calendar.
    pub style: String,
    pub value_list: String,
    /// Repetitions shown, and whether they stack vertically.
    pub repetitions_shown: u32,
    pub vertical: bool,
    /// Tab controls, slide controls, portals, groups and button bars hold
    /// other objects.
    pub is_group: bool,
    pub children: Vec<LayoutObject>,
}

#[derive(Clone, Debug, Default)]
pub struct ValueList {
    pub id: u64,
    pub name: String,
    pub items: Vec<String>,
}

/// A table's records as ExecuteSQL sees them.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

impl Schema {
    pub fn base_table(&self, name: &str) -> Option<&BaseTable> {
        self.base_tables.iter().find(|t| eq(&t.name, name))
    }

    pub fn occurrence(&self, name: &str) -> Option<&Occurrence> {
        self.occurrences.iter().find(|t| eq(&t.name, name))
    }

    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts.iter().find(|l| eq(&l.name, name))
    }

    pub fn value_list(&self, name: &str) -> Option<&ValueList> {
        self.value_lists.iter().find(|l| eq(&l.name, name))
    }

    /// The base table behind a table occurrence.
    pub fn occurrence_table(&self, to: &str) -> Option<&BaseTable> {
        self.base_table(&self.occurrence(to)?.base_table)
    }

    /// `TO::Field`, or `Field` in `current`'s table occurrence.
    pub fn field<'a>(
        &'a self,
        name: &str,
        current: Option<&str>,
    ) -> Option<(&'a Occurrence, &'a Field)> {
        let (to, f) = match name.split_once("::") {
            Some((t, f)) => (t, f),
            None => (current?, name),
        };
        let o = self.occurrence(to)?;
        let field = self
            .base_table(&o.base_table)?
            .fields
            .iter()
            .find(|x| eq(&x.name, f))?;
        Some((o, field))
    }
}

impl BaseTable {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| eq(&f.name, name))
    }
}
