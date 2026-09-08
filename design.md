# vibecheck — design notes

Status: exploration. Nothing decided except what is marked **decided**.
Last update: 2026-09-08

## 1. Idea

A local tool that learns *my* labelling of music and pre-labels new tracks
with it.

**This is not genre classification.** The app learns whatever flat set of label
strings its user provides, and assigns no meaning to them. **decided**

My own vocabulary is 24 labels — 4 colors (Red/Green/Blue/Purple) x dark/light
for feel, x 3 levels for how much I like it, written as strings like
`Red_High`. The app never knows any of that; it sees 24 opaque symbols.
Someone else's vocabulary will look nothing like this, and the app must not
care.

Clearing out old tags is the user's own job, done before ingest and outside the
app's scope. No public vocabulary (Discogs, Beatport, ID3 genre lists) enters
anywhere. The entire point is learning one person's taste, so that a growing
collection stays under control without tedious manual management.

Consequences that follow directly from this and drive the rest of the design:

- **No free training data.** Every one of ~13.8k tracks starts unlabelled.
  Labels only exist because the user produced them, so user attention is the
  scarcest resource in the system, and the review loop (§6) *is* the product.
- **Closed vocabulary.** No new labels will ever appear, unlike an open genre
  taxonomy. Discovery of unseen classes stops being a requirement.
- Pretrained audio models are used **only as feature extractors** (audio ->
  numeric fingerprint). Every decision is learned from the user's labels.

## 2. Goals / non-goals

**Goals**
- Learn my personal taste/labelling, not a public consensus. **decided**
- Flat user-defined label set, opaque to the app; no color, rating, axis or
  genre vocabulary is hard-coded, and label strings are never parsed. **decided**
- Handle DJ-scale collections (200 GB+ and growing). **decided**
- The label is written into the mp3's ID3 genre field, so other software
  (Rekordbox, Traktor, players, file managers) sees it. **decided**
- Runs on mac / linux / windows.
- Corrections improve future guesses.

**Non-goals (for now)**
- Not a tag editor / library manager. It touches one field.
- Not a recommender or player — the app never plays audio; listening happens in
  the user's own music software. **decided**
- Playlists are work lists, not a feature: no smart/auto playlist generation,
  and no label information is stored in them.
- One label per track; no multi-label. **decided**
- Audio only — no metadata signals (artist/album/year/filename). **decided**
  Artist is probably the strongest predictor of my labels; it is excluded
  deliberately so the model learns sound, not lookup. Revisit later.
- Track-level labels only; no per-cue-region analysis. **decided**
- No near-duplicate detection: remixes, edits and re-rips are separate tracks.
  **decided** Caveat: duplicates split across train/test inflate cross-validation
  scores, so use artist-grouped splits for any number worth trusting.

## 3. Requirements

### Functional
- **Scan**: walk one or more root folders, find mp3s, read the ID3 genre field.
- **Index**: persist state in the collection root, under `.vibecheck/`
  (§7 Storage). Track index and an append-only label log in one SQLite file,
  embeddings in a separate cache file, config in a text file. **decided**
- **Train**: fit a classifier on tracks that carry a label.
- **Predict**: for unlabelled/new tracks, output a label + confidence, or
  abstain below a threshold.
- **Write**: predicted labels go straight into the tag; review happens
  afterwards in the user's software. A dry-run mode writes nothing and only
  reports what would change.
- **Abstain**: a **configurable confidence threshold**. **decided** Below it, a
  track is skipped rather than given a low-confidence guess; it stays unlabelled
  and is retried later. The right value depends on how much reviewing a wrong
  label costs versus labelling from scratch — taste, so a dial, not a constant.
  Zero means label everything.
- **Playlists as work lists**: a run's working set can be restricted to the
  tracks in a `.m3u8`, and a run can report the tracks it touched as a `.m3u8`.
  Playlists never carry label information and are disposable. **decided**
- **Feedback**: detect labels that are new or changed since the last run by
  comparing each file's current tag to the last-seen value in the DB, and
  retrain on them.
- **Evaluate**: report artist-grouped cross-validated accuracy on the user's own
  labels, overall and per label, against the majority-class baseline. This is
  the only meaningful benchmark and drives every model choice.

