//! A hosted file as an engine [`Database`]: one table per layout (its
//! fields as the layout shows them), loaded from the Data API, so Browse,
//! Find, sort, List and Table views work on it as on a local file. Changes
//! go back to the server with [`Remote::sync`]: new records are created,
//! edited ones patched (with their modification id, so someone else's
//! change is not overwritten), deleted ones deleted.
//!
//! Related fields (`Occurrence::Field`, first related record) are loaded
//! read-only; portals become a related table each (the rows of
//! `portalData`, matched to their record by a hidden key) drawn as a
//! portal under the fields; value lists of the layout's metadata become
//! the file's value lists, attached to their fields as pop-up menus or
//! drop-down lists.
//!
//! ponytail: a snapshot of each layout's records (up to [`MAX_RECORDS`]);
//! find and sort run on it locally. Server-side find for files larger than
//! that, and live refresh, come later. Portal rows are not written back.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    /// Hidden field holding the record's id on the server (portals match on it).
    key: u32,
    /// Container fields: uploaded when a file is put in them.
    containers: Vec<u32>,
    pub portals: Vec<RemotePortal>,
}

/// A portal of a layout, as a related table of its own.
#[derive(Debug, Clone)]
pub struct RemotePortal {
    /// `portalData` key: the portal's object name, else its occurrence.
    pub name: String,
    pub table: u32,
    /// Engine field id → `Occurrence::Field`; field 1 is the parent's key.
    fields: Vec<(u32, String)>,
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
        // Containers come as URLs, downloaded when a record is shown
        // ([`Remote::fetch_containers`]).
        "container" => 6,
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
        validation_calc: None,
        range: None,
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
    for m in &meta.fields {
        let id = engine_fields.len() as u32 + 1;
        let kind = if m.kind == "normal" { 1 } else { 2 };
        engine_fields.push(field(
            id,
            &m.name,
            kind,
            data_type(&m.result),
            m.repetitions,
        ));
        // Related fields show the first related record, read-only.
        let related = m.name.contains("::");
        fields.push((
            id,
            m.name.clone(),
            m.kind == "normal" && !m.global && m.result != "container" && !related,
        ));
    }
    let containers = meta
        .fields
        .iter()
        .zip(&fields)
        .filter(|(m, _)| {
            m.result == "container" && m.kind == "normal" && !m.global && !m.name.contains("::")
        })
        .map(|(_, (id, _, _))| *id)
        .collect();
    let key = engine_fields.len() as u32 + 1;
    engine_fields.push(field(key, "__key", 1, 1, 1));
    let to = TO_BASE + i as u32;
    (
        RemoteLayout {
            name: name.into(),
            table,
            fields: fields.clone(),
            known: BTreeMap::new(),
            truncated: false,
            key,
            containers,
            portals: Vec::new(),
        },
        Table::new(table, name, engine_fields),
        form(i as u32 + 1, name, to, &fields),
    )
}

/// Value lists of the layout's metadata: the file's value lists (by name,
/// shared between layouts), on their fields as drop-down lists or pop-up
/// menus as the field's display type says.
fn attach_value_lists(meta: &LayoutMeta, l: &RemoteLayout, layout: &mut Layout, db: &mut Database) {
    for m in &meta.fields {
        let (Some(name), Some((fid, _, _))) = (
            m.value_list.as_ref(),
            l.fields.iter().find(|(_, n, _)| *n == m.name),
        ) else {
            continue;
        };
        let Some(values) = meta.value_lists.get(name) else {
            continue;
        };
        let id = match db.value_lists.iter().find(|v| v.name == *name) {
            Some(v) => v.id,
            None => {
                let id = db.value_lists.len() as u32 + 1;
                db.value_lists.push(fmp12::valuelists::ValueList {
                    id,
                    name: name.clone(),
                    source: fmp12::valuelists::Source::Custom(values.join("\r")),
                    raw: Vec::new(),
                });
                id
            }
        };
        let display = match m.display.as_str() {
            "popupMenu" | "checkBox" | "radioButtons" | "selectionList" => 2,
            _ => 1,
        };
        for o in layout
            .objects
            .iter_mut()
            .filter(|o| o.field.is_some_and(|f| f.1 == *fid as u64))
        {
            o.value_list = Some(id);
            o.display = display;
        }
    }
}

