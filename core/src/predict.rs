//! Deciding what to say about a track.
//!
//! Three independent axes: hue (4 values), tone (2), stars (6). Hue and tone
//! form the colour grid; the rating is separate. Each axis is decided on its
//! own, by expected cost: knowing an axis of n values saves log2(n) binary
//! decisions, getting it wrong costs that back plus `misleading_cost`. So an
//! axis is asserted when
//!
//! ```text
//! (1 - p) * (cost + misleading_cost) < cost
//! ```
//!
//! The classifier is scikit-learn's default: features standardised, then L2
//! logistic regression (C = 0.001) -- multinomial for more than two classes,
//! binary for two -- fitted here by L-BFGS to the same optimum.

use std::collections::{BTreeSet, HashMap};

use crate::palette::{self, C as Colour};
use crate::store::Label;

pub const C: f64 = 0.001;
pub const HUE: &str = "hue";
pub const TONE: &str = "tone";
pub const STARS: &str = "stars";

#[derive(Debug, Clone)]
pub struct Axis {
    pub name: &'static str,
    /// what the vocabulary allows, not what was seen
    pub n_values: usize,
    pub misleading_cost: f64,
    /// decisions saved by knowing it
    pub cost: f64,
}

impl Axis {
    fn new(name: &'static str, n_values: usize, misleading_cost: f64) -> Axis {
        Axis { name, n_values, misleading_cost, cost: (n_values.max(1) as f64).log2() }
    }
}

pub fn axes_for(misleading_cost: f64, stars_misleading_cost: f64) -> Vec<Axis> {
    vec![
        Axis::new(HUE, palette::HUES.len(), misleading_cost),
        Axis::new(TONE, palette::TONES.len(), misleading_cost),
        Axis::new(STARS, palette::MAX_STARS as usize + 1, stars_misleading_cost),
    ]
}

/// The value a label states on an axis, as a string; None when it says nothing.
pub fn target(l: &Label, axis: &str) -> Option<String> {
    match axis {
        HUE => l.hue().map(str::to_string),
        TONE => l.tone().map(str::to_string),
        STARS => l.stars.map(|s| s.to_string()),
        _ => None,
    }
}

/// What was asserted on each axis, and the best guess on every axis.
#[derive(Debug, Clone, Default)]
pub struct Prediction {
    pub values: HashMap<&'static str, String>,
    pub confidence: HashMap<&'static str, f64>,
    pub top: HashMap<&'static str, (String, f64)>,
}

impl Prediction {
    pub fn colour(&self) -> Option<Colour> {
        palette::from_grid(self.values.get(HUE).map(String::as_str), self.values.get(TONE).map(String::as_str))
    }
    pub fn stars(&self) -> Option<u8> {
        self.values.get(STARS).and_then(|s| s.parse().ok())
    }
    pub fn label(&self) -> Label {
        Label { colour: self.colour(), stars: self.stars() }
    }
    /// How far the prediction got: a colour, one axis of it, or nothing.
    pub fn specificity(&self) -> &'static str {
        if self.colour().is_some() {
            "colour"
        } else if self.values.contains_key(HUE) {
            "hue"
        } else if self.values.contains_key(TONE) {
            "tone"
        } else {
            "none"
        }
    }
}

/// Standardise, then logistic regression: one fitted classifier.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Lr {
    pub classes: Vec<String>,
    mean: Vec<f64>,
    scale: Vec<f64>,
    /// rows: one per class (multinomial) or one (binary, for classes[1])
    w: Vec<Vec<f64>>,
    b: Vec<f64>,
}

impl Lr {
    pub fn fit(x: &[&[f32]], y: &[String], c: f64) -> Lr {
        let (n, d) = (x.len(), x[0].len());
        let classes: Vec<String> = sort_classes(y.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect());
        let mut mean = vec![0f64; d];
        let mut scale = vec![0f64; d];
        for r in x {
            for (m, v) in mean.iter_mut().zip(r.iter()) {
                *m += *v as f64;
            }
        }
        mean.iter_mut().for_each(|m| *m /= n as f64);
        for r in x {
            for j in 0..d {
                scale[j] += (r[j] as f64 - mean[j]).powi(2);
            }
        }
        for s in scale.iter_mut() {
            *s = (*s / n as f64).sqrt();
            if *s == 0.0 {
                *s = 1.0;
            }
        }
        let xs: Vec<f64> = x.iter().flat_map(|r| (0..d).map(|j| (r[j] as f64 - mean[j]) / scale[j])).collect();
        let yi: Vec<usize> = y.iter().map(|v| classes.iter().position(|c| c == v).unwrap()).collect();
        let k = classes.len();
        let rows = if k == 2 { 1 } else { k };
        // parameters: rows x (d weights + 1 intercept)
        let p = rows * (d + 1);
        let f = |th: &[f64], g: &mut [f64]| -> f64 { objective(th, g, &xs, &yi, n, d, rows, c) };
        let th = lbfgs(vec![0.0; p], f, 5000, 1e-7);
        let w = (0..rows).map(|r| th[r * (d + 1)..r * (d + 1) + d].to_vec()).collect();
        let b = (0..rows).map(|r| th[r * (d + 1) + d]).collect();
        Lr { classes, mean, scale, w, b }
    }

