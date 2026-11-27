//! Scripts (`[17]`): catalog entries, step lists and step parameters.
//!
//! * `[17].[5].[S]` key 16 = name, key 240 = options, key 4 (or, when longer
//!   than 1000 bytes, keys 1…n of `[S].[4]`) = step records of 28 bytes;
//! * `[17].[5].[S].[5].[n]` = parameters of step *n* (its record's own index):
//!   sub-node `[128 + p]` holds parameter *p* (XML `position`), as text in
//!   key 1 or as a calculation in `.[5]` (keys 4, 5, 6, see [`crate::calc`]).
//!
//! See `docs/FORMAT.md` §8 for the layout of each step kind.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::to_ref;
use crate::{Chunk, Error, File, int_path, path_int, pb::Msg, text};

/// Size of one step record in key 4 of a script node. The last record of
/// the list has its trailing zero bytes trimmed.
pub(crate) const STEP_LEN: usize = 28;

/// The step catalogue, `id<TAB>name` per line (see the file's header).
const CATALOGUE: &str = include_str!("../../../docs/reference/script_steps.txt");

/// FileMaker's English name of a script step id (XML `Step name`).
pub fn step_name(id: u16) -> Option<&'static str> {
    CATALOGUE
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(i, _)| i.parse() == Ok(id))
        .map(|(_, n)| n)
}

/// Step ids used by the typed decoding and the script engine.
pub mod id {
    pub const PERFORM_SCRIPT: u16 = 1;
    pub const GO_TO_LAYOUT: u16 = 6;
    pub const NEW_RECORD: u16 = 7;
    pub const DUPLICATE_RECORD: u16 = 8;
    pub const DELETE_RECORD: u16 = 9;
    pub const DELETE_ALL_RECORDS: u16 = 10;
    pub const GO_TO_RECORD: u16 = 16;
    pub const GO_TO_FIELD: u16 = 17;
    pub const UNSORT: u16 = 21;
    pub const ENTER_FIND_MODE: u16 = 22;
    pub const SHOW_ALL: u16 = 23;
    pub const OMIT_RECORD: u16 = 25;
    pub const OMIT_MULTIPLE: u16 = 26;
    pub const SHOW_OMITTED: u16 = 27;
    pub const PERFORM_FIND: u16 = 28;
    pub const SORT: u16 = 39;
    pub const ENTER_PREVIEW_MODE: u16 = 41;
    pub const REVERT_RECORD: u16 = 51;
    pub const ENTER_BROWSE_MODE: u16 = 55;
    pub const INSERT_TEXT: u16 = 61;
    pub const INSERT_CURRENT_DATE: u16 = 13;
    pub const INSERT_CURRENT_TIME: u16 = 14;
    pub const RELOOKUP: u16 = 40;
    pub const PASTE: u16 = 48;
    pub const INSERT_CURRENT_USER_NAME: u16 = 60;
    pub const INSERT_CALCULATED_RESULT: u16 = 77;
    pub const REPLACE_FIELD_CONTENTS: u16 = 91;
    pub const PERFORM_FIND_REPLACE: u16 = 128;
    pub const SET_FIELD_BY_NAME: u16 = 147;
    pub const PAUSE_RESUME: u16 = 62;
    pub const IF: u16 = 68;
    pub const ELSE: u16 = 69;
    pub const END_IF: u16 = 70;
    pub const LOOP: u16 = 71;
    pub const EXIT_LOOP_IF: u16 = 72;
    pub const END_LOOP: u16 = 73;
    pub const COMMIT: u16 = 75;
    pub const SET_FIELD: u16 = 76;
    pub const FREEZE_WINDOW: u16 = 79;
    pub const REFRESH_WINDOW: u16 = 80;
    pub const ALLOW_USER_ABORT: u16 = 85;
    pub const SET_ERROR_CAPTURE: u16 = 86;
    pub const SHOW_CUSTOM_DIALOG: u16 = 87;
    pub const COMMENT: u16 = 89;
    pub const HALT_SCRIPT: u16 = 90;
    pub const BEEP: u16 = 93;
    pub const GO_TO_PORTAL_ROW: u16 = 99;
    pub const EXIT_SCRIPT: u16 = 103;
    pub const CLOSE_WINDOW: u16 = 121;
    pub const NEW_WINDOW: u16 = 122;
    pub const ELSE_IF: u16 = 125;
    pub const SET_VARIABLE: u16 = 141;
    pub const GO_TO_OBJECT: u16 = 145;
}

