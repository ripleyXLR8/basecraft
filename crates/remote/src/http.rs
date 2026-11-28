//! One plain HTTP(S) request, for the Insert from URL script step: the
//! same ureq agent and rustls TLS as the Data API client, configured per
//! request (certificate checks, redirects, timeouts).

use std::time::Duration;

/// A request as a script step builds it from its URL and cURL options.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// `None`: no body (not even `Content-Length: 0`).
    pub body: Option<Vec<u8>>,
    /// Check the server's certificate (Verify SSL Certificates, no `-k`).
    pub verify: bool,
    /// Follow redirects (`--location`).
    pub follow: bool,
    pub connect_timeout: Option<Duration>,
    /// The whole exchange (`--max-time`).
    pub max_time: Option<Duration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// `HTTP/1.1`.
    pub version: String,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// The first header named `name` (any case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The status line and headers as curl's `--dump-header` writes them.
    pub fn header_text(&self) -> String {
        let mut s = format!("{} {} {}\r\n", self.version, self.status, self.reason);
        for (k, v) in &self.headers {
            s.push_str(&format!("{k}: {v}\r\n"));
        }
        s.push_str("\r\n");
        s
    }
}

/// Why a request failed before a response came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The URL cannot be parsed or names no host.
    Url(String),
    /// Unknown host, refused or reset connection.
    Connect(String),
    Timeout(String),
    /// Certificate or other TLS failure.
    Tls(String),
    Other(String),
}

/// Sends `r` and reads the whole response (any status).
pub fn send(r: &Request) -> Result<Response, Failure> {
    let tls = ureq::tls::TlsConfig::builder()
        .disable_verification(!r.verify)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(if r.follow { 50 } else { 0 })
        .allow_non_standard_methods(true)
        .timeout_connect(r.connect_timeout)
        .timeout_global(r.max_time)
        .user_agent("BaseCraft")
        .tls_config(tls)
        .build()
        .into();
    let mut b = ureq::http::Request::builder()
        .method(r.method.as_str())
        .uri(r.url.as_str());
    for (k, v) in &r.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let url = |e: ureq::http::Error| Failure::Url(e.to_string());
    let res = match &r.body {
        Some(body) => agent.run(b.body(body.clone()).map_err(url)?),
        None => agent.run(b.body(()).map_err(url)?),
    };
    let mut resp = res.map_err(failure)?;
    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let version = format!("{:?}", resp.version());
    let body = resp
        .body_mut()
        .with_config()
        .limit(1024 * 1024 * 1024)
        .read_to_vec()
        .map_err(failure)?;
    Ok(Response {
        status: status.as_u16(),
        version,
        reason: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
    })
}

fn failure(e: ureq::Error) -> Failure {
    let m = e.to_string();
    match e {
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => Failure::Url(m),
        ureq::Error::Timeout(_) => Failure::Timeout(m),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            Failure::Connect(m)
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => Failure::Tls(m),
        _ => Failure::Other(m),
    }
}
