//! Running FileMaker scripts headless (CLI `run-script`, MCP `run_script`).

use engine::Database;
use script::{Dialog, DialogAnswer, Host, Window};
use serde_json::{Value as Json, json};

use crate::{Doc, Res, value_json};

/// Host without a screen: dialogs are logged and answered with `answer`.
struct Headless<'a> {
    db: &'a mut Database,
    window: Window,
    answer: usize,
    dialogs: Vec<Json>,
}

impl Host for Headless<'_> {
    fn db(&mut self) -> &mut Database {
        self.db
    }
    fn window(&mut self) -> &mut Window {
        &mut self.window
    }
    fn dialog(&mut self, d: &Dialog) -> DialogAnswer {
        self.dialogs.push(
            json!({ "title": d.title, "message": d.message, "buttons": d.buttons,
                                  "answered": self.answer }),
        );
        DialogAnswer {
            button: self.answer,
            inputs: Vec::new(),
        }
    }
}

pub fn list(d: &Doc) -> Res<Json> {
    let engine = d.engine()?;
    Ok(
        json!({ "scripts": engine.scripts.iter().filter(|s| !s.is_folder() && !s.steps.is_empty() && d.db.script_access(s.id) != fmp12::security::NO_ACCESS).map(|s| json!({
        "id": s.id,
        "name": s.name,
        "steps": s.steps.iter().map(|st| fmp12::scripts::step_name(st.id).unwrap_or("?")).collect::<Vec<_>>(),
    })).collect::<Vec<_>>() }),
    )
}

/// Runs script `name` (or numeric id) from layout `layout` (default: the first layout).
pub fn run(
    d: &mut Doc,
    name: &str,
    param: Option<&str>,
    layout: Option<&str>,
    answer: usize,
) -> Res<Json> {
    let layout_id = match layout {
        Some(l) => {
            d.db.layouts
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(l))
                .map(|x| x.id)
                .ok_or_else(|| format!("no layout named {l:?}"))?
        }
        None => {
            d.db.layouts
                .iter()
                .find(|x| x.table_occurrence != 0)
                .map(|x| x.id)
                .ok_or("file has no layouts")?
        }
    };
    let mut engine = d.take_engine()?;
    let target = engine
        .scripts
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name) || name.parse() == Ok(s.id))
        .map(|s| s.id);
    if target.is_some_and(|id| d.db.script_access(id) == fmp12::security::NO_ACCESS) {
        d.engine = Some(engine);
        return Err(format!("{} (error {})", engine::DENIED_MESSAGE, engine::RECORD_DENIED));
    }
    let window = Window::new(&d.db, layout_id);
    let mut host = Headless {
        db: &mut d.db,
        window,
        answer: answer.max(1),
        dialogs: Vec::new(),
    };
    let param = param.map(calc::Value::text);
    let outcome = match name.parse::<u32>() {
        Ok(id) => engine.perform(&mut host, id, param),
        Err(_) => engine.perform_named(&mut host, name, param),
    };
    let Headless {
        window, dialogs, ..
    } = host;
    d.engine = Some(engine);
    let layout = d.db.layouts.iter().find(|l| l.id == window.layout);
    let table = layout.and_then(|l| d.db.table_of((l.table_occurrence & 0xFFF) as u32));
    let current_id = table.and_then(|t| {
        let i = *window.found.get(window.current)?;
        Some(t.records.get(i)?.id)
    });
    Ok(json!({
        "result": value_json(&outcome.result),
        "last_error": outcome.last_error,
        "halted": outcome.halted,
        "dialogs": dialogs,
        "window": {
            "layout": layout.map(|l| l.name.clone()),
            "mode": format!("{:?}", window.mode),
            "found": window.found.len(),
            "current_record_id": current_id,
        },
    }))
}
