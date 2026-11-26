//! Cue points, learned from your own cues (docs/cues.md).
//!
//! Per-bar features: sound (mel, chroma, loudness, brightness, onsets) and structure
//! (Laplacian segmentation, McFee & Ellis 2014), on rekordbox's own grid, as the prototype's
//! cues.py made them. A small bidirectional GRU reads them, in one of two models:
//!
//! - **v2, generic**: per bar, where each hot cue slot goes, beats, loops, memory cues;
//!   decoded into cues as they are (`decode_generic`). Trained in `cue_train`.
//! - **v1, shipped**: four transition points (mix-in, two between, mix-out), with cues
//!   placed around them by settings (`place`: `lead`, `loops`, `memory`).

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::path::Path;

use realfft::RealFftPlanner;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::rekordbox::Track;
use crate::{Res, audio, keyenergy, now, store};

/// The prototype's feature version: part of the cache key.
pub const VERSION: &str = "cue1/818c48ebef3e";
pub const SR: u32 = 22_050;
const N_FFT: usize = 2048;
const HOP: usize = 512;
const N_MELS: usize = 64;
const KS: [usize; 5] = [3, 4, 5, 6, 8];
const VIEW_KS: [usize; 3] = [4, 6, 8];
pub const TRANSITIONS: usize = 4;
pub const GAP: usize = 8;
pub const SLOTS: &[u8] = b"ABCDEFGH";
const CUE_RGB: (u8, u8, u8) = (40, 226, 20);
const LOOP_RGB: (u8, u8, u8) = (255, 140, 0);
pub const MODEL_FILE: &str = "cue_model.bin";
const SHIPPED: &[u8] = include_bytes!("cue_model.bin");

pub type Mark = Vec<(String, String)>;

fn get<'a>(m: &'a Mark, k: &str) -> Option<&'a str> {
    m.iter().find(|a| a.0 == k).map(|a| a.1.as_str())
}

fn num(m: &Mark) -> i64 {
    get(m, "Num").and_then(|v| v.parse().ok()).unwrap_or(-1)
}

fn start(m: &Mark) -> f64 {
    get(m, "Start").and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

// --- the grid ----------------------------------------------------------------

/// Every bar start of rekordbox's grid, in seconds. Each TEMPO entry runs until
/// the next; the first is extended back to the start of the track.
pub fn downbeats(tempos: &[(f64, f64, i64)], duration: f64) -> Vec<f64> {
    beats(tempos, duration).into_iter().filter(|x| x.1 == 0).map(|x| x.0).collect()
}

/// Every beat of rekordbox's grid, in seconds, with its place in the bar (0 the downbeat).
pub fn beats(tempos: &[(f64, f64, i64)], duration: f64) -> Vec<(f64, i64)> {
    let (mut times, mut pos) = (vec![], vec![]);
    for (i, &(ini, bpm, beat)) in tempos.iter().enumerate() {
        let end = tempos.get(i + 1).map_or(duration, |t| t.0);
        let step = 60.0 / bpm;
        let n = (((end - ini) / step - 1e-6).floor() as i64 + 1).max(0);
        for k in 0..n {
            times.push(ini + k as f64 * step);
            pos.push((beat - 1 + k).rem_euclid(4));
        }
    }
    let Some(&(ini, bpm, beat)) = tempos.first() else { return vec![] };
    let step = 60.0 / bpm;
    let (mut t, mut p, mut pre) = (ini - step, (beat - 2).rem_euclid(4), vec![]);
    while t >= 0.0 {
        pre.push((t, p));
        t -= step;
        p = (p - 1).rem_euclid(4);
    }
    pre.reverse();
    pre.into_iter().chain(times.into_iter().zip(pos)).collect()
}

/// The grid's identity, as the prototype computes it: part of the cache key.
pub fn grid_digest(tempos: &[(f64, f64, i64)]) -> String {
    let r = |v: f64, d: usize| store::py_float(format!("{v:.d$}").parse::<f64>().unwrap());
    let body = tempos.iter().map(|(a, b, c)| format!("[{}, {}, {c}]", r(*a, 3), r(*b, 3))).collect::<Vec<_>>().join(", ");
    let h = Sha256::digest(format!("[{body}]").as_bytes());
    hex::encode(h)[..12].to_string()
}

fn median(v: &[f64]) -> f64 {
    let mut s: Vec<f64> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

pub fn bar_time(down: &[f64], b: usize) -> f64 {
    if b < down.len() {
        return down[b];
    }
    let tail = &down[down.len().saturating_sub(16)..];
    let step = median(&tail.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    down[down.len() - 1] + (b - down.len() + 1) as f64 * step
}

/// The bar a cue starts on, or None if it is not near a downbeat.
pub fn bar_of(down: &[f64], seconds: f64) -> Option<usize> {
    let b = (0..down.len()).min_by(|&a, &b| (down[a] - seconds).abs().partial_cmp(&(down[b] - seconds).abs()).unwrap())?;
    let step = median(&down.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    if (down[b] - seconds).abs() > 0.6 * step { None } else { Some(b) }
}

// --- per-bar sound (librosa's definitions) ----------------------------------

fn hz_to_mel(f: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
}

fn mel_to_hz(m: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
}

/// librosa.filters.mel(sr, n_fft, n_mels): slaney scale and norm, 0 .. sr/2.
fn mel_basis() -> Vec<Vec<f64>> {
    let nb = N_FFT / 2 + 1;
    let fft: Vec<f64> = (0..nb).map(|k| k as f64 * SR as f64 / N_FFT as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(SR as f64 / 2.0));
    let mel_f: Vec<f64> = (0..N_MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64)).collect();
    (0..N_MELS)
        .map(|i| {
            let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
            fft.iter()
                .map(|&f| {
                    let lower = (f - mel_f[i]) / (mel_f[i + 1] - mel_f[i]);
                    let upper = (mel_f[i + 2] - f) / (mel_f[i + 2] - mel_f[i + 1]);
                    lower.min(upper).max(0.0) * enorm
                })
                .collect()
        })
        .collect()
}

/// librosa.filters.chroma(sr, n_fft, tuning): 12 x bins, C first.
fn chroma_basis(tuning: f64) -> Vec<Vec<f64>> {
    let n = 12usize;
    let a440 = 440.0 * 2f64.powf(tuning / n as f64);
    let octs = |f: f64| (f / (a440 / 16.0)).log2();
    let mut frq: Vec<f64> = (1..N_FFT).map(|k| n as f64 * octs(k as f64 * SR as f64 / N_FFT as f64)).collect();
    frq.insert(0, frq[0] - 1.5 * n as f64);
    let mut width: Vec<f64> = frq.windows(2).map(|w| (w[1] - w[0]).max(1.0)).collect();
    width.push(1.0);
    let n2 = (n as f64 / 2.0).round();
    let mut w = vec![vec![0f64; N_FFT]; n];
    for (c, row) in w.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            let d = (frq[j] - c as f64 + n2 + 10.0 * n as f64).rem_euclid(n as f64) - n2;
            *v = (-0.5 * (2.0 * d / width[j]).powi(2)).exp();
        }
    }
    // each column to unit L2 norm, then the octave weighting
    for j in 0..N_FFT {
        let s = (0..n).map(|c| w[c][j] * w[c][j]).sum::<f64>().sqrt();
        let g = (-0.5 * ((frq[j] / n as f64 - 5.0) / 2.0).powi(2)).exp();
        for row in w.iter_mut() {
            if s > f64::MIN_POSITIVE {
                row[j] /= s;
            }
            row[j] *= g;
        }
    }
    // base C: roll by -3
    let mut out: Vec<Vec<f64>> = (0..n).map(|c| w[(c + 3) % n][..N_FFT / 2 + 1].to_vec()).collect();
    for r in out.iter_mut() {
        for v in r.iter_mut() {
            *v = *v as f32 as f64;
        }
    }
    out
}

/// librosa.estimate_tuning over a power spectrogram (frames x bins).
fn estimate_tuning(s: &[Vec<f64>]) -> f64 {
    let nb = N_FFT / 2 + 1;
    let fft = |k: usize| k as f64 * SR as f64 / N_FFT as f64;
    let (mut pitches, mut mags) = (vec![], vec![]);
    for fr in s {
        let mx = fr.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let rv = 0.1 * mx;
        let x: Vec<f64> = fr.iter().map(|v| if *v > rv { *v } else { 0.0 }).collect();
        for k in 1..nb - 1 {
            let f = fft(k);
            if !(150.0 <= f && f < 4000.0) {
                continue;
            }
            if !(x[k] > x[k - 1] && x[k] >= x[k + 1]) {
                continue;
            }
            let a = fr[k + 1] + fr[k - 1] - 2.0 * fr[k];
            let b = (fr[k + 1] - fr[k - 1]) / 2.0;
            let shift = if b.abs() >= a.abs() { 0.0 } else { -b / a };
            let avg = (fr[k + 1] - fr[k - 1]) / 2.0;
            pitches.push((k as f64 + shift) * SR as f64 / N_FFT as f64);
            mags.push(fr[k] + 0.5 * avg * shift);
        }
    }
    let ok: Vec<usize> = (0..pitches.len()).filter(|&i| pitches[i] > 0.0).collect();
    if ok.is_empty() {
        return 0.0;
    }
    let threshold = median(&ok.iter().map(|&i| mags[i]).collect::<Vec<_>>());
    let mut counts = [0usize; 100];
    let mut any = false;
    for &i in &ok {
        if mags[i] < threshold {
            continue;
        }
        let mut r = (12.0 * (pitches[i] / 27.5).log2()).rem_euclid(1.0);
        if r >= 0.5 {
            r -= 1.0;
        }
        // np.histogram over 101 edges from -0.5 to 0.5; the last bin is closed
        let b = (((r + 0.5) * 100.0).floor() as isize).clamp(0, 99) as usize;
        counts[b] += 1;
        any = true;
    }
    if !any {
        return 0.0;
    }
    let best = (0..100).fold(0, |b, i| if counts[i] > counts[b] { i } else { b });
    -0.5 + best as f64 * 0.01
}

fn power_to_db(v: &mut [f64]) {
    for x in v.iter_mut() {
        *x = 10.0 * x.max(1e-10).log10();
    }
    let top = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    for x in v.iter_mut() {
        *x = x.max(top - 80.0);
    }
}

#[derive(Default)]
pub struct Bars {
    pub mel: Vec<Vec<f64>>,
    pub chroma: Vec<Vec<f64>>,
    pub rms: Vec<f64>,
    pub centroid: Vec<f64>,
    pub onset: Vec<f64>,
}

/// Spectrum, notes, loudness, brightness and onsets, averaged per bar.
pub fn bar_audio(y: &[f32], down: &[f64]) -> Bars {
    let nb = N_FFT / 2 + 1;
    let win: Vec<f64> = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / N_FFT as f64).cos()).collect();
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(N_FFT);
    let (mut inp, mut out) = (fft.make_input_vec(), fft.make_output_vec());
    let frames = 1 + y.len() / HOP;
    let half = (N_FFT / 2) as isize;
    // power spectrogram, frames x bins, centred and zero-padded
    let mut s: Vec<Vec<f64>> = Vec::with_capacity(frames);
    for t in 0..frames {
        if t % 4096 == 0 && crate::stopping() {
            return Bars::default();
        }
        let c = (t * HOP) as isize - half;
        for i in 0..N_FFT {
            let k = c + i as isize;
            let v = if k >= 0 && (k as usize) < y.len() { y[k as usize] as f64 } else { 0.0 };
            inp[i] = v * win[i];
        }
        fft.process(&mut inp, &mut out).unwrap();
        s.push(out.iter().map(|z| (z.norm_sqr() as f32) as f64).collect());
    }
    // each mel filter is a triangle over a few bins: summing only its span skips exact zeros,
    // so the result is the same as the full product, ~30x less work (it was most of export)
    let mb: Vec<(usize, Vec<f64>)> = mel_basis()
        .into_iter()
        .map(|row| {
            let a = row.iter().position(|w| *w != 0.0).unwrap_or(0);
            let b = row.iter().rposition(|w| *w != 0.0).map_or(a, |b| b + 1);
            (a, row[a..b].to_vec())
        })
        .collect();
    let mut mel: Vec<f64> = vec![0.0; frames * N_MELS];
    for (t, fr) in s.iter().enumerate() {
        for (m, (a, row)) in mb.iter().enumerate() {
            mel[t * N_MELS + m] = row.iter().zip(&fr[*a..]).map(|(a, b)| a * b).sum();
        }
    }
    power_to_db(&mut mel);
    let cb = chroma_basis(estimate_tuning(&s));
    let mut chroma = vec![0f64; frames * 12];
    for (t, fr) in s.iter().enumerate() {
        if t % 4096 == 0 && crate::stopping() {
            return Bars::default();
        }
        let row: Vec<f64> = cb.iter().map(|c| c.iter().zip(fr).map(|(a, b)| a * b).sum()).collect();
        let mx = row.iter().cloned().fold(0f64, |m, v| m.max(v.abs()));
        for (c, v) in row.iter().enumerate() {
            chroma[t * 12 + c] = if mx > f64::MIN_POSITIVE { v / mx } else { *v };
        }
    }
    let mut rms: Vec<f64> = s.iter().map(|fr| fr.iter().sum::<f64>() + 1e-10).collect();
    power_to_db(&mut rms);
    let cen: Vec<f64> = s
        .iter()
        .map(|fr| {
            let mag: Vec<f64> = fr.iter().map(|v| v.sqrt()).collect();
            let sum: f64 = mag.iter().sum();
            if sum < f64::MIN_POSITIVE {
                return 0.0;
            }
            mag.iter().enumerate().map(|(k, m)| k as f64 * SR as f64 / N_FFT as f64 * m / sum).sum()
        })
        .collect();
    // onset strength on the dB mel: mean positive difference, shifted by 3 frames
    let mut ons = vec![0f64; frames];
    for t in 1..frames {
        let v: f64 = (0..N_MELS).map(|m| (mel[t * N_MELS + m] - mel[(t - 1) * N_MELS + m]).max(0.0)).sum::<f64>() / N_MELS as f64;
        if t + 2 < frames {
            ons[t + 2] = v;
        }
    }
    let _ = nb;
    // per bar: frames whose time falls in [downbeat, next downbeat)
    let ft = |t: usize| (t * HOP) as f64 / SR as f64;
    let mut edges = down.to_vec();
    let n = down.len();
    edges.push(down[n - 1] + (down[n - 1] - down[n - 2]));
    let idx: Vec<usize> = edges.iter().map(|&e| (0..frames).take_while(|&t| ft(t) < e).count()).collect();
    let span = |i: usize| (idx[i], idx[i + 1].max(idx[i] + 1).min(frames));
    let per_bar = |data: &[f64], w: usize| -> Vec<Vec<f64>> {
        (0..n)
            .map(|i| {
                let (a, b) = span(i);
                if a >= frames {
                    return vec![f64::NAN; w];
                }
                let mut v = vec![0f64; w];
                for t in a..b {
                    for (j, x) in v.iter_mut().enumerate() {
                        *x += data[t * w + j];
                    }
                }
                v.into_iter().map(|x| x / (b - a) as f64).collect()
            })
            .collect()
    };
    let one = |d: &[f64]| per_bar(d, 1).into_iter().map(|v| v[0]).collect::<Vec<_>>();
    Bars { mel: per_bar(&mel, N_MELS), chroma: per_bar(&chroma, 12), rms: one(&rms), centroid: one(&cen), onset: one(&ons) }
}

