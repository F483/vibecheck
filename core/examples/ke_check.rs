//! Parity: key and energy against the prototype's cached answers.
//! usage: ke_check [N]
use rusqlite::params;
use vibecheck::{keyenergy, store};

fn main() -> vibecheck::Res<()> {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let root = std::path::PathBuf::from(std::env::var("HOME")?).join("Music/Collection");
    let hb = store::hash_by_path(&store::labels_db(&root)?)?;
    let cache = store::cache_db(&root)?;
    let mut st = cache.prepare("SELECT hash, key, energy, detail FROM analysis WHERE version=?1")?;
    let rows: std::collections::HashMap<String, (String, i64, String)> =
        st.query_map(params![keyenergy::VERSION], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?.collect::<Result<_, _>>()?;
    let mut paths: Vec<&String> = hb.iter().filter(|(_, h)| rows.contains_key(*h)).map(|(p, _)| p).collect();
    paths.sort();
    let step = (paths.len() / n).max(1);
    let picked: Vec<String> = paths.into_iter().step_by(step).take(n).cloned().collect();
    let res = keyenergy::parallel(&picked, |p| keyenergy::analyse_file(&root.join(p)).ok(), "");
    let (mut k_ok, mut e_ok, mut prof) = (0, 0, 0f64);
    for (p, r) in picked.iter().zip(res) {
        let (key, en, detail) = &rows[&hb[p]];
        let Some((r, d)) = r else { println!("FAILED {p}"); continue };
        k_ok += (&r.key == key) as usize;
        e_ok += (r.energy as i64 == *en) as usize;
        let a: serde_json::Value = serde_json::from_str(detail)?;
        let b: serde_json::Value = serde_json::from_str(&d)?;
        for i in 0..12 {
            prof = prof.max((a["profile"][i].as_f64().unwrap() - b["profile"][i].as_f64().unwrap()).abs());
        }
        if &r.key != key || r.energy as i64 != *en {
            println!("py {key} {en}  rs {} {}  score py {} rs {}  {p}", r.key, r.energy, a["energy_score"], b["energy_score"]);
        }
    }
    println!("{} tracks: key same {k_ok}, energy same {e_ok}, profile max diff {prof:.5}", picked.len());
    Ok(())
}
