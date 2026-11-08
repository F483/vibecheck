//! label, sync, discard, analyse, cues: the prototype's loop, command for
//! command.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use vibecheck::predict::{self, Prediction};
use vibecheck::rekordbox::{self, Entry, WriteOpts};
use vibecheck::store::{self, Label};
use vibecheck::workflow::{self, Corrections, CuePlan, log_cues, plan_cues};
use vibecheck::{Res, config::Config, cues, embed, evaluate, keyenergy, now, playlist};

use crate::Args;

fn local(ts: i64, fmt: &str) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map(|d| d.format(fmt).to_string()).unwrap_or_default()
}

fn xml_path(args: &Args) -> Option<PathBuf> {
    if args.flag("--no-rekordbox") {
        return None;
    }
    let p = args.opts.get("--rekordbox").cloned().unwrap_or("~/Documents/rekordbox.xml".into());
    let home = std::env::var("HOME").unwrap_or_default();
    Some(PathBuf::from(p.replacen('~', &home, 1)))
}

fn axes(cfg: &Config) -> Vec<predict::Axis> {
    let mc = cfg.float("predict", "misleading_cost", 1.0);
    predict::axes_for(mc, cfg.float("predict", "rating_misleading_cost", mc))
}

fn want_key_energy(cfg: &Config) -> bool {
    cfg.bool("rekordbox", "write_tonality", true) || cfg.bool("rekordbox", "write_comment", true)
}

/// The tracks a playlist names, as stored in the index.
fn scope_of(root: &Path, pl: Option<&String>) -> Res<Option<HashSet<String>>> {
    let Some(p) = pl else { return Ok(None) };
    let canon = store::path_index(&store::labels_db(root)?)?;
    Ok(Some(playlist::read(Path::new(p), root)?.into_iter().map(|x| canon.get(&store::norm(&x)).cloned().unwrap_or(x)).collect()))
}

fn read_corrections(root: &Path, xml: &Path, pl: Option<&String>, adopt_all: bool, stale_ok: bool) -> Res<Corrections> {
    workflow::read_corrections(root, xml, scope_of(root, pl)?, adopt_all, stale_ok)
}

/// Tracks named by a playlist, or all of `live`.
fn chosen(root: &Path, con: &rusqlite::Connection, input: Option<&String>, live: &[String]) -> Res<Vec<String>> {
    let Some(pl) = input else { return Ok(live.to_vec()) };
    let canon = store::path_index(con)?;
    let want: HashSet<String> =
        playlist::read(Path::new(pl), root)?.into_iter().map(|r| canon.get(&store::norm(&r)).cloned().unwrap_or(r)).collect();
    Ok(live.iter().filter(|p| want.contains(*p)).cloned().collect())
}

fn report(n: &Corrections) {
    println!(
        "      {} new, {} corrected, {} confirmed{}",
        n.added,
        n.changed,
        n.confirmed,
        if n.missing > 0 { format!(", {} not in the collection", n.missing) } else { String::new() }
    );
    if n.newer > 0 {
        println!("      left {} alone: labelled after this export was made", n.newer);
    }
    if n.unknown > 0 {
        println!("      ignored {} tracks carrying a colour this app has no record of (sync --all to adopt them)", n.unknown);
    }
}

pub fn sync(root: &Path, args: &Args) -> Res<()> {
    let xml = xml_path(args).ok_or("sync needs the rekordbox xml")?;
    let n = read_corrections(root, &xml, args.pos.first(), args.flag("--all"), args.flag("--stale-ok")).map_err(|e| {
        let s = e.to_string();
        if s.starts_with("STALE") {
            format!(
                "{}.\n  It cannot contain your corrections. Export a fresh one from\n  rekordbox first, or pass --stale-ok if you really mean it.",
                &s[7..]
            )
        } else {
            format!("{s}\n  export one after correcting (File > Export Collection in xml format).")
        }
    })?;
    println!("read {}: {} tracks carry a colour or rating", xml.file_name().unwrap().to_string_lossy(), n.read);
    report(&n);
    println!("labelled now: {}", store::current(&store::labels_db(root)?, Some("user"))?.len());
    Ok(())
}

