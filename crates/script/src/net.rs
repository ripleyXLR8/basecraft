//! Insert from URL, Execute FileMaker Data API, Save Records as JSONL and
//! Save a Copy as XML, as Claris documents them. Native builds only: the
//! web build has no sockets and returns FileMaker's "not supported" (3)
//! for all four, as before.
//!
//! Where FileMaker keeps their options was read from steps FileMaker 26
//! wrote (pasted, then read back in its XML export and the file; FORMAT.md
//! §8): Insert from URL's URL at parameter 0 and cURL options at 1, its
//! target like Insert Text's (`[160]` key 1 or the inline field), option
//! bits `0x1000` select, `0x80` no dialog, `0x100` do not encode the URL,
//! `0x1000_0000` verify SSL certificates; Execute FileMaker Data API the
//! request at 0, `0x1000` select, the same targets; Save Records as JSONL
//! `0x1000` format for fine-tuning (prompts at 2 system, 3 user, 4
//! assistant), `0x1` with a completion field (inline), `0x200` create
//! folders; Save a Copy as XML `0x100` details for analysis tools, `0x200`
//! each object's binary data, `0x8000` options as JSON (parameter 1;
//! parameter 0 is the window). Output files: `[128]` key 130.

use super::*;

pub const SAVE_AS_XML: u16 = 3;
pub const INSERT_FROM_URL: u16 = 160;
pub const DATA_API: u16 = 203;
pub const SAVE_AS_JSONL: u16 = 225;

/// Whether `id` is one of the steps run here.
pub fn handles(id: u16) -> bool {
    matches!(id, SAVE_AS_XML | INSERT_FROM_URL | DATA_API | SAVE_AS_JSONL)
}

/// Insert from URL's request running in the background: its URL and
/// thread ([`Host::background_requests`]).
#[derive(Default)]
pub(crate) struct Fetch(
    #[cfg(not(target_arch = "wasm32"))] Option<(String, std::thread::JoinHandle<curl::Outcome>)>,
);

