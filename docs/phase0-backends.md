# Phase 0 — backend comparison

Date: 2026-09-09. Status: comparison complete, winner not yet locked in.

Phase 0 asks one question (README §8): **does predicting a user's own labels
from audio beat always guessing their commonest label, and by how much?**
Deliverable is numbers, not software.

## 1. What was done

1. **Scan** the collection: 14,194 mp3, ~199 GB. Read the ID3 genre field of
   every file, record it in an append-only label log. 8,619 tracks carry a
   label (60.7%); 24 distinct label strings, no typos.
2. **Embed** with three backends. Every backend sees the same tracks and the
   same amount of audio.
3. **Train** one multinomial logistic regression per backend on the same
   training split.
4. **Score** on a held-out validation slice, against the majority baseline.

## 2. Pipeline

```
mp3 -> decode excerpts -> resample -> backend -> pool -> cache (sqlite)
                                                           |
                                        report <- logistic regression
```

- **Decode**: `ffmpeg` seeks to each excerpt and decodes only that region
  (~1 MB read per file, not 14 MB). One narrow function owns this, so the
  decoder can be swapped later without touching anything else.
- **Excerpts**: 9 windows x 10 s, evenly spread through the track — 90 s per
  track for every backend. DJ tracks have long intros and outros, so a single
  window from the start is unrepresentative. Window length matters
  independently: attention cost is quadratic in it, and 3 x 30 s exhausted
  memory on a 16 GB machine where 9 x 10 s does not.
- **Pooling**: mean over time, then mean over windows, then L2 for the neural
  backends.
- **Cache**: keyed by `(audio hash, backend, version, preprocessing digest)`.
  Any change to a backend or its config lands in a separate keyspace, so two
  configurations can never be silently mixed in one comparison.
- **Identity**: content hash of the *audio region only*, skipping ID3 tags.
  Tags are what this app writes, so hashing them would mean every label written
  changes a track's identity and orphans its embedding.

## 3. The backends

| | input | dim | what it is |
|---|---|---|---|
| **MFCC** | 16 kHz | 168 | Hand-built summary statistics: MFCCs and their deltas, spectral centroid, rolloff, zero-crossing rate, RMS — mean and standard deviation of each over time. No model, no downloads. The floor a neural model must clear to justify itself. |
| **MERT-v1-95M** | 24 kHz raw | 768 | Self-supervised music transformer, HuBERT-style. Takes **raw waveform**, so there is no spectrogram to reproduce when porting — the only preprocessing is resampling. |
| **CLAP** (`larger_clap_music_and_speech`) | 48 kHz | 512 | Audio-text contrastive model. Mel-based, so it carries a full DSP chain that a native port would have to reproduce exactly. Its text encoder also allows label-free zero-shot queries. |

Both neural models are used **only as feature extractors**. They are frozen;
nothing about the user's labels reaches them. All label knowledge lives in the
logistic regression trained on top.

## 4. Method

- **Split**: by content hash into `hash % 10` — bucket 0 test, bucket 1
  validation, remainder training. Deterministic, so it is stable as labels
  accumulate, survives renames, and can be reproduced after deleting the
  database. **Test has not been looked at** and stays sealed until a backend is
  chosen; comparing several models against one holdout inflates the winner's
  score by selection alone.
- **Baseline**: always predict the commonest label. This is the number that
  matters, not 1/24 = 4.2%.
- **Diagnostics**: the reference vocabulary is `Colour_Level`, so accuracy is
  also reported for the colour half and the level half separately. The app
  itself never splits a label — this decomposition exists only in the report,
  because a model that learns nothing but "say the commonest level" would
  otherwise look respectable on the joint task.

## 5. Results

2,681 labelled tracks, 2,161 training, 269 validation.

```
                    overall         colour          level
baseline            22.7%           24.5%           64.3%
MFCC                24.5%  (+1.9)   34.9%  (+10.4)  55.8%  (-8.5)
MERT                34.2%  (+11.5)  46.1%  (+21.6)  58.4%  (-5.9)
CLAP                34.6%  (+11.9)  47.2%  (+22.7)  58.4%  (-5.9)
```

Ranking behaviour, MERT, on the earlier split:

```
top-1  32.9%   (frequency-only baseline 24.2%)
top-2  45.5%   (41.9%)
top-3  53.8%   (46.6%)
top-5  66.8%   (57.4%)

confidence threshold   kept    precision
        0.0           100.0%     32.9%
        0.5            81.2%     36.4%
        0.7            56.3%     42.3%
```

Learning curve (MERT, training tracks -> accuracy): 200 -> 25.6%,
802 -> 28.5%, 1,203 -> 30.7%, 2,005 -> 32.5%. Still climbing at the end.