// --- structure ---------------------------------------------------------------

/// scipy.ndimage.median_filter along one axis, mode 'reflect'.
fn median_filter_rows(m: &[Vec<f64>], size: usize, along_rows: bool) -> Vec<Vec<f64>> {
    let (r, c) = (m.len(), m.first().map_or(0, |x| x.len()));
    let h = (size / 2) as isize;
    let refl = |i: isize, n: usize| -> usize {
        let n = n as isize;
        let mut i = i;
        loop {
            if i < 0 {
                i = -i - 1;
            } else if i >= n {
                i = 2 * n - i - 1;
            } else {
                return i as usize;
            }
        }
    };
    let mut out = vec![vec![0f64; c]; r];
    let mut buf = Vec::with_capacity(size);
    for i in 0..r {
        for j in 0..c {
            buf.clear();
            for o in -h..=h {
                buf.push(if along_rows { m[refl(i as isize + o, r)][j] } else { m[i][refl(j as isize + o, c)] });
            }
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap());
            out[i][j] = buf[size / 2];
        }
    }
    out
}

/// A small deterministic random source for k-means++ starts.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// k-means (k-means++ starts, Lloyd), best of several starts by inertia.
fn kmeans(y: &[Vec<f64>], k: usize, starts: usize) -> Vec<usize> {
    let n = y.len();
    if n <= k {
        return (0..n).collect();
    }
    let d2 = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, z)| (x - z).powi(2)).sum::<f64>();
    let dim = y[0].len();
    let var: f64 = (0..dim)
        .map(|j| {
            let m = y.iter().map(|r| r[j]).sum::<f64>() / n as f64;
            y.iter().map(|r| (r[j] - m).powi(2)).sum::<f64>() / n as f64
        })
        .sum::<f64>()
        / dim as f64;
    let tol = 1e-4 * var;
    let mut rng = Rng(0);
    let mut best: (f64, Vec<usize>) = (f64::INFINITY, vec![]);
    let trials = 2 + (k as f64).ln() as usize;
    for _ in 0..starts {
        // k-means++ with local trials, as scikit-learn
        let mut cent: Vec<Vec<f64>> = vec![y[(rng.next() * n as f64) as usize % n].clone()];
        let mut closest: Vec<f64> = y.iter().map(|r| d2(r, &cent[0])).collect();
        while cent.len() < k {
            let total: f64 = closest.iter().sum();
            let mut pick = None;
            let mut pot_best = f64::INFINITY;
            for _ in 0..trials {
                let mut r = rng.next() * total;
                let mut c = n - 1;
                for (i, v) in closest.iter().enumerate() {
                    if r < *v {
                        c = i;
                        break;
                    }
                    r -= v;
                }
                let pot: f64 = y.iter().zip(&closest).map(|(row, cl)| cl.min(d2(row, &y[c]))).sum();
                if pot < pot_best {
                    pot_best = pot;
                    pick = Some(c);
                }
            }
            let c = pick.unwrap();
            cent.push(y[c].clone());
            for (cl, row) in closest.iter_mut().zip(y) {
                *cl = cl.min(d2(row, &y[c]));
            }
        }
        let mut lab = vec![0usize; n];
        for _ in 0..300 {
            for (i, row) in y.iter().enumerate() {
                lab[i] = (0..k).min_by(|&a, &b| d2(row, &cent[a]).partial_cmp(&d2(row, &cent[b])).unwrap()).unwrap();
            }
            let mut nc = vec![vec![0f64; dim]; k];
            let mut cnt = vec![0usize; k];
            for (row, &l) in y.iter().zip(&lab) {
                cnt[l] += 1;
                for (a, b) in nc[l].iter_mut().zip(row) {
                    *a += b;
                }
            }
            let mut shift = 0.0;
            for c in 0..k {
                if cnt[c] == 0 {
                    nc[c] = cent[c].clone();
                } else {
                    nc[c].iter_mut().for_each(|v| *v /= cnt[c] as f64);
                }
                shift += d2(&nc[c], &cent[c]);
            }
            cent = nc;
            if shift <= tol {
                break;
            }
        }
        for (i, row) in y.iter().enumerate() {
            lab[i] = (0..k).min_by(|&a, &b| d2(row, &cent[a]).partial_cmp(&d2(row, &cent[b])).unwrap()).unwrap();
        }
        let inertia: f64 = y.iter().zip(&lab).map(|(r, &l)| d2(r, &cent[l])).sum();
        if inertia < best.0 {
            best = (inertia, lab);
        }
    }
    best.1
}

