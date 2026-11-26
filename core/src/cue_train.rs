//! Training the cue network on your cues, with the gradients written out by hand. It
//! writes the model file `cues::Model` reads (v2, `cues::FORMAT_V2`).
//!
//! The network, per track (every bar at once), as the prototype's (`fit` in its cues.py):
//! Linear(d, 64) -> GELU -> dropout -> 2-layer bidirectional GRU(64), dropout between the
//! layers -> Linear(128, K) per bar, and Linear(128, 8) on the mean over the bars.
//!
//! What it learns is the cues themselves (docs/cues.md, "the model, generic"), not
//! transition points with placement settings around them (v1):
//!
//! - per hot cue slot A..H, where it goes: cross-entropy over the bars plus "empty" (the
//!   pooled output), as the prototype chose the mix-in;
//! - a hot cue on this bar at all: binary cross-entropy, positives weighted 20 (ranks bars);
//! - a memory cue on this bar: binary cross-entropy, unweighted (read as a probability);
//! - per cue, the beat in its bar and its loop length (per slot; memory cues share one);
//!   per hot cue, which slot it is in (decoding picks slots by it).
//!
//! Each track's losses are weighted by trust x rating as colours are (`app::Trust`,
//! `app::rating_factor`). AdamW, 2e-3, weight decay 1e-2, 60 epochs, batches of 16
//! tracks, three networks from three seeds, averaged at use.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::app::{Trust, rating_factor, recency};
use crate::config::Config;
use crate::cues::{At, Grid, LOOP_BEATS, N_HOT, Raw, Target, out};
use crate::rekordbox::Track;
use crate::{Res, cues, store};

const H: usize = 64;
const G: usize = 3 * H;
const POS_WEIGHT: f32 = 20.0;
const DROPOUT: f32 = 0.2;
const LR: f32 = 2e-3;
const DECAY: f32 = 1e-2;
const BATCH: usize = 16;
pub const EPOCHS: usize = 60;
pub const SEEDS: usize = 3;

const K: usize = out::K;
const LOOPS: usize = LOOP_BEATS.len();

/// One track to learn from: its per-bar features, your cues on its bars, and what they are
/// worth (trust x rating).
pub struct Example {
    pub rel: String,
    pub x: Vec<Vec<f32>>,
    pub target: Target,
    pub w: f32,
    /// set by hand (rekordbox, or changed in the app), not a suggestion kept: what accuracy
    /// is measured on
    pub hand: bool,
    pub trust: Trust,
}

// --- the parameters, laid out as the model file names them -----------------------

#[derive(Clone, Copy)]
struct Dir {
    wih: usize,
    whh: usize,
    bih: usize,
    bhh: usize,
    inp: usize,
}

struct Layout {
    d: usize,
    inp_w: usize,
    inp_b: usize,
    rnn: [[Dir; 2]; 2],
    out_w: usize,
    out_b: usize,
    emp_w: usize,
    emp_b: usize,
    size: usize,
    names: Vec<(String, Vec<usize>, usize)>,
}

impl Layout {
    fn new(d: usize) -> Layout {
        let mut names: Vec<(String, Vec<usize>, usize)> = vec![];
        let mut off = 0;
        let mut add = |name: String, shape: Vec<usize>| {
            let o = off;
            off += shape.iter().product::<usize>();
            names.push((name, shape, o));
            o
        };
        let inp_w = add("inp.0.weight".into(), vec![H, d]);
        let inp_b = add("inp.0.bias".into(), vec![H]);
        let mut rnn = [[Dir { wih: 0, whh: 0, bih: 0, bhh: 0, inp: 0 }; 2]; 2];
        for (l, layer) in rnn.iter_mut().enumerate() {
            let inp = if l == 0 { H } else { 2 * H };
            for (k, s) in ["", "_reverse"].iter().enumerate() {
                layer[k] = Dir {
                    wih: add(format!("rnn.weight_ih_l{l}{s}"), vec![G, inp]),
                    whh: add(format!("rnn.weight_hh_l{l}{s}"), vec![G, H]),
                    bih: add(format!("rnn.bias_ih_l{l}{s}"), vec![G]),
                    bhh: add(format!("rnn.bias_hh_l{l}{s}"), vec![G]),
                    inp,
                };
            }
        }
        let out_w = add("out.weight".into(), vec![K, 2 * H]);
        let out_b = add("out.bias".into(), vec![K]);
        let emp_w = add("empty.weight".into(), vec![N_HOT, 2 * H]);
        let emp_b = add("empty.bias".into(), vec![N_HOT]);
        Layout { d, inp_w, inp_b, rnn, out_w, out_b, emp_w, emp_b, size: off, names }
    }

    /// PyTorch's default initialisation: uniform within 1/sqrt(fan-in).
    fn init(&self, rng: &mut Rng) -> Vec<f32> {
        let mut p = vec![0f32; self.size];
        for (name, shape, o) in &self.names {
            let fan = if name.starts_with("rnn.") {
                H
            } else if name.starts_with("inp.") {
                self.d
            } else {
                2 * H
            };
            let b = 1.0 / (fan as f32).sqrt();
            for v in &mut p[*o..*o + shape.iter().product::<usize>()] {
                *v = (rng.unit() * 2.0 - 1.0) * b;
            }
        }
        p
    }
}

/// xorshift64*: initialisation, shuffling and dropout, reproducible per seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

