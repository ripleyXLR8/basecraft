//! A hosted file as an engine [`Database`]: one table per layout (its
//! fields as the layout shows them), loaded from the Data API, so Browse,
//! Find, sort, List and Table views work on it as on a local file. Changes
//! go back to the server with [`Remote::sync`]: new records are created,
//! edited ones patched (with their modification id, so someone else's
//! change is not overwritten), deleted ones deleted.
//!
//! ponytail: a snapshot of each layout's records (up to [`MAX_RECORDS`]);
//! find and sort run on it locally. Server-side find for files larger than
//! that, and live refresh, come later.

use std::collections::BTreeMap;

use engine::{Database, Field, Table};
use fmp12::layouts::{Bounds, Layout, LayoutObject, Part};

use crate::{Client, Error, LayoutMeta, Query, RemoteRecord, Result};

/// Records loaded per layout.
pub const MAX_RECORDS: usize = 50_000;
const PAGE: usize = 1_000;

/// First table occurrence id (FileMaker's ids start at 128).
const TO_BASE: u32 = 128;

/// What the server last had for one record.
#[derive(Debug, Clone)]
struct Known {
    remote_id: u32,
    mod_id: u32,
    values: BTreeMap<u32, String>,
}

/// One layout of the hosted file and its engine table.
#[derive(Debug, Clone)]
pub struct RemoteLayout {
    pub name: String,
    pub table: u32,
    /// Engine field id → Data API field name and whether it is written
    /// back (normal, not global, not a container; related fields are not
    /// loaded).
    fields: Vec<(u32, String, bool)>,
    /// Engine record id → the server's record.
    known: BTreeMap<u32, Known>,
    /// More records on the server than were loaded.
    pub truncated: bool,
}

/// A hosted file opened through the Data API.
#[derive(Debug)]
pub struct Remote {
    pub client: Client,
    pub layouts: Vec<RemoteLayout>,
}

/// Engine data type of a Data API field result. Values stay as the Data
/// API writes them: dates `MM/dd/yyyy` (the engine's US format), times
/// `HH:mm:ss`, numbers in JSON's notation.
fn data_type(result: &str) -> u8 {
    match result {
        "number" => 2,
        "date" => 3,
        "time" => 4,
        "timestamp" => 5,
        // ponytail: containers come as URLs and stay text; downloading
        // them into the GUI comes later (`Client::container` has it).
        _ => 1,
    }
}

