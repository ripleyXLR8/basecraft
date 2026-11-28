//! Writing scripts (`docs/FORMAT.md` §8): typed step parameters back to
//! step records and parameter trees, and [`save_scripts`], which rebuilds
//! a file with edited, new and deleted scripts.
//!
//! Untouched scripts keep their bytes. An edited script gets its step list
//! and its parameter nodes `[17].[5].[S].[5]` rebuilt from [`Step::raw`] and
//! [`Step::raw_params`]; [`Step::sync`] writes a step's typed parameters
//! into those two first. Decoding then encoding any template script gives
//! back FileMaker's bytes (tests in `tests/scripts_save.rs`).

use std::collections::{BTreeMap, BTreeSet};

use crate::calc::compiled;
use crate::save::{
    encode_text, entry_kv, fresh_uuid, ipath, kv_entry, len_int, marker, path_elem, read_len_int,
    spelled, value_entries,
};
use crate::scripts::{
    Calc, FieldRef, LayoutDest, RawParams, RecordDest, STEP_LEN, Script, SortOrder, Step,
    StepParams, Target, id, opt, step_number,
};
use crate::write::{self, Entry, Item};
use crate::{Error, File};

impl RawParams {
    /// Removes everything at or under `p`.
    pub fn clear(&mut self, p: &[u32]) {
        self.kv.retain(|(path, _), _| !path.starts_with(p));
        self.long.retain(|(path, _, _)| !path.starts_with(p));
        self.markers.retain(|m| !m.starts_with(p));
    }

    /// Sets `key` at `path`, creating the nodes on the way.
    pub fn put(&mut self, path: &[u32], key: u32, v: Vec<u8>) {
        for i in 0..=path.len() {
            self.markers.insert(path[..i].to_vec());
        }
        self.kv.insert((path.to_vec(), key), v);
    }

    /// Drops node `p` when nothing is left in it.
    fn prune(&mut self, p: &[u32]) {
        let used = self.kv.keys().any(|(x, _)| x.starts_with(p))
            || self.long.iter().any(|(x, ..)| x.starts_with(p))
            || self
                .markers
                .iter()
                .any(|m| m.starts_with(p) && m.len() > p.len());
        if !used {
            self.markers.remove(p);
        }
    }

    /// Text in key 1 of parameter `p` (`None` removes it).
    /// Text that is not one byte per UTF-16 unit, or over 1000 bytes,
    /// moves to `[128 + p].[1]`: key 0 = byte and unit counts, keys 1… =
    /// 1000-byte chunks (as record values, §13).
    pub fn set_text(&mut self, p: u32, s: Option<&str>) {
        self.kv.remove(&(vec![128 + p], 1));
        self.clear(&[128 + p, 1]);
        let Some(s) = s else {
            self.prune(&[128 + p]);
            return;
        };
        let b = encode_text(s);
        let units = s.encode_utf16().count();
        if b.len() == units && b.len() <= 1000 {
            self.put(&[128 + p], 1, b);
            return;
        }
        let mut head = (b.len() as u32).to_be_bytes().to_vec();
        head.extend((units as u32).to_be_bytes());
        if b.len() > 1000 {
            head.extend(4u32.to_be_bytes());
        }
        self.put(&[128 + p, 1], 0, head);
        for (i, c) in b.chunks(1000).enumerate() {
            self.put(&[128 + p, 1], i as u32 + 1, c.to_vec());
        }
    }

    /// Calculation of parameter `p`: keys 4 (`01 00`), 5 (source tokens)
    /// and 6 (compiled form) of `[128 + p].[5]`.
    /// ponytail: a token stream over 1000 bytes is stored inline; FileMaker
    /// may split long values (§6), not seen for calculations yet.
    pub fn set_calc(&mut self, p: u32, c: Option<&Calc>) {
        self.clear(&[128 + p, 5]);
        match c {
            Some(c) => {
                let path = [128 + p, 5];
                self.put(&path, 4, vec![1, 0]);
                self.put(&path, 5, c.0.clone());
                self.put(&path, 6, compiled(&c.0).unwrap_or_else(|_| c.0.clone()));
            }
            None => self.prune(&[128 + p]),
        }
    }

    /// Byte in key 3 of parameter `p`.
    fn set_byte(&mut self, p: u32, v: Option<u8>) {
        self.kv.remove(&(vec![128 + p], 3));
        match v {
            Some(v) => self.put(&[128 + p], 3, vec![v]),
            None => self.prune(&[128 + p]),
        }
    }