// --- small vector maths, written so the compiler can vectorise it ---------------

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// y += a x
fn axpy(a: f32, x: &[f32], y: &mut [f32]) {
    for (y, x) in y.iter_mut().zip(x) {
        *y += a * x;
    }
}

fn matvec(w: &[f32], x: &[f32], out: &mut [f32]) {
    let c = x.len();
    for (r, o) in out.iter_mut().enumerate() {
        *o = dot(&w[r * c..(r + 1) * c], x);
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn softplus(x: f32) -> f32 {
    x.max(0.0) + (-x.abs()).exp().ln_1p()
}

fn gelu(x: f32) -> f32 {
    (0.5 * x as f64 * (1.0 + cues::erf(x as f64 / std::f64::consts::SQRT_2))) as f32
}

fn gelu_grad(x: f32) -> f32 {
    let x = x as f64;
    let cdf = 0.5 * (1.0 + cues::erf(x / std::f64::consts::SQRT_2));
    let pdf = (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt();
    (cdf + x * pdf) as f32
}

// --- one track forward and back ----------------------------------------------------

/// What one direction of one GRU layer kept for the way back, per step in the
/// order it read the bars.
struct DirCache {
    r: Vec<f32>,
    z: Vec<f32>,
    n: Vec<f32>,
    ghn: Vec<f32>,
    h: Vec<f32>, // (steps + 1) x H, the state before each step, then the last
}

fn bar(s: usize, n: usize, rev: bool) -> usize {
    if rev { n - 1 - s } else { s }
}

/// One direction over the sequence `xs` (n x in): writes its states into `y`
/// (n x 2H) at column `off`.
fn dir_forward(p: &[f32], l: Dir, xs: &[f32], n: usize, rev: bool, y: &mut [f32], off: usize) -> DirCache {
    let inp = l.inp;
    let (wih, whh) = (&p[l.wih..l.wih + G * inp], &p[l.whh..l.whh + G * H]);
    let (bih, bhh) = (&p[l.bih..l.bih + G], &p[l.bhh..l.bhh + G]);
    let mut c = DirCache { r: vec![0.0; n * H], z: vec![0.0; n * H], n: vec![0.0; n * H], ghn: vec![0.0; n * H], h: vec![0.0; (n + 1) * H] };
    let (mut gi, mut gh) = (vec![0f32; G], vec![0f32; G]);
    for s in 0..n {
        let t = bar(s, n, rev);
        matvec(wih, &xs[t * inp..(t + 1) * inp], &mut gi);
        let (prev, next) = c.h.split_at_mut((s + 1) * H);
        let hp = &prev[s * H..];
        matvec(whh, hp, &mut gh);
        for j in 0..H {
            let r = sigmoid(gi[j] + bih[j] + gh[j] + bhh[j]);
            let z = sigmoid(gi[H + j] + bih[H + j] + gh[H + j] + bhh[H + j]);
            let ghn = gh[2 * H + j] + bhh[2 * H + j];
            let nn = (gi[2 * H + j] + bih[2 * H + j] + r * ghn).tanh();
            let h = (1.0 - z) * nn + z * hp[j];
            c.r[s * H + j] = r;
            c.z[s * H + j] = z;
            c.n[s * H + j] = nn;
            c.ghn[s * H + j] = ghn;
            next[j] = h;
            y[t * 2 * H + off + j] = h;
        }
    }
    c
}

/// The way back through one direction: `dy` (n x 2H) is the gradient on its
/// output at column `off`; adds the gradient on its input to `dx` (n x in).
#[allow(clippy::too_many_arguments)]
fn dir_backward(p: &[f32], g: &mut [f32], l: Dir, xs: &[f32], n: usize, rev: bool, c: &DirCache, dy: &[f32], off: usize, dx: &mut [f32]) {
    let inp = l.inp;
    let mut dh = vec![0f32; H];
    let (mut dgi, mut dgh, mut dhp) = (vec![0f32; G], vec![0f32; G], vec![0f32; H]);
    for s in (0..n).rev() {
        let t = bar(s, n, rev);
        for j in 0..H {
            dh[j] += dy[t * 2 * H + off + j];
        }
        let hp = &c.h[s * H..(s + 1) * H];
        for j in 0..H {
            let k = s * H + j;
            let (r, z, nn, ghn) = (c.r[k], c.z[k], c.n[k], c.ghn[k]);
            let dn = dh[j] * (1.0 - z);
            let dz = dh[j] * (hp[j] - nn);
            dhp[j] = dh[j] * z;
            let dan = dn * (1.0 - nn * nn);
            let dr = dan * ghn;
            let dar = dr * r * (1.0 - r);
            let daz = dz * z * (1.0 - z);
            dgi[j] = dar;
            dgi[H + j] = daz;
            dgi[2 * H + j] = dan;
            dgh[j] = dar;
            dgh[H + j] = daz;
            dgh[2 * H + j] = dan * r;
        }
        let x = &xs[t * inp..(t + 1) * inp];
        let dxt = &mut dx[t * inp..(t + 1) * inp];
        for row in 0..G {
            let (a, b) = (dgi[row], dgh[row]);
            g[l.bih + row] += a;
            g[l.bhh + row] += b;
            axpy(a, x, &mut g[l.wih + row * inp..l.wih + (row + 1) * inp]);
            axpy(b, hp, &mut g[l.whh + row * H..l.whh + (row + 1) * H]);
            axpy(a, &p[l.wih + row * inp..l.wih + (row + 1) * inp], dxt);
            axpy(b, &p[l.whh + row * H..l.whh + (row + 1) * H], &mut dhp);
        }
        dh.copy_from_slice(&dhp);
    }
}

/// Dropout keep-masks, scaled; all ones when not training.
fn mask(len: usize, rng: Option<&mut Rng>) -> Vec<f32> {
    match rng {
        None => vec![1.0; len],
        Some(rng) => (0..len).map(|_| if rng.unit() < DROPOUT { 0.0 } else { 1.0 / (1.0 - DROPOUT) }).collect(),
    }
}

/// Cross-entropy over `len` scores at `off`, target `target`, weighted `w`; its gradient
/// set into `dout`.
fn ce(o: &[f32], dout: &mut [f32], off: usize, len: usize, target: usize, w: f32) -> f32 {
    let v = &o[off..off + len];
    let mx = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let ln = mx + v.iter().map(|x| (x - mx).exp()).sum::<f32>().ln();
    for (j, x) in v.iter().enumerate() {
        dout[off + j] += w * ((x - ln).exp() - (j == target) as u8 as f32);
    }
    w * (ln - v[target])
}

/// One track's loss and its gradient added into `g`. `wt` weighs the per-bar losses (one
/// over the batch's bars), `wc` the per-cue ones (one over its tracks), both times the
/// track's weight.
#[allow(clippy::too_many_arguments)]
fn track_grad(lay: &Layout, p: &[f32], g: &mut [f32], x: &[f32], n: usize, t: &Target, wt: f32, wc: f32, rng: Option<&mut Rng>) -> f32 {
    let d = lay.d;
    let (mut rng_a, rng) = match rng {
        Some(r) => {
            let a = Rng::new(r.next());
            (Some(a), Some(r))
        }
        None => (None, None),
    };
    // input layer
    let mut a0 = vec![0f32; n * H];
    for s in 0..n {
        matvec(&p[lay.inp_w..lay.inp_w + H * d], &x[s * d..(s + 1) * d], &mut a0[s * H..(s + 1) * H]);
        for j in 0..H {
            a0[s * H + j] += p[lay.inp_b + j];
        }
    }
    let m0 = mask(n * H, rng_a.as_mut());
    let u: Vec<f32> = a0.iter().zip(&m0).map(|(a, m)| gelu(*a) * m).collect();
    // GRU layers
    let mut y0 = vec![0f32; n * 2 * H];
    let c00 = dir_forward(p, lay.rnn[0][0], &u, n, false, &mut y0, 0);
    let c01 = dir_forward(p, lay.rnn[0][1], &u, n, true, &mut y0, H);
    let m1 = mask(n * 2 * H, rng);
    let y0d: Vec<f32> = y0.iter().zip(&m1).map(|(a, m)| a * m).collect();
    let mut y1 = vec![0f32; n * 2 * H];
    let c10 = dir_forward(p, lay.rnn[1][0], &y0d, n, false, &mut y1, 0);
    let c11 = dir_forward(p, lay.rnn[1][1], &y0d, n, true, &mut y1, H);
    // scores per bar, and per slot that it stays empty (from the mean over the bars)
    let mut o = vec![0f32; n * K];
    for s in 0..n {
        matvec(&p[lay.out_w..lay.out_w + K * 2 * H], &y1[s * 2 * H..(s + 1) * 2 * H], &mut o[s * K..(s + 1) * K]);
        axpy(1.0, &p[lay.out_b..lay.out_b + K], &mut o[s * K..(s + 1) * K]);
    }
    let mut pool = vec![0f32; 2 * H];
    for s in 0..n {
        axpy(1.0 / n as f32, &y1[s * 2 * H..(s + 1) * 2 * H], &mut pool);
    }
    let mut e = [0f32; N_HOT];
    matvec(&p[lay.emp_w..lay.emp_w + N_HOT * 2 * H], &pool, &mut e);
    axpy(1.0, &p[lay.emp_b..lay.emp_b + N_HOT], &mut e);
    // losses and their gradient on the scores
    let (mut dout, mut de) = (vec![0f32; n * K], [0f32; N_HOT]);
    let mut loss = 0f32;
    // each slot: a choice over the bars and empty
    for k in 0..N_HOT {
        let target = t.hot[k].map_or(n, |a| a.bar);
        let logit = |s: usize| if s < n { o[s * K + out::SLOT + k] } else { e[k] };
        let mx = (0..=n).map(logit).fold(f32::NEG_INFINITY, f32::max);
        let ln = mx + (0..=n).map(|s| (logit(s) - mx).exp()).sum::<f32>().ln();
        loss += wc * (ln - logit(target));
        for s in 0..=n {
            let d = wc * ((logit(s) - ln).exp() - (s == target) as u8 as f32);
            if s < n { dout[s * K + out::SLOT + k] = d } else { de[k] = d }
        }
    }
    // a hot cue here (ranked, positives weighted), a memory cue here (a probability)
    let (mut hot, mut mem) = (vec![0f32; n], vec![0f32; n]);
    t.hot.iter().flatten().for_each(|a| hot[a.bar] = 1.0);
    t.mem.iter().for_each(|a| mem[a.bar] = 1.0);
    for (off, y, pw) in [(out::HOT, &hot, POS_WEIGHT), (out::MEM, &mem, 1.0)] {
        for s in 0..n {
            let (v, y) = (o[s * K + off], y[s]);
            loss += wt * (pw * y * softplus(-v) + (1.0 - y) * softplus(v));
            let sg = sigmoid(v);
            dout[s * K + off] = wt * ((1.0 - y) * sg - pw * y * (1.0 - sg));
        }
    }
    // each cue: the beat in its bar, its loop length
    let all: Vec<(Option<usize>, At)> = t.all().collect();
    let wq = wc / all.len().max(1) as f32;
    for (k, a) in &all {
        let lp = k.map_or(out::MEM_LOOP, |k| out::HOT_LOOP + k * LOOPS);
        loss += ce(&o, &mut dout, a.bar * K + out::BEAT, 4, a.beat, wq);
        loss += ce(&o, &mut dout, a.bar * K + lp, LOOPS, a.lp, wq);
        if let Some(k) = k {
            loss += ce(&o, &mut dout, a.bar * K + out::WHICH, N_HOT, *k, wq);
        }
    }
    // back: output layers
    let mut dy1 = vec![0f32; n * 2 * H];
    for s in 0..n {
        for k in 0..K {
            let dk = dout[s * K + k];
            if dk == 0.0 {
                continue;
            }
            g[lay.out_b + k] += dk;
            axpy(dk, &y1[s * 2 * H..(s + 1) * 2 * H], &mut g[lay.out_w + k * 2 * H..lay.out_w + (k + 1) * 2 * H]);
            axpy(dk, &p[lay.out_w + k * 2 * H..lay.out_w + (k + 1) * 2 * H], &mut dy1[s * 2 * H..(s + 1) * 2 * H]);
        }
    }
    let mut dpool = vec![0f32; 2 * H];
    for k in 0..N_HOT {
        g[lay.emp_b + k] += de[k];
        axpy(de[k], &pool, &mut g[lay.emp_w + k * 2 * H..lay.emp_w + (k + 1) * 2 * H]);
        axpy(de[k], &p[lay.emp_w + k * 2 * H..lay.emp_w + (k + 1) * 2 * H], &mut dpool);
    }
    for s in 0..n {
        axpy(1.0 / n as f32, &dpool, &mut dy1[s * 2 * H..(s + 1) * 2 * H]);
    }
    // GRU layers
    let mut dy0d = vec![0f32; n * 2 * H];
    dir_backward(p, g, lay.rnn[1][0], &y0d, n, false, &c10, &dy1, 0, &mut dy0d);
    dir_backward(p, g, lay.rnn[1][1], &y0d, n, true, &c11, &dy1, H, &mut dy0d);
    let dy0: Vec<f32> = dy0d.iter().zip(&m1).map(|(a, m)| a * m).collect();
    let mut du = vec![0f32; n * H];
    dir_backward(p, g, lay.rnn[0][0], &u, n, false, &c00, &dy0, 0, &mut du);
    dir_backward(p, g, lay.rnn[0][1], &u, n, true, &c01, &dy0, H, &mut du);
    // input layer
    for s in 0..n {
        for j in 0..H {
            let da = du[s * H + j] * m0[s * H + j] * gelu_grad(a0[s * H + j]);
            g[lay.inp_b + j] += da;
            axpy(da, &x[s * d..(s + 1) * d], &mut g[lay.inp_w + j * d..lay.inp_w + (j + 1) * d]);
        }
    }
    loss
}

// --- training ----------------------------------------------------------------------

/// Progress of the running training for the app to show: steps done, of all.
pub static DONE: AtomicUsize = AtomicUsize::new(0);
pub static TOTAL: AtomicUsize = AtomicUsize::new(0);

/// Train the networks on `data` and return the model file's bytes, `source`
/// naming whose cues they learned from. Gives up when the app closes.
pub fn fit(data: &[Example], source: &str, seeds: usize, epochs: usize) -> Res<Vec<u8>> {
    let d = data.first().ok_or("no tracks to learn from")?.x[0].len();
    let rows = data.iter().map(|e| e.x.len()).sum::<usize>() as f64;
    let mut mu = vec![0f64; d];
    for e in data {
        for r in &e.x {
            for (m, v) in mu.iter_mut().zip(r) {
                *m += *v as f64;
            }
        }
    }
    mu.iter_mut().for_each(|m| *m /= rows);
    let mut var = vec![0f64; d];
    for e in data {
        for r in &e.x {
            for (j, v) in r.iter().enumerate() {
                var[j] += (*v as f64 - mu[j]).powi(2);
            }
        }
    }
    let sd: Vec<f32> = var.iter().map(|v| ((v / rows).sqrt() + 1e-6) as f32).collect();
    let mu: Vec<f32> = mu.iter().map(|m| *m as f32).collect();
    let xs: Vec<Vec<f32>> = data.iter().map(|e| e.x.iter().flat_map(|r| r.iter().enumerate().map(|(j, v)| (v - mu[j]) / sd[j])).collect()).collect();
    let lay = Layout::new(d);
    let batches = data.len().div_ceil(BATCH);
    TOTAL.store(seeds * epochs * batches, Ordering::Relaxed);
    DONE.store(0, Ordering::Relaxed);
    // the networks side by side, each spreading its batches over a share of the cores
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let workers = cores.div_ceil(seeds).max(1);
    let one = |seed: usize| -> Option<Vec<f32>> {
        let mut p = lay.init(&mut Rng::new(seed as u64 + 1));
        let (mut m, mut v) = (vec![0f32; lay.size], vec![0f32; lay.size]);
        let mut step = 0;
        for ep in 0..epochs {
            let mut order: Vec<usize> = (0..data.len()).collect();
            Rng::new((seed * 100 + ep) as u64 + 7).shuffle(&mut order);
            for (bi, b) in order.chunks(BATCH).enumerate() {
                if crate::stopping() {
                    return None;
                }
                // per track times its own weight, in batch_grad
                let wt = 1.0 / b.iter().map(|&i| xs[i].len() / d).sum::<usize>() as f32;
                let wc = 1.0 / b.len() as f32;
                let g = batch_grad(&lay, &p, &xs, data, b, wt, wc, ((seed * epochs + ep) * batches + bi) as u64, workers);
                step += 1;
                adamw(&mut p, &g, &mut m, &mut v, step);
                DONE.fetch_add(1, Ordering::Relaxed);
            }
        }
        Some(p)
    };
    let nets: Option<Vec<Vec<f32>>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..seeds).map(|seed| s.spawn(move || one(seed))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let nets = nets.ok_or("stopped")?;
    Ok(to_bytes(&lay, &mu, &sd, &nets, data.len(), source))
}

/// The batch's summed gradient, its tracks spread over `workers` threads; summed in the
/// batch's order, so a seed trains the same every time.
#[allow(clippy::too_many_arguments)]
fn batch_grad(lay: &Layout, p: &[f32], xs: &[Vec<f32>], data: &[Example], b: &[usize], wt: f32, wc: f32, seed: u64, workers: usize) -> Vec<f32> {
    let next = AtomicUsize::new(0);
    let each: Vec<Mutex<Vec<f32>>> = b.iter().map(|_| Mutex::new(vec![])).collect();
    std::thread::scope(|s| {
        for _ in 0..workers.min(b.len()) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    if k >= b.len() {
                        break;
                    }
                    let i = b[k];
                    let mut g = vec![0f32; lay.size];
                    let mut rng = Rng::new(seed.wrapping_mul(1_000_003).wrapping_add(i as u64));
                    let w = data[i].w;
                    track_grad(lay, p, &mut g, &xs[i], xs[i].len() / lay.d, &data[i].target, wt * w, wc * w, Some(&mut rng));
                    *each[k].lock().unwrap() = g;
                }
            });
        }
    });
    let mut total = vec![0f32; lay.size];
    for g in each {
        for (a, x) in total.iter_mut().zip(g.into_inner().unwrap().iter()) {
            *a += x;
        }
    }
    total
}

