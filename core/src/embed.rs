//! What a track sounds like, as one vector: CLAP (laion/larger_clap_music_and_speech)
//! run through ONNX Runtime, 24 windows of 10 s, kept as their mean and
//! standard deviation (1024 numbers).
//!
//! The model file takes 10 s of 48 kHz mono PCM and returns the unit-length
//! 512-d embedding; the spectrogram is inside it. Its output matches the
//! prototype's torch pipeline to 1e-6, so the cache keys are the prototype's
//! and its cached vectors are used as they are.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use ort::session::Session;
use ort::value::Tensor;
use rusqlite::{Connection, params};

use crate::{Res, audio, now, store};

pub const BACKEND: &str = "clapw";
pub const VERSION: &str = "2";
/// The prototype's preprocessing digest for this backend: part of the cache key.
pub const PREPROC: &str = "54a22387107bbbc7";
pub const MODEL_FILE: &str = "clap_audio.onnx";
pub const RATE: u32 = 48_000;
pub const WINDOWS: usize = 24;
pub const SECONDS: f64 = 10.0;
pub const DIM: usize = 512;
const SAMPLES: usize = 480_000;

/// Everything that decides what a vector means; recorded with each round.
pub fn fingerprint() -> String {
    format!("{BACKEND}/{VERSION}/{PREPROC}")
}

/// Where model files are looked for, in order: $VIBECHECK_MODELS; `models/`
/// next to the program (the bundle: one folder holding the program and its
/// model, to copy, share or back up whole); `../Resources/models` for a macOS
/// .app; then the per-user data folder.
pub fn model_dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(d) = std::env::var_os("VIBECHECK_MODELS") {
        v.push(PathBuf::from(d));
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("models"));
            v.push(dir.join("../Resources/models"));
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    v.push(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/vibecheck/models")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home).join("vibecheck/models")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or(home.join(".local/share")).join("vibecheck/models")
    });
    v
}

pub fn model_path() -> Res<PathBuf> {
    let dirs = model_dirs();
    dirs.iter().map(|d| d.join(MODEL_FILE)).find(|p| p.exists()).ok_or_else(|| {
        format!(
            "no audio model ({MODEL_FILE}); looked in:\n{}",
            dirs.iter().map(|d| format!("  {}", d.display())).collect::<Vec<_>>().join("\n")
        )
        .into()
    })
}

pub struct Embedder {
    session: Session,
}

/// ONNX Runtime allows one environment per process; every model shares it.
fn environment() -> Res<&'static ort::environment::Environment> {
    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(e) = ENV.get() {
        return Ok(e);
    }
    let e = ort::init().build().map_err(|e| e.to_string())?;
    Ok(ENV.get_or_init(|| e))
}

impl Embedder {
    pub fn load() -> Res<Embedder> {
        Embedder::load_from(&model_path()?)
    }

