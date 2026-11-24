//! Training the cue network on your own cues, as the prototype did in PyTorch
//! (`fit` in its cues.py): the same network, losses and optimiser, with the
//! gradients written out by hand. It writes the model file `cues::Model` reads.
//!
//! The network, per track (every bar at once): Linear(d, 64) -> GELU -> dropout
//! -> 2-layer bidirectional GRU(64), dropout between the layers -> Linear(128, 3).
//! Per bar it scores: a transition, the mix-in, the mix-out. Losses: binary
//! cross-entropy on transitions (positives weighted 20, mean over bars), and
//! cross-entropy over the bars for the mix-in (your first transition) and the
//! mix-out (your last). AdamW, 2e-3, weight decay 1e-2, 60 epochs, batches of 16
//! tracks, three networks from three seeds, averaged at use.
//!
//! One difference: PyTorch padded a batch's tracks to the longest, so the
//! backward direction of a short track first read padding; here each track is
//! read as it is, as it is at use.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
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

/// One track to learn from: its per-bar features and your transition bars.
pub struct Example {
    pub rel: String,
    pub x: Vec<Vec<f32>>,
    pub t: Vec<usize>,
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
        let out_w = add("out.weight".into(), vec![3, 2 * H]);
        let out_b = add("out.bias".into(), vec![3]);
        Layout { d, inp_w, inp_b, rnn, out_w, out_b, size: off, names }
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

/// One track's loss and its gradient added into `g`. `wt` weighs the transition
/// loss (one over the batch's bars), `wc` the mix-in and mix-out (one over its
/// tracks), as PyTorch's means over the batch did.
#[allow(clippy::too_many_arguments)]
fn track_grad(lay: &Layout, p: &[f32], g: &mut [f32], x: &[f32], n: usize, t: &[usize], wt: f32, wc: f32, rng: Option<&mut Rng>) -> f32 {
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
    // scores
    let mut o = vec![[0f32; 3]; n];
    for s in 0..n {
        for k in 0..3 {
            o[s][k] = dot(&p[lay.out_w + k * 2 * H..lay.out_w + (k + 1) * 2 * H], &y1[s * 2 * H..(s + 1) * 2 * H]) + p[lay.out_b + k];
        }
    }
    // losses and their gradient on the scores
    let mut is_t = vec![false; n];
    for &b in t {
        is_t[b] = true;
    }
    let mut dout = vec![[0f32; 3]; n];
    let mut loss = 0f32;
    for s in 0..n {
        let (v, y) = (o[s][0], is_t[s] as u8 as f32);
        loss += wt * (POS_WEIGHT * y * softplus(-v) + (1.0 - y) * softplus(v));
        let sg = sigmoid(v);
        dout[s][0] = wt * ((1.0 - y) * sg - POS_WEIGHT * y * (1.0 - sg));
    }
    for (k, target) in [(1, t[0]), (2, *t.last().unwrap())] {
        let mx = o.iter().map(|r| r[k]).fold(f32::NEG_INFINITY, f32::max);
        let sum: f32 = o.iter().map(|r| (r[k] - mx).exp()).sum();
        loss += wc * (mx + sum.ln() - o[target][k]);
        for s in 0..n {
            dout[s][k] = wc * ((o[s][k] - mx).exp() / sum - (s == target) as u8 as f32);
        }
    }
    // back: output layer
    let mut dy1 = vec![0f32; n * 2 * H];
    for s in 0..n {
        for k in 0..3 {
            let dk = dout[s][k];
            g[lay.out_b + k] += dk;
            axpy(dk, &y1[s * 2 * H..(s + 1) * 2 * H], &mut g[lay.out_w + k * 2 * H..lay.out_w + (k + 1) * 2 * H]);
            axpy(dk, &p[lay.out_w + k * 2 * H..lay.out_w + (k + 1) * 2 * H], &mut dy1[s * 2 * H..(s + 1) * 2 * H]);
        }
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
                    track_grad(lay, p, &mut g, &xs[i], xs[i].len() / lay.d, &data[i].t, wt, wc, Some(&mut rng));
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
        "format": "vibecheck-cue-model/1", "d": lay.d, "n_tracks": n_tracks, "source": source,
        "version": cues::VERSION, "nets": nets.len(), "tensors": tensors,
    });
    let mut out = head.to_string().into_bytes();
    out.push(b'\n');
    out.extend(data.iter().flat_map(|v| v.to_le_bytes()));
    out
}

// --- your cues in, a model out --------------------------------------------------------

/// Your own cues (`cues::yours`: not generated, and not this app's left where they were
/// written) on tracks in your library, so ones it can read: the transition bars behind them.
pub fn yours(root: &Path, xml: &HashMap<String, Track>, cfg: &Config) -> Res<HashMap<String, Vec<usize>>> {
    let con = store::labels_db(root)?;
    let hb = store::hash_by_path(&con)?;
    let mut own = cues::yours(xml, &store::cues_written(&con)?, &cfg.ints("cues", "lead", &[0]));
    own.retain(|r, _| hb.contains_key(r));
    Ok(own)
}

/// The tracks to learn from: `yours`, with their features, from the cache or the audio.
pub fn examples(root: &Path, xml: &HashMap<String, Track>, cfg: &Config) -> Res<Vec<Example>> {
    let own = yours(root, xml, cfg)?;
    let tracks: HashMap<String, Track> = own.keys().filter_map(|r| Some((r.clone(), xml.get(r)?.clone()))).collect();
    let (f, _) = cues::features_all(root, &tracks)?;
    let mut out: Vec<Example> = f
        .into_iter()
        .filter_map(|(rel, x)| {
            let t: Vec<usize> = own[&rel].iter().copied().filter(|b| *b < x.len()).collect();
            (t.len() >= 3).then_some(Example { rel, x, t })
        })
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

pub struct Trained {
    pub tracks: usize,
}

/// Learn from your cues and save the model as yours; it takes over from the shipped one.
pub fn train(root: &Path, xml: &HashMap<String, Track>, cfg: &Config) -> Res<Trained> {
    let data = examples(root, xml, cfg)?;
    if data.len() < 20 {
        return Err(format!("only {} tracks with your cues to learn from", data.len()).into());
    }
    let bytes = fit(&data, "yours", SEEDS, EPOCHS)?;
    cues::Model::from_bytes(&bytes)?;
    let path = store::vibecheck_dir(root)?.join(cues::MODEL_FILE);
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(Trained { tracks: data.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-written gradient against finite differences of the loss, on a
    /// short random track, in a few places of every kind of parameter.
    #[test]
    fn gradient_matches_finite_differences() {
        let (d, n) = (5, 7);
        let lay = Layout::new(d);
        let mut rng = Rng::new(3);
        let p = lay.init(&mut rng);
        let x: Vec<f32> = (0..n * d).map(|_| rng.unit() * 2.0 - 1.0).collect();
        let t = [1, 3, 5];
        let (wt, wc) = (1.0 / n as f32, 1.0);
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
                let e = 1e-2f32;
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

    /// A trained model file reads back as the model the app uses.
    #[test]
    fn model_file_round_trips() {
        let mut rng = Rng::new(9);
        let data: Vec<Example> = (0..4)
            .map(|i| Example {
                rel: i.to_string(),
                x: (0..20).map(|_| (0..6).map(|_| rng.unit()).collect()).collect(),
                t: vec![2, 9, 15],
            })
            .collect();
        let b = fit(&data, "test", 1, 1).unwrap();
        let m = cues::Model::from_bytes(&b).unwrap();
        assert_eq!(m.n_tracks, 4);
        assert_eq!(m.scores(&data[0].x).len(), 20);
    }
}