/// PyTorch's AdamW with its defaults (betas 0.9, 0.999; eps 1e-8), decay on every parameter.
fn adamw(p: &mut [f32], g: &[f32], m: &mut [f32], v: &mut [f32], step: i32) {
    let (b1, b2) = (0.9f32, 0.999f32);
    let (c1, c2) = (1.0 - b1.powi(step), 1.0 - b2.powi(step));
    for i in 0..p.len() {
        p[i] *= 1.0 - LR * DECAY;
        m[i] = b1 * m[i] + (1.0 - b1) * g[i];
        v[i] = b2 * v[i] + (1.0 - b2) * g[i] * g[i];
        p[i] -= LR * (m[i] / c1) / ((v[i] / c2).sqrt() + 1e-8);
    }
}

/// The model file: one line of JSON (what it is, and per tensor its shape and
/// offset in floats), a newline, then every tensor as little-endian f32.
fn to_bytes(lay: &Layout, mu: &[f32], sd: &[f32], nets: &[Vec<f32>], n_tracks: usize, source: &str) -> Vec<u8> {
    let mut tensors = vec![
        serde_json::json!({"name": "mu", "shape": [lay.d], "offset": 0}),
        serde_json::json!({"name": "sd", "shape": [lay.d], "offset": lay.d}),
    ];
    let mut data: Vec<f32> = mu.iter().chain(sd).copied().collect();
    for (i, p) in nets.iter().enumerate() {
        for (name, shape, o) in &lay.names {
            tensors.push(serde_json::json!({"name": format!("net{i}.{name}"), "shape": shape, "offset": data.len()}));
            data.extend_from_slice(&p[*o..*o + shape.iter().product::<usize>()]);
        }
    }
    let head = serde_json::json!({
        "format": cues::FORMAT_V2, "d": lay.d, "n_tracks": n_tracks, "source": source,
        "version": cues::VERSION, "nets": nets.len(), "tensors": tensors, "trained": crate::now(),
    });
    let mut out = head.to_string().into_bytes();
    out.push(b'\n');
    out.extend(data.iter().flat_map(|v| v.to_le_bytes()));
    out
}