    /// The tree as file entries under `base`. Elements above 127 keep the
    /// spelling they were read with; new ones are spelled `00 xx` (128 +
    /// xx), as FileMaker spells every parameter node: it does not find a
    /// parameter spelled `80 xx` (checked in FileMaker 26).
    pub fn entries(&self, base: &[u32]) -> Vec<Entry> {
        let at = |rel: &[u32]| -> Vec<Vec<u8>> {
            let mut p = spelled(base);
            p.extend(rel.iter().map(|v| {
                self.spelling.get(v).cloned().unwrap_or_else(|| match *v {
                    0..128 => vec![*v as u8],
                    _ => ((*v - 128) as u16).to_be_bytes().to_vec(),
                })
            }));
            p
        };
        let entry = |path: Vec<Vec<u8>>, mut e: Entry| {
            e.path = path;
            e
        };
        let mut out: Vec<Entry> = self
            .markers
            .iter()
            .map(|m| entry(at(m), marker(&[])))
            .collect();
        for ((path, key), v) in &self.kv {
            out.push(entry(at(path), kv_item(*key, v.clone())));
        }
        for (path, key, v) in &self.long {
            out.push(Entry {
                path: at(path),
                item: write::encode(&Item::KvLong {
                    key: key.clone(),
                    data: v.clone(),
                }),
            });
        }
        out
    }
}

/// A key/value entry. Keys 128–215 with a 1-, 2-, 4-, 6- or 8-byte value
/// (134, Go to Layout's animation; 139 of the PDF steps) are spelled as
/// 2-byte path integers (`0B 00 06`) as in the templates; other values
/// (a path list in key 130) and keys from 216 up use one byte (`06 82
/// len`), as FileMaker 26 writes them.
fn kv_item(key: u32, data: Vec<u8>) -> Entry {
    let n = data.len();
    if !(128..216).contains(&key) || !matches!(n, 1 | 2 | 4 | 6 | 8) {
        return kv_entry(&[], key, data);
    }
    let mut item = vec![if n == 1 { 0x09 } else { 0x09 + (n / 2) as u8 }];
    item.extend(path_elem(key).iter().map(|b| b & 0x7F));
    item.extend(data);
    Entry {
        path: Vec::new(),
        item,
    }
}

/// Default options of a new step (the most common value in the templates).
fn default_options(step: u16) -> u32 {
    match step {
        id::PERFORM_SCRIPT => 0x40,
        id::GO_TO_LAYOUT => 0xA,
        id::GO_TO_RECORD | id::GO_TO_PORTAL_ROW => 0x2,
        id::GO_TO_FIELD | id::SET_FIELD => 0x1,
        id::INSERT_TEXT => 0x5,
        id::DELETE_RECORD | id::REVERT_RECORD | id::COMMIT | id::OMIT_MULTIPLE => 0x80,
        id::SORT => 0x80,
        id::SET_VARIABLE | id::SHOW_CUSTOM_DIALOG => 0x4,
        id::SET_ERROR_CAPTURE => opt::ON,
        id::NEW_WINDOW => 0x8,
        _ => 0,
    }
}

/// Typed parameters of a new step.
fn default_params(step: u16) -> StepParams {
    let mut s = Step {
        id: step,
        enabled: true,
        options: default_options(step),
        raw: vec![0; STEP_LEN],
        ..Default::default()
    };
    s.raw[20..22].copy_from_slice(&step.to_be_bytes());
    s.raw[24..28].copy_from_slice(&s.options.to_be_bytes());
    s.decode();
    s.params
}

fn set_bit(o: &mut u32, bit: u32, on: bool) {
    if on {
        *o |= bit;
    } else {
        *o &= !bit;
    }
}

/// The plain text of a calculation that is a single text literal.
fn literal(c: &Calc) -> Option<String> {
    let b = &c.0;
    if b.first() != Some(&0x13) {
        return None;
    }
    let (s, n) = crate::calc::run(b, 2, *b.get(1)? as usize)?;
    (3 + n == b.len()).then_some(s)
}

/// `u32 count`, then per key: TO, field, u16 repetition, two references
/// `01 00`, u16 1, value list (empty slot if none), order, u32 0.
fn encode_sort(keys: &[(FieldRef, SortOrder)]) -> Vec<u8> {
    let mut out = (keys.len() as u32).to_be_bytes().to_vec();
    for (f, o) in keys {
        out.extend(to_slot(f.to));
        out.extend(id_slot(f.field));
        out.extend(f.repetition.to_be_bytes());
        out.extend([2, 1, 0, 2, 1, 0, 0, 1]);
        match o {
            SortOrder::Custom(v) => out.extend(id_slot(*v)),
            _ => out.push(0),
        }
        out.push(u8::from(*o == SortOrder::Descending));
        out.extend([0, 0, 0, 0]);
    }
    out
}

