//! Godot's view of the core: one call that takes and returns JSON, and the
//! cover picture as bytes. Everything is static and stateless, so GDScript can
//! call it from a worker thread; the work lives in `vibecheck::app`.

use std::path::{Path, PathBuf};

use godot::prelude::*;
use serde_json::{Value, json};
use vibecheck::{Res, app, config};

struct Extension;

#[gdextension]
unsafe impl ExtensionLibrary for Extension {}

#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct Vibecheck {}

fn home(p: &str) -> PathBuf {
    let h = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(if let Some(r) = p.strip_prefix("~/") { format!("{h}/{r}") } else { p.to_string() })
}

fn s<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}

fn n(a: &Value, k: &str, d: i64) -> i64 {
    a.get(k).and_then(Value::as_f64).map(|f| f as i64).unwrap_or(d)
}

fn run(cmd: &str, a: &Value) -> Res<Value> {
    let root = home(s(a, "root"));
    let xml = home(s(a, "xml"));
    let cfg = || config::load(&root);
    Ok(match cmd {
        "open" => json!(app::open(&root, &xml)?),
        "library" => json!(app::library(&root, &xml)?),
        "queue" => {
            let skip: Vec<String> = a.get("skip").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
            json!(app::queue(&root, &xml, &cfg()?, n(a, "threshold", 100) as usize, n(a, "n", 200) as usize, &skip)?)
        }
        "decide" => {
            let key = Some(s(a, "key")).filter(|k| !k.is_empty());
            let energy = Some(n(a, "energy", 0)).filter(|e| *e > 0);
            app::decide(&root, s(a, "path"), n(a, "colour", 0), n(a, "stars", 0), key, energy)?;
            json!(true)
        }
        "embed" => {
            let paths: Vec<String> = serde_json::from_value(a.get("paths").cloned().unwrap_or(json!([])))?;
            json!(app::embed(&root, &paths)?)
        }
        "unheard" => json!(app::unheard(&root, n(a, "n", 20) as usize)?),
        "export" => json!(app::export(&root, &xml, &cfg()?)?),
        "sync" => json!(app::sync(&root, &xml)?),
        "stats" => json!(app::stats(&root, &cfg()?)?),
        "wave" => json!(app::wave(&root, &xml, &cfg()?, s(a, "path"), n(a, "bins", 600) as usize)?),
        "settings" => app::settings(&cfg()?),
        "set" => {
            app::set(&root, s(a, "section"), s(a, "key"), a.get("value").unwrap_or(&Value::Null))?;
            app::settings(&cfg()?)
        }
        "exported" => json!(vibecheck::rekordbox::exported(&xml).unwrap_or(0)),
        _ => return Err(format!("unknown call {cmd}").into()),
    })
}

#[godot_api]
impl Vibecheck {
    #[func]
    fn version() -> GString {
        vibecheck::VERSION.into()
    }

    /// {"ok": result} or {"error": message}, as JSON.
    #[func]
    fn call(cmd: GString, args: GString) -> GString {
        let (cmd, args) = (cmd.to_string(), args.to_string());
        let out = std::panic::catch_unwind(|| {
            let a: Value = serde_json::from_str(&args).unwrap_or(json!({}));
            run(&cmd, &a)
        });
        let v = match out {
            Ok(Ok(v)) => json!({ "ok": v }),
            Ok(Err(e)) => json!({ "error": e.to_string() }),
            Err(p) => json!({ "error": p.downcast_ref::<String>().cloned().or(p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or("crashed".into()) }),
        };
        GString::from(v.to_string().as_str())
    }

    /// The app is closing: long work stops at its next safe point.
    #[func]
    fn stop() {
        vibecheck::STOP.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The cover picture embedded in the file, jpeg or png bytes; empty without one.
    #[func]
    fn cover(root: GString, path: GString) -> PackedByteArray {
        let p = home(&root.to_string()).join(Path::new(&path.to_string()));
        vibecheck::cover::read(&p).map(|b| PackedByteArray::from(b.as_slice())).unwrap_or_default()
    }
}
