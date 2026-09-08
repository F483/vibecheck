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
- Runs on macOS, Apple Silicon (M1 or newer). Mac-only. **decided**
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
- **Abstain**: a **configurable confidence threshold**, exposed to the user as a
  slider for how much wrongness they will tolerate. **decided** Below it a track
  is skipped rather than guessed at; it stays unlabelled and is retried later.
  Zero means label everything. Wrong labels are expected and fine — the point is
  that the user chooses the rate.
  For the slider to mean anything, the number behind it must be **calibrated**:
  logistic regression on high-dimensional embeddings is overconfident by
  default, happily reporting 0.97 on tracks it gets wrong, which would make the
  threshold inert. Fit a temperature scalar on the holdout, and document the
  slider in measured terms ("0.8 => ~85% of written labels correct on your
  holdout"), not in raw model self-report.
- **Playlists as work lists**: a run's working set can be restricted to the
  tracks in a `.m3u8`, and a run can report the tracks it touched as a `.m3u8`.
  Playlists never carry label information and are disposable. **decided**
- **Feedback**: detect labels that are new or changed since the last run by
  comparing each file's current tag to the last-seen value in the DB, and
  retrain on them.
- **Holdout**: a fixed slice of labelled tracks (~10-20%) is **never trained
  on**, and exists only to report honest accuracy. **decided** Membership is
  derived deterministically from the content hash (e.g. `hash % 10 == 0`), so it
  is stable as labels accumulate, needs no bookkeeping, and cannot drift.
- **Evaluate**: report accuracy on the holdout — overall and per label — against
  the majority-class baseline. Near-duplicate tracks are grouped so that copies,
  remixes and re-rips never straddle the train/holdout boundary (see §4.7).
  This is the only meaningful benchmark and drives every model choice.

### Non-functional
- Incremental and resumable: rescans touch only changed files; a killed run
  loses at most the current batch.
- Embeddings computed once per file and cached, keyed by **(content hash,
  backend, model version, preprocessing config hash)**. **decided** The content
  hash alone is not enough: switching backends, bumping a model version or
  changing a preprocessing parameter must invalidate the cache, or vectors from
  two different pipelines get silently mixed — the same class of invisible
  failure §4.2 exists to prevent. Content hashing means moves and renames never
  force a recompute.
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
scan -> decode excerpts -> resample -> preprocess -> embed -> pool -> [cache]
                                                                        |
                             write tag <- categorise <- train <---------+
                                  |                       ^
                                  v                       |
                            batch.m3u8 -> user fixes -> tags
```

### 4.1 Stages

1. **Scan** — walk roots, stat files, partial hash (first + last N bytes + size).
2. **Decode** — mp3 -> PCM. Only the excerpt regions, not the whole file.
   mp3 seeking is frame-aligned and the bit reservoir makes the first frames
   after a seek inexact: decode slightly early and discard ~0.5 s.
3. **Resample** — models want a fixed rate (typically 16 kHz mono; MERT 24 kHz);
   mp3s are 44.1 kHz. The resampler is part of the model contract, not a detail:
   a different one shifts the embeddings.
4. **Preprocess** — log-mel spectrogram (or whatever the chosen model expects).
   **See 4.2 — this is where the project can silently lose accuracy.**
5. **Embed** — frozen model -> one vector per excerpt (512-1280 dim).
6. **Pool** — mean across the 3 excerpts, then L2-normalise. Store the
   per-excerpt vectors too: 3 x 1280 floats is 15 KB/track, ~200 MB for the
   library, which buys the freedom to re-pool later without re-decoding.
7. **Train** — logistic regression over pooled vectors, `source='user'` rows only.
8. **Categorise** — predict, apply the confidence threshold, write tags, report
   the batch as a playlist.

### 4.2 Where the DSP lives — the parity trap

The chain from file to embedding:

```
mp3 -> PCM 44.1k -> resample 16k -> STFT -> mel -> log -> neural net -> embedding
                    \_________________ DSP ____________/   \__ the model file __/
```

Normally the exported model file contains only the neural net; everything left
of it is done in python with librosa before feeding the model. Ship a native app
and someone has to rewrite that DSP — matching sample rate, window function,
window and hop length, centre padding, FFT size, magnitude vs power, mel band
count, mel scale formula (**htk** and **slaney** both exist and differ),
filterbank normalisation, log base, epsilon, and any final normalisation.

Get one wrong and the model receives inputs unlike anything it was trained on.
**Nothing errors.** The embeddings are just worse, accuracy drops a few points,
and the cause is invisible.

Three ways to place the DSP:

| option | what it means | cost |
|---|---|---|
| **A. inside the model file** | exported graph takes raw PCM, returns embedding; the app only decodes | export work per model; in-graph resampling is awkward |
| **B. reimplemented natively** | Swift computes mel via vDSP/Accelerate | must replicate librosa exactly; silent-failure risk; redo per model |
| **C. ship python in the app** | bundle python + librosa + onnxruntime | 200-400 MB, hardened-runtime friction — but zero parity risk |

**Decision: prototype in python (effectively C). Target A for the shipped app,
keep C as the fallback, never B.** **decided** A is the only option that is both
lean and parity-safe; B is the worst of both, hand-written DSP *and* the risk.

Bundle-size expectation for A: ONNX Runtime (~50 MB) + weights (30-100 MB) + a
few MB of app = a notarisable 100-150 MB `.app` with no python. C lands at
200-400 MB — acceptable if A fails, not the goal.

**Some models remove the problem entirely.** Raw-waveform models (MERT is
HuBERT-style: raw 24 kHz into a conv encoder) have no spectrogram to reproduce;
the only preprocessing is resampling. Mel-based models (effnet, CLAP) carry the
whole chain. So the model choice *is* the choice of how hard the port is —
prefer raw-waveform models when scores are close.

**Two things that cost nothing now and keep A reachable:** **decided**

- **Pin the preprocessing config as data in the repo** — sample rate, window,
  hop, n_fft, n_mels, mel scale, log base, epsilon, normalisation — never rely
  on library defaults. Librosa defaults have changed across versions, so an
  implicit default is a bug waiting for an upgrade. This config *is* the port
  spec later.
- **Keep decode behind a narrow interface** — one function, `(path, offset,
  duration) -> float array`. Then swapping ffmpeg for AVFoundation is a one-file
  change, and the brew prototype never forecloses the notarised `.app`.

**The parity test**: embed a fixed set of ~50 tracks through both paths and
compare cosine similarity, expecting > 0.999. Written once, it catches this
entire bug class permanently.

### 4.3 Two runtimes, one artifact

Research and shipping have opposite needs: research wants every model available
and fast iteration; shipping wants a small signed binary with no toolchain.
Resolve it by making the **exported model file the contract between them**.
**decided**

- **Research (Phase 0)**: python. torch/transformers/librosa, whatever it takes
  to try candidates. Ends by exporting the winner to ONNX (with preprocessing
  baked in) and recording its accuracy.
- **Shipping**: a native binary that loads that ONNX file. No python, no torch.
- Export cleanliness is a **selection criterion**, not an afterthought. A model
  that scores well but will not export with its preprocessing is not a
  candidate. No ready-made ONNX exports of the leading music embedding models
  were found, so budget export work per candidate and verify numerically
  (cosine similarity vs the python reference on a fixed set of tracks) before
  trusting any of it.

### 4.4 Training must ship, not just inference

Most ML apps ship **inference only**: the developer trains, freezes and ships a
model file. This one cannot. The classifier is trained on the user's machine
from their own labels, repeatedly — everyone has different taste — so the
**training code must live inside the shipped binary**, not just the prediction
code.

That is normally what forces an app to bundle python and sklearn. Here it does
not, provided the classifier stays small: multinomial logistic regression over
~14k x 1280 floats is one matmul plus a softmax to predict (~10 lines) and a few
dozen lines of gradient descent to fit, running in seconds.

So logistic regression is the **default, not a ban**. Keep the classifier behind
a narrow boundary so a heavier one is a contained change, and require evidence
before paying for it: measure whether anything actually beats LR on real labels.
Lean and fast where possible — but a big app beats no app, so if a heavier
classifier is what makes the thing work, ship it.

### 4.5 Tech per stage, under the packaging constraint

Packaging an installable, notarised mac app rules some things out.

Target platform is **macOS on Apple Silicon (M1 or newer), mac-only**.
**decided** Cross-platform is dropped; it was costing generality nobody asked
for.

| stage | shipped-app option | notes |
|---|---|---|
| decode | `minimp3` (public domain, single header) | tiny, embeddable, no license issue |
| | AVFoundation / ExtAudioFile | native macOS, zero dependency, mac-only |
| | ~~ffmpeg~~ | **avoid in the shipped app** — see below |
| resample | baked into the model graph (4.2) | preferred |
| | libsamplerate (BSD) / Apple AudioConverter | fallback |
| preprocess | baked into the model graph (4.2) | preferred |
| inference | ONNX Runtime (+ CoreML EP) | portable, one artifact everywhere |
| | CoreML | ANE, smallest bundle, mac-only |
| classifier | hand-written LR | ~50 lines, trains and predicts |
| storage | SQLite | in every runtime already |

**ffmpeg: depend on it, do not bundle it.** **decided** The distinction matters
and only one side of it is a problem:

- *Depending* on it — a brew formula with `depends_on "ffmpeg"`, called as a
  subprocess — is clean. Nothing is redistributed, calling a separate program at
  arm's length creates no derivative work, and the MIT license is unaffected.
- *Bundling* it inside a distributed `.app`/`.dmg` means redistributing
  GPL/LGPL binaries, and the obligations attach to the distribution.

So: **prototype ships via brew with ffmpeg as a dependency.** The concern only
returns for a double-clickable notarised `.app`, and on mac that is solved for
free by AVFoundation, which decodes mp3 natively with no dependency at all —
a one-file change behind the decode interface (§4.2).

Shipping stack for the app phase: **Swift + AVFoundation + CoreML/ONNX Runtime +
SQLite.** Smallest bundle, Neural Engine available, standard Xcode signing. A
Godot front end remains possible over that core if a GUI is ever wanted; Godot
cannot do the inference itself either way.

### 4.6 macOS distribution requirements

Wanting other people to install this on their macs implies, concretely:

- **Prototype distribution is brew** — no signing, no notarisation, no Apple
  account needed. **decided** Everything below applies to the later `.app`, and
  the point of §4.2 and the decode interface is that nothing in the prototype
  forecloses it.
- **Apple Developer Program membership** (~$99/yr) for a Developer ID
  certificate. There is no notarisation without it.
- **Codesign + hardened runtime + notarise + staple.** Unnotarised downloads get
  progressively more hostile Gatekeeper treatment on recent macOS.
- **arm64-only is acceptable** (M1 or newer, per the constraint) — no universal
  binary needed.
- **Ship the model weights inside the bundle** (30-100 MB) rather than
  downloading on first run; no hosting, no integrity checks, works offline.
- **File-access permission**: a `.app` must obtain its own consent to read the
  user's music folder, unlike a CLI run from Terminal which inherits the
  terminal's. Verify what macOS currently requires for `~/Music` before
  assuming it is unrestricted, and avoid the Mac App Store sandbox unless
  there is a reason to accept it.

### 4.7 Grouping for honest evaluation

The collection contains many near-identical files — an original, a remix, an
edit, a re-rip of the same track. If one copy is trained on and its twin sits in
the holdout, the model scores well by effectively recognising a track it has
already seen rather than by having learned anything. The reported accuracy is
then too high, and every decision built on it is built on a number that is not
real.

**Group near-duplicates using the embeddings themselves** — cosine similarity
near 1.0 — and keep a group wholly inside training or wholly inside the
holdout. **decided** This needs no metadata at all: no artist tag, no album, no
folder heuristics, consistent with audio-only classification (§2). It is also
free, since the embeddings already exist.

To be explicit about a distinction that is easy to blur: file paths and content
hashes are read to *identify files on disk*. They are never inputs to the model.

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

### The normal workflow

Simplicity is the priority. The expected use is not "sit down and train a
model", it is: **new music arrives, the app labels it, the user corrects what is
wrong while DJing, and the app picks that up by itself.** **decided**

- **Corrections are detected, not declared.** The app compares each file's
  current tag against the value last recorded in the DB. A changed tag is a
  correction and becomes a `source='user'` row. The user never has to say "I
  reviewed these".
- **This needs no daemon.** An incremental scan is cheap — stat 14k files, then
  read tags only for files whose mtime or size moved — so every run starts with
  one. A scheduled background run is an option, not a requirement.
- **The app must not learn from itself.** A label the app wrote is logged as
  `source='model'`. If the tag still matches what the app wrote, nothing
  happened and no user row is created. Only genuine differences count. Without
  this the model retrains on its own guesses and amplifies its own errors.
- **Explicit confirmation stays available** for the case where the user *has*
  deliberately reviewed a batch: training over a playlist means "these labels
  are mine now", changed or not. Over the whole collection it means only
  "pick up what changed" — it must never promote unreviewed model labels.
- **Taste drifts, and that is normal.** The log is append-only and the newest
  row for a track wins, so relabelling a track years later simply supersedes.
  Timestamps make recency-weighted training possible later; not needed now.

### Which tracks to work on

Two different jobs, with opposite selection criteria — conflating them produces
an empty batch, because one asks for the tracks the other refuses to touch:

**Bulk labelling (the user-facing job).** Label as much new music as possible.
Take the tracks the model is **most** confident about; the confidence threshold
skips the rest for a later run. Unattended, writes tags, `--limit` is throughput.

**Uncertainty review (a development tool, not a user feature).** **decided**
Take the tracks the model is **least** sure about — they teach the most per
listen. Nobody wants to be handed a random pile of music to educate a
classifier, so this stays a diagnostic for tuning and evaluating the model
during the prototype, and is not part of the normal workflow.

For the record, if it is ever wanted as a user feature, the selection method is
active learning: cold start by farthest-first traversal (k-center greedy) over
embeddings; once trained, margin sampling (`p(top1) - p(top2)`, smallest first)
over a candidate pool, greedily diversified so the batch is not 20 copies of the
same confusion; with a slice of pure random picks, since actively-selected
tracks are a biased sample and accuracy measured on them means nothing.

Retraining takes seconds at this size, so it happens whenever labels change; no
incremental algorithm is needed.

**Prerequisite**: tracks must be embedded before they can be labelled or
selected. That batch cost is unavoidable, which is why Phase 0 measures
extraction wall-clock per backend, not only accuracy.

Runs report progress: labels so far, holdout accuracy against the majority
baseline, and which labels are still weak.

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

## 8. Roadmap

**Phase 0 — feasibility. Deliverable: numbers, not software.** *(next)*
Throwaway python. Take the tracks already labelled by hand, embed them with 2-3
backends plus the MFCC baseline, and measure. In this order — the order matters:
**decided**

1. implement the backends behind one interface;
2. **export spike per candidate**: export to ONNX with preprocessing included,
   one track, check cosine against the python reference. Half a day each, and it
   can veto a model *before* any bulk work;
3. embed the labelled subset; measure accuracy and wall-clock;
4. pick the winner on accuracy + wall-clock + export feasibility together;
5. only then embed the full library.

Measuring accuracy first, picking a winner, then discovering it will not port is
how a project ends up hand-writing DSP with the labelling effort already spent.

Reporting: holdout accuracy overall and per label against the majority baseline,
a learning curve (accuracy vs labels per label-value), and wall-clock plus
install friction per backend. The learning curve answers "how many labels does
this actually need" empirically, instead of guessing a number in advance.

*Go/no-go*: does any backend beat always-guessing-the-commonest-label by a
margin worth building on? If not, stop here — a valid outcome, cheaply reached.

**Phase 1 — usable CLI. Deliverable: a tool in daily use.**
Scan and index the full library, embed everything (the overnight job), train,
bulk-label with the confidence threshold, write tags, incremental rescan that
picks up corrections automatically, playlists as work lists. This is where it
starts saving work.

**Phase 2 — refinement. Deliverable: it improves efficiently.**
Calibration, progress reporting, near-duplicate grouping, and the uncertainty
review tool (§6 — a development aid, not a user feature). Deliberately later:
with thousands of labels available up front, none of this is load-bearing for
the thing working.

**Phase 3 — the app. Deliverable: something other people can install.**
Swift, ONNX export with preprocessing baked in, AVFoundation decode, codesign,
notarise, `.dmg`. Only worth doing once Phases 0-1 prove the idea and the CLI
has been lived with.

## 9. Open questions

Deferred until Phase 0 produces numbers (cannot be answered by opinion):
- Which embedding backend wins on my labels, at what wall-clock and install cost.
- Whether any cloud backend is worth a one-off comparison run.
- Where the learning curve flattens — how many labels this problem needs.
- Whether anything beats plain logistic regression enough to justify shipping a
  heavier classifier (§4.4).

Known, accepted, not problems to solve:
- **Anchoring**: reviewing a suggested label is not the same as labelling blind,
  so acceptance rate is not evidence of accuracy. Nothing to fix — just never
  report acceptance rate as accuracy; the holdout (§3) is the honest number.
- **Taste drift**: labels legitimately change over time. Newest row wins.

Needs an answer from me, but not blocking:
- How many of the initial labels came from a playlist rule over metadata rather
  than from listening? Anything driven by an inaudible rule caps what an
  audio-only model can reach, and would be mistaken for the approach failing.
  If the share is meaningful, Phase 0 should report those subsets separately.
- Ceiling check: relabel ~100 tracks blind and compare to the earlier labels.
  That self-agreement rate is the real accuracy target; chasing above it is
  chasing noise.

## References

- [Essentia models](https://essentia.upf.edu/models.html)
- [MAEB: Massive Audio Embedding Benchmark](https://arxiv.org/pdf/2602.16008)
- [CLAP vs MERT fine-grained comparison](https://www.mdpi.com/2079-9292/15/8/1723)