// --- your cues in, a model out --------------------------------------------------------

/// A track's cues to learn from, where they came from, and how far they are trusted.
pub struct Source {
    pub cues: Vec<Raw>,
    pub trust: Trust,
    pub hand: bool,
    /// when that last changed (set, edited, written, played), unix seconds; 0: before vibecheck
    pub at: i64,
}

/// What a track's cues are worth, as for colours (docs/cues.md): set in the app (the
/// suggestion kept: kept; changed: confirmed), changed in rekordbox, after we wrote them or
/// as a sync saw (confirmed), there before vibecheck (existing), played since (played).
/// Ours, written and left alone, are not learned from. `decided`: (cues, suggestion kept,
/// when); `theirs`: rekordbox's cues when they are not ours as written; `written`: when we
/// last wrote some; `edited`: when a sync last saw them change.
pub fn source(
    decided: Option<(Vec<Raw>, bool, i64)>,
    theirs: Option<Vec<Raw>>,
    written: Option<i64>,
    edited: Option<i64>,
    played: Option<i64>,
) -> Option<Source> {
    let changed = written.max(edited);
    let (cues, trust, since, hand) = match (decided, theirs) {
        // set in the app after rekordbox's last change, or written and left as set
        (Some((c, kept, ts)), th) if changed.is_none_or(|w| ts >= w) || th.is_none() => {
            (c, if kept { Trust::Kept } else { Trust::Confirmed }, ts, !kept)
        }
        (_, Some(c)) => (c, if changed.is_some() { Trust::Confirmed } else { Trust::Existing }, changed.unwrap_or(0), true),
        _ => return None,
    };
    let (trust, at) = match played.filter(|p| *p > since) {
        Some(p) => (Trust::Played, p),
        None => (trust, since),
    };
    Some(Source { cues, trust, hand, at })
}

