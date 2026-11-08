//! The steps the command line and the app share: reading corrections back from
//! the rekordbox xml, and placing cue points.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::config::Config;
use crate::rekordbox::{self, Track};
use crate::store::{self, Label};
use crate::{Res, cues};

fn local(ts: i64, fmt: &str) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map(|d| d.format(fmt).to_string()).unwrap_or_default()
}

// --- reading corrections -------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct Corrections {
    pub added: usize,
    pub changed: usize,
    pub confirmed: usize,
    pub missing: usize,
    pub unknown: usize,
    pub read: usize,
    /// labelled after the export was made: the export cannot know about them
    pub newer: usize,
    /// the tracks given a new label, added or corrected
    pub paths: Vec<String>,
}

/// Read what the user changed in rekordbox and record it. A value that differs
/// from the database is taken whatever the scope; one that matches a
/// prediction counts as confirmed only for tracks in `scope` (None: those out
/// in an open round); a colour this app has no record of is ignored unless in
/// scope or `adopt_all`. A track labelled after the export was made is left
/// alone: what the export holds for it is older than what we know.
pub fn read_corrections(
    root: &Path,
    xml: &Path,
    scope: Option<HashSet<String>>,
    adopt_all: bool,
    stale_ok: bool,
) -> Res<Corrections> {
    let con = store::labels_db(root)?;
    let ids = store::track_ids(&con)?;
    let last_user = store::current(&con, Some("user"))?;
    let last_any = store::current(&con, None)?;
    let canon = store::path_index(&con)?;
    let since: HashMap<String, i64> = {
        let mut st = con.prepare("SELECT path, ts FROM current WHERE label_id IS NOT NULL")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    let reviewed: HashSet<String> = match scope {
        Some(s) => s,
        None => {
            let mut st = con.prepare(
                "SELECT t.path FROM predictions p JOIN tracks t ON t.id = p.track_id
                 JOIN rounds r ON r.id = p.round_id WHERE r.closed IS NULL",
            )?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        }
    };
    if !xml.exists() {
        return Err(format!("no rekordbox export at {}", xml.display()).into());
    }
    let newest: Option<i64> = con.query_row("SELECT MAX(ts) FROM labels WHERE source='model'", [], |r| r.get(0))?;
    let exported = rekordbox::exported(xml).unwrap_or(0);
    if let (Some(n), false) = (newest, stale_ok)
        && exported < n
    {
        return Err(format!(
            "STALE: {} was exported {}, before the current batch was made ({})",
            xml.file_name().unwrap().to_string_lossy(),
            local(exported, "%Y-%m-%d %H:%M"),
            local(n, "%Y-%m-%d %H:%M")
        )
        .into());
    }
    let reading: HashMap<String, Label> = rekordbox::read(xml, root)?
        .into_iter()
        .filter_map(|(k, t)| Some((canon.get(&k).cloned().unwrap_or(k), t.label?)))
        .collect();
    let of_round = store::round_of(&con)?;
    let mut n = Corrections { read: reading.values().filter(|l| l.said()).count(), ..Default::default() };
    let mut touched: HashSet<i64> = HashSet::new();
    con.execute_batch("BEGIN")?;
    let mut keys: Vec<&String> = reading.keys().collect();
    keys.sort();
    for rel in keys {
        // rekordbox reports "no stars" as 0: a statement, like a blank colour
        let now_l = reading[rel];
        let Some(&tid) = ids.get(rel) else {
            n.missing += 1;
            continue;
        };
        let prev = last_any.get(rel).copied();
        let in_scope = adopt_all || reviewed.contains(rel);
        if !now_l.said() && prev.is_none() {
            continue;
        }
        let rid = of_round.get(rel).copied();
        if Some(now_l) == prev {
            // left as it was: confirms the command line's round; a guess the app exported stays a guess
            if in_scope && !last_user.contains_key(rel) && rid.is_some() {
                store::log_label(&con, tid, now_l, "user", rid)?;
                touched.extend(rid);
                n.confirmed += 1;
            }
            continue;
        }
        if since.get(rel).is_some_and(|t| *t > exported) {
            n.newer += 1;
            continue;
        }
        if prev.is_none() && !in_scope {
            n.unknown += 1;
            continue;
        }
        // only the axes the previous statement made can be contradicted
        let contradicts = prev.is_some_and(|p| {
            (p.colour.is_some() && p.colour != now_l.colour) || (p.stars.is_some() && p.stars != now_l.stars)
        });
        store::log_label(&con, tid, now_l, "user", rid)?;
        touched.extend(rid);
        if prev.is_none() {
            n.added += 1;
            n.paths.push(rel.clone());
        } else if contradicts {
            n.changed += 1;
            n.paths.push(rel.clone());
        } else {
            n.confirmed += 1;
        }
    }
    for rid in touched {
        store::close_round(&con, rid)?;
    }
    con.execute_batch("COMMIT")?;
    Ok(n)
}

// --- cues ----------------------------------------------------------------------

pub struct CuePlan {
    /// path -> (marks to write, transition bars)
    pub marks: HashMap<String, (Vec<cues::Mark>, Vec<usize>)>,
    pub skipped: usize,
    pub failed: usize,
    pub cached: usize,
    pub model: String,
}

pub fn plan_cues(root: &Path, cfg: &Config, rels: &[String], xml: &HashMap<String, Track>) -> Res<CuePlan> {
    let lead = cfg.ints("cues", "lead", &[0]);
    let loops = cfg.int_map("cues", "loops");
    let memory = cfg.bool("cues", "memory", true);
    let ok: HashMap<String, Track> = rels
        .iter()
        .filter_map(|r| xml.get(r).filter(|t| !t.tempos.is_empty() && cues::writable(t)).map(|t| (r.clone(), t.clone())))
        .collect();
    let model = cues::Model::load(root)?;
    let (f, st) = cues::features_all(root, &ok)?;
    let mut marks = HashMap::new();
    for (r, f) in &f {
        let t = model.transitions(f);
        let down = cues::downbeats(&ok[r].tempos, ok[r].duration + 2.0);
        marks.insert(r.clone(), (cues::place(&t, &down, &lead, &loops, memory), t));
    }
    Ok(CuePlan {
        marks,
        skipped: rels.iter().filter(|r| xml.contains_key(*r) && !ok.contains_key(*r)).count(),
        failed: st.failed,
        cached: st.cached,
        model: format!("{}/{}-{}", cues::VERSION, model.source, model.n_tracks),
    })
}

pub fn log_cues(root: &Path, plan: &CuePlan, round: Option<i64>) -> Res<()> {
    let con = store::labels_db(root)?;
    let ids = store::track_ids(&con)?;
    con.execute_batch("BEGIN")?;
    for (rel, (m, t)) in &plan.marks {
        store::log_cue_write(&con, ids[rel], round, &plan.model, t, &cues::hot_starts(m))?;
    }
    con.execute_batch("COMMIT")?;
    Ok(())
}
