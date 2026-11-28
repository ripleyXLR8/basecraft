//! `Get()` flags about the application and the computer: version,
//! languages, OS version, host name, folders. Paths are in FileMaker's
//! format (`/C:/Users/me/Desktop/`, `/Macintosh HD/Users/me/Desktop/`);
//! in the browser (wasm32) there is no file system and they are empty.

use crate::value::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `Get ( ApplicationVersion )`: FileMaker's "Pro 26.0.1" format. BaseCraft
/// reports the FileMaker Pro release it is checked against (the oracle's
/// 26.0.1), so that solutions testing the version ("≥ 19") keep working.
pub const APPLICATION_VERSION: &str = "Pro 26.0.1";

thread_local! {
    static APP_LANGUAGE: std::cell::Cell<&'static str> = const { std::cell::Cell::new("English") };
}

/// `Get ( ApplicationLanguage )`: the interface language, in English
/// ("English", "French"…), set by the host for its thread.
pub fn set_application_language(language: &'static str) {
    APP_LANGUAGE.with(|l| l.set(language));
}

thread_local! {
    /// Page being previewed or printed and the print job's page count.
    static PAGE: std::cell::Cell<(usize, Option<usize>)> = const { std::cell::Cell::new((0, None)) };
}

/// `Get ( PageNumber )` and `Get ( PageCount )` while the host previews
/// (`count`: `None`) or prints / saves as PDF page `number` (1-based) of
/// `count`; `(0, None)` when done. Claris: PageNumber is 0 when nothing is
/// printed or previewed; PageCount is the page count "in the current print
/// job while it's printing", nothing otherwise.
pub fn set_page(number: usize, count: Option<usize>) {
    PAGE.with(|p| p.set((number, count)));
}

pub(super) fn system_get(flag: &str) -> Option<Value> {
    let path = |p: Option<PathBuf>| Value::text(p.map(|p| fm_folder(&p)).unwrap_or_default());
    Some(match flag {
        "ApplicationVersion" => Value::text(APPLICATION_VERSION),
        "ApplicationLanguage" => Value::text(APP_LANGUAGE.with(|l| l.get())),
        "SystemLanguage" => Value::text(system_language()),
        "SystemVersion" => Value::text(system_version()),
        "HostName" => Value::text(host_name()),
        "Device" => Value::int(match std::env::consts::OS {
            "macos" => 1,
            "windows" => 2,
            "ios" => 3,
            "android" => 5,
            "linux" => 6,
            _ => 0,
        }),
        // A local file with network sharing off: one user, no connection.
        "MultiUserState" | "ConnectionState" => Value::int(0),
        "UserCount" => Value::int(1),
        "NetworkProtocol" => Value::text("TCP/IP"),
        "InstalledFMPlugins" => Value::text(""),
        // BaseCraft does not open encrypted (EAR) files.
        "EncryptionState" => Value::int(0),
        "UUIDNumber" => Value::Number(uuid_number()),
        "DesktopPath" => path(home().map(|h| h.join("Desktop"))),
        "DocumentsPath" => path(home().map(|h| h.join("Documents"))),
        "PreferencesPath" => path(preferences()),
        "TemporaryPath" => path(temporary()),
        "FileMakerPath" => path(
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf)),
        ),
        "SystemDrive" => Value::text(system_drive()),
        "PageNumber" => Value::int(PAGE.with(|p| p.get().0) as i64),
        "PageCount" => PAGE
            .with(|p| p.get().1)
            .map_or_else(Value::empty, |n| Value::int(n as i64)),
        _ => return None,
    })
}

/// 24 random bytes as a number (FileMaker: "a unique, 24-byte (192-bit)
/// number"), up to 58 digits.
fn uuid_number() -> crate::num::Num {
    use super::number::random_u64;
    let mut b = [0u8; 24];
    for c in b.chunks_mut(8) {
        c.copy_from_slice(&random_u64().to_le_bytes());
    }
    let n = bigdecimal::num_bigint::BigInt::from_bytes_be(bigdecimal::num_bigint::Sign::Plus, &b);
    crate::num::Num::from(n)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn home() -> Option<PathBuf> {
    env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn preferences() -> Option<PathBuf> {
    if cfg!(windows) {
        env("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Preferences"))
    } else {
        env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))
    }
}

fn temporary() -> Option<PathBuf> {
    // std::env::temp_dir panics where there is no file system.
    if cfg!(target_arch = "wasm32") {
        None
    } else {
        Some(std::env::temp_dir())
    }
}

/// A folder in FileMaker's format, with a trailing slash.
pub fn fm_folder(p: &Path) -> String {
    let mut s = fm_path(p);
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

/// An absolute path in FileMaker's format, without prefix: `/C:/dir/x`
/// on Windows, `/Volume/dir/x` on macOS (the boot volume's name first),
/// the path itself elsewhere.
pub fn fm_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
    if cfg!(windows) {
        match s.strip_prefix("//") {
            // A UNC path: `//server/share/x`.
            Some(_) => s,
            None => format!("/{s}"),
        }
    } else if cfg!(target_os = "macos") && !s.starts_with("/Volumes/") {
        format!("/{}{s}", boot_volume())
    } else if let Some(v) = s.strip_prefix("/Volumes") {
        v.to_string()
    } else {
        s
    }
}

