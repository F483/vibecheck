//! Command line over the core: the same commands as the Python prototype, on the
//! same collection and the same `.vibecheck/` files.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;

use vibecheck::{Res, config, index, palette, store};

mod commands;

const USAGE: &str = "\
usage: vibecheck [--root DIR] <command> [args]

the loop
  vibecheck scan                     find your tracks (once, and after adding music)
  vibecheck label 100                read your corrections, learn from them,
                                     then label 100 more and write the colours,
                                     star ratings, key, energy and cue points
                                     into the rekordbox xml

  ... refresh the rekordbox xml node in rekordbox, import the new playlist,
      correct what is wrong while you listen, export the collection again,
      and run the same command again ...

commands
  scan                  find tracks, and notice what moved
  status                counts, labels, and what is embedded
  label [N]             read corrections, then label N tracks (default 300)
      --dry-run           predict and write the playlist, change nothing
      --no-sync           do not read your corrections first
      --include-unconfirmed   also pick tracks out in an unreviewed batch
      --separate-file     write a new xml instead of updating in place
      --no-rekordbox      skip the rekordbox xml
      --output FILE       playlist path (its name names the round)
      --seed N            fix the random selection
  sync [PLAYLIST]       read your corrections back (--all, --stale-ok)
  discard [PLAYLIST]    throw a batch away, free its tracks (--dry-run)
  analyse [PLAYLIST]    key and energy alone (--all, --dry-run, --no-rekordbox)
  cues [PLAYLIST]       cue points alone (--all, --dry-run)
  migrate [--apply]     bring the database up to date

  --rekordbox XML      the rekordbox export (default: ~/Documents/rekordbox.xml)

  --root DIR           your music collection (default: ~/Music/Collection)

what it touches
  the rekordbox xml you point it at, and <collection>/.vibecheck/ .
  Never your music files. Nothing changes in rekordbox until you import.
";

/// Options that take a value; everything else starting with -- is a flag.
const VALUED: &[&str] = &["--root", "--rekordbox", "--output", "--seed"];

pub struct Args {
    cmd: String,
    pub pos: Vec<String>,
    flags: HashSet<String>,
    pub opts: HashMap<String, String>,
}

impl Args {
    fn parse(raw: Vec<String>) -> Result<Args, String> {
        let (mut cmd, mut pos, mut flags, mut opts) = (String::new(), vec![], HashSet::new(), HashMap::new());
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            if let Some((k, v)) = a.split_once('=').filter(|_| a.starts_with("--")) {
                opts.insert(k.to_string(), v.to_string());
            } else if VALUED.contains(&a.as_str()) {
                opts.insert(a.clone(), it.next().ok_or(format!("{a} needs a value"))?);
            } else if a.starts_with("--") || a == "-h" {
                flags.insert(a);
            } else if cmd.is_empty() {
                cmd = a;
            } else {
                pos.push(a);
            }
        }
        Ok(Args { cmd, pos, flags, opts })
    }
    pub fn flag(&self, f: &str) -> bool {
        self.flags.contains(f)
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Rust ignores SIGPIPE, so printing into a closed pipe (`vibecheck status | head`)
/// panics. Restore the default: exit quietly like any other command line tool.
fn default_sigpipe() {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn signal(sig: i32, handler: usize) -> usize;
        }
        const SIGPIPE: i32 = 13; // Linux, macOS and the BSDs
        const SIG_DFL: usize = 0;
        unsafe { signal(SIGPIPE, SIG_DFL) };
    }
}