/// `04 03 D0 hi lo`: a table occurrence slot.
fn to_slot(to: u32) -> Vec<u8> {
    let mut out = vec![4, 3, 0xD0];
    out.extend((to.saturating_sub(128) as u16).to_be_bytes());
    out
}

/// `len n path-int`: an object id slot, `n` the path integer's length
/// (`02 01 7F` = 127, `03 02 80 00` = 128, as FileMaker 26 writes them).
fn id_slot(v: u32) -> Vec<u8> {
    let r = id_ref(v);
    let mut out = vec![r.len() as u8];
    out.extend(r);
    out
}

/// `n path-int`: an object id, `n` the path integer's length.
fn id_ref(v: u32) -> Vec<u8> {
    let e = path_elem(v);
    let mut out = vec![e.len() as u8];
    out.extend(e);
    out
}

impl Step {
    /// A new step with FileMaker's default options and parameters.
    pub fn new(step: u16) -> Step {
        let mut s = Step {
            id: step,
            enabled: true,
            options: default_options(step),
            raw: vec![0; STEP_LEN],
            params: default_params(step),
            ..Default::default()
        };
        s.raw[20..22].copy_from_slice(&step.to_be_bytes());
        s
    }

    fn pad(&mut self) {
        self.raw.resize(STEP_LEN.max(self.raw.len()), 0);
    }

    /// Inline object id at byte 6 (layout, script); the rest of the slot
    /// keeps its bytes.
    fn set_inline_id(&mut self, v: Option<u32>) {
        self.pad();
        match v {
            Some(v) => {
                let s = id_slot(v);
                self.raw[6..6 + s.len()].copy_from_slice(&s);
            }
            None => self.raw[6..12].fill(0),
        }
    }

    /// Inline field: TO at byte 6, field at 12, repetition at 18.
    fn set_inline_field(&mut self, f: Option<FieldRef>) {
        self.pad();
        // Bytes between the slots are left as they are (uninitialised in
        // FileMaker's files).
        if f.is_none() {
            self.raw[6..20].fill(0);
        }
        if let Some(f) = f {
            let t = to_slot(f.to);
            self.raw[6..11].copy_from_slice(&t);
            let s = id_slot(f.field);
            self.raw[12..12 + s.len()].copy_from_slice(&s);
            self.raw[18..20].copy_from_slice(&f.repetition.to_be_bytes());
        }
    }

    /// Writes the typed parameters ([`Step::params`]), the enabled flag and
    /// the options into the record ([`Step::raw`]) and the parameter tree
    /// ([`Step::raw_params`]). A step whose typed parameters still match
    /// its bytes is left as is.
    pub fn sync(&mut self) {
        self.pad();
        let mut fresh = self.clone();
        fresh.options =
            u32::from_be_bytes([self.raw[24], self.raw[25], self.raw[26], self.raw[27]]);
        fresh.arg = u16::from_be_bytes([self.raw[22], self.raw[23]]);
        fresh.decode();
        if fresh.params != self.params {
            self.write_params();
        }
        self.write_record();
    }

    /// Writes the typed parameters even when they look unchanged (tests:
    /// the encoders must give back the decoded bytes).
    pub fn rewrite(&mut self) {
        self.pad();
        self.write_params();
        self.write_record();
    }

    fn write_record(&mut self) {
        let signed = if self.enabled {
            self.id as i16
        } else {
            -(self.id as i16)
        };
        self.raw[20..22].copy_from_slice(&signed.to_be_bytes());
        self.raw[22..24].copy_from_slice(&self.arg.to_be_bytes());
        self.raw[24..28].copy_from_slice(&self.options.to_be_bytes());
    }

