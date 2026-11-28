//! Script triggers that run before their event and can cancel it ("pre"
//! triggers: the script runs at once and an `Exit Script [ False ]`
//! cancels), and the ones the app detects itself: keystrokes, panel
//! switches, window size, window open and close, transactions. The
//! queued "post" triggers are `App::fire` and `App::fire_object`.
//!
//! Never fired, because a desktop client never sees their event:
//! OnObjectAVPlayerChange and OnFileAVPlayerChange (the AVPlayer steps
//! play media on FileMaker Go only), OnGestureTap (touch gestures,
//! FileMaker Go) and OnExternalCommandReceived (lock screen and remote
//! controls, FileMaker Go).

use std::collections::BTreeMap;

use eframe::egui;
use fmp12::scripts::Calc;
use fmp12::triggers::{Trigger, event};

use crate::{App, Mode};

impl App {
    /// The current layout's trigger for `ev` in the current mode (none in
    /// Layout mode).
    pub(crate) fn layout_trigger(&self, ev: u16) -> Option<Trigger> {
        let d = self.doc.as_ref()?;
        let l = d.layouts.get(self.layout)?;
        let bit = match self.mode() {
            Mode::Find => fmp12::triggers::mode::FIND,
            Mode::Preview => fmp12::triggers::mode::PREVIEW,
            Mode::Browse => fmp12::triggers::mode::BROWSE,
            Mode::Layout => return None,
        };
        d.triggers.layout(l.id, ev, bit).cloned()
    }

    pub(crate) fn object_trigger(&self, object: u64, ev: u16) -> Option<Trigger> {
        let d = self.doc.as_ref()?;
        let l = d.layouts.get(self.layout)?;
        d.triggers.object(l.id, object, ev).cloned()
    }

    /// Queues the file's trigger for `ev` (File Options).
    pub(crate) fn fire_file(&mut self, ev: u16) {
        if let Some(t) = self.doc.as_ref().and_then(|d| d.triggers.file(ev)) {
            self.trigger_queue
                .push_back((t.script, t.parameter.clone()));
        }
    }

    /// A trigger's optional parameter, evaluated on the current record.
    pub(crate) fn trigger_value(&mut self, param: Option<Calc>) -> Option<calc::Value> {
        let c = param?;
        let t = self.table()?;
        let rec = self.current_record();
        let d = self.doc.as_ref()?;
        let to = (d.layouts.get(self.layout)?.table_occurrence & 0xFFF) as u32;
        Some(d.db.eval_tokens(&c.0, t, to, rec))
    }

    /// Runs a "pre" trigger's script now, before its event. False when the
    /// script returns False (a non-empty result that is not true), which
    /// cancels the event, as in FileMaker. No trigger, or a script already
    /// running, lets the event happen.
    ///
    /// ponytail: a script that waits (a dialog, a pause) lets the event
    /// happen at once instead of after its answer.
    pub(crate) fn run_pre(&mut self, t: Option<Trigger>, info: script::TriggerInfo) -> bool {
        let Some(t) = t else { return true };
        if self.active.is_some() || self.engine.is_none() {
            return true;
        }
        let value = self.trigger_value(t.parameter);
        if let Some(e) = self.engine.as_mut() {
            e.trigger = info;
        }
        let was = std::mem::replace(&mut self.from_trigger, true);
        let r = self.with_host(|e, h| {
            let mut run = e.start(h, t.script, value);
            let st = run.resume(e, h, script::Answer::Start);
            (run, st)
        });
        let Some((run, st)) = r else {
            self.from_trigger = was;
            return true;
        };
        let ok = match &st {
            script::Status::Finished(o) => {
                self.from_trigger = was;
                o.result.is_empty() || o.result.truth()
            }
            script::Status::Suspended(_) => true,
        };
        self.after(run, st, 0.0);
        ok
    }

    /// OnModeExit, before leaving the current mode: false keeps it.
    pub(crate) fn mode_exit(&mut self) -> bool {
        let t = self.layout_trigger(event::MODE_EXIT);
        self.run_pre(t, Default::default())
    }

