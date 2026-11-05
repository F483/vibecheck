//! Honest measurement: a holdout fixed by the content hash, never trained on.

use crate::predict::{Axis, Model, target};
use crate::store::Label;

pub const VAL: &str = "val";

/// Bucket 0 is the test slice, bucket 1 validation, the rest training.
pub fn part(hash: &str) -> &'static str {
    match u64::from_str_radix(&hash[..8], 16).map(|v| v % 10) {
        Ok(0) => "test",
        Ok(1) => "val",
        _ => "train",
    }
}

#[derive(Debug, Clone)]
pub struct AxisScore {
    pub axis: &'static str,
    pub n_values: usize,
    pub cost: f64,
    pub misleading: f64,
    pub n: usize,
    pub spoke: usize,
    pub correct: usize,
    /// decisions still to make, per track
    pub cost_left: f64,
}

impl AxisScore {
    pub fn coverage(&self) -> f64 {
        if self.n > 0 { self.spoke as f64 / self.n as f64 } else { 0.0 }
    }
    pub fn accuracy(&self) -> f64 {
        if self.spoke > 0 { self.correct as f64 / self.spoke as f64 } else { 0.0 }
    }
}

/// Fit on the training slice only, then score each axis on the validation
/// slice, through the real decision rule.
pub fn score_axes(x: &[&[f32]], y: &[Label], hashes: &[&str], axes: &[Axis]) -> Vec<AxisScore> {
    let (mut xt, mut yt, mut xh, mut yh) = (vec![], vec![], vec![], vec![]);
    for i in 0..x.len() {
        match part(hashes[i]) {
            "train" => {
                xt.push(x[i]);
                yt.push(y[i]);
            }
            p if p == VAL => {
                xh.push(x[i]);
                yh.push(y[i]);
            }
            _ => {}
        }
    }
    if xt.len() < 2 || xh.is_empty() {
        return vec![];
    }
    let mut m = Model::new(axes.to_vec());
    m.train(&xt, &yt);
    let preds = m.predict(&xh);
    let mut out = vec![];
    for ax in axes {
        if !m.models.contains_key(ax.name) {
            continue;
        }
        let (mut n, mut spoke, mut correct, mut left) = (0, 0, 0, 0.0);
        for (truth, p) in yh.iter().zip(&preds) {
            let Some(t) = target(truth, ax.name) else { continue };
            n += 1;
            match p.values.get(ax.name) {
                None => left += ax.cost,
                Some(v) if *v == t => {
                    spoke += 1;
                    correct += 1;
                }
                Some(_) => {
                    spoke += 1;
                    left += ax.cost + ax.misleading_cost;
                }
            }
        }
        if n > 0 {
            out.push(AxisScore {
                axis: ax.name,
                n_values: ax.n_values,
                cost: ax.cost,
                misleading: ax.misleading_cost,
                n,
                spoke,
                correct,
                cost_left: left / n as f64,
            });
        }
    }
    out
}