/// Step option bits (XML `<Options>`, record bytes 24–27).
pub mod opt {
    /// The step holds at least one calculation.
    pub const CALC: u32 = 0x4000;
    /// "With dialog: Off" (Delete/Revert/Commit/Sort/Go to Record…).
    pub const NO_DIALOG: u32 = 0x80;
    /// Set Error Capture / Allow User Abort: On.
    pub const ON: u32 = 0x2_0000;
    /// Go to Field / Insert Text: select (perform).
    pub const SELECT: u32 = 0x1000;
    /// Go to Record: exit after last.
    pub const EXIT_AFTER_LAST: u32 = 0x2000;
    /// Commit: skip data entry validation; force commit.
    pub const SKIP_VALIDATION: u32 = 0x100;
    pub const FORCE_COMMIT: u32 = 0x200;
    /// Refresh Window: flush cached join results; flush cached external data.
    pub const FLUSH_JOINS: u32 = 0x100;
    pub const FLUSH_EXTERNAL: u32 = 0x200;
    /// Enter Find/Browse Mode: pause; Perform Find/Sort/Enter Find Mode: restore.
    pub const PAUSE: u32 = 0x100_0000;
    pub const RESTORE: u32 = 0x200_0000;
}

/// The parameter sub-tree of one step: `[S].[5].[n]` (or `[obj].[17]` for a
/// button). Paths are relative to that node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawParams {
    /// (relative path, key) → value. A long value (keys 1…n under
    /// `path + [key]`) is joined back into one.
    pub kv: BTreeMap<(Vec<u32>, u32), Vec<u8>>,
    /// Byte-string keys: (relative path, key, value), e.g. find criteria.
    pub long: Vec<(Vec<u32>, Vec<u8>, Vec<u8>)>,
    /// Nodes (relative paths) that carry a node marker; every node does.
    pub markers: BTreeSet<Vec<u32>>,
    /// How the file spells path elements above 127 (`80 00` or `00 00`
    /// both read 128), so that a rewrite keeps the bytes.
    pub spelling: BTreeMap<u32, Vec<u8>>,
}

impl RawParams {
    fn add(&mut self, rel: &[u32], c: &Chunk) {
        match c {
            Chunk::Kv { key, data } => {
                self.kv.insert((rel.to_vec(), *key), data.to_vec());
            }
            // `00 k` is key k with an empty value.
            Chunk::Data([k]) => {
                self.kv.insert((rel.to_vec(), *k as u32), Vec::new());
            }
            Chunk::KvLong { key, data } => {
                self.long.push((rel.to_vec(), key.to_vec(), data.to_vec()))
            }
            Chunk::Marker => {
                self.markers.insert(rel.to_vec());
            }
            _ => {}
        }
    }

    /// Value of `key` at `path`, joining a long value stored as keys 1…n
    /// of `path + [key]`.
    pub fn get(&self, path: &[u32], key: u32) -> Option<Vec<u8>> {
        if let Some(v) = self.kv.get(&(path.to_vec(), key)) {
            return Some(v.clone());
        }
        let mut sub = path.to_vec();
        sub.push(key);
        let parts: Vec<&Vec<u8>> = (1..)
            .map_while(|k| self.kv.get(&(sub.clone(), k)))
            .collect();
        (!parts.is_empty()).then(|| parts.into_iter().flatten().copied().collect())
    }

