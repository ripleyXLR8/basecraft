//! Script triggers (`docs/FORMAT.md` §8, "Script triggers"), decoded by
//! setting every trigger in FileMaker 26 on copies of Tasks and diffing.
//!
//! A layout keeps its triggers in `[4].[5].[L].[48]`, an object in
//! `[4].[5].[L].[5].[obj].[48]`: key 4 = u16 number of triggers, then one
//! node `[48].[5].[128 + event]` per trigger. Its key 1 is the record
//!
//! | Bytes | Meaning |
//! |------:|---------|
//! | 0–1 | event id, u16 ([`event`]) |
//! | 2… | script, length-prefixed `01` + path integer |
//! | +1 | `00` (an empty length-prefixed slot; FileMaker 26 writes it, older files do not) |
//! | +4 | modes, u32 ([`mode`]) |
//! | +6 | zeros |
//!
//! and its parameter is a calculation in `[128 + event].[49]` (keys 4, 5, 6).
//!
//! File triggers are slots of the file options record `[2]` key 24 (see
//! [`file_triggers`]). ponytail: their parameter was not found (FileMaker's
//! File Options dialog offered none to the UI driver); `parameter` is `None`.

use std::collections::BTreeMap;

use crate::scripts::Calc;
use crate::{Chunk, Error, File, int_path, path_int};

/// Trigger events by id (XML `ScriptTrigger id`).
pub mod event {
    pub const OBJECT_ENTER: u16 = 1;
    pub const OBJECT_EXIT: u16 = 2;
    pub const OBJECT_MODIFY: u16 = 3;
    pub const OBJECT_SAVE: u16 = 4;
    pub const OBJECT_KEYSTROKE: u16 = 5;
    pub const OBJECT_VALIDATE: u16 = 6;
    /// ponytail: by elimination (the one object event between 6 and 8 that
    /// a field does not offer); not seen.
    pub const PANEL_SWITCH: u16 = 7;
    pub const OBJECT_AVPLAYER_CHANGE: u16 = 8;
    pub const LAYOUT_ENTER: u16 = 101;
    pub const LAYOUT_KEYSTROKE: u16 = 102;
    pub const RECORD_LOAD: u16 = 103;
    pub const RECORD_COMMIT: u16 = 104;
    pub const RECORD_REVERT: u16 = 105;
    pub const MODE_ENTER: u16 = 106;
    pub const MODE_EXIT: u16 = 107;
    pub const LAYOUT_EXIT: u16 = 109;
    pub const VIEW_CHANGE: u16 = 110;
    pub const LAYOUT_SIZE_CHANGE: u16 = 111;
    pub const GESTURE_TAP: u16 = 112;
    pub const EXTERNAL_COMMAND_RECEIVED: u16 = 113;
    pub const FIRST_WINDOW_OPEN: u16 = 201;
    pub const LAST_WINDOW_CLOSE: u16 = 202;
    pub const WINDOW_OPEN: u16 = 204;
    pub const WINDOW_CLOSE: u16 = 205;
    pub const FILE_AVPLAYER_CHANGE: u16 = 206;
    pub const WINDOW_TRANSACTION: u16 = 209;
}

/// FileMaker's name of an event (XML `action`).
pub fn event_name(e: u16) -> Option<&'static str> {
    use event::*;
    Some(match e {
        OBJECT_ENTER => "OnObjectEnter",
        OBJECT_EXIT => "OnObjectExit",
        OBJECT_MODIFY => "OnObjectModify",
        OBJECT_SAVE => "OnObjectSave",
        OBJECT_KEYSTROKE => "OnObjectKeystroke",
        OBJECT_VALIDATE => "OnObjectValidate",
        PANEL_SWITCH => "OnPanelSwitch",
        OBJECT_AVPLAYER_CHANGE => "OnObjectAVPlayerChange",
        LAYOUT_ENTER => "OnLayoutEnter",
        LAYOUT_KEYSTROKE => "OnLayoutKeystroke",
        RECORD_LOAD => "OnRecordLoad",
        RECORD_COMMIT => "OnRecordCommit",
        RECORD_REVERT => "OnRecordRevert",
        MODE_ENTER => "OnModeEnter",
        MODE_EXIT => "OnModeExit",
        LAYOUT_EXIT => "OnLayoutExit",
        VIEW_CHANGE => "OnViewChange",
        LAYOUT_SIZE_CHANGE => "OnLayoutSizeChange",
        GESTURE_TAP => "OnGestureTap",
        EXTERNAL_COMMAND_RECEIVED => "OnExternalCommandReceived",
        FIRST_WINDOW_OPEN => "OnFirstWindowOpen",
        LAST_WINDOW_CLOSE => "OnLastWindowClose",
        WINDOW_OPEN => "OnWindowOpen",
        WINDOW_CLOSE => "OnWindowClose",
        FILE_AVPLAYER_CHANGE => "OnFileAVPlayerChange",
        WINDOW_TRANSACTION => "OnWindowTransaction",
        _ => return None,
    })
}

