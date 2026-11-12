//! The defaults, with whatever <collection>/.vibecheck/config.toml sets on top,
//! section by section -- so a config file only needs what it changes.

use std::path::Path;

use toml::{Table, Value};

use crate::{Res, store};

pub(crate) const DEFAULT: &str = include_str!("default_config.toml");

#[derive(Debug, Clone)]
pub struct Config(pub Table);

pub fn load(root: &Path) -> Res<Config> {
    let mut cfg: Table = DEFAULT.parse()?;
    let user = store::vibecheck_dir(root)?.join("config.toml");
    if user.exists() {
        let mine: Table = std::fs::read_to_string(&user)?.parse().map_err(|e| format!("{}: {e}", user.display()))?;
        for (k, v) in mine {
            match (cfg.get_mut(&k), v) {
                (Some(Value::Table(base)), Value::Table(over)) => base.extend(over),
                (_, v) => {
                    cfg.insert(k, v);
                }
            }
        }
    }
    Ok(Config(cfg))
}

impl Config {
    pub fn get(&self, section: &str, key: &str) -> Option<&Value> {
        self.0.get(section)?.as_table()?.get(key)
    }
    pub fn bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.get(section, key).and_then(Value::as_bool).unwrap_or(default)
    }
    pub fn float(&self, section: &str, key: &str, default: f64) -> f64 {
        match self.get(section, key) {
            Some(Value::Float(f)) => *f,
            Some(Value::Integer(i)) => *i as f64,
            _ => default,
        }
    }
    pub fn int(&self, section: &str, key: &str, default: i64) -> i64 {
        self.get(section, key).and_then(Value::as_integer).unwrap_or(default)
    }
    pub fn ints(&self, section: &str, key: &str, default: &[i64]) -> Vec<i64> {
        match self.get(section, key).and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_integer).collect(),
            None => default.to_vec(),
        }
    }
    /// A table of slot -> integer, e.g. loops = { A = 16, G = 16 }.
    pub fn int_map(&self, section: &str, key: &str) -> Vec<(String, i64)> {
        match self.get(section, key).and_then(Value::as_table) {
            Some(t) => t.iter().filter_map(|(k, v)| Some((k.clone(), v.as_integer()?))).collect(),
            None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_default_parses() {
        let t: toml::Table = super::DEFAULT.parse().unwrap();
        assert!(t["cues"]["write"].as_bool().unwrap());
    }
}