    /// OnRecordRevert, before Revert Record: false keeps the changes.
    pub(crate) fn record_revert(&mut self) -> bool {
        let t = self.layout_trigger(event::RECORD_REVERT);
        self.run_pre(t, Default::default())
    }

    /// OnWindowClose (and OnLastWindowClose for the file's last window),
    /// before window `name` closes: false keeps it open.
    ///
    /// ponytail: the script runs in the front window, also when a
    /// background window closes.
    pub(crate) fn window_close(&mut self, name: &str) -> bool {
        let last = self.window_names().len() == 1 && name == self.front_name();
        let t = self
            .doc
            .as_ref()
            .and_then(|d| d.triggers.file(event::WINDOW_CLOSE).cloned());
        if !self.run_pre(t, Default::default()) {
            return false;
        }
        let t = self
            .doc
            .as_ref()
            .and_then(|d| d.triggers.file(event::LAST_WINDOW_CLOSE).cloned());
        !last || self.run_pre(t, Default::default())
    }

    /// OnObjectKeystroke (the field with the focus) then OnLayoutKeystroke,
    /// before each keystroke of this frame reaches the layout; a False
    /// result drops the keystroke (and an object's False skips the layout's
    /// trigger). `Get ( TriggerKeystroke )` is the character, or FileMaker's
    /// control character for Enter, Tab, arrows…; menu shortcuts (Ctrl or
    /// Cmd) do not count.
    pub(crate) fn keystrokes(&mut self, ctx: &egui::Context) {
        if self.active.is_some() {
            return;
        }
        let focused =
            crate::toolbar::last_focus(ctx).is_some_and(|f| ctx.memory(|m| m.has_focus(f.id)));
        let obj = self
            .objs
            .focus
            .filter(|_| focused)
            .and_then(|o| self.object_trigger(o, event::OBJECT_KEYSTROKE));
        let lay = self.layout_trigger(event::LAYOUT_KEYSTROKE);
        if obj.is_none() && lay.is_none() {
            return;
        }
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let mut drop = Vec::new();
        for (n, e) in events.iter().enumerate() {
            let Some((keystroke, m)) = keystroke(e, mods) else {
                continue;
            };
            let info = script::TriggerInfo {
                keystroke,
                modifier_keys: modifier_keys(m),
                ..Default::default()
            };
            if !(self.run_pre(obj.clone(), info.clone()) && self.run_pre(lay.clone(), info)) {
                drop.push(n);
            }
        }
        if !drop.is_empty() {
            ctx.input_mut(|i| {
                let mut n = 0;
                i.events.retain(|_| {
                    n += 1;
                    !drop.contains(&(n - 1))
                });
            });
        }
    }

    /// OnPanelSwitch of the tab and slide controls whose front panel changed
    /// this frame (`before` is the choice before drawing); False puts the
    /// panel back. `Get ( TriggerCurrentPanel )` and `Get ( TriggerTargetPanel )`
    /// are "index¶name".
    pub(crate) fn panel_switches(&mut self, before: &BTreeMap<u64, u64>) {
        let changed: Vec<(u64, u64)> = self
            .tabs
            .iter()
            .filter(|(k, v)| before.get(k) != Some(v))
            .map(|(k, v)| (*k, *v))
            .collect();
        for (tc, to) in changed {
            let Some(l) = self.doc.as_ref().and_then(|d| d.layouts.get(self.layout)) else {
                return;
            };
            let mut panels: Vec<&fmp12::layouts::LayoutObject> =
                l.objects.iter().filter(|x| x.parent == Some(tc)).collect();
            panels.sort_by_key(|p| p.id);
            let from = before
                .get(&tc)
                .copied()
                .or_else(|| l.objects.iter().find(|x| x.id == tc)?.active_child)
                .or_else(|| panels.first().map(|p| p.id));
            if from == Some(to) {
                continue;
            }
            let info = |id: Option<u64>| {
                panels
                    .iter()
                    .position(|p| Some(p.id) == id)
                    .map(|i| format!("{}\r{}", i + 1, panels[i].name))
                    .unwrap_or_default()
            };
            let info = script::TriggerInfo {
                current_panel: info(from),
                target_panel: info(Some(to)),
                ..Default::default()
            };
            let t = self.object_trigger(tc, event::PANEL_SWITCH);
            if !self.run_pre(t, info) {
                match before.get(&tc) {
                    Some(p) => self.tabs.insert(tc, *p),
                    None => self.tabs.remove(&tc),
                };
            }
        }
    }

