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

Existing tags in the library are irrelevant and will be wiped; no public
vocabulary (Discogs, Beatport, ID3 genre lists) enters anywhere. The entire
point is learning one person's taste, so that a growing collection stays under
control without tedious manual management.

Consequences that follow directly from this and drive the rest of the design:

- **No free training data.** Every one of ~13.8k tracks starts unlabelled.
  Labels only exist because the user produced them, so user attention is the
  scarcest resource in the system and the labelling loop (§6) *is* the product.
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
- Not a recommender, player, or playlist generator.
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
- **Index**: persist per-track state in a DB (path, size, mtime, content hash,
  embedding, current tag, predicted tag, confidence, label history).
- **Train**: fit a classifier on tracks that carry a label.
- **Predict**: for unlabelled/new tracks, output a label + confidence, or
  abstain below a threshold.
- **Ingest policy**, configurable, `clear` is the default: **decided**
  - `clear` — wipe existing tags on first ingest, label from scratch
  - `preserve` — keep existing tags as ground truth (offered for other users
    whose collections carry labels they trust; not the primary path)
- **Write policy**, configurable: **decided**
  - `confirm` (default) — show suggestion, write only on accept
  - `auto` — write immediately
  - `never` — DB only
- **Feedback**: a manual label change (in the app, or detected in the file on
  rescan) is recorded and used in the next training run.
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
scan -> decode excerpts -> embed -> [cache] -> train classifier -> predict -> write tag
                                                    ^                            |
                                                    +------ corrections ---------+
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

## 6. Labelling loop

No fixed seed set. The user labels tracks until they choose to stop, and the
model starts helping as early as it can. **decided**

```
present track -> user labels -> retrain (cheap) -> suggest on next track -> ...
```

- Suggestions appear as soon as there are ~2+ labels per class; early ones are
  wrong and that is fine — accepting a correct suggestion is one keypress, so a
  mediocre model already beats typing.
- Retraining is seconds on this data size, so retrain after every few labels.
  No incremental algorithm needed.
- **Which track to ask about next matters more than the model.** Random order
  wastes the user's attention on 200 near-identical tracks from one folder.
  Selection policy, in phases: **decided**

  **A. Cold start (0 labels), no model** — farthest-first traversal (k-center
  greedy) over embeddings: pick one track at random, then repeatedly pick the
  track maximally distant from everything picked so far. ~20-30 labels covers
  every distinct region of the sound-space. No k, no convergence, deterministic.

  **B. Warm (>=2-3 labels/class)** — margin sampling: rank by
  `p(top1) - p(top2)`, smallest first. Margin beats entropy here; entropy
  fixates on tracks confused among many classes, margin targets the specific
  pairwise boundaries that actually need separating.
  Never take the top-N uncertain directly — they cluster, all uncertain for the
  same reason. Take the ~500 most uncertain as a candidate pool, then greedily
  select ~10 that are mutually distant. Label the batch, retrain, repeat.
  Score a random subsample of the pool (5-10k) per round; full scoring costs
  latency and buys nothing.

  **C. Steady mix**, per batch:
  - **60% margin** — sharpen boundaries.
  - **20% novelty** — tracks farthest from all labelled data. Demoted from its
    original justification: the vocabulary is closed, so there are no unseen
    classes to discover. It still earns its slot as coverage insurance against
    whole regions of the library the model has never been asked about.
  - **20% pure random** — see below.

  Plus a mild score bonus for candidates predicted into thin classes. A bonus,
  not a hard quota.

- **The random stream is not optional.** Actively-selected labels are a biased
  sample by construction (they over-represent hard cases), so accuracy measured
  on them is meaningless. Reserve the random-stream labels as an untouched
  holdout — that is the only honest accuracy estimate, and the only trustworthy
  basis for deciding when to stop labelling. Without it the progress number lies
  pessimistically and the user over-labels.
- Classifier: logistic regression or kNN on embeddings. Logistic regression
  gives usable probabilities for margin; kNN needs distance-weighted votes to
  produce a confidence at all.
- **Prerequisite**: the whole candidate pool must be embedded up front — active
  learning can only choose among tracks it has already seen. That batch cost is
  unavoidable, which is why Phase 0 must measure extraction wall-clock per
  backend, not only accuracy.
- Progress must be visible: labels so far, current accuracy estimate, which
  labels are still weak. The user stops when the numbers say it is good enough.
- Stopping is never final; labelling can resume, and every later correction is
  just another label.

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

**Storage**: SQLite. One file, no server, 14k rows is nothing, embeddings as
BLOBs (~70 MB per backend at 1280-d float32).

**Prototype runtime**: python — all the model tooling lives there.

**App shell** (later, deliberately undecided): Godot UI + local sidecar process
speaking JSON over stdio; or a python GUI; or Tauri. Deferred until the CLI
proves the idea works.

## 8. Plan

- **Phase 0 — CLI prototype** *(next)*: scan, embed, hand-label a few hundred
  tracks through the loop, train, artist-grouped cross-validation, report
  overall and per-label accuracy against the majority baseline, plus install
  friction and wall-clock per backend. Read-only, no tag writes. Answers the
  question the whole project rests on: *does this beat always-guessing-the-
  commonest-label on my own labels, and by how much?*
- **Phase 1 — CLI usable**: predict + write policies, corrections, incremental
  rescan.
- **Phase 2 — app**: GUI over the same core.

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
