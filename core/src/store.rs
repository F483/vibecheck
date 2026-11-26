//! State lives in the collection root, under .vibecheck/: labels.db is hours of
//! listening and irreplaceable, cache.db is CPU time and can be deleted.
//!
//! The same two files, and the same schema, as the Python prototype: either can
//! open what the other wrote.
//!
//! labels.db is a record of what happened, not a snapshot: the current label is
//! the newest row in `labels`, nothing is overwritten.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::palette::{self, C};
use crate::{Res, now};

const HEAD_TAIL_BYTES: u64 = 256 * 1024;

/// (version, name, sql), in order. The number is the version the database is AT
/// once applied.
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (3, "baseline", include_str!("../migrations/0003_baseline.sql")),
    (4, "cue_writes", include_str!("../migrations/0004_cue_writes.sql")),
    (5, "key_energy", include_str!("../migrations/0005_key_energy.sql")),
    (6, "batches", include_str!("../migrations/0006_batches.sql")),
    (7, "label_origin", include_str!("../migrations/0007_label_origin.sql")),
    (8, "label_card", include_str!("../migrations/0008_label_card.sql")),
    (9, "cue_decisions", include_str!("../migrations/0009_cue_decisions.sql")),
    (10, "plays", include_str!("../migrations/0010_plays.sql")),
    (11, "cues_seen", include_str!("../migrations/0011_cues_seen.sql")),
];
pub const SCHEMA_VERSION: i64 = 11;
const BASELINE: i64 = 3;

