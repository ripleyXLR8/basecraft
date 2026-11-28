//! Insert from URL's cURL options (Claris, "Supported cURL options"):
//! the request they describe and the errors FileMaker gives.
//!
//! Done: `-X`/`--request`, `-H`/`--header`, `-d`/`--data`, `--data-ascii`,
//! `--data-raw`, `--data-binary`, `--data-urlencode` (`@$var` sends a
//! variable), `-I`/`--head`, `-D`/`--dump-header $var`, `-u`/`--user`
//! (basic), `-b`/`--cookie`, `-A`/`--user-agent`, `-e`/`--referer`,
//! `-L`/`--location`, `--connect-timeout`, `-m`/`--max-time`,
//! `-k`/`--insecure`, `-S`/`--show-error`, `-o`/`--output` (the
//! container's name), `--upload-file $var` (PUT),
//! `--FM-return-container-variable`. Other options are ignored, as
//! FileMaker ignores unsupported ones (their value is skipped). Only http
//! and https: other protocols give 1626.

use std::time::Duration;

use super::*;
use remote::http::{Failure, Request, Response};

/// Protocol not supported, authentication failed, URL format incorrect,
/// connection failed (Claris error codes).
pub const PROTOCOL: u32 = 1626;
pub const AUTH_FAILED: u32 = 1627;
pub const BAD_URL: u32 = 1630;
pub const CONNECTION_FAILED: u32 = 1631;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Options {
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    /// `-d` parts (joined with `&`): (kind, value).
    pub data: Vec<(Data, String)>,
    pub head: bool,
    pub dump_header: Option<String>,
    pub user: Option<String>,
    pub cookie: Option<String>,
    pub follow: bool,
    pub connect_timeout: Option<f64>,
    pub max_time: Option<f64>,
    pub insecure: bool,
    pub show_error: bool,
    pub output: Option<String>,
    pub upload: Option<String>,
    pub container: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Data {
    /// `-d`, `--data-ascii`: `@` reads a variable, CR and LF dropped.
    Ascii,
    /// `--data-raw`: as typed.
    Raw,
    /// `--data-binary`: `@` reads a variable as is.
    Binary,
    /// `--data-urlencode`: `name=value` with the value percent-encoded.
    UrlEncode,
}

/// Splits the options as a shell would: blanks separate, `"…"` and `'…'`
/// group, `\` escapes the next character.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut any = false;
    let mut quote: Option<char> = None;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match (quote, c) {
            (None, c) if c.is_whitespace() => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => cur.push(c),
            (_, '\\') => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
                any = true;
            }
            (_, c) => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// Options of FileMaker's list that take a value BaseCraft ignores.
const IGNORED_WITH_VALUE: &[&str] = &[
    "--aws-sigv4",
    "--cert",
    "-E",
    "--cert-type",
    "--ciphers",
    "--key",
    "--key-type",
    "--pass",
    "--pubkey",
    "--form",
    "-F",
    "--form-string",
    "--range",
    "-r",
    "--time-cond",
    "-z",
    "--continue-at",
    "-C",
    "--cookie-jar",
    "-c",
    "--expect100-timeout",
    "--keepalive-time",
    "--limit-rate",
    "--max-filesize",
    "--speed-limit",
    "-Y",
    "--speed-time",
    "-y",
    "--interface",
    "--local-port",
    "--resolve",
    "--proxy",
    "-x",
    "--proxy-user",
    "-U",
    "--proxy-header",
    "--noproxy",
    "--socks4",
    "--socks4a",
    "--socks5",
    "--socks5-hostname",
    "--ftp-alternative-to-user",
    "--ftp-method",
    "--ftp-port",
    "-P",
    "--ftp-ssl-ccc-mode",
    "--quote",
    "-Q",
    "--mail-auth",
    "--mail-from",
    "--mail-rcpt",
    "--trace",
    "--trace-ascii",
    "--FM-text-encoding",
];

pub fn parse(s: &str) -> Options {
    let mut o = Options::default();
    let w = words(s);
    let mut it = w.into_iter();
    while let Some(a) = it.next() {
        // `--opt=value` is not curl syntax; `-Hvalue` is.
        let (name, inline) = match a.as_str() {
            x if x.starts_with("--") => (x.to_string(), None),
            x if x.len() > 2 && x.starts_with('-') && x.is_char_boundary(2) => {
                (x[..2].to_string(), Some(x[2..].to_string()))
            }
            x => (x.to_string(), None),
        };
        let val = |it: &mut std::vec::IntoIter<String>| inline.clone().or_else(|| it.next());
        match name.as_str() {
            "-X" | "--request" => o.method = val(&mut it),
            "-H" | "--header" => {
                if let Some((k, v)) = val(&mut it).as_deref().and_then(|h| h.split_once(':'))
                    && !v.trim().is_empty()
                {
                    o.headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            "-d" | "--data" | "--data-ascii" => {
                o.data.extend(val(&mut it).map(|v| (Data::Ascii, v)))
            }
            "--data-raw" => o.data.extend(val(&mut it).map(|v| (Data::Raw, v))),
            "--data-binary" => o.data.extend(val(&mut it).map(|v| (Data::Binary, v))),
            "--data-urlencode" => o.data.extend(val(&mut it).map(|v| (Data::UrlEncode, v))),
            "-I" | "--head" => o.head = true,
            "-D" | "--dump-header" => o.dump_header = val(&mut it).filter(|v| v.starts_with('$')),
            "-u" | "--user" => o.user = val(&mut it),
            "-b" | "--cookie" => o.cookie = val(&mut it),
            "-A" | "--user-agent" => o
                .headers
                .extend(val(&mut it).map(|v| ("User-Agent".to_string(), v))),
            "-e" | "--referer" => o
                .headers
                .extend(val(&mut it).map(|v| ("Referer".to_string(), v))),
            "-L" | "--location" | "--location-trusted" => o.follow = true,
            "--connect-timeout" => o.connect_timeout = val(&mut it).and_then(|v| v.parse().ok()),
            "-m" | "--max-time" => o.max_time = val(&mut it).and_then(|v| v.parse().ok()),
            "-k" | "--insecure" => o.insecure = true,
            "-S" | "--show-error" => o.show_error = true,
            "-o" | "--output" => o.output = val(&mut it),
            "-T" | "--upload-file" => o.upload = val(&mut it),
            "--FM-return-container-variable" => o.container = true,
            x if IGNORED_WITH_VALUE.contains(&x) => {
                val(&mut it);
            }
            _ => {}
        }
    }
    o
}

/// What the step makes of the response beyond its bytes.
#[derive(Debug, Default)]
pub struct Named {
    /// Name of container data: `--output`, else the URL's last segment.
    pub name: String,
}

fn bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Container(c) => c.data.clone(),
        v => v.to_text().into_bytes(),
    }
}

/// Percent-encodes what cannot stand in a URL (blanks, quotes, non-ASCII…),
/// keeping its reserved characters and existing escapes: "Automatically
/// encode URL".
fn encode_url(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'!'..=b'~' if !b"\"<>\\^`{|}".contains(&b) => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `application/x-www-form-urlencoded` encoding of a value.
fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A `$var` (or `@$var` with `at`) names a variable: its contents.
fn value_of(v: &str, at: bool, read: &dyn Fn(&str) -> Value) -> Vec<u8> {
    let name = if at { v.strip_prefix('@') } else { Some(v) };
    match name {
        Some(n) if n.starts_with('$') => bytes(&read(n)),
        _ => v.as_bytes().to_vec(),
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = c.iter().fold(0u32, |a, &b| a << 8 | b as u32) << (8 * (3 - c.len()));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The request of the URL and options, or FileMaker's error for the URL.
pub fn request(
    url: &str,
    o: &Options,
    verify: bool,
    read: &dyn Fn(&str) -> Value,
) -> (Result<Request, u32>, Named) {
    let url = encode_url(url);
    let name = o.output.clone().unwrap_or_else(|| {
        let path = url.split(['?', '#']).next().unwrap_or("");
        let last = path.rsplit('/').next().unwrap_or("");
        let host_only = path.matches('/').count() <= 2;
        if last.is_empty() || host_only {
            "index.html".to_string()
        } else {
            last.to_string()
        }
    });
    let result = Named { name };
    let Some((scheme, rest)) = url.split_once("://") else {
        return (Err(BAD_URL), result);
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return (Err(PROTOCOL), result);
    }
    if rest.is_empty() || rest.starts_with('/') {
        return (Err(BAD_URL), result);
    }
    let mut headers = o.headers.clone();
    let has = |h: &[(String, String)], k: &str| h.iter().any(|(x, _)| x.eq_ignore_ascii_case(k));
    let mut body = None;
    if !o.data.is_empty() {
        let mut b: Vec<u8> = Vec::new();
        for (i, (kind, v)) in o.data.iter().enumerate() {
            if i > 0 {
                b.push(b'&');
            }
            match kind {
                Data::Ascii => b.extend(
                    value_of(v, true, read)
                        .into_iter()
                        .filter(|c| *c != b'\r' && *c != b'\n'),
                ),
                Data::Binary => b.extend(value_of(v, true, read)),
                Data::Raw => b.extend(v.as_bytes()),
                Data::UrlEncode => {
                    let (n, val) = match v.split_once('=') {
                        Some((n, val)) => (Some(n), val.to_string()),
                        None => (None, v.clone()),
                    };
                    let val = match val.strip_prefix('@') {
                        Some(x) if x.starts_with('$') => read(x).to_text(),
                        _ => val,
                    };
                    if let Some(n) = n.filter(|n| !n.is_empty()) {
                        b.extend(n.as_bytes());
                        b.push(b'=');
                    }
                    b.extend(form_encode(&val).into_bytes());
                }
            }
        }
        if !has(&headers, "Content-Type") {
            headers.push((
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ));
        }
        body = Some(b);
    }
    if let Some(u) = &o.upload {
        body = Some(value_of(u, false, read));
    }
    if let Some(u) = &o.user {
        let u = String::from_utf8_lossy(&value_of(u, false, read)).into_owned();
        headers.push((
            "Authorization".into(),
            format!("Basic {}", base64(u.as_bytes())),
        ));
    }
    if let Some(c) = &o.cookie {
        let c = String::from_utf8_lossy(&value_of(c, false, read)).into_owned();
        if c.contains('=') {
            headers.push(("Cookie".into(), c));
        }
    }
    let method = match (&o.method, o.head, &o.upload, &body) {
        (Some(m), ..) => m.to_ascii_uppercase(),
        (None, true, ..) => "HEAD".into(),
        (None, _, Some(_), _) => "PUT".into(),
        (None, _, _, Some(_)) => "POST".into(),
        _ => "GET".into(),
    };
    let secs = |s: Option<f64>| s.filter(|s| *s > 0.0).map(Duration::from_secs_f64);
    let req = Request {
        method,
        url,
        headers,
        body,
        verify: verify && !o.insecure,
        follow: o.follow,
        connect_timeout: secs(o.connect_timeout),
        max_time: secs(o.max_time),
    };
    (Ok(req), result)
}

/// A response (none when the connection failed), FileMaker's error and
/// `Get ( LastErrorDetail )`.
pub struct Outcome {
    pub response: Option<Response>,
    pub code: u32,
    pub detail: String,
}

/// Sends the request. Errors as Claris documents them: 401 gives 1627;
/// with `--show-error`, a failed connection gives 1631 and curl's message,
/// a status of 400 or more 1631 and "Response code: nnn"; without it,
/// other statuses give 0 and a failed connection 1631 with no detail.
pub fn exchange(req: &Request, o: &Options) -> Outcome {
    match remote::http::send(req) {
        Ok(r) => {
            let (code, detail) = match r.status {
                401 => (AUTH_FAILED, String::new()),
                s if s >= 400 && o.show_error => (CONNECTION_FAILED, format!("Response code: {s}")),
                _ => (0, String::new()),
            };
            Outcome {
                response: Some(r),
                code,
                detail,
            }
        }
        Err(e) => {
            let m = match e {
                Failure::Url(m)
                | Failure::Connect(m)
                | Failure::Timeout(m)
                | Failure::Tls(m)
                | Failure::Other(m) => m,
            };
            Outcome {
                response: None,
                code: CONNECTION_FAILED,
                detail: if o.show_error { m } else { String::new() },
            }
        }
    }
}

/// The body as text: UTF-8, or Latin-1 when the response says so.
pub fn decode(r: &Response) -> String {
    let latin1 = r.header("Content-Type").is_some_and(|c| {
        let c = c.to_ascii_lowercase();
        c.contains("charset=iso-8859-1") || c.contains("charset=latin1")
    });
    if latin1 {
        r.body.iter().map(|&b| b as char).collect()
    } else {
        String::from_utf8_lossy(&r.body).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_split_like_curl() {
        let o = parse(
            r#"-X put --header "Content-Type: application/json" -d @$body -L -m 2.5 -Hx:1 --cert c.pem -k -D $h"#,
        );
        assert_eq!(o.method.as_deref(), Some("put"));
        assert_eq!(
            o.headers,
            vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("x".to_string(), "1".to_string())
            ]
        );
        assert_eq!(o.data, vec![(Data::Ascii, "@$body".to_string())]);
        assert!(o.follow && o.insecure);
        assert_eq!(o.max_time, Some(2.5));
        assert_eq!(o.dump_header.as_deref(), Some("$h"));
        assert_eq!(words(r#"a "b c" 'd\e' f\ g"#), ["a", "b c", "d\\e", "f g"]);
        assert_eq!(base64(b"user:pw"), "dXNlcjpwdw==");
        assert_eq!(encode_url(" http://h/a b?q=é "), "http://h/a%20b?q=%C3%A9");
    }

    #[test]
    fn urls_filemaker_refuses() {
        let none = |_: &str| Value::empty();
        let o = Options::default();
        assert_eq!(
            request("ftp://h/x", &o, true, &none).0.err(),
            Some(PROTOCOL)
        );
        assert_eq!(request("nothing", &o, true, &none).0.err(), Some(BAD_URL));
        assert_eq!(request("http://", &o, true, &none).0.err(), Some(BAD_URL));
        let r = request("https://h/f.pdf", &o, true, &none);
        assert_eq!(r.1.name, "f.pdf");
        assert_eq!(r.0.map(|r| r.method).ok().as_deref(), Some("GET"));
    }
}