fn field(id: u32, name: &str, kind: u8, dt: u8, reps: u32) -> Field {
    let mut options = vec![kind, dt];
    // Repetitions: options bytes 24–25, as fmp12::schema::Field reads them.
    options.resize(26, 0);
    options[24..26].copy_from_slice(&(reps.min(u16::MAX as u32) as u16).to_be_bytes());
    let def = fmp12::schema::Field {
        name: name.into(),
        options,
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

/// A plain form layout: one label and one edit box per field.
fn form(id: u32, name: &str, to: u32, fields: &[(u32, String, bool)]) -> Layout {
    let row = 28.0;
    let mut objects = Vec::new();
    for (i, (f, n, _)) in fields.iter().enumerate() {
        let top = 10.0 + i as f64 * row;
        objects.push(LayoutObject {
            id: 2 * i as u64 + 1,
            kind: 2,
            bounds: Bounds {
                top,
                left: 10.0,
                bottom: top + 20.0,
                right: 150.0,
            },
            text: Some(n.clone()),
            ..Default::default()
        });
        objects.push(LayoutObject {
            id: 2 * i as u64 + 2,
            kind: 1,
            bounds: Bounds {
                top,
                left: 160.0,
                bottom: top + 20.0,
                right: 460.0,
            },
            field: Some((to as u64, *f as u64, 1)),
            ..Default::default()
        });
    }
    Layout {
        id,
        name: name.into(),
        table_occurrence: to as u64,
        width: 480.0,
        parts: vec![Part {
            kind: 4,
            top: 0.0,
            height: 20.0 + fields.len() as f64 * row,
            ..Default::default()
        }],
        objects,
        ..Default::default()
    }
}

/// Engine table and layout for one Data API layout.
fn layout_table(i: usize, name: &str, meta: &LayoutMeta) -> (RemoteLayout, Table, Layout) {
    let table = i as u32 + 1;
    let mut fields = Vec::new();
    let mut engine_fields = Vec::new();
    for m in meta.fields.iter().filter(|m| !m.name.contains("::")) {
        let id = engine_fields.len() as u32 + 1;
        let kind = if m.kind == "normal" { 1 } else { 2 };
        engine_fields.push(field(
            id,
            &m.name,
            kind,
            data_type(&m.result),
            m.repetitions,
        ));
        fields.push((
            id,
            m.name.clone(),
            m.kind == "normal" && !m.global && m.result != "container",
        ));
    }
    let to = TO_BASE + i as u32;
    (
        RemoteLayout {
            name: name.into(),
            table,
            fields: fields.clone(),
            known: BTreeMap::new(),
            truncated: false,
        },
        Table::new(table, name, engine_fields),
        form(i as u32 + 1, name, to, &fields),
    )
}

/// Engine values of a record (repetitions `Field(2)`… as rep keys).
fn values(l: &RemoteLayout, r: &RemoteRecord) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    for (id, name, _) in &l.fields {
        if let Some(v) = r.fields.get(name)
            && !v.is_empty()
        {
            out.insert(*id, v.clone());
        }
        for (k, v) in &r.fields {
            if let Some(rep) = k
                .strip_prefix(name.as_str())
                .and_then(|s| s.strip_prefix('('))
                .and_then(|s| s.strip_suffix(')'))
                .and_then(|s| s.parse::<u32>().ok())
                && rep > 1
                && !v.is_empty()
            {
                out.insert(engine::rep_key(*id, rep), v.clone());
            }
        }
    }
    out
}

/// The Data API name of an engine value key (`Field(2)` for repetition 2).
fn api_name(l: &RemoteLayout, key: u32) -> Option<(String, bool)> {
    let (id, rep) = engine::split_rep_key(key);
    let (_, name, writable) = l.fields.iter().find(|(f, _, _)| *f == id)?;
    Some(if rep > 1 {
        (format!("{name}({rep})"), *writable)
    } else {
        (name.clone(), *writable)
    })
}

impl Remote {
    /// Loads every layout of the file (fields and records) into a new
    /// engine database. Layouts the account cannot read are skipped.
    pub fn open(client: Client) -> Result<(Remote, Database)> {
        let names = client.layouts()?;
        let mut db = Database {
            tables: Vec::new(),
            occurrences: Vec::new(),
            relationships: Vec::new(),
            layouts: Vec::new(),
            themes: Vec::new(),
            value_lists: Vec::new(),
            accounts: Vec::new(),
            privilege_sets: Vec::new(),
            // The Data API's formats: MM/dd/yyyy, HH:mm:ss.
            locale: fmp12::schema::Locale {
                am: String::new(),
                pm: String::new(),
                ..Default::default()
            },
            access: Default::default(),
            custom_functions: Default::default(),
            base_directories: Vec::new(),
        };
        let mut layouts = Vec::new();
        for name in names {
            let meta = match client.layout(&name) {
                Ok(m) => m,
                Err(e) if e.code().is_some() => continue,
                Err(e) => return Err(e),
            };
            let i = layouts.len();
            let (mut rl, mut table, layout) = layout_table(i, &name, &meta);
            rl.truncated = load(&client, &mut rl, &mut table)?;
            db.occurrences.push(fmp12::graph::TableOccurrence {
                id: TO_BASE + i as u32,
                name: name.clone(),
                base_table: table.id,
                ..Default::default()
            });
            db.tables.push(table);
            db.layouts.push(layout);
            layouts.push(rl);
        }
        Ok((Remote { client, layouts }, db))
    }

    /// Sends the changes made to `db` since the last load or sync. Returns
    /// the errors (a record changed on the server meanwhile: 306); the
    /// other changes still go through.
    pub fn sync(&mut self, db: &Database) -> Vec<Error> {
        let mut errors = Vec::new();
        for l in &mut self.layouts {
            let Some(t) = db.table(l.table) else { continue };
            let present: Vec<u32> = t.records.iter().map(|r| r.id).collect();
            let gone: Vec<u32> = l
                .known
                .keys()
                .filter(|k| !present.contains(k))
                .copied()
                .collect();
            for id in gone {
                if let Some(k) = l.known.remove(&id)
                    && let Err(e) = self.client.delete(&l.name, k.remote_id)
                {
                    errors.push(e);
                }
            }
            for r in &t.records {
                let old = l.known.get(&r.id);
                let empty = BTreeMap::new();
                let before = old.map_or(&empty, |o| &o.values);
                let mut keys: Vec<u32> = r.values.keys().chain(before.keys()).copied().collect();
                keys.sort_unstable();
                keys.dedup();
                let changed: Vec<(String, String)> = keys
                    .into_iter()
                    .filter_map(|k| {
                        let now = r.values.get(&k).map_or("", String::as_str);
                        let was = before.get(&k).map_or("", String::as_str);
                        let (name, writable) = api_name(l, k)?;
                        (now != was && writable).then(|| (name, now.to_string()))
                    })
                    .collect();
                if old.is_some() && changed.is_empty() {
                    continue;
                }
                let done = match old {
                    Some(o) => self
                        .client
                        .edit(&l.name, o.remote_id, &changed, Some(o.mod_id))
                        .map(|m| (o.remote_id, m)),
                    None => self.client.create(&l.name, &changed),
                };
                match done {
                    Ok((remote_id, mod_id)) => {
                        l.known.insert(
                            r.id,
                            Known {
                                remote_id,
                                mod_id,
                                values: r.values.clone(),
                            },
                        );
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
        errors
    }

    /// Reloads one layout's records from the server (after a conflict, or
    /// to see other users' changes).
    pub fn refresh(&mut self, db: &mut Database, table: u32) -> Result<()> {
        let Some(l) = self.layouts.iter_mut().find(|l| l.table == table) else {
            return Ok(());
        };
        let Some(t) = db.tables.iter_mut().find(|t| t.id == table) else {
            return Ok(());
        };
        t.records.clear();
        l.known.clear();
        l.truncated = load(&self.client, l, t)?;
        Ok(())
    }

    /// The layout of an engine table.
    pub fn layout_of(&self, table: u32) -> Option<&RemoteLayout> {
        self.layouts.iter().find(|l| l.table == table)
    }
}

/// Loads a layout's records into its table; true when there were more.
fn load(client: &Client, l: &mut RemoteLayout, t: &mut Table) -> Result<bool> {
    let mut offset = 1;
    loop {
        let page = client.records(
            &l.name,
            &Query {
                offset,
                limit: Some(PAGE),
                ..Default::default()
            },
        )?;
        let n = page.records.len();
        for r in &page.records {
            let i = t.new_record();
            let v = values(l, r);
            t.records[i].values = v.clone();
            l.known.insert(
                t.records[i].id,
                Known {
                    remote_id: r.id,
                    mod_id: r.mod_id,
                    values: v,
                },
            );
        }
        offset += n;
        if n < PAGE || offset > page.found {
            return Ok(false);
        }
        if offset > MAX_RECORDS {
            return Ok(true);
        }
    }
}
