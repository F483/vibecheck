//! The engine: decode, embed, learn, write rekordbox. No interface, no Godot.
//!
//! A port of the Python prototype in `prototype/`, reading and writing the same
//! `.vibecheck/` files, so either can be used on the same collection.

pub mod app;
pub mod audio;
pub mod config;
pub mod cover;
pub mod cues;
pub mod embed;
pub mod evaluate;
pub mod index;
pub mod keyenergy;
pub mod palette;
pub mod playlist;
pub mod predict;
pub mod rekordbox;
pub mod store;
pub mod workflow;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Set when the app is closing: long work (listening, measuring) gives up at the
/// next safe point instead of holding the window open.
pub static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn stopping() -> bool {
    STOP.load(std::sync::atomic::Ordering::Relaxed)
}

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Unix seconds, as every row in the database records time.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