    fn write_params(&mut self) {
        let params = self.params.clone();
        let mut o = self.options;
        let p = &mut self.raw_params;
        let mut calc = false;
        match &params {
            StepParams::Comment(t) => {
                p.set_text(0, (!t.is_empty()).then_some(t.as_str()));
                set_bit(&mut o, 0x4, !t.is_empty());
            }
            StepParams::Switch(on) => set_bit(&mut o, opt::ON, *on),
            StepParams::Condition(c) => {
                p.set_calc(0, c.as_ref());
                calc = c.is_some();
            }
            StepParams::PerformScript { script, parameter } => {
                p.set_calc(0, parameter.as_ref());
                calc = parameter.is_some();
                o |= 0x40;
                let s = *script;
                self.set_inline_id(s);
            }
            StepParams::ExitScript(c) => {
                p.set_calc(0, c.as_ref());
                calc = c.is_some();
            }
            StepParams::SetVariable {
                name,
                value,
                repetition,
            } => {
                p.set_text(0, Some(name));
                p.set_calc(1, value.as_ref());
                p.set_calc(2, repetition.as_ref());
                calc = value.is_some() || repetition.is_some();
            }
            StepParams::SetField {
                field,
                value,
                repetition,
            } => {
                p.set_calc(0, value.as_ref());
                p.set_calc(10, repetition.as_ref());
                calc = value.is_some() || repetition.is_some();
                set_bit(&mut o, 0x1, field.is_some());
                let f = *field;
                self.set_inline_field(f);
            }
            StepParams::InsertText {
                target,
                text,
                select,
            } => {
                p.set_text(0, Some(text));
                set_bit(&mut o, opt::SELECT, *select);
                let (var, field) = match target {
                    Target::Variable(v) => (Some(v.clone()), None),
                    Target::Field(f) => (None, Some(*f)),
                    Target::None => (None, None),
                };
                p.set_text(32, var.as_deref());
                o |= 0x1;
                if var.is_none() {
                    self.set_inline_field(field);
                } else {
                    // As FileMaker writes a variable target: slots `01 00`
                    // and `01 20` (parameter 32), repetition 1.
                    self.raw[6..9].copy_from_slice(&[2, 1, 0]);
                    self.raw[12..15].copy_from_slice(&[2, 1, 0x20]);
                    self.raw[18..20].copy_from_slice(&[0, 1]);
                }
            }
            StepParams::GoToLayout { dest, animation } => {
                if *animation != 0 || p.kv.contains_key(&(vec![], 134)) {
                    p.put(&[], 134, animation.to_be_bytes().to_vec());
                }
                let (arg, layout, c) = match dest {
                    LayoutDest::Original => (1, None, None),
                    LayoutDest::NameByCalc(c) => (3, None, Some(c)),
                    LayoutDest::NumberByCalc(c) => (4, None, Some(c)),
                    LayoutDest::Layout(l) => (5, Some(*l), None),
                };
                p.set_calc(0, c);
                calc = c.is_some();
                // 0x8 marks an inline layout; FileMaker 26 also sets 0x2
                // (older templates have 0x8 alone, kept when present).
                set_bit(&mut o, 0x8, layout.is_some());
                if layout.is_none() {
                    o |= 0x2;
                }
                self.arg = arg;
                self.set_inline_id(layout);
            }
            StepParams::GoToRecord {
                dest,
                exit_after_last,
                dialog,
            } => {
                let (arg, c) = match dest {
                    RecordDest::First => (1, None),
                    RecordDest::Last => (2, None),
                    RecordDest::Previous => (3, None),
                    RecordDest::Next => (4, None),
                    RecordDest::ByCalc(c) => (5, Some(c)),
                };
                p.set_calc(0, c);
                if c.is_some() {
                    p.put(&[128], 3, vec![0xFF; 4]);
                }
                calc = c.is_some();
                set_bit(&mut o, opt::EXIT_AFTER_LAST, *exit_after_last);
                set_bit(&mut o, opt::NO_DIALOG, !*dialog);
                self.arg = arg;
            }
            StepParams::GoToField { field, select } => {
                set_bit(&mut o, opt::SELECT, *select);
                set_bit(&mut o, 0x1, field.is_some());
                let f = *field;
                self.set_inline_field(f);
            }
            StepParams::GoToObject { name, repetition } => {
                p.set_calc(0, name.as_ref());
                p.set_calc(1, repetition.as_ref());
                calc = name.is_some() || repetition.is_some();
            }
            StepParams::ShowCustomDialog {
                title,
                message,
                buttons,
                input,
            } => {
                p.set_calc(0, title.as_ref());
                p.set_calc(1, message.as_ref());
                p.set_calc(2, input.as_ref().and_then(|(_, l)| l.as_ref()));
                for i in 0..3u32 {
                    let b = buttons.get(i as usize);
                    let label = b.and_then(|b| b.label.as_ref());
                    p.set_calc(5 + i, label);
                    p.set_text(i, label.and_then(literal).as_deref());
                    if let Some(b) = b.filter(|b| b.label.is_some()) {
                        p.set_byte(3 + i, Some(u8::from(b.commit)));
                    }
                }
                calc = true;
                set_bit(&mut o, 0x1, input.is_some());
                let f = input.as_ref().map(|(f, _)| *f);
                self.set_inline_field(f);
            }
            StepParams::Records { dialog, count } => {
                p.set_calc(0, count.as_ref());
                calc = count.is_some();
                set_bit(&mut o, opt::NO_DIALOG, !*dialog);
            }
            StepParams::Commit {
                dialog,
                skip_validation,
                force,
            } => {
                set_bit(&mut o, opt::NO_DIALOG, !*dialog);
                set_bit(&mut o, opt::SKIP_VALIDATION, *skip_validation);
                set_bit(&mut o, opt::FORCE_COMMIT, *force);
            }
            StepParams::Find {
                pause,
                restore,
                requests,
            } => {
                set_bit(&mut o, opt::PAUSE, *pause);
                set_bit(&mut o, opt::RESTORE, *restore);
                p.clear(&[6, 6]);
                if !requests.is_empty() {
                    if !p.markers.contains(&vec![6, 5]) {
                        // ponytail: `[6].[5]` (not decoded) as FileMaker 26
                        // writes it for a new Perform Find.
                        p.put(&[6, 5], 0, 6u32.to_be_bytes().to_vec());
                    }
                    p.put(&[6, 6], 0, len_int(requests.len() as u64));
                    for (r, q) in requests.iter().enumerate() {
                        let path = vec![6, 6, r as u32 + 1];
                        p.markers.insert(path.clone());
                        if q.omit {
                            p.put(&path, 0, Vec::new());
                        }
                        for (f, t) in &q.criteria {
                            let mut key = vec![0];
                            key.extend(&to_slot(f.to)[1..]);
                            key.extend(&id_slot(f.field)[1..]);
                            p.long.push((path.clone(), key, encode_text(t)));
                        }
                    }
                }
            }
            StepParams::Mode { pause } => set_bit(&mut o, opt::PAUSE, *pause),
            StepParams::Pause(c) => {
                p.set_calc(0, c.as_ref());
                calc = c.is_some();
            }
            StepParams::Sort {
                dialog,
                restore,
                keys,
            } => {
                set_bit(&mut o, opt::NO_DIALOG, !*dialog);
                set_bit(&mut o, opt::RESTORE, *restore);
                p.kv.remove(&(vec![], 10));
                if !keys.is_empty() {
                    p.put(&[], 10, encode_sort(keys));
                }
            }
            StepParams::RefreshWindow {
                flush_joins,
                flush_external,
            } => {
                set_bit(&mut o, opt::FLUSH_JOINS, *flush_joins);
                set_bit(&mut o, opt::FLUSH_EXTERNAL, *flush_external);
            }
            StepParams::None | StepParams::Other => {}
        }
        if !matches!(params, StepParams::None | StepParams::Other) {
            set_bit(&mut o, opt::CALC, calc);
        }
        self.options = o;
    }
}