/// Laplacian structural segmentation (McFee & Ellis 2014): a section label
/// per bar, for each section count.
/// Tracks of at most this many bars are segmented bar by bar, as the prototype did: every
/// normal track (512 bars is 16 minutes at 128 BPM). The eigendecomposition is cubic in bars,
/// 100 s on a 100-minute mix and not interruptible, so a longer one is averaged in groups of
/// 2, 4, 8.. bars first (phrase-aligned) and the labels spread back over the bars.
const SEGMENT_MAX: usize = 512;

fn segment(x: &[Vec<f64>], ks: &[usize]) -> Vec<(usize, Vec<usize>)> {
    let n = x.len();
    let mut f = 1;
    while n.div_ceil(f) > SEGMENT_MAX {
        f *= 2;
    }
    if f == 1 {
        return segment_bars(x, ks);
    }
    let pooled: Vec<Vec<f64>> =
        x.chunks(f).map(|c| (0..c[0].len()).map(|j| c.iter().map(|r| r[j]).sum::<f64>() / c.len() as f64).collect()).collect();
    segment_bars(&pooled, ks).into_iter().map(|(k, lab)| (k, (0..n).map(|i| lab[i / f]).collect())).collect()
}

fn segment_bars(x: &[Vec<f64>], ks: &[usize]) -> Vec<(usize, Vec<usize>)> {
    let n = x.len();
    let m = x[0].len();
    let mut xn = x.to_vec();
    for j in 0..m {
        let mean = x.iter().map(|r| r[j]).sum::<f64>() / n as f64;
        let sd = (x.iter().map(|r| (r[j] - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
        for r in xn.iter_mut() {
            r[j] = (r[j] - mean) / (sd + 1e-6);
        }
    }
    let mut d = vec![vec![0f64; n]; n];
    for i in 0..n {
        for j in 0..n {
            d[i][j] = xn[i].iter().zip(&xn[j]).map(|(a, b)| (a - b).powi(2)).sum();
        }
    }
    let k = 3usize.max(2 * (n as f64).sqrt().ceil() as usize);
    let mut near: Vec<f64> = vec![];
    let mut nn: Vec<Vec<usize>> = vec![];
    for row in d.iter() {
        let mut o: Vec<usize> = (0..n).collect();
        o.sort_by(|a, b| row[*a].partial_cmp(&row[*b]).unwrap());
        let kk = k.min(n - 1);
        near.extend(o[1..=kk].iter().map(|&j| row[j]));
        nn.push(o[1..=kk].to_vec());
    }
    let sig = median(&near) + 1e-9;
    let w: Vec<Vec<f64>> = d.iter().map(|r| r.iter().map(|v| (-v / sig).exp()).collect()).collect();
    let mut mm = vec![vec![false; n]; n];
    for (i, js) in nn.iter().enumerate() {
        for &j in js {
            mm[i][j] = true;
        }
    }
    let rec: Vec<Vec<f64>> =
        (0..n).map(|i| (0..n).map(|j| if mm[i][j] && mm[j][i] { w[i][j] } else { 0.0 }).collect()).collect();
    let r = median_filter_rows(&rec, 3, false);
    let step: Vec<f64> = (0..n - 1).map(|i| w[i][i + 1]).collect();
    let mut p = vec![vec![0f64; n]; n];
    for i in 0..n - 1 {
        p[i][i + 1] = step[i];
        p[i + 1][i] = step[i];
    }
    let dr: Vec<f64> = r.iter().map(|row| row.iter().sum()).collect();
    let dp: Vec<f64> = p.iter().map(|row| row.iter().sum()).collect();
    let mu = dp.iter().zip(&dr).map(|(a, b)| a * (a + b)).sum::<f64>()
        / (dp.iter().zip(&dr).map(|(a, b)| (a + b).powi(2)).sum::<f64>() + 1e-9);
    // normalised Laplacian of mu R + (1 - mu) P, diagonal ignored (scipy)
    let mut a = nalgebra::DMatrix::<f64>::zeros(n, n);
    for i in 0..n {
        for j in 0..n {
            if i != j {
                a[(i, j)] = mu * r[i][j] + (1.0 - mu) * p[i][j];
            }
        }
    }
    let deg: Vec<f64> = (0..n).map(|j| (0..n).map(|i| a[(i, j)]).sum()).collect();
    let sq: Vec<f64> = deg.iter().map(|v| if *v == 0.0 { 1.0 } else { v.sqrt() }).collect();
    let mut l = nalgebra::DMatrix::<f64>::zeros(n, n);
    for i in 0..n {
        for j in 0..n {
            l[(i, j)] = if i == j {
                if deg[i] == 0.0 { 0.0 } else { 1.0 }
            } else {
                -a[(i, j)] / sq[i] / sq[j]
            };
        }
    }
    // numpy's eigh reads the lower triangle only; the median filter left the
    // matrix slightly asymmetric, so mirror the lower triangle to match
    for i in 0..n {
        for j in i + 1..n {
            l[(i, j)] = l[(j, i)];
        }
    }
    let eig = nalgebra::SymmetricEigen::new(l);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|x, y| eig.eigenvalues[*x].partial_cmp(&eig.eigenvalues[*y]).unwrap());

    let kmax = *ks.iter().max().unwrap();
    let ev: Vec<Vec<f64>> = (0..n).map(|i| order.iter().take(kmax).map(|&c| eig.eigenvectors[(i, c)]).collect()).collect();
    let ev = median_filter_rows(&ev, 5, true);
    ks.iter()
        .map(|&kk| {
            let y: Vec<Vec<f64>> = ev
                .iter()
                .map(|r| {
                    let s = r[..kk].iter().map(|v| v * v).sum::<f64>().sqrt() + 1e-9;
                    r[..kk].iter().map(|v| v / s).collect()
                })
                .collect();
            (kk, kmeans(&y, kk, 8))
        })
        .collect()
}

/// bars x 77 features: where the bar sits, how it sounds relative to the
/// track, how it relates to the sections. Columns in the prototype's order.
pub fn features(b: &Bars) -> Option<Vec<Vec<f32>>> {
    let bad = |i: usize| {
        b.mel[i].iter().any(|v| v.is_nan())
            || b.chroma[i].iter().any(|v| v.is_nan())
            || b.rms[i].is_nan()
            || b.centroid[i].is_nan()
            || b.onset[i].is_nan()
    };
    let n = (0..b.rms.len()).rev().find(|&i| !bad(i))? + 1;
    let fix_rows = |m: &[Vec<f64>]| -> Vec<Vec<f64>> {
        let w = m[0].len();
        let med: Vec<f64> = (0..w)
            .map(|j| median(&m[..n].iter().map(|r| r[j]).filter(|v| !v.is_nan()).collect::<Vec<_>>()))
            .collect();
        m[..n].iter().map(|r| r.iter().enumerate().map(|(j, v)| if v.is_nan() { med[j] } else { *v }).collect()).collect()
    };
    let fix = |v: &[f64]| -> Vec<f64> {
        let med = median(&v[..n].iter().cloned().filter(|x| !x.is_nan()).collect::<Vec<_>>());
        v[..n].iter().map(|x| if x.is_nan() { med } else { *x }).collect()
    };
    let (mel, chroma) = (fix_rows(&b.mel), fix_rows(&b.chroma));
    let (rms, cen, ons) = (fix(&b.rms), fix(&b.centroid), fix(&b.onset));
    let nf = n as f64;
    let med_c = |v: &[f64]| -> Vec<f64> {
        let m = median(v);
        v.iter().map(|x| x - m).collect()
    };
    let low: Vec<f64> = mel.iter().map(|r| r[..8].iter().sum::<f64>() / 8.0).collect();
    let high: Vec<f64> = mel.iter().map(|r| r[40..].iter().sum::<f64>() / 24.0).collect();
    let bars: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let mut f: Vec<(String, Vec<f64>)> = vec![
        ("pos".into(), bars.iter().map(|b| b / nf).collect()),
        ("bar".into(), bars.clone()),
        ("to_end".into(), bars.iter().map(|b| nf - b).collect()),
        ("nbars".into(), vec![nf; n]),
        ("phase32mod".into(), (0..n).map(|i| (i % 32) as f64).collect()),
        ("rms".into(), med_c(&rms)),
        ("low".into(), med_c(&low)),
        ("high".into(), med_c(&high)),
        ("cen".into(), med_c(&cen.iter().map(|c| (c + 1.0).ln()).collect::<Vec<_>>())),
        ("ons".into(), {
            let m = median(&ons) + 1e-9;
            ons.iter().map(|o| o / m).collect()
        }),
    ];
    for p in [4, 8, 16, 32] {
        f.push((format!("phase{p}"), (0..n).map(|i| (i % p == 0) as u8 as f64).collect()));
    }
    let dist = |bd: &[bool]| -> Vec<f64> {
        let at: Vec<usize> = (0..n).filter(|&i| bd[i]).collect();
        (0..n).map(|i| at.iter().map(|&a| (i as f64 - a as f64).abs()).fold(f64::INFINITY, f64::min)).collect()
    };
    let bounds = |lab: &[usize]| -> Vec<bool> { (0..n).map(|i| i == 0 || lab[i] != lab[i - 1]).collect() };
    let whole: Vec<Vec<f64>> =
        mel.iter().zip(&chroma).map(|(m, c)| m.iter().cloned().chain(c.iter().map(|v| v * 10.0)).collect()).collect();
    let mut nb = vec![0f64; n];
    let (rms_med, low_med) = (median(&rms), median(&low));
    for (kk, lab) in segment(&whole, &KS) {
        let bd = bounds(&lab);
        for (a, x) in nb.iter_mut().zip(&bd) {
            *a += *x as u8 as f64;
        }
        f.push((format!("dist{kk}"), dist(&bd)));
        let mut sid = vec![0usize; n];
        let mut s = 0;
        for i in 0..n {
            if i > 0 && bd[i] {
                s += 1;
            }
            sid[i] = s;
        }
        let ns = s + 1;
        let seg_mean = |v: &[f64]| -> Vec<f64> {
            (0..ns)
                .map(|q| {
                    let xs: Vec<f64> = (0..n).filter(|&i| sid[i] == q).map(|i| v[i]).collect();
                    xs.iter().sum::<f64>() / xs.len() as f64
                })
                .collect()
        };
        let (sr, sl) = (seg_mean(&rms), seg_mean(&low));
        let rise = |s: &[f64]| -> Vec<f64> {
            (0..n).map(|i| if bd[i] { s[sid[i]] - s[sid[i].saturating_sub(1)] } else { 0.0 }).collect()
        };
        f.push((format!("seg_rms{kk}"), (0..n).map(|i| sr[sid[i]] - rms_med).collect()));
        f.push((format!("seg_low{kk}"), (0..n).map(|i| sl[sid[i]] - low_med).collect()));
        f.push((format!("seg_rise{kk}"), rise(&sr)));
        f.push((format!("seg_lowrise{kk}"), rise(&sl)));
        f.push((format!("seg_first{kk}"), (0..n).map(|i| (sid[i] == 0) as u8 as f64).collect()));
    }
    f.push(("nbound".into(), nb));
    let views: [(&str, Vec<Vec<f64>>); 3] = [
        ("bass", mel.iter().map(|r| r[..12].to_vec()).collect()),
        ("timbre", mel.iter().map(|r| r[12..].to_vec()).collect()),
        ("notes", chroma.iter().map(|r| r.iter().map(|v| v * 10.0).collect()).collect()),
    ];
    for (v, x) in views.iter() {
        let mut vb = vec![0f64; n];
        for (kk, lab) in segment(x, &VIEW_KS) {
            let bd = bounds(&lab);
            for (a, z) in vb.iter_mut().zip(&bd) {
                *a += *z as u8 as f64;
            }
            f.push((format!("{v}_dist{kk}"), dist(&bd)));
        }
        f.push((format!("{v}_nbound"), vb));
    }
    f.sort_by(|a, b| a.0.cmp(&b.0));
    let bands: Vec<Vec<f64>> = mel.iter().map(|r| (0..8).map(|g| r[g * 8..g * 8 + 8].iter().sum::<f64>() / 8.0).collect()).collect();
    let band_med: Vec<f64> = (0..8).map(|g| median(&bands.iter().map(|r| r[g]).collect::<Vec<_>>())).collect();
    let ch_mean: Vec<f64> = (0..12).map(|c| chroma.iter().map(|r| r[c]).sum::<f64>() / nf).collect();
    Some(
        (0..n)
            .map(|i| {
                f.iter()
                    .map(|(_, v)| v[i])
                    .chain((0..8).map(|g| bands[i][g] - band_med[g]))
                    .chain((0..12).map(|c| chroma[i][c] - ch_mean[c]))
                    .map(|v| v as f32)
                    .collect()
            })
            .collect(),
    )
}

// --- the network ---------------------------------------------------------------

struct Gru {
    // per direction: (w_ih [3H x in], w_hh [3H x H], b_ih, b_hh)
    dirs: Vec<(Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>)>,
    input: usize,
}

struct Net {
    inp_w: Vec<f32>,
    inp_b: Vec<f32>,
    layers: Vec<Gru>,
    out_w: Vec<f32>,
    out_b: Vec<f32>,
    /// v2: per hot cue slot, that it stays empty, from the mean over the bars
    empty_w: Vec<f32>,
    empty_b: Vec<f32>,
}

const H: usize = 64;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// erf, Abramowitz & Stegun 7.1.26 refined: |error| < 1.2e-7.
pub(crate) fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.5 * x.abs());
    let y = 1.0
        - t * (-x * x - 1.26551223
            + t * (1.00002368
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))))
            .exp();
    if x >= 0.0 { y } else { -y }
}