fn main() -> ExitCode {
    default_sigpipe();
    let args = match Args::parse(std::env::args().skip(1).collect()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.cmd.is_empty() || args.flag("--help") || args.flag("-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let root = args.opts.get("--root").map(PathBuf::from).unwrap_or_else(|| home().join("Music/Collection"));
    let root = root.canonicalize().unwrap_or(root);
    let r = config::load(&root).and_then(|cfg| match args.cmd.as_str() {
        "scan" => cmd_scan(&root),
        "status" => cmd_status(&root, &cfg),
        "migrate" => cmd_migrate(&root, &args),
        "label" => commands::label(&root, &cfg, &args),
        "sync" => commands::sync(&root, &args),
        "discard" => commands::discard(&root, &args),
        "analyse" | "analyze" => commands::analyse(&root, &cfg, &args),
        "cues" => commands::cues(&root, &cfg, &args),
        "version" => {
            println!("vibecheck {}", vibecheck::VERSION);
            Ok(())
        }
        other => Err(format!("unknown command: {other}\n\n{USAGE}").into()),
    });
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_scan(root: &std::path::Path) -> Res<()> {
    println!("scanning {}", root.display());
    let st = index::scan(root, 12)?;
    let mut parts = vec![format!("{} mp3 files", st.files), format!("{} new", st.new)];
    for (n, what) in [
        (st.moved, "moved"),
        (st.changed, "changed"),
        (st.missing, "now missing"),
        (st.returned, "back"),
        (st.unreadable, "unreadable"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    println!("   {}", parts.join(", "));
    Ok(())
}

/// The holdout, derived from the content hash: bucket 0 test, 1 validation.
fn holdout(hash: &str) -> bool {
    u64::from_str_radix(&hash[..8], 16).map(|v| v % 10 < 2).unwrap_or(false)
}

fn cmd_status(root: &std::path::Path, _cfg: &config::Config) -> Res<()> {
    let con = store::labels_db(root)?;
    let (total, gone): (Option<i64>, Option<i64>) = con.query_row(
        "SELECT SUM(missing_at IS NULL), SUM(missing_at IS NOT NULL) FROM tracks",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (total, gone) = (total.unwrap_or(0), gone.unwrap_or(0));
    let lab = store::current(&con, Some("user"))?;
    let cache = store::cache_db(root)?;
    let have: HashSet<String> = {
        let mut st = cache.prepare("SELECT hash FROM embeddings WHERE backend='clapw'")?;
        st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    let hb = store::hash_by_path(&con)?;

    println!("collection      {total} tracks{}", if gone > 0 { format!("   ({gone} no longer on disk)") } else { "".into() });
    println!("labelled        {}", lab.len());
    println!("unlabelled      {}", total - lab.len() as i64);
    println!("embedded        {}", hb.values().filter(|h| have.contains(*h)).count());
    if !lab.is_empty() {
        let held = lab.keys().filter_map(|p| hb.get(p)).filter(|h| holdout(h)).count();
        println!(
            "holdout         {held} of {} labels{}",
            lab.len(),
            if held == 0 { "   (none yet: nothing can be measured)" } else { "" }
        );
    }
    let last: Option<(i64, i64, i64, Option<String>)> = con
        .query_row(
            "SELECT f.id, f.ts, f.n_train, r.name FROM fits f LEFT JOIN rounds r ON r.id = f.round_id
             ORDER BY f.id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();
    if let Some((fid, ts, n, name)) = last {
        let (left, hand): (f64, f64) =
            con.query_row("SELECT SUM(cost_left), SUM(cost) FROM fit_axes WHERE fit_id=?1", [fid], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        println!(
            "last measured   {left:.2} of {hand:.2} decisions left per track   ({}, {n} labels, {})",
            name.as_deref().unwrap_or("no round"),
            vibecheck_date(ts)
        );
    }
    let rounds: Vec<(String, i64, i64, Option<i64>)> = {
        let mut st = con.prepare("SELECT name, size, n_labels, closed FROM rounds ORDER BY id")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?
    };
    if !rounds.is_empty() {
        println!("rounds          {}", rounds.len());
        for (name, size, n, closed) in &rounds[rounds.len().saturating_sub(5)..] {
            println!("   {name:22} {size:4} tracks, knew {n:5}{}", if closed.is_some() { "" } else { "   (open)" });
        }
    }
    if !lab.is_empty() {
        println!("colours");
        for c in &palette::COLOURS {
            let n = lab.values().filter(|l| l.colour == Some(c)).count();
            if n > 0 {
                println!("   {:8} {n:5}   {}/{}", c.name, c.hue, c.tone);
            }
        }
        println!("stars");
        let mut by: BTreeMap<u8, usize> = BTreeMap::new();
        for s in lab.values().filter_map(|l| l.stars) {
            *by.entry(s).or_default() += 1;
        }
        for (k, n) in by.iter().rev() {
            let s = if *k == 0 { "-".to_string() } else { "*".repeat(*k as usize) };
            println!("   {s:8} {n:5}");
        }
    }
    Ok(())
}

fn vibecheck_date(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

fn cmd_migrate(root: &std::path::Path, args: &Args) -> Res<()> {
    let Some((con, path)) = store::labels_db_raw(root)? else {
        println!("no database at {}; run scan to create one", store::labels_path(root)?.display());
        return Ok(());
    };
    let (have, todo) = store::pending(&con)?;
    println!("{}", path.display());
    println!("   at schema v{have}, this build is v{}", store::SCHEMA_VERSION);
    if todo.is_empty() {
        println!("   up to date, nothing to do");
        return Ok(());
    }
    println!("   {} migration(s) to apply: {}", todo.len(), todo.join(", "));
    if !args.flag("--apply") {
        println!("\n(nothing changed -- pass --apply to migrate)");
        return Ok(());
    }
    let done = store::migrate(&con, Some(&path))?;
    println!("   applied {}; now v{}", done.join(", "), store::SCHEMA_VERSION);
    println!("   previous database kept at labels.db.pre-v{have}");
    Ok(())
}