    pub fn predict_proba(&self, x: &[f32]) -> Vec<f64> {
        let z: Vec<f64> = self
            .w
            .iter()
            .zip(&self.b)
            .map(|(w, b)| b + w.iter().enumerate().map(|(j, wj)| wj * (x[j] as f64 - self.mean[j]) / self.scale[j]).sum::<f64>())
            .collect();
        if self.classes.len() == 2 {
            let p = sigmoid(z[0]);
            vec![1.0 - p, p]
        } else {
            softmax(&z)
        }
    }
}

/// np.unique order: numbers numerically (all of stars are digits), else text.
fn sort_classes(mut v: Vec<String>) -> Vec<String> {
    if v.iter().all(|s| s.parse::<i64>().is_ok()) {
        v.sort_by_key(|s| s.parse::<i64>().unwrap());
    }
    v
}

fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}

fn softmax(z: &[f64]) -> Vec<f64> {
    let m = z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let e: Vec<f64> = z.iter().map(|v| (v - m).exp()).collect();
    let s: f64 = e.iter().sum();
    e.into_iter().map(|v| v / s).collect()
}

/// C * sum of log losses + 0.5 * |W|^2 (intercepts unpenalised), and its gradient.
#[allow(clippy::too_many_arguments)]
fn objective(th: &[f64], g: &mut [f64], x: &[f64], y: &[usize], n: usize, d: usize, rows: usize, c: f64) -> f64 {
    g.iter_mut().for_each(|v| *v = 0.0);
    let mut loss = 0.0;
    let mut z = vec![0f64; rows];
    for i in 0..n {
        let xi = &x[i * d..(i + 1) * d];
        for r in 0..rows {
            let w = &th[r * (d + 1)..r * (d + 1) + d];
            z[r] = th[r * (d + 1) + d] + w.iter().zip(xi).map(|(a, b)| a * b).sum::<f64>();
        }
        if rows == 1 {
            let t = if y[i] == 1 { 1.0 } else { 0.0 };
            // log(1 + e^z) - t z, stably
            let zz = z[0];
            loss += if zz > 0.0 { zz + (-zz).exp().ln_1p() } else { zz.exp().ln_1p() } - t * zz;
            let e = c * (sigmoid(zz) - t);
            for (gj, xj) in g[..d].iter_mut().zip(xi) {
                *gj += e * xj;
            }
            g[d] += e;
        } else {
            let p = softmax(&z);
            loss -= p[y[i]].max(1e-300).ln();
            for r in 0..rows {
                let e = c * (p[r] - if r == y[i] { 1.0 } else { 0.0 });
                let gr = &mut g[r * (d + 1)..(r + 1) * (d + 1)];
                for (gj, xj) in gr[..d].iter_mut().zip(xi) {
                    *gj += e * xj;
                }
                gr[d] += e;
            }
        }
    }
    let mut pen = 0.0;
    for r in 0..rows {
        for j in 0..d {
            let w = th[r * (d + 1) + j];
            pen += w * w;
            g[r * (d + 1) + j] += w;
        }
    }
    c * loss + 0.5 * pen
}

