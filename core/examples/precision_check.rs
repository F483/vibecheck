//! Full against half-precision model on the same decoded audio, track by track.
//! Writes one JSON line per track, so a long run can be read while it goes.
//! usage: precision_check <fp32.onnx> <fp16.onnx> <N> <out.jsonl>
use std::io::Write;
use std::time::Instant;
use vibecheck::{audio, config, embed, predict, store};

fn cos(a: &[f32], b: &[f32]) -> f64 {
    let d: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    d / na / nb
}

fn main() -> vibecheck::Res<()> {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a[3].parse()?;
    let root = std::path::PathBuf::from(std::env::var("HOME")?).join("Music/Collection");
    let cfg = config::load(&root)?;
    let mc = cfg.float("predict", "misleading_cost", 1.0);
    let con = store::labels_db(&root)?;
    let lab = store::current(&con, Some("user"))?;
    let mut keys: Vec<String> = lab.keys().cloned().collect();
    keys.sort();
    let (paths, x) = embed::load(&root, &keys)?;
    let mut m = predict::Model::new(predict::axes_for(mc, cfg.float("predict", "rating_misleading_cost", mc)));
    m.train(&x.iter().map(|v| v.as_slice()).collect::<Vec<_>>(), &paths.iter().map(|p| lab[p]).collect::<Vec<_>>());
    // unlabelled tracks with a cached vector, evenly spread through the collection
    let live: Vec<String> = store::live_paths(&con)?.into_iter().filter(|p| !lab.contains_key(p)).collect();
    let (cand, cached) = embed::load(&root, &live)?;
    let step = (cand.len() / n).max(1);
    let mut e32 = embed::Embedder::load_from(std::path::Path::new(&a[1]))?;
    let mut e16 = embed::Embedder::load_from(std::path::Path::new(&a[2]))?;
    let mut out = std::fs::File::create(&a[4])?;
    let (mut t32, mut t16) = (0f64, 0f64);
    for (k, i) in (0..cand.len()).step_by(step).take(n).enumerate() {
        let d = audio::decode(&root.join(&cand[i]), embed::RATE)?;
        let ws = audio::excerpts(&d, embed::WINDOWS, embed::SECONDS);
        let pool = |e: &mut embed::Embedder| -> vibecheck::Res<Vec<f32>> {
            let mut v = e.windows(&ws)?;
            while v.len() < embed::WINDOWS {
                v.push(*v.last().unwrap());
            }
            Ok(embed::pool(&v))
        };
        let t = Instant::now();
        let v32 = pool(&mut e32)?;
        t32 += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let v16 = pool(&mut e16)?;
        t16 += t.elapsed().as_secs_f64();
        let p = m.predict(&[v32.as_slice(), v16.as_slice(), cached[i].as_slice()]);
        const AXES: [&str; 3] = ["hue", "tone", "stars"];
        let axis = |q: &predict::Prediction| {
            let said: Vec<Option<String>> = AXES.iter().map(|a| q.values.get(a).cloned()).collect();
            let top: Vec<Option<(String, f64)>> = AXES.iter().map(|a| q.top.get(a).cloned()).collect();
            serde_json::json!({
                "said": said,
                "top": top,
                "colour": q.colour().map(|c| c.name),
                "stars": q.stars(),
            })
        };
        writeln!(
            out,
            "{}",
            serde_json::json!({
                "rel": cand[i], "cos_16_32": cos(&v16, &v32), "cos_32_cache": cos(&v32, &cached[i]),
                "cos_16_cache": cos(&v16, &cached[i]), "fp32": axis(&p[0]), "fp16": axis(&p[1]), "cache": axis(&p[2]),
            })
        )?;
        out.flush()?;
        if (k + 1) % 25 == 0 {
            println!("{}/{n}  fp32 {:.2} s/track, fp16 {:.2} s/track", k + 1, t32 / (k + 1) as f64, t16 / (k + 1) as f64);
        }
    }
    println!("done: fp32 {:.2} s/track, fp16 {:.2} s/track", t32 / n as f64, t16 / n as f64);
    Ok(())
}
