//! FileMaker Data API client: files hosted by FileMaker Server, opened
//! through `/fmi/data/vLatest` (login, layouts, records, find, edits,
//! scripts, containers) and mapped onto the engine (`source`).
//!
//! Synchronous and native only: the web build has no sockets, so the
//! crate is empty on `wasm32`.
//!
//! Passwords and session tokens are never logged, printed or stored:
//! [`Client`] keeps the token in memory, `Debug` hides it, and
//! [`Client::logout`] (or dropping the client) ends the session.

#![cfg(not(target_arch = "wasm32"))]

pub mod source;

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value as Json, json};

/// A Data API failure. FileMaker's own errors keep their code (the codes
/// `Get ( LastError )` gives), so BaseCraft reports them as FileMaker does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// FileMaker error `code` (with the server's message).
    FileMaker { code: u32, message: String },
    /// An HTTP error without a FileMaker error code.
    Http { status: u16, message: String },
    /// No connection, TLS failure (untrusted certificate…), timeout.
    Transport(String),
    /// A reply BaseCraft does not understand, or a bad URL.
    Protocol(String),
}

/// What a FileMaker error means for BaseCraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 212: wrong account or password (FileMaker's login alert).
    Login,
    /// 9, 200–216 (but 212): the account's privileges refuse it.
    Denied,
    /// 952: the session expired; log in again.
    Expired,
    /// 101 record, 102 field, 104 script, 105 layout, 802 file missing.
    Missing,
    /// 401: no records match the find.
    NoMatch,
    /// 301 record in use, 306 modification id changed.
    Conflict,
    /// 500–511: field validation.
    Validation,
    Other,
}

impl Error {
    /// The FileMaker error code, if the server gave one.
    pub fn code(&self) -> Option<u32> {
        match self {
            Error::FileMaker { code, .. } => Some(*code),
            _ => None,
        }
    }

    pub fn kind(&self) -> Kind {
        match self.code() {
            Some(212) => Kind::Login,
            Some(9 | 200..=216) => Kind::Denied,
            Some(952) => Kind::Expired,
            Some(101 | 102 | 104 | 105 | 802) => Kind::Missing,
            Some(401) => Kind::NoMatch,
            Some(301 | 306) => Kind::Conflict,
            Some(500..=511) => Kind::Validation,
            _ => Kind::Other,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::FileMaker { code, message } => {
                let what = match code {
                    212 => "The account name and password do not give access to this file.",
                    9 => "Your privileges do not allow this action.",
                    802 => {
                        "The file is not hosted, or no privilege set of it has the \
                         fmrest extended privilege (Access via FileMaker Data API)."
                    }
                    306 => "The record was changed by someone else since it was read.",
                    301 => "The record is being modified by another user.",
                    952 => "The session expired: log in again.",
                    _ => "",
                };
                if what.is_empty() {
                    write!(f, "FileMaker error {code}: {message}")
                } else {
                    write!(f, "{what} (FileMaker error {code}: {message})")
                }
            }
            Error::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            Error::Transport(m) => write!(f, "connection failed: {m}"),
            Error::Protocol(m) => write!(f, "unexpected reply: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Where a hosted file is: `https://host[:port]` and the database name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub database: String,
}

impl Target {
    /// `https://host/Database` (also `https://host/fmi/data/…/databases/Database`).
    pub fn parse(url: &str) -> Result<Target> {
        let url = url.trim().trim_end_matches('/');
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| Error::Protocol(format!("{url:?} is not an http(s) URL")))?;
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let db = path.rsplit('/').next().unwrap_or("");
        if db.is_empty() || host.is_empty() {
            return Err(Error::Protocol(format!(
                "{url:?}: expected https://host/Database"
            )));
        }
        Ok(Target {
            host: format!("{scheme}://{host}"),
            database: unescape(db),
        })
    }
}

/// How to reach the server.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Accept any certificate (self-signed test servers). Off by default:
    /// with it on, anyone between BaseCraft and the server can read the
    /// password and the data.
    pub insecure: bool,
}

/// The warning shown when a connection accepts any certificate.
pub const INSECURE_WARNING: &str = "Certificate checks are off for this connection: anyone \
     between BaseCraft and the server could read the password and the data. Use only with \
     your own test servers.";

/// One find request: `field → criterion` (FileMaker's find syntax), omit or not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindRequest {
    pub criteria: Vec<(String, String)>,
    pub omit: bool,
}