### Non-functional
- Incremental and resumable: rescans touch only changed files; a killed run
  loses at most the current batch.
- Embeddings computed once per file, cached, keyed by content hash so moves and
  renames don't force recompute.
- Parallel extraction across cores; must survive corrupt/unreadable mp3s.
- **Local-first**: runs entirely on a laptop-class machine, offline, no upload of
  audio. Cloud embedding stays a one-off benchmark, never a runtime dependency.
  **decided**
- **Portable**: must work on other people's macs and other people's collections.
  No hard-coded paths, no assumption about how much of a library is tagged, and
  no assumption about which label strings a user uses. Every collection bootstraps
  its own vocabulary from zero. **decided**
- Must degrade gracefully on a smaller machine: 8 GB RAM and no GPU should still
  work, just with a cheaper embedding backend.
- Never destroy tags: back up original tag values in the DB before any write.

### Scale estimate
Reference collection (measured 2026-09-08): **13,848 mp3, 199 GB**, ~14 MB and
~6 min per track — DJ-length material at high bitrate. Other formats present but
out of scope: mp3 only, the user converts beforehand. **decided**

Reference machine: M1 Pro, 16 GB, collection on internal SSD. Sized for this,
but **must not assume it** — see portability below.

Decode is not a bottleneck: excerpt seeking reads ~1 MB per file, not 14 MB.

**Feasibility on the reference machine** — the 199 GB is a red herring; what
matters is 13.8k tracks x 90 s of sampled audio = ~350 h of audio to embed,
once. Estimated on M1 Pro: **under an hour** for a CNN backend, **5-10 h** (one
overnight) for a transformer one. After that, embeddings are cached and only new
files are processed — seconds each. Training and prediction are trivial at this
size, and 16 GB RAM is ample. The project is comfortably within the machine;
the binding constraints are user labelling effort and arm64 dependency friction,
not compute.

## 4. Pipeline

```
scan -> decode excerpts -> embed -> [cache] -> train -> categorise -> write tag
                                                 ^                        |
                                                 |                        v
                          train --input=batch.m3u8 <-- user fixes <-- batch.m3u8
```

1. **Scan** — walk roots, stat files, hash (partial hash: first+last N bytes +
   size is enough and fast), read ID3.
2. **Excerpts** — DJ tracks have long intros/outros; a single 30 s window from
   the start is unrepresentative. Take k windows (e.g. 3 × 30 s at 25/50/75 %),
   embed each, average (or keep all and vote).
3. **Embed** — frozen pretrained audio model -> vector (typically 512–1280 dim).
   Pluggable backend; the winner is decided by §3 evaluation, not by reputation.
4. **Classify** — one N-class model over the user's label set, trained on their
   labels only. Start with logistic regression / kNN on the embeddings. Fitting
   14k × 1k floats takes seconds, so "online learning" is really just *retrain
   on every change*; no genuinely incremental algorithm is needed.
5. **Abstain** — below a confidence threshold, output nothing rather than a
   guess. Tuned so accepted suggestions are usually right.

## 5. Labels

**One flat set of label strings. One label per track. That is the entire
model.** **decided**

- The vocabulary is whatever strings the user provides — e.g. `Red_High`,
  `Red_Mid`, `Red_Low`, `Green_High`, ...
- The app assigns **no meaning** to them: no axes, no ordering, no hue, no
  rating, no adjacency, no parsing of the string. They are opaque symbols.
- Any structure lives in the user's naming convention and their head only.
- The label is written to the track metadata as-is; no composition, no template.

So: **a single N-class classifier over the user's label set.** Nothing else.

Cost of this, stated once for the record: a flat 24-value vocabulary needs
examples for all 24 (~20 each at the few-shot knee, so ~500 labels) where a
structured scheme could have shared data across values and needed roughly half
that. Accepted deliberately — the app not knowing anything about label semantics
is worth more than the labels saved, and it is what makes the tool work
unchanged for anyone else's scheme.

### What to expect on my own vocabulary

Not app behaviour — just realistic expectation-setting for the reference config.
My labels mix a "sound" component with a "how much I like it" component. The
second is the risky part:

- Preference is weakly predicted by audio in general; that is why recommenders
  lean on collaborative filtering rather than the signal itself.