/// Bytes 22–23 of control steps: If and Else If point at the next branch
/// (Else If, Else) or the End If; Else at the End If; Loop and Exit Loop
/// If at the End Loop; End Loop at its Loop; End If holds 0. Disabled
/// steps are left out of the structure.
pub fn block_links(steps: &mut [Step]) {
    // (is loop, opening step, branches)
    let mut stack: Vec<(bool, Vec<usize>)> = Vec::new();
    let mut set: Vec<(usize, u16)> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        if !s.enabled {
            continue;
        }
        match s.id {
            id::IF => stack.push((false, vec![i])),
            id::ELSE_IF | id::ELSE => {
                if let Some((false, b)) = stack.last_mut() {
                    b.push(i);
                }
            }
            id::END_IF => {
                if let Some((false, b)) = stack.pop() {
                    for (k, &x) in b.iter().enumerate() {
                        set.push((x, b.get(k + 1).copied().unwrap_or(i) as u16));
                    }
                    set.push((i, 0));
                }
            }
            id::LOOP => stack.push((true, vec![i])),
            id::EXIT_LOOP_IF => {
                if let Some((_, v)) = stack.iter_mut().rev().find(|(l, _)| *l) {
                    v.push(i);
                }
            }
            id::END_LOOP => {
                while let Some((false, _)) = stack.last() {
                    stack.pop();
                }
                if let Some((true, v)) = stack.pop() {
                    for &x in &v {
                        set.push((x, i as u16));
                    }
                    set.push((i, v[0] as u16));
                }
            }
            _ => {}
        }
    }
    for (i, a) in set {
        steps[i].arg = a;
    }
}

