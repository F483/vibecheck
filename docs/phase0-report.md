# Vibe Check — Phase 0 report: can a model learn one person's music labels?

Date: 2026-09-10. Status: **complete. Verdict: GO.**

---

## 1. Conclusion first

A model trained only on the audio predicts this user's own 24 labels
**correctly 45.6% of the time**, against **22.9%** for the best strategy that
ignores the audio entirely. Measured on 861 tracks that were never used for any
decision during development.

What that buys in practice:

| you want | you get |
|---|---|
| the exact label, unattended | 45.6% correct — **not trustworthy on its own** |
| the right *colour* (8 options) | 55.7% correct, vs 26.1% for guessing the commonest |
| the right *level* (3 options) | 69.6% correct, vs 64.2% for always saying "C" — barely better |
| a shortlist of 3 to pick from | 69.1% contain the right answer |
| only its confident guesses | top 23% of tracks at **65.5%** correct; top 13% at **73.0%** |

**The honest summary: this is a good assistant and a bad oracle.** It cannot be
turned loose on the library unsupervised. Used as a pre-filter — accept the
confident predictions, review them, ignore the rest — it does real work. Two
thirds of its confident guesses are right, and correcting one is much cheaper
than assigning one from a blank field.

**Colour is genuinely learnable** (+29.6 points over baseline). **Level is
barely learnable** (+5.3). If the goal is to reduce manual labelling effort, the
colour half is where the value is.

---

## 2. What was done

1. **Indexed** the collection: 14,194 mp3, ~199 GB, of which 8,619 carry a
   hand-applied label. 24 distinct label strings, no typos, applied by ear.
2. **Embedded** every labelled track with four different audio models, each
   turning a track into a fixed list of numbers describing its sound.
3. **Trained** a small classifier on those numbers to predict the user's label.
4. **Measured** on tracks held back from training, against the strategy of
   always guessing the commonest label.
5. **Chose** the winning setup by cross-validation, then read a sealed test set
   exactly once.

---

## 3. Setup

### Data

- 8,619 labelled tracks; 8,386 distinct after identical copies collapse.
- Labels are `Colour_Level`: 8 colours x 3 levels, where A is best.
- Heavily imbalanced: `Purple_C` 2,061 tracks, `Blue_A` 5. Ratio 412x.
- Level distribution is skewed: C 65%, B 30%, A 5%.

### Pipeline

```
mp3 -> decode 9 x 10 s excerpts -> resample -> model -> pool -> cache
                                                                  |
                                        report <- logistic regression
```

- Excerpts are spread through the track, not taken from the start: DJ tracks
  have long intros and outros.
- Every model sees the same 90 seconds per track, so comparisons are about the
  model rather than how much it listened to.
- Embeddings are cached, keyed by audio content plus model plus settings, so no
  two configurations can be silently mixed.

### Models compared

| | what it is | result |
|---|---|---|
| **MFCC** | Hand-built acoustic statistics. No neural model. The floor a model must clear to justify itself. | +1.9 — barely above baseline |
| **MERT-v1-95M** | Music-specific self-supervised transformer, raw waveform. | +18.2 |
| **Whisper-small encoder** | Speech/captioning model, used purely as a feature extractor. | +21.6 |
| **CLAP** (`larger_clap_music_and_speech`) | Audio-text contrastive model. | **+23.4 — winner** |

### Final configuration

```
model            laion/larger_clap_music_and_speech
audio            48 kHz, 9 windows x 10 s, mean-pooled, L2-normalised
features         512 numbers per track
classifier       logistic regression on standardised features, C = 0.001
                 (C chosen by 5-fold cross-validation on the training split)
```

The audio models are **frozen**. They never see a label. All knowledge of the
user's taste lives in the small classifier trained on top — which is what makes
the same system work for someone else's completely different vocabulary.

### How it was measured

Tracks were split deterministically by audio content into three parts:

- **training** (80%) — the classifier learns from these.
- **validation** (10%) — used to explore ideas and compare options.
- **test** (10%) — **never touched until the configuration was locked**, then
  read once.

---

## 4. Results

### Final, on the sealed test set (861 tracks)