## 6. How to read this

- **Only the bracketed numbers mean anything.** Raw accuracies are not
  comparable across columns: 64.3% on level looks strong until you notice that
  always saying the commonest level scores exactly that. The bracket is points
  above a strategy that ignores the audio entirely.
- **Noise floor is ~3 points** at 269 validation tracks. MERT vs CLAP (0.4
  points) is a tie. MERT vs MFCC (9.7) is real.
- **Colour is learnable, level is not.** Both neural backends reach ~+22 on
  colour, and all three sit *below* baseline on level — worse than a constant
  guess, meaning the model is adding noise that displaces a good default.
  Three representations agree, so this is a property of the problem rather than
  of weak features.
- **Neural clearly beats hand-built features.** MFCC gets +1.9; the transformers
  get ~+11.5. The floor was worth measuring, and it was cleared decisively.
- **Not yet usable** at the settings above — but see §10, which recovers a large
  part of this for free.
- **This is a quarter of the available data.** 2,161 training tracks of 8,619
  labelled, and the learning curve has not flattened.

## 7. Known contaminants

- **Duplicate downgrading.** When the maintainer meets a track already in the
  collection, they often lower the duplicate's level so it surfaces less, and
  do not revisit its colour. Some level labels therefore encode a fact about
  the *library*, not about the audio — unlearnable by construction, and a live
  candidate explanation for the level result. 634 duplicate groups exist
  (676 redundant copies, 4.8% of the collection); 74 groups carry conflicting
  labels. Cleaning them up and re-running is a clean test: if level accuracy
  improves, contamination was the cause; if not, taste genuinely is not
  audio-predictable.
- **Severe class imbalance**, 412x between the largest and smallest label.
  Labels with fewer than ~30 examples can be neither learned nor measured.
- **Small validation slice** (269 tracks) — hence the ~3 point noise floor.

## 8. Bugs found (and what they cost)

Recorded because each was silent, and each would have produced a confident
wrong conclusion:

- **L2-normalising a heterogeneous feature vector.** Rolloff in Hz swamped
  MFCCs and annihilated zero-crossing rate; the "MFCC baseline" was really
  centroid+rolloff and scored *below* baseline. Would have read as "audio
  features do not work".
- **`class_weight="balanced"`** cost 10-18 points at this imbalance.
- **Variable shadowing** clobbered the split array with a list; `list == str`
  is `False`, and numpy indexing with a scalar `False` returns an empty array
  rather than raising — so the learning curve silently produced no rows and
  exited 0.
- **Hashing the ID3 tag region** made two copies of one track collide, and
  would have invalidated the embedding cache on every tag write.
- **Subset selection keyed on the content hash** silently re-drew the
  comparison set when the hash function changed. The subset is now a frozen
  artifact.
- **`ProcessPoolExecutor` hangs instead of raising** when the OS kills its
  worker. Two runs stalled silently for an hour. Single-worker paths now run
  inline, with process-level chunking so an OOM kill is visible and resumable.

## 9. Free wins: the classifier was leaving points on the table

Everything here reuses embeddings already computed. No new inference.

```
best single backend (CLAP), LR C=1.0        34.6%  (+11.9)
MERT + CLAP concatenated, LR C=1.0          36.8%  (+14.1)
kNN cosine k=20 on the concatenation        42.4%  (+19.7)
MERT + CLAP concatenated, LR C=0.001        46.5%  (+23.8)
```

- **Concatenating MERT and CLAP helps** (+2.2). They are trained differently and
  make different mistakes. Adding MFCC on top *hurts* — it is mostly noise
  beside a transformer.
- **Regularisation was the single biggest miss.** `C=1.0` was picked without
  tuning; the optimum is a flat plateau around `C=0.001-0.005`, worth **~10
  points**. With 1,280 dimensions and 2,161 training examples this is exactly
  the regime where strong regularisation matters, and it should have been swept
  from the start.
- **kNN with cosine distance beats logistic regression at default settings**
  (42.4% vs 36.8%), though a properly regularised LR then beats kNN. Worth
  keeping as a check whenever the feature space changes.
- **PCA does not help** (64 dims: 40.5%, worse than regularised LR on the full
  space).
- **The abstain threshold now works.** Previously, discarding 44% of predictions
  bought 32.9% -> 42.3%. Now:

```
threshold   kept    precision
   0.0     100.0%     45.7%
   0.3      72.1%     53.1%
   0.4      53.2%     58.7%
   0.5      39.0%     63.8%
```

  Keeping the confident 39% at ~64% precision is the difference between a
  curiosity and something worth running over the library.