/// The step list (28-byte records, the last one's trailing zeros trimmed)
/// and the parameter nodes of `script`, as entries under `[17].[5].[S]`.
/// FileMaker finds a step's parameters by its position, not by bytes 0–3:
/// step *i* (0-based) always gets node `[S].[5].[i + 1]` (checked in
/// FileMaker 26: a new step numbered otherwise shows no parameters).
/// Steps without a parameter node, or sharing one with an earlier step,
/// get a new UUID. Returns the entries and the step count (key 0 of
/// `[S].[5]`).
pub fn step_entries(script: &Script) -> (Vec<Entry>, u32) {
    let s = script.id;
    let mut steps = script.steps.clone();
    block_links(&mut steps);
    let mut used = BTreeSet::new();
    let mut list = Vec::new();
    let mut out = Vec::new();
    let last = steps.len() as u32;
    for (i, st) in steps.iter_mut().enumerate() {
        st.sync();
        let n = i as u32 + 1;
        if !step_number(&st.raw).is_some_and(|m| used.insert(m)) {
            st.raw_params.kv.remove(&(vec![], 217));
            st.raw_params
                .put(&[], 216, fresh_uuid(((s as u64) << 32) | n as u64));
        }
        let slot = id_slot(n);
        st.raw[..4].fill(0);
        st.raw[..slot.len()].copy_from_slice(&slot);
        let mut r = st.raw.clone();
        r.resize(STEP_LEN, 0);
        list.extend(r);
        out.extend(st.raw_params.entries(&[17, 5, s, 5, n]));
    }
    // The last record keeps its stored length, at least up to its last
    // non-zero byte (FileMaker trims trailing zeros in some files only).
    let keep = script.steps.last().map_or(0, |s| s.raw.len());
    let mut trimmed = list.len();
    while trimmed > 0 && list[trimmed - 1] == 0 {
        trimmed -= 1;
    }
    let full = list.len();
    list.truncate(trimmed.max(full.saturating_sub(STEP_LEN) + keep).min(full));
    if list.len() > 1000 {
        let p = [17, 5, s, 4];
        out.push(marker(&p));
        out.push(kv_entry(&p, 0, (list.len() as u32).to_be_bytes().to_vec()));
        for (i, c) in list.chunks(1000).enumerate() {
            out.push(kv_entry(&p, i as u32 + 1, c.to_vec()));
        }
    } else if !list.is_empty() {
        out.push(kv_entry(&[17, 5, s], 4, list));
    }
    if !steps.is_empty() {
        out.push(marker(&[17, 5, s, 5]));
        // The step count as an id (`02 80 0C` = 140), not a len-int.
        out.push(kv_entry(&[17, 5, s, 5], 0, id_ref(last)));
    }
    (out, last)
}

/// FileMaker's collation key of a name in a catalog name index
/// (`[17].[1].[1]`, `[4].[1].[1]`): two-byte primary weights, case
/// folded, then `00 00 00`. `None` for characters whose weight is not
/// known (only seen ones: space, `-`, `'` and Latin letters but x).
pub fn collation_key(name: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let w: u16 = match c {
            ' ' => 0x020A,
            '-' => 0x0223,
            '\'' => 0x02EE,
            'a' => 0x120F,
            'b' => 0x1225,
            'c' => 0x123D,
            'd' => 0x1250,
            'e' => 0x126B,
            'f' => 0x12A3,
            'g' => 0x12B0,
            'h' => 0x12D3,
            'i' => 0x12EC,
            'j' => 0x1305,
            'k' => 0x131E,
            'l' => 0x1330,
            'm' => 0x135F,
            'n' => 0x136D,
            'o' => 0x138E,
            'p' => 0x13B3,
            'q' => 0x13C8,
            'r' => 0x13DA,
            's' => 0x1410,
            't' => 0x1433,
            'u' => 0x1453,
            'v' => 0x147B,
            'w' => 0x148D,
            'y' => 0x149C,
            'z' => 0x14AD,
            _ => return None,
        };
        out.extend(w.to_be_bytes());
    }
    out.extend([0, 0, 0]);
    Some(out)
}

fn kvl(path: &[u32], key: Vec<u8>, data: Vec<u8>) -> Entry {
    Entry {
        path: spelled(path),
        item: write::encode(&Item::KvLong { key, data }),
    }
}

/// (key, value) of a byte-string-key entry.
fn entry_kvl(e: &Entry) -> Option<(Vec<u8>, Vec<u8>)> {
    match crate::chunk::parse(&e.item).ok()?.first()? {
        crate::Chunk::KvLong { key, data } => Some((key.to_vec(), data.to_vec())),
        _ => None,
    }
}