```
  baseline         22.9%
  accuracy         45.6%   (+22.8)
  colour only      55.7%   (baseline 26.1, +29.6)
  level only       69.6%   (baseline 64.2,  +5.3)
  top-1 45.6%   top-2 58.4%   top-3 69.1%

  confidence   tracks kept   of those, correct
     0.3          63.8%           55.9%
     0.4          45.5%           60.5%
     0.5          32.2%           63.2%
     0.6          22.9%           65.5%
     0.7          13.4%           73.0%
```

### Model comparison (5-fold cross-validation, 2,161 training tracks)

```
  clap                  43.5% +/- 0.6     <- best
  clap+mert             43.5% +/- 1.1
  clap+whisper          42.8% +/- 1.0
  whisper               39.8% +/- 1.4
  mert                  37.9% +/- 2.5
  clap+whisper+mert     42.6% +/- 0.6
```

Combining models does not help. CLAP alone is as good as anything tried.

### Effect of more labels

Trained on 2,161 tracks, level prediction was *below* baseline — the model was
worse than a constant guess. Trained on 6,891, it is +5.3 above. Accuracy
overall rose from roughly 43% to 45.6% on honest measurement. **The learning
curve had still not flattened**, so more labels should continue to help.

---

## 5. How to read these numbers

**Compare against the baseline, never against 100% or against 1-in-24.**
Random guessing among 24 labels would be 4.2%, but nobody would guess randomly —
they would always say the commonest label and score 22.9%. That is the number to
beat, and the bracketed figures throughout are points above it.

**Raw percentages are not comparable between columns.** Level scores 69.6%,
which looks far better than colour's 55.7% — but always saying "C" already gets
64.2%, while always guessing the commonest colour gets 26.1%. Colour is the far
stronger result despite the lower raw number.

**The noise floor depends on sample size.** With 861 test tracks, differences
under about 2 points are not meaningful. With the 269-track validation slice
used for early exploration, the floor was around 3 points — which is exactly how
an apparent ensemble improvement turned out to be nothing.

**Validation and test differ, and the difference is informative.** Validation
said 47.8%; test said 45.6%. That 2.2-point gap is the cost of having compared
about 25 configurations against the validation slice: picking the best of many
flatters the winner. Because the test slice was sealed from the beginning, that
inflation was measured rather than believed.

---

## 6. Limitations

- **Level labels are contaminated.** When the user encounters a track already in
  the collection, they often lower the duplicate's level so it surfaces less,
  without revisiting its colour. Some level labels therefore encode a fact about
  the library rather than about the music, which no audio model can learn. 676
  redundant copies exist; 74 groups carry conflicting labels. Cleaning these up
  and re-measuring is the one known, unexploited improvement for the weakest
  axis.
- **Rare labels cannot be learned or measured.** Four labels have fewer than 30
  examples; `Blue_A` has 5. They contribute nothing but noise to per-label
  figures.
- **Only 60% of the collection is labelled**, and the learning curve has not
  flattened, so these numbers are a floor rather than a ceiling.
- **One user, one collection.** Nothing here shows the approach transfers to a
  different person's vocabulary, though nothing in the design depends on this
  one.

---

## 7. Ruled out

- **Combining models.** Cross-validation showed no combination beats CLAP alone.
- **MERT and Whisper** as ensemble partners — both weaker, neither additive.
- **MFCC** as anything but a floor: +1.9 versus CLAP's +23.4 settles whether a
  neural model earns its weight.
- **PCA** dimensionality reduction: worse than regularising the full space.
- **Class-balanced training weights**: cost 10-18 points at this imbalance.

---

## 8. Next

1. **Phase 1 — the CLI that uses this.** The result is good enough to be useful
   with a confidence threshold.
2. **Duplicate cleanup, then re-measure level.** The clean test of whether the
   weak axis is contaminated or genuinely hard.
3. **More labels.** The curve is still climbing.
4. **Cheap unexplored ideas**: mean+standard-deviation pooling, more windows per
   track, per-label thresholds instead of one global threshold.

---

## Appendix A — bugs found, and what they would have cost

Every one of these was silent. None raised an error; each would have produced a
confident, wrong conclusion.