fn gelu(x: f32) -> f32 {
    (0.5 * x as f64 * (1.0 + erf(x as f64 / std::f64::consts::SQRT_2))) as f32
}

fn matvec(w: &[f32], x: &[f32], rows: usize) -> Vec<f32> {
    let c = x.len();
    (0..rows).map(|r| w[r * c..(r + 1) * c].iter().zip(x).map(|(a, b)| a * b).sum()).collect()
}

impl Gru {
    /// Both directions over the sequence; per step [forward, backward].
    fn run(&self, xs: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let n = xs.len();
        let mut out = vec![vec![0f32; 2 * H]; n];
        for (d, (wih, whh, bih, bhh)) in self.dirs.iter().enumerate() {
            let mut h = vec![0f32; H];
            let order: Vec<usize> = if d == 0 { (0..n).collect() } else { (0..n).rev().collect() };
            for t in order {
                let gi = matvec(wih, &xs[t], 3 * H);
                let gh = matvec(whh, &h, 3 * H);
                let mut nh = vec![0f32; H];
                for j in 0..H {
                    let r = sigmoid(gi[j] + bih[j] + gh[j] + bhh[j]);
                    let z = sigmoid(gi[H + j] + bih[H + j] + gh[H + j] + bhh[H + j]);
                    let nn = (gi[2 * H + j] + bih[2 * H + j] + r * (gh[2 * H + j] + bhh[2 * H + j])).tanh();
                    nh[j] = (1.0 - z) * nn + z * h[j];
                }
                h = nh;
                out[t][d * H..(d + 1) * H].copy_from_slice(&h);
            }
        }
        let _ = self.input;
        out
    }
}

impl Net {
    /// Per bar its scores, and (v2) per slot that it stays empty.
    fn forward(&self, x: &[Vec<f32>]) -> (Vec<Vec<f32>>, Vec<f32>) {
        let mut h: Vec<Vec<f32>> = x
            .iter()
            .map(|r| matvec(&self.inp_w, r, H).iter().zip(&self.inp_b).map(|(a, b)| gelu(a + b)).collect())
            .collect();
        for l in &self.layers {
            h = l.run(&h);
        }
        let k = self.out_b.len();
        let per_bar = h.iter().map(|r| matvec(&self.out_w, r, k).iter().zip(&self.out_b).map(|(a, b)| a + b).collect()).collect();
        let mut pool = vec![0f32; 2 * H];
        for r in &h {
            axpy(1.0 / h.len().max(1) as f32, r, &mut pool);
        }
        let empty = matvec(&self.empty_w, &pool, self.empty_b.len()).iter().zip(&self.empty_b).map(|(a, b)| a + b).collect();
        (per_bar, empty)
    }
}

fn axpy(a: f32, x: &[f32], y: &mut [f32]) {
    for (y, x) in y.iter_mut().zip(x) {
        *y += a * x;
    }
}