pub fn vibecheck_dir(root: &Path) -> Res<PathBuf> {
    // VIBECHECK_STATE moves the state elsewhere: for trying things on a copy
    let d = std::env::var_os("VIBECHECK_STATE").map(PathBuf::from).unwrap_or_else(|| root.join(".vibecheck"));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn connect(path: &Path) -> Res<Connection> {
    let con = Connection::open(path)?;
    // busy_timeout first: setting journal_mode itself needs a lock
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    con.execute_batch("PRAGMA foreign_keys=ON")?;
    // rollback journal, not WAL: one file at rest, no -wal/-shm litter
    let _ = con.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL");
    Ok(con)
}

fn user_version(con: &Connection) -> Res<i64> {
    Ok(con.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Migrations still to apply, by name.
pub fn pending(con: &Connection) -> Res<(i64, Vec<String>)> {
    let mut have = user_version(con)?;
    // v1 predates the version stamp; this is the table only it had
    if have == 0 {
        let old: Option<i64> = con
            .query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='label_log'", [], |r| r.get(0))
            .optional()?;
        if old.is_some() {
            have = 1;
        }
    }
    let todo = MIGRATIONS.iter().filter(|m| m.0 > have).map(|m| format!("{:04}_{}", m.0, m.1)).collect();
    Ok((have, todo))
}

/// Bring a database up to SCHEMA_VERSION, keeping a copy of the old one first.
pub fn migrate(con: &Connection, path: Option<&Path>) -> Res<Vec<String>> {
    let (have, _) = pending(con)?;
    let name = path.map(|p| p.display().to_string()).unwrap_or("this database".into());
    if have != 0 && have < BASELINE {
        return Err(format!(
            "{name} is schema v{have}, which this build cannot upgrade: the oldest it knows how to build \
             from is v{BASELINE}.\n  Move it aside and rescan. Nothing has been changed."
        )
        .into());
    }
    if have > SCHEMA_VERSION {
        return Err(format!(
            "{name} is schema v{have}, newer than this build's v{SCHEMA_VERSION}.\n  Update vibecheck. \
             Nothing has been changed."
        )
        .into());
    }
    let todo: Vec<_> = MIGRATIONS.iter().filter(|m| m.0 > have).collect();
    if todo.is_empty() {
        return Ok(vec![]);
    }
    if let (true, Some(p)) = (have != 0, path) {
        // irreplaceable: rounds, predictions and fits exist nowhere else
        let backup = p.with_extension(format!("db.pre-v{have}"));
        if !backup.exists() {
            std::fs::copy(p, &backup)?;
        }
    }
    let mut done = vec![];
    for (v, n, sql) in todo {
        con.execute_batch("BEGIN")?;
        let r = con.execute_batch(sql).and_then(|_| con.execute_batch(&format!("PRAGMA user_version={v}")));
        match r {
            Ok(_) => con.execute_batch("COMMIT")?,
            Err(e) => {
                con.execute_batch("ROLLBACK")?;
                return Err(e.into());
            }
        }
        done.push(format!("{v:04}_{n}"));
    }
    Ok(done)
}

/// The palette lives in two places; this keeps them honest.
fn check_palette(con: &Connection) -> Res<()> {
    let mut st = con.prepare("SELECT id, name, hue, tone, rgb FROM colours ORDER BY id")?;
    let rows: Vec<(i64, String, String, String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?;
    let ok = rows.len() == 8
        && rows.iter().zip(palette::COLOURS.iter()).all(|(r, c)| {
            r.0 == c.id && r.1 == c.name && r.2 == c.hue && r.3 == c.tone && r.4 == c.rgb
        });
    if !ok {
        return Err("the colours table does not match the palette.\n  This database was built by a different \
                    build; migrate it rather than editing the palette."
            .into());
    }
    Ok(())
}

pub fn labels_path(root: &Path) -> Res<PathBuf> {
    Ok(vibecheck_dir(root)?.join("labels.db"))
}

pub fn labels_db(root: &Path) -> Res<Connection> {
    let path = labels_path(root)?;
    let con = connect(&path)?;
    if user_version(&con)? != SCHEMA_VERSION {
        migrate(&con, Some(&path))?;
    }
    check_palette(&con)?;
    Ok(con)
}

/// Opened without migrating, for `migrate` to look first.
pub fn labels_db_raw(root: &Path) -> Res<Option<(Connection, PathBuf)>> {
    let path = labels_path(root)?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some((connect(&path)?, path)))
}

pub fn cache_db(root: &Path) -> Res<Connection> {
    let con = connect(&vibecheck_dir(root)?.join("cache.db"))?;
    con.execute_batch(
        "CREATE TABLE IF NOT EXISTS embeddings (
           hash TEXT NOT NULL, backend TEXT NOT NULL, version TEXT NOT NULL,
           preproc TEXT NOT NULL, dim INTEGER NOT NULL, vec BLOB NOT NULL,
           ts INTEGER NOT NULL, PRIMARY KEY (hash, backend, version, preproc));
         CREATE TABLE IF NOT EXISTS analysis (
           hash TEXT NOT NULL, version TEXT NOT NULL, key TEXT NOT NULL,
           energy INTEGER NOT NULL, detail TEXT NOT NULL, ts INTEGER NOT NULL,
           PRIMARY KEY (hash, version));
         CREATE TABLE IF NOT EXISTS cue_bars (
           hash TEXT NOT NULL, version TEXT NOT NULL, grid TEXT NOT NULL,
           bars INTEGER NOT NULL, dim INTEGER NOT NULL, data BLOB NOT NULL,
           ts INTEGER NOT NULL, PRIMARY KEY (hash, version, grid));
         CREATE TABLE IF NOT EXISTS peaks (
           hash TEXT NOT NULL, bins INTEGER NOT NULL, duration REAL NOT NULL,
           data BLOB NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY (hash, bins));",
    )?;
    Ok(con)
}

// --- identity ----------------------------------------------------------------

/// Canonical form for comparing paths that came from elsewhere: macOS stores
/// filenames decomposed (NFD), rekordbox's xml writes them composed (NFC).
pub fn norm(rel: &str) -> String {
    rel.nfc().collect()
}

fn id3v2_length(f: &mut File) -> std::io::Result<u64> {
    let mut head = [0u8; 10];
    if f.read(&mut head)? < 10 || &head[..3] != b"ID3" {
        return Ok(0);
    }
    let size = head[6..10].iter().fold(0u64, |s, b| (s << 7) | (b & 0x7f) as u64);
    Ok(10 + size + if head[5] & 0x10 != 0 { 10 } else { 0 })
}

fn has_id3v1(f: &mut File, size: u64) -> std::io::Result<bool> {
    if size < 128 {
        return Ok(false);
    }
    f.seek(SeekFrom::End(-128))?;
    let mut t = [0u8; 3];
    f.read_exact(&mut t)?;
    Ok(&t == b"TAG")
}

fn read_upto(f: &mut File, n: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(n as usize);
    f.take(n).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Content identity of the *audio*, ignoring tags: the first and last 256 KB
/// between the ID3 tags, so a tag edit never orphans a cached embedding.
pub fn partial_hash(path: &Path, size: u64) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let start = id3v2_length(&mut f)?;
    let end = size.saturating_sub(if has_id3v1(&mut f, size)? { 128 } else { 0 });
    let span = end.saturating_sub(start);
    let mut h = Sha256::new();
    h.update(span.to_string().as_bytes());
    f.seek(SeekFrom::Start(start))?;
    h.update(read_upto(&mut f, HEAD_TAIL_BYTES.min(span))?);
    if span > 2 * HEAD_TAIL_BYTES {
        f.seek(SeekFrom::Start(end - HEAD_TAIL_BYTES))?;
        h.update(read_upto(&mut f, HEAD_TAIL_BYTES)?);
    }
    Ok(hex::encode(h.finalize())[..32].to_string())
}

// --- reading -------------------------------------------------------------------

/// One complete statement about a track. None means "said nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Label {
    pub colour: Option<C>,
    /// 0..5; 0 is rekordbox's "unrated"
    pub stars: Option<u8>,
}

impl Label {
    pub const NONE: Label = Label { colour: None, stars: None };

    pub fn hue(&self) -> Option<&'static str> {
        self.colour.map(|c| c.hue)
    }
    pub fn tone(&self) -> Option<&'static str> {
        self.colour.map(|c| c.tone)
    }
    /// Whether anything was said: a colour, or a rating other than "unrated".
    pub fn said(&self) -> bool {
        self.colour.is_some() || self.stars.unwrap_or(0) > 0
    }
}