- **Range restriction** makes it harder here: the library is already curated,
  every track passed a bar to be kept, so the easy variance is gone and what
  remains is fine discrimination inside a selected pool. That is exactly the
  point of the app, and also the reason it is hard.
- Part of preference is not in the audio at all: novelty, recency, whether a
  track has been played out, whether it fits a current set.
- Self-consistency on that component will be lower than on the sound component,
  and the ceiling binds there first.

Consequences for evaluation, which *are* app behaviour:

- **The baseline to beat is always-predict-majority, not 1/N.** A 24-class
  problem where one label covers 30% of tracks means 30% is the floor, not 4%.
- **Report per-label accuracy, not just overall.** If the sound component is
  learnable and the preference component is not, that shows up as labels
  sharing a prefix being confused with each other — visible per-label, invisible
  in the aggregate number.
- **Abstain is per-label-confidence**, and a wrong suggestion costs more than no
  suggestion.
- If Phase 0 shows the model can only reach the majority baseline, the honest
  outcome is to say so rather than ship a coin flip.

## 6. Interface and workflow

**The label lives in the mp3 tag. Nowhere else.** Playlists carry no label
information whatsoever — they are temporary work lists: which tracks a run
should operate on, and which tracks a run touched. They are disposable and get
deleted often. **decided**

The app never plays audio. Judging a track means listening to it, and the user's
own music software is already the best place for that.

### Core vs interface

The architecture is a **core engine** plus thin front ends. The core owns
everything real: index, embed, train, predict, select a batch, read/write tags,
read/write playlists. A front end only maps user intent onto those operations
and prints results. **decided**

The CLI sketch below is illustrative — a rough shape to reason about, **not a
settled interface**. Flag names, subcommand split and defaults are deliberately
unresolved until the architecture is proven; getting the core right comes first,
and a wrong interface is cheap to change while a wrong core is not.

```sh
# train from entire collection, looking for new or changed labels
vibecheck train --limit=100

# train from a limited playlist, looking for new or changed labels
vibecheck train --limit=100 --input=<playlist>.m3u8

# categorise from entire collection, assigning labels to files without one
vibecheck categorise --limit=100 --output=<playlist>.m3u8

# categorise within a playlist, assigning labels to files without one
vibecheck categorise --limit=100 --input=<playlist>.m3u8 --output=<playlist>.m3u8
```

What matters at this stage is only the shape:

- a run's **working set** is either the whole collection or the tracks in a
  playlist;
- a run can **bound how much work it does**;
- a run can **report which tracks it touched** as a playlist, so they can be
  reviewed in the user's software.

### The round trip

```
categorise a batch, report it as batch.m3u8  ->  labels written into tags
open batch.m3u8 in music software            ->  user listens, fixes wrong tags
train over batch.m3u8                        ->  corrections picked up, retrain
```

Corrections are just tag edits. They are detected by comparing each file's
current tag against the value last recorded in the DB, which is why the index
keeps the last-seen label and a label history (§3).

Nothing about a playlist is authoritative and nothing is inferred from absence:
a track missing from a playlist simply is not part of that batch.

### Which tracks go in a batch

A bounded run says how many tracks to do, not which. That choice is worth
getting right: the tracks a run picks are exactly the tracks the user will
review, so their corrections are the next training signal. The selection problem
is active learning, even though the user is never asked a question
directly. **decided**

- **Cold start (no model)** — farthest-first traversal (k-center greedy) over
  embeddings: pick one track at random, then repeatedly pick the track
  maximally distant from everything picked so far. ~20-30 tracks covers every
  distinct region of the sound-space before any model exists. No k, no
  convergence, deterministic.
- **Warm** — margin sampling: rank by `p(top1) - p(top2)`, smallest first.
  Margin beats entropy here; entropy fixates on tracks confused among many
  labels, margin targets the specific boundaries that need separating.
  Never take the top-N most uncertain directly — they cluster, all uncertain for
  the same reason. Take the ~500 most uncertain as a candidate pool, then
  greedily select the batch from it so members are mutually distant. Score a
  random subsample (5-10k) per run; full scoring costs latency and buys nothing.
- **Steady mix**, per batch: 60% margin, 20% novelty (tracks farthest from all
  labelled data — coverage insurance against whole regions never asked about),
  20% pure random.
