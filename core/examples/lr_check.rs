//! Parity: train on the user's labels and compare with sklearn's probabilities.
//! usage: lr_check <lr_py.json>

use std::time::Instant;

use vibecheck::{config, embed, predict, store};

fn main() -> vibecheck::Res<()> {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::env::args().nth(1).unwrap())?)?;
    let root = std::path::PathBuf::from(std::env::var("HOME")?).join("Music/Collection");
    let cfg = config::load(&root)?;
    let mc = cfg.float("predict", "misleading_cost", 1.0);
    let axes = predict::axes_for(mc, cfg.float("predict", "rating_misleading_cost", mc));
    let lab = store::current(&store::labels_db(&root)?, Some("user"))?;
    let mut keys: Vec<String> = lab.keys().cloned().collect();
    keys.sort();
    let (paths, x) = embed::load(&root, &keys)?;
    let xr: Vec<&[f32]> = x.iter().map(|v| v.as_slice()).collect();
    let y: Vec<_> = paths.iter().map(|p| lab[p]).collect();
    let mut m = predict::Model::new(axes);
    let t0 = Instant::now();
    m.train(&xr, &y);
    println!("fit {:.2}s", t0.elapsed().as_secs_f64());
    let qp: Vec<String> = j["paths"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let (_, q) = embed::load(&root, &qp)?;
    let qr: Vec<&[f32]> = q.iter().map(|v| v.as_slice()).collect();
    for (name, lr) in &m.models {
        let a = &j["axes"][*name];
        let classes: Vec<&str> = a["classes"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(classes, lr.classes.iter().map(String::as_str).collect::<Vec<_>>(), "{name} classes");
        let mut worst = 0f64;
        for (i, r) in qr.iter().enumerate() {
            let p = lr.predict_proba(r);
            for (k, v) in p.iter().enumerate() {
                worst = worst.max((v - a["proba"][i][k].as_f64().unwrap()).abs());
            }
        }
        println!("{name:6} max |p_rust - p_sklearn| = {worst:.2e}");
    }
    let preds = m.predict(&qr);
    let mut same = 0;
    for (p, v) in preds.iter().zip(j["values"].as_array().unwrap()) {
        let py: std::collections::BTreeMap<String, String> =
            v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        let rs: std::collections::BTreeMap<String, String> = p.values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        same += (py == rs) as usize;
    }
    println!("same decisions on {same} of {} tracks", preds.len());
    Ok(())
}