/// Your cues on tracks in your library with a beat grid in rekordbox (none: nothing to
/// learn on), from the app and from rekordbox, with their trust.
pub fn yours(root: &Path, xml: &HashMap<String, Track>) -> Res<HashMap<String, Source>> {
    let con = store::labels_db(root)?;
    let hb = store::hash_by_path(&con)?;
    let written = store::cue_writes_last(&con)?;
    let played = store::played_at(&con)?;
    let seen = store::cues_seen(&con)?;
    let parse = |j: &str| -> Vec<Raw> {
        serde_json::from_str::<Vec<serde_json::Value>>(j)
            .unwrap_or_default()
            .iter()
            .filter_map(|c| Some((c["num"].as_i64()?, c["start"].as_f64()?, c["end"].as_f64())))
            .collect()
    };
    let same = |a: &[Raw], b: &[Raw]| {
        let r = |v: &[Raw]| {
            let mut v: Vec<(i64, i64, Option<i64>)> =
                v.iter().map(|c| (c.0, (c.1 * 100.0).round() as i64, c.2.map(|e| (e * 100.0).round() as i64))).collect();
            v.sort();
            v
        };
        r(a) == r(b)
    };
    let decided: HashMap<String, (Vec<Raw>, bool, i64)> = store::cue_decisions_made(&con)?
        .into_iter()
        // Bulk label's (card "guessed") are the model's, not yours
        .filter(|(_, (_, _, _, card))| card.as_deref() != Some("guessed"))
        .map(|(p, (c, shown, ts, _))| {
            let c = parse(&c);
            let kept = shown.is_some_and(|s| same(&c, &parse(&s)));
            (p, (c, kept, ts))
        })
        .collect();
    let mut out = HashMap::new();
    let paths: std::collections::HashSet<&String> = decided.keys().chain(xml.keys()).collect();
    for p in paths {
        if !hb.contains_key(p) || xml.get(p).is_none_or(|t| t.tempos.is_empty()) {
            continue;
        }
        let w = written.get(p);
        let theirs = xml
            .get(p)
            .filter(|t| cues::cue_list(&t.marks).iter().any(|c| c.0 >= 0))
            .filter(|t| !cues::generated(&t.marks) && w.is_none_or(|w| w.0 != cues::hot_starts(&t.marks)))
            .map(|t| cues::cue_list(&t.marks));
        let edited = seen.get(p).and_then(|s| s.1);
        if let Some(s) = source(decided.get(p).cloned(), theirs, w.map(|w| w.1), edited, played.get(p).copied()) {
            out.insert(p.clone(), s);
        }
    }
    Ok(out)
}

