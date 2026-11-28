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

/// The system's language, in English, from the locale variables
/// (`LANG`, `LC_ALL`, `LANGUAGE`); English when none is set.
// ponytail: Windows and macOS keep the region format outside the
// environment; reading it needs OS calls (GetUserDefaultLocaleName,
// CFLocale). English is answered there unless LANG is set.
fn system_language() -> &'static str {
    let code = ["LC_ALL", "LANG", "LANGUAGE"]
        .iter()
        .find_map(|v| env(v))
        .unwrap_or_default()
        .to_lowercase();
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
        if cfg!(windows) {
            assert_eq!(t("Device"), "2");
            assert!(["10.0", "11.0", "6.3"].contains(&t("SystemVersion").as_str()));
            assert!(t("SystemDrive").ends_with(":/"));
        }
    }
}