pub fn discard(root: &Path, args: &Args) -> Res<()> {
    let con = store::labels_db(root)?;
    let ids = store::track_ids(&con)?;
    let confirmed: HashSet<String> = store::current(&con, Some("user"))?.into_keys().collect();
    let rows: HashMap<String, Option<i64>> = {
        let mut st = con.prepare("SELECT path, round_id FROM current WHERE source = 'model'")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    let rels: Vec<String> = match args.pos.first() {
        Some(p) => {
            let canon = store::path_index(&con)?;
            playlist::read(Path::new(p), root)?.into_iter().map(|r| canon.get(&store::norm(&r)).cloned().unwrap_or(r)).collect()
        }
        None => rows.keys().cloned().collect(),
    };
    let dry = args.flag("--dry-run");
    let mut cleared = 0;
    con.execute_batch("BEGIN")?;
    for rel in rels {
        if confirmed.contains(&rel) || !rows.contains_key(&rel) {
            continue;
        }
        if !dry {
            store::log_label(&con, ids[&rel], Label::NONE, "model", rows[&rel])?;
        }
        cleared += 1;
    }
    con.execute_batch("COMMIT")?;
    println!("discarded {cleared} unconfirmed labels");
    if cleared > 0 && !dry {
        println!("   what was already imported into rekordbox is rekordbox's;\n   select the batch playlist there to clear it");
    }
    if dry {
        println!("(dry run -- nothing changed)");
    }
    Ok(())
}

// --- cues --------------------------------------------------------------------

/// Learning from your own cues is not ported yet; say when it would have run.
fn cue_learning_note(root: &Path, cfg: &Config, xml: &HashMap<String, rekordbox::Track>) -> Res<()> {
    let written = store::cues_written(&store::labels_db(root)?)?;
    let own = cues::yours(xml, &written, &cfg.ints("cues", "lead", &[0]));
    let model = cues::Model::load(root)?;
    if own.len() >= model.n_tracks + cfg.int("cues", "retrain_after", 50) as usize {
        println!(
            "      note: {} tracks carry your cues, the model in use knew {}; learning from them is not in\n      this version yet (the prototype's `vibecheck cues --train`, then scripts/export_cue_model.py)",
            own.len(),
            model.n_tracks
        );
    }
    Ok(())
}

// --- label -------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

pub fn label(root: &Path, cfg: &Config, args: &Args) -> Res<()> {
    let count: usize = args.pos.first().map(|v| v.parse()).transpose()?.unwrap_or(300);
    let dry = args.flag("--dry-run");
    let xml = xml_path(args);
    let syncing = !args.flag("--no-sync") && !dry && xml.is_some();
    let with_ke = want_key_energy(cfg);
    let with_cues = cfg.bool("cues", "write", true) && xml.is_some() && !dry;
    let steps = 4 + syncing as usize + with_ke as usize + with_cues as usize;
    let mut done = 0;
    let mut step = |msg: &str| {
        done += 1;
        println!("[{done}/{steps}] {msg}");
    };

    if syncing {
        step("reading your corrections");
        let x = xml.as_ref().unwrap();
        match read_corrections(root, x, None, false, false) {
            Ok(n) => report(&n),
            Err(e) if e.to_string().starts_with("STALE") => {
                println!("      SKIPPED: {}.", &e.to_string()[7..]);
                println!("      Export a fresh xml from rekordbox and run again if you meant to include corrections.");
            }
            Err(e) if e.to_string().starts_with("no rekordbox export") => println!("      no rekordbox export found; skipping"),
            Err(e) => return Err(e),
        }
    }

    let con = store::labels_db(root)?;
    let ids = store::track_ids(&con)?;
    let confirmed: HashSet<String> = store::current(&con, Some("user"))?.into_keys().collect();
    let spoken: HashSet<String> = store::current(&con, None)?.into_keys().collect();
    let taken = if args.flag("--include-unconfirmed") { &confirmed } else { &spoken };
    let pool: Vec<String> = store::live_paths(&con)?.into_iter().filter(|p| !taken.contains(p)).collect();
    if pool.is_empty() {
        return Err("nothing left unlabelled".into());
    }
    let out_now = spoken.len() - confirmed.len();
    if out_now > 0 && !args.flag("--include-unconfirmed") {
        println!(
            "note: {out_now} tracks are out in a batch awaiting your corrections and are excluded (--include-unconfirmed to reuse)"
        );
    } else if out_now > 0 {
        println!("note: including {out_now} tracks that already carry an unconfirmed label");
    }
    let seed = args.opts.get("--seed").and_then(|s| s.parse().ok()).unwrap_or(now() as u64);
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut idx: Vec<usize> = (0..pool.len()).collect();
    let k = count.min(pool.len());
    for i in 0..k {
        let j = i + rng.below(pool.len() - i);
        idx.swap(i, j);
    }
    let mut batch: Vec<String> = idx[..k].iter().map(|&i| pool[i].clone()).collect();
    batch.sort();
    // One name for the round, everywhere: playlist, rekordbox, database.
    let out = args.opts.get("--output").map(PathBuf::from).unwrap_or_else(|| {
        store::vibecheck_dir(root)
            .unwrap_or_default()
            .join("playlists")
            .join(format!("vibecheck-{}.m3u8", local(now(), "%Y%m%d-%H%M")))
    });
    let name = out.file_stem().unwrap().to_string_lossy().to_string();
    step(&format!("selected {} of {} unlabelled tracks as {name}", batch.len(), pool.len()));

    step("listening to the tracks that are new to it (about 5 seconds each)");
    let st = embed::embed_all(root, &batch)?;
    println!(
        "      {} embedded, {} already known{}",
        st.done,
        st.cached,
        if st.failed > 0 { format!(", {} failed", st.failed) } else { String::new() }
    );

    let mut ke = HashMap::new();
    if with_ke {
        step("working out key and energy");
        let (k, st) = keyenergy::analyse_all(root, &batch)?;
        println!(
            "      {} analysed, {} already known{}",
            st.done,
            st.cached,
            if st.failed > 0 { format!(", {} failed", st.failed) } else { String::new() }
        );
        ke = k;
    }

    let mut plan: Option<CuePlan> = None;
    if with_cues {
        step("placing cue points");
        let x = xml.as_ref().unwrap();
        if !x.exists() {
            println!("      no rekordbox export found; skipping");
        } else {
            let tracks = rekordbox::read(x, root)?;
            cue_learning_note(root, cfg, &tracks)?;
            let p = plan_cues(root, cfg, &batch, &tracks)?;
            println!(
                "      {} tracks get cues{}{}",
                p.marks.len(),
                if p.skipped > 0 { format!(", {} keep the cues they have", p.skipped) } else { String::new() },
                if p.failed > 0 { format!(", {} failed", p.failed) } else { String::new() }
            );
            plan = Some(p);
        }
    }

    step("training on what you have labelled so far");
    let lab = store::current(&con, Some("user"))?;
    if lab.len() < 20 {
        return Err(format!("only {} labels; label some tracks first", lab.len()).into());
    }
    let mut keys: Vec<String> = lab.keys().cloned().collect();
    keys.sort();
    let (paths, x) = embed::load(root, &keys)?;
    let xr: Vec<&[f32]> = x.iter().map(|v| v.as_slice()).collect();
    let y: Vec<Label> = paths.iter().map(|p| lab[p]).collect();
    let ax = axes(cfg);
    let mut model = predict::Model::new(ax.clone());
    let info = model.train(&xr, &y);
    let hb = store::hash_by_path(&con)?;
    let hashes: Vec<&str> = paths.iter().map(|p| hb[p].as_str()).collect();
    let n_train = hashes.iter().filter(|h| evaluate::part(h) == "train").count();
    let n_labels = lab.values().map(|l| (l.colour.map(|c| c.id), l.stars)).collect::<HashSet<_>>().len();
    let scores = evaluate::score_axes(&xr, &y, &hashes, &ax);
    println!("      {} tracks, {n_labels} distinct labels, axes: {}", info.tracks, info.axes.join(", "));
    for s in &scores {
        println!(
            "      {:7} speaks on {:3.0}% at {:3.0}%, leaving {:.2} of {:.2} decisions   (holdout n={})",
            s.axis,
            s.coverage() * 100.0,
            s.accuracy() * 100.0,
            s.cost_left,
            s.cost,
            s.n
        );
    }
    if scores.is_empty() {
        println!("      not scored: no labelled track falls in the measurement slice yet, so this round goes unrecorded");
    }

    let encoder = embed::fingerprint();
    let rid = if dry { None } else { Some(store::open_round(&con, &name, batch.len(), embed::BACKEND, &encoder, info.tracks)?) };

    step("deciding what it can say about each track");
    let (paths, x) = embed::load(root, &batch)?;
    let xr: Vec<&[f32]> = x.iter().map(|v| v.as_slice()).collect();
    let preds: Vec<Prediction> = model.predict(&xr);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rated = 0;
    let mut entries = vec![];
    con.execute_batch("BEGIN")?;
    for (rel, p) in paths.iter().zip(&preds) {
        let l = p.label();
        *counts.entry(p.specificity()).or_default() += 1;
        rated += l.stars.is_some() as usize;
        if l.said() && !dry {
            store::log_label(&con, ids[rel], l, "model", rid)?;
        }
        let mut shown = format!(
            "{}{}",
            l.colour.map_or("?", |c| c.name),
            l.stars.filter(|s| *s > 0).map(|s| format!(" {}", "*".repeat(s as usize))).unwrap_or_default()
        );
        if let Some(k) = ke.get(rel) {
            shown += &format!("  {} - Energy {}", k.key, k.energy);
        }
        let file = Path::new(rel).file_name().unwrap().to_string_lossy();
        entries.push((rel.clone(), format!("{shown:30} | {file}")));
    }
    if let Some(r) = rid {
        let rows: Vec<(i64, &Prediction)> = paths.iter().zip(&preds).map(|(rel, p)| (ids[rel], p)).collect();
        store::log_predictions(&con, r, &rows)?;
        if !scores.is_empty() {
            store::log_fit(&con, Some(r), embed::BACKEND, &encoder, n_train, n_labels, &scores)?;
        }
    }
    con.execute_batch("COMMIT")?;

    if let (Some(src), false) = (&xml, dry) {
        if !src.exists() {
            return Err(format!(
                "no rekordbox export at {}\n  export one from rekordbox (File > Export Collection in xml format),\n  or pass --rekordbox <path>.",
                src.display()
            )
            .into());
        }
        let in_place = !args.flag("--separate-file");
        let dst = if in_place { src.clone() } else { out.with_extension("xml") };
        let rk: Vec<Entry> = paths
            .iter()
            .zip(&preds)
            .map(|(rel, p)| Entry {
                rel: rel.clone(),
                level: p.specificity(),
                label: Some(p.label()),
                key: ke.get(rel).map(|k| k.key.clone()),
                energy: ke.get(rel).map(|k| k.energy),
                cues: plan.as_ref().and_then(|c| c.marks.get(rel)).map(|m| m.0.clone()),
            })
            .collect();
        let o = WriteOpts {
            name: &name,
            tonality: cfg.bool("rekordbox", "write_tonality", true),
            comment: cfg.bool("rekordbox", "write_comment", true),
            changed_only: false,
        };
        let st = rekordbox::write(src, &dst, root, &rk, &o)?;
        println!("\nwrote {}", dst.display());
        if let Some(p) = &plan
            && !p.marks.is_empty()
        {
            log_cues(root, p, rid)?;
        }
        println!(
            "   {} colours set, {} ratings set{}{}{}",
            st.coloured,
            st.rated,
            if with_ke { format!(", {} keys, {} energy comments", st.keyed, st.commented) } else { String::new() },
            if plan.as_ref().is_some_and(|p| !p.marks.is_empty()) { format!(", cues on {} tracks", st.cued) } else { String::new() },
            if st.not_in_xml > 0 { format!(", {} not found in the export", st.not_in_xml) } else { String::new() }
        );
        if let Some(b) = &st.backup {
            println!("   previous export kept at {}", b.file_name().unwrap().to_string_lossy());
        }
        if in_place {
            println!("   refresh the rekordbox xml node in rekordbox's sidebar; the batch appears as the playlist {name}");
        } else {
            println!("   point rekordbox at this file, or drop --separate-file to update the one it already watches");
        }
    }

    let risky = playlist::write(&out, root, &entries)?;
    println!("\nwrote {}", out.display());
    if !risky.is_empty() {
        println!("   note: {} path(s) contain '#', which Rekordbox treats as a comment and will skip:", risky.len());
        for r in risky.iter().take(3) {
            println!("     {r}");
        }
    }
    let mut c: Vec<_> = counts.into_iter().collect();
    c.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, v) in c {
        println!("   {k:7} {v}");
    }
    println!("   stars   {rated} (independent of the colour)");
    println!("\ncorrect them in your DJ software, export it again, then run the same command again");
    Ok(())
}