/// The tracks to learn from: `yours` on rekordbox's grid, with their features (from the
/// cache or the audio) and weights (trust x rating).
pub fn examples(root: &Path, xml: &HashMap<String, Track>, cfg: &Config) -> Res<Vec<Example>> {
    let own = yours(root, xml)?;
    let stars: HashMap<String, Option<u8>> =
        store::current_made(&store::labels_db(root)?)?.into_iter().map(|(p, c)| (p, c.label.stars)).collect();
    let tracks: HashMap<String, Track> =
        own.keys().filter_map(|r| Some((r.clone(), xml.get(r).filter(|t| !t.tempos.is_empty())?.clone()))).collect();
    let (f, _) = cues::features_all(root, &tracks)?;
    let mut out: Vec<Example> = f
        .into_iter()
        .filter_map(|(rel, x)| {
            let s = &own[&rel];
            let target = cues::target_of(&s.cues, &Grid::of(&tracks[&rel]), x.len());
            // none of its cues on the grid: nothing to learn from (an empty set made in the app is)
            if target.all().next().is_none() && !s.cues.is_empty() {
                return None;
            }
            let w = (s.trust.weight(cfg) * rating_factor(stars.get(&rel).copied().flatten(), cfg) * recency(s.at, cfg)) as f32;
            Some(Example { rel, x, target, w, hand: s.hand, trust: s.trust })
        })
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

pub struct Trained {
    pub tracks: usize,
    pub conflicts: usize,
    pub refresh: usize,
}

// --- where the model now disagrees -----------------------------------------------------

/// A track whose cues the model, having learned them, still puts elsewhere (docs/cues.md):
/// yours of high trust are asked again (a cue conflict), low trust ones (from before
/// vibecheck, or ours left alone) Bulk label refreshes with `cues`. `mine`: the share of the
/// track's hot cues it would set itself; `sure`: how sure it is of its own, on average.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Conflict {
    pub rel: String,
    pub mine: f32,
    pub sure: f32,
    /// what it would set instead
    #[serde(default)]
    pub cues: Vec<Raw>,
}