/// The layout's portals: a related table each (key → parent), drawn as a
/// portal of five rows below the fields, with a header of field names.
fn add_portals(meta: &LayoutMeta, l: &mut RemoteLayout, layout: &mut Layout, db: &mut Database) {
    let lto = layout.table_occurrence as u32;
    let mut top = layout.parts.first().map_or(0.0, |p| p.height) + 10.0;
    for (name, pfields) in &meta.portals {
        let n = db.tables.len() as u32;
        let (tid, pto) = (10_000 + n, 2048 + n);
        let mut fields = vec![field(1, "__parent", 1, 1, 1)];
        let mut names = Vec::new();
        for (k, m) in pfields.iter().enumerate() {
            let id = k as u32 + 2;
            fields.push(field(id, &m.name, 1, data_type(&m.result), 1));
            names.push((id, m.name.clone()));
        }
        db.tables.push(Table::new(tid, name, fields));
        db.occurrences.push(fmp12::graph::TableOccurrence {
            id: pto,
            name: name.clone(),
            base_table: tid,
            ..Default::default()
        });
        db.relationships.push(fmp12::graph::Relationship {
            id: db.relationships.len() as u32 + 1,
            left: lto,
            right: pto,
            predicates: vec![fmp12::graph::Predicate {
                op: fmp12::graph::JoinOp::Equal,
                left_field: l.key,
                right_field: 1,
            }],
            left_sort: Vec::new(),
            right_sort: Vec::new(),
            raw: Vec::new(),
        });
        // Header, then the portal and its fields (bounds inside the portal).
        let rows = 5u32;
        let width = 460.0 / names.len().max(1) as f64;
        let next = |layout: &Layout| layout.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        let title = next(layout);
        layout.objects.push(LayoutObject {
            id: title,
            kind: 2,
            bounds: Bounds {
                top,
                left: 10.0,
                bottom: top + 20.0,
                right: 470.0,
            },
            text: Some(name.clone()),
            ..Default::default()
        });
        top += 22.0;
        for (k, (_, n)) in names.iter().enumerate() {
            let id = next(layout);
            let left = 10.0 + k as f64 * width;
            layout.objects.push(LayoutObject {
                id,
                kind: 2,
                bounds: Bounds {
                    top,
                    left,
                    bottom: top + 18.0,
                    right: left + width,
                },
                text: Some(n.rsplit("::").next().unwrap_or(n).to_string()),
                ..Default::default()
            });
        }
        top += 20.0;
        let portal = next(layout);
        let row = 24.0;
        layout.objects.push(LayoutObject {
            id: portal,
            kind: 9,
            bounds: Bounds {
                top,
                left: 10.0,
                bottom: top + row * rows as f64,
                right: 470.0,
            },
            portal: Some((pto as u64, rows, 1)),
            ..Default::default()
        });
        for (k, (fid, _)) in names.iter().enumerate() {
            let id = next(layout);
            let left = k as f64 * width;
            layout.objects.push(LayoutObject {
                id,
                kind: 1,
                parent: Some(portal),
                bounds: Bounds {
                    top: 2.0,
                    left: left + 2.0,
                    bottom: row - 2.0,
                    right: left + width - 2.0,
                },
                field: Some((pto as u64, *fid as u64, 1)),
                ..Default::default()
            });
        }
        top += row * rows as f64 + 10.0;
        l.portals.push(RemotePortal {
            name: name.clone(),
            table: tid,
            fields: names,
        });
    }
    if let Some(p) = layout.parts.first_mut() {
        p.height = p.height.max(top);
    }
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
        Remote::open_counting(client, &AtomicUsize::new(0))
    }

    /// [`Remote::open`], counting the records loaded in `loaded` (for a
    /// progress indicator while it runs on another thread).
    pub fn open_counting(client: Client, loaded: &AtomicUsize) -> Result<(Remote, Database)> {
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
            let (mut rl, table, mut layout) = layout_table(i, &name, &meta);
            db.occurrences.push(fmp12::graph::TableOccurrence {
                id: TO_BASE + i as u32,
                name: name.clone(),
                base_table: table.id,
                ..Default::default()
            });
            db.tables.push(table);
            attach_value_lists(&meta, &rl, &mut layout, &mut db);
            add_portals(&meta, &mut rl, &mut layout, &mut db);
            rl.truncated = load(&client, &mut rl, &mut db, loaded)?;
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
            for (index, r) in t.records.iter().enumerate() {
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
                // Files put in container fields: uploaded once the record
                // exists on the server.
                let uploads: Vec<(String, u32, String, Vec<u8>)> = r
                    .values
                    .iter()
                    .filter(|(k, v)| {
                        let (f, _) = engine::split_rep_key(**k);
                        l.containers.contains(&f)
                            && !v.is_empty()
                            && !v.starts_with("http")
                            && before.get(*k) != Some(*v)
                    })
                    .filter_map(|(k, _)| {
                        let (f, rep) = engine::split_rep_key(*k);
                        let (_, name, _) = l.fields.iter().find(|x| x.0 == f)?;
                        match db.value_rep(l.table, index, f, rep) {
                            calc::Value::Container(c) if !c.data.is_empty() => {
                                Some((name.clone(), rep, c.name.clone(), c.data.clone()))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                if old.is_some() && changed.is_empty() && uploads.is_empty() {
                    continue;
                }
                let done = match old {
                    Some(o) if changed.is_empty() => Ok((o.remote_id, o.mod_id)),
                    Some(o) => self
                        .client
                        .edit(&l.name, o.remote_id, &changed, Some(o.mod_id))
                        .map(|m| (o.remote_id, m)),
                    None => self.client.create(&l.name, &changed),
                };
                let done = done.and_then(|(remote_id, mut mod_id)| {
                    for (field, rep, name, data) in &uploads {
                        mod_id = self
                            .client
                            .upload(&l.name, remote_id, field, *rep, name, data)?;
                    }
                    Ok((remote_id, mod_id))
                });
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
        let mut ids: Vec<u32> = l.portals.iter().map(|p| p.table).collect();
        ids.push(table);
        for t in db.tables.iter_mut().filter(|t| ids.contains(&t.id)) {
            t.records.clear();
        }
        l.known.clear();
        l.truncated = load(&self.client, l, db, &AtomicUsize::new(0))?;
        Ok(())
    }

    /// The layout of an engine table.
    pub fn layout_of(&self, table: u32) -> Option<&RemoteLayout> {
        self.layouts.iter().find(|l| l.table == table)
    }

    /// Downloads the containers of one record (their values are the
    /// server's URLs until then) into the record. Returns how many came.
    /// ponytail: on the caller's thread, one record at a time.
    pub fn fetch_containers(
        &mut self,
        db: &mut Database,
        table: u32,
        record: usize,
    ) -> Result<usize> {
        let Some(t) = db.table(table) else {
            return Ok(0);
        };
        let urls: Vec<(u32, String)> = t
            .fields
            .iter()
            .filter(|f| f.data_type == fmp12::schema::DataType::Container)
            .filter_map(|f| {
                let v = t.records.get(record)?.values.get(&f.id)?;
                v.starts_with("http").then(|| (f.id, v.clone()))
            })
            .collect();
        for (f, url) in &urls {
            let data = self.client.container(url)?;
            let path = url.split('?').next().unwrap_or(url);
            let name = path.rsplit('/').next().unwrap_or("file").to_string();
            db.insert_file(table, record, *f, 1, &name, data);
        }
        // What the server has now: nothing to upload back.
        if let (Some(l), Some(r)) = (
            self.layouts.iter_mut().find(|l| l.table == table),
            db.table(table).and_then(|t| t.records.get(record)),
        ) && let Some(k) = l.known.get_mut(&r.id)
        {
            k.values = r.values.clone();
        }
        Ok(urls.len())
    }
}

/// Loads a layout's records into its table, and their portal rows into
/// the portals' tables; true when there were more records.
fn load(
    client: &Client,
    l: &mut RemoteLayout,
    db: &mut Database,
    loaded: &AtomicUsize,
) -> Result<bool> {
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
            let key = r.id.to_string();
            for p in &l.portals {
                let Some(pt) = db.tables.iter_mut().find(|t| t.id == p.table) else {
                    continue;
                };
                for row in r.portals.get(&p.name).into_iter().flatten() {
                    let j = pt.new_record();
                    let mut v = BTreeMap::from([(1, key.clone())]);
                    for (fid, name) in &p.fields {
                        if let Some(x) = row.fields.get(name).filter(|x| !x.is_empty()) {
                            v.insert(*fid, x.clone());
                        }
                    }
                    pt.records[j].values = v;
                }
            }
            let Some(t) = db.tables.iter_mut().find(|t| t.id == l.table) else {
                return Ok(false);
            };
            let i = t.new_record();
            let mut v = values(l, r);
            v.insert(l.key, key);
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
        loaded.fetch_add(n, Ordering::Relaxed);
        offset += n;
        if n < PAGE || offset > page.found {
            return Ok(false);
        }
        if offset > MAX_RECORDS {
            return Ok(true);
        }
    }
}