| bug | consequence if unnoticed |
|---|---|
| L2-normalised a feature vector whose dimensions had different units — rolloff in Hz swamped MFCCs, annihilated zero-crossing rate | The MFCC baseline scored *below* chance and would have read as "audio features don't work here" |
| `class_weight="balanced"` at 412x imbalance | Cost 10-18 points; looked like a modelling failure |
| Regularisation left at the untuned default | Cost ~10 points, the single largest miss |
| Variable shadowing clobbered the split array; `list == str` is `False`, and numpy indexing with `False` returns an empty array rather than raising | Learning curve silently produced no rows, exit code 0 |
| Hashed the ID3 tag region as part of track identity | Two copies of one track collided; and every tag write would have invalidated that track's cached embedding |
| Comparison subset selected by sorting on content hash | Changing the hash function silently re-drew the sample and stranded completed work |
| `ProcessPoolExecutor` hangs instead of raising when the OS kills its worker | Two runs stalled silently for an hour each |
| `cached` count computed after a `limit` truncated the work list | Reported a run as complete when it was 17% done |

---

## Appendix B — terminology

**Baseline (majority-class baseline)** — the score from always guessing the
single commonest label, ignoring the audio. The number any real model must beat.
Here 22.9%.

**Accuracy** — the fraction of predictions that exactly match the true label.

**Top-k accuracy** — the fraction where the true label is among the model's k
best guesses. Useful when the model offers a shortlist rather than one answer.

**Precision (on kept predictions)** — of the predictions the model was confident
enough to make, the fraction that were correct. Rises as the confidence
threshold rises, because low-confidence guesses are discarded.

**Confidence threshold / abstain** — the model reports how sure it is. Below a
chosen threshold it declines to answer rather than guessing. Trading coverage
for precision.

**Embedding** — a fixed-length list of numbers describing a track's sound,
produced by a pretrained model. Two tracks that sound alike get similar numbers.

**Feature extractor (frozen)** — using a pretrained model only to produce
embeddings, never adjusting it. All label-specific learning happens in a
separate, small classifier on top.

**Linear probe** — the standard name for that arrangement: frozen embeddings
plus a simple linear classifier. It tests what a representation already knows.

**Logistic regression** — the simple classifier used here. Learns one weighted
sum per label and picks the highest.

**Regularisation (`C`)** — how strongly the classifier is discouraged from
fitting noise. Lower `C` means stronger discouragement. With 512 numbers per
track and a few thousand examples, this mattered by about 10 points.

**Standardisation** — rescaling each of the 512 numbers to a comparable range
before training, so that dimensions with large units do not dominate.

**L2 normalisation** — scaling a whole vector to unit length. Appropriate when
all its dimensions share units; actively harmful when they do not, as one bug
above demonstrates.

**Pooling** — reducing many per-moment values to one per track, here by
averaging over time and over windows.

**Training / validation / test split** — training teaches the model; validation
is used while exploring options; test is held back untouched and read once, to
get a number not shaped by any decision.

**Cross-validation (k-fold)** — repeatedly training on most of the training data
and scoring on the rest, then averaging. Gives a much steadier estimate than a
single small holdout, without spending the test set.

**Selection bias / inflation** — the flattery that comes from picking the best of
many options judged on the same data. Measured here at 2.2 points.

**Noise floor** — how large a difference must be before it means anything, given
the sample size. About 2 points on 861 tracks; about 3 on 269.

**Class imbalance** — some labels being far commoner than others; 412x here
between the largest and smallest.

**Content hash** — a short fingerprint of a file's audio, used to identify a
track regardless of its filename or tags.

**MFCC** — mel-frequency cepstral coefficients, a classical compact description
of timbre. Used here as the no-model floor.

**Mel spectrogram** — a representation of sound as energy across frequency bands
over time, spaced to match human hearing. The input format most audio models
expect.

**CLAP / MERT / Whisper** — the pretrained models compared. CLAP is trained to
match audio with text descriptions; MERT is trained on music alone; Whisper is a
speech recognition model whose encoder can be reused for audio features.

**ONNX** — a portable format for shipping a trained model so it can run without
Python or PyTorch.

**MPS** — Apple's GPU backend, used to run the models on the M1's graphics
hardware.