    /// A specific model file, e.g. to compare two.
    pub fn load_from(path: &Path) -> Res<Embedder> {
        let threads = std::env::var("VIBECHECK_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4));
        let env = environment()?;
        #[allow(unused_mut)]
        let mut eps = vec![];
        // Apple's GPU and Neural Engine through CoreML, unless told not to
        // CoreML cannot run this graph yet (it fails to compile or to
        // predict), so it is opt-in while that is worked out
        #[cfg(target_os = "macos")]
        if std::env::var_os("VIBECHECK_COREML").is_some() {
            eps.push(
                ort::ep::CoreML::default()
                    .with_model_format(match std::env::var("VIBECHECK_COREML_FORMAT").as_deref() {
                        Ok("mlprogram") => ort::ep::coreml::ModelFormat::MLProgram,
                        _ => ort::ep::coreml::ModelFormat::NeuralNetwork,
                    })
                    .with_compute_units(match std::env::var("VIBECHECK_COREML_UNITS").as_deref() {
                        Ok("gpu") => ort::ep::coreml::ComputeUnits::CPUAndGPU,
                        Ok("ane") => ort::ep::coreml::ComputeUnits::CPUAndNeuralEngine,
                        _ => ort::ep::coreml::ComputeUnits::All,
                    })
                    .build(),
            );
        }
        let session = Session::builder(env)
            .and_then(|b| b.with_intra_threads(threads).map_err(|e| e.into()))
            .and_then(|b| b.with_execution_providers(eps).map_err(|e| e.into()))
            .and_then(|mut b| b.commit_from_file(path))
            .map_err(|e| format!("loading {}: {e}", path.display()))?;
        Ok(Embedder { session })
    }

    /// One unit vector per window.
    pub fn windows(&mut self, windows: &[&[f32]]) -> Res<Vec<[f32; DIM]>> {
        let mut out = Vec::with_capacity(windows.len());
        for chunk in windows.chunks(std::env::var("VIBECHECK_BATCH").ok().and_then(|v| v.parse().ok()).unwrap_or(2)) {
            let mut data = vec![0f32; chunk.len() * SAMPLES];
            for (i, w) in chunk.iter().enumerate() {
                let row = &mut data[i * SAMPLES..(i + 1) * SAMPLES];
                // repeat a short window to fill, then pad with silence, as
                // CLAP's feature extractor does ("repeatpad")
                let reps = if w.is_empty() { 0 } else { SAMPLES / w.len() };
                let fill = (reps * w.len()).min(SAMPLES);
                for (j, v) in row[..fill].iter_mut().enumerate() {
                    *v = w[j % w.len()];
                }
            }
            let t = Tensor::from_array(([chunk.len(), SAMPLES], data))?;
            let res = self.session.run(ort::inputs!["pcm" => t])?;
            let (_, e) = res["embedding"].try_extract_tensor::<f32>()?;
            for v in e.chunks_exact(DIM) {
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
                let mut a = [0f32; DIM];
                for (o, x) in a.iter_mut().zip(v) {
                    *o = x / n;
                }
                out.push(a);
            }
        }
        Ok(out)
    }

    /// The track's vector: mean and standard deviation across its windows.
    pub fn track(&mut self, path: &Path) -> Res<Vec<f32>> {
        let d = audio::decode(path, RATE)?;
        let ws = audio::excerpts(&d, WINDOWS, SECONDS);
        let mut v = self.windows(&ws)?;
        if v.is_empty() {
            return Err("no windows".into());
        }
        while v.len() < WINDOWS {
            v.push(*v.last().unwrap());
        }
        Ok(pool(&v))
    }
}

pub fn pool(v: &[[f32; DIM]]) -> Vec<f32> {
    let n = v.len() as f64;
    let mut out = vec![0f32; 2 * DIM];
    for d in 0..DIM {
        let mean = v.iter().map(|w| w[d] as f64).sum::<f64>() / n;
        let var = v.iter().map(|w| (w[d] as f64 - mean).powi(2)).sum::<f64>() / n;
        out[d] = mean as f32;
        out[DIM + d] = var.sqrt() as f32;
    }
    out
}

fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_bytes(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn cached_hashes(cache: &Connection) -> Res<HashSet<String>> {
    let mut st = cache.prepare("SELECT hash FROM embeddings WHERE backend=?1 AND version=?2 AND preproc=?3")?;
    let out = st.query_map(params![BACKEND, VERSION, PREPROC], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(out)
}

#[derive(Debug, Default)]
pub struct Stats {
    pub total: usize,
    pub cached: usize,
    pub done: usize,
    pub failed: usize,
}

/// Embed whatever of `paths` is not cached yet. Decoding runs on several
/// threads; the model runs one batch at a time on all cores.
pub fn embed_all(root: &Path, paths: &[String]) -> Res<Stats> {
    let lab = store::labels_db(root)?;
    let cache = store::cache_db(root)?;
    let hb = store::hash_by_path(&lab)?;
    let have = cached_hashes(&cache)?;
    let todo: Vec<&String> = paths.iter().filter(|p| hb.get(*p).is_some_and(|h| !have.contains(h))).collect();
    let mut st = Stats { total: paths.len(), cached: paths.len() - todo.len(), ..Default::default() };
    if todo.is_empty() {
        return Ok(st);
    }
    let mut emb = Embedder::load()?;
    let t0 = Instant::now();
    let mut last = t0;
    // decode ahead on worker threads while the model works
    let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, Res<audio::Decoded>)>(4);
    let next = Mutex::new(0usize);
    std::thread::scope(|s| -> Res<()> {
        for _ in 0..3 {
            let tx = tx.clone();
            let (next, todo) = (&next, &todo);
            s.spawn(move || {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    let Some(rel) = todo.get(i) else { break };
                    let r = audio::decode(&root.join(rel.as_str()), RATE);
                    if tx.send((i, r)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for (k, (i, pcm)) in rx.into_iter().enumerate() {
            let rel = todo[i];
            let vec = pcm.and_then(|d| {
                let ws = audio::excerpts(&d, WINDOWS, SECONDS);
                let mut v = emb.windows(&ws)?;
                if v.is_empty() {
                    return Err("no windows".into());
                }
                while v.len() < WINDOWS {
                    v.push(*v.last().unwrap());
                }
                Ok(pool(&v))
            });
            match vec {
                Ok(v) => {
                    cache.execute(
                        "INSERT OR REPLACE INTO embeddings (hash, backend, version, preproc, dim, vec, ts)
                         VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        params![hb[rel], BACKEND, VERSION, PREPROC, v.len() as i64, to_bytes(&v), now()],
                    )?;
                    st.done += 1;
                }
                Err(_) => st.failed += 1,
            }
            let n = k + 1;
            if last.elapsed().as_secs_f64() >= 15.0 || n == todo.len() {
                last = Instant::now();
                let el = t0.elapsed().as_secs_f64();
                let rate = n as f64 / el;
                let bar = 20 * n / todo.len();
                println!(
                    "  [{}{}] {n}/{}  {rate:.1}/s  {}s elapsed, ~{}s left",
                    "#".repeat(bar),
                    ".".repeat(20 - bar),
                    todo.len(),
                    el as u64,
                    ((todo.len() - n) as f64 / rate) as u64
                );
            }
        }
        Ok(())
    })?;
    Ok(st)
}

/// (paths that have a vector, row-major vectors), in the order given.
pub fn load(root: &Path, paths: &[String]) -> Res<(Vec<String>, Vec<Vec<f32>>)> {
    let lab = store::labels_db(root)?;
    let cache = store::cache_db(root)?;
    let hb = store::hash_by_path(&lab)?;
    let mut st = cache.prepare("SELECT hash, vec FROM embeddings WHERE backend=?1 AND version=?2 AND preproc=?3")?;
    let rows: HashMap<String, Vec<u8>> = st
        .query_map(params![BACKEND, VERSION, PREPROC], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let (mut keep, mut vecs) = (vec![], vec![]);
    for p in paths {
        if let Some(b) = hb.get(p).and_then(|h| rows.get(h)) {
            keep.push(p.clone());
            vecs.push(from_bytes(b));
        }
    }
    Ok((keep, vecs))
}