// --- analyse and cues: one field, many tracks ----------------------------------------

fn finish_write(src: &Path, root: &Path, entries: &[Entry], name: &str, cfg: &Config, dry: bool) -> Res<rekordbox::WriteStats> {
    let o = WriteOpts {
        name,
        tonality: cfg.bool("rekordbox", "write_tonality", true),
        comment: cfg.bool("rekordbox", "write_comment", true),
        changed_only: true,
    };
    if dry {
        let tmp = std::env::temp_dir().join(format!("vibecheck-dry-{}.xml", std::process::id()));
        let st = rekordbox::write(src, &tmp, root, entries, &o)?;
        let _ = std::fs::remove_file(&tmp);
        return Ok(st);
    }
    rekordbox::write(src, src, root, entries, &o)
}

fn import_hint(src: &Path, name: &str) {
    let exported = rekordbox::exported(src).unwrap_or(0);
    println!(
        "wrote {}, previous kept as {}.bak\n   refresh the rekordbox xml node, then import the playlist {name}.\n   \
         Importing applies every field this export holds for those tracks,\n   so it should be fresh (this one was exported {}).",
        src.display(),
        src.file_name().unwrap().to_string_lossy(),
        local(exported, "%Y-%m-%d %H:%M")
    );
}

pub fn analyse(root: &Path, cfg: &Config, args: &Args) -> Res<()> {
    let con = store::labels_db(root)?;
    let live = store::live_paths(&con)?;
    if args.pos.is_empty() && !args.flag("--all") {
        return Err("choose the tracks: a playlist, or --all".into());
    }
    let rels = chosen(root, &con, args.pos.first(), &live)?;
    println!("key and energy for {} tracks", rels.len());
    let (ke, st) = keyenergy::analyse_all(root, &rels)?;
    println!(
        "   {} analysed, {} already known{}",
        st.done,
        st.cached,
        if st.failed > 0 { format!(", {} failed", st.failed) } else { String::new() }
    );
    let mut en = [0usize; 11];
    let mut keys: HashMap<&str, usize> = HashMap::new();
    for r in ke.values() {
        en[r.energy as usize] += 1;
        *keys.entry(&r.key).or_default() += 1;
    }
    println!(
        "   energy  {}",
        (1..=10).filter(|e| en[*e] > 0).map(|e| format!("{e}:{}", en[e])).collect::<Vec<_>>().join("  ")
    );
    let mut kv: Vec<_> = keys.into_iter().collect();
    kv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!(
        "   keys    {}{}",
        kv.iter().take(8).map(|(k, n)| format!("{k}:{n}")).collect::<Vec<_>>().join("  "),
        if kv.len() > 8 { "  ..." } else { "" }
    );
    let Some(src) = xml_path(args) else { return Ok(()) };
    if !src.exists() {
        return Err(format!("no rekordbox export at {}", src.display()).into());
    }
    let name = format!("vibecheck-analyse-{}", local(now(), "%Y%m%d-%H%M"));
    let mut rels: Vec<&String> = ke.keys().collect();
    rels.sort();
    let entries: Vec<Entry> = rels
        .iter()
        .map(|r| Entry { rel: (*r).clone(), level: "none", key: Some(ke[*r].key.clone()), energy: Some(ke[*r].energy), ..Default::default() })
        .collect();
    let dry = args.flag("--dry-run");
    let st = finish_write(&src, root, &entries, &name, cfg, dry)?;
    let changed: usize = st.playlists.values().sum();
    println!(
        "\n{} {changed} tracks ({} keys, {} comments); {} already up to date{}",
        if dry { "would change" } else { "changed" },
        st.keyed,
        st.commented,
        st.unchanged,
        if st.not_in_xml > 0 { format!(", {} not in the export", st.not_in_xml) } else { String::new() }
    );
    if dry {
        println!("(dry run -- nothing written)");
    } else if st.written {
        import_hint(&src, &name);
    } else {
        println!("nothing to change; the export was left alone");
    }
    Ok(())
}