/// Range, sort and portals of a records or find call.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// First record, from 1 (0 = the server's default, 1).
    pub offset: usize,
    /// Number of records (None = the server's default, 100).
    pub limit: Option<usize>,
    /// Field name and descending order.
    pub sort: Vec<(String, bool)>,
    /// Portals whose rows to return (empty: all of them).
    pub portals: Vec<String>,
}

/// A record as the Data API gives it: values as text, numbers in
/// JSON's notation, dates as `MM/dd/yyyy`, containers as URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteRecord {
    pub id: u32,
    pub mod_id: u32,
    pub fields: BTreeMap<String, String>,
    /// Portal name (object name, else occurrence) → related rows.
    pub portals: BTreeMap<String, Vec<RemoteRecord>>,
}

/// A page of records and the found set's size.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub total: usize,
    pub found: usize,
    pub records: Vec<RemoteRecord>,
}

/// A field of a layout (`fieldMetaData`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldMeta {
    /// `Field`, or `Occurrence::Field` for a related field.
    pub name: String,
    /// `normal`, `calculation`, `summary`.
    pub kind: String,
    /// `text`, `number`, `date`, `time`, `timestamp`, `container`.
    pub result: String,
    pub repetitions: u32,
    pub global: bool,
    pub value_list: Option<String>,
}

/// Fields, portals and value lists of a layout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutMeta {
    pub fields: Vec<FieldMeta>,
    pub portals: BTreeMap<String, Vec<FieldMeta>>,
    /// Value list name → values (display text when it differs).
    pub value_lists: BTreeMap<String, Vec<String>>,
}

/// A script's outcome: its last error and `Exit Script` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptResult {
    pub error: u32,
    pub result: Option<String>,
}

/// An open Data API session on one hosted file.
pub struct Client {
    agent: ureq::Agent,
    /// `https://host/fmi/data/vLatest`
    base: String,
    pub target: Target,
    token: Option<String>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("target", &self.target)
            .field("logged_in", &self.token.is_some())
            .finish()
    }
}

fn agent(options: Options) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .disable_verification(options.insecure)
        .build();
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(120)))
        .tls_config(tls)
        .build()
        .into()
}

/// Plain HTTP only to this machine (tests); a password must not cross a
/// network in clear.
fn check_scheme(host: &str) -> Result<()> {
    let h = host.to_ascii_lowercase();
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| h == *p || h.starts_with(&format!("{p}:")));
    if h.starts_with("https://") || local {
        Ok(())
    } else {
        Err(Error::Protocol(format!(
            "{host}: use https:// (plain http only for this machine)"
        )))
    }
}

/// Percent-encodes a path segment or query value.
fn esc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b[i], hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A JSON value as FileMaker text.
fn text(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Null => String::new(),
        other => other.to_string(),
    }
}

fn num(v: Option<&Json>) -> u32 {
    v.map(text).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// The reply's FileMaker error (`messages[0].code`), 0 when none.
fn reply_code(j: &Json) -> (u32, String) {
    let m = &j["messages"][0];
    (num(m.get("code")), text(&m["message"]))
}

fn record(j: &Json) -> RemoteRecord {
    let fields = |o: &Json| {
        o.as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), text(v))).collect())
            .unwrap_or_default()
    };
    let mut r = RemoteRecord {
        id: num(j.get("recordId")),
        mod_id: num(j.get("modId")),
        fields: fields(&j["fieldData"]),
        portals: BTreeMap::new(),
    };
    if let Some(ps) = j["portalData"].as_object() {
        for (name, rows) in ps {
            let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
            let list = rows
                .iter()
                .map(|row| {
                    let mut f: BTreeMap<String, String> = fields(row);
                    f.remove("recordId");
                    f.remove("modId");
                    RemoteRecord {
                        id: num(row.get("recordId")),
                        mod_id: num(row.get("modId")),
                        fields: f,
                        portals: BTreeMap::new(),
                    }
                })
                .collect();
            r.portals.insert(name.clone(), list);
        }
    }
    r
}

fn page(j: &Json) -> Page {
    let info = &j["response"]["dataInfo"];
    Page {
        total: num(info.get("totalRecordCount")) as usize,
        found: num(info.get("foundCount")) as usize,
        records: j["response"]["data"]
            .as_array()
            .map(|a| a.iter().map(record).collect())
            .unwrap_or_default(),
    }
}

fn field_meta(j: &Json) -> FieldMeta {
    FieldMeta {
        name: text(&j["name"]),
        kind: text(&j["type"]),
        result: text(&j["result"]),
        repetitions: num(j.get("maxRepeat")).max(1),
        global: j["global"].as_bool().unwrap_or(false),
        value_list: j.get("valueList").map(text),
    }
}

