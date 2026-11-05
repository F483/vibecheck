//! Parity: embed tracks the prototype already embedded and compare.
//! usage: embed_check [N]   (collection at ~/Music/Collection)

use std::time::Instant;

use vibecheck::{embed, store};

fn main() -> vibecheck::Res<()> {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let root = std::path::PathBuf::from(std::env::var("HOME")?).join("Music/Collection");
    let con = store::labels_db(&root)?;
    let mut paths = store::live_paths(&con)?;
    // a fixed spread through the collection
    let step = (paths.len() / 997).max(1);
    paths = paths.into_iter().step_by(step).collect();
    let (keep, vecs) = embed::load(&root, &paths)?;
    let mut e = embed::Embedder::load()?;
    let (mut cos, mut worst) = (vec![], 0f32);
    let t0 = Instant::now();
    for (p, v) in keep.iter().zip(&vecs).take(n) {
        let r = e.track(&root.join(p))?;
        let dot: f32 = r.iter().zip(v).map(|(a, b)| a * b).sum();
        let na = r.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        let c = dot / na / nb;
        let d = r.iter().zip(v).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        worst = worst.max(d);
        println!("{c:.6}  {d:.2e}  {p}");
        cos.push(c);
    }
    cos.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{} tracks  cosine min {:.6} median {:.6}  max abs diff {worst:.2e}  {:.2} s/track",
        cos.len(),
        cos[0],
        cos[cos.len() / 2],
        t0.elapsed().as_secs_f64() / cos.len() as f64
    );
    Ok(())
}
