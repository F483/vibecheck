//! Key and energy, worked out from the audio. Nothing here learns at runtime.
//!
//! **Key.** A pitch-class profile from spectral peaks only (harmonic pitch
//! class profile, Gomez 2006), after estimating the tuning; each of the 24 keys
//! scores it with a fitted template plus a minor prior (key_profiles.json).
//!
//! **Energy.** Loudness, brightness and noisiness (MPEG-7), each in units of
//! its spread over reference audio, summed and placed on 1-10 so a typical club
//! track is 6.
//!
//! A port of the prototype's keyenergy.py; same method version, same cache.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use realfft::RealFftPlanner;
use rusqlite::params;

use crate::{Res, audio, now, store};

/// The prototype's method version: part of the cache key.
pub const VERSION: &str = "ke1/6d426c19f036";

const RATE: u32 = 22_050;
const WINDOWS: usize = 24;
const SECONDS: f64 = 10.0;
const N_FFT: usize = 4096;
const HOP: usize = 2048;
const FMIN: f64 = 100.0;
const FMAX: f64 = 5000.0;
const MAX_PEAKS: usize = 60;
const PEAK_FLOOR_DB: f64 = 60.0;
const BINS: usize = 36;
const WIDTH: f64 = 4.0 / 3.0;
const LOG_FLOOR: f64 = 1e-3;

const ENERGY_MEAN: [f64; 3] = [22.458, 2249.7, -4.9324];
const ENERGY_STD: [f64; 3] = [4.520, 753.7, 1.6096];
const ENERGY_MID: f64 = 0.3813;
const ENERGY_SPREAD: f64 = 2.3585;
const ENERGY_TYPICAL: f64 = 6.0;

const MAJOR: [f64; 12] = [
    2.970263, -1.473117, 0.253488, -1.49761, 1.603655, -0.371686, -1.278785, 2.237478, -1.445589, -0.200077, -1.062539,
    -0.324599,
];
const MINOR: [f64; 12] = [
    3.140219, -1.301526, -0.260705, 0.702875, -0.454154, -0.365293, -1.061849, 2.672508, -0.737646, -1.197832, 0.584875,
    -1.132353,
];
const MINOR_BIAS: f64 = 4.337129;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyEnergy {
    /// alphanumeric, "8A"
    pub key: String,
    /// 1..10
    pub energy: u8,
}

/// Pitch class (C = 0) and mode -> "8A" / "8B": each step round the wheel is
/// a fifth; A minor and C major share 8.
pub fn alphanumeric(tonic: usize, minor: bool) -> String {
    let n = (7 * tonic + if minor { 5 } else { 8 }) % 12;
    format!("{}{}", if n == 0 { 12 } else { n }, if minor { 'A' } else { 'B' })
}

/// Magnitude spectra, frames x (N_FFT/2+1), of one window; frames fully inside.
fn spectra(x: &[f32], fft: &dyn realfft::RealToComplex<f64>) -> Vec<Vec<f64>> {
    let win: Vec<f64> = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / N_FFT as f64).cos()).collect();
    let mut inp = fft.make_input_vec();
    let mut out = fft.make_output_vec();
    let mut res = vec![];
    let mut s = 0;
    while s + N_FFT <= x.len() {
        for i in 0..N_FFT {
            inp[i] = x[s + i] as f64 * win[i];
        }
        fft.process(&mut inp, &mut out).unwrap();
        res.push(out.iter().map(|c| c.norm()).collect());
        s += HOP;
    }
    res
}

/// Frequency and magnitude of each frame's strongest spectral peaks.
fn peaks(m: &[f64]) -> Vec<(f64, f64)> {
    let sr = RATE as f64;
    let db: Vec<f64> = m.iter().map(|v| 20.0 * (v + 1e-12).log10()).collect();
    let top = db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    // np.searchsorted on rfftfreq: first bin at or above the frequency
    let bin = |f: f64| (0..m.len()).find(|&k| k as f64 * sr / N_FFT as f64 >= f).unwrap_or(m.len());
    let (lo, hi) = (bin(FMIN), bin(FMAX));
    let mut out = vec![];
    for k in lo..hi {
        let (c, l, r) = (db[k], db[k - 1], db[k + 1]);
        if !(c > l && c >= r && c > top - PEAK_FLOOR_DB) {
            continue;
        }
        let off = 0.5 * (l - r) / (l - 2.0 * c + r);
        let freq = (k as f64 + off) * sr / N_FFT as f64;
        let mag = 10f64.powf((c - 0.25 * (l - r) * off) / 20.0);
        out.push((freq, mag));
    }
    if out.len() > MAX_PEAKS {
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        out.truncate(MAX_PEAKS);
    }
    out
}