pub fn track_ids(con: &Connection) -> Res<HashMap<String, i64>> {
    pairs(con, "SELECT path, id FROM tracks")
}

/// One track's audio hash, None when it is not in the library.
pub fn hash_of(con: &Connection, path: &str) -> Res<Option<String>> {
    Ok(con.query_row("SELECT hash FROM tracks WHERE path=?1", [path], |r| r.get(0)).optional()?)
}

pub fn hash_by_path(con: &Connection) -> Res<HashMap<String, String>> {
    pairs(con, "SELECT path, hash FROM tracks")
}

fn pairs<K: rusqlite::types::FromSql + std::hash::Hash + Eq, V: rusqlite::types::FromSql>(
    con: &Connection,
    sql: &str,
) -> Res<HashMap<K, V>> {
    let mut st = con.prepare(sql)?;
    let out = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(out)
}

pub fn live_paths(con: &Connection) -> Res<Vec<String>> {
    let mut st = con.prepare("SELECT path FROM tracks WHERE missing_at IS NULL ORDER BY path")?;
    let out = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// Map any normalisation of a path back to the form stored in the index.
pub fn path_index(con: &Connection) -> Res<HashMap<String, String>> {
    let mut st = con.prepare("SELECT path FROM tracks")?;
    let out = st
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|p| p.map(|p| (norm(&p), p)))
        .collect::<Result<_, _>>()?;
    Ok(out)
}

/// Where each track stands, for tracks anything has been said about.
pub fn current(con: &Connection, source: Option<&str>) -> Res<HashMap<String, Label>> {
    let mut q = "SELECT path, colour_id, stars FROM current WHERE label_id IS NOT NULL".to_string();
    if source.is_some() {
        q += " AND source = ?1";
    }
    let mut st = con.prepare(&q)?;
    let map = |r: &rusqlite::Row| -> rusqlite::Result<(String, Option<i64>, Option<i64>)> {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    };
    let rows: Vec<_> = match source {
        Some(s) => st.query_map([s], map)?.collect::<Result<_, _>>()?,
        None => st.query_map([], map)?.collect::<Result<_, _>>()?,
    };
    Ok(rows
        .into_iter()
        .filter(|(_, c, s)| c.is_some() || s.is_some())
        .map(|(p, c, s)| (p, Label { colour: c.and_then(palette::by_id), stars: s.map(|s| s as u8) }))
        .collect())
}

