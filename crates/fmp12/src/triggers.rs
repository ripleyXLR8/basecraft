//! Script triggers (`docs/FORMAT.md` §8, "Script triggers").
//!
//! A layout keeps its triggers in `[4].[5].[L].[48]`: key 4 (`00 01`,
//! not decoded), then one node `[48].[5].[128 + event]` per trigger whose
//! key 1 is the trigger record:
//!
//! | Bytes | Meaning |
//! |------:|---------|
//! | 0–1 | event id, u16 (101 = OnLayoutEnter, from the XML export) |
//! | 2… | script, length-prefixed `01` + path integer |
//! | +4 | modes, u32: 1 Browse (seen), 2 Find, 4 Preview (guessed) |
//! | +6 | zeros |
//!
//! ponytail: only OnLayoutEnter (101, Browse) appears in the templates.
//! Event ids other than 101, the mode bits other than Browse, the place of
//! object triggers (`[4].[5].[L].[5].[obj].[48]`), of file triggers
//! (any other `[…].[48].[5].[n]`) and of a trigger's parameter
//! (`[…].[128 + event].[5]`, a calculation) are guesses modelled on the
//! layout triggers. Ids follow the order of FileMaker's own trigger table
//! (DBEngine), where OnLayoutEnter is the first layout trigger.

use std::collections::BTreeMap;

use crate::scripts::Calc;
use crate::{Chunk, Error, File, int_path, path_int};

/// Trigger events by id.
pub mod event {
    pub const OBJECT_ENTER: u16 = 1;
    pub const OBJECT_VALIDATE: u16 = 2;
    pub const OBJECT_EXIT: u16 = 3;
    pub const OBJECT_MODIFY: u16 = 4;
    pub const OBJECT_SAVE: u16 = 5;
    pub const OBJECT_KEYSTROKE: u16 = 6;
    pub const PANEL_SWITCH: u16 = 7;
    pub const LAYOUT_ENTER: u16 = 101;
    pub const LAYOUT_EXIT: u16 = 102;
    pub const LAYOUT_KEYSTROKE: u16 = 103;
    pub const RECORD_LOAD: u16 = 104;
    pub const RECORD_COMMIT: u16 = 105;
    pub const RECORD_REVERT: u16 = 106;
    pub const MODE_ENTER: u16 = 107;
    pub const MODE_EXIT: u16 = 108;
    pub const VIEW_CHANGE: u16 = 109;
    pub const FIRST_WINDOW_OPEN: u16 = 201;
    pub const LAST_WINDOW_CLOSE: u16 = 202;
}

/// FileMaker's name of an event (XML `action`).
pub fn event_name(e: u16) -> Option<&'static str> {
    use event::*;
    Some(match e {
        OBJECT_ENTER => "OnObjectEnter",
        OBJECT_VALIDATE => "OnObjectValidate",
        OBJECT_EXIT => "OnObjectExit",
        OBJECT_MODIFY => "OnObjectModify",
        OBJECT_SAVE => "OnObjectSave",
        OBJECT_KEYSTROKE => "OnObjectKeystroke",
        PANEL_SWITCH => "OnPanelSwitch",
        LAYOUT_ENTER => "OnLayoutEnter",
        LAYOUT_EXIT => "OnLayoutExit",
        LAYOUT_KEYSTROKE => "OnLayoutKeystroke",
        RECORD_LOAD => "OnRecordLoad",
        RECORD_COMMIT => "OnRecordCommit",
        RECORD_REVERT => "OnRecordRevert",
        MODE_ENTER => "OnModeEnter",
        MODE_EXIT => "OnModeExit",
        VIEW_CHANGE => "OnViewChange",
        FIRST_WINDOW_OPEN => "OnFirstWindowOpen",
        LAST_WINDOW_CLOSE => "OnLastWindowClose",
        _ => return None,
    })
}

/// Mode bits of a layout trigger.
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
    /// Modes it runs in ([`mode`]); 0 for object and file triggers.
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

/// Decodes a trigger record (key 1 of `[48].[5].[n]`).
pub fn parse_record(b: &[u8]) -> Option<Trigger> {
    let event = u16::from_be_bytes([*b.first()?, *b.get(1)?]);
    let n = *b.get(2)? as usize;
    let slot = b.get(3..3 + n)?;
    let script = match slot {
        [1, rest @ ..] => path_int(rest)?,
        _ => return None,
    };
    let m = b.get(3 + n..7 + n)?;
    Some(Trigger {
        event,
        script,
        modes: u32::from_be_bytes([m[0], m[1], m[2], m[3]]),
        parameter: None,
    })
}

/// Encodes a trigger record (inverse of [`parse_record`]).
pub fn encode_record(t: &Trigger) -> Vec<u8> {
    let mut out = t.event.to_be_bytes().to_vec();
    let e = crate::save::path_elem(t.script);
    out.push(1 + e.len() as u8);
    out.push(1);
    out.extend(e);
    out.extend(t.modes.to_be_bytes());
    out.extend([0; 6]);
    out
}

/// Every trigger of the file.
pub fn triggers(file: &File) -> Result<Triggers, Error> {
    let mut out = Triggers::default();
    // (owner path, event node) → (record, parameter)
    type Found = (Option<Trigger>, Option<Calc>);
    let mut found: BTreeMap<(Vec<u32>, u32), Found> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        let Some(i) = p.windows(2).rposition(|w| w == [48, 5]) else {
            return;
        };
        let (owner, rest) = (p[..i].to_vec(), &p[i + 2..]);
        match (rest, c) {
            ([n], Chunk::Kv { key: 1, data }) => {
                found.entry((owner, *n)).or_default().0 = parse_record(data);
            }
            ([n, .., 5], Chunk::Kv { key: 5, data }) => {
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
            _ => out.file.push(t),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_enter_record() {
        // Contacts "Contact List": OnLayoutEnter → script 3, Browse mode.
        let b = [0, 0x65, 2, 1, 3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let t = parse_record(&b).unwrap();
        assert_eq!((t.event, t.script, t.modes), (101, 3, 1));
        assert_eq!(encode_record(&t), b);
        assert_eq!(event_name(t.event), Some("OnLayoutEnter"));
    }
}