- **Top-k**: top-1 45.7%, top-2 58.4%, top-3 68.0%.
- **Level nudged above baseline for the first time** (65.8% vs 64.3%) — within
  noise, so not a result, but no longer clearly negative. Colour reached 51.7%
  against a 24.5% baseline.

**Selection-bias warning.** Roughly 25 configurations have now been scored
against the same 269-track validation slice. Picking the best of 25 inflates
that winner by an estimated 2-4 points. These numbers are for *choosing*, not
for reporting. The sealed test slice is the honest figure and must be looked at
exactly once, after the configuration is locked.

## 10. Configurations still worth trying

Ranked by expected value per unit of work.

1. **Full data.** 2,161 of 8,619 labelled tracks are in use and the learning
   curve has not flattened. Biggest known lever, one overnight run.
2. **MERT middle layers.** Only the final layer is used, because MERT's bundled
   code under transformers 5.x stops exposing the rest. Published probes
   routinely do better from middle layers or a weighted sum. Recoverable with
   forward hooks on the encoder layers; re-embed required.
3. **Mean + standard deviation pooling** instead of mean alone. Captures how
   much a track varies over its length, which is plausibly relevant to both
   axes, and doubles the dimension for one line of code.
4. **A Whisper-encoder backend.** Published linear probes have captioning-trained
   encoders beating MERT substantially on genre (~91% vs ~79% on GTZAN) —
   genre words are common in captions. Small, fast, well-supported, easy to
   export.
5. **More coverage.** 9 x 10 s samples 90 s of a ~6 minute track. 15 windows
   would cover half of it, at proportional cost.
6. **Bigger music models**: MERT-v1-330M, MuQ, MusicFM. Better representations
   at 3-4x the inference cost, and a heavier bundle to ship.
7. **Per-label thresholds and temperature scaling**, once a configuration is
   locked — the global threshold currently treats a 2,000-example label and a
   5-example label identically.

## 11. Full data, and the sealed test

CLAP embedded over all 8,619 labelled tracks; Whisper added on the subset as a
deliberately different (captioning-trained) candidate.

**Cross-validation overturned the ensemble result.** 5-fold CV on 2,161
training tracks, which is far tighter than the 269-track validation slice:

```
  clap                  43.5% +/- 0.6     <- best
  clap+mert             43.5% +/- 1.1
  clap+whisper          42.8% +/- 1.0
  whisper               39.8% +/- 1.4
  mert                  37.9% +/- 2.5
  clap+whisper+mert     42.6% +/- 0.6
```

The apparent +1.9 from `clap+whisper` on the validation slice was noise. No
combination beats CLAP alone. **Configuration locked: CLAP, logistic regression
on standardised features, C=0.001 chosen by CV on the training split.**

**Sealed test, read once, after locking:**

```
clap, LR C=0.001, 6,891 train / 861 test

  baseline         22.9%
  accuracy         45.6%   (+22.8)
  colour only      55.7%   (baseline 26.1, +29.6)
  level only       69.6%   (baseline 64.2, +5.3)
  top-1 45.6%   top-2 58.4%   top-3 69.1%

  threshold  kept   precision
     0.3     63.8%    55.9%
     0.4     45.5%    60.5%
     0.5     32.2%    63.2%
     0.6     22.9%    65.5%
     0.7     13.4%    73.0%
```

Validation said 47.8%, test says 45.6%. That 2.2-point drop is the selection
inflation predicted in §9 — the estimate was 2-4 points, and holding the test
slice back is what made it visible instead of believed.

**Level became learnable with more data.** At 2,161 training tracks it was
*below* baseline for all three backends, and the earlier conclusion here was
that taste is not audio-predictable. At 6,891 it is +5.3 on held-out data.
Small, but real and outside noise. The earlier conclusion was wrong, and it was
wrong because of data volume, not representation. Colour was never in doubt and
is now +29.6.

**Phase 0 go/no-go: GO.** The question was whether this beats always guessing
the commonest label. It does, by 22.8 points on data never used for any
decision, and the confidence threshold behaves: keeping the top 45% of
predictions gives ~61% precision, the top 23% gives ~66%.

## 12. Next

- Phase 1: the CLI that uses this (scan, embed, bulk-label with the threshold,
  write tags, pick up corrections).
- Duplicate cleanup then re-measure: 676 redundant copies, and level labels are
  contaminated by duplicate-downgrading (§7). Level is the weakest axis and this
  is the one known, unexploited fix for it.
- More labels still help: the learning curve had not flattened at 6,891.
- Cheap unexplored: mean+std pooling, more windows per track, per-label
  thresholds.
- Not worth pursuing: MERT and Whisper as ensemble partners; MFCC as anything
  but a floor.
