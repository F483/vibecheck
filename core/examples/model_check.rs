//! A candidate audio model against the cached vectors: how close its vectors
//! are, whether the colour and rating decisions change, and how fast it runs.
//! usage: VIBECHECK_MODELS=<dir> model_check N
use std::time::Instant;
use vibecheck::{config, embed, predict, store};

fn main() -> vibecheck::Res<()> {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let root = std::path::PathBuf::from(std::env::var("HOME")?).join("Music/Collection");
    let cfg = config::load(&root)?;
    let mc = cfg.float("predict", "misleading_cost", 1.0);
    let lab = store::current(&store::labels_db(&root)?, Some("user"))?;
    let mut keys: Vec<String> = lab.keys().cloned().collect();
    keys.sort();
    let (paths, x) = embed::load(&root, &keys)?;
    let mut m = predict::Model::new(predict::axes_for(mc, cfg.float("predict", "rating_misleading_cost", mc)));
    m.train(&x.iter().map(|v| v.as_slice()).collect::<Vec<_>>(), &paths.iter().map(|p| lab[p]).collect::<Vec<_>>());
    // unlabelled tracks with cached vectors, spread through the collection
    let live: Vec<String> = store::live_paths(&store::labels_db(&root)?)?.into_iter().filter(|p| !lab.contains_key(p)).collect();
    let (cand, cv) = embed::load(&root, &live)?;
    let step = (cand.len() / n).max(1);
    let mut e = embed::Embedder::load()?;
    println!("model {}", embed::model_path()?.display());
    let (mut cos, mut same, mut axes_same, mut axes_total) = (vec![], 0, 0, 0);
    let t0 = Instant::now();
    for i in (0..cand.len()).step_by(step).take(n) {
        let v = e.track(&root.join(&cand[i]))?;
        let c = &cv[i];
        let dot: f32 = v.iter().zip(c).map(|(a, b)| a * b).sum();
        cos.push(dot / v.iter().map(|a| a * a).sum::<f32>().sqrt() / c.iter().map(|a| a * a).sum::<f32>().sqrt());
        let (a, b) = (&m.predict(&[v.as_slice()])[0], &m.predict(&[c.as_slice()])[0]);
        same += (a.values == b.values) as usize;
        for ax in ["hue", "tone", "stars"] {
            axes_total += 1;
            axes_same += (a.values.get(ax) == b.values.get(ax)) as usize;
        }
    }
    let secs = t0.elapsed().as_secs_f64() / cos.len() as f64;
    cos.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{} tracks: cosine min {:.6} median {:.6}; same decisions on {same} tracks, {axes_same}/{axes_total} axes; {secs:.2} s/track",
        cos.len(),
        cos[0],
        cos[cos.len() / 2]
    );
    Ok(())
}
