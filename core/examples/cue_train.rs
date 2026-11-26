//! The generic cue network (v2), measured as v1 was: your cued tracks split five ways, each
//! fifth predicted by a model trained on the other four, compared bar for bar with the cues
//! you set by hand (suggestions kept unchanged are trained on, never measured). v1
//! reproduced 50.1% of your hot cue bars, held out, on the 646 tracks it could read: the bar
//! v2 has to clear there. Reads only; writes cache rows for features.
//! usage: cue_train [epochs] [--folds N] [--hand-only]
use std::collections::HashSet;
use std::time::Instant;
use vibecheck::{cue_train, cues, rekordbox, store};

#[derive(Default)]
struct Score {
    tracks: usize,
    hot_hit: usize,
    hot_mine: usize,
    hot_got: usize,
    half: usize,
    none: usize,
    slot: usize,
    slots: usize,
    beat: usize,
    lp: usize,
    matched: usize,
    mem_hit: usize,
    mem_mine: usize,
    mem_got: usize,
}

impl Score {
    fn add(&mut self, mine: &cues::Target, got: &cues::Target) {
        let bars = |t: &cues::Target| -> HashSet<usize> { t.hot.iter().flatten().map(|a| a.bar).collect() };
        let (m, g) = (bars(mine), bars(got));
        let hit = m.intersection(&g).count();
        self.tracks += 1;
        self.hot_hit += hit;
        self.hot_mine += m.len();
        self.hot_got += g.len();
        self.half += (2 * hit >= m.len()) as usize;
        self.none += (hit == 0 && !m.is_empty()) as usize;
        for k in 0..cues::N_HOT {
            if let Some(a) = mine.hot[k] {
                self.slots += 1;
                self.slot += (got.hot[k].map(|b| b.bar) == Some(a.bar)) as usize;
            }
        }
        for a in mine.hot.iter().flatten() {
            if let Some(b) = got.hot.iter().flatten().find(|b| b.bar == a.bar) {
                self.matched += 1;
                self.beat += (a.beat == b.beat) as usize;
                self.lp += (a.lp == b.lp) as usize;
            }
        }
        let mb: HashSet<usize> = mine.mem.iter().map(|a| a.bar).collect();
        let gb: HashSet<usize> = got.mem.iter().map(|a| a.bar).collect();
        self.mem_hit += mb.intersection(&gb).count();
        self.mem_mine += mb.len();
        self.mem_got += gb.len();
    }

    fn print(&self, name: &str) {
        let pc = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
        println!("{name}, {} tracks:", self.tracks);
        println!("  your hot cues reproduced, exact bar   {:.1}%   (v1: 50.1% on the 646)", pc(self.hot_hit, self.hot_mine));
        println!("  of the hot cues it set, yours         {:.1}%   ({} set, you have {})", pc(self.hot_hit, self.hot_got), self.hot_got, self.hot_mine);
        println!("  tracks with at least half exact       {:.1}%   (v1: 62.7%)", pc(self.half, self.tracks));
        println!("  tracks with none exact                {:.1}%   (v1: 6.3%)", pc(self.none, self.tracks));
        println!("  in the same slot too                  {:.1}%", pc(self.slot, self.slots));
        println!("  on a bar of yours: beat / loop right  {:.1}% / {:.1}%", pc(self.beat, self.matched), pc(self.lp, self.matched));
        println!("  memory cues: yours found / its yours  {:.1}% / {:.1}%", pc(self.mem_hit, self.mem_mine), pc(self.mem_hit, self.mem_got));
    }
}

fn main() -> vibecheck::Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(cue_train::EPOCHS);
    let folds: usize = args.iter().position(|a| a == "--folds").and_then(|i| args.get(i + 1)?.parse().ok()).unwrap_or(5);
    let hand_only = args.iter().any(|a| a == "--hand-only");
    let home = std::path::PathBuf::from(std::env::var("HOME")?);
    let root = home.join("Music/Collection");
    let cfg = vibecheck::config::load(&root)?;
    let xml = rekordbox::read(&home.join("Documents/rekordbox.xml"), &root)?;
    let t0 = Instant::now();
    let data = cue_train::examples(&root, &xml, &cfg)?;
    // the tracks v1 could learn from: your cues in its pairs, 16 and 8 bars before a change
    let old: HashSet<String> = cues::yours(&xml, &store::cues_written(&store::labels_db(&root)?)?, &[16, 8]).into_keys().collect();
    let hand = data.iter().filter(|e| e.hand).count();
    let bars: usize = data.iter().map(|e| e.x.len()).sum();
    println!(
        "{} tracks ({} set by hand, {} suggestions kept), {} bars, {} of v1's 646; features in {:.0}s{}",
        data.len(),
        hand,
        data.len() - hand,
        bars,
        data.iter().filter(|e| old.contains(&e.rel)).count(),
        t0.elapsed().as_secs_f64(),
        if hand_only { "; training on the ones set by hand only" } else { "" }
    );
    // as the app decodes; the same with v1's count (8 a track) and with a wider gap
    const DEC: [&str; 4] = ["as used (expected count, gap 4, slot by bar)", "8 a track, as v1 sets them", "slot by position", "slot by both"];
    let mut all: Vec<Score> = DEC.iter().map(|_| Score::default()).collect();
    let mut v1: Vec<Score> = DEC.iter().map(|_| Score::default()).collect();
    for f in 0..folds {
        let t0 = Instant::now();
        let (test, train): (Vec<_>, Vec<_>) = data.iter().enumerate().partition(|(i, _)| i % folds == f);
        let train: Vec<cue_train::Example> = train
            .into_iter()
            .filter(|(_, e)| e.hand || !hand_only)
            .map(|(_, e)| cue_train::Example { rel: e.rel.clone(), x: e.x.clone(), target: e.target.clone(), w: e.w, hand: e.hand, trust: e.trust })
            .collect();
        let m = cues::Model::from_bytes(&cue_train::fit(&train, "cv", cue_train::SEEDS, epochs)?)?;
        for (_, e) in test.into_iter().filter(|(_, e)| e.hand) {
            let (s, emp) = m.scores(&e.x);
            let a = cues::decode_generic(&s, &emp);
            let k = a.hot.iter().flatten().count();
            let g = cues::HOT_GAP;
            let got = [
                a,
                cues::decode_with(&s, &emp, 8, g, cues::SlotBy::Which),
                cues::decode_with(&s, &emp, k, g, cues::SlotBy::Position),
                cues::decode_with(&s, &emp, k, g, cues::SlotBy::Both),
            ];
            for (i, g) in got.iter().enumerate() {
                all[i].add(&e.target, g);
                if old.contains(&e.rel) {
                    v1[i].add(&e.target, g);
                }
            }
        }
        println!("fold {}/{folds}: {:.0}s", f + 1, t0.elapsed().as_secs_f64());
    }
    println!("held out, {epochs} epochs:");
    for (i, d) in DEC.iter().enumerate() {
        println!("=== decoder {d}");
        v1[i].print("v1's 646");
        all[i].print("all set by hand");
    }
    Ok(())
}
