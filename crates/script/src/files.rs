//! FileMaker paths (`file:`, `filewin:`, `filemac:`, `/C:/dir/x.txt`)
//! and a [`Disk`] that host implementations use for the data file steps.
//! Relative paths are relative to the database file's folder; a write
//! outside that folder answers [`CONFIRM`] until the user agreed.

use std::path::{Component, Path, PathBuf};

/// Error code a host returns when a write leaves the file's folder and
/// the user has not confirmed it yet: the engine asks
/// ([`crate::UiRequest::ConfirmFile`]) and runs the step again.
pub const CONFIRM: u32 = u32::MAX;
/// FileMaker's "File is missing".
pub const FILE_MISSING: u32 = 100;
/// FileMaker's "File cannot be created on disk" / unable to write.
pub const FILE_WRITE: u32 = 800;

/// The platform path of a FileMaker path, and whether it stays inside
/// `folder`. `None` for a path of the other platform or an empty one.
/// Only the first line of a multi-line path list is used by callers
/// that write; readers try each line ([`candidates`]).
pub fn resolve(fm: &str, folder: Option<&Path>) -> Option<(PathBuf, bool)> {
    let fm = fm.trim();
    let rest = if let Some(r) = fm.strip_prefix("filewin:") {
        if !cfg!(windows) {
            return None;
        }
        r
    } else if let Some(r) = fm.strip_prefix("filemac:") {
        if cfg!(windows) {
            return None;
        }
        r
    } else {
        fm.strip_prefix("file:").unwrap_or(fm)
    };
    if rest.is_empty() {
        return None;
    }
    let p = if let Some(abs) = rest.strip_prefix('/') {
        if let Some(unc) = abs.strip_prefix('/') {
            PathBuf::from(format!("\\\\{}", unc.replace('/', "\\")))
        } else if cfg!(windows) || abs.get(1..2) == Some(":") {
            // `/C:/dir/x` → `C:/dir/x`.
            PathBuf::from(abs)
        } else {
            PathBuf::from(format!("/{abs}"))
        }
    } else {
        folder?.join(rest)
    };
    let p = normalize(&p);
    let inside = folder.is_some_and(|f| p.starts_with(normalize(f)));
    Some((p, inside))
}

/// A stored path list (parameter key 130, FORMAT.md §8): u32 count,
/// then per path a u8-prefixed kind (`file`, empty for a variable such as
/// `$path`) and a u16-prefixed path; the strings are XOR 0x5A. Each path
/// as FileMaker shows it (`file:out.txt`, `$path`).
pub fn decode_path_list(b: &[u8]) -> Vec<String> {
    let s = |x: &[u8]| {
        String::from_utf8_lossy(&x.iter().map(|c| c ^ 0x5A).collect::<Vec<_>>()).into_owned()
    };
    let mut out = Vec::new();
    let n = b
        .get(..4)
        .map_or(0, |x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let mut i = 4;
    for _ in 0..n {
        let Some(&k) = b.get(i) else { break };
        let Some(kind) = b.get(i + 1..i + 1 + k as usize) else {
            break;
        };
        i += 1 + k as usize;
        let Some(m) = b.get(i..i + 2) else { break };
        let m = u16::from_be_bytes([m[0], m[1]]) as usize;
        let Some(path) = b.get(i + 2..i + 2 + m) else {
            break;
        };
        i += 2 + m;
        out.push(if kind.is_empty() {
            s(path)
        } else {
            format!("{}:{}", s(kind), s(path))
        });
    }
    out
}

/// The inverse of [`decode_path_list`]: `file:a.txt` is kind `file`,
/// path `a.txt`; a path without a known prefix has no kind.
pub fn encode_path_list(paths: &[&str]) -> Vec<u8> {
    let x = |v: &str| v.bytes().map(|c| c ^ 0x5A).collect::<Vec<u8>>();
    let mut b = (paths.len() as u32).to_be_bytes().to_vec();
    for p in paths {
        let (kind, path) = ["file", "filewin", "filemac", "image", "imagewin", "imagemac"]
            .iter()
            .find_map(|k| p.strip_prefix(k)?.strip_prefix(':').map(|r| (*k, r)))
            .unwrap_or(("", p));
        b.push(kind.len() as u8);
        b.extend(x(kind));
        b.extend((path.len() as u16).to_be_bytes());
        b.extend(x(path));
    }
    b
}

/// Every non-empty line of a FileMaker path list.
pub fn candidates(list: &str) -> impl Iterator<Item = &str> {
    list.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty())
}