/// The name of the volume mounted at `/` (macOS: the `/Volumes` entry
/// that links to the root).
fn boot_volume() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::fs::read_dir("/Volumes")
            .into_iter()
            .flatten()
            .flatten()
            .find(|e| std::fs::read_link(e.path()).is_ok_and(|t| t == Path::new("/")))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .unwrap_or_else(|| "Macintosh HD".into())
    })
}

fn system_drive() -> String {
    if cfg!(target_arch = "wasm32") {
        String::new()
    } else if cfg!(windows) {
        format!("/{}/", env("SystemDrive").unwrap_or_else(|| "C:".into()))
    } else if cfg!(target_os = "macos") {
        format!("/{}/", boot_volume())
    } else {
        "/".into()
    }
}

/// Output of a command, run once per process.
fn command(program: &str, args: &[&str]) -> Option<String> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// The computer's name (FileMaker: "the current registered computer name"
/// for a file it hosts, as a local file is).
fn host_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        env("COMPUTERNAME")
            .or_else(|| {
                cfg!(target_os = "macos")
                    .then(|| command("scutil", &["--get", "ComputerName"]))
                    .flatten()
            })
            .or_else(|| env("HOSTNAME"))
            .or_else(|| {
                std::fs::read_to_string("/etc/hostname")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| command("hostname", &[]))
            .unwrap_or_default()
    })
    .clone()
}

/// FileMaker's `Get ( SystemVersion )`: "10.0" for Windows 10, "11.0" for
/// Windows 11, the macOS version ("26.0"); the kernel release on Linux.
fn system_version() -> String {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        if cfg!(windows) {
            // `ver`: "Microsoft Windows [Version 10.0.26100.1234]".
            let v = command("cmd", &["/C", "ver"]).unwrap_or_default();
            let digits: String = v
                .rsplit(' ')
                .next()
                .unwrap_or("")
                .trim_end_matches(']')
                .to_string();
            let mut parts = digits.split('.');
            let major = parts.next().unwrap_or("");
            let minor = parts.next().unwrap_or("0");
            let build: u32 = parts.next().and_then(|b| b.parse().ok()).unwrap_or(0);
            match (major, build) {
                ("", _) => String::new(),
                // Windows 11 still says 10.0; its builds start at 22000.
                ("10", b) if b >= 22_000 => "11.0".into(),
                (m, _) => format!("{m}.{minor}"),
            }
        } else if cfg!(target_os = "macos") {
            let v = command("sw_vers", &["-productVersion"]).unwrap_or_default();
            let mut p = v.split('.');
            match (p.next(), p.next()) {
                (Some(a), b) if !a.is_empty() => format!("{a}.{}", b.unwrap_or("0")),
                _ => String::new(),
            }
        } else {
            std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        }
    })
    .clone()
}

/// The system's language, in English: the locale variables (`LC_ALL`,
/// `LANG`, `LANGUAGE`) first, then the OS's preferred language (Windows'
/// user locale, macOS's preferred languages, through `sys-locale`);
/// English when neither is known (and in the browser).
fn system_language() -> &'static str {
    let code = ["LC_ALL", "LANG", "LANGUAGE"]
        .iter()
        .find_map(|v| env(v))
        .or_else(os_locale)
        .unwrap_or_default()
        .to_lowercase();
    language_name(&code)
}

#[cfg(not(target_arch = "wasm32"))]
fn os_locale() -> Option<String> {
    sys_locale::get_locale()
}

#[cfg(target_arch = "wasm32")]
fn os_locale() -> Option<String> {
    None
}

/// Seconds to add to UTC for the computer's local time now (FileMaker's
/// clock is local). Native: Windows' `ActiveTimeBias` (daylight saving
/// included) or `date +%z`, read again after a minute so that a change of
/// daylight saving time is seen; browser: JavaScript's `Date`. 0 if unknown.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn utc_offset_secs() -> i64 {
    use std::sync::Mutex;
    use web_time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, i64)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, off)) = *c
        && at.elapsed() < Duration::from_secs(60)
    {
        return off;
    }
    let off = if cfg!(windows) {
        command(
            "reg",
            &[
                "query",
                r"HKLM\SYSTEM\CurrentControlSet\Control\TimeZoneInformation",
                "/v",
                "ActiveTimeBias",
            ],
        )
        .and_then(|o| parse_bias(&o))
    } else {
        command("date", &["+%z"]).and_then(|o| parse_z(&o))
    }
    .unwrap_or(0);
    *c = Some((Instant::now(), off));
    off
}