/// How far the track sits from A440, in semitones: the weighted circular mean
/// of every peak's distance from the nearest equal-tempered note.
fn tuning(frames: &[Vec<(f64, f64)>]) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (f, m) in frames.iter().flatten() {
        let s = 12.0 * (f.max(1e-6) / 440.0).log2();
        let d = s - s.round_ties_even();
        let w = m * m;
        re += w * (2.0 * PI * d).cos();
        im += w * (2.0 * PI * d).sin();
    }
    im.atan2(re) / (2.0 * PI)
}

fn pymod(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r < 0.0 { r + b } else { r }
}

/// The track's twelve-note profile, C first, summing to 1.
fn pitch_profile(frames: &[Vec<(f64, f64)>]) -> [f64; 12] {
    let r = tuning(frames);
    let half = WIDTH / 2.0;
    let mut p36 = [0f64; BINS];
    for fr in frames {
        let mut p = [0f64; BINS];
        for (f, m) in fr {
            let pos = pymod(12.0 * (f.max(1e-6) / 440.0).log2() - r + 9.0, 12.0);
            for (b, pb) in p.iter_mut().enumerate() {
                let d = pymod(pos - b as f64 * 12.0 / BINS as f64 + 6.0, 12.0) - 6.0;
                if d.abs() <= half {
                    *pb += m * m * (PI / 2.0 * d / half).cos().powi(2);
                }
            }
        }
        let mx = p.iter().cloned().fold(f64::NEG_INFINITY, f64::max) + 1e-12;
        for (a, v) in p36.iter_mut().zip(p) {
            *a += v / mx;
        }
    }
    let mut p12 = [0f64; 12];
    for (k, v) in p12.iter_mut().enumerate() {
        *v = p36[(3 * k + BINS - 1) % BINS] + p36[3 * k] + p36[3 * k + 1];
    }
    let s: f64 = p12.iter().sum::<f64>() + 1e-12;
    p12.map(|v| v / s)
}

