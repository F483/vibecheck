//! The Rust core's timings on the same fixed tracks as the prototype's bench.
//! usage: bench <tracks.json>
use std::collections::HashMap;
use std::time::Instant;
use vibecheck::{config, cues, embed, keyenergy, predict, rekordbox, store};

fn main() -> vibecheck::Res<()> {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::env::args().nth(1).unwrap())?)?;
    let list = |k: &str| -> Vec<String> { j[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
    let home = std::path::PathBuf::from(std::env::var("HOME")?);
    let root = home.join("Music/Collection");
    let mut e = embed::Embedder::load()?;
    let em = list("embed");
    e.track(&root.join(&em[0]))?;
    let t = Instant::now();
    for r in &em[1..] {
        e.track(&root.join(r))?;
    }
    let embed_s = t.elapsed().as_secs_f64() / (em.len() - 1) as f64;
    let kel = list("ke");
    let t = Instant::now();
    keyenergy::parallel(&kel, |r| keyenergy::analyse_file(&root.join(r)).ok(), "");
    let ke_rate = kel.len() as f64 / t.elapsed().as_secs_f64();
    let xml = rekordbox::read(&home.join("Documents/rekordbox.xml"), &root)?;
    let cl = list("cue");
    let tr: HashMap<String, rekordbox::Track> = cl.iter().map(|r| (r.clone(), xml[r].clone())).collect();
    let t = Instant::now();
    keyenergy::parallel(&cl, |r| cues::track_features(&root.join(r), &tr[r]), "");
    let cue_rate = cl.len() as f64 / t.elapsed().as_secs_f64();
    let cfg = config::load(&root)?;
    let mc = cfg.float("predict", "misleading_cost", 1.0);
    let lab = store::current(&store::labels_db(&root)?, Some("user"))?;
    let mut keys: Vec<String> = lab.keys().cloned().collect();
    keys.sort();
    let (paths, x) = embed::load(&root, &keys)?;
    let t = Instant::now();
    let mut m = predict::Model::new(predict::axes_for(mc, cfg.float("predict", "rating_misleading_cost", mc)));
    m.train(&x.iter().map(|v| v.as_slice()).collect::<Vec<_>>(), &paths.iter().map(|p| lab[p]).collect::<Vec<_>>());
    let lr = t.elapsed().as_secs_f64();
    println!("{{\"embed_s_per_track\": {embed_s}, \"ke_tracks_per_s\": {ke_rate}, \"cue_tracks_per_s\": {cue_rate}, \"lr_fit_s\": {lr}}}");
    Ok(())
}