/// An ensemble of networks plus the input scaling they were trained with.
pub struct Model {
    nets: Vec<Net>,
    mu: Vec<f32>,
    sd: Vec<f32>,
    pub n_tracks: usize,
    pub source: String,
    /// v2: learns the cues themselves (`out`); v1: transition points, placed by settings
    pub generic: bool,
    /// when it was trained, unix seconds; 0 for the shipped one
    pub trained: i64,
}

impl Model {
    pub fn from_bytes(b: &[u8]) -> Res<Model> {
        let nl = b.iter().position(|c| *c == b'\n').ok_or("bad cue model")?;
        let head: serde_json::Value = serde_json::from_slice(&b[..nl])?;
        let data: Vec<f32> = b[nl + 1..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let mut t: HashMap<String, Vec<f32>> = HashMap::new();
        for e in head["tensors"].as_array().ok_or("bad cue model")? {
            let size: usize = e["shape"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).product();
            let off = e["offset"].as_u64().unwrap() as usize;
            t.insert(e["name"].as_str().unwrap().to_string(), data[off..off + size].to_vec());
        }
        let d = head["d"].as_u64().ok_or("bad cue model")? as usize;
        let mut take = |k: &str| t.remove(k).ok_or_else(|| format!("cue model lacks {k}"));
        let mut nets = vec![];
        for i in 0..head["nets"].as_u64().unwrap_or(0) {
            let p = format!("net{i}.");
            let mut layers = vec![];
            for l in 0..2 {
                let mut dirs = vec![];
                for s in ["", "_reverse"] {
                    dirs.push((
                        take(&format!("{p}rnn.weight_ih_l{l}{s}"))?,
                        take(&format!("{p}rnn.weight_hh_l{l}{s}"))?,
                        take(&format!("{p}rnn.bias_ih_l{l}{s}"))?,
                        take(&format!("{p}rnn.bias_hh_l{l}{s}"))?,
                    ));
                }
                layers.push(Gru { dirs, input: if l == 0 { H } else { 2 * H } });
            }
            nets.push(Net {
                inp_w: take(&format!("{p}inp.0.weight"))?,
                inp_b: take(&format!("{p}inp.0.bias"))?,
                layers,
                out_w: take(&format!("{p}out.weight"))?,
                out_b: take(&format!("{p}out.bias"))?,
                empty_w: take(&format!("{p}empty.weight")).unwrap_or_default(),
                empty_b: take(&format!("{p}empty.bias")).unwrap_or_default(),
            });
        }
        let (mu, sd) = (take("mu")?, take("sd")?);
        if mu.len() != d {
            return Err("cue model: feature count mismatch".into());
        }
        let generic = head["format"].as_str() == Some(FORMAT_V2);
        let k = if generic { out::K } else { 3 };
        if nets.iter().any(|n| n.out_b.len() != k || (generic && n.empty_b.len() != N_HOT)) {
            return Err("cue model: output count mismatch".into());
        }
        Ok(Model {
            nets,
            mu,
            sd,
            n_tracks: head["n_tracks"].as_u64().unwrap_or(0) as usize,
            source: head["source"].as_str().unwrap_or("").to_string(),
            generic,
            trained: head["trained"].as_i64().unwrap_or(0),
        })
    }

    /// Yours, once you have one (converted from the prototype's cue_model.pt
    /// by scripts/export_cue_model.py); otherwise the one shipped.
    pub fn load(root: &Path) -> Res<Model> {
        let own = store::vibecheck_dir(root)?.join(MODEL_FILE);
        if own.exists() {
            let mut m = Model::from_bytes(&std::fs::read(&own)?)?;
            if m.trained == 0 {
                // files from before it was recorded: when the file was written
                let t = std::fs::metadata(&own)?.modified()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                m.trained = t as i64;
            }
            return Ok(m);
        }
        Model::from_bytes(SHIPPED)
    }

    /// Per bar the networks' scores, averaged, and (v2) per slot that it stays empty.
    pub fn scores(&self, f: &[Vec<f32>]) -> (Vec<Vec<f32>>, Vec<f32>) {
        let x: Vec<Vec<f32>> =
            f.iter().map(|r| r.iter().enumerate().map(|(j, v)| (v - self.mu[j]) / self.sd[j]).collect()).collect();
        let outs: Vec<(Vec<Vec<f32>>, Vec<f32>)> = self.nets.iter().map(|n| n.forward(&x)).collect();
        let w = 1.0 / outs.len() as f32;
        let mut s = vec![vec![0f32; outs[0].0.first().map_or(0, |r| r.len())]; f.len()];
        let mut e = vec![0f32; outs[0].1.len()];
        for (o, oe) in &outs {
            for (row, r) in s.iter_mut().zip(o) {
                axpy(w, r, row);
            }
            axpy(w, oe, &mut e);
        }
        (s, e)
    }

    /// v1: the four transition points. v2: the four bars it most hears a cue at.
    pub fn transitions(&self, f: &[Vec<f32>]) -> Vec<usize> {
        let (s, _) = self.scores(f);
        if self.generic {
            return pick(&s.iter().map(|r| r[out::HOT]).collect::<Vec<_>>(), TRANSITIONS, &[]);
        }
        decode(&s.iter().map(|r| [r[0], r[1], r[2]]).collect::<Vec<_>>())
    }

    /// The cues it would set on a track, and the bars it hears it change at (the waveform's
    /// parts). v2 learned the cues as they are; v1 places them around its transitions with
    /// `place` (the `[cues]` settings).
    pub fn suggest(&self, f: &[Vec<f32>], t: &Track, p: &Placement) -> (Vec<Mark>, Vec<usize>) {
        let down = downbeats(&t.tempos, t.duration + 2.0);
        if !self.generic {
            let tr = self.transitions(f);
            return (place(&tr, &down, &p.lead, &p.loops, p.memory), tr);
        }
        let (s, e) = self.scores(f);
        let parts = pick(&s.iter().map(|r| r[out::HOT]).collect::<Vec<_>>(), TRANSITIONS, &[]);
        (marks_of(&decode_generic(&s, &e), &Grid::of(t)), parts)
    }
}

/// How the v1 model's transitions become cues: `[cues]` in the config.
pub struct Placement {
    pub lead: Vec<i64>,
    pub loops: Vec<(String, i64)>,
    pub memory: bool,
}

impl Placement {
    pub fn of(cfg: &crate::config::Config) -> Placement {
        Placement { lead: cfg.ints("cues", "lead", &[0]), loops: cfg.int_map("cues", "loops"), memory: cfg.bool("cues", "memory", true) }
    }
}

/// The k best bars, at least GAP apart, keeping any already fixed.
pub fn pick(score: &[f32], k: usize, fixed: &[usize]) -> Vec<usize> {
    let mut out = fixed.to_vec();
    let mut order: Vec<usize> = (0..score.len()).collect();
    order.sort_by(|a, b| score[*b].partial_cmp(&score[*a]).unwrap_or(std::cmp::Ordering::Equal));
    for b in order {
        if out.len() >= k {
            break;
        }
        if out.iter().all(|&o| b.abs_diff(o) >= GAP) {
            out.push(b);
        }
    }
    out.sort();
    out
}

/// Mix-in first, mix-out after it, the rest from the transition score.
pub fn decode(s: &[[f32; 3]]) -> Vec<usize> {
    let n = s.len();
    let argmax = |v: &[f32]| (0..v.len()).fold(0, |b, i| if v[i] > v[b] { i } else { b });
    let t: Vec<f32> = s.iter().map(|r| r[0]).collect();
    let f = argmax(&s.iter().map(|r| r[1]).collect::<Vec<_>>());
    if f + GAP >= n {
        return pick(&t, TRANSITIONS, &[]);
    }
    let mut lo: Vec<f32> = s.iter().map(|r| r[2]).collect();
    lo[..f + GAP].iter_mut().for_each(|v| *v = f32::NEG_INFINITY);
    let last = argmax(&lo);
    let mut mid = t.clone();
    mid[..f + GAP].iter_mut().for_each(|v| *v = f32::NEG_INFINITY);
    mid[(last + 1).saturating_sub(GAP)..].iter_mut().for_each(|v| *v = f32::NEG_INFINITY);
    pick(&mid, TRANSITIONS, &[f, last])
}

// --- generic cues (model v2) ----------------------------------------------------

pub const FORMAT_V2: &str = "vibecheck-cue-model/2";
/// Hot cue slots, A..H.
pub const N_HOT: usize = 8;
/// rekordbox keeps up to 10 memory cues.
pub const MAX_MEMORY: usize = 10;
/// The loop lengths a cue can have, in beats; 0: not a loop.
pub const LOOP_BEATS: [i64; 8] = [0, 4, 8, 16, 32, 64, 128, 256];
const LOOPS: usize = LOOP_BEATS.len();

/// The v2 model's scores per bar, by offset: where each hot cue slot goes (with `empty`, a
/// choice over the bars), a hot cue here at all, the beat in the bar, each slot's loop
/// length, a memory cue here, its loop length, which slot a hot cue here is in.
pub mod out {
    use super::{LOOPS, N_HOT};
    pub const SLOT: usize = 0;
    pub const HOT: usize = SLOT + N_HOT;
    pub const BEAT: usize = HOT + 1;
    pub const HOT_LOOP: usize = BEAT + 4;
    pub const MEM: usize = HOT_LOOP + N_HOT * LOOPS;
    pub const MEM_LOOP: usize = MEM + 1;
    pub const WHICH: usize = MEM_LOOP + LOOPS;
    pub const K: usize = WHICH + N_HOT;
}

/// A cue as rekordbox has it: slot 0..7 (A..H) or -1 for a memory cue; start, seconds; end
/// for a loop.
pub type Raw = (i64, f64, Option<f64>);

/// One cue on the grid: its bar, the beat in the bar (0 the downbeat), its loop length
/// (an index into `LOOP_BEATS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    pub bar: usize,
    pub beat: usize,
    pub lp: usize,
}