/// (tonic, minor, margin over the runner-up) of the best-scoring key.
fn key_of(profile: &[f64; 12]) -> (usize, bool, f64) {
    let f: Vec<f64> = profile.iter().map(|v| (v + LOG_FLOOR).ln()).collect();
    let mut s = [0f64; 24];
    for t in 0..12 {
        let rot = |i: usize| f[(i + t) % 12];
        s[t] = (0..12).map(|i| rot(i) * MAJOR[i]).sum();
        s[12 + t] = (0..12).map(|i| rot(i) * MINOR[i]).sum::<f64>() + MINOR_BIAS;
    }
    let mut order: Vec<usize> = (0..24).collect();
    order.sort_by(|a, b| s[*a].partial_cmp(&s[*b]).unwrap());
    let best = order[23];
    (best % 12, best >= 12, s[best] - s[order[22]])
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// (loudness dB, brightness Hz, noisiness log): medians over frames.
fn energy_features(spec: &[Vec<f64>]) -> [f64; 3] {
    let (mut loud, mut cen, mut flat) = (vec![], vec![], vec![]);
    for m in spec {
        let m = &m[1..]; // drop DC
        let n = m.len() as f64;
        let (mut sp, mut slp, mut smf, mut sm) = (0.0, 0.0, 0.0, 0.0);
        for (k, v) in m.iter().enumerate() {
            let f = (k + 1) as f64 * RATE as f64 / N_FFT as f64;
            let p = v * v + 1e-12;
            sp += p;
            slp += p.ln();
            smf += v * f;
            sm += v;
        }
        let mean = sp / n;
        loud.push(10.0 * mean.log10());
        cen.push(smf / (sm + 1e-12));
        flat.push(((slp / n).exp() / mean).ln());
    }
    [median(loud), median(cen), median(flat)]
}

fn energy_score(f: &[f64; 3]) -> f64 {
    (0..3).map(|i| (f[i] - ENERGY_MEAN[i]) / ENERGY_STD[i]).sum()
}

fn energy_of(f: &[f64; 3]) -> u8 {
    let level = ENERGY_TYPICAL + (energy_score(f) - ENERGY_MID) / ENERGY_SPREAD;
    level.round_ties_even().clamp(1.0, 10.0) as u8
}

fn round_to(v: f64, d: i32) -> f64 {
    let k = 10f64.powi(d);
    (v * k).round_ties_even() / k
}

/// Key and energy of a decoded track, with the detail the cache keeps.
pub fn analyse(d: &audio::Decoded) -> Res<(KeyEnergy, String)> {
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let spec: Vec<Vec<f64>> = audio::excerpts(d, WINDOWS, SECONDS)
        .into_iter()
        .filter(|w| w.len() >= N_FFT)
        .flat_map(|w| spectra(w, fft.as_ref()))
        .collect();
    if spec.is_empty() {
        return Err("too short to analyse".into());
    }
    let frames: Vec<Vec<(f64, f64)>> = spec.iter().map(|m| peaks(m)).collect();
    let profile = pitch_profile(&frames);
    let (tonic, minor, margin) = key_of(&profile);
    let feats = energy_features(&spec);
    let r = KeyEnergy { key: alphanumeric(tonic, minor), energy: energy_of(&feats) };
    let detail = format!(
        "{{\"profile\": [{}], \"key_score\": {}, \"energy_features\": [{}], \"energy_score\": {}}}",
        profile.iter().map(|v| store::py_float(round_to(*v, 5))).collect::<Vec<_>>().join(", "),
        store::py_float(round_to(margin, 4)),
        feats.iter().map(|v| store::py_float(*v)).collect::<Vec<_>>().join(", "),
        store::py_float(round_to(energy_score(&feats), 4)),
    );
    Ok((r, detail))
}

pub fn analyse_file(path: &Path) -> Res<(KeyEnergy, String)> {
    analyse(&audio::decode(path, RATE)?)
}

#[derive(Debug, Default)]
pub struct Stats {
    pub cached: usize,
    pub done: usize,
    pub failed: usize,
}

/// Key and energy for each track, from the cache where it can.
pub fn analyse_all(root: &Path, rels: &[String]) -> Res<(HashMap<String, KeyEnergy>, Stats)> {
    let cache = store::cache_db(root)?;
    let hb = store::hash_by_path(&store::labels_db(root)?)?;
    let have: HashMap<String, KeyEnergy> = {
        let mut st = cache.prepare("SELECT hash, key, energy FROM analysis WHERE version=?1")?;
        st.query_map([VERSION], |r| Ok((r.get(0)?, KeyEnergy { key: r.get(1)?, energy: r.get::<_, i64>(2)? as u8 })))?
            .collect::<Result<_, _>>()?
    };
    let mut out = HashMap::new();
    let mut todo = vec![];
    for rel in rels {
        let Some(h) = hb.get(rel) else { continue };
        match have.get(h) {
            Some(k) => {
                out.insert(rel.clone(), k.clone());
            }
            None => todo.push(rel.clone()),
        }
    }
    let mut st = Stats { cached: out.len(), ..Default::default() };
    if todo.is_empty() {
        return Ok((out, st));
    }
    let results = parallel(&todo, |rel| analyse_file(&root.join(rel)).ok(), "      ");
    cache.execute_batch("BEGIN")?;
    for (rel, r) in todo.iter().zip(results) {
        match r {
            Some((k, detail)) => {
                cache.execute(
                    "INSERT OR REPLACE INTO analysis (hash, version, key, energy, detail, ts) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![hb[rel], VERSION, k.key, k.energy as i64, detail, now()],
                )?;
                out.insert(rel.clone(), k);
                st.done += 1;
            }
            None => st.failed += 1,
        }
    }
    cache.execute_batch("COMMIT")?;
    Ok((out, st))
}

/// Run `f` over items on all cores, in order, printing progress for long runs.
pub fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync, indent: &str) -> Vec<R> {
    let n = items.len();
    let next = Mutex::new(0usize);
    let done = Mutex::new((0usize, Instant::now(), Instant::now()));
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut k = next.lock().unwrap();
                        *k += 1;
                        *k - 1
                    };
                    if i >= n {
                        break;
                    }
                    let r = f(&items[i]);
                    *slots[i].lock().unwrap() = Some(r);
                    let mut d = done.lock().unwrap();
                    d.0 += 1;
                    if n > 50 && (d.2.elapsed().as_secs_f64() >= 15.0 || d.0 == n) {
                        d.2 = Instant::now();
                        let rate = d.0 as f64 / d.1.elapsed().as_secs_f64();
                        println!("{indent}{}/{n}  {rate:.1}/s, ~{}s left", d.0, ((n - d.0) as f64 / rate) as u64);
                    }
                }
            });
        }
    });
    slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel() {
        assert_eq!(alphanumeric(9, true), "8A"); // A minor
        assert_eq!(alphanumeric(0, false), "8B"); // C major
        assert_eq!(alphanumeric(4, true), "9A"); // E minor
        assert_eq!(alphanumeric(7, false), "9B"); // G major
        assert_eq!(alphanumeric(5, false), "7B"); // F major
    }
}
