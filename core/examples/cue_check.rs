//! Parity: the GRU on the prototype's cached features, and features from scratch.
//! usage: cue_check <cue_py.json>
use std::collections::HashMap;
use vibecheck::{cues, rekordbox};

fn main() -> vibecheck::Res<()> {
    let j: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(std::env::args().nth(1).unwrap())?)?;
    let home = std::path::PathBuf::from(std::env::var("HOME")?);
    let root = home.join("Music/Collection");
    let xml = rekordbox::read(&home.join("Documents/rekordbox.xml"), &root)?;
    let model = cues::Model::load(&root)?;
    let tracks: HashMap<String, rekordbox::Track> =
        j.iter().map(|e| e["rel"].as_str().unwrap().to_string()).map(|r| (r.clone(), xml[&r].clone())).collect();
    let (cached, st) = cues::features_all(&root, &tracks)?;
    println!("cached features: {} (computed {})", st.cached, st.done);
    let (mut same_t, mut same_own, mut feat_close, mut n, mut worst) = (0, 0, 0, 0, 0f32);
    let rels: Vec<String> = j.iter().map(|e| e["rel"].as_str().unwrap().to_string()).collect();
    let own = vibecheck::keyenergy::parallel(&rels, |r| cues::track_features(&root.join(r), &tracks[r]), "");
    for (e, mine) in j.iter().zip(own) {
        let rel = e["rel"].as_str().unwrap();
        let py_t: Vec<usize> = e["T"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
        let f = &cached[rel];
        let s = model.scores(f);
        for (i, row) in e["scores"].as_array().unwrap().iter().enumerate() {
            for k in 0..3 {
                worst = worst.max((s[i][k] - row[k].as_f64().unwrap() as f32).abs());
            }
        }
        n += 1;
        same_t += (cues::decode(&s) == py_t) as usize;
        if let Some(mine) = mine {
            let t2 = model.transitions(&mine);
            same_own += (t2 == py_t) as usize;
            let close = mine.len() == f.len();
            feat_close += close as usize;
            if t2 != py_t {
                println!("own features differ: py {py_t:?} rs {t2:?}  bars {} vs {}  {rel}", f.len(), mine.len());
            }
        }
    }
    println!("{n} tracks: GRU score max diff {worst:.2e}; same transitions from cached features {same_t}; from own features {same_own}; same bar count {feat_close}");
    Ok(())
}