/// Rebuilds `file` with `scripts`, the whole script list in display order.
/// Scripts equal to the file's keep their bytes; edited ones get a new
/// step list and parameter nodes (and name); new ones (ids the file does
/// not have) get catalog entries `[17].[1].[7].[S]`, a position in
/// `[17].[1].[3]`, a name index entry `[17].[1].[1]` (when the name's
/// collation key is known) and a script node `[17].[5].[S]` modelled on
/// what FileMaker writes for a new script; scripts missing from the list
/// are deleted. When scripts are added, deleted or renamed, `[17].[1]`
/// key 8 is recomputed ([`catalog_digest`]).
pub fn save_scripts(file: &File, scripts: &[Script]) -> Result<Vec<u8>, Error> {
    let old = crate::scripts::scripts(file)?;
    let mut entries = write::entries(file)?;
    let olds: BTreeMap<u32, &Script> = old.iter().map(|s| (s.id, s)).collect();
    let news: BTreeSet<u32> = scripts.iter().map(|s| s.id).collect();
    let mut add: Vec<Entry> = Vec::new();
    let mut renamed = false;
    let in_script =
        |e: &Entry, s: u32| ipath(e).is_some_and(|p| p.len() >= 3 && p[..3] == [17, 5, s]);
    // Catalog keys copied onto new scripts: user, account, timestamp.
    let mut meta: Vec<(u32, Vec<u8>)> = Vec::new();
    for e in &entries {
        if ipath(e).as_deref() == Some(&[17, 1])
            && let Some((k, v)) = entry_kv(e)
            && k >= 31873
        {
            meta.push((k, v));
        }
    }
    // Deleted scripts.
    for o in &old {
        if news.contains(&o.id) {
            continue;
        }
        let idv = len_int(o.id as u64);
        entries.retain(|e| {
            let p = ipath(e);
            !(in_script(e, o.id)
                || p.as_deref()
                    .is_some_and(|p| p.starts_with(&[17, 1, 7, o.id]))
                || p.as_deref() == Some(&[17, 1, 1]) && entry_kvl(e).is_some_and(|(_, v)| v == idv))
        });
    }
    for s in scripts {
        match olds.get(&s.id) {
            Some(o) if *o == s => {}
            Some(o) => {
                if o.name != s.name {
                    renamed = true;
                    rename(&mut entries, &mut add, s);
                }
                if o.steps != s.steps {
                    entries.retain(|e| {
                        let Some(p) = ipath(e) else { return true };
                        let list = p.as_slice() == [17, 5, s.id]
                            && entry_kv(e).is_some_and(|(k, _)| k == 4);
                        let sub = p.len() >= 4 && p[..3] == [17, 5, s.id] && matches!(p[3], 4 | 5);
                        !(list || sub)
                    });
                    add.extend(step_entries(s).0);
                }
            }
            None => {
                let sp = [17, 5, s.id];
                add.push(marker(&sp));
                add.extend(value_entries(&spelled(&sp), 16, &s.name));
                add.push(kv_entry(&sp, 19, vec![0; 4]));
                add.push(kv_entry(&sp, 216, fresh_uuid(s.id as u64)));
                add.push(kv_entry(&sp, 252, vec![1, 1]));
                if s.options != 8 {
                    add.push(kv_entry(&sp, 240, s.options.to_be_bytes().to_vec()));
                }
                for (k, v) in &meta {
                    add.push(kv_entry(&sp, *k, v.clone()));
                }
                add.extend(step_entries(s).0);
                let cp = [17, 1, 7, s.id];
                add.push(marker(&cp));
                add.extend(value_entries(&spelled(&cp), 16, &s.name));
                if s.options != 8 {
                    add.push(kv_entry(&cp, 240, s.options.to_be_bytes().to_vec()));
                }
                if let Some(k) = collation_key(&s.name) {
                    add.push(kvl(&[17, 1, 1], k, len_int(s.id as u64)));
                }
            }
        }
    }
    // Display order, count and last id when the list changed.
    let ids: Vec<u32> = scripts.iter().map(|s| s.id).collect();
    let old_ids: Vec<u32> = old.iter().map(|s| s.id).collect();
    if ids != old_ids {
        let (mut last, mut counter) = (0, 0);
        entries.retain(|e| {
            let p = ipath(e);
            if p.as_deref() == Some(&[17, 1])
                && let Some((k, v)) = entry_kv(e)
            {
                if k == 0 {
                    last = read_len_int(&v).unwrap_or(0) as u32;
                }
                if k == 252 {
                    counter = read_len_int(&v).unwrap_or(0);
                }
                return !matches!(k, 0 | 4 | 252);
            }
            !(p.as_deref() == Some(&[17, 1, 3]) && e.item.first() != Some(&0x80))
        });
        if !entries
            .iter()
            .any(|e| ipath(e).as_deref() == Some(&[17, 1, 3]))
        {
            add.push(marker(&[17, 1, 3]));
        }
        for (i, sid) in ids.iter().enumerate() {
            add.push(kvl(
                &[17, 1, 3],
                (i as u32 + 1).to_be_bytes().to_vec(),
                len_int(*sid as u64),
            ));
        }
        let max = ids.iter().copied().max().unwrap_or(0).max(last);
        add.push(kv_entry(&[17, 1], 0, len_int(max as u64)));
        add.push(kv_entry(&[17, 1], 252, len_int(counter + 1)));
        add.push(kv_entry(
            &[17, 1],
            4,
            (ids.len() as u32).to_be_bytes().to_vec(),
        ));
    }
    // Every node has a marker: a file with no script yet has none for the
    // name index and the catalog members (FileMaker's consistency check
    // calls such a file damaged).
    for node in [[17, 1, 1], [17, 1, 7]] {
        let under = |e: &Entry| ipath(e).is_some_and(|p| p.starts_with(&node));
        let marked = |e: &Entry| ipath(e).as_deref() == Some(&node[..]) && e.item == [0x80];
        if add.iter().any(under) && !entries.iter().chain(&add).any(marked) {
            add.push(marker(&node));
        }
    }
    entries.extend(add);
    if renamed || ids != old_ids {
        entries.retain(|e| {
            !(ipath(e).as_deref() == Some(&[17, 1]) && entry_kv(e).is_some_and(|(k, _)| k == 8))
        });
        if let Some(d) = catalog_digest(&entries) {
            entries.push(kv_entry(&[17, 1], 8, d.to_vec()));
        }
    }
    write::sort(&mut entries);
    Ok(write::build(&file.bytes()[..crate::SECTOR], &entries))
}

