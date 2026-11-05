//! Quality, not identity: transitions from Rust's features vs the prototype's
//! cached ones, both scored against the user's own hand-set cues.
//! usage: cue_quality [N]
use std::collections::HashMap;
use vibecheck::{config, cues, rekordbox, store};

fn main() -> vibecheck::Res<()> {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let home = std::path::PathBuf::from(std::env::var("HOME")?);
    let root = home.join("Music/Collection");
    let cfg = config::load(&root)?;
    let lead = cfg.ints("cues", "lead", &[0]);
    let xml = rekordbox::read(&home.join("Documents/rekordbox.xml"), &root)?;
    let written = store::cues_written(&store::labels_db(&root)?)?;
    let own = cues::yours(&xml, &written, &lead);
    let cache = store::cache_db(&root)?;
    let hb = store::hash_by_path(&store::labels_db(&root)?)?;
    let mut rels: Vec<String> = own.keys().cloned().collect();
    rels.sort();
    let rels: Vec<String> = rels.iter().filter(|r| hb.contains_key(*r)).step_by((rels.len() / n).max(1)).take(n).cloned().collect();
    let tracks: HashMap<String, rekordbox::Track> = rels.iter().map(|r| (r.clone(), xml[r].clone())).collect();
    let model = cues::Model::load(&root)?;
    let mine = vibecheck::keyenergy::parallel(&rels, |r| cues::track_features(&root.join(r), &tracks[r]), "");
    let score = |t: &[usize], truth: &[usize]| t.iter().filter(|x| truth.contains(x)).count();
    let (mut py_hit, mut rs_hit, mut total, mut both) = (0, 0, 0, 0);
    for (r, f) in rels.iter().zip(mine) {
        let (Some(f), Some(h)) = (f, hb.get(r)) else { continue };
        let py: Option<Vec<u8>> = cache
            .query_row("SELECT data FROM cue_bars WHERE hash=?1 AND version=?2 AND grid=?3",
                rusqlite::params![h, cues::VERSION, cues::grid_digest(&tracks[r].tempos)], |x| x.get(0))
            .ok();
        let truth = &own[r];
        let t_rs = model.transitions(&f);
        rs_hit += score(&t_rs, truth);
        total += truth.len();
        if let Some(b) = py {
            let v: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let fp: Vec<Vec<f32>> = v.chunks(77).map(|x| x.to_vec()).collect();
            py_hit += score(&model.transitions(&fp), truth);
            both += 1;
        }
    }
    println!("{} of your cued tracks: Rust features hit {rs_hit} of {total} of your transition points ({:.1}%)", rels.len(), 100.0 * rs_hit as f64 / total as f64);
    println!("prototype's cached features available for {both} of them, hitting {py_hit}");
    Ok(())
}