/// A track's cues as the v2 model learns and makes them: a cue or nothing per hot cue
/// slot, and the memory cues.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub hot: [Option<At>; N_HOT],
    pub mem: Vec<At>,
}

impl Target {
    /// Every cue, hot and memory, with the hot cue's slot (None: memory).
    pub fn all(&self) -> impl Iterator<Item = (Option<usize>, At)> + '_ {
        self.hot.iter().enumerate().filter_map(|(k, a)| a.map(|a| (Some(k), a))).chain(self.mem.iter().map(|a| (None, *a)))
    }
}

/// rekordbox's grid for one track: every beat from the first downbeat, with its bar and
/// place in the bar.
pub struct Grid {
    beats: Vec<(f64, usize, usize)>,
}

impl Grid {
    pub fn of(t: &Track) -> Grid {
        let mut bar: Option<usize> = None;
        let mut out = vec![];
        for (time, pos) in beats(&t.tempos, t.duration + 2.0) {
            if pos == 0 {
                bar = Some(bar.map_or(0, |b| b + 1));
            }
            if let Some(b) = bar {
                out.push((time, b, pos as usize));
            }
        }
        Grid { beats: out }
    }

    /// The beat nearest `seconds`: its index; None before the first downbeat or off the grid.
    fn index(&self, seconds: f64) -> Option<usize> {
        let i = self.beats.partition_point(|b| b.0 < seconds);
        let near = [i.checked_sub(1), (i < self.beats.len()).then_some(i)].into_iter().flatten();
        let best = near.min_by(|a, b| (self.beats[*a].0 - seconds).abs().total_cmp(&(self.beats[*b].0 - seconds).abs()))?;
        (seconds > self.beats[0].0 - self.beat_len(0) / 2.0).then_some(best)
    }

    fn beat_len(&self, i: usize) -> f64 {
        let i = i.min(self.beats.len().saturating_sub(2));
        self.beats.get(i + 1).zip(self.beats.get(i)).map_or(0.5, |(b, a)| b.0 - a.0)
    }

    /// Where a cue starting at `start` (and ending at `end`, a loop) sits on the grid.
    pub fn at(&self, start: f64, end: Option<f64>) -> Option<At> {
        let i = self.index(start)?;
        let (_, bar, beat) = self.beats[i];
        let lp = end.map_or(0, |e| loop_class((e - start) / self.beat_len(i)));
        Some(At { bar, beat, lp })
    }

    /// The time of a beat in a bar, and of the beat `n` beats after it; past the grid's
    /// end, at its last spacing.
    fn time(&self, bar: usize, beat: usize, n: i64) -> Option<(f64, f64)> {
        let i = self.beats.iter().position(|b| b.1 == bar && b.2 == beat).or_else(|| self.beats.iter().position(|b| b.1 == bar))?;
        let j = i + n as usize;
        let last = self.beats.len() - 1;
        let at = |j: usize| if j <= last { self.beats[j].0 } else { self.beats[last].0 + (j - last) as f64 * self.beat_len(last) };
        Some((at(i), at(j)))
    }
}

/// The loop length nearest `beats`, as an index into `LOOP_BEATS`; 0 for none.
pub fn loop_class(beats: f64) -> usize {
    if beats < 2.0 {
        return 0;
    }
    (1..LOOPS).min_by(|a, b| (beats.log2() - (LOOP_BEATS[*a] as f64).log2()).abs().total_cmp(&(beats.log2() - (LOOP_BEATS[*b] as f64).log2()).abs())).unwrap()
}

/// A track's cues as (slot 0..7, or -1 for memory; start; end for a loop), on its grid, as
/// the model learns them: cues past `bars` (the features' end) left out.
pub fn target_of(cues: &[Raw], g: &Grid, bars: usize) -> Target {
    let mut t = Target::default();
    let mut cues = cues.to_vec();
    cues.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (num, start, end) in cues {
        let Some(a) = g.at(start, end).filter(|a| a.bar < bars) else { continue };
        match num {
            0..=7 => t.hot[num as usize] = Some(a),
            -1 if t.mem.len() < MAX_MEMORY && !t.mem.iter().any(|m| (m.bar, m.beat) == (a.bar, a.beat)) => t.mem.push(a),
            _ => {}
        }
    }
    t
}

fn softmax_ln(v: &[f32]) -> Vec<f32> {
    let mx = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let ln = mx + v.iter().map(|x| (x - mx).exp()).sum::<f32>().ln();
    v.iter().map(|x| x - ln).collect()
}

fn argmax(v: &[f32]) -> usize {
    (0..v.len()).fold(0, |b, i| if v[i] > v[b] { i } else { b })
}

/// The v2 model's cues from its scores (docs/cues.md, "decoding"): as many hot cues as it
/// expects slots filled, on the bars it ranks likeliest to hold one, `gap` bars apart; each
/// bar then the slot that fits it best, all slots at once (an exact matching); memory cues
/// where one is more likely than not, surest first, up to 10.
pub fn decode_generic(s: &[Vec<f32>], e: &[f32]) -> Target {
    let lp = slot_ln(s, e);
    let n = s.len();
    let k = lp.iter().map(|l| 1.0 - l[n].exp()).sum::<f32>().round() as usize;
    decode_with(s, e, k, HOT_GAP, SlotBy::Which)
}

/// Hot cues at least this many bars apart.
pub const HOT_GAP: usize = 4;

/// Per slot: log-probabilities over the bars, then empty.
fn slot_ln(s: &[Vec<f32>], e: &[f32]) -> Vec<Vec<f32>> {
    (0..N_HOT).map(|k| softmax_ln(&s.iter().map(|r| r[out::SLOT + k]).chain([e[k]]).collect::<Vec<_>>())).collect()
}

/// What picks a hot cue's slot: the slot's own choice over the bars, which slot a cue on
/// that bar is in, or both.
#[derive(Clone, Copy)]
pub enum SlotBy {
    Position,
    Which,
    Both,
}

/// `decode_generic` with the number of hot cues, their spacing and how slots are picked given.
pub fn decode_with(s: &[Vec<f32>], e: &[f32], k: usize, gap: usize, by: SlotBy) -> Target {
    let n = s.len();
    let mut t = Target::default();
    if n == 0 {
        return t;
    }
    let lp = slot_ln(s, e);
    let mut bars: Vec<usize> = vec![];
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| s[*b][out::HOT].total_cmp(&s[*a][out::HOT]));
    for b in order {
        if bars.len() >= k.min(N_HOT) {
            break;
        }
        if bars.iter().all(|o| b.abs_diff(*o) >= gap) {
            bars.push(b);
        }
    }
    let fit = |slot: usize, b: usize| {
        let which = || softmax_ln(&s[b][out::WHICH..out::WHICH + N_HOT])[slot];
        match by {
            SlotBy::Position => lp[slot][b] - lp[slot][n],
            SlotBy::Which => which(),
            SlotBy::Both => lp[slot][b] - lp[slot][n] + which(),
        }
    };
    // the best slot for every bar together: over the bars, which slots are taken so far
    let full = 1usize << N_HOT;
    let mut best = vec![f32::NEG_INFINITY; full];
    let mut from: Vec<Vec<(usize, usize)>> = vec![vec![(0, 0); full]; bars.len() + 1];
    best[0] = 0.0;
    for (i, &b) in bars.iter().enumerate() {
        let mut next = vec![f32::NEG_INFINITY; full];
        for m in 0..full {
            if best[m] == f32::NEG_INFINITY {
                continue;
            }
            for slot in (0..N_HOT).filter(|j| m & (1 << j) == 0) {
                let v = best[m] + fit(slot, b);
                if v > next[m | 1 << slot] {
                    next[m | 1 << slot] = v;
                    from[i + 1][m | 1 << slot] = (m, slot);
                }
            }
        }
        best = next;
    }
    let mut m = (0..full).max_by(|a, b| best[*a].total_cmp(&best[*b])).unwrap_or(0);
    for i in (1..=bars.len()).rev() {
        let (prev, slot) = from[i][m];
        let b = bars[i - 1];
        let lps = &s[b][out::HOT_LOOP + slot * LOOPS..out::HOT_LOOP + (slot + 1) * LOOPS];
        t.hot[slot] = Some(At { bar: b, beat: argmax(&s[b][out::BEAT..out::BEAT + 4]), lp: argmax(lps) });
        m = prev;
    }
    let mut mem: Vec<(f32, usize)> = (0..n).map(|b| (s[b][out::MEM], b)).filter(|x| x.0 > 0.0).collect();
    mem.sort_by(|a, b| b.0.total_cmp(&a.0));
    t.mem = mem
        .into_iter()
        .take(MAX_MEMORY)
        .map(|(_, b)| At { bar: b, beat: argmax(&s[b][out::BEAT..out::BEAT + 4]), lp: argmax(&s[b][out::MEM_LOOP..out::MEM_LOOP + LOOPS]) })
        .collect();
    t.mem.sort_by_key(|a| (a.bar, a.beat));
    t
}