/// At most this share of the hot cues where the model would set them is a disagreement,
pub const CONFLICT_SHARE: f32 = 0.25;
/// when it is at least this sure of its own, on average: more likely right than not (as a
/// colour conflict needs the model sure of its colour).
pub const CONFLICT_SURE: f32 = 0.5;
const CONFLICTS_FILE: &str = "cue_conflicts.json";
const REFRESH_FILE: &str = "cue_refresh.json";

/// Where `m` disagrees with the cues of `data` (each with its trust), surest first.
pub fn disagree(m: &cues::Model, data: &[Example], xml: &HashMap<String, Track>) -> Vec<(Trust, Conflict)> {
    let next = AtomicUsize::new(0);
    let found = Mutex::new(vec![]);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..cores {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(e) = data.get(i) else { break };
                    let Some(t) = xml.get(&e.rel) else { continue };
                    let mine: Vec<usize> = e.target.hot.iter().flatten().map(|a| a.bar).collect();
                    if mine.len() < 2 {
                        continue;
                    }
                    let (sc, emp) = m.scores(&e.x);
                    let decoded = cues::decode_generic(&sc, &emp);
                    let got: Vec<usize> = decoded.hot.iter().flatten().map(|a| a.bar).collect();
                    let share = mine.iter().filter(|b| got.contains(b)).count() as f32 / mine.len() as f32;
                    if share > CONFLICT_SHARE || got.is_empty() {
                        continue;
                    }
                    // the hot-cue-here score learned with positives weighted: back to a probability
                    let p = |b: usize| sigmoid(sc[b][out::HOT] - POS_WEIGHT.ln());
                    let sure = got.iter().map(|b| p(*b)).sum::<f32>() / got.len() as f32;
                    if sure >= CONFLICT_SURE {
                        let cues = cues::cue_list(&cues::marks_of(&decoded, &Grid::of(t)));
                        found.lock().unwrap().push((e.trust, Conflict { rel: e.rel.clone(), mine: share, sure, cues }));
                    }
                }
            });
        }
    });
    let mut out = found.into_inner().unwrap();
    out.sort_by(|a, b| b.1.sure.total_cmp(&a.1.sure).then_with(|| a.1.rel.cmp(&b.1.rel)));
    out
}

/// Low trust: Bulk label's to change; high: asked again.
pub fn low(t: Trust) -> bool {
    matches!(t, Trust::Existing | Trust::Guessed)
}

fn load_list(root: &Path, file: &str) -> Vec<Conflict> {
    store::vibecheck_dir(root)
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join(file)).ok())
        .and_then(|s| serde_json::from_str::<Vec<Conflict>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.sure >= CONFLICT_SURE)
        .collect()
}

/// The cue conflicts the last training found (high trust), surest first; none before one.
pub fn load_conflicts(root: &Path) -> Vec<Conflict> {
    load_list(root, CONFLICTS_FILE)
}

/// The low trust cues the last training would refresh, with its own, surest first.
pub fn load_refresh(root: &Path) -> Vec<Conflict> {
    load_list(root, REFRESH_FILE)
}

/// Cues of ours left as written (guessed: not learned from) on tracks with a grid, with
/// their features: what the model may refresh.
fn ours_left(root: &Path, xml: &HashMap<String, Track>, own: &HashMap<String, Source>) -> Res<Vec<Example>> {
    let ours = cues::Written::load(&store::labels_db(root)?)?;
    let tracks: HashMap<String, Track> = xml
        .iter()
        .filter(|(p, t)| !own.contains_key(*p) && !t.tempos.is_empty() && !t.marks.is_empty() && ours.untouched(p, t))
        .map(|(p, t)| (p.clone(), t.clone()))
        .collect();
    let (f, _) = cues::features_all(root, &tracks)?;
    Ok(f.into_iter()
        .map(|(rel, x)| {
            let target = cues::target_of(&cues::cue_list(&tracks[&rel].marks), &Grid::of(&tracks[&rel]), x.len());
            Example { rel, x, target, w: 0.0, hand: false, trust: Trust::Guessed }
        })
        .collect())
}

