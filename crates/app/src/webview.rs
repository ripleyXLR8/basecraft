//! Web viewers in Browse mode: a native child webview (wry: WebView2 on
//! Windows, WKWebView on macOS) over the object's rectangle in the main
//! window. Elsewhere (Linux, the other OS windows, the browser, the UI
//! tests) the placeholder of `objects.rs` shows.
//!
//! Each frame `objects::web_viewer` asks for the web viewers it drew
//! ([`want`]); [`sync`] then creates, moves, hides and destroys the
//! webviews, applies the scripts' actions ([`command`]) and reads the
//! pages' messages: `FileMaker.PerformScript` calls ([`take_call`]) and
//! each page's address and source ([`pages`]).
//!
//! Pages are untrusted: they get the webview's own sandbox and, only when
//! the web viewer allows JavaScript to perform scripts, `window.FileMaker`
//! (PerformScript, PerformScriptWithOption). No other app state reaches
//! them, new windows are refused.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use eframe::egui;
use fmp12::layouts::web;
use script::WebAction;

/// A web viewer drawn: layout id, object id, row offset (list view).
pub type Key = (u32, u64, i64);

/// One web viewer drawn this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub key: Key,
    pub name: String,
    /// The address calculation's result, as loaded ([`resolve`]).
    pub url: String,
    /// Web Viewer Setup options ([`fmp12::layouts::web`]).
    pub flags: u64,
    /// Visible part in physical pixels of the window (x, y, width,
    /// height); `None` when scrolled away or covered by a menu.
    pub rect: Option<[i32; 4]>,
}

/// What a webview is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Nav {
    Load(String),
    Reload,
    Back,
    Forward,
    Eval(String),
    FlushCookies,
}

/// A live webview (wry's, or a test's fake).
pub trait View {
    /// Shows it at `rect` (physical pixels), or hides it.
    fn place(&mut self, rect: Option<[i32; 4]>);
    fn navigate(&mut self, nav: Nav);
}

/// A page's message (`window.ipc.postMessage`) sent by the bridge script.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `FileMaker.PerformScript[WithOption] ( name, parameter, option )`.
    Script {
        name: String,
        parameter: String,
        option: u8,
    },
    /// The page loaded: its address and HTML source.
    Page { url: String, html: String },
}

type Inbox = Rc<RefCell<Vec<(Key, String)>>>;

struct Live {
    want: Want,
    view: Box<dyn View>,
}

