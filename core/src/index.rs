//! Scanning: find tracks, identify them by a hash of their audio, notice what
//! moved. Files that disappear are marked, never deleted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::params;

use crate::{Res, now, store};

#[derive(Debug, Default)]
pub struct ScanStats {
    pub files: usize,
    pub new: usize,
    pub moved: usize,
    pub changed: usize,
    pub missing: usize,
    pub returned: usize,
    pub unreadable: usize,
}

/// Every .mp3 under root, sorted, as (absolute, relative).
pub fn mp3s(root: &Path) -> Vec<(PathBuf, String)> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp3")) {
                out.push(p);
            }
        }
    }
    let mut v = vec![];
    walk(root, &mut v);
    let mut out: Vec<_> = v
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(root).ok()?.to_string_lossy().into_owned();
            Some((p, rel))
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// st_mtime as CPython computes it, so the two compare equal.
fn mtime(m: &std::fs::Metadata) -> f64 {
    use std::os::unix::fs::MetadataExt;
    m.mtime() as f64 + m.mtime_nsec() as f64 * 1e-9
}

struct Known {
    id: i64,
    hash: String,
    size: i64,
    mtime: f64,
    missing: Option<i64>,
}

pub fn scan(root: &Path, workers: usize) -> Res<ScanStats> {
    let con = store::labels_db(root)?;
    let mut st = ScanStats::default();
    let t = now();

    let mut known: HashMap<String, Known> = HashMap::new();
    {
        let mut q = con.prepare("SELECT id, path, hash, size, mtime, missing_at FROM tracks")?;
        let rows = q.query_map([], |r| {
            Ok((r.get::<_, String>(1)?, Known {
                id: r.get(0)?,
                hash: r.get(2)?,
                size: r.get(3)?,
                mtime: r.get(4)?,
                missing: r.get(5)?,
            }))
        })?;
        for r in rows {
            let (p, k) = r?;
            known.insert(p, k);
        }
    }

    let files = mp3s(root);
    st.files = files.len();
    let present: HashSet<&str> = files.iter().map(|f| f.1.as_str()).collect();

    // A path that vanished and a hash that reappears elsewhere is one file
    // moving -- only when unambiguous.
    let mut orphan: HashMap<&str, Vec<&str>> = HashMap::new();
    for (rel, k) in &known {
        if !present.contains(rel.as_str()) {
            orphan.entry(k.hash.as_str()).or_default().push(rel.as_str());
        }
    }
    for v in orphan.values_mut() {
        v.sort();
    }

    // hash only where size or mtime changed, in parallel
    let metas: Vec<Option<(u64, f64)>> =
        files.iter().map(|(p, _)| std::fs::metadata(p).ok().map(|m| (m.len(), mtime(&m)))).collect();
    let need: Vec<usize> = (0..files.len())
        .filter(|&i| match (&metas[i], known.get(&files[i].1)) {
            (Some((sz, mt)), Some(k)) => !(k.size == *sz as i64 && k.mtime == *mt),
            (Some(_), None) => true,
            (None, _) => false,
        })
        .collect();
    let mut hashes: Vec<Option<String>> = vec![None; files.len()];
    {
        let next = AtomicUsize::new(0);
        let results: Vec<Vec<(usize, Option<String>)>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..workers.max(1))
                .map(|_| {
                    s.spawn(|| {
                        let mut out = vec![];
                        loop {
                            let j = next.fetch_add(1, Ordering::Relaxed);
                            let Some(&i) = need.get(j) else { break };
                            let (sz, _) = metas[i].unwrap();
                            out.push((i, store::partial_hash(&files[i].0, sz).ok()));
                        }
                        out
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (i, h) in results.into_iter().flatten() {
            hashes[i] = h;
        }
    }

    let mut claimed: HashSet<String> = HashSet::new();
    con.execute_batch("BEGIN")?;
    for (i, (_, rel)) in files.iter().enumerate() {
        let Some((size, mt)) = metas[i] else {
            st.unreadable += 1;
            continue;
        };
        let prev = known.get(rel);
        let unchanged = prev.is_some_and(|k| k.size == size as i64 && k.mtime == mt);
        let h = if unchanged { Some(prev.unwrap().hash.clone()) } else { hashes[i].clone() };
        let Some(h) = h else {
            st.unreadable += 1;
            continue;
        };
        match prev {
            None => {
                let from: Vec<&str> = orphan
                    .get(h.as_str())
                    .map(|v| v.iter().copied().filter(|o| !claimed.contains(*o)).collect())
                    .unwrap_or_default();
                if from.len() == 1 {
                    let was = from[0];
                    claimed.insert(was.to_string());
                    con.execute(
                        "UPDATE tracks SET path=?1, size=?2, mtime=?3, last_seen=?4, missing_at=NULL WHERE id=?5",
                        params![rel, size as i64, mt, t, known[was].id],
                    )?;
                    st.moved += 1;
                    continue;
                }
                con.execute(
                    "INSERT INTO tracks (path, hash, size, mtime, first_seen, last_seen) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![rel, h, size as i64, mt, t, t],
                )?;
                st.new += 1;
            }
            Some(k) => {
                if k.missing.is_some() {
                    st.returned += 1;
                }
                if !unchanged {
                    st.changed += 1;
                }
                con.execute(
                    "UPDATE tracks SET hash=?1, size=?2, mtime=?3, last_seen=?4, missing_at=NULL WHERE id=?5",
                    params![h, size as i64, mt, t, k.id],
                )?;
            }
        }
    }
    for (rel, k) in &known {
        if !present.contains(rel.as_str()) && !claimed.contains(rel) && k.missing.is_none() {
            con.execute("UPDATE tracks SET missing_at=?1 WHERE id=?2", params![t, k.id])?;
            st.missing += 1;
        }
    }
    con.execute_batch("COMMIT")?;
    Ok(st)
}