pub fn cues(root: &Path, cfg: &Config, args: &Args) -> Res<()> {
    let src = xml_path(args).ok_or("cues needs the rekordbox xml")?;
    if !src.exists() {
        return Err(format!(
            "no rekordbox export at {}\n  export one from rekordbox (File > Export Collection in xml format).",
            src.display()
        )
        .into());
    }
    if args.flag("--train") {
        return Err("learning from your cues is not in this version yet: use the prototype's `vibecheck cues --train`,\n  \
                    then `uv run python -I scripts/export_cue_model.py <collection>/.vibecheck/cue_model.pt \
                    <collection>/.vibecheck/cue_model.bin`"
            .into());
    }
    println!("reading {}", src.display());
    let xml = rekordbox::read(&src, root)?;
    let con = store::labels_db(root)?;
    let live = store::live_paths(&con)?;
    let rels: Vec<String> = if args.pos.first().is_some() || args.flag("--all") {
        chosen(root, &con, args.pos.first(), &live)?
    } else {
        // every track this app has labelled: the ones you are working through
        let have: HashSet<String> = store::current(&con, None)?.into_keys().collect();
        live.into_iter().filter(|p| have.contains(p)).collect()
    };
    println!("cue points for {} tracks", rels.len());
    cue_learning_note(root, cfg, &xml)?;
    let plan = plan_cues(root, cfg, &rels, &xml)?;
    println!(
        "   {} placed, {} from the cache{}{}",
        plan.marks.len(),
        plan.cached,
        if plan.skipped > 0 { format!(", {} keep the cues they have", plan.skipped) } else { String::new() },
        if plan.failed > 0 { format!(", {} failed", plan.failed) } else { String::new() }
    );
    if plan.marks.is_empty() {
        println!("nothing to change; the export was left alone");
        return Ok(());
    }
    let name = format!("vibecheck-cues-{}", local(now(), "%Y%m%d-%H%M"));
    let mut rels: Vec<&String> = plan.marks.keys().collect();
    rels.sort();
    let entries: Vec<Entry> =
        rels.iter().map(|r| Entry { rel: (*r).clone(), level: "none", cues: Some(plan.marks[*r].0.clone()), ..Default::default() }).collect();
    let dry = args.flag("--dry-run");
    let st = finish_write(&src, root, &entries, &name, cfg, dry)?;
    if dry {
        println!("would set cues on {} tracks\n(dry run -- nothing written)", st.cued);
        return Ok(());
    }
    if !st.written {
        println!("nothing to change; the export was left alone");
        return Ok(());
    }
    log_cues(root, &plan, None)?;
    print!("\ncues set on {} tracks; ", st.cued);
    import_hint(&src, &name);
    Ok(())
}