/// Removes `.` and `..` without touching the disk.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// The local file system, seen from a database file in `folder` (`None`:
/// a file that was never saved; only absolute paths work then).
/// `confirmed` holds the paths the user agreed to write outside it.
#[derive(Debug, Default, Clone)]
pub struct Disk {
    pub folder: Option<PathBuf>,
    pub confirmed: Vec<PathBuf>,
}

impl Disk {
    fn path(&self, fm: &str) -> Result<(PathBuf, bool), u32> {
        resolve(fm, self.folder.as_deref()).ok_or(FILE_MISSING)
    }

    /// A path the script may write: inside the folder or confirmed.
    fn writable(&self, fm: &str) -> Result<PathBuf, u32> {
        let (p, inside) = self.path(fm)?;
        if inside || self.confirmed.contains(&p) {
            Ok(p)
        } else {
            Err(CONFIRM)
        }
    }

    /// Records the user's agreement to write `fm`.
    pub fn confirm(&mut self, fm: &str) {
        if let Ok((p, _)) = self.path(fm) {
            self.confirmed.push(p);
        }
    }

    pub fn stat(&self, fm: &str) -> Result<Option<crate::FileStat>, u32> {
        let (p, _) = self.path(fm)?;
        Ok(std::fs::metadata(&p).ok().map(|m| crate::FileStat {
            folder: m.is_dir(),
            size: m.len(),
        }))
    }

    pub fn read(&self, fm: &str) -> Result<Vec<u8>, u32> {
        let (p, _) = self.path(fm)?;
        std::fs::read(p).map_err(|_| FILE_MISSING)
    }

    pub fn write(&self, fm: &str, data: &[u8], folders: bool) -> Result<(), u32> {
        let p = self.writable(fm)?;
        if folders && let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|_| FILE_WRITE)?;
        }
        std::fs::write(p, data).map_err(|_| FILE_WRITE)
    }

    pub fn delete(&self, fm: &str) -> Result<(), u32> {
        let p = self.writable(fm)?;
        if !p.is_file() {
            return Err(FILE_MISSING);
        }
        std::fs::remove_file(p).map_err(|_| FILE_WRITE)
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), u32> {
        let a = self.writable(from)?;
        let b = self.writable(to)?;
        if !a.exists() {
            return Err(FILE_MISSING);
        }
        std::fs::rename(a, b).map_err(|_| FILE_WRITE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let dir = Path::new("/db");
        let r = |s| resolve(s, Some(dir));
        assert_eq!(r("file:a.txt"), Some((PathBuf::from("/db/a.txt"), true)));
        assert_eq!(r("a.txt"), Some((PathBuf::from("/db/a.txt"), true)));
        assert_eq!(
            r("file:sub/../b.txt"),
            Some((PathBuf::from("/db/b.txt"), true))
        );
        assert_eq!(r("file:../x.txt").map(|x| x.1), Some(false));
        assert_eq!(r("file:"), None);
        assert_eq!(resolve("file:a.txt", None), None);
        if cfg!(windows) {
            assert_eq!(r("filemac:/HD/x"), None);
            let (p, inside) = r("filewin:/C:/tmp/x.txt").unwrap();
            assert_eq!((p, inside), (PathBuf::from("C:/tmp/x.txt"), false));
            let (p, _) = r("/C:/tmp/x.txt").unwrap();
            assert_eq!(p, PathBuf::from("C:/tmp/x.txt"));
        } else {
            assert_eq!(r("filewin:/C:/x"), None);
            assert_eq!(r("file:/tmp/x").unwrap(), (PathBuf::from("/tmp/x"), false));
        }
        // Bytes FileMaker 26 wrote for `file:a.txt` and `$p`.
        let a = [0, 0, 0, 1, 4, 0x3c, 0x33, 0x36, 0x3f, 0, 5, 0x3b, 0x74, 0x2e, 0x22, 0x2e];
        assert_eq!(decode_path_list(&a), ["file:a.txt"]);
        assert_eq!(encode_path_list(&["file:a.txt"]), a);
        let v = [0, 0, 0, 1, 0, 0, 2, 0x7e, 0x2a];
        assert_eq!(decode_path_list(&v), ["$p"]);
        assert_eq!(encode_path_list(&["$p"]), v);
        assert_eq!(
            candidates("\r file:a \n\nfile:b").collect::<Vec<_>>(),
            ["file:a", "file:b"]
        );
    }
}
