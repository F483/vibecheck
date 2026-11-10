//! What the app asks of the core, as plain data: the library as rows, the queue
//! in the order that teaches the model most, one decision, export, sync, the
//! numbers for stats and one track's waveform. The Godot extension only
//! converts these to and from JSON.
//!
//! Decisions made in the app are `user` labels outside any round. What has been
//! labelled since the last export is the batch the next export writes; the
//! export itself is remembered in `.vibecheck/app.json`, with each track's play
//! count then, so a later sync can tell which were played.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::predict::{self, HUE, Prediction, STARS, TONE};
use crate::rekordbox::{self, Entry, Track, WriteOpts};
use crate::store::{self, Label};
use crate::{Res, audio, cues, embed, evaluate, index, keyenergy, now, palette, workflow};

fn local(ts: i64, fmt: &str) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map(|d| d.format(fmt).to_string()).unwrap_or_default()
}

// --- the xml, read once per change -------------------------------------------------

type Xml = Arc<HashMap<String, Track>>;

/// The rekordbox export by indexed path, read again only when the file changes.
pub fn xml(root: &Path, xml: &Path) -> Res<Xml> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (i64, Xml)>>> = OnceLock::new();
    let Some(mtime) = rekordbox::exported(xml) else { return Ok(Arc::new(HashMap::new())) };
    let cache = CACHE.get_or_init(Default::default);
    if let Some((t, x)) = cache.lock().unwrap().get(xml)
        && *t == mtime
    {
        return Ok(x.clone());
    }
    let canon = store::path_index(&store::labels_db(root)?)?;
    let x: Xml = Arc::new(
        rekordbox::read(xml, root)?.into_iter().map(|(k, t)| (canon.get(&k).cloned().unwrap_or(k), t)).collect(),
    );
    cache.lock().unwrap().insert(xml.to_path_buf(), (mtime, x.clone()));
    Ok(x)
}

// --- what the app remembers ------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Export {
    pub ts: i64,
    pub name: String,
    /// path -> play count at export
    pub plays: HashMap<String, i64>,
    /// dropped tracks exported with the colour and stars they had in rekordbox
    /// (no label here): what the next sync compares against
    #[serde(default)]
    pub theirs: HashMap<String, (Option<i64>, Option<i64>)>,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    export: Option<Export>,
    /// tracks dropped on the list in the app: in the next export as they are
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "unsaved")]
    dropped: Vec<String>,
    /// path -> when it was dropped, unix ms: its place in the list
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    dropped_at: HashMap<String, i64>,
}

fn saved_path(root: &Path) -> Res<PathBuf> {
    Ok(store::vibecheck_dir(root)?.join("app.json"))
}