/// Learn from your cues and save the model as yours; it takes over from the shipped one,
/// and the one it replaces is kept next to it (`cue_model.prev.bin`). Then where it
/// disagrees: high trust cues become cue conflicts, low trust ones Bulk label's to refresh.
pub fn train(root: &Path, xml: &HashMap<String, Track>, cfg: &Config) -> Res<Trained> {
    let data = examples(root, xml, cfg)?;
    if data.len() < 20 {
        return Err(format!("only {} tracks with your cues to learn from", data.len()).into());
    }
    let bytes = fit(&data, "yours", SEEDS, EPOCHS)?;
    cues::Model::from_bytes(&bytes)?;
    let dir = store::vibecheck_dir(root)?;
    let path = dir.join(cues::MODEL_FILE);
    if path.exists() {
        std::fs::copy(&path, path.with_extension("prev.bin"))?;
    }
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    let m = cues::Model::from_bytes(&bytes)?;
    let mut all = disagree(&m, &data, xml);
    all.extend(disagree(&m, &ours_left(root, xml, &yours(root, xml)?)?, xml));
    all.sort_by(|a, b| b.1.sure.total_cmp(&a.1.sure).then_with(|| a.1.rel.cmp(&b.1.rel)));
    let (lo, hi): (Vec<_>, Vec<_>) = all.into_iter().partition(|(t, _)| low(*t));
    let (lo, hi): (Vec<Conflict>, Vec<Conflict>) = (lo.into_iter().map(|x| x.1).collect(), hi.into_iter().map(|x| x.1).collect());
    std::fs::write(dir.join(CONFLICTS_FILE), serde_json::to_string(&hi)?)?;
    std::fs::write(dir.join(REFRESH_FILE), serde_json::to_string(&lo)?)?;
    Ok(Trained { tracks: data.len(), conflicts: hi.len(), refresh: lo.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        let at = |bar, beat, lp| At { bar, beat, lp };
        let mut t = Target::default();
        t.hot[0] = Some(at(1, 0, 3));
        t.hot[2] = Some(at(3, 2, 0));
        t.hot[7] = Some(at(5, 0, 5));
        t.mem = vec![at(1, 0, 0), at(6, 1, 2)];
        t
    }

    /// The hand-written gradient against finite differences of the loss, on a
    /// short random track, in a few places of every kind of parameter.
    #[test]
    fn gradient_matches_finite_differences() {
        let (d, n) = (5, 7);
        let lay = Layout::new(d);
        let mut rng = Rng::new(3);
        let p = lay.init(&mut rng);
        let x: Vec<f32> = (0..n * d).map(|_| rng.unit() * 2.0 - 1.0).collect();
        let t = target();
        let (wt, wc) = (0.7 / n as f32, 0.7);
        let mut g = vec![0f32; lay.size];
        track_grad(&lay, &p, &mut g, &x, n, &t, wt, wc, None);
        let loss = |q: &[f32]| {
            let mut s = vec![0f32; lay.size];
            track_grad(&lay, q, &mut s, &x, n, &t, wt, wc, None) as f64
        };
        let mut checked = 0;
        for (name, shape, o) in &lay.names {
            let len: usize = shape.iter().product();
            for k in [0, len / 2, len - 1] {
                let i = o + k;
                // the loss sums many terms in f32: a wider step keeps its rounding below the slope
                let e = 3e-2f32;
                let (mut a, mut b) = (p.clone(), p.clone());
                a[i] += e;
                b[i] -= e;
                let num = (loss(&a) - loss(&b)) / (2.0 * e as f64);
                let ana = g[i] as f64;
                // f32 finite differences carry ~1e-4 of noise: relative to the larger of the two, floored there
                let err = (num - ana).abs() / num.abs().max(ana.abs()).max(1e-2);
                assert!(err < 0.02, "{name}[{k}]: numeric {num:.6} vs analytic {ana:.6}");
                checked += 1;
            }
        }
        assert!(checked > 40);
    }

    /// Where cues come from and what they are worth.
    #[test]
    fn cue_trust_follows_colours() {
        let c = vec![(0, 10.0, None)];
        let t = |d: Option<bool>, dts: i64, theirs: bool, w: Option<i64>, pl: Option<i64>| {
            source(d.map(|k| (c.clone(), k, dts)), theirs.then(|| c.clone()), w, None, pl).map(|s| (s.trust, s.hand))
        };
        // before vibecheck; changed in rekordbox after our write; ours left alone
        assert_eq!(t(None, 0, true, None, None), Some((Trust::Existing, true)));
        assert_eq!(t(None, 0, true, Some(5), None), Some((Trust::Confirmed, true)));
        assert_eq!(t(None, 0, false, Some(5), None), None);
        // set in the app: kept or changed, exported and left, or changed in rekordbox since
        assert_eq!(t(Some(true), 9, false, None, None), Some((Trust::Kept, false)));
        assert_eq!(t(Some(false), 9, false, Some(10), None), Some((Trust::Confirmed, true)));
        assert_eq!(t(Some(true), 9, true, Some(10), None), Some((Trust::Confirmed, true)));
        // played after it was set
        assert_eq!(t(Some(true), 9, false, Some(10), Some(20)), Some((Trust::Played, false)));
        assert_eq!(t(None, 0, true, Some(30), Some(20)), Some((Trust::Confirmed, true)));
        // your own old cues, edited in rekordbox as a sync saw: confirmed from then on
        let e = |d: Option<i64>, edited: Option<i64>, pl: Option<i64>| {
            source(d.map(|ts| (c.clone(), true, ts)), Some(c.clone()), None, edited, pl).map(|s| (s.trust, s.at))
        };
        assert_eq!(e(None, Some(40), None), Some((Trust::Confirmed, 40)));
        assert_eq!(e(None, Some(40), Some(50)), Some((Trust::Played, 50)));
        assert_eq!(e(None, Some(40), Some(30)), Some((Trust::Confirmed, 40)));
        assert_eq!(e(None, None, None), Some((Trust::Existing, 0)));
        // edited in rekordbox after it was set in the app, and the other way round
        assert_eq!(e(Some(30), Some(40), None), Some((Trust::Confirmed, 40)));
        assert_eq!(e(Some(50), Some(40), None), Some((Trust::Kept, 50)));
    }

    /// A trained model file reads back as the model the app uses.
    #[test]
    fn model_file_round_trips() {
        let mut rng = Rng::new(9);
        let data: Vec<Example> = (0..4)
            .map(|i| Example {
                rel: i.to_string(),
                x: (0..20).map(|_| (0..6).map(|_| rng.unit()).collect()).collect(),
                target: target(),
                w: 0.5,
                hand: true,
                trust: Trust::Confirmed,
            })
            .collect();
        let b = fit(&data, "test", 1, 1).unwrap();
        let m = cues::Model::from_bytes(&b).unwrap();
        assert_eq!((m.n_tracks, m.generic), (4, true));
        let (s, e) = m.scores(&data[0].x);
        assert_eq!((s.len(), s[0].len(), e.len()), (20, out::K, N_HOT));
    }
}