- Plus a mild score bonus for candidates predicted into thin labels. A bonus,
  not a hard quota.
- **The random slice is not optional.** Actively-selected tracks are a biased
  sample by construction (they over-represent hard cases), so accuracy measured
  on them is meaningless. Reserve the random-slice tracks as an untouched
  holdout — the only honest accuracy estimate, and the only trustworthy basis
  for deciding when the model is good enough.

Retraining takes seconds at this size, so it happens on every training run; no
incremental algorithm is needed. Predictions get useful once there are a few
labels per class, and a mediocre model still helps: fixing a wrong label is
cheaper than assigning one from scratch.

**Prerequisite**: the candidate pool must be embedded up front — selection can
only choose among tracks it has already seen. That batch cost is unavoidable,
which is why Phase 0 measures extraction wall-clock per backend, not only
accuracy.

Every training run reports progress: labels so far, current accuracy estimate
against the majority baseline, and which labels are still weak.

### Playlist I/O robustness

Playlists come back from software the app does not control, so import must be
forgiving: **decided**

- `.m3u8` is UTF-8 by definition; write it as such, read it as such.
- Match entries to indexed tracks by, in order: exact path, normalised path
  (percent-decoding, `file://` prefixes, separator differences, resolved
  relative paths, case-insensitive on macOS), then content hash, then basename.
  Report anything still unmatched rather than silently dropping it — a silent
  drop looks like the user removing tracks from a batch.
- Write absolute paths on export; assume nothing about what comes back.

## 7. Tech candidates

Backends are not chosen; the surrounding decisions below are.

**Audio decode**: `ffmpeg` subprocess (robust, handles anything, easy seeking to
excerpts) vs a library binding. ffmpeg is the safe default.

**Tag read/write**: `mutagen` (python, ID3 native) or TagLib. mutagen if the
prototype is python.

**Embeddings** (feature extractors only):
| candidate | notes |
|---|---|
| Essentia discogs-effnet | music-specific, CPU-cheap, embedding layer exposed |
| MERT-v1-95M | self-supervised music transformer, strong on music tasks |
| CLAP (LAION) | audio-text contrastive; strong general audio, music decent |
| OpenL3 / VGGish | older, cheap, weaker |
| MFCC/chroma/tempo baseline | trivial, no deps — the floor to beat |

Recent benchmarks show no universal winner; ranking is task-dependent. So:
implement 2–3 behind one interface, measure on my library, keep the winner.
The MFCC baseline is mandatory — if a 200 MB neural model doesn't clearly beat
it on my labels, it isn't worth carrying.

**Local vs cloud**: **local. decided.** Measured scale (13.8k tracks) puts a
full-library embed at well under an hour for a CNN backend and one overnight run
for a transformer one, on the reference M1 Pro. Cloud is kept only as a one-off
Phase 0 comparison (~$7 on a 500-track subset) to know what is being given up —
never a runtime dependency.

**Hardware**: reference is M1 Pro / 16 GB, but **no design may assume a GPU**
(portability, §3). Cheap CPU backends (effnet, MFCC) must stay viable; anything
heavier is an optional accelerated path. Install-cleanliness on arm64 macOS is a
first-class selection criterion — prefer **ONNX Runtime** (clean arm64 wheels,
CoreML EP) over TensorFlow / `essentia-tensorflow`, whose Apple-silicon story is
the most likely place to lose a weekend. A backend scoring 3 points higher but
needing a bespoke toolchain is the wrong choice for a solo-maintained tool.

**Storage**: in the collection root, under a `.vibecheck/` directory:
**decided**

```
~/Music/Collection/.vibecheck/
  config.toml     # handful of options, human-edited
  labels.db       # track index + label log — precious, tiny (~5 MB)
  cache.db        # embeddings — large, regenerable, deletable
```

**Why in the collection root** — consequences that fall out for free:

- **The index travels with the collection.** Move, rename or copy the folder and
  the labels, embeddings and history come along. Nothing to re-derive.
- **Store paths relative to the root**, which the directory defines. The
  collection can then be mounted anywhere — different user, machine, external
  drive — without invalidating every row.
- **The collection root is the only configuration.** No `~/.config` state, no
  registry of known libraries, no ambiguity about which collection a run means.
  Several collections are simply several folders, each self-describing.