/// Mode bits of a layout or object trigger (all three seen).
pub mod mode {
    pub const BROWSE: u32 = 1;
    pub const FIND: u32 = 2;
    pub const PREVIEW: u32 = 4;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub event: u16,
    /// Script id.
    pub script: u32,
    /// Modes it runs in ([`mode`]); 0 for file triggers.
    pub modes: u32,
    pub parameter: Option<Calc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Triggers {
    /// Layout id → its triggers.
    pub layouts: BTreeMap<u32, Vec<Trigger>>,
    /// (layout id, object id) → its triggers.
    pub objects: BTreeMap<(u32, u64), Vec<Trigger>>,
    /// File triggers (OnFirstWindowOpen…).
    pub file: Vec<Trigger>,
    /// File Options › Open › "Switch to layout": the layout the first
    /// window opens on.
    pub open_layout: Option<u32>,
}

impl Triggers {
    /// Script and parameter of the layout's trigger for `event` in a mode.
    pub fn layout(&self, layout: u32, event: u16, mode_bit: u32) -> Option<&Trigger> {
        self.layouts
            .get(&layout)?
            .iter()
            .find(|t| t.event == event && (t.modes == 0 || t.modes & mode_bit != 0))
    }

    pub fn object(&self, layout: u32, object: u64, event: u16) -> Option<&Trigger> {
        self.objects
            .get(&(layout, object))?
            .iter()
            .find(|t| t.event == event)
    }

