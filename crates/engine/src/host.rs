//! Describes the database to the calculation engine (design functions,
//! ExecuteSQL, GetNthRecord…).

use calc::schema as cs;
use fmp12::schema::{DataType, FieldKind};

use crate::Database;

fn data_type(d: DataType) -> cs::DataType {
    match d {
        DataType::Number => cs::DataType::Number,
        DataType::Date => cs::DataType::Date,
        DataType::Time => cs::DataType::Time,
        DataType::Timestamp => cs::DataType::Timestamp,
        DataType::Container => cs::DataType::Container,
        _ => cs::DataType::Text,
    }
}

impl Database {
    /// Schema description for calc's design functions.
    pub fn calc_schema(&self, file: &str) -> cs::Schema {
        let to_name = |id: u32| {
            self.occurrences
                .iter()
                .find(|o| o.id == id)
                .map_or(String::new(), |o| o.name.clone())
        };
        let field_name = |to: u32, f: u32| {
            self.table_of(to)
                .and_then(|t| t.field(f))
                .map_or(String::new(), |f| f.name.clone())
        };
        cs::Schema {
            file: file.to_string(),
            base_tables: self
                .tables
                .iter()
                .map(|t| cs::BaseTable {
                    id: t.id as u64,
                    name: t.name.clone(),
                    comment: String::new(),
                    fields: t
                        .fields
                        .iter()
                        .map(|f| cs::Field {
                            id: f.id as u64,
                            name: f.name.clone(),
                            storage: match f.kind {
                                FieldKind::Summary => cs::Storage::Summary,
                                _ if f.def.global() => cs::Storage::Global,
                                FieldKind::Calculated if f.def.stored() => cs::Storage::StoredCalc,
                                FieldKind::Calculated => cs::Storage::UnstoredCalc,
                                _ => cs::Storage::Standard,
                            },
                            data_type: data_type(f.data_type),
                            indexed: f.def.indexing() != fmp12::schema::Indexing::None,
                            repetitions: f.def.max_repetitions().max(1) as u32,
                            comment: f.def.comment.clone(),
                            ..Default::default()
                        })
                        .collect(),
                })
                .collect(),
            occurrences: self
                .occurrences
                .iter()
                .map(|o| cs::Occurrence {
                    id: o.id as u64,
                    name: o.name.clone(),
                    base_table: self
                        .table(o.base_table)
                        .map_or(String::new(), |t| t.name.clone()),
                    ..Default::default()
                })
                .collect(),
            relationships: self
                .relationships
                .iter()
                .map(|r| cs::Relationship {
                    left: to_name(r.left),
                    right: to_name(r.right),
                    criteria: r
                        .predicates
                        .iter()
                        .map(|p| {
                            (
                                field_name(r.left, p.left_field),
                                p.op.symbol().replace('×', "x"),
                                field_name(r.right, p.right_field),
                            )
                        })
                        .collect(),
                    left_options: cs::RelOptions {
                        create: r.left_cascade_create(),
                        delete: r.left_cascade_delete(),
                        sorted: !r.left_sort.is_empty(),
                    },
                    right_options: cs::RelOptions {
                        create: r.right_cascade_create(),
                        delete: r.right_cascade_delete(),
                        sorted: !r.right_sort.is_empty(),
                    },
                })
                .collect(),
            layouts: self
                .layouts
                .iter()
                .filter(|l| l.table_occurrence != 0)
                .map(|l| cs::Layout {
                    id: l.id as u64,
                    name: l.name.clone(),
                    occurrence: to_name((l.table_occurrence & 0xFFF) as u32),
                    // ponytail: objects (names, nesting) not described yet.
                    objects: Vec::new(),
                })
                .collect(),
            scripts: Vec::new(),
            value_lists: self
                .value_lists
                .iter()
                .map(|v| cs::ValueList {
                    id: v.id as u64,
                    name: v.name.clone(),
                    items: self.value_list_values(v.id),
                })
                .collect(),
        }
    }

    /// Rows of a base table for ExecuteSQL.
    pub fn calc_rows(&self, table: &str) -> Option<cs::Rows> {
        let t = self
            .tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(table))
            .or_else(|| {
                let o = self
                    .occurrences
                    .iter()
                    .find(|o| o.name.eq_ignore_ascii_case(table))?;
                self.table(o.base_table)
            })?;
        Some(cs::Rows {
            columns: t.fields.iter().map(|f| f.name.clone()).collect(),
            rows: (0..t.records.len())
                .map(|r| t.fields.iter().map(|f| self.value(t.id, r, f.id)).collect())
                .collect(),
        })
    }
}