/// Limited-memory BFGS with a backtracking (Armijo) line search; stops when the
/// largest gradient component falls below `tol`.
pub fn lbfgs(mut x: Vec<f64>, mut f: impl FnMut(&[f64], &mut [f64]) -> f64, iters: usize, tol: f64) -> Vec<f64> {
    const M: usize = 10;
    let n = x.len();
    let mut g = vec![0f64; n];
    let mut fx = f(&x, &mut g);
    let (mut ss, mut ys, mut rhos): (Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<f64>) = (vec![], vec![], vec![]);
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    for _ in 0..iters {
        // closing: the caller throws the half-trained result away
        if crate::stopping() || g.iter().fold(0f64, |m, v| m.max(v.abs())) < tol {
            break;
        }
        // two-loop recursion
        let mut q = g.clone();
        let mut alpha = vec![0f64; ss.len()];
        for i in (0..ss.len()).rev() {
            alpha[i] = rhos[i] * dot(&ss[i], &q);
            for (qj, yj) in q.iter_mut().zip(&ys[i]) {
                *qj -= alpha[i] * yj;
            }
        }
        if let (Some(s), Some(y)) = (ss.last(), ys.last()) {
            let h = dot(s, y) / dot(y, y);
            q.iter_mut().for_each(|v| *v *= h);
        } else {
            let gn = dot(&g, &g).sqrt().max(1e-12);
            q.iter_mut().for_each(|v| *v /= gn);
        }
        for i in 0..ss.len() {
            let beta = rhos[i] * dot(&ys[i], &q);
            for (qj, sj) in q.iter_mut().zip(&ss[i]) {
                *qj += sj * (alpha[i] - beta);
            }
        }
        let dir: Vec<f64> = q.iter().map(|v| -v).collect();
        let slope = dot(&g, &dir);
        if slope >= 0.0 {
            ss.clear();
            ys.clear();
            rhos.clear();
            continue;
        }
        let mut t = 1.0;
        let mut g2 = vec![0f64; n];
        let (mut x2, mut f2);
        loop {
            x2 = x.iter().zip(&dir).map(|(a, b)| a + t * b).collect::<Vec<_>>();
            f2 = f(&x2, &mut g2);
            if f2 <= fx + 1e-4 * t * slope || t < 1e-12 {
                break;
            }
            t *= 0.5;
        }
        let s: Vec<f64> = x2.iter().zip(&x).map(|(a, b)| a - b).collect();
        let y: Vec<f64> = g2.iter().zip(&g).map(|(a, b)| a - b).collect();
        let sy = dot(&s, &y);
        if sy > 1e-12 {
            if ss.len() == M {
                ss.remove(0);
                ys.remove(0);
                rhos.remove(0);
            }
            ss.push(s);
            ys.push(y);
            rhos.push(1.0 / sy);
        }
        let done = (fx - f2).abs() <= 1e-15 * fx.abs().max(1.0);
        x = x2;
        g = g2;
        fx = f2;
        if done {
            break;
        }
    }
    x
}

pub struct Model {
    pub axes: Vec<Axis>,
    pub models: HashMap<&'static str, Lr>,
}

pub struct TrainInfo {
    pub tracks: usize,
    pub axes: Vec<&'static str>,
}

impl Model {
    pub fn new(axes: Vec<Axis>) -> Model {
        Model { axes, models: HashMap::new() }
    }

    /// y: per track, its label. An axis the label says nothing about is
    /// left out of that axis's training, not trained as "unknown".
    pub fn train(&mut self, x: &[&[f32]], y: &[Label]) -> TrainInfo {
        self.models.clear();
        let mut trained = vec![];
        for ax in &self.axes {
            let (mut xs, mut ys) = (vec![], vec![]);
            for (r, l) in x.iter().zip(y) {
                if let Some(v) = target(l, ax.name) {
                    xs.push(*r);
                    ys.push(v);
                }
            }
            if ys.iter().collect::<BTreeSet<_>>().len() < 2 {
                continue; // nothing to learn yet
            }
            self.models.insert(ax.name, Lr::fit(&xs, &ys, C));
            trained.push(ax.name);
        }
        TrainInfo { tracks: x.len(), axes: trained }
    }

    pub fn predict(&self, x: &[&[f32]]) -> Vec<Prediction> {
        x.iter()
            .map(|r| {
                let mut p = Prediction::default();
                for ax in &self.axes {
                    let Some(m) = self.models.get(ax.name) else { continue };
                    let pr = m.predict_proba(r);
                    // first maximum, as numpy's argmax
                    let j = (0..pr.len()).fold(0, |b, i| if pr[i] > pr[b] { i } else { b });
                    p.top.insert(ax.name, (m.classes[j].clone(), pr[j]));
                    if (1.0 - pr[j]) * (ax.cost + ax.misleading_cost) < ax.cost {
                        p.values.insert(ax.name, m.classes[j].clone());
                        p.confidence.insert(ax.name, pr[j]);
                    }
                }
                p
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lbfgs_finds_a_quadratic_minimum() {
        let x = lbfgs(vec![5.0, -3.0], |x, g| {
            g[0] = 2.0 * (x[0] - 1.0);
            g[1] = 4.0 * (x[1] + 2.0);
            (x[0] - 1.0).powi(2) + 2.0 * (x[1] + 2.0).powi(2)
        }, 200, 1e-10);
        assert!((x[0] - 1.0).abs() < 1e-8 && (x[1] + 2.0).abs() < 1e-8);
    }

    #[test]
    fn the_thresholds_follow_the_costs() {
        // hue: cost 2, misleading 1 -> speaks above 1/3
        let a = &axes_for(1.0, 4.0)[0];
        assert!((1.0 - 0.34) * (a.cost + a.misleading_cost) < a.cost);
        assert!((1.0 - 0.33) * (a.cost + a.misleading_cost) >= a.cost);
    }
}
