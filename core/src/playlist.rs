//! m3u8 read and write.
//!
//! Plain absolute paths on write: rekordbox strips a `file://` scheme but does
//! not percent-decode. A `#` anywhere in a path starts a comment for
//! rekordbox's parser, so those tracks fail to import; they are reported.

use std::path::Path;

use crate::Res;

/// entries: (relative path, display title). Returns paths likely to fail.
pub fn write(path: &Path, root: &Path, entries: &[(String, String)]) -> Res<Vec<String>> {
    let mut s = String::from("#EXTM3U\n");
    let mut risky = vec![];
    for (rel, title) in entries {
        s.push_str(&format!("#EXTINF:-1,{title}\n{}\n", root.join(rel).display()));
        if rel.contains('#') {
            risky.push(rel.clone());
        }
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, s)?;
    Ok(risky)
}

/// Relative paths, skipping anything outside the collection.
pub fn read(path: &Path, root: &Path) -> Res<Vec<String>> {
    let root = root.canonicalize().unwrap_or(root.to_path_buf());
    let mut out = vec![];
    for line in std::fs::read_to_string(path)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut p = line.to_string();
        if let Some(r) = p.strip_prefix("file://") {
            let r = r.strip_prefix("localhost").unwrap_or(r);
            p = percent_encoding::percent_decode_str(r).decode_utf8_lossy().into_owned();
        }
        let abs = Path::new(&p).canonicalize().unwrap_or_else(|_| Path::new(&p).to_path_buf());
        if let Ok(rel) = abs.strip_prefix(&root) {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
    Ok(out)
}