    /// OnLayoutSizeChange after the window's content size changed.
    pub(crate) fn layout_size(&mut self, size: egui::Vec2) {
        let name = self.front_name();
        if self.sizes.insert(name, size).is_some_and(|s| s != size) {
            self.fire(event::LAYOUT_SIZE_CHANGE);
        }
    }

    /// OnWindowTransaction after records were committed or deleted: the
    /// script parameter is FileMaker's JSON, `{ "file": { "base table":
    /// [ [ "Modified" | "Deleted", record id, field ] ] } }`, the field being
    /// the table's field named OnWindowTransaction (else "").
    ///
    /// ponytail: a commit is always "Modified" (a new record's first commit
    /// is "New" in FileMaker); a field's JSON goes as text; the trigger's
    /// Field Name option is not decoded.
    pub(crate) fn window_transaction(&mut self, op: &str, t: u32, records: &[usize]) {
        let Some(d) = self.doc.as_ref() else { return };
        let Some(trig) = d.triggers.file(event::WINDOW_TRANSACTION) else {
            return;
        };
        let Some(tbl) = d.db.table(t) else { return };
        let field = tbl.field_by_name("OnWindowTransaction").map(|f| f.id);
        let rows: Vec<String> = records
            .iter()
            .filter_map(|&r| {
                let id = tbl.records.get(r)?.id;
                let v = field
                    .map(|f| d.db.value(t, r, f).to_text())
                    .unwrap_or_default();
                Some(format!("[{},{id},{}]", json(op), json(&v)))
            })
            .collect();
        let file = d.name.trim_end_matches(".fmp12");
        let param = format!(
            "{{{}:{{{}:[{}]}}}}",
            json(file),
            json(&tbl.name),
            rows.join(",")
        );
        let script = trig.script;
        self.trigger_queue
            .push_back((script, Some(Calc(fmp12::save::text_literal_tokens(&param)))));
    }
}

/// A JSON string.
fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `Get ( TriggerKeystroke )` of an input event and its modifiers.
fn keystroke(e: &egui::Event, mods: egui::Modifiers) -> Option<(String, egui::Modifiers)> {
    use egui::Key::*;
    match e {
        egui::Event::Text(s) if !mods.command => Some((s.clone(), mods)),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if !modifiers.command => {
            let c = match key {
                Home => 1,
                End => 4,
                Backspace => 8,
                Tab => 9,
                PageUp => 11,
                PageDown => 12,
                Enter => 13,
                Escape => 27,
                ArrowLeft => 28,
                ArrowRight => 29,
                ArrowUp => 30,
                ArrowDown => 31,
                Delete => 127,
                _ => return None,
            };
            Some((char::from(c).to_string(), *modifiers))
        }
        _ => None,
    }
}

/// `Get ( TriggerModifierKeys )`: Shift 1, Ctrl 4, Alt 8, Cmd 16 (Caps
/// Lock, 2, is not known to egui).
fn modifier_keys(m: egui::Modifiers) -> u32 {
    u32::from(m.shift) | u32::from(m.ctrl) << 2 | u32::from(m.alt) << 3 | u32::from(m.mac_cmd) << 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystrokes_and_json() {
        let none = egui::Modifiers::NONE;
        let k = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: none,
        };
        assert_eq!(
            keystroke(&k(egui::Key::Enter), none).map(|x| x.0),
            Some("\r".into())
        );
        assert_eq!(keystroke(&k(egui::Key::A), none), None);
        assert_eq!(
            keystroke(&egui::Event::Text("é".into()), none).map(|x| x.0),
            Some("é".into())
        );
        assert_eq!(
            modifier_keys(egui::Modifiers::SHIFT | egui::Modifiers::ALT),
            9
        );
        assert_eq!(json("a\"b\r"), "\"a\\\"b\\r\"");
    }
}