#[cfg(target_arch = "wasm32")]
pub(super) fn utc_offset_secs() -> i64 {
    // getTimezoneOffset: minutes from local time to UTC.
    -(js_sys::Date::new_0().get_timezone_offset() * 60.0) as i64
}

/// `reg query … /v ActiveTimeBias`: "ActiveTimeBias    REG_DWORD
/// 0xffffffc4", minutes from local time to UTC (−60 for UTC+1).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn parse_bias(out: &str) -> Option<i64> {
    let line = out.lines().find(|l| l.contains("ActiveTimeBias"))?;
    let hex = line.split_whitespace().last()?.strip_prefix("0x")?;
    let bias = u32::from_str_radix(hex, 16).ok()? as i32;
    Some(-(bias as i64) * 60)
}

/// `date +%z`: "+0100", "-0530".
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn parse_z(out: &str) -> Option<i64> {
    let (sign, d) = match out.trim().split_at_checked(1)? {
        ("+", d) => (1, d),
        ("-", d) => (-1, d),
        _ => return None,
    };
    if d.len() != 4 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (i64, i64) = (d[..2].parse().ok()?, d[2..].parse().ok()?);
    Some(sign * (h * 3600 + m * 60))
}

/// English name of the language of a locale code ("fr-FR", "de_DE.UTF-8").
fn language_name(code: &str) -> &'static str {
    let lang = code.get(..2).unwrap_or("");
    match lang {
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "es" => "Spanish",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        "pt" => "Portuguese",
        "da" => "Danish",
        "fi" => "Finnish",
        "nb" | "no" => "Norwegian",
        "pl" => "Polish",
        "ru" => "Russian",
        "cs" => "Czech",
        "tr" => "Turkish",
        "el" => "Greek",
        "hu" => "Hungarian",
        _ => "English",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fm_paths() {
        if cfg!(windows) {
            assert_eq!(
                fm_folder(Path::new(r"C:\Users\me\Desktop")),
                "/C:/Users/me/Desktop/"
            );
            assert_eq!(
                fm_path(Path::new(r"\\srv\share\x.fmp12")),
                "//srv/share/x.fmp12"
            );
        } else if !cfg!(target_os = "macos") {
            assert_eq!(
                fm_folder(Path::new("/home/me/Desktop")),
                "/home/me/Desktop/"
            );
        }
    }

    #[test]
    fn utc_offsets() {
        let reg = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\…\r\n    ActiveTimeBias    REG_DWORD    0xffffffc4\r\n";
        assert_eq!(parse_bias(reg), Some(3600));
        assert_eq!(parse_bias("ActiveTimeBias REG_DWORD 0x12c"), Some(-18_000));
        assert_eq!(parse_z("+0100\n"), Some(3600));
        assert_eq!(parse_z("-0530"), Some(-19_800));
        assert_eq!(parse_z("UTC"), None);
        // The computer's offset: whole quarter hours within ±14 h.
        let off = utc_offset_secs();
        assert!(off % 900 == 0 && off.abs() <= 14 * 3600, "{off}");
        // Get ( CurrentTimestamp ) is local, the UTC milliseconds are not.
        let get = |f: &str| match super::super::builtin_get(f) {
            Some(Value::Timestamp(n) | Value::Number(n)) => n.to_string().parse::<i64>().unwrap(),
            v => panic!("{v:?}"),
        };
        let utc_ms = get("CurrentTimeUTCMilliseconds");
        let local = get("CurrentTimestamp");
        // Both count from 0001-01-01.
        let diff = local - utc_ms / 1000 - off;
        assert!(diff.abs() <= 2, "{diff}");
    }

    #[test]
    fn system_flags() {
        let t = |f: &str| system_get(f).map(|v| v.to_text()).unwrap_or_default();
        assert_eq!(t("ApplicationVersion"), "Pro 26.0.1");
        assert_eq!(t("ApplicationLanguage"), "English");
        set_application_language("French");
        assert_eq!(t("ApplicationLanguage"), "French");
        set_application_language("English");
        assert_eq!(t("UserCount"), "1");
        assert_eq!(t("MultiUserState"), "0");
        let n = t("UUIDNumber");
        assert!(n.len() > 40 && n.chars().all(|c| c.is_ascii_digit()), "{n}");
        assert_ne!(n, t("UUIDNumber"));
        assert!(t("DesktopPath").ends_with("/Desktop/"));
        assert!(t("TemporaryPath").starts_with('/') && t("TemporaryPath").ends_with('/'));
        assert_eq!(language_name("fr-fr"), "French");
        assert_eq!(language_name("de_de.utf-8"), "German");
        assert_eq!(language_name(""), "English");
        if cfg!(windows) {
            assert_eq!(t("Device"), "2");
            assert!(["10.0", "11.0", "6.3"].contains(&t("SystemVersion").as_str()));
            assert!(t("SystemDrive").ends_with(":/"));
        }
    }
}
