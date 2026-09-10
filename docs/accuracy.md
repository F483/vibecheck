# What accuracy to expect

Measured with the shipped setup (CLAP embeddings + logistic regression,
`docs/setup.md`) on 8,619 labelled tracks: 6,891 training, 861 test, test never
used to choose anything about the model.

---

## Headline

| level | classes | accuracy | baseline | gain |
|---|---|---|---|---|
| **colour** (Red, Orange, …) | 8 | **56.0%** | 26.1% | +29.9 |
| **hue** (Warm/Acidic/Cool/Vibrant) | 4 | **66.2%** | 32.8% | +33.4 |
| **tone** (Dark/Light) | 2 | **80.1%** | 66.7% | +13.4 |

Cross-validated on training data, which is the steadier estimate:
colour 55.1% ±2.3, hue 64.7% ±2.7, tone 76.3% ±4.4.

The baseline column is what you get by always guessing the commonest answer and
never listening. That, not 100%, is what these numbers should be read against.

## Confidence: coverage versus precision

The model reports how sure it is. Discarding its least confident predictions
raises accuracy on what remains — this is the dial that makes the tool usable.

**colour (8 classes)**

| threshold | tracks kept | of those, correct |
|---|---|---|
| none | 100% | 56% |
| 0.5 | 53% | 69% |
| 0.6 | 38% | 71% |
| 0.7 | 24% | 76% |
| 0.8 | 10% | 84% |
| 0.9 | 2% | 95% |

**hue (4 classes)**

| threshold | tracks kept | of those, correct |
|---|---|---|
| none | 100% | 66% |
| 0.5 | 72% | 74% |
| 0.6 | 53% | 80% |
| 0.7 | 36% | 86% |
| 0.8 | 24% | 89% |
| 0.9 | 13% | 91% |

**tone (2 classes)**

| threshold | tracks kept | of those, correct |
|---|---|---|
| none | 100% | 80% |
| 0.6 | 87% | 83% |
| 0.7 | 74% | 85% |
| 0.8 | 54% | 88% |
| 0.9 | 32% | 94% |

Reading these together is what the cascade does: take the finest level that
clears its bar. Roughly, at a target of ~85% per level it labels 84% of tracks,
mostly at tone; at ~90% it labels 72%, with about an eighth getting a full
colour.

## Learning curves — and the correction they force

Test accuracy against number of training tracks:

```
colour   344:50.6%   689:52.3%   1378:54.2%   2756:55.3%   4823:56.9%   6891:56.0%
hue      344:63.3%   689:63.6%   1378:64.8%   2756:64.9%   4823:65.5%   6891:66.2%
tone     344:78.6%   689:79.0%   1378:79.4%   2756:78.9%   4823:80.0%   6891:80.1%
```

**These are flat.** Twenty times more labels buys +5.4 points on colour, +2.9 on
hue, +1.5 on tone — and colour peaks at 4,823 then goes slightly *down*, which
is noise, not learning.

This overturns an earlier conclusion in `docs/phase0-report.md`, which said the
curve had not flattened and that more labels were the largest available lever.
That was measured on the 24-way joint task, where the curve genuinely was still
rising. Measured per level, it is not. **Labelling the remaining 5,575 tracks
should be expected to buy about one point.**

Tone is the extreme case: 78.6% from just 344 training tracks, 80.1% from
6,891. It is essentially free.

## Per-window experiment (24 windows, mean+std)

Two hypotheses tested with a 9-hour re-embed keeping every window separate:
that mean-pooling blurs the section of a track the label describes, and that
9 x 10 s was too little of a ~6 minute track.

```
colour, validation
   9 windows, mean          55.6%     (the shipped setup)
   9 windows, mean+std      55.2%
  24 windows, mean          56.7%
  24 windows, mean+std      58.5%     <- best, +2.0
```

- **Both changes are needed.** More windows alone: +0.2. mean+std alone: worse.
  The standard deviation across a track only becomes a stable estimate with
  enough windows to measure it.
- **Colour only.** hue 68.4 -> 68.3, tone 79.6 -> 78.9.
- **The stated hypothesis was wrong.** If a track's label came from its peak
  section, predicting from the single most confident window should have won. It
  came last (55.1%). Training on all 165,384 windows individually reached
  exactly 58.5% -- the same as pooling the same information better. There is no
  characteristic moment carrying the label; the small gain is from how much a
  track *varies*, not from finding its best part.

Cost: 2.7x the embedding time (0.3 vs 0.7 tracks/s) for +2.0 on one axis.
Worth adopting only if colour accuracy matters more than embedding throughput.

## What limits this

Four independent representations were tested — CLAP (audio-text contrastive),
MuQ (music-only masked prediction), MERT (music self-supervised), Whisper
(speech recognition encoder) — plus MFCC as a floor. On colour, CLAP and MuQ
land within 0.3 points of each other. Classifier variations (kNN, SVC, MLP,
one-vs-rest, per-label thresholds, factorised prediction, PCA, class weighting)
all land within about two points.

So the ceiling is not the choice of frozen representation, not the classifier,
and not the amount of labelled data. All three have been tested and none of
them moves it.

What remains untested is **fine-tuning** — letting gradients reshape the audio
encoder itself rather than reading a frozen one. Every model tried so far was
frozen, so this is the one qualitatively different intervention left. It may or
may not help; nothing measured so far predicts the answer.

## Honest summary

- **Tone is solved** for practical purposes: 80% unconditionally, 88% on the
  confident half.
- **Hue is usable with a threshold**: 80% on the confident half, 86% on the
  confident third.
- **Colour is not usable unattended**: 56%, and reaching 80% requires discarding
  90% of predictions.
- **More labels will not fix this.** Neither will a better off-the-shelf model.