#[derive(Default)]
struct State {
    wants: Vec<Want>,
    /// Script actions: (layout id, object name, action).
    commands: Vec<(u32, String, WebAction)>,
    live: Vec<Live>,
    inbox: Inbox,
    calls: std::collections::VecDeque<(String, String)>,
    pages: BTreeMap<String, (String, String)>,
    #[cfg(any(windows, target_os = "macos"))]
    context: Option<wry::WebContext>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Live webviews on this platform.
const NATIVE: bool = cfg!(any(windows, target_os = "macos"));

/// A web viewer drawn this frame.
pub fn want(w: Want) {
    STATE.with_borrow_mut(|s| {
        s.wants.retain(|x| x.key != w.key);
        s.wants.push(w);
    });
}

/// A script's action on web viewer `name` of `layout` (`Host::web_viewer`),
/// applied at the next frame. Perform JavaScript answers 3 where there
/// are no webviews.
pub fn command(layout: u32, name: &str, a: WebAction) -> u32 {
    if !NATIVE && matches!(a, WebAction::Js { .. }) {
        return script::error::UNAVAILABLE;
    }
    STATE.with_borrow_mut(|s| s.commands.push((layout, name.into(), a)));
    0
}

/// The next `FileMaker.PerformScript` call: (script name, parameter).
pub fn take_call() -> Option<(String, String)> {
    STATE.with_borrow_mut(|s| s.calls.pop_front())
}

/// The pages shown, by web viewer name: (address, HTML source).
pub fn pages() -> BTreeMap<String, (String, String)> {
    STATE.with_borrow(|s| s.pages.clone())
}

/// Brings the webviews in line with the web viewers drawn this frame in
/// the main window (`frame`'s). Menus and modal dialogs hide them: a
/// native webview would cover them.
pub fn sync(ctx: &egui::Context, frame: &eframe::Frame) {
    let hide = egui::Popup::is_any_open(ctx) || ctx.memory(|m| m.top_modal_layer().is_some());
    STATE.with_borrow_mut(|s| {
        let inbox = s.inbox.clone();
        #[cfg(any(windows, target_os = "macos"))]
        {
            let mut wc = s.context.take().unwrap_or_else(|| {
                wry::WebContext::new(crate::prefs::dir().map(|d| d.join("WebView")))
            });
            s.update(hide, &mut |w| {
                native::create(frame, &mut wc, w, inbox.clone(), ctx.clone())
            });
            s.context = Some(wc);
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (frame, inbox);
            s.update(hide, &mut |_| None);
        }
    });
}

impl State {
    fn update(&mut self, hide: bool, make: &mut dyn FnMut(&Want) -> Option<Box<dyn View>>) {
        let wants = std::mem::take(&mut self.wants);
        // Not drawn this frame (another layout or mode, hidden, closed): gone.
        let pages = &mut self.pages;
        self.live.retain(|l| {
            let keep = wants.iter().any(|w| w.key == l.want.key);
            if !keep {
                pages.remove(&l.want.name);
            }
            keep
        });
        for w in wants {
            let w = Want {
                rect: w.rect.filter(|_| !hide),
                ..w
            };
            match self.live.iter_mut().find(|l| l.want.key == w.key) {
                Some(l) => {
                    if l.want.url != w.url {
                        l.view.navigate(Nav::Load(w.url.clone()));
                    }
                    if l.want.rect != w.rect {
                        l.view.place(w.rect);
                    }
                    l.want = w;
                }
                None => {
                    if let Some(mut view) = make(&w) {
                        view.place(w.rect);
                        self.live.push(Live { want: w, view });
                    }
                }
            }
        }
        for (layout, name, a) in std::mem::take(&mut self.commands) {
            let flush = a == WebAction::FlushCookies;
            // Cookies are shared by all web viewers: one flush is enough.
            let mut on = self
                .live
                .iter_mut()
                .filter(|l| flush || l.want.key.0 == layout && l.want.name == name)
                .take(if flush { 1 } else { usize::MAX });
            for l in &mut on {
                if let Some(n) = nav(&a, &l.want) {
                    l.view.navigate(n);
                }
            }
        }
        let inbox = std::mem::take(&mut *self.inbox.borrow_mut());
        for (key, msg) in inbox {
            let Some(l) = self.live.iter().find(|l| l.want.key == key) else {
                continue;
            };
            match parse(&msg) {
                Some(Message::Script {
                    name, parameter, ..
                }) if l.want.flags & web::JAVASCRIPT != 0 => {
                    self.calls.push_back((name, parameter));
                }
                Some(Message::Page { url, html }) if !l.want.name.is_empty() => {
                    self.pages.insert(l.want.name.clone(), (url, html));
                }
                _ => {}
            }
        }
    }
}

/// The webview's part of a script action.
fn nav(a: &WebAction, w: &Want) -> Option<Nav> {
    Some(match a {
        WebAction::Reset => Nav::Load(w.url.clone()),
        WebAction::Reload => Nav::Reload,
        WebAction::Forward => Nav::Forward,
        WebAction::Back => Nav::Back,
        WebAction::Go(u) => Nav::Load(resolve(u, w.flags & web::ENCODE_URL != 0)),
        WebAction::Js {
            function,
            parameters,
        } => Nav::Eval(js_call(function, parameters)?),
        WebAction::FlushCookies => Nav::FlushCookies,
    })
}

/// The address a web viewer loads: `about:blank` when empty, `http://`
/// before an address without a scheme, and with "Automatically encode
/// URL" the characters a URL cannot hold percent-encoded (not in `data:`
/// URLs, whose content is the page). 🔸 FileMaker's exact rules were not
/// observed.
pub fn resolve(url: &str, encode: bool) -> String {
    let url = url.trim();
    if url.is_empty() {
        return "about:blank".into();
    }
    let scheme = url.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    let url = if scheme {
        url.to_string()
    } else {
        format!("http://{url}")
    };
    if !encode
        || url
            .get(..5)
            .is_some_and(|p| p.eq_ignore_ascii_case("data:"))
    {
        return url;
    }
    let mut out = String::new();
    for b in url.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A JavaScript string literal holding `s`.
pub fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' || c == '<' => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Perform JavaScript in Web Viewer: `function("p1", "p2"…);`, the
/// parameters as strings as FileMaker passes them. `None` unless the
/// function is a name or a dotted path (`app.show`): nothing else runs.
pub fn js_call(function: &str, parameters: &[String]) -> Option<String> {
    let ident = |p: &str| {
        p.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    };
    if !function.split('.').all(ident) {
        return None;
    }
    let args: Vec<String> = parameters.iter().map(|p| js_string(p)).collect();
    Some(format!("{function}({});", args.join(", ")))
}

/// Reads a bridge message: `s|option|name|parameter` or `c|url|html`, the
/// texts as `encodeURIComponent` writes them. `None` for anything else.
pub fn parse(msg: &str) -> Option<Message> {
    let mut it = msg.split('|');
    let m = match it.next()? {
        "s" => {
            let option = it.next()?.parse::<u8>().ok().filter(|o| *o <= 5)?;
            Message::Script {
                name: unescape(it.next()?)?,
                parameter: unescape(it.next()?)?,
                option,
            }
        }
        "c" => Message::Page {
            url: unescape(it.next()?)?,
            html: unescape(it.next()?)?,
        },
        _ => return None,
    };
    it.next().is_none().then_some(m)
}

/// Undoes `encodeURIComponent`.
fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The script run in each page before its own: reports the page's
/// source once loaded, adds `window.FileMaker` when the web viewer lets
/// JavaScript perform scripts, and swallows input when interaction is off.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub fn bridge(flags: u64) -> String {
    let mut js = String::from(
        "(function () {\n\
         var e = encodeURIComponent;\n\
         window.addEventListener('load', function () {\n\
         window.ipc.postMessage('c|' + e(location.href) + '|' + e(document.documentElement.outerHTML));\n\
         });\n",
    );
    if flags & web::JAVASCRIPT != 0 {
        js.push_str(
            "var call = function (n, p, o) {\n\
             window.ipc.postMessage('s|' + (Number(o) | 0) + '|' + e(String(n)) + '|' + e(p == null ? '' : String(p)));\n\
             };\n\
             Object.defineProperty(window, 'FileMaker', { value: Object.freeze({\n\
             PerformScript: function (n, p) { call(n, p, 0); },\n\
             PerformScriptWithOption: function (n, p, o) { call(n, p, o); }\n\
             }) });\n",
        );
    }
    if flags & web::INTERACTION == 0 {
        js.push_str(
            "['mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'wheel', 'touchstart', 'keydown', 'keypress', 'keyup'].forEach(function (t) {\n\
             window.addEventListener(t, function (ev) { ev.preventDefault(); ev.stopImmediatePropagation(); }, { capture: true, passive: false });\n\
             });\n",
        );
    }
    js.push_str("})();\n");
    js
}

#[cfg(any(windows, target_os = "macos"))]
mod native {
    use super::*;
    use wry::raw_window_handle::HasWindowHandle;

