//! The cue network trained here, measured as the prototype was: your cued tracks split five
//! ways, each fifth predicted by a model trained on the other four, cues placed with your
//! `lead` and compared with yours bar for bar. Reads only; writes cache rows for features.
//! usage: cue_train [epochs] [--folds N]
use std::collections::HashSet;
use std::time::Instant;
use vibecheck::{config, cue_train, cues, rekordbox};

fn main() -> vibecheck::Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(cue_train::EPOCHS);
    let folds: usize = args.iter().position(|a| a == "--folds").and_then(|i| args.get(i + 1)?.parse().ok()).unwrap_or(5);
    let home = std::path::PathBuf::from(std::env::var("HOME")?);
    let root = home.join("Music/Collection");
    let cfg = config::load(&root)?;
    let lead = cfg.ints("cues", "lead", &[0]);
    let xml = rekordbox::read(&home.join("Documents/rekordbox.xml"), &root)?;
    let t0 = Instant::now();
    let data = cue_train::examples(&root, &xml, &cfg)?;
    let bars: usize = data.iter().map(|e| e.x.len()).sum();
    println!("{} tracks with your cues, {} bars, features in {:.0}s", data.len(), bars, t0.elapsed().as_secs_f64());
    // your hot cue bars per track
    let hand = |rel: &str| -> HashSet<usize> {
        let t = &xml[rel];
        let down = cues::downbeats(&t.tempos, t.duration);
        let m = cues::place(&[], &down, &lead, &[], false);
        let _ = m;
        t.marks.iter().filter_map(|m| {
            let num: i64 = m.iter().find(|(k, _)| k == "Num")?.1.parse().ok()?;
            let st: f64 = m.iter().find(|(k, _)| k == "Start")?.1.parse().ok()?;
            (num >= 0).then(|| cues::bar_of(&down, st)).flatten()
        }).collect()
    };
    let (mut cue_hit, mut cue_all, mut t_hit, mut t_all, mut half, mut none, mut fin, mut fout) = (0, 0, 0, 0, 0, 0, 0, 0);
    let mut shipped_hit = (0, 0);
    let shipped = cues::Model::from_bytes(include_bytes!("../src/cue_model.bin"))?;
    for f in 0..folds {
        let t0 = Instant::now();
        let (test, train): (Vec<_>, Vec<_>) = data.iter().enumerate().partition(|(i, _)| i % folds == f);
        let train: Vec<cue_train::Example> = train.into_iter().map(|(_, e)| cue_train::Example { rel: e.rel.clone(), x: e.x.clone(), t: e.t.clone() }).collect();
        let m = cues::Model::from_bytes(&cue_train::fit(&train, "cv", cue_train::SEEDS, epochs)?)?;
        for (_, e) in test {
            let tr = m.transitions(&e.x);
            let t = &xml[&e.rel];
            let down = cues::downbeats(&t.tempos, t.duration);
            let placed: HashSet<usize> = cues::place(&tr, &down, &lead, &[], false).iter()
                .filter_map(|m| cues::bar_of(&down, m.iter().find(|(k, _)| k == "Start")?.1.parse().ok()?)).collect();
            let mine = hand(&e.rel);
            let hit = mine.intersection(&placed).count();
            cue_hit += hit;
            cue_all += mine.len();
            half += (2 * hit >= mine.len()) as usize;
            none += (hit == 0) as usize;
            t_hit += tr.iter().filter(|b| e.t.contains(b)).count();
            t_all += e.t.len();
            fin += (tr.first() == e.t.first()) as usize;
            fout += (tr.last() == e.t.last()) as usize;
            let st = shipped.transitions(&e.x);
            shipped_hit.0 += st.iter().filter(|b| e.t.contains(b)).count();
            shipped_hit.1 += e.t.len();
        }
        println!("fold {}/{folds}: {:.0}s", f + 1, t0.elapsed().as_secs_f64());
    }
    let n = data.len() as f64;
    let pc = |a: usize, b: f64| 100.0 * a as f64 / b;
    println!("held out, {epochs} epochs:");
    println!("  your cues reproduced, exact bar   {:.1}%   (prototype 49%)", pc(cue_hit, cue_all as f64));
    println!("  tracks with at least half exact   {:.1}%   (63%)", pc(half, n));
    println!("  tracks with none exact            {:.1}%   (7%)", pc(none, n));
    println!("  transition points exact           {:.1}%   (42%)", pc(t_hit, t_all as f64));
    println!("  mix-in / mix-out exact            {:.1}% / {:.1}%   (21% / 20%)", pc(fin, n), pc(fout, n));
    println!("  shipped model on the same tracks (seen in training): transitions {:.1}%", pc(shipped_hit.0, shipped_hit.1 as f64));
    Ok(())
}