fn layout_meta(j: &Json) -> LayoutMeta {
    let r = &j["response"];
    let list = |a: &Json| -> Vec<FieldMeta> {
        a.as_array()
            .map(|a| a.iter().map(field_meta).collect())
            .unwrap_or_default()
    };
    LayoutMeta {
        fields: list(&r["fieldMetaData"]),
        portals: r["portalMetaData"]
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), list(v))).collect())
            .unwrap_or_default(),
        value_lists: r["valueLists"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|vl| {
                        let values = vl["values"]
                            .as_array()
                            .map(|v| v.iter().map(|x| text(&x["value"])).collect())
                            .unwrap_or_default();
                        (text(&vl["name"]), values)
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Layout names, folders flattened.
fn layout_names(a: &Json, out: &mut Vec<String>) {
    for l in a.as_array().map(Vec::as_slice).unwrap_or_default() {
        if l["isFolder"].as_bool() == Some(true) {
            layout_names(&l["folderLayoutNames"], out);
        } else {
            out.push(text(&l["name"]));
        }
    }
}

fn sort_json(sort: &[(String, bool)]) -> Json {
    Json::Array(
        sort.iter()
            .map(|(f, desc)| {
                json!({"fieldName": f, "sortOrder": if *desc { "descend" } else { "ascend" }})
            })
            .collect(),
    )
}

fn fields_json(fields: &[(String, String)]) -> Json {
    Json::Object(
        fields
            .iter()
            .map(|(k, v)| (k.clone(), Json::String(v.clone())))
            .collect(),
    )
}

/// Sends a request and reads the JSON reply; FileMaker errors become
/// [`Error::FileMaker`].
fn exchange(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    auth: &str,
    body: Option<&Json>,
) -> Result<Json> {
    let transport = |e: ureq::Error| Error::Transport(e.to_string());
    let mut resp = match (method, body) {
        ("GET", _) => agent.get(url).header("Authorization", auth).call(),
        ("DELETE", _) => agent.delete(url).header("Authorization", auth).call(),
        (m, b) => {
            let b = b.cloned().unwrap_or_else(|| json!({})).to_string();
            let req = match m {
                "PATCH" => agent.patch(url),
                _ => agent.post(url),
            };
            req.header("Authorization", auth)
                .header("Content-Type", "application/json")
                .send(b)
        }
    }
    .map_err(transport)?;
    let status = resp.status().as_u16();
    let raw = resp
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_string()
        .map_err(transport)?;
    let j: Json = match serde_json::from_str(&raw) {
        Ok(j) => j,
        Err(_) => {
            let message: String = raw.chars().take(200).collect();
            return Err(if (200..300).contains(&status) {
                Error::Protocol(message)
            } else {
                Error::Http { status, message }
            });
        }
    };
    let (code, message) = reply_code(&j);
    if code != 0 {
        return Err(Error::FileMaker { code, message });
    }
    if !(200..300).contains(&status) {
        return Err(Error::Http { status, message });
    }
    Ok(j)
}

fn basic(account: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64(format!("{account}:{password}").as_bytes())
    )
}

fn api(host: &str) -> String {
    format!("{}/fmi/data/vLatest", host.trim_end_matches('/'))
}

/// Databases the server lists for this account (`GET /databases`).
pub fn databases(
    host: &str,
    account: &str,
    password: &str,
    options: Options,
) -> Result<Vec<String>> {
    check_scheme(host)?;
    let j = exchange(
        &agent(options),
        "GET",
        &format!("{}/databases", api(host)),
        &basic(account, password),
        None,
    )?;
    Ok(j["response"]["databases"]
        .as_array()
        .map(|a| a.iter().map(|d| text(&d["name"])).collect())
        .unwrap_or_default())
}

impl Client {
    /// Logs in (`POST /sessions`, Basic auth) and keeps the session token.
    pub fn login(
        target: &Target,
        account: &str,
        password: &str,
        options: Options,
    ) -> Result<Client> {
        check_scheme(&target.host)?;
        let agent = agent(options);
        let base = api(&target.host);
        let j = exchange(
            &agent,
            "POST",
            &format!("{base}/databases/{}/sessions", esc(&target.database)),
            &basic(account, password),
            Some(&json!({})),
        )?;
        let token = j["response"]["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::Protocol("no session token".into()))?
            .to_string();
        Ok(Client {
            agent,
            base,
            target: target.clone(),
            token: Some(token),
        })
    }

    /// Ends the session (`DELETE /sessions/{token}`).
    pub fn logout(mut self) -> Result<()> {
        self.end()
    }

    fn end(&mut self) -> Result<()> {
        let Some(t) = self.token.take() else {
            return Ok(());
        };
        let url = format!("{}/sessions/{}", self.db_url(), esc(&t));
        exchange(&self.agent, "DELETE", &url, &format!("Bearer {t}"), None).map(|_| ())
    }

    fn db_url(&self) -> String {
        format!("{}/databases/{}", self.base, esc(&self.target.database))
    }

    fn call(&self, method: &str, path: &str, body: Option<&Json>) -> Result<Json> {
        let auth = format!("Bearer {}", self.token.as_deref().unwrap_or(""));
        exchange(
            &self.agent,
            method,
            &format!("{}/{path}", self.db_url()),
            &auth,
            body,
        )
    }

    fn layout_path(layout: &str) -> String {
        format!("layouts/{}", esc(layout))
    }

    /// Layout names (folders flattened).
    pub fn layouts(&self) -> Result<Vec<String>> {
        let j = self.call("GET", "layouts", None)?;
        let mut out = Vec::new();
        layout_names(&j["response"]["layouts"], &mut out);
        Ok(out)
    }

    /// Script names (folders flattened).
    pub fn scripts(&self) -> Result<Vec<String>> {
        let j = self.call("GET", "scripts", None)?;
        let mut out = Vec::new();
        layout_names(&j["response"]["scripts"], &mut out);
        Ok(out)
    }

    /// Fields, portals and value lists of a layout.
    pub fn layout(&self, layout: &str) -> Result<LayoutMeta> {
        Ok(layout_meta(&self.call(
            "GET",
            &Self::layout_path(layout),
            None,
        )?))
    }

    fn range(q: &Query, body: &mut serde_json::Map<String, Json>) {
        if q.offset > 0 {
            body.insert("offset".into(), json!(q.offset.to_string()));
        }
        if let Some(l) = q.limit {
            body.insert("limit".into(), json!(l.to_string()));
        }
        if !q.sort.is_empty() {
            body.insert("sort".into(), sort_json(&q.sort));
        }
        if !q.portals.is_empty() {
            body.insert("portal".into(), json!(q.portals));
        }
    }

    /// Records of a layout's table (`GET …/records`): range, sort, portals.
    pub fn records(&self, layout: &str, q: &Query) -> Result<Page> {
        let mut query = Vec::new();
        if q.offset > 0 {
            query.push(format!("_offset={}", q.offset));
        }
        if let Some(l) = q.limit {
            query.push(format!("_limit={l}"));
        }
        if !q.sort.is_empty() {
            query.push(format!("_sort={}", esc(&sort_json(&q.sort).to_string())));
        }
        if !q.portals.is_empty() {
            query.push(format!("portal={}", esc(&json!(q.portals).to_string())));
        }
        let mut path = format!("{}/records", Self::layout_path(layout));
        if !query.is_empty() {
            path = format!("{path}?{}", query.join("&"));
        }
        Ok(page(&self.call("GET", &path, None)?))
    }

    /// One record by id.
    pub fn record(&self, layout: &str, id: u32) -> Result<RemoteRecord> {
        let j = self.call(
            "GET",
            &format!("{}/records/{id}", Self::layout_path(layout)),
            None,
        )?;
        page(&j)
            .records
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("no record in the reply".into()))
    }

    /// Find (`POST …/_find`): FileMaker find requests, in order, omit
    /// requests included. No match (401) is an empty page.
    pub fn find(&self, layout: &str, requests: &[FindRequest], q: &Query) -> Result<Page> {
        let query: Vec<Json> = requests
            .iter()
            .map(|r| {
                let mut m: serde_json::Map<String, Json> = r
                    .criteria
                    .iter()
                    .map(|(k, v)| (k.clone(), Json::String(v.clone())))
                    .collect();
                if r.omit {
                    m.insert("omit".into(), json!("true"));
                }
                Json::Object(m)
            })
            .collect();
        let mut body = serde_json::Map::new();
        body.insert("query".into(), Json::Array(query));
        Self::range(q, &mut body);
        match self.call(
            "POST",
            &format!("{}/_find", Self::layout_path(layout)),
            Some(&Json::Object(body)),
        ) {
            Err(e) if e.kind() == Kind::NoMatch => Ok(Page::default()),
            r => r.map(|j| page(&j)),
        }
    }

    /// New record with these values; returns its id and modification id.
    pub fn create(&self, layout: &str, fields: &[(String, String)]) -> Result<(u32, u32)> {
        let j = self.call(
            "POST",
            &format!("{}/records", Self::layout_path(layout)),
            Some(&json!({"fieldData": fields_json(fields)})),
        )?;
        Ok((
            num(j["response"].get("recordId")),
            num(j["response"].get("modId")),
        ))
    }

    /// Edits a record; with `mod_id`, only if nobody changed it since
    /// (else error 306). Returns the new modification id.
    pub fn edit(
        &self,
        layout: &str,
        id: u32,
        fields: &[(String, String)],
        mod_id: Option<u32>,
    ) -> Result<u32> {
        let mut body = json!({"fieldData": fields_json(fields)});
        if let Some(m) = mod_id {
            body["modId"] = json!(m.to_string());
        }
        let j = self.call(
            "PATCH",
            &format!("{}/records/{id}", Self::layout_path(layout)),
            Some(&body),
        )?;
        Ok(num(j["response"].get("modId")))
    }

    pub fn delete(&self, layout: &str, id: u32) -> Result<()> {
        self.call(
            "DELETE",
            &format!("{}/records/{id}", Self::layout_path(layout)),
            None,
        )
        .map(|_| ())
    }

    /// Runs a script on the server (`GET …/script/{name}`) in the
    /// context of `layout`, with an optional parameter.
    pub fn run_script(
        &self,
        layout: &str,
        script: &str,
        param: Option<&str>,
    ) -> Result<ScriptResult> {
        let mut path = format!("{}/script/{}", Self::layout_path(layout), esc(script));
        if let Some(p) = param {
            path = format!("{path}?script.param={}", esc(p));
        }
        let j = self.call("GET", &path, None)?;
        let r = &j["response"];
        Ok(ScriptResult {
            error: num(r.get("scriptError")),
            result: r.get("scriptResult").map(text),
        })
    }

    /// Downloads a container (the URL a record gives for it). FileMaker
    /// Server answers the first request with a cookie and a redirect to
    /// the same URL; the second request, with the cookie, gets the data.
    /// Only the URL's path is used, on the host logged into: the server
    /// may name itself differently (another port behind a proxy), and
    /// nothing is fetched from anywhere else.
    pub fn container(&self, url: &str) -> Result<Vec<u8>> {
        let transport = |e: ureq::Error| Error::Transport(e.to_string());
        let on_host = |u: &str| -> Result<String> {
            let path = match u.split_once("://") {
                Some((_, rest)) => rest.find('/').map_or("/", |i| &rest[i..]),
                None => u,
            };
            if !path.starts_with("/Streaming") {
                return Err(Error::Protocol(format!(
                    "{path}: not a FileMaker container URL"
                )));
            }
            Ok(format!("{}{path}", self.target.host))
        };
        let mut url = on_host(url)?;
        let mut cookie = String::new();
        for _ in 0..4 {
            let mut req = self.agent.get(&url);
            if !cookie.is_empty() {
                req = req.header("Cookie", &cookie);
            }
            let mut resp = req.call().map_err(transport)?;
            let status = resp.status().as_u16();
            let cookies: Vec<String> = resp
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .filter_map(|v| v.split(';').next())
                .map(str::to_string)
                .collect();
            if !cookies.is_empty() {
                cookie = cookies.join("; ");
            }
            if (300..400).contains(&status) {
                let loc = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                url = on_host(&loc)?;
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(Error::Http {
                    status,
                    message: "container download".into(),
                });
            }
            return resp
                .body_mut()
                .with_config()
                .limit(2 * 1024 * 1024 * 1024)
                .read_to_vec()
                .map_err(transport);
        }
        Err(Error::Protocol("too many container redirects".into()))
    }
}

impl Drop for Client {
    /// Ends the session: FileMaker Server counts open sessions.
    fn drop(&mut self) {
        let _ = self.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        assert_eq!(base64(b"api:test123"), "YXBpOnRlc3QxMjM=");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(esc("My File é"), "My%20File%20%C3%A9");
        assert_eq!(unescape("My%20File%20%C3%A9"), "My File é");
    }

    #[test]
    fn targets() {
        let t = Target::parse("https://fms.example.com:8443/Sales%20Data/").unwrap();
        assert_eq!(t.host, "https://fms.example.com:8443");
        assert_eq!(t.database, "Sales Data");
        assert!(Target::parse("https://host").is_err());
        assert!(check_scheme("http://fms.example.com").is_err());
        assert!(check_scheme("http://127.0.0.1:9000").is_ok());
        assert!(check_scheme("https://fms.example.com").is_ok());
    }
}