/// Which round last predicted each track: a correction belongs to the round
/// that provoked it.
pub fn round_of(con: &Connection) -> Res<HashMap<String, i64>> {
    pairs(
        con,
        "SELECT t.path, l.round_id FROM labels l
         JOIN tracks t ON t.id = l.track_id
         JOIN (SELECT track_id, MAX(id) AS id FROM labels
               WHERE source = 'model' GROUP BY track_id) m ON l.id = m.id
         WHERE l.round_id IS NOT NULL",
    )
}

// --- writing -------------------------------------------------------------------

pub fn log_label(con: &Connection, track_id: i64, label: Label, source: &str, round_id: Option<i64>) -> Res<()> {
    log_label_from(con, track_id, label, source, round_id, None)
}

/// As `log_label`, saying where it came from: Some("xml") when read from rekordbox.
pub fn log_label_from(
    con: &Connection,
    track_id: i64,
    label: Label,
    source: &str,
    round_id: Option<i64>,
    origin: Option<&str>,
) -> Res<()> {
    log_label_as(con, track_id, label, source, round_id, Made { origin, ..Default::default() })
}

/// How a label came to be, beyond its source: where from (`origin`, Some("xml") from
/// rekordbox), which card asked (`card`), and what was on screen when it was made
/// (`shown`: palette id, None = blind). Training trusts a label by these.
#[derive(Clone, Copy, Default)]
pub struct Made<'a> {
    pub origin: Option<&'a str>,
    pub card: Option<&'a str>,
    pub shown: Option<i64>,
    pub shown_stars: Option<i64>,
}

pub fn log_label_as(con: &Connection, track_id: i64, label: Label, source: &str, round_id: Option<i64>, m: Made) -> Res<()> {
    con.execute(
        "INSERT INTO labels (track_id, colour_id, stars, source, round_id, ts, origin, card, shown, shown_stars)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![track_id, label.colour.map(|c| c.id), label.stars, source, round_id, now(), m.origin, m.card, m.shown, m.shown_stars],
    )?;
    Ok(())
}

/// The current label of a track with everything said about how it was made.
pub struct Current {
    pub label: Label,
    pub source: String,
    pub origin: Option<String>,
    pub card: Option<String>,
    pub shown: Option<i64>,
    /// when it was made, unix seconds
    pub ts: i64,
}

/// As `current`, with how each label was made, for tracks with a label that says something.
pub fn current_made(con: &Connection) -> Res<HashMap<String, Current>> {
    let mut st = con.prepare(
        "SELECT path, colour_id, stars, source, origin, card, shown, ts FROM current WHERE label_id IS NOT NULL",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<i64>>(7)?,
        ))
    })?;
    let mut out = HashMap::new();
    for r in rows {
        let (p, c, s, source, origin, card, shown, ts) = r?;
        if c.is_none() && s.is_none() {
            continue;
        }
        let label = Label { colour: c.and_then(palette::by_id), stars: s.map(|s| s as u8) };
        out.insert(p, Current { label, source, origin, card, shown, ts: ts.unwrap_or(0) });
    }
    Ok(out)
}