    /// Text in key 1 of parameter `p` (node `[128 + p]`).
    pub fn text(&self, p: u32) -> Option<String> {
        self.get(&[128 + p], 1).map(|b| text(&b))
    }

    /// Calculation of parameter `p` (node `[128 + p].[5]`).
    pub fn calc(&self, p: u32) -> Option<Calc> {
        self.get(&[128 + p, 5], 5).map(Calc)
    }

    /// Byte in key 3 of parameter `p` (button commit flags…).
    pub fn byte(&self, p: u32) -> Option<u8> {
        self.get(&[128 + p], 3)?.last().copied()
    }
}

/// A calculation stored in a step: its token stream (key 5). The context
/// (key 4) of script calculations is `01 00`, the current layout's
/// occurrence, so every field reference shows as `TO::Field`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calc(pub Vec<u8>);

impl Calc {
    pub fn text(&self, names: &dyn crate::calc::Names) -> Result<String, crate::calc::CalcError> {
        crate::calc::decompile(&self.0, 0, names)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldRef {
    pub to: u32,
    pub field: u32,
    pub repetition: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutDest {
    Original,
    Layout(u32),
    NameByCalc(Calc),
    NumberByCalc(Calc),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordDest {
    First,
    Last,
    Previous,
    Next,
    ByCalc(Calc),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Field(FieldRef),
    Variable(String),
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindRequest {
    pub omit: bool,
    /// (field, criterion text)
    pub criteria: Vec<(FieldRef, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
    /// By the order of a value list.
    Custom(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogButton {
    pub label: Option<Calc>,
    pub commit: bool,
}

/// Typed parameters of the steps the engine runs. Anything else is
/// [`StepParams::Other`]; the raw tree stays in [`Step::raw_params`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StepParams {
    #[default]
    None,
    Comment(String),
    /// Set Error Capture, Allow User Abort: On/Off.
    Switch(bool),
    /// If, Else If, Exit Loop If.
    Condition(Option<Calc>),
    PerformScript {
        script: Option<u32>,
        parameter: Option<Calc>,
    },
    ExitScript(Option<Calc>),
    SetVariable {
        name: String,
        value: Option<Calc>,
        repetition: Option<Calc>,
    },
    SetField {
        field: Option<FieldRef>,
        value: Option<Calc>,
        repetition: Option<Calc>,
    },
    InsertText {
        target: Target,
        text: String,
        select: bool,
    },
    GoToLayout {
        dest: LayoutDest,
        animation: u32,
    },
    GoToRecord {
        dest: RecordDest,
        exit_after_last: bool,
        dialog: bool,
    },
    GoToField {
        field: Option<FieldRef>,
        select: bool,
    },
    GoToObject {
        name: Option<Calc>,
        repetition: Option<Calc>,
    },
    ShowCustomDialog {
        title: Option<Calc>,
        message: Option<Calc>,
        buttons: Vec<DialogButton>,
        /// First input field and its label (more are not decoded).
        input: Option<(FieldRef, Option<Calc>)>,
    },
    /// New/Duplicate/Delete/Revert Record, Omit Multiple Records…
    Records {
        dialog: bool,
        count: Option<Calc>,
    },
    Commit {
        dialog: bool,
        skip_validation: bool,
        force: bool,
    },
    /// Enter Find Mode, Perform Find.
    Find {
        pause: bool,
        restore: bool,
        requests: Vec<FindRequest>,
    },
    /// Enter Browse/Preview Mode.
    Mode {
        pause: bool,
    },
    Sort {
        dialog: bool,
        restore: bool,
        keys: Vec<(FieldRef, SortOrder)>,
    },
    RefreshWindow {
        flush_joins: bool,
        flush_external: bool,
    },
    /// Pause/Resume Script: indefinitely (`None`) or for a duration in
    /// seconds. ponytail: storage not seen in a sample; the duration is
    /// read as the calculation of parameter 0 like other steps.
    Pause(Option<Calc>),
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Step {
    /// Script step type id (e.g. 39 = Sort Records), see [`step_name`].
    pub id: u16,
    pub enabled: bool,
    /// Step option bits as shown in FileMaker's XML `<Options>` ([`opt`]).
    pub options: u32,
    /// Record bytes 22–23: index (0-based) of the step closing the block for
    /// If/Else If/Else/Loop/Exit Loop If (and of the Loop for End Loop);
    /// the destination kind for Go to Layout (1 original, 3 name, 4 number,
    /// 5 layout) and Go to Record (1 first … 5 by calculation).
    pub arg: u16,
    pub raw: Vec<u8>,
    pub raw_params: RawParams,
    pub params: StepParams,
}

impl Step {
    pub fn name(&self) -> Option<&'static str> {
        step_name(self.id)
    }

    /// Bytes 6–19 of the record: inline references.
    fn block(&self) -> [u8; 14] {
        let mut b = [0; 14];
        if let Some(s) = self.raw.get(6..20.min(self.raw.len())) {
            b[..s.len()].copy_from_slice(s);
        }
        b
    }

    /// Length-prefixed value at `off` of the inline block.
    fn slot(&self, off: usize) -> Vec<u8> {
        let b = self.block();
        let n = b[off] as usize;
        b.get(off + 1..off + 1 + n).unwrap_or_default().to_vec()
    }

    /// Object id of the first slot (`01` + path integer): layout, script.
    pub(crate) fn inline_id(&self) -> Option<u32> {
        id_ref(&self.slot(0))
    }

    /// Field of the inline block: TO at 0, field at 6, repetition at 12.
    pub fn inline_field(&self) -> Option<FieldRef> {
        let b = self.block();
        Some(FieldRef {
            to: to_ref(&self.slot(0))?,
            field: id_ref(&self.slot(6))?,
            repetition: u16::from_be_bytes([b[12], b[13]]),
        })
    }

    pub(crate) fn decode(&mut self) {
        let o = self.options;
        let p = &self.raw_params;
        let dialog = o & opt::NO_DIALOG == 0;
        self.params = match self.id {
            id::COMMENT => StepParams::Comment(p.text(0).unwrap_or_default()),
            id::SET_ERROR_CAPTURE | id::ALLOW_USER_ABORT => StepParams::Switch(o & opt::ON != 0),
            id::IF | id::ELSE_IF | id::EXIT_LOOP_IF => StepParams::Condition(p.calc(0)),
            id::PERFORM_SCRIPT => StepParams::PerformScript {
                script: self.inline_id(),
                parameter: p.calc(0),
            },
            id::EXIT_SCRIPT => StepParams::ExitScript(p.calc(0)),
            id::SET_VARIABLE => StepParams::SetVariable {
                name: p.text(0).unwrap_or_default(),
                value: p.calc(1),
                repetition: p.calc(2),
            },
            id::SET_FIELD => StepParams::SetField {
                field: self.inline_field(),
                value: p.calc(0),
                repetition: p.calc(10),
            },
            id::INSERT_TEXT => StepParams::InsertText {
                target: match (p.text(32), self.inline_field()) {
                    (Some(v), _) => Target::Variable(v),
                    (None, Some(f)) => Target::Field(f),
                    (None, None) => Target::None,
                },
                text: p.text(0).unwrap_or_default(),
                select: o & opt::SELECT != 0,
            },
            id::GO_TO_LAYOUT => StepParams::GoToLayout {
                dest: match (self.arg, p.calc(0)) {
                    (3, Some(c)) => LayoutDest::NameByCalc(c),
                    (4, Some(c)) => LayoutDest::NumberByCalc(c),
                    (5, _) => LayoutDest::Layout(self.inline_id().unwrap_or(0)),
                    _ => LayoutDest::Original,
                },
                animation: p.get(&[], 134).map_or(0, |b| be(&b)),
            },
            id::GO_TO_RECORD => StepParams::GoToRecord {
                dest: match self.arg {
                    1 => RecordDest::First,
                    2 => RecordDest::Last,
                    3 => RecordDest::Previous,
                    5 => RecordDest::ByCalc(p.calc(0).unwrap_or_default()),
                    _ => RecordDest::Next,
                },
                exit_after_last: o & opt::EXIT_AFTER_LAST != 0,
                dialog,
            },
            id::GO_TO_FIELD => StepParams::GoToField {
                field: self.inline_field(),
                select: o & opt::SELECT != 0,
            },
            id::GO_TO_OBJECT => StepParams::GoToObject {
                name: p.calc(0),
                repetition: p.calc(1),
            },
            id::SHOW_CUSTOM_DIALOG => StepParams::ShowCustomDialog {
                title: p.calc(0),
                message: p.calc(1),
                buttons: (0..3)
                    .map(|i| DialogButton {
                        label: p.calc(5 + i),
                        commit: p.byte(3 + i) == Some(1),
                    })
                    .collect(),
                input: self.inline_field().map(|f| (f, p.calc(2))),
            },
            id::NEW_RECORD
            | id::DUPLICATE_RECORD
            | id::DELETE_RECORD
            | id::DELETE_ALL_RECORDS
            | id::REVERT_RECORD
            | id::OMIT_MULTIPLE => StepParams::Records {
                dialog,
                count: p.calc(0),
            },
            id::COMMIT => StepParams::Commit {
                dialog,
                skip_validation: o & opt::SKIP_VALIDATION != 0,
                force: o & opt::FORCE_COMMIT != 0,
            },
            id::ENTER_FIND_MODE | id::PERFORM_FIND => StepParams::Find {
                pause: o & opt::PAUSE != 0,
                restore: o & opt::RESTORE != 0,
                requests: find_requests(p),
            },
            id::ENTER_BROWSE_MODE | id::ENTER_PREVIEW_MODE => StepParams::Mode {
                pause: o & opt::PAUSE != 0,
            },
            id::SORT => StepParams::Sort {
                dialog,
                restore: o & opt::RESTORE != 0,
                keys: p.get(&[], 10).map(|b| sort_spec(&b)).unwrap_or_default(),
            },
            id::PAUSE_RESUME => StepParams::Pause(p.calc(0)),
            id::REFRESH_WINDOW => StepParams::RefreshWindow {
                flush_joins: o & opt::FLUSH_JOINS != 0,
                flush_external: o & opt::FLUSH_EXTERNAL != 0,
            },
            id::ELSE
            | id::END_IF
            | id::LOOP
            | id::END_LOOP
            | id::UNSORT
            | id::SHOW_ALL
            | id::OMIT_RECORD
            | id::SHOW_OMITTED
            | id::FREEZE_WINDOW
            | id::BEEP
            | id::HALT_SCRIPT => StepParams::None,
            _ => StepParams::Other,
        };
    }
}

fn be(b: &[u8]) -> u32 {
    b.iter().fold(0, |a, x| (a << 8) | *x as u32)
}

/// `01` + path integer: an object id (layout, script, field, value list).
fn id_ref(b: &[u8]) -> Option<u32> {
    match b {
        [1, rest @ ..] => path_int(rest),
        _ => None,
    }
}

/// Stored find requests: `[6].[6]` key 0 = count; `[6].[6].[r]` = request
/// r, key 0 (empty) when it omits; byte-string keys
/// `00 | TO ref | 01 field` → criterion text.
fn find_requests(p: &RawParams) -> Vec<FindRequest> {
    let mut out: BTreeMap<u32, FindRequest> = BTreeMap::new();
    for (path, key) in p.kv.keys() {
        if let [6, 6, r] = path.as_slice() {
            out.entry(*r).or_default().omit |= *key == 0;
        }
    }
    for (path, key, value) in &p.long {
        let [6, 6, r] = path.as_slice() else { continue };
        let (Some(to), Some(field)) = (
            to_ref(key.get(1..5).unwrap_or_default()),
            id_ref(key.get(5..).unwrap_or_default()),
        ) else {
            continue;
        };
        let f = FieldRef {
            to,
            field,
            repetition: 1,
        };
        out.entry(*r).or_default().criteria.push((f, text(value)));
    }
    out.into_values().collect()
}

/// Sort specification (step key 10): u32 count, then per key: TO, field
/// (length-prefixed), u16 repetition, two length-prefixed references (not
/// decoded), u16, value list (length-prefixed, empty if none), order byte
/// (0 ascending, 1 descending), u32.
pub(crate) fn sort_spec(b: &[u8]) -> Vec<(FieldRef, SortOrder)> {
    let mut i = 4;
    let mut out = Vec::new();
    let slot = |i: &mut usize| -> Option<Vec<u8>> {
        let n = *b.get(*i)? as usize;
        let v = b.get(*i + 1..*i + 1 + n)?.to_vec();
        *i += 1 + n;
        Some(v)
    };
    for _ in 0..be(b.get(..4).unwrap_or_default()) {
        let Some(to) = slot(&mut i).and_then(|s| to_ref(&s)) else {
            break;
        };
        let Some(field) = slot(&mut i).and_then(|s| id_ref(&s)) else {
            break;
        };
        let Some(rep) = b.get(i..i + 2) else { break };
        let repetition = u16::from_be_bytes([rep[0], rep[1]]);
        i += 2;
        // Two references not decoded yet (always `01 00` in the samples).
        if slot(&mut i).is_none() || slot(&mut i).is_none() {
            break;
        }
        i += 2;
        let vl = slot(&mut i).and_then(|s| id_ref(&s));
        let Some(order) = b.get(i) else { break };
        i += 1 + 4;
        let order = match (vl, order) {
            (Some(v), _) => SortOrder::Custom(v),
            (None, 1) => SortOrder::Descending,
            _ => SortOrder::Ascending,
        };
        out.push((
            FieldRef {
                to,
                field,
                repetition,
            },
            order,
        ));
    }
    out
}

/// Decodes a 28-byte step record (or a button's step message fields).
pub(crate) fn step_from(raw: Vec<u8>, raw_params: RawParams) -> Step {
    let mut r = raw.clone();
    r.resize(STEP_LEN, 0);
    // A disabled step stores its id negated.
    let signed = i16::from_be_bytes([r[20], r[21]]);
    let mut s = Step {
        id: signed.unsigned_abs(),
        enabled: signed >= 0,
        arg: u16::from_be_bytes([r[22], r[23]]),
        options: u32::from_be_bytes([r[24], r[25], r[26], r[27]]),
        raw,
        raw_params,
        params: StepParams::None,
    };
    s.decode();
    s
}

/// Number of the record's parameter node `[5].[n]`: bytes 0… are a
/// length-prefixed `01` + path integer.
pub(crate) fn step_number(r: &[u8]) -> Option<u32> {
    id_ref(r.get(1..1 + *r.first()? as usize)?)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub id: u32,
    pub name: String,
    /// Raw value of key 240 (XML `<Options>` text: 2 = separator, 8 = script, 9 = folder…).
    pub options: u64,
    pub steps: Vec<Step>,
}

impl Script {
    pub fn is_folder(&self) -> bool {
        self.options & 1 != 0
    }
    pub fn is_separator(&self) -> bool {
        self.options == 2
    }
}

#[derive(Default)]
struct RawScript {
    list: Vec<u8>,
    long_list: BTreeMap<u32, Vec<u8>>,
    params: BTreeMap<u32, RawParams>,
}

pub fn scripts(file: &File) -> Result<Vec<Script>, Error> {
    let mut out: BTreeMap<u32, Script> = BTreeMap::new();
    let mut raw: BTreeMap<u32, RawScript> = BTreeMap::new();
    // Display order: `[17].[1].[3]` maps a u32 position to a script id.
    let mut order: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match (p.as_slice(), c) {
            ([17, 1, 3], Chunk::KvLong { key, data }) => {
                if let Some(id) = crate::save::read_len_int(data) {
                    order.insert(key.to_vec(), id as u32);
                }
            }
            ([17, 5, id], Chunk::Kv { key, data }) => {
                let s = out.entry(*id).or_insert_with(|| Script {
                    id: *id,
                    ..Default::default()
                });
                match key {
                    16 => s.name = text(data),
                    240 => s.options = data.iter().fold(0, |a, b| (a << 8) | *b as u64),
                    4 => raw.entry(*id).or_default().list.extend_from_slice(data),
                    _ => {}
                }
            }
            // Step lists over 1000 bytes: keys 1…n (key 0 = total length).
            ([17, 5, id, 4], Chunk::Kv { key, data }) if *key >= 1 => {
                raw.entry(*id)
                    .or_default()
                    .long_list
                    .insert(*key, data.to_vec());
            }
            ([17, 5, id, 5, n, rest @ ..], c) => {
                raw.entry(*id)
                    .or_default()
                    .params
                    .entry(*n)
                    .or_default()
                    .add(rest, c);
                if let Some(r) = raw.get_mut(id).and_then(|r| r.params.get_mut(n)) {
                    for (v, e) in rest.iter().zip(&path[path.len() - rest.len()..]) {
                        if *v > 127 {
                            r.spelling.entry(*v).or_insert_with(|| e.to_vec());
                        }
                    }
                }
            }
            _ => {}
        }
    })?;
    for (id, mut r) in raw {
        let Some(s) = out.get_mut(&id) else { continue };
        let list: Vec<u8> = if r.list.is_empty() {
            r.long_list.into_values().flatten().collect()
        } else {
            r.list
        };
        s.steps = list
            .chunks(STEP_LEN)
            .enumerate()
            .map(|(i, c)| {
                let n = step_number(c).unwrap_or(i as u32 + 1);
                step_from(c.to_vec(), r.params.remove(&n).unwrap_or_default())
            })
            .collect();
    }
    let mut list: Vec<Script> = order.values().filter_map(|id| out.remove(id)).collect();
    list.extend(out.into_values());
    Ok(list)
}

/// A layout object's button action (object field 16): field 3 holds the
/// step as a message {1 id, 2 arg, 3 options, 4 the 14-byte inline block};
/// `params` is the tree under `[4].[5].[L].[5].[obj].[17]`. A button that
/// performs a script is a Perform Script step. `None` for "Do nothing".
pub fn button_action(raw16: &[u8], params: RawParams) -> Option<Step> {
    let m = Msg::parse(raw16)?.msg(3)?;
    let id = m.uint(1)? as u16;
    if id == 0 {
        return None;
    }
    let mut r = vec![0u8; STEP_LEN];
    if let Some(b) = m.bytes(4) {
        let n = b.len().min(14);
        r[6..6 + n].copy_from_slice(&b[..n]);
    }
    r[20..22].copy_from_slice(&id.to_be_bytes());
    r[22..24].copy_from_slice(&(m.uint(2).unwrap_or(0) as u16).to_be_bytes());
    r[24..28].copy_from_slice(&(m.uint(3).unwrap_or(0) as u32).to_be_bytes());
    Some(step_from(r, params))
}

/// A new step aimed at a layout (Go to Layout [layout]) or a script
/// (Perform Script [script]), as FileMaker writes them on buttons: the
/// target is the inline slot `len 01 path-int`. The byte after the slot is
/// `BE` or `6F` in FileMaker's files whatever the target (unused).
pub fn targeted_step(step: u16, target: u32) -> Step {
    let (arg, options) = match step {
        id::GO_TO_LAYOUT => (5, 8),
        _ => (4, 0x40),
    };
    let mut r = vec![0u8; STEP_LEN];
    let e = crate::save::path_elem(target);
    r[6] = 1 + e.len() as u8;
    r[7] = 1;
    r[8..8 + e.len()].copy_from_slice(&e);
    r[8 + e.len()] = 0xBE;
    r[20..22].copy_from_slice(&step.to_be_bytes());
    r[22..24].copy_from_slice(&(arg as u16).to_be_bytes());
    r[24..28].copy_from_slice(&(options as u32).to_be_bytes());
    step_from(r, RawParams::default())
}

/// Collects a button's parameter tree while walking a file.
pub(crate) fn add_param(p: &mut RawParams, rel: &[u32], c: &Chunk) {
    p.add(rel, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn catalogue_names() {
        assert_eq!(step_name(1), Some("Perform Script"));
        assert_eq!(step_name(89), Some("# (comment)"));
        assert_eq!(step_name(247), Some("Cancel PDF"));
        assert_eq!(step_name(2), None);
    }

    #[test]
    fn set_field_record_and_disabled_step() {
        // Set Field [Assignees::PrimaryKey] from the Tasks template.
        let r = hex("02011700bebe0403d000047f02010bbebfef0001004c000000004001");
        let s = step_from(r, RawParams::default());
        assert_eq!((s.id, s.enabled, s.options), (76, true, 0x4001));
        let StepParams::SetField { field, .. } = s.params else {
            panic!()
        };
        assert_eq!(
            field,
            Some(FieldRef {
                to: 132,
                field: 11,
                repetition: 1
            })
        );
        let d = step_from(
            hex(&["02013900", "0000", &"00".repeat(14), "ffa3"].concat()),
            RawParams::default(),
        );
        assert_eq!((d.id, d.enabled), (93, false));
    }

    #[test]
    fn sort_specs() {
        // Ascending Task, descending Due Date (Tasks), custom by value list 2.
        let b = hex(&[
            "00000002",
            // TO 130, field 1, rep 1, two empty refs, u16, no value list, ascending, u32
            "0403d00002",
            "020101",
            "0001",
            "020100020100",
            "0001",
            "00",
            "00",
            "00000000",
            // TO 130, field 4, rep 1, …, value list 2
            "0403d00002",
            "020104",
            "0001",
            "020100020100",
            "0001",
            "020102",
            "00",
            "00000000",
        ]
        .concat());
        assert_eq!(
            sort_spec(&b),
            vec![
                (
                    FieldRef {
                        to: 130,
                        field: 1,
                        repetition: 1
                    },
                    SortOrder::Ascending
                ),
                (
                    FieldRef {
                        to: 130,
                        field: 4,
                        repetition: 1
                    },
                    SortOrder::Custom(2)
                ),
            ]
        );
    }

    #[test]
    fn button_go_to_layout() {
        // Task List object 2: Go to Layout ["Task List"].
        let b = hex("080210001a16080610051808220e0201026f00000000000000000000280c3000");
        let s = button_action(&b, RawParams::default()).unwrap();
        assert_eq!(s.id, id::GO_TO_LAYOUT);
        assert_eq!(
            s.params,
            StepParams::GoToLayout {
                dest: LayoutDest::Layout(2),
                animation: 0
            }
        );
        // Perform Script 3 from a button.
        let b = hex("080010021a16080110041840220e020103be000000000000000000002006280d3001");
        let s = button_action(&b, RawParams::default()).unwrap();
        assert!(matches!(
            s.params,
            StepParams::PerformScript {
                script: Some(3),
                parameter: None
            }
        ));
    }
}