/// Cues on the grid as the xml's marks.
pub fn marks_of(t: &Target, g: &Grid) -> Vec<Mark> {
    t.all()
        .filter_map(|(k, a)| {
            let (start, end) = g.time(a.bar, a.beat, LOOP_BEATS[a.lp])?;
            Some(mark(k.map_or(-1, |k| k as i64), start, (a.lp > 0).then_some(end)))
        })
        .collect()
}

// --- your cues, both ways ------------------------------------------------------

/// Every cue on the track was named by a tool ("Cue 1", "Cue 2", ...).
pub fn generated(marks: &[Mark]) -> bool {
    !marks.is_empty()
        && marks.iter().all(|m| {
            let name = get(m, "Name").unwrap_or("");
            name.strip_prefix("Cue ").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// Nothing of yours to lose: no cues, only generated ones, or a lone memory
/// cue on the first downbeat (what clearing generated cues leaves).
pub fn writable(t: &Track) -> bool {
    let m = &t.marks;
    if m.is_empty() || generated(m) {
        return true;
    }
    if m.len() == 1 && num(&m[0]) == -1 && !t.tempos.is_empty() {
        let first = downbeats(&t.tempos, t.duration + 2.0);
        return first.first().is_some_and(|f| (start(&m[0]) - f).abs() < 0.01);
    }
    false
}

/// Cues compared as rekordbox writes them back: every cue, to 10 ms, in order.
pub fn cue_key(cues: &[Raw]) -> String {
    let r = |v: f64| (v * 100.0).round() as i64;
    let mut v: Vec<(i64, i64, Option<i64>)> = cues.iter().map(|c| (c.0, r(c.1), c.2.map(r))).collect();
    v.sort();
    serde_json::to_string(&v).unwrap_or_default()
}

/// A track's cues from its marks.
pub fn cue_list(marks: &[Mark]) -> Vec<Raw> {
    marks.iter().map(|m| (num(m), start(m), get(m, "End").and_then(|v| v.parse().ok()))).collect()
}

pub fn hot_starts(marks: &[Mark]) -> Vec<f64> {
    let mut v: Vec<f64> = marks.iter().filter(|m| num(m) >= 0).map(|m| format!("{:.2}", start(m)).parse().unwrap()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

/// The transition points a set of cue bars was placed around: with lead
/// [16, 8], cues at c and c + 8 mean a transition at c + 16.
pub fn transitions_from(bars: &[usize], lead: &[i64]) -> Vec<usize> {
    let have: HashSet<i64> = bars.iter().map(|b| *b as i64).collect();
    let mut lead = lead.to_vec();
    lead.sort_by(|a, b| b.cmp(a));
    let (mut used, mut out) = (HashSet::new(), vec![]);
    let mut sorted = bars.to_vec();
    sorted.sort();
    for c in sorted {
        let t = c as i64 + lead[0];
        let group: HashSet<i64> = lead.iter().map(|l| t - l).collect();
        if used.contains(&(c as i64)) || !group.is_subset(&have) || !group.is_disjoint(&used) {
            continue;
        }
        used.extend(group);
        out.push(t as usize);
    }
    out.sort();
    out
}

/// POSITION_MARK attributes for transition points T: one hot cue per lead,
/// in consecutive slots -- the first takes the front slots, the last the back
/// ones; memory cues copy them, plus one on the first downbeat.
pub fn place(t: &[usize], down: &[f64], lead: &[i64], loops: &[(String, i64)], memory: bool) -> Vec<Mark> {
    let w = lead.len();
    let mut t = t.to_vec();
    t.sort();
    if t.len() * w > SLOTS.len() {
        let mut k: Vec<usize> = t[..SLOTS.len() / w - 1].to_vec();
        k.push(*t.last().unwrap());
        t = k;
    }
    let mut slots = vec![];
    for (j, &x) in t.iter().enumerate() {
        let last = j == t.len() - 1 && t.len() > 1;
        let base = if last && w > 1 { SLOTS.len() - w } else { j * w };
        for (i, l) in lead.iter().enumerate() {
            slots.push((base + i, x as i64 - l));
        }
    }
    let s = |v: &str| v.to_string();
    let (mut hot, mut seen) = (vec![], HashSet::new());
    for (n, b) in slots {
        let b = b.max(0) as usize;
        if !seen.insert(b) {
            continue;
        }
        let slot = (SLOTS[n] as char).to_string();
        let lp = loops.iter().find(|(k, _)| *k == slot).map(|x| x.1).filter(|v| *v != 0);
        let mut m: Mark = vec![
            (s("Name"), s("")),
            (s("Type"), s(if lp.is_some() { "4" } else { "0" })),
            (s("Start"), format!("{:.3}", bar_time(down, b))),
            (s("Num"), n.to_string()),
        ];
        if let Some(l) = lp {
            m.push((s("End"), format!("{:.3}", bar_time(down, b + l as usize))));
        }
        let (r, g, bl) = if lp.is_some() { LOOP_RGB } else { CUE_RGB };
        m.extend([(s("Red"), r.to_string()), (s("Green"), g.to_string()), (s("Blue"), bl.to_string())]);
        hot.push(m);
    }
    if !memory {
        return hot;
    }
    let first: Mark =
        vec![(s("Name"), s("")), (s("Type"), s("0")), (s("Start"), format!("{:.3}", bar_time(down, 0))), (s("Num"), s("-1"))];
    let fs = first[2].1.clone();
    let copies: Vec<Mark> = hot
        .iter()
        .map(|m| {
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "Red" | "Green" | "Blue"))
                .map(|(k, v)| (k.clone(), if k == "Num" { s("-1") } else { v.clone() }))
                .collect::<Mark>()
        })
        .filter(|c| get(c, "Start") != Some(fs.as_str()))
        .collect();
    let mut out = hot;
    out.push(first);
    out.extend(copies);
    out
}

/// One cue as rekordbox's xml holds it: hot cue `num` 0..7 (A..H) or memory (-1), a loop
/// when it has an end; hot cues coloured as `place` colours them.
pub fn mark(num: i64, start: f64, end: Option<f64>) -> Mark {
    let s = |v: &str| v.to_string();
    let mut m: Mark = vec![
        (s("Name"), s("")),
        (s("Type"), s(if end.is_some() { "4" } else { "0" })),
        (s("Start"), format!("{start:.3}")),
    ];
    if let Some(e) = end {
        m.push((s("End"), format!("{e:.3}")));
    }
    m.push((s("Num"), num.to_string()));
    if num >= 0 {
        let (r, g, b) = if end.is_some() { LOOP_RGB } else { CUE_RGB };
        m.extend([(s("Red"), r.to_string()), (s("Green"), g.to_string()), (s("Blue"), b.to_string())]);
    }
    m
}

/// Tracks whose cues you set, and the transition points behind them: not
/// generated, and not this app's cues left where they were written.
pub fn yours(tracks: &HashMap<String, Track>, written: &HashMap<String, Vec<f64>>, lead: &[i64]) -> HashMap<String, Vec<usize>> {
    let mut out = HashMap::new();
    for (rel, t) in tracks {
        if t.tempos.is_empty() || !t.marks.iter().any(|m| num(m) >= 0) || generated(&t.marks) {
            continue;
        }
        if written.get(rel).is_some_and(|w| *w == hot_starts(&t.marks)) {
            continue;
        }
        let down = downbeats(&t.tempos, t.duration);
        if down.len() < 16 {
            continue;
        }
        let mut bars: Vec<usize> =
            t.marks.iter().filter(|m| num(m) >= 0).filter_map(|m| bar_of(&down, start(m))).collect::<HashSet<_>>().into_iter().collect();
        bars.sort();
        let tr = transitions_from(&bars, lead);
        if tr.len() >= 3 {
            out.insert(rel.clone(), tr);
        }
    }
    out
}

// --- many tracks ---------------------------------------------------------------

/// Per-bar features of one file on its grid; None when too short or off-grid.
pub fn track_features(path: &Path, t: &Track) -> Option<Vec<Vec<f32>>> {
    features_of(&audio::decode(path, SR).ok()?, t)
}

/// Per-bar features of audio already decoded at the cue model's rate, `SR`.
pub fn features_of(d: &audio::Decoded, t: &Track) -> Option<Vec<Vec<f32>>> {
    assert_eq!(d.rate, SR);
    let y = &d.samples;
    if y.len() < SR as usize * 30 {
        return None;
    }
    let down = downbeats(&t.tempos, y.len() as f64 / SR as f64);
    if down.len() < 16 {
        return None;
    }
    features(&bar_audio(y, &down))
}

#[derive(Debug, Default)]
pub struct Stats {
    pub cached: usize,
    pub done: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Per-bar features for each track, from the cache where it can (keyed by
/// audio, method and grid).
pub fn features_all(root: &Path, tracks: &HashMap<String, Track>) -> Res<(HashMap<String, Vec<Vec<f32>>>, Stats)> {
    let cache = store::cache_db(root)?;
    let hb = store::hash_by_path(&store::labels_db(root)?)?;
    let mut out = HashMap::new();
    let mut todo = vec![];
    let mut keys: Vec<&String> = tracks.keys().collect();
    keys.sort();
    for rel in keys {
        let t = &tracks[rel];
        let Some(h) = hb.get(rel) else { continue };
        if t.tempos.is_empty() {
            continue;
        }
        match cached_features(&cache, h, t) {
            Some(f) => {
                out.insert(rel.clone(), f);
            }
            None => todo.push(rel.clone()),
        }
    }
    let mut st = Stats { cached: out.len(), ..Default::default() };
    if todo.is_empty() {
        return Ok((out, st));
    }
    let res = keyenergy::parallel(&todo, |rel| track_features(&root.join(rel), &tracks[rel]), "      ");
    cache.execute_batch("BEGIN")?;
    for (rel, f) in todo.iter().zip(res) {
        match f {
            Some(f) => {
                keep_features(&cache, &hb[rel], &tracks[rel], &f)?;
                out.insert(rel.clone(), f);
                st.done += 1;
            }
            None => st.failed += 1,
        }
    }
    cache.execute_batch("COMMIT")?;
    Ok((out, st))
}

/// One track's per-bar features from the cache, keyed by audio, method and grid.
pub fn cached_features(cache: &Connection, hash: &str, t: &Track) -> Option<Vec<Vec<f32>>> {
    let (n, d, data): (i64, i64, Vec<u8>) = cache
        .query_row(
            "SELECT bars, dim, data FROM cue_bars WHERE hash=?1 AND version=?2 AND grid=?3",
            params![hash, VERSION, grid_digest(&t.tempos)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok()?;
    let v: Vec<f32> = data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Some(v.chunks(d as usize).take(n as usize).map(|r| r.to_vec()).collect())
}

pub fn keep_features(cache: &Connection, hash: &str, t: &Track, f: &[Vec<f32>]) -> Res<()> {
    let data: Vec<u8> = f.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    cache.execute(
        "INSERT OR REPLACE INTO cue_bars VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![hash, VERSION, grid_digest(&t.tempos), f.len() as i64, f[0].len() as i64, data, now()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downbeats_follow_the_grid() {
        let d = downbeats(&[(0.5, 120.0, 1)], 10.0);
        assert_eq!(d.len(), 5);
        assert!((d[1] - 2.5).abs() < 1e-9);
        let d = downbeats(&[(0.0, 120.0, 3)], 6.0);
        assert!((d[0] - 1.0).abs() < 1e-9 && d.len() == 3);
        let d = downbeats(&[(4.1, 120.0, 1)], 8.0);
        assert!((d[0] - 0.1).abs() < 1e-9 && d.len() == 4);
        let d = downbeats(&[(0.0, 120.0, 1), (4.0, 60.0, 1)], 12.0);
        assert_eq!(d.iter().map(|v| (v * 10.0).round() as i64).collect::<Vec<_>>(), vec![0, 20, 40, 80]);
    }

    fn grid(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 2.0).collect()
    }

    #[test]
    fn pairs_fill_the_slots_by_role() {
        let loops = vec![("A".to_string(), 16), ("G".to_string(), 16)];
        let m = place(&[32, 64, 80, 112], &grid(200), &[16, 8], &loops, false);
        let bars: Vec<(String, i64)> = m.iter().map(|x| (get(x, "Num").unwrap().to_string(), (start(x) / 2.0).round() as i64)).collect();
        let want = [16, 24, 48, 56, 64, 72, 96, 104];
        assert_eq!(bars.iter().map(|b| b.1).collect::<Vec<_>>(), want);
        assert_eq!(get(&m[0], "Type"), Some("4"));
        assert_eq!(get(&m[0], "End"), Some("64.000"));
    }

    #[test]
    fn placement_and_reading_back_agree() {
        for lead in [vec![16, 8], vec![0], vec![8]] {
            let m = place(&[32, 64, 80, 112], &grid(200), &lead, &[], true);
            let bars: Vec<usize> = m.iter().filter(|x| num(x) >= 0).map(|x| bar_of(&grid(200), start(x)).unwrap()).collect();
            assert_eq!(transitions_from(&bars, &lead), vec![32, 64, 80, 112]);
        }
        assert_eq!(transitions_from(&[16, 24, 40, 48, 56, 90], &[16, 8]), vec![32, 56]);
    }

    #[test]
    fn decode_takes_four_bars_apart_with_the_mix_in_first() {
        let mut s = vec![[0f32; 3]; 120];
        for (i, r) in s.iter_mut().enumerate() {
            r[0] = ((i * 7919) % 97) as f32 / 97.0;
        }
        s[20][1] = 50.0;
        s[100][2] = 50.0;
        let t = decode(&s);
        assert_eq!(t.len(), 4);
        assert_eq!((t[0], t[3]), (20, 100));
        assert!(t.windows(2).all(|w| w[1] - w[0] >= GAP));
    }

    #[test]
    fn generated_cues_are_writable() {
        let m = |name: &str, n: &str, s: &str| -> Mark {
            vec![("Name".into(), name.into()), ("Type".into(), "0".into()), ("Start".into(), s.into()), ("Num".into(), n.into())]
        };
        let tr = |marks: Vec<Mark>| Track { tempos: vec![(0.5, 120.0, 1)], duration: 300.0, marks, ..Default::default() };
        assert!(writable(&tr(vec![])));
        assert!(writable(&tr(vec![m("Cue 1", "0", "10.0"), m("Cue 2", "-1", "10.0")])));
        assert!(!writable(&tr(vec![m("Cue 1", "0", "10.0"), m("", "0", "10.0")])));
        assert!(writable(&tr(vec![m("", "-1", "0.500")])));
        assert!(!writable(&tr(vec![m("", "-1", "10.500")])));
    }

    /// v2 decoding: the expected number of hot cues on the best-ranked bars, each in the slot
    /// that wants it, memory cues where likely.
    #[test]
    fn generic_cues_take_the_ranked_bars_in_their_slots() {
        let n = 40;
        let mut s = vec![vec![0f32; out::K]; n];
        let mut e = vec![8f32; N_HOT];
        // A wants bar 8, B bar 16; C would take 9 but it is too close to 8
        for (k, b) in [(0, 8), (1, 16), (2, 9)] {
            s[b][out::SLOT + k] = 20.0;
            s[b][out::HOT] = 10.0;
            e[k] = -5.0;
        }
        s[9][out::HOT] = 9.0;
        s[30][out::HOT] = 1.0;
        s[16][out::HOT_LOOP + LOOPS + 5] = 3.0;
        s[8][out::WHICH] = 5.0;
        s[16][out::WHICH + 1] = 5.0;
        s[8][out::BEAT + 2] = 3.0;
        s[30][out::MEM] = 2.0;
        let t = decode_generic(&s, &e);
        let hot: Vec<(usize, usize)> = t.hot.iter().enumerate().filter_map(|(k, a)| a.map(|a| (k, a.bar))).collect();
        assert_eq!(hot.len(), 3);
        assert!(hot.contains(&(0, 8)) && hot.contains(&(1, 16)));
        assert!(!hot.iter().any(|h| h.1 == 9));
        assert_eq!((t.hot[0].unwrap().beat, t.hot[1].unwrap().lp), (2, 5));
        assert_eq!(t.mem.iter().map(|a| a.bar).collect::<Vec<_>>(), vec![30]);
    }

    /// Cues read onto the grid and written back land where they were.
    #[test]
    fn cues_round_trip_through_the_grid() {
        let tr = Track { tempos: vec![(0.5, 120.0, 1)], duration: 120.0, ..Default::default() };
        let g = Grid::of(&tr);
        let cues: Vec<Raw> = vec![(0, 0.5, None), (3, 17.0, Some(49.0)), (-1, 0.5, None), (-1, 9.0, None)];
        let t = target_of(&cues, &g, 100);
        assert_eq!(t.hot[0], Some(At { bar: 0, beat: 0, lp: 0 }));
        assert_eq!(t.hot[3], Some(At { bar: 8, beat: 1, lp: 5 }));
        assert_eq!(t.mem.len(), 2);
        let back: Vec<Raw> = cue_list(&marks_of(&t, &g));
        assert!(back.contains(&(0, 0.5, None)) && back.contains(&(3, 17.0, Some(49.0))) && back.contains(&(-1, 9.0, None)));
    }

    #[test]
    fn the_shipped_model_loads() {
        let m = Model::from_bytes(SHIPPED).unwrap();
        assert_eq!((m.nets.len(), m.mu.len(), m.n_tracks), (3, 77, 646));
    }

    #[test]
    fn the_grid_digest_matches_the_prototype() {
        // cues.grid_digest([(0.05, 128.0, 1)]) in Python
        assert_eq!(grid_digest(&[(0.05, 128.0, 1)]).len(), 12);
    }
}