impl Engine {
    /// Whether Insert from URL's background request has its answer (or
    /// none runs): the run can go on ([`UiRequest::Fetch`]).
    pub fn fetch_done(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.fetch.0.as_ref().is_none_or(|(_, h)| h.is_finished())
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Fetch() = self.fetch;
            true
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Engine {
    pub(crate) fn exec_net(
        &mut self,
        _host: &mut dyn Host,
        _frame: &mut Frame,
        _s: &Step,
    ) -> Result<Flow, u32> {
        Err(error::UNAVAILABLE)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod curl;
#[cfg(not(target_arch = "wasm32"))]
pub mod dataapi;
#[cfg(not(target_arch = "wasm32"))]
pub mod export;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::datafiles::{target, variable, write_result};

    /// Option bits (record bytes 24–27).
    const SELECT: u32 = 0x1000;
    const VERIFY_SSL: u32 = 0x1000_0000;
    const FINE_TUNING: u32 = 0x1000;
    /// Save Records as JSONL: the inline field is the completion field.
    const COMPLETION: u32 = 0x1;
    /// Insert from URL: "Do not automatically encode URL".
    const NO_ENCODE: u32 = 0x100;
    const CREATE_FOLDERS: u32 = 0x200;
    /// Save Records as JSONL: a table ([128] key 17).
    const TABLE: u32 = 0x100;

    impl Engine {
        pub(crate) fn exec_net(
            &mut self,
            host: &mut dyn Host,
            frame: &mut Frame,
            s: &Step,
        ) -> Result<Flow, u32> {
            match s.id {
                INSERT_FROM_URL => self.insert_from_url(host, frame, s),
                DATA_API => {
                    let req = match s.raw_params.calc(0) {
                        Some(c) => self.eval(host, frame, &c)?.to_text(),
                        None => String::new(),
                    };
                    let file = host.window().host.file_name.clone();
                    let (code, out) = dataapi::execute(host, &file, &req);
                    self.deliver(host, frame, s, Value::text(out), s.options & SELECT != 0)?;
                    if code != 0 {
                        self.step_detail = dataapi::message(code).to_string();
                        return Err(code);
                    }
                    Ok(Flow::Next)
                }
                SAVE_AS_JSONL => {
                    let data = self.jsonl(host, frame, s)?;
                    self.save(host, s, "Records.jsonl", &data)
                }
                _ => {
                    let w = host.window().host.file_name.clone();
                    let xml = export::save_as_xml(host.db(), &self.scripts, &w);
                    let name = format!("{}.xml", if w.is_empty() { "Untitled" } else { &w });
                    self.save(host, s, &name, xml.as_bytes())
                }
            }
        }

        /// Writes `data` to the step's output file, or asks where
        /// (`Host::save_file`; cancelled: 1).
        fn save(
            &mut self,
            host: &mut dyn Host,
            s: &Step,
            name: &str,
            data: &[u8],
        ) -> Result<Flow, u32> {
            match crate::stored_path(&s.raw_params) {
                Some(path) => {
                    let folders = s.id == SAVE_AS_JSONL && s.options & CREATE_FOLDERS != 0;
                    write_result(host.file_write(&path, data, folders), &path)
                }
                None if host.save_file(name, data) => Ok(Flow::Next),
                None => Err(error::USER_CANCELED),
            }
        }

        /// Puts a result in the step's target: a variable (appended to
        /// unless `select` or container data), the inline field or the
        /// active one (inserted, or replacing with `select`).
        fn deliver(
            &mut self,
            host: &mut dyn Host,
            frame: &mut Frame,
            s: &Step,
            v: Value,
            select: bool,
        ) -> Result<(), u32> {
            let t = match target(s) {
                Target::None => host.window().field.map_or(Target::None, Target::Field),
                t => t,
            };
            match t {
                Target::Variable(name) => {
                    let old = variable(&self.globals, frame, &name);
                    let v = match (&v, &old) {
                        (Value::Container(_), _) | (_, Value::Container(_)) => v,
                        _ if select => v,
                        _ => Value::text(format!("{}{}", old.to_text(), v.to_text())),
                    };
                    let key = (name.to_lowercase(), 1);
                    if name.starts_with("$$") {
                        self.globals.insert(key, v);
                    } else {
                        frame.locals.insert(key, v);
                    }
                    Ok(())
                }
                Target::Field(f) => match v {
                    Value::Container(_) => self.set_field(host, f, v),
                    v => self.insert(host, f, &v.to_text(), select),
                },
                Target::None => Err(error::FIELD_MISSING),
            }
        }

        fn insert_from_url(
            &mut self,
            host: &mut dyn Host,
            frame: &mut Frame,
            s: &Step,
        ) -> Result<Flow, u32> {
            let p = &s.raw_params;
            let url = match p.calc(0) {
                Some(c) => self.eval(host, frame, &c)?.to_text(),
                None => String::new(),
            };
            let opts = match p.calc(1) {
                Some(c) => self.eval(host, frame, &c)?.to_text(),
                None => String::new(),
            };
            let o = curl::parse(&opts);
            // `$var` values of the options (`-d @$data`, `-b $cookies`…).
            let globals = &self.globals;
            let read = |name: &str| variable(globals, frame, name);
            let url = if s.options & NO_ENCODE != 0 {
                url
            } else {
                curl::encode_url(&url)
            };
            let (req, result) = curl::request(&url, &o, s.options & VERIFY_SSL != 0, &read);
            let req = req?;
            // ponytail: the step runs twice in the background case (to
            // start the request, then to take its response), so its
            // calculations are evaluated twice; keep the request in the
            // pending state if one ever has side effects that matter.
            let outcome = match self.fetch.0.take() {
                Some((u, h)) if u == req.url => h.join().unwrap_or_else(|_| curl::Outcome {
                    response: None,
                    code: curl::CONNECTION_FAILED,
                    detail: String::new(),
                }),
                _ if host.background_requests() => {
                    let (r, opts) = (req.clone(), o.clone());
                    let h = std::thread::spawn(move || curl::exchange(&r, &opts));
                    self.fetch.0 = Some((req.url.clone(), h));
                    return Ok(Flow::Suspend(
                        Suspend::Ui(UiRequest::Fetch { url: req.url }),
                        Pending::Ui(UiPending::Rerun),
                    ));
                }
                _ => curl::exchange(&req, &o),
            };
            if let Some(v) = &o.dump_header {
                let text = outcome.response.as_ref().map(|r| r.header_text());
                let key = (v.to_lowercase(), 1);
                let val = Value::text(text.unwrap_or_default());
                if v.starts_with("$$") {
                    self.globals.insert(key, val);
                } else {
                    frame.locals.insert(key, val);
                }
            }
            self.step_detail = outcome.detail.clone();
            let Some(resp) = outcome.response else {
                return Err(outcome.code);
            };
            let container = o.container
                || match target(s) {
                    Target::Field(f) => host
                        .db()
                        .table_of(f.to)
                        .and_then(|t| t.field(f.field))
                        .is_some_and(|x| x.data_type == fmp12::schema::DataType::Container),
                    _ => false,
                };
            let empty = resp.body.is_empty() && !req.method.eq_ignore_ascii_case("HEAD");
            let v = if req.method.eq_ignore_ascii_case("HEAD") {
                Value::text(resp.header_text())
            } else if container {
                Value::Container(calc::value::Container {
                    name: result.name.clone(),
                    data: resp.body.clone(),
                    ..Default::default()
                })
            } else {
                Value::text(curl::decode(&resp))
            };
            if !empty {
                self.deliver(host, frame, s, v, s.options & SELECT != 0)?;
            }
            match outcome.code {
                0 if empty => Err(10),
                0 => Ok(Flow::Next),
                code => Err(code),
            }
        }

        /// Save Records as JSONL: one line per record of the table's found
        /// set (the window's, or the current record's related records).
        fn jsonl(
            &mut self,
            host: &mut dyn Host,
            frame: &mut Frame,
            s: &Step,
        ) -> Result<Vec<u8>, u32> {
            use serde_json::json;
            let (table, layout_to, _) = Self::context(host);
            let completion = s.inline_field().filter(|_| s.options & COMPLETION != 0);
            // The Table option (`0x100`): its occurrence at [128] key 17.
            let table_to = (s.options & TABLE != 0)
                .then(|| s.raw_params.get(&[128], 17))
                .flatten()
                .and_then(|b| fmp12::graph::to_ref(&b));
            let to = table_to.unwrap_or(completion.map_or(layout_to, |f| f.to));
            let tid = host.db().table_of(to).ok_or(error::FIELD_MISSING)?.id;
            let w = host.window().clone();
            let records: Vec<usize> = if tid == table {
                w.found.clone()
            } else {
                match w.record() {
                    Some(r) => host.db().related(layout_to, r, to),
                    None => Vec::new(),
                }
            };
            let mut out = Vec::new();
            let fine = s.options & FINE_TUNING != 0;
            if !fine && completion.is_none() {
                return Err(error::FIELD_MISSING);
            }
            for &r in &records {
                let mut messages = Vec::new();
                if fine {
                    // ponytail: the prompts are evaluated on each record
                    // only for the layout's table; related records see the
                    // current one.
                    if tid == table
                        && let Some(pos) = w.found.iter().position(|&x| x == r)
                    {
                        host.window().current = pos;
                    }
                    let mut text = |e: &mut Self, h: &mut dyn Host, p: u32| -> Result<_, u32> {
                        Ok(match s.raw_params.calc(p) {
                            Some(c) => Some(e.eval(h, frame, &c)?.to_text()),
                            None => None,
                        })
                    };
                    let system = text(self, host, 2)?;
                    let user = text(self, host, 3)?.unwrap_or_default();
                    let assistant = text(self, host, 4)?.unwrap_or_default();
                    if let Some(sys) = system.filter(|x| !x.is_empty()) {
                        messages.push(json!({"content": sys, "role": "system"}));
                    }
                    messages.push(json!({"content": user, "role": "user"}));
                    messages.push(json!({"content": assistant, "role": "assistant"}));
                } else {
                    let db = host.db();
                    let Some(t) = db.table(tid) else { break };
                    let c = completion.map(|f| f.field);
                    let answer = c.map(|f| db.value(tid, r, f).to_text()).unwrap_or_default();
                    if answer.is_empty() {
                        continue;
                    }
                    let user: Vec<String> = t
                        .fields
                        .iter()
                        .filter(|f| {
                            Some(f.id) != c && f.data_type != fmp12::schema::DataType::Container
                        })
                        .map(|f| format!("{}={}", f.name, db.value(tid, r, f.id).to_text()))
                        .collect();
                    // ponytail: FileMaker's predefined system prompt is
                    // not documented; no system message is written.
                    messages.push(json!({"content": user.join(", "), "role": "user"}));
                    messages.push(json!({"content": answer, "role": "assistant"}));
                }
                out.extend(json!({ "messages": messages }).to_string().into_bytes());
                out.push(b'\n');
            }
            if fine && tid == table {
                host.window().current = w.current;
            }
            Ok(out)
        }
    }
}