- Multi-user portability (§3) comes free: hand someone the folder, they have a
  working index.

**Why three files and not one:**

- **The label log is the only irreplaceable thing in the project.** It is hours
  of listening. Embeddings cost 5-10 h of CPU but are re-derivable from the
  audio at any time; the log is re-derivable from nothing. Separate files mean
  the log can be backed up, synced, even kept in git, while a 100 MB+ cache is
  ignored — and the cache can be deleted to switch embedding backends without
  ever putting the log at risk.
- **Config a human edits belongs in a text file, not rows in a binary DB.** A
  handful of options in `config.toml` is editable, greppable and diffable, and
  needs no subcommand to change a threshold. The DB holds only data the program
  owns.

**Journal mode: default rollback journal, not WAL.** **decided** WAL only pays
off for concurrent readers alongside a writer, which never happens here — one
process doing batch work. Default mode keeps each DB a single file at rest, with
no `-wal`/`-shm` sidecars in the collection. `synchronous=FULL`; the write volume
is far too low for the difference to matter.

**Append-only label log.** The bulk of the DB is a log of label changes:

```sql
CREATE TABLE label_log (
  id     INTEGER PRIMARY KEY,
  path   TEXT NOT NULL,      -- relative to collection root
  hash   TEXT NOT NULL,      -- survives renames
  label  TEXT,               -- NULL = label removed
  source TEXT NOT NULL,      -- 'user' | 'model'
  ts     INTEGER NOT NULL
);
```

Never `UPDATE`, never `DELETE`; the current label is the newest row for a path.
This gives correction history for free, and the file stays trivially
recoverable.

**`source` is not bookkeeping, it is a correctness requirement.** **decided**
Without it: a categorise run writes a tag, a later training run reads that tag
back, and the model trains on its own output — a feedback loop that quietly
amplifies its own errors until the labels are the model's opinion rather than
the user's. Training consumes `source='user'` rows only.

**Caution for other setups**: SQLite on cloud-synced storage (Dropbox, iCloud)
can corrupt. Irrelevant on the reference internal SSD, but worth a warning for
anyone whose collection lives on synced storage.

**Prototype runtime**: python — all the model tooling lives there.

**Playlist I/O**: `.m3u8` read/write is a few dozen lines; no library needed.
The work is path normalisation on import, not parsing.

**Config format**: TOML — stdlib `tomllib` in python 3.11+, no dependency.

**App shell**: deferred, and possibly never — see Phase 2. If it happens:
Godot UI + local sidecar over stdio, a python GUI, or Tauri.

## 8. Plan

- **Phase 0 — feasibility, read-only** *(next)*: scan, embed, read labels the
  user has applied by hand in their own software (a few hundred tracks), train,
  artist-grouped cross-validation. Report overall and per-label accuracy against
  the majority baseline, plus install friction and wall-clock per backend.
  Writes nothing. Answers the question the whole project rests on: *does this
  beat always-guessing-the-commonest-label on my labels, and by how much?*
  If the answer is no, stop here — that is a valid outcome, cheaply reached.
- **Phase 1 — usable**: tag writing, confidence threshold, playlist in/out,
  batch selection, incremental rescan.
- **Phase 2 — app**: *possibly unnecessary.* With playlists as the interface,
  listening and correcting both happen in the user's existing software, so the
  CLI may be the finished product. Only build a GUI if using the CLI proves
  annoying in practice — do not assume it.

## 9. Open questions

Deferred until Phase 0 produces numbers (cannot be answered by opinion):
- Which embedding backend wins on my labels, and at what wall-clock cost.
- Whether one cloud backend is worth a one-off comparison run.
- How many labels are actually needed before suggestions become useful.

Needs an answer from me, but not blocking Phase 0:
- Ceiling check: re-label ~100 tracks blind and compare to my earlier labels.
  That self-agreement rate is the real accuracy target; chasing above it is
  chasing noise.

## References

- [Essentia models](https://essentia.upf.edu/models.html)
- [MAEB: Massive Audio Embedding Benchmark](https://arxiv.org/pdf/2602.16008)
- [CLAP vs MERT fine-grained comparison](https://www.mdpi.com/2079-9292/15/8/1723)