fn load_saved(root: &Path) -> Saved {
    saved_path(root).ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(root: &Path, s: &Saved) -> Res<()> {
    let p = saved_path(root)?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

/// Tracks dropped on the list go out with the next export as they are, labelled
/// or not: kept here until then.
pub fn add_to_batch(root: &Path, paths: &[String]) -> Res<()> {
    let mut s = load_saved(root);
    let at = now_ms();
    for (i, p) in paths.iter().enumerate() {
        if !s.dropped.contains(p) {
            s.dropped.push(p.clone());
            s.dropped_at.insert(p.clone(), at + i as i64);
        }
    }
    save(root, &s)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// When each track in the next export joined the list, unix ms: dropped on it,
/// else its first label since the last export. Orders the list's # column.
fn joined(con: &rusqlite::Connection, s: &Saved, after: i64) -> Res<HashMap<String, i64>> {
    let mut st = con.prepare(
        "SELECT t.path, MIN(l.ts) FROM labels l JOIN tracks t ON t.id = l.track_id
         WHERE l.ts > ?1 AND l.round_id IS NULL AND l.source IN ('user', 'model') GROUP BY t.path",
    )?;
    let mut out: HashMap<String, i64> = st.query_map([after], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? * 1000)))?.collect::<Result<_, _>>()?;
    for (p, at) in &s.dropped_at {
        let e = out.entry(p.clone()).or_insert(*at);
        *e = (*e).min(*at);
    }
    Ok(out)
}

/// Labels made after this belong to the next export: the last export, else the
/// last round the command line made.
fn since(con: &rusqlite::Connection, s: &Saved) -> Res<i64> {
    if let Some(e) = &s.export {
        return Ok(e.ts);
    }
    Ok(con.query_row("SELECT COALESCE(MAX(started), 0) FROM rounds", [], |r| r.get(0))?)
}

/// Tracks labelled here (not read from rekordbox) since the last export, and the ones
/// dropped on the list: waiting for the next export, so rekordbox does not know their
/// labels yet and a sync must not take its value for them as a correction.
fn unsent(con: &rusqlite::Connection, s: &Saved) -> Res<HashSet<String>> {
    let mut st = con.prepare(
        "SELECT path FROM current WHERE source IN ('user', 'model') AND origin IS NULL AND round_id IS NULL AND ts > ?1",
    )?;
    let mut out: HashSet<String> = st.query_map([since(con, s)?], |r| r.get(0))?.collect::<Result<_, _>>()?;
    out.extend(s.dropped.iter().cloned());
    Ok(out)
}

fn batch(con: &rusqlite::Connection, after: i64) -> Res<Vec<String>> {
    let mut st = con.prepare(
        "SELECT path FROM current WHERE source IN ('user', 'model') AND round_id IS NULL AND ts > ?1 AND missing_at IS NULL ORDER BY ts",
    )?;
    let out = st.query_map([after], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

// --- opening ---------------------------------------------------------------------

#[derive(Serialize)]
pub struct Opened {
    pub files: usize,
    pub new: usize,
    pub missing: usize,
    pub xml: bool,
    /// when the xml was last written, unix seconds; 0 without one
    pub exported: i64,
}

/// Bring the index up to date with the folder.
pub fn open(root: &Path, xml_path: &Path) -> Res<Opened> {
    if !root.is_dir() {
        return Err(format!("no folder at {}", root.display()).into());
    }
    let st = index::scan(root, 12)?;
    Ok(Opened {
        files: st.files,
        new: st.new,
        missing: st.missing,
        xml: xml_path.exists(),
        exported: rekordbox::exported(xml_path).unwrap_or(0),
    })
}

// --- the library -----------------------------------------------------------------

#[derive(Serialize, Default)]
pub struct Row {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub mix: String,
    /// palette id, 0 for none
    pub colour: i64,
    /// 0..5, -1 for none said
    pub stars: i64,
    /// "user", "model", "rekordbox" (there before vibecheck) or ""
    pub source: &'static str,
    /// 4 played, 3 set by hand, 2 existing label, 1 untouched guess, 0 nothing
    pub trust: u8,
    pub energy: i64,
    pub key: String,
    pub bpm: f64,
    pub plays: i64,
    pub added: String,
    /// when the current label was made, unix seconds
    pub labelled: i64,
    /// labelled here since the last export: goes out with the next one
    pub batch: bool,
    /// when it joined the export list, unix ms; 0 when not in it
    pub joined: i64,
    pub embedded: bool,
    pub length: f64,
}

/// "8A - Energy 6 ..." -> 6
fn energy_in(comment: &str) -> Option<i64> {
    let i = comment.find("Energy")?;
    comment[i + 6..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

pub fn library(root: &Path, xml_path: &Path) -> Res<Vec<Row>> {
    let con = store::labels_db(root)?;
    let cache = store::cache_db(root)?;
    let x = xml(root, xml_path)?;
    let embedded = embed::cached_hashes(&cache)?;
    let ke: HashMap<String, (String, i64)> = {
        let mut st = cache.prepare("SELECT hash, key, energy FROM analysis WHERE version = ?1")?;
        st.query_map([keyenergy::VERSION], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<Result<_, _>>()?
    };
    let mine = store::key_energy(&con)?;
    let saved = load_saved(root);
    let after = since(&con, &saved)?;
    let mut in_batch: HashSet<String> = batch(&con, after)?.into_iter().collect();
    in_batch.extend(saved.dropped.iter().cloned());
    let joined = joined(&con, &saved, after)?;
    let mut st = con.prepare(
        "SELECT path, hash, colour_id, stars, source, ts FROM current WHERE missing_at IS NULL ORDER BY path",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut out = vec![];
    for row in rows {
        let (path, hash, colour, stars, source, ts) = row?;
        let t = x.get(&path);
        let info = t.map(|t| t.info.clone()).unwrap_or_default();
        let (title, artist) = if info.name.is_empty() {
            let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            match stem.split_once(" - ") {
                Some((a, t)) => (t.to_string(), a.to_string()),
                None => (stem, String::new()),
            }
        } else {
            (info.name.clone(), info.artist.clone())
        };
        let said = colour.is_some() || stars.is_some_and(|s| s > 0);
        let theirs = t.and_then(|t| t.label).filter(|l| l.said());
        let (colour, stars, source, trust) = match source.as_deref() {
            Some("user") if said => (colour, stars, "user", if info.plays > 0 { 4 } else { 3 }),
            Some("model") if said => (colour, stars, "model", 1),
            _ => match theirs {
                Some(l) => (l.colour.map(|c| c.id), l.stars.map(|s| s as i64), "rekordbox", if info.plays > 0 { 4 } else { 2 }),
                None => (None, None, "", 0),
            },
        };
        // what an export writes: your correction, else the analysis, else what rekordbox has
        let (ak, ae) = ke.get(&hash).cloned().unwrap_or_default();
        let (mk, me) = mine.get(&path).cloned().unwrap_or_default();
        out.push(Row {
            title,
            artist,
            mix: if info.mix.is_empty() { info.genre.clone() } else { info.mix.clone() },
            colour: colour.unwrap_or(0),
            stars: stars.unwrap_or(-1),
            source,
            trust,
            energy: me.map(i64::from).or((ae > 0).then_some(ae)).or(energy_in(&info.comment)).unwrap_or(0),
            key: mk.or((!ak.is_empty()).then_some(ak)).unwrap_or(info.tonality.clone()),
            bpm: info.bpm,
            plays: info.plays,
            added: info.added.clone(),
            labelled: if source == "user" || source == "model" { ts.unwrap_or(0) } else { 0 },
            batch: in_batch.contains(&path),
            joined: if in_batch.contains(&path) { joined.get(&path).copied().unwrap_or(0) } else { 0 },
            embedded: embedded.contains(&hash),
            length: t.map(|t| t.duration).unwrap_or(0.0),
            path,
        });
    }
    Ok(out)
}

// --- measuring ---------------------------------------------------------------------

fn axes(cfg: &Config) -> Vec<predict::Axis> {
    let mc = cfg.float("predict", "misleading_cost", 1.0);
    predict::axes_for(mc, cfg.float("predict", "rating_misleading_cost", mc))
}

/// The colour the model would pick if made to choose, and how sure it is.
fn best_colour(p: &Prediction) -> Option<(i64, f64)> {
    let (h, ph) = p.top.get(HUE)?;
    let (t, pt) = p.top.get(TONE)?;
    Some((palette::from_grid(Some(h), Some(t))?.id, ph * pt))
}

/// 95% Wilson interval of k right out of n.
pub fn wilson(k: usize, n: usize) -> [f64; 2] {
    if n == 0 {
        return [0.0, 1.0];
    }
    let (z, n, p) = (1.96f64, n as f64, k as f64 / n as f64);
    let den = 1.0 + z * z / n;
    let mid = (p + z * z / (2.0 * n)) / den;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / den;
    [(mid - half).max(0.0), (mid + half).min(1.0)]
}

struct Labelled {
    paths: Vec<String>,
    x: Vec<Vec<f32>>,
    y: Vec<Label>,
    hashes: Vec<String>,
}

fn labelled(root: &Path) -> Res<Labelled> {
    let con = store::labels_db(root)?;
    let lab = store::current(&con, Some("user"))?;
    let mut keys: Vec<String> = lab.keys().cloned().collect();
    keys.sort();
    let (paths, x) = embed::load(root, &keys)?;
    let hb = store::hash_by_path(&con)?;
    let y = paths.iter().map(|p| lab[p]).collect();
    let hashes = paths.iter().map(|p| hb[p].clone()).collect();
    Ok(Labelled { paths, x, y, hashes })
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Score {
    /// colour picked right, as a 95% range
    pub colour: [f64; 2],
    pub colour_n: usize,
    pub stars: [f64; 2],
    pub stars_n: usize,
    /// (truth, picked, times) for the commonest mix-ups, palette ids
    pub confused: Vec<(i64, i64, usize)>,
}

/// Train on the training slice (the first `take` of it), score the validation
/// slice: the colour the model would pick, against the one you gave.
fn fit(l: &Labelled, ax: &[predict::Axis], take: usize) -> (predict::Model, Score) {
    let (mut xt, mut yt, mut xv, mut yv) = (vec![], vec![], vec![], vec![]);
    for i in 0..l.x.len() {
        match evaluate::part(&l.hashes[i]) {
            "train" if xt.len() < take => {
                xt.push(l.x[i].as_slice());
                yt.push(l.y[i]);
            }
            p if p == evaluate::VAL => {
                xv.push(l.x[i].as_slice());
                yv.push(l.y[i]);
            }
            _ => {}
        }
    }
    let mut m = predict::Model::new(ax.to_vec());
    let mut s = Score { colour: [0.0, 1.0], stars: [0.0, 1.0], ..Default::default() };
    if xt.len() < 2 {
        return (m, s);
    }
    m.train(&xt, &yt);
    let (mut ck, mut sk) = (0, 0);
    let mut mix: HashMap<(i64, i64), usize> = HashMap::new();
    for (p, y) in m.predict(&xv).iter().zip(&yv) {
        if let (Some(c), Some((g, _))) = (y.colour, best_colour(p)) {
            s.colour_n += 1;
            if c.id == g {
                ck += 1;
            } else {
                *mix.entry((c.id, g)).or_default() += 1;
            }
        }
        if let (Some(st), Some((g, _))) = (y.stars, p.top.get(STARS)) {
            s.stars_n += 1;
            sk += (g.parse::<u8>().ok() == Some(st)) as usize;
        }
    }
    s.colour = wilson(ck, s.colour_n);
    s.stars = wilson(sk, s.stars_n);
    let mut mx: Vec<_> = mix.into_iter().map(|((a, b), n)| (a, b, n)).collect();
    mx.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    mx.truncate(3);
    s.confused = mx;
    (m, s)
}

/// The model in use: trained on the training slice, so the score it shows is
/// its own. Training takes seconds, so it is kept until the labels or the dial
/// change: in memory, and in cache.db so the next start skips it too.
/// The model trained on your labels, cached in memory and in cache.db. With
/// `stale_ok`, the last one trained on fewer of your labels will do (and the
/// second value says so): the app starts on it and retrains behind it.
/// What a model trained now would be cached under: its kind (settings and embedding), and
/// the kind with your labels as they are.
fn model_key(root: &Path, cfg: &Config, l: &Labelled) -> Res<(String, String)> {
    let con = store::labels_db(root)?;
    // only your labels change it: the model's guesses don't count
    let last: i64 = con.query_row("SELECT COALESCE(MAX(id), 0) FROM labels WHERE source = 'user'", [], |r| r.get(0))?;
    let ax = axes(cfg);
    let kind = format!(
        "m2|{}|{}|{}|{}|{}",
        root.display(),
        ax[0].misleading_cost,
        ax[2].misleading_cost,
        predict::C,
        embed::fingerprint()
    );
    let key = format!("{kind}#{last}|{}", l.y.len());
    Ok((kind, key))
}

fn model(root: &Path, cfg: &Config, l: &Labelled, stale_ok: bool) -> Res<(Arc<(predict::Model, Score)>, bool)> {
    #[derive(Serialize, Deserialize)]
    struct Saved {
        models: HashMap<String, predict::Lr>,
        score: Score,
    }
    type Cached = Option<(String, Arc<(predict::Model, Score)>)>;
    static CACHE: Mutex<Cached> = Mutex::new(None);
    let (kind, key) = model_key(root, cfg, l)?;
    let ax = axes(cfg);
    if let Some((k, m)) = CACHE.lock().unwrap().as_ref()
        && (*k == key || stale_ok && k.starts_with(&format!("{kind}#")))
    {
        return Ok((m.clone(), *k != key));
    }
    let cache = store::cache_db(root)?;
    cache.execute_batch("CREATE TABLE IF NOT EXISTS models (key TEXT PRIMARY KEY, data TEXT NOT NULL, ts INTEGER NOT NULL)")?;
    let saved: Option<(String, String)> = if stale_ok {
        cache.query_row("SELECT key, data FROM models WHERE substr(key, 1, ?2) = ?1", rusqlite::params![format!("{kind}#"), (kind.len() + 1) as i64], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    } else {
        cache.query_row("SELECT key, data FROM models WHERE key = ?1", [&key], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    };
    let stale = saved.as_ref().is_some_and(|(k, _)| *k != key);
    let m = match saved.and_then(|(_, d)| serde_json::from_str::<Saved>(&d).ok()) {
        Some(s) => {
            let mut m = predict::Model::new(ax.clone());
            for a in &ax {
                if let Some(lr) = s.models.get(a.name) {
                    m.models.insert(a.name, lr.clone());
                }
            }
            Arc::new((m, s.score))
        }
        None => {
            let (m, score) = fit(l, &ax, usize::MAX);
            if crate::stopping() {
                return Err("stopped".into());
            }
            let s = Saved { models: m.models.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), score: score.clone() };
            cache.execute("DELETE FROM models", [])?;
            cache.execute("INSERT INTO models (key, data, ts) VALUES (?1, ?2, ?3)", rusqlite::params![key, serde_json::to_string(&s)?, now()])?;
            Arc::new((m, score))
        }
    };
    if !stale {
        *CACHE.lock().unwrap() = Some((key, m.clone()));
    }
    Ok((m, stale))
}

/// Retrain on all your labels now, if the cached model is behind them.
pub fn train(root: &Path, cfg: &Config) -> Res<()> {
    model(root, cfg, &labelled(root)?, false)?;
    Ok(())
}

// --- the queue ---------------------------------------------------------------------

#[derive(Serialize)]
pub struct Item {
    pub path: String,
    /// "calibration", "unsure", "conflict" or "old label"
    pub tag: &'static str,
    /// the colour the model would pick, palette id; 0 when asked blind
    pub guess: i64,
    pub stars: i64,
}

#[derive(Serialize)]
pub struct Queue {
    pub items: Vec<Item>,
    /// your labels: what the model learns from
    pub labels: usize,
    pub accuracy: [f64; 2],
    pub measured: usize,
    /// tracks that could be asked: listened to, not labelled by you
    pub pool: usize,
    /// asked of a model trained before your latest labels: retrain, then ask again
    pub stale: bool,
}

fn unit(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter().map(|x| x / n).collect()
}

/// Up to `n` of `pool`, each as far as possible from everything labelled or
/// picked so far: covers the sound of the collection, not its commonest corner.
fn spread(pool: &[Vec<f32>], have: &[Vec<f32>], n: usize) -> Vec<usize> {
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let mut d: Vec<f32> = pool.iter().map(|p| have.iter().map(|h| 1.0 - dot(p, h)).fold(f32::MAX, f32::min)).collect();
    let mut out = vec![];
    while out.len() < n.min(pool.len()) {
        let i = (0..d.len()).fold(0, |b, i| if d[i] > d[b] { i } else { b });
        out.push(i);
        let p = pool[i].clone();
        for j in 0..d.len() {
            d[j] = d[j].min(1.0 - dot(&pool[j], &p));
        }
        d[i] = f32::MIN;
    }
    out
}

/// The next `n` tracks to ask about. Below `threshold` labels only calibration
/// tracks, blind and spread across the sound of the collection. From then on
/// the tracks the model is least sure about, mixed with conflicts (your label
/// against a model that is sure otherwise) and old labels (a colour from
/// before vibecheck the model disagrees with), those two asked blind.
pub fn queue(root: &Path, xml_path: &Path, cfg: &Config, threshold: usize, n: usize, skip: &[String], stale_ok: bool) -> Res<Queue> {
    let con = store::labels_db(root)?;
    let l = labelled(root)?;
    let mine: HashSet<String> = store::current(&con, Some("user"))?.into_keys().collect();
    let skip: HashSet<&String> = skip.iter().collect();
    let live = store::live_paths(&con)?;
    let open: Vec<String> = live.into_iter().filter(|p| !mine.contains(p) && !skip.contains(p)).collect();
    let (pool, px) = embed::load(root, &open)?;
    let (ms, stale) = model(root, cfg, &l, stale_ok)?;
    let (m, s) = (&ms.0, &ms.1);
    let mut q = Queue { items: vec![], labels: mine.len(), accuracy: s.colour, measured: s.colour_n, pool: pool.len(), stale };
    if mine.len() < threshold || l.x.len() < 20 {
        let have: Vec<Vec<f32>> = l.x.iter().map(|v| unit(v)).collect();
        let ux: Vec<Vec<f32>> = px.iter().map(|v| unit(v)).collect();
        for i in spread(&ux, &have, n) {
            q.items.push(Item { path: pool[i].clone(), tag: "calibration", guess: 0, stars: -1 });
        }
        return Ok(q);
    }
    let xr: Vec<&[f32]> = l.x.iter().map(|v| v.as_slice()).collect();
    // least sure first
    let pr: Vec<&[f32]> = px.iter().map(|v| v.as_slice()).collect();
    let preds = m.predict(&pr);
    let mut unsure: Vec<(f64, usize)> = preds.iter().enumerate().filter_map(|(i, p)| Some((best_colour(p)?.1, i))).collect();
    unsure.sort_by(|a, b| a.0.total_cmp(&b.0));
    // yours, where the model is sure of another colour
    let mut conflicts: Vec<(f64, String)> = m
        .predict(&xr)
        .iter()
        .zip(&l.y)
        .zip(&l.paths)
        .filter(|((_, y), p)| y.colour.is_some() && !skip.contains(p))
        .filter_map(|((p, y), path)| {
            let (g, pg) = best_colour(p)?;
            (g != y.colour?.id && pg >= 0.5).then(|| (pg, path.clone()))
        })
        .collect();
    conflicts.sort_by(|a, b| b.0.total_cmp(&a.0));
    // colours from before vibecheck the model disagrees with
    let x = xml(root, xml_path)?;
    let spoken: HashSet<String> = store::current(&con, None)?.into_keys().collect();
    let mut old: Vec<(f64, usize)> = preds
        .iter()
        .enumerate()
        .filter(|(i, _)| !spoken.contains(&pool[*i]))
        .filter_map(|(i, p)| {
            let theirs = x.get(&pool[i])?.label?.colour?;
            let (g, pg) = best_colour(p)?;
            (g != theirs.id && pg >= 0.5).then_some((pg, i))
        })
        .collect();
    old.sort_by(|a, b| b.0.total_cmp(&a.0));
    let star = |p: &Prediction| p.top.get(STARS).and_then(|s| s.0.parse().ok()).unwrap_or(-1);
    let (mut a, mut b, mut c) = (unsure.iter(), conflicts.iter(), old.iter());
    let mut used: HashSet<usize> = HashSet::new();
    let mut slot = 0;
    while q.items.len() < n {
        slot += 1;
        let want = if slot % 5 == 3 { 1 } else if slot % 7 == 5 { 2 } else { 0 };
        let mut placed = false;
        for k in [want, 0, 1, 2] {
            match k {
                1 => {
                    if let Some((_, p)) = b.next() {
                        q.items.push(Item { path: p.clone(), tag: "conflict", guess: 0, stars: -1 });
                        placed = true;
                    }
                }
                2 => {
                    if let Some((_, i)) = c.find(|(_, i)| !used.contains(i)) {
                        used.insert(*i);
                        q.items.push(Item { path: pool[*i].clone(), tag: "old label", guess: 0, stars: -1 });
                        placed = true;
                    }
                }
                _ => {
                    if let Some((_, i)) = a.find(|(_, i)| !used.contains(i)) {
                        used.insert(*i);
                        let p = &preds[*i];
                        q.items.push(Item {
                            path: pool[*i].clone(),
                            tag: "unsure",
                            guess: best_colour(p).map(|g| g.0).unwrap_or(0),
                            stars: star(p),
                        });
                        placed = true;
                    }
                }
            }
            if placed {
                break;
            }
        }
        if !placed {
            break;
        }
    }
    Ok(q)
}

// --- deciding ----------------------------------------------------------------------

/// Your label for one track: a colour (palette id), and stars when given (1..5);
/// with a key ("8A") and energy (1..10) when you corrected them.
pub fn decide(root: &Path, path: &str, colour: i64, stars: i64, key: Option<&str>, energy: Option<i64>) -> Res<()> {
    let con = store::labels_db(root)?;
    let id: i64 = con.query_row("SELECT id FROM tracks WHERE path = ?1", [path], |r| r.get(0))?;
    let colour = Some(palette::by_id(colour).ok_or("no such colour")?);
    let stars = (1..=5).contains(&stars).then_some(stars as u8);
    let key = key.filter(|k| is_key(k));
    let energy = energy.filter(|e| (1..=10).contains(e)).map(|e| e as u8);
    con.execute_batch("BEGIN")?;
    store::log_label(&con, id, Label { colour, stars }, "user", None)?;
    if key.is_some() || energy.is_some() {
        // a correction of one keeps the other as it was corrected before
        let (k0, e0) = store::key_energy(&con)?.remove(path).unwrap_or_default();
        store::log_key_energy(&con, id, key.or(k0.as_deref()), energy.or(e0))?;
    }
    con.execute_batch("COMMIT")?;
    Ok(())
}

/// "1A".."12B": rekordbox's alphanumeric keys.
fn is_key(k: &str) -> bool {
    let (n, m) = k.split_at(k.len().saturating_sub(1));
    (m == "A" || m == "B") && n.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n))
}

/// Listen to tracks that have no vector yet, so they can be learned from and asked.
#[derive(Serialize)]
pub struct Guess {
    pub path: String,
    /// palette id, 0 when the model has no colour for it
    pub colour: i64,
    pub stars: i64,
}

/// The model's colour and stars for tracks with none, saved as its labels
/// (source "model", trust untouched) so they go out with the next export;
/// listens first to any it hasn't heard. Tracks that have a label, or that it
/// has no colour for, are left out; none while it has too little to learn from.
pub fn guess(root: &Path, cfg: &Config, paths: &[String]) -> Res<Vec<Guess>> {
    embed::embed_all(root, paths)?;
    let l = labelled(root)?;
    if l.x.len() < 20 {
        return Ok(vec![]);
    }
    let (ms, _) = model(root, cfg, &l, true)?;
    let con = store::labels_db(root)?;
    let said = store::current(&con, None)?;
    let open: Vec<String> = paths.iter().filter(|p| !said.get(*p).is_some_and(|l| l.said())).cloned().collect();
    let (ps, x) = embed::load(root, &open)?;
    let xr: Vec<&[f32]> = x.iter().map(|v| v.as_slice()).collect();
    let preds = ms.0.predict(&xr);
    let mut out = vec![];
    con.execute_batch("BEGIN")?;
    for (p, path) in preds.iter().zip(ps) {
        let Some(colour) = best_colour(p).and_then(|g| palette::by_id(g.0)) else { continue };
        let stars: i64 = p.top.get(STARS).and_then(|s| s.0.parse().ok()).unwrap_or(-1);
        let id: i64 = con.query_row("SELECT id FROM tracks WHERE path = ?1", [&path], |r| r.get(0))?;
        let label = Label { colour: Some(colour), stars: (1..=5).contains(&stars).then_some(stars as u8) };
        store::log_label(&con, id, label, "model", None)?;
        out.push(Guess { path, colour: colour.id, stars });
    }
    con.execute_batch("COMMIT")?;
    Ok(out)
}

pub fn embed(root: &Path, paths: &[String]) -> Res<usize> {
    Ok(embed::embed_all(root, paths)?.done)
}

/// Nothing left to listen to or guess: free the listening model's memory.
pub fn rest() {
    embed::unload();
}

/// Up to `n` live tracks with no vector yet, in a fixed random order.
/// Files dropped on the app (absolute, as Finder or rekordbox give them) as the
/// library's own paths; None for a file outside the collection or not in it.
pub fn dropped(root: &Path, paths: &[String]) -> Res<Vec<Option<String>>> {
    let con = store::labels_db(root)?;
    let mut st = con.prepare("SELECT path FROM tracks WHERE missing_at IS NULL")?;
    let known: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(match_dropped(root, paths, &known))
}

/// Matched on the NFC form: macOS hands out names as stored, which may be NFD
/// or NFC; a symlink in the collection's own path is resolved on both sides.
fn match_dropped(root: &Path, paths: &[String], known: &[String]) -> Vec<Option<String>> {
    let by: HashMap<String, &String> = known.iter().map(|p| (store::norm(p), p)).collect();
    let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    paths
        .iter()
        .map(|p| {
            let p = Path::new(p);
            let p2 = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            let rel = p.strip_prefix(root).or_else(|_| p2.strip_prefix(&real)).ok()?;
            by.get(&store::norm(&rel.to_string_lossy())).map(|s| s.to_string())
        })
        .collect()
}

pub fn unheard(root: &Path, n: usize) -> Res<Vec<String>> {
    let con = store::labels_db(root)?;
    let have = embed::cached_hashes(&store::cache_db(root)?)?;
    let mut st = con.prepare("SELECT path, hash FROM tracks WHERE missing_at IS NULL ORDER BY hash")?;
    let rows: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(rows.into_iter().filter(|(_, h)| !have.contains(h)).map(|(p, _)| p).take(n).collect())
}

// --- export and sync -----------------------------------------------------------------

#[derive(Serialize, Default)]
pub struct Exported {
    pub name: String,
    pub count: usize,
    pub coloured: usize,
    pub rated: usize,
    pub keyed: usize,
    pub commented: usize,
    pub cued: usize,
    pub not_in_xml: usize,
    /// the xml's time after writing, so a watcher can tell our write from rekordbox's
    pub mtime: i64,
}

/// Write what was labelled since the last export into the xml, as one playlist,
/// with key, energy and cue points as configured.
pub fn export(root: &Path, xml_path: &Path, cfg: &Config) -> Res<Exported> {
    if !xml_path.exists() {
        return Err(format!(
            "no rekordbox export at {}: in rekordbox, File › Export Collection in xml format",
            xml_path.display()
        )
        .into());
    }
    let con = store::labels_db(root)?;
    let mut saved = load_saved(root);
    let mut paths = batch(&con, since(&con, &saved)?)?;
    let live: HashSet<String> = store::live_paths(&con)?.into_iter().collect();
    for p in &saved.dropped {
        if live.contains(p) && !paths.contains(p) {
            paths.push(p.clone());
        }
    }
    if paths.is_empty() {
        return Err("nothing labelled since the last export".into());
    }
    // yours, or the model's guess for a track dropped in the app with no colour
    let labels = store::current(&con, None)?;
    let tonality = cfg.bool("rekordbox", "write_tonality", true);
    let comment = cfg.bool("rekordbox", "write_comment", true);
    let mut ke = if tonality || comment { keyenergy::analyse_all(root, &paths)?.0 } else { HashMap::new() };
    for (p, (k, e)) in store::key_energy(&con)? {
        if let Some(a) = ke.get_mut(&p) {
            a.key = k.unwrap_or(a.key.clone());
            a.energy = e.unwrap_or(a.energy);
        }
    }
    let x = xml(root, xml_path)?;
    let plan = if cfg.bool("cues", "write", true) { Some(workflow::plan_cues(root, cfg, &paths, &x)?) } else { None };
    let name = format!("vibecheck-{}", local(now(), "%Y%m%d-%H%M"));
    let entries: Vec<Entry> = paths
        .iter()
        .map(|p| Entry {
            rel: p.clone(),
            level: "colour",
            label: labels.get(p).copied(),
            key: ke.get(p).map(|k| k.key.clone()),
            energy: ke.get(p).map(|k| k.energy),
            cues: plan.as_ref().and_then(|c| c.marks.get(p)).map(|m| m.0.clone()),
        })
        .collect();
    let o = WriteOpts { name: &name, tonality, comment, changed_only: false };
    let st = rekordbox::write(xml_path, xml_path, root, &entries, &o)?;
    if let Some(p) = &plan
        && !p.marks.is_empty()
    {
        workflow::log_cues(root, p, None)?;
    }
    let ts = now();
    // the ones with no label here: remember what rekordbox had, to see a change at the next sync
    let theirs = paths
        .iter()
        .filter(|p| !labels.contains_key(*p))
        .map(|p| {
            let l = x.get(p).and_then(|t| t.label);
            (p.clone(), (l.and_then(|l| l.colour).map(|c| c.id), l.and_then(|l| l.stars).map(i64::from)))
        })
        .collect();
    saved.export = Some(Export {
        ts,
        name: name.clone(),
        plays: paths.iter().map(|p| (p.clone(), x.get(p).map(|t| t.info.plays).unwrap_or(0))).collect(),
        theirs,
    });
    saved.dropped.clear();
    saved.dropped_at.clear();
    save(root, &saved)?;
    let out = Exported {
        name: name.clone(),
        count: paths.len(),
        coloured: st.coloured,
        rated: st.rated,
        keyed: st.keyed,
        commented: st.commented,
        cued: st.cued,
        not_in_xml: st.not_in_xml,
        mtime: rekordbox::exported(xml_path).unwrap_or(ts),
    };
    let rows: Vec<Kept> = paths
        .iter()
        .map(|p| Kept {
            path: p.clone(),
            what: "exported",
            before: None,
            after: labels.get(p).copied().or_else(|| x.get(p).and_then(|t| t.label)),
            key: ke.get(p).map(|k| k.key.clone()),
            energy: ke.get(p).map(|k| k.energy as i64),
            cues: plan.as_ref().and_then(|c| c.marks.get(p)).map(|m| m.0.len() as i64),
        })
        .collect();
    log_batch(&con, "out", ts, &name, None, &serde_json::to_value(&out)?, &rows)?;
    Ok(out)
}

#[derive(Serialize, Default)]
pub struct Synced {
    /// tracks in the last export
    pub out: usize,
    pub played: usize,
    pub corrected: usize,
    pub untouched: usize,
    /// labels taken from rekordbox anywhere in the collection
    pub added: usize,
    pub changed: usize,
    /// out in a round of the command line's, now confirmed
    pub confirmed: usize,
    /// left alone: labelled here after the xml was exported
    pub newer: usize,
    pub unknown: usize,
}

/// Read what changed in rekordbox: corrections become your labels, and the
/// tracks of the last export that were played since are counted.
pub fn sync(root: &Path, xml_path: &Path) -> Res<Synced> {
    let mut saved = load_saved(root);
    let con = store::labels_db(root)?;
    let out_id = out_batch(&con, &saved)?;
    let was = store::current(&con, None)?;
    // tracks exported with rekordbox's own colour are compared below, not taken as new labels
    let scope: Option<HashSet<String>> =
        saved.export.as_ref().map(|e| e.plays.keys().filter(|p| !e.theirs.contains_key(*p)).cloned().collect());
    // stale_ok: the app saves guesses for dropped tracks at any time, so an export older
    // than the newest guess is normal; the tracks labelled after it are skipped one by one
    // (`newer`), which is all the command line's whole-file refusal protects
    let hold = unsent(&con, &saved)?;
    let mut n = workflow::read_corrections(root, xml_path, scope.clone(), false, true, &hold)?;
    let x = xml(root, xml_path)?;
    if let Some(e) = &saved.export {
        let theirs = their_corrections(root, e, &x)?;
        n.changed += theirs.len();
        n.paths.extend(theirs);
    }
    let mut s = Synced { added: n.added, changed: n.changed, confirmed: n.confirmed, newer: n.newer, unknown: n.unknown, ..Default::default() };
    let is = store::current(&con, None)?;
    let ids = store::track_ids(&con)?;
    let mut rows: Vec<Kept> = n
        .paths
        .iter()
        .map(|p| Kept::label(p, if was.contains_key(p) { "corrected" } else { "added" }, was.get(p), is.get(p)))
        .collect();
    if let Some(e) = saved.export.as_mut() {
        let fixed: HashSet<&String> = n.paths.iter().collect();
        s.out = e.plays.len();
        for (p, before) in e.plays.iter_mut() {
            let plays = x.get(p).map(|t| t.info.plays).unwrap_or(*before);
            // what became of it, for the history page: played stays played, a
            // correction outranks untouched
            let fate = if plays > *before {
                s.played += 1;
                *before = plays;
                if !fixed.contains(p) {
                    rows.push(Kept::label(p, "played", was.get(p), is.get(p)));
                }
                "played"
            } else if fixed.contains(p) {
                s.corrected += 1;
                "corrected"
            } else {
                s.untouched += 1;
                "untouched"
            };
            if let (Some(b), Some(t)) = (out_id, ids.get(p)) {
                con.execute(
                    "UPDATE batch_tracks SET fate = ?3 WHERE batch_id = ?1 AND track_id = ?2
                     AND (fate IS NULL OR ?3 = 'played' OR (?3 = 'corrected' AND fate = 'untouched'))",
                    rusqlite::params![b, t, fate],
                )?;
            }
        }
        save(root, &saved)?;
    }
    let name = xml_path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    log_batch(&con, "in", now(), &name, rekordbox::exported(xml_path), &serde_json::to_value(&s)?, &rows)?;
    Ok(s)
}

/// Dropped tracks exported with rekordbox's colour: a colour or stars changed in
/// rekordbox since is your label now; one left as it was stays as it is.
fn their_corrections(root: &Path, e: &Export, x: &HashMap<String, Track>) -> Res<Vec<String>> {
    let con = store::labels_db(root)?;
    let have = store::current(&con, None)?;
    let mut out = vec![];
    con.execute_batch("BEGIN")?;
    for (p, (c0, s0)) in &e.theirs {
        let Some(l) = x.get(p).and_then(|t| t.label) else { continue };
        let (c, s) = (l.colour.map(|c| c.id), l.stars.map(i64::from));
        if (c, s) == (*c0, *s0) || !l.said() || have.contains_key(p) {
            continue;
        }
        let id: i64 = con.query_row("SELECT id FROM tracks WHERE path = ?1", [p], |r| r.get(0))?;
        store::log_label_from(&con, id, l, "user", None, Some("xml"))?;
        out.push(p.clone());
    }
    con.execute_batch("COMMIT")?;
    Ok(out)
}

// --- history -----------------------------------------------------------------------

/// A track of a batch, as it is kept.
struct Kept {
    path: String,
    what: &'static str,
    before: Option<Label>,
    after: Option<Label>,
    key: Option<String>,
    energy: Option<i64>,
    cues: Option<i64>,
}

impl Kept {
    fn label(path: &str, what: &'static str, before: Option<&Label>, after: Option<&Label>) -> Kept {
        Kept { path: path.to_string(), what, before: before.copied(), after: after.copied(), key: None, energy: None, cues: None }
    }
}

fn log_batch(
    con: &rusqlite::Connection,
    dir: &str,
    ts: i64,
    name: &str,
    xml_ts: Option<i64>,
    counts: &serde_json::Value,
    rows: &[Kept],
) -> Res<i64> {
    let ids = store::track_ids(con)?;
    con.execute_batch("BEGIN")?;
    con.execute(
        "INSERT INTO batches (dir, ts, name, xml_ts, counts) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![dir, ts, name, xml_ts, counts.to_string()],
    )?;
    let id = con.last_insert_rowid();
    let mut st = con.prepare(
        "INSERT OR REPLACE INTO batch_tracks (batch_id, track_id, what, colour_before, stars_before, colour, stars, key, energy, cues)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )?;
    for r in rows {
        let Some(t) = ids.get(&r.path) else { continue };
        st.execute(rusqlite::params![
            id,
            t,
            r.what,
            r.before.and_then(|l| l.colour).map(|c| c.id),
            r.before.and_then(|l| l.stars),
            r.after.and_then(|l| l.colour).map(|c| c.id),
            r.after.and_then(|l| l.stars),
            r.key,
            r.energy,
            r.cues
        ])?;
    }
    con.execute_batch("COMMIT")?;
    Ok(id)
}

/// The last export's batch. One made before history was kept is entered from what
/// app.json remembers of it: its tracks, with their labels as they are now.
fn out_batch(con: &rusqlite::Connection, saved: &Saved) -> Res<Option<i64>> {
    use rusqlite::OptionalExtension;
    let Some(e) = &saved.export else { return Ok(None) };
    let had = con
        .query_row("SELECT id FROM batches WHERE dir = 'out' AND name = ?1 ORDER BY id DESC LIMIT 1", [&e.name], |r| r.get(0))
        .optional()?;
    if had.is_some() {
        return Ok(had);
    }
    let labels = store::current(con, None)?;
    let rows: Vec<Kept> = e.plays.keys().map(|p| Kept::label(p, "exported", None, labels.get(p))).collect();
    Ok(Some(log_batch(con, "out", e.ts, &e.name, None, &serde_json::json!({ "count": rows.len() }), &rows)?))
}

/// One sync or export on the history page.
#[derive(Serialize)]
pub struct Batch {
    pub id: i64,
    /// "in" from rekordbox, "out" to it
    pub dir: String,
    pub ts: i64,
    pub name: String,
    pub xml_ts: Option<i64>,
    /// the summary, as `Synced` or `Exported`
    pub counts: serde_json::Value,
    pub tracks: usize,
    /// out: what became of its tracks, as the syncs since saw them
    pub played: usize,
    pub corrected: usize,
    pub untouched: usize,
}

/// Every sync and export, newest first.
pub fn history(root: &Path) -> Res<Vec<Batch>> {
    let con = store::labels_db(root)?;
    out_batch(&con, &load_saved(root))?;
    let mut st = con.prepare(
        "SELECT b.id, b.dir, b.ts, b.name, b.xml_ts, b.counts, COUNT(t.track_id),
                SUM(t.fate = 'played'), SUM(t.fate = 'corrected'), SUM(t.fate = 'untouched')
         FROM batches b LEFT JOIN batch_tracks t ON t.batch_id = b.id
         GROUP BY b.id ORDER BY b.ts DESC, b.id DESC",
    )?;
    let rows = st.query_map([], |r| {
        let n = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0) as usize);
        Ok(Batch {
            id: r.get(0)?,
            dir: r.get(1)?,
            ts: r.get(2)?,
            name: r.get(3)?,
            xml_ts: r.get(4)?,
            counts: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
            tracks: n(6)?,
            played: n(7)?,
            corrected: n(8)?,
            untouched: n(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A track of a batch, for its list.
#[derive(Serialize)]
pub struct BatchTrack {
    pub path: String,
    pub what: String,
    pub colour_before: Option<i64>,
    pub stars_before: Option<i64>,
    pub colour: Option<i64>,
    pub stars: Option<i64>,
    pub key: Option<String>,
    pub energy: Option<i64>,
    pub cues: Option<i64>,
    pub fate: Option<String>,
}

pub fn batch_tracks(root: &Path, id: i64) -> Res<Vec<BatchTrack>> {
    let con = store::labels_db(root)?;
    let mut st = con.prepare(
        "SELECT k.path, t.what, t.colour_before, t.stars_before, t.colour, t.stars, t.key, t.energy, t.cues, t.fate
         FROM batch_tracks t JOIN tracks k ON k.id = t.track_id WHERE t.batch_id = ?1 ORDER BY k.path",
    )?;
    let rows = st.query_map([id], |r| {
        Ok(BatchTrack {
            path: r.get(0)?,
            what: r.get(1)?,
            colour_before: r.get(2)?,
            stars_before: r.get(3)?,
            colour: r.get(4)?,
            stars: r.get(5)?,
            key: r.get(6)?,
            energy: r.get(7)?,
            cues: r.get(8)?,
            fate: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// --- stats -------------------------------------------------------------------------

#[derive(Serialize, Default)]
pub struct Stats {
    pub labels: usize,
    pub now: Score,
    /// (labels trained on, colour right) on the same holdout; the last one measured, which
    /// may be from fewer labels (see `fresh`), or empty before the first
    pub curve: Vec<(usize, f64)>,
    /// whether `now` and `curve` are measured on your labels as they are
    pub fresh: bool,
    /// tracks you labelled twice, and how often the colour stayed
    pub again: usize,
    pub agreed: usize,
}

/// What the stats page shows, from what is cached: quick. The learning curve trains a model
/// per point (seconds), so it is measured by `curve` and kept in the cache by your labels.
pub fn stats(root: &Path, cfg: &Config) -> Res<Stats> {
    let l = labelled(root)?;
    let (kind, key) = model_key(root, cfg, &l)?;
    let (ms, stale) = model(root, cfg, &l, true)?;
    let mut s = Stats { labels: l.y.len(), now: ms.1.clone(), ..Default::default() };
    let cache = store::cache_db(root)?;
    curves_table(&cache)?;
    let kept: Option<(String, String)> = cache
        .query_row("SELECT key, data FROM curves WHERE substr(key, 1, ?2) = ?1", rusqlite::params![format!("{kind}#"), (kind.len() + 1) as i64], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    if let Some((k, d)) = kept {
        s.curve = serde_json::from_str(&d)?;
        s.fresh = k == key && !stale;
    }
    let con = store::labels_db(root)?;
    let mut st = con.prepare("SELECT track_id, colour_id FROM labels WHERE source = 'user' AND colour_id IS NOT NULL ORDER BY id")?;
    let mut last: HashMap<i64, Vec<i64>> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (t, c) = r?;
        last.entry(t).or_default().push(c);
    }
    for v in last.values().filter(|v| v.len() >= 2) {
        s.again += 1;
        s.agreed += (v[v.len() - 1] == v[v.len() - 2]) as usize;
    }
    Ok(s)
}

/// Measure the learning curve on your labels as they are, unless the cache has it, then
/// the stats with it.
pub fn curve(root: &Path, cfg: &Config) -> Res<Stats> {
    let s = stats(root, cfg)?;
    if s.fresh {
        return Ok(s);
    }
    let l = labelled(root)?;
    let (_, key) = model_key(root, cfg, &l)?;
    let ax = axes(cfg);
    let n_train = l.hashes.iter().filter(|h| evaluate::part(h) == "train").count();
    let (ms, _) = model(root, cfg, &l, false)?;
    let mut curve = vec![];
    let mut k = 25;
    while k < n_train {
        if crate::stopping() {
            return Err("stopped".into());
        }
        let (_, sc) = fit(&l, &ax, k);
        if sc.colour_n > 0 {
            curve.push((k, (sc.colour[0] + sc.colour[1]) / 2.0));
        }
        k *= 2;
    }
    if ms.1.colour_n > 0 {
        curve.push((n_train, (ms.1.colour[0] + ms.1.colour[1]) / 2.0));
    }
    let cache = store::cache_db(root)?;
    curves_table(&cache)?;
    cache.execute("DELETE FROM curves", [])?;
    cache.execute("INSERT INTO curves (key, data, ts) VALUES (?1, ?2, ?3)", rusqlite::params![key, serde_json::to_string(&curve)?, now()])?;
    Ok(Stats { now: ms.1.clone(), curve, fresh: true, ..s })
}

fn curves_table(cache: &rusqlite::Connection) -> Res<()> {
    cache.execute_batch("CREATE TABLE IF NOT EXISTS curves (key TEXT PRIMARY KEY, data TEXT NOT NULL, ts INTEGER NOT NULL)")?;
    Ok(())
}

// --- one track -----------------------------------------------------------------------

/// The track as loudness bins: drawn as soon as it comes, before its parts.
#[derive(Serialize)]
pub struct Wave {
    pub duration: f64,
    /// loudness per bin, 0..1
    pub peaks: Vec<f32>,
}

/// What the waveform shows over the peaks, and what an export would write.
#[derive(Serialize)]
pub struct Parts {
    /// where each part starts, seconds, and its name
    pub sections: Vec<(f64, &'static str)>,
    /// what an export would write: your correction, else the analysis
    pub key: String,
    pub energy: i64,
    /// the cue points: what an export would write, or yours when the track has
    /// cues of your own (export never touches those)
    pub cues: Vec<Cue>,
    pub own_cues: bool,
}

#[derive(Serialize)]
pub struct Cue {
    pub start: f64,
    /// loops only
    pub end: Option<f64>,
    /// hot cue slot 0..7 (A..H), -1 for a memory cue
    pub num: i64,
    /// "#rrggbb" as rekordbox shows it; "" for none
    pub colour: String,
}

fn cue_of(m: &cues::Mark) -> Option<Cue> {
    let g = |k: &str| m.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let rgb = |k: &str| g(k).and_then(|v| v.parse::<u8>().ok());
    Some(Cue {
        start: g("Start")?.parse().ok()?,
        end: g("End").and_then(|v| v.parse().ok()),
        num: g("Num").and_then(|v| v.parse().ok()).unwrap_or(-1),
        colour: match (rgb("Red"), rgb("Green"), rgb("Blue")) {
            (Some(r), Some(gr), Some(b)) => format!("#{r:02x}{gr:02x}{b:02x}"),
            _ => String::new(),
        },
    })
}

/// The audio `wave` last decoded, for `parts` of the same track: the player asks for one
/// right after the other, so a track new to the analysis is decoded once, not three times.
// the cue features and key analysis share one decode, so they must read the same rate
const _: () = assert!(cues::SR == keyenergy::RATE);
static DECODED: Mutex<Option<(String, Arc<audio::Decoded>)>> = Mutex::new(None);

/// The track as `bins` loudness values, kept in the cache by audio hash.
pub fn wave(root: &Path, path: &str, bins: usize) -> Res<Wave> {
    let cache = store::cache_db(root)?;
    let hash = store::hash_of(&store::labels_db(root)?, path)?;
    if let Some(h) = &hash
        && let Ok((duration, data)) = cache.query_row(
            "SELECT duration, data FROM peaks WHERE hash=?1 AND bins=?2",
            rusqlite::params![h, bins as i64],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
    {
        let peaks = data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        return Ok(Wave { duration, peaks });
    }
    let d = Arc::new(audio::decode_native(&root.join(path))?);
    *DECODED.lock().unwrap() = Some((path.to_string(), d.clone()));
    let duration = d.samples.len() as f64 / d.rate as f64;
    let per = (d.samples.len() / bins.max(1)).max(1);
    let mut peaks: Vec<f32> =
        d.samples.chunks(per).map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let top = peaks.iter().cloned().fold(1e-6, f32::max);
    peaks.iter_mut().for_each(|p| *p /= top);
    if let Some(h) = &hash {
        let data: Vec<u8> = peaks.iter().flat_map(|v| v.to_le_bytes()).collect();
        cache.execute(
            "INSERT OR REPLACE INTO peaks VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![h, bins as i64, duration, data, now()],
        )?;
    }
    Ok(Wave { duration, peaks })
}

/// The track's parts: where the cue model puts the mix-in, two changes and the mix-out,
/// when rekordbox has a beat grid; its cues, key and energy. Analysis the cache lacks is
/// made from one decode, shared.
pub fn parts(root: &Path, xml_path: &Path, cfg: &Config, path: &str) -> Res<Parts> {
    let kept = DECODED.lock().unwrap().take().filter(|(p, _)| p == path).map(|(_, d)| d);
    let mut slow: Option<audio::Decoded> = None;
    let cache = store::cache_db(root)?;
    let labels = store::labels_db(root)?;
    let hash = store::hash_of(&labels, path)?;
    let x = xml(root, xml_path)?;
    let t = x.get(path);
    let mut marks: Vec<cues::Mark> = t.map(|t| t.marks.clone()).unwrap_or_default();
    let own_cues = t.is_some_and(|t| !cues::writable(t));
    let mut sections = vec![(0.0, "start")];
    if let (Some(t), Some(h)) = (t.filter(|t| !t.tempos.is_empty()), &hash) {
        let f = match cues::cached_features(&cache, h, t) {
            Some(f) => Some(f),
            None => {
                let f = cues::features_of(audio22(root, path, &kept, &mut slow)?, t);
                if let Some(f) = &f {
                    cues::keep_features(&cache, h, t, f)?;
                }
                f
            }
        };
        if let Some(f) = f {
            let down = cues::downbeats(&t.tempos, t.duration + 2.0);
            let names = ["mix in", "change", "change", "mix out"];
            let trans = cues::Model::load(root)?.transitions(&f);
            if !own_cues && cfg.bool("cues", "write", true) {
                let lead = cfg.ints("cues", "lead", &[0]);
                marks = cues::place(&trans, &down, &lead, &cfg.int_map("cues", "loops"), cfg.bool("cues", "memory", true));
            }
            for (b, name) in trans.iter().zip(names) {
                let s = cues::bar_time(&down, *b);
                if s > sections.last().unwrap().0 + 1.0 && s < t.duration {
                    sections.push((s, name));
                }
            }
        }
    }
    let (mut key, mut energy) = Default::default();
    if let Some(h) = &hash {
        let k = match keyenergy::cached(&cache, h) {
            Some(k) => Some(k),
            None => {
                match keyenergy::analyse(audio22(root, path, &kept, &mut slow)?) {
                    Ok((k, detail)) => {
                        keyenergy::keep(&cache, h, &k, &detail)?;
                        Some(k)
                    }
                    Err(_) => None,
                }
            }
        };
        if let Some(k) = k {
            (key, energy) = (k.key, k.energy as i64);
        }
    }
    if let Some((k, e)) = store::key_energy(&labels)?.remove(path) {
        key = k.unwrap_or(key);
        energy = e.map(i64::from).unwrap_or(energy);
    }
    let cues = marks.iter().filter_map(cue_of).collect();
    Ok(Parts { sections, key, energy, cues, own_cues })
}

/// The track at the analysis rate, decoded (or taken from `kept`) the first time it is asked for.
fn audio22<'a>(
    root: &Path,
    path: &str,
    kept: &Option<Arc<audio::Decoded>>,
    slot: &'a mut Option<audio::Decoded>,
) -> Res<&'a audio::Decoded> {
    if slot.is_none() {
        let d = match kept {
            Some(d) => d.clone(),
            None => Arc::new(audio::decode_native(&root.join(path))?),
        };
        let samples = audio::resample(&d.samples, d.rate, cues::SR)?;
        *slot = Some(audio::Decoded { samples, rate: cues::SR, duration: d.duration });
    }
    Ok(slot.as_ref().unwrap())
}

// --- settings --------------------------------------------------------------------------

/// The settings the app shows, as the config has them now.
pub fn settings(cfg: &Config) -> serde_json::Value {
    serde_json::json!({
        "write_tonality": cfg.bool("rekordbox", "write_tonality", true),
        "write_comment": cfg.bool("rekordbox", "write_comment", true),
        "cues_write": cfg.bool("cues", "write", true),
        "cues_lead": cfg.ints("cues", "lead", &[0]),
        "cues_loops": cfg.int_map("cues", "loops").into_iter().collect::<HashMap<_, _>>(),
        "cues_memory": cfg.bool("cues", "memory", true),
    })
}

/// Set one key in `.vibecheck/config.toml`, keeping everything else in the file,
/// comments included.
pub fn set(root: &Path, section: &str, key: &str, value: &serde_json::Value) -> Res<()> {
    let v = toml_value(value).ok_or("unsupported value")?;
    let p = store::vibecheck_dir(root)?.join("config.toml");
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let out = set_in(&text, section, key, &v.to_string());
    out.parse::<toml::Table>().map_err(|e| format!("config.toml would not parse: {e}"))?;
    std::fs::write(p, out)?;
    Ok(())
}

fn toml_value(v: &serde_json::Value) -> Option<toml::Value> {
    use serde_json::Value as J;
    Some(match v {
        J::Bool(b) => toml::Value::Boolean(*b),
        J::Number(n) if n.is_i64() => toml::Value::Integer(n.as_i64()?),
        J::Number(n) => match n.as_f64()? {
            f if f.fract() == 0.0 && f.abs() < 1e15 => toml::Value::Integer(f as i64),
            f => toml::Value::Float(f),
        },
        J::String(s) => toml::Value::String(s.clone()),
        J::Array(a) => toml::Value::Array(a.iter().map(toml_value).collect::<Option<_>>()?),
        J::Object(o) => toml::Value::Table(o.iter().map(|(k, v)| Some((k.clone(), toml_value(v)?))).collect::<Option<_>>()?),
        J::Null => return None,
    })
}

/// The text with `key = value` in `[section]`: replaced where it is, else added
/// at the end of the section, else in a new section at the end.
fn set_in(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let header = |l: &str| l.trim().strip_prefix('[').and_then(|r| r.split(']').next()).map(|s| s.trim().to_string());
    let line = format!("{key} = {value}");
    let start = lines.iter().position(|l| header(l).as_deref() == Some(section));
    match start {
        Some(s) => {
            let end = (s + 1..lines.len()).find(|&i| header(&lines[i]).is_some()).unwrap_or(lines.len());
            let at = (s + 1..end).find(|&i| {
                let t = lines[i].trim_start();
                t.strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('='))
            });
            match at {
                Some(i) => lines[i] = line,
                None => {
                    // after the section's last non-blank line
                    let mut i = end;
                    while i > s + 1 && lines[i - 1].trim().is_empty() {
                        i -= 1;
                    }
                    lines.insert(i, line);
                }
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(line);
        }
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_files_become_library_paths() {
        let root = Path::new("/nowhere/Music/Collection");
        let known = ["Mo\u{308}we/01 Tide.mp3".to_string(), "A/b.mp3".to_string()];
        let got = match_dropped(root, &[
            "/nowhere/Music/Collection/M\u{f6}we/01 Tide.mp3".into(),
            "/nowhere/Music/Collection/Mo\u{308}we/01 Tide.mp3".into(),
            "/nowhere/Downloads/A/b.mp3".into(),
            "/nowhere/Music/Collection/A/c.mp3".into(),
        ], &known);
        assert_eq!(got, vec![Some(known[0].clone()), Some(known[0].clone()), None, None]);
    }

    #[test]
    fn a_key_is_set_in_place_and_comments_stay() {
        let t = "# mine\n\n[cues]\n# pair\nlead = [16, 8]\nloops = { A = 16 }\n\n[x]\ny = 1\n";
        let o = set_in(t, "cues", "lead", "[0]");
        assert_eq!(o, "# mine\n\n[cues]\n# pair\nlead = [0]\nloops = { A = 16 }\n\n[x]\ny = 1\n");
        let o = set_in(t, "cues", "memory", "false");
        assert!(o.contains("loops = { A = 16 }\nmemory = false\n\n[x]"));
        let o = set_in(t, "rekordbox", "write_comment", "false");
        assert!(o.ends_with("y = 1\n\n[rekordbox]\nwrite_comment = false\n"));
        assert!(set_in("", "a", "b", "1").starts_with("[a]\nb = 1"));
    }

    #[test]
    fn keys_are_alphanumeric() {
        assert!(is_key("8A") && is_key("12B") && is_key("1A"));
        assert!(!is_key("13A") && !is_key("0B") && !is_key("8C") && !is_key("") && !is_key("A"));
    }

    #[test]
    fn energy_is_read_from_the_comment() {
        assert_eq!(energy_in("8A - Energy 6 /* x */"), Some(6));
        assert_eq!(energy_in("12B - Energy 10"), Some(10));
        assert_eq!(energy_in("nothing"), None);
    }

    #[test]
    fn the_interval_holds_the_rate() {
        let [lo, hi] = wilson(56, 100);
        assert!(lo < 0.56 && hi > 0.56 && hi - lo < 0.21);
        assert_eq!(wilson(0, 0), [0.0, 1.0]);
    }

    #[test]
    fn spread_starts_far_from_what_is_known() {
        let pool = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.7071, 0.7071]];
        assert_eq!(spread(&pool, &[vec![1.0, 0.0]], 2), vec![1, 2]);
    }
}