    struct Wry(wry::WebView);

    fn bounds([x, y, w, h]: [i32; 4]) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::PhysicalPosition::new(x, y).into(),
            size: wry::dpi::PhysicalSize::new(w.max(1) as u32, h.max(1) as u32).into(),
        }
    }

    impl View for Wry {
        fn place(&mut self, rect: Option<[i32; 4]>) {
            if let Some(r) = rect {
                let _ = self.0.set_bounds(bounds(r));
            }
            let _ = self.0.set_visible(rect.is_some());
        }
        fn navigate(&mut self, nav: Nav) {
            let v = &self.0;
            let _ = match nav {
                Nav::Load(u) => v.load_url(&u),
                Nav::Reload => v.reload(),
                Nav::Back => v.go_back(),
                Nav::Forward => v.go_forward(),
                Nav::Eval(js) => v.evaluate_script(&js),
                Nav::FlushCookies => v.cookies().map(|cs| {
                    for c in cs {
                        let _ = v.delete_cookie(&c);
                    }
                }),
            };
        }
    }

    /// A child webview of the main window; `None` when the window has no
    /// native handle (the UI tests) or the engine is missing (no WebView2
    /// runtime).
    pub fn create(
        parent: &impl HasWindowHandle,
        wc: &mut wry::WebContext,
        w: &Want,
        inbox: Inbox,
        ctx: egui::Context,
    ) -> Option<Box<dyn View>> {
        let key = w.key;
        wry::WebViewBuilder::new_with_web_context(wc)
            .with_url(&w.url)
            .with_initialization_script_for_main_only(bridge(w.flags), true)
            .with_ipc_handler(move |r| {
                inbox.borrow_mut().push((key, r.body().clone()));
                ctx.request_repaint();
            })
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_bounds(bounds(w.rect.unwrap_or([0, 0, 1, 1])))
            .with_visible(w.rect.is_some())
            .build_as_child(parent)
            .ok()
            .map(|v| Box::new(Wry(v)) as Box<dyn View>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A webview that records what it is asked.
    struct Fake(Rc<RefCell<Vec<String>>>);
    impl View for Fake {
        fn place(&mut self, rect: Option<[i32; 4]>) {
            self.0.borrow_mut().push(format!("place {rect:?}"));
        }
        fn navigate(&mut self, nav: Nav) {
            self.0.borrow_mut().push(format!("{nav:?}"));
        }
    }

    fn w(key: i64, url: &str, flags: u64, rect: Option<[i32; 4]>) -> Want {
        Want {
            key: (1, 5, key),
            name: "web".into(),
            url: url.into(),
            flags,
            rect,
        }
    }

    #[test]
    fn urls() {
        assert_eq!(resolve("  ", true), "about:blank");
        assert_eq!(resolve("example.com/a b", false), "http://example.com/a b");
        assert_eq!(
            resolve("https://x.org/é b?q=1&r=%20", true),
            "https://x.org/%C3%A9%20b?q=1&r=%20"
        );
        assert_eq!(
            resolve("data:text/html,<b>a b</b>", true),
            "data:text/html,<b>a b</b>"
        );
        assert_eq!(resolve("about:blank", true), "about:blank");
    }

    #[test]
    fn javascript_calls() {
        let p = ["a\"b\\c\n".to_string(), "</script>\u{2028}".into()];
        // `\` + `u003C`, `\` + `u2028`.
        let u = |hex: &str| format!("{}u{hex}", '\\');
        let want = format!(
            r#"app.show("a\"b\\c\n", "{}/script>{}");"#,
            u("003C"),
            u("2028")
        );
        assert_eq!(js_call("app.show", &p), Some(want));
        assert_eq!(js_call("f", &[]).as_deref(), Some("f();"));
        for bad in ["", "alert(1);x", "a..b", "1a", "a b", "a[0]"] {
            assert_eq!(js_call(bad, &[]), None, "{bad}");
        }
    }

    #[test]
    fn bridge_messages() {
        assert_eq!(
            parse("s|0|My%20script|a%7Cb%C3%A9"),
            Some(Message::Script {
                name: "My script".into(),
                parameter: "a|bé".into(),
                option: 0
            })
        );
        assert_eq!(
            parse("c|about%3Ablank|%3Chtml%3E"),
            Some(Message::Page {
                url: "about:blank".into(),
                html: "<html>".into()
            })
        );
        for bad in [
            "",
            "s|9|a|b",
            "s|0|a",
            "s|0|a|b|c",
            "s|x|a|b",
            "s|0|%G0|b",
            "s|0|%C3|b",
            "x|0|a|b",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        let js = bridge(web::INTERACTION | web::JAVASCRIPT);
        assert!(js.contains("FileMaker") && !js.contains("preventDefault"));
        let js = bridge(0);
        assert!(!js.contains("FileMaker") && js.contains("preventDefault"));
    }

    /// Webviews follow the web viewers drawn: created, moved, hidden,
    /// reloaded when the address changes, destroyed when no longer drawn;
    /// script actions and page messages reach the right one.
    #[test]
    fn webviews_follow_the_layout() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut s = State::default();
        let mut made = 0;
        let l = log.clone();
        let mut make = move |_: &Want| -> Option<Box<dyn View>> {
            made += 1;
            l.borrow_mut().push(format!("make {made}"));
            Some(Box::new(Fake(l.clone())))
        };
        let r = Some([10, 20, 300, 200]);
        s.wants.push(w(0, "about:blank", web::JAVASCRIPT, r));
        s.update(false, &mut make);
        s.wants.push(w(0, "about:blank", web::JAVASCRIPT, r));
        s.update(false, &mut make);
        // Moved, then a menu opens, then a new address.
        s.wants
            .push(w(0, "about:blank", web::JAVASCRIPT, Some([0, 0, 300, 200])));
        s.update(true, &mut make);
        s.wants.push(w(0, "http://x/", web::JAVASCRIPT, r));
        s.commands
            .push((1, "web".into(), WebAction::Go("y.org".into())));
        s.commands.push((2, "web".into(), WebAction::Reload));
        s.commands.push((
            1,
            "web".into(),
            WebAction::Js {
                function: "f".into(),
                parameters: vec!["1".into()],
            },
        ));
        s.inbox
            .borrow_mut()
            .extend([((1, 5, 0), "s|0|Go|p".into()), ((1, 5, 0), "c|u|h".into())]);
        s.update(false, &mut make);
        assert_eq!(s.calls.pop_front(), Some(("Go".into(), "p".into())));
        assert_eq!(pages_of(&s), Some(("u".into(), "h".into())));
        // Gone from the layout: destroyed, its page forgotten.
        s.update(false, &mut make);
        assert!(s.live.is_empty() && s.pages.is_empty());
        // Scripts not allowed: the call is dropped.
        s.wants.push(w(0, "about:blank", 0, r));
        s.inbox.borrow_mut().push(((1, 5, 0), "s|0|Go|p".into()));
        s.update(false, &mut make);
        assert!(s.calls.is_empty());
        assert_eq!(
            log.borrow().join("; "),
            "make 1; place Some([10, 20, 300, 200]); place None; \
             Load(\"http://x/\"); place Some([10, 20, 300, 200]); \
             Load(\"http://y.org\"); Eval(\"f(\\\"1\\\");\"); \
             make 2; place Some([10, 20, 300, 200])"
        );
    }

    fn pages_of(s: &State) -> Option<(String, String)> {
        s.pages.get("web").cloned()
    }
}