    pub fn file(&self, event: u16) -> Option<&Trigger> {
        self.file.iter().find(|t| t.event == event)
    }
}

/// A length-prefixed script slot at `b[i]`: (script id if any, bytes used).
fn slot(b: &[u8], i: usize) -> Option<(Option<u32>, usize)> {
    let n = *b.get(i)? as usize;
    let s = b.get(i + 1..i + 1 + n)?;
    let id = match s {
        [] => None,
        [1, rest @ ..] => Some(path_int(rest)?),
        _ => return None,
    };
    Some((id, 1 + n))
}

/// Decodes a trigger record (key 1 of `[48].[5].[n]`).
pub fn parse_record(b: &[u8]) -> Option<Trigger> {
    let event = u16::from_be_bytes([*b.first()?, *b.get(1)?]);
    let (script, n) = slot(b, 2)?;
    let mut i = 2 + n;
    // FileMaker 26 puts an empty slot before the modes (16-byte record).
    if b.len() - i > 10 {
        i += slot(b, i)?.1;
    }
    let m = b.get(i..i + 4)?;
    Some(Trigger {
        event,
        script: script?,
        modes: u32::from_be_bytes([m[0], m[1], m[2], m[3]]),
        parameter: None,
    })
}

/// Encodes a trigger record as FileMaker 26 writes it.
pub fn encode_record(t: &Trigger) -> Vec<u8> {
    let mut out = t.event.to_be_bytes().to_vec();
    let e = crate::save::path_elem(t.script);
    out.push(1 + e.len() as u8);
    out.push(1);
    out.extend(e);
    out.push(0);
    out.extend(t.modes.to_be_bytes());
    out.extend([0; 6]);
    out
}

/// File triggers in the file options record (`[2]` key 24): u32, two
/// bytes, then script slots and fixed fields in this order:
/// OnFirstWindowOpen, OnLastWindowClose, the layout to show on open,
/// u16, u32, u16, two bytes, OnWindowOpen, OnWindowClose, u32,
/// OnFileAVPlayerChange, u16, u16, OnWindowTransaction (absent in older
/// files). A slot is `00` when empty.
pub fn file_triggers(b: &[u8]) -> Vec<Trigger> {
    use event::*;
    // (event, or None for a slot that is not a trigger) and the fixed bytes after it.
    let plan: [(Option<u16>, usize); 8] = [
        (Some(FIRST_WINDOW_OPEN), 0),
        (Some(LAST_WINDOW_CLOSE), 0),
        (None, 10),
        (Some(WINDOW_OPEN), 0),
        (Some(WINDOW_CLOSE), 4),
        (Some(FILE_AVPLAYER_CHANGE), 4),
        (Some(WINDOW_TRANSACTION), 0),
        (None, 0),
    ];
    let mut out = Vec::new();
    let mut i = 6;
    for (ev, skip) in plan {
        let Some((script, n)) = slot(b, i) else { break };
        i += n + skip;
        if let (Some(event), Some(script)) = (ev, script) {
            out.push(Trigger {
                event,
                script,
                modes: 0,
                parameter: None,
            });
        }
    }
    out
}

/// The layout slot of the file options record (`[2]` key 24), after the
/// OnFirstWindowOpen and OnLastWindowClose slots.
///
/// ponytail: the slot always names a layout; the u16 after it is taken as
/// the "Switch to layout" switch (2 on, 1 off), from the starter templates
/// (1, open on their first layout) and one user file (2, opens on the slot's
/// layout). Confirm by toggling the option in FileMaker's File Options.
pub fn open_layout(b: &[u8]) -> Option<u32> {
    let mut i = 6;
    for _ in 0..2 {
        i += slot(b, i)?.1;
    }
    let (layout, n) = slot(b, i)?;
    let on = b.get(i + n..i + n + 2)? == [0, 2];
    if on { layout } else { None }
}

/// Every trigger of the file.
pub fn triggers(file: &File) -> Result<Triggers, Error> {
    let mut out = Triggers::default();
    // (owner path, event node) → (record, parameter)
    type Found = (Option<Trigger>, Option<Calc>);
    let mut found: BTreeMap<(Vec<u32>, u32), Found> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        if p == [2]
            && let Chunk::Kv { key: 24, data } = c
        {
            out.file = file_triggers(data);
            out.open_layout = open_layout(data);
            return;
        }
        let Some(i) = p.windows(2).rposition(|w| w == [48, 5]) else {
            return;
        };
        let (owner, rest) = (p[..i].to_vec(), &p[i + 2..]);
        match (rest, c) {
            ([n], Chunk::Kv { key: 1, data }) => {
                found.entry((owner, *n)).or_default().0 = parse_record(data);
            }
            ([n, 49], Chunk::Kv { key: 5, data }) => {
                found.entry((owner, *n)).or_default().1 = Some(Calc(data.to_vec()));
            }
            _ => {}
        }
    })?;
    for ((owner, _), (t, param)) in found {
        let Some(mut t) = t else { continue };
        t.parameter = param;
        match owner.as_slice() {
            [4, 5, l] => out.layouts.entry(*l).or_default().push(t),
            [4, 5, l, 5, obj] => out.objects.entry((*l, *obj as u64)).or_default().push(t),
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_records() {
        // Contacts "Contact List" (older file): OnLayoutEnter → script 3, Browse.
        let old = [0, 0x65, 2, 1, 3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let t = parse_record(&old).unwrap();
        assert_eq!((t.event, t.script, t.modes), (101, 3, 1));
        assert_eq!(event_name(t.event), Some("OnLayoutEnter"));
        // FileMaker 26: OnObjectKeystroke → script 7, Find only.
        let new = [0, 5, 2, 1, 7, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        let t = parse_record(&new).unwrap();
        assert_eq!((t.event, t.script, t.modes), (5, 7, mode::FIND));
        assert_eq!(encode_record(&t), new);
    }

    #[test]
    fn file_trigger_slots() {
        let hex = "000004430101020102020103020103000100000001000002000201040201050000000002010603e7000102010700";
        let b: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let got: Vec<(u16, u32)> = file_triggers(&b)
            .iter()
            .map(|t| (t.event, t.script))
            .collect();
        assert_eq!(
            got,
            [(201, 2), (202, 3), (204, 4), (205, 5), (206, 6), (209, 7)]
        );
        // No triggers in the templates.
        let tpl = "0000044301010000020103000100000001000002000000000000000003e70001";
        let b: Vec<u8> = (0..tpl.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&tpl[i..i + 2], 16).unwrap())
            .collect();
        assert!(file_triggers(&b).is_empty());
        // The templates name layout 3 but open on their first layout.
        assert_eq!(open_layout(&b), None);
        // "Switch to layout" with layout 100 and no file triggers.
        let user = "0000044304010000020164000200000001000002000000000000000000640001";
        let b: Vec<u8> = (0..user.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&user[i..i + 2], 16).unwrap())
            .collect();
        assert!(file_triggers(&b).is_empty());
        assert_eq!(open_layout(&b), Some(100));
    }
}