/// Tracks played with a label of yours on them: a sync saw the play count rise while
/// rekordbox had the label it had here (`plays`), or, before schema v10, an export of ours
/// was played after (its fate).
pub fn played(con: &Connection) -> Res<HashSet<String>> {
    let mut st = con.prepare(
        "SELECT t.path FROM plays p JOIN tracks t ON t.id = p.track_id WHERE p.label_id IS NOT NULL
         UNION
         SELECT t.path FROM batch_tracks bt
         JOIN batches b ON b.id = bt.batch_id JOIN tracks t ON t.id = bt.track_id
         WHERE b.dir = 'out' AND bt.fate = 'played'",
    )?;
    let out = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// The play count each track had when last seen (a sync, or the export that wrote it).
pub fn play_counts(con: &Connection) -> Res<HashMap<String, i64>> {
    pairs(con, "SELECT t.path, p.plays FROM play_counts p JOIN tracks t ON t.id = p.track_id")
}

/// Remember play counts as seen now: (track id, plays).
pub fn set_play_counts(con: &Connection, counts: &[(i64, i64)], ts: i64) -> Res<()> {
    let mut st = con.prepare(
        "INSERT INTO play_counts (track_id, plays, ts) VALUES (?1,?2,?3)
         ON CONFLICT(track_id) DO UPDATE SET plays = excluded.plays, ts = excluded.ts",
    )?;
    for (t, n) in counts {
        st.execute(params![t, n, ts])?;
    }
    Ok(())
}

/// A play seen: the count went from `was` to `now`, with `label_id` on the track (None: not
/// a label of yours, or not the one rekordbox had).
pub fn log_play(con: &Connection, track_id: i64, label_id: Option<i64>, was: i64, now: i64, ts: i64) -> Res<()> {
    con.execute(
        "INSERT INTO plays (track_id, label_id, count_was, count_now, ts) VALUES (?1,?2,?3,?4,?5)",
        params![track_id, label_id, was, now, ts],
    )?;
    Ok(())
}

/// The id of a track's current label.
pub fn label_id(con: &Connection, track_id: i64) -> Res<Option<i64>> {
    Ok(con.query_row("SELECT label_id FROM current WHERE track_id = ?1", [track_id], |r| r.get(0)).optional()?.flatten())
}

pub fn open_round(con: &Connection, name: &str, size: usize, backend: &str, encoder: &str, n_labels: usize) -> Res<i64> {
    con.execute(
        "INSERT INTO rounds (name, started, size, backend, encoder, n_labels) VALUES (?1,?2,?3,?4,?5,?6)",
        params![name, now(), size as i64, backend, encoder, n_labels as i64],
    )?;
    Ok(con.last_insert_rowid())
}

/// First sync that reads a round back closes it; later syncs do not move it.
pub fn close_round(con: &Connection, round_id: i64) -> Res<()> {
    con.execute("UPDATE rounds SET closed = ?1 WHERE id = ?2 AND closed IS NULL", params![now(), round_id])?;
    Ok(())
}

pub fn log_cue_write(
    con: &Connection,
    track_id: i64,
    round_id: Option<i64>,
    model: &str,
    transitions: &[usize],
    hot: &[f64],
) -> Res<()> {
    con.execute(
        "INSERT INTO cue_writes (track_id, round_id, model, transitions, hot, ts) VALUES (?1,?2,?3,?4,?5,?6)",
        params![track_id, round_id, model, py_json_ints(transitions), py_json_floats(hot), now()],
    )?;
    Ok(())
}

/// path -> hot cue starts this app last wrote there.
/// Tracks whose current label came from a `card` card after `ts`.
pub fn recent_cards(con: &Connection, card: &str, ts: i64) -> Res<HashSet<String>> {
    let mut st = con.prepare("SELECT path FROM current WHERE card = ?1 AND ts > ?2")?;
    let out = st.query_map(params![card, ts], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// Tracks whose cues a card of this kind asked about since `ts`.
pub fn recent_cue_cards(con: &Connection, card: &str, ts: i64) -> Res<HashSet<String>> {
    let mut st = con.prepare("SELECT DISTINCT t.path FROM cue_decisions d JOIN tracks t ON t.id = d.track_id WHERE d.card = ?1 AND d.ts > ?2")?;
    let out = st.query_map(params![card, ts], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

pub fn cues_written(con: &Connection) -> Res<HashMap<String, Vec<f64>>> {
    let m: HashMap<String, String> = pairs(
        con,
        "SELECT t.path, w.hot FROM cue_writes w JOIN tracks t ON t.id = w.track_id
         WHERE w.id = (SELECT MAX(id) FROM cue_writes WHERE track_id = w.track_id)",
    )?;
    Ok(m.into_iter().map(|(p, h)| (p, serde_json::from_str(&h).unwrap_or_default())).collect())
}

/// Your cues for a track as set in the player (JSON), with the suggestion shown then.
pub fn log_cue_decision(con: &Connection, track_id: i64, cues: &str, shown: Option<&str>, card: Option<&str>) -> Res<()> {
    con.execute(
        "INSERT INTO cue_decisions (track_id, cues, shown, card, ts) VALUES (?1,?2,?3,?4,?5)",
        params![track_id, cues, shown, card, now()],
    )?;
    Ok(())
}

/// Your cues per track, the newest decision of each, as JSON.
pub fn cue_decisions(con: &Connection) -> Res<HashMap<String, String>> {
    pairs(
        con,
        "SELECT t.path, d.cues FROM cue_decisions d JOIN tracks t ON t.id = d.track_id
         WHERE d.id = (SELECT MAX(id) FROM cue_decisions WHERE track_id = d.track_id)",
    )
}

/// A cue decision: cues, the suggestion shown then, when.
pub type Decided = (String, Option<String>, i64);

/// Your cues per track as set in the app, the newest decision of each: (cues, the
/// suggestion shown then, when), JSON as `log_cue_decision` keeps them.
pub fn cue_decisions_made(con: &Connection) -> Res<HashMap<String, Decided>> {
    let mut st = con.prepare(
        "SELECT t.path, d.cues, d.shown, d.ts FROM cue_decisions d JOIN tracks t ON t.id = d.track_id
         WHERE d.id = (SELECT MAX(id) FROM cue_decisions WHERE track_id = d.track_id)",
    )?;
    let out = st.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// The hot cues this app last wrote per track, and when.
pub fn cue_writes_last(con: &Connection) -> Res<HashMap<String, (Vec<f64>, i64)>> {
    let mut st = con.prepare(
        "SELECT t.path, w.hot, w.ts FROM cue_writes w JOIN tracks t ON t.id = w.track_id
         WHERE w.id = (SELECT MAX(id) FROM cue_writes WHERE track_id = w.track_id)",
    )?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    let mut out = HashMap::new();
    for r in rows {
        let (p, h, ts) = r?;
        out.insert(p, (serde_json::from_str(&h).unwrap_or_default(), ts));
    }
    Ok(out)
}

/// rekordbox's cues as last seen, per track: their key (`cues::cue_key`) and when a sync
/// last saw you change them.
pub fn cues_seen(con: &Connection) -> Res<HashMap<String, (String, Option<i64>)>> {
    let mut st = con.prepare("SELECT t.path, s.cues, s.changed FROM cues_seen s JOIN tracks t ON t.id = s.track_id")?;
    let out = st.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// Note tracks' cues as seen now: (track id, key). Seen before and different, and not
/// `ours` (written by an export), they changed now. Returns how many changed.
pub fn see_cues(con: &Connection, seen: &[(i64, String)], ts: i64, ours: bool) -> Res<usize> {
    let mut get = con.prepare("SELECT cues FROM cues_seen WHERE track_id = ?1")?;
    let mut put = con.prepare(
        "INSERT INTO cues_seen (track_id, cues, ts, changed) VALUES (?1, ?2, ?3, NULL)
         ON CONFLICT(track_id) DO UPDATE SET cues = excluded.cues, ts = excluded.ts,
         changed = CASE WHEN ?4 THEN ?3 ELSE changed END",
    )?;
    let mut n = 0;
    for (t, key) in seen {
        let was: Option<String> = get.query_row([t], |r| r.get(0)).optional()?;
        let changed = !ours && was.as_ref().is_some_and(|w| w != key);
        n += changed as usize;
        if was.as_ref() != Some(key) || changed {
            put.execute(params![t, key, ts, changed])?;
        }
    }
    Ok(n)
}

/// When each track was last seen played: the sync that saw its play count rise, or, before
/// schema v10, the export of ours it was played after.
pub fn played_at(con: &Connection) -> Res<HashMap<String, i64>> {
    let mut st = con.prepare(
        "SELECT t.path, p.ts FROM plays p JOIN tracks t ON t.id = p.track_id
         UNION ALL
         SELECT t.path, b.ts FROM batch_tracks bt
         JOIN batches b ON b.id = bt.batch_id JOIN tracks t ON t.id = bt.track_id
         WHERE b.dir = 'out' AND bt.fate = 'played'",
    )?;
    let mut out: HashMap<String, i64> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (p, ts) = r?;
        let e = out.entry(p).or_insert(ts);
        *e = (*e).max(ts);
    }
    Ok(out)
}

/// Your key and energy for a track, as corrected; None leaves that one alone.
pub fn log_key_energy(con: &Connection, track_id: i64, key: Option<&str>, energy: Option<u8>) -> Res<()> {
    con.execute(
        "INSERT INTO key_energy (track_id, key, energy, ts) VALUES (?1,?2,?3,?4)",
        params![track_id, key, energy, now()],
    )?;
    Ok(())
}

/// path -> (key, energy) as you last corrected them.
pub fn key_energy(con: &Connection) -> Res<HashMap<String, (Option<String>, Option<u8>)>> {
    let mut st = con.prepare(
        "SELECT t.path, k.key, k.energy FROM key_energy k JOIN tracks t ON t.id = k.track_id
         WHERE k.id = (SELECT MAX(id) FROM key_energy WHERE track_id = k.track_id)",
    )?;
    let out = st.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// Tracks with any statement at all, including a deliberate blank.
pub fn answered(con: &Connection) -> Res<HashSet<String>> {
    let mut st = con.prepare("SELECT path FROM current WHERE label_id IS NOT NULL")?;
    let out = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// JSON the way Python's json.dumps writes it, so rows read the same either way.
pub fn py_json_ints(v: &[usize]) -> String {
    format!("[{}]", v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "))
}

pub fn py_json_floats(v: &[f64]) -> String {
    format!("[{}]", v.iter().map(|x| py_float(*x)).collect::<Vec<_>>().join(", "))
}

/// Python's repr of a float: shortest round-trip, always with a decimal point.
pub fn py_float(x: f64) -> String {
    let s = format!("{x:?}");
    if s.contains('e') {
        // Python writes 1e-05 where Rust writes 1e-5
        let (m, e) = s.split_once('e').unwrap();
        let (sign, digits) = if let Some(d) = e.strip_prefix('-') { ("-", d) } else { ("+", e) };
        return format!("{m}e{sign}{digits:0>2}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_as_python_does() {
        assert_eq!(py_float(12.34), "12.34");
        assert_eq!(py_float(3.0), "3.0");
        assert_eq!(py_float(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(py_json_floats(&[1.5, 20.0]), "[1.5, 20.0]");
    }

    #[test]
    fn nfd_and_nfc_compare_equal_after_norm() {
        assert_eq!(norm("Mo\u{0308}tley"), norm("M\u{00f6}tley"));
    }
}

/// What the model thought on every axis of every track in a round.
pub fn log_predictions(con: &Connection, round_id: i64, rows: &[(i64, &crate::predict::Prediction)]) -> Res<()> {
    use crate::predict::{HUE, STARS, TONE};
    let mut st = con.prepare(
        "INSERT OR REPLACE INTO predictions (round_id, track_id, hue, hue_p, hue_said, tone, tone_p, tone_said,
         stars, stars_p, stars_said, colour_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
    )?;
    for (tid, p) in rows {
        let top = |a: &str| p.top.get(a).cloned().unwrap_or_default();
        let ((h, hp), (t, tp), (s, sp)) = (top(HUE), top(TONE), top(STARS));
        st.execute(params![
            round_id,
            tid,
            h,
            hp,
            p.values.contains_key(HUE) as i64,
            t,
            tp,
            p.values.contains_key(TONE) as i64,
            s.parse::<i64>().unwrap_or(0),
            sp,
            p.values.contains_key(STARS) as i64,
            p.colour().map(|c| c.id)
        ])?;
    }
    Ok(())
}

pub fn log_fit(
    con: &Connection,
    round_id: Option<i64>,
    backend: &str,
    encoder: &str,
    n_train: usize,
    n_labels: usize,
    scores: &[crate::evaluate::AxisScore],
) -> Res<i64> {
    con.execute(
        "INSERT INTO fits (round_id, ts, backend, encoder, slice, n_train, n_holdout, n_labels)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            round_id,
            now(),
            backend,
            encoder,
            crate::evaluate::VAL,
            n_train as i64,
            scores.iter().map(|s| s.n).max().unwrap_or(0) as i64,
            n_labels as i64
        ],
    )?;
    let fit = con.last_insert_rowid();
    for s in scores {
        con.execute(
            "INSERT INTO fit_axes (fit_id, axis, n_values, cost, misleading, n, spoke, correct, cost_left)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![fit, s.axis, s.n_values as i64, s.cost, s.misleading, s.n as i64, s.spoke as i64, s.correct as i64, s.cost_left],
        )?;
    }
    Ok(fit)
}
