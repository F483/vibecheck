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
    STOP.load(std::sync::atomic::Ordering::Relaxed) || abandoned()
}

/// The tracks the player still wants: the one on screen and those it reads ahead.
static WANTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    /// The player's track this thread is reading, if it is reading one.
    static FOR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn want(paths: Vec<String>) {
    *WANTED.lock().unwrap() = paths;
}

fn abandoned() -> bool {
    FOR.with(|p| p.borrow().as_ref().is_some_and(|p| !WANTED.lock().unwrap().contains(p)))
}

/// Run `f` for the player's track `path`. Once the player moves on (`want` without it),
/// `stopping()` turns true on this thread, so a long decode gives up at its next safe point
/// instead of holding up the next track; None then.
pub fn for_player<T>(path: &str, f: impl FnOnce() -> T) -> Option<T> {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FOR.with(|p| *p.borrow_mut() = None);
        }
    }
    FOR.with(|p| *p.borrow_mut() = Some(path.to_string()));
    let _reset = Reset;
    let r = f();
    (!abandoned()).then_some(r)
}

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Unix seconds, as every row in the database records time.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