/// `[17].[1]` key 8, the integrity digest FileMaker checks when it opens
/// the file (it refuses a mismatch as "access privileges damaged"): MD5
/// over the catalog members `[17].[1].[7].[S]` in id order (folders and
/// separators included), each fed 8 zero bytes then its name in UTF-16BE,
/// nothing else. `None` for an empty catalog, which has no key 8.
/// ponytail: the 8 bytes are "private flags" in the public description;
/// no member in any file we have carries a key for them, so they are zero.
pub fn catalog_digest(entries: &[Entry]) -> Option<[u8; 16]> {
    use md5::{Digest, Md5};
    // id → inline name; (id, key) → chunks of a long name.
    let mut names: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut long: BTreeMap<(u32, u32), Vec<u8>> = BTreeMap::new();
    for e in entries {
        let Some(p) = ipath(e) else { continue };
        if p.len() < 4 || p[..3] != [17, 1, 7] {
            continue;
        }
        let n = names.entry(p[3]).or_default();
        match (p.len(), entry_kv(e)) {
            (4, Some((16, v))) => *n = v,
            (5, Some((k, v))) if p[4] == 16 && k >= 1 => {
                long.insert((p[3], k), v);
            }
            _ => {}
        }
    }
    if names.is_empty() {
        return None;
    }
    let mut h = Md5::new();
    for (id, inline) in names {
        let chunks: Vec<u8> = long
            .range((id, 1)..=(id, u32::MAX))
            .flat_map(|(_, c)| c.iter().copied())
            .collect();
        let b = if chunks.is_empty() { inline } else { chunks };
        h.update([0u8; 8]);
        for u in crate::text(&b).encode_utf16() {
            h.update(u.to_be_bytes());
        }
    }
    let mut out = [0; 16];
    out.copy_from_slice(&h.finalize());
    Some(out)
}

/// New name in the catalog, the script node and the name index.
fn rename(entries: &mut Vec<Entry>, add: &mut Vec<Entry>, s: &Script) {
    let idv = len_int(s.id as u64);
    let bases: [&[u32]; 2] = [&[17, 1, 7, s.id], &[17, 5, s.id]];
    for base in bases {
        let mut long = base.to_vec();
        long.push(16);
        entries.retain(|e| {
            let p = ipath(e);
            let inline = p.as_deref() == Some(base) && entry_kv(e).is_some_and(|(k, _)| k == 16);
            !(inline || p.as_deref().is_some_and(|p| p.starts_with(&long)))
        });
        add.extend(value_entries(&spelled(base), 16, &s.name));
    }
    for e in entries.iter_mut() {
        let p = ipath(e);
        // ponytail: an index entry whose new key is unknown keeps the old
        // key (FileMaker's collation is only known for Latin letters).
        if p.as_deref() == Some(&[17, 1, 1])
            && entry_kvl(e).is_some_and(|(_, v)| v == idv)
            && let Some(k) = collation_key(&s.name)
        {
            *e = kvl(&[17, 1, 1], k, idv.clone());
        }
    }
}
