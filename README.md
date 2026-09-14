# Vibe Check

Learns how *you* label music, from the audio, and applies it to the rest of
your collection.

Not a genre classifier. There is no built-in taxonomy, no Discogs or Beatport
vocabulary, and the app never interprets a label — it only learns which sounds
go with which of your strings. The default vocabulary maps onto the eight
colour tags DJ software already exposes, so it fits an existing workflow rather
than asking you to invent one.

Local, MIT licensed. Developed on macOS; the code is portable — CUDA, Metal or
CPU, and every other dependency is cross-platform.

---

## Install

Requires [uv](https://docs.astral.sh/uv/) and ffmpeg. Uses CUDA or Apple Metal
if present, CPU otherwise — on CPU expect roughly 3–5× longer to analyse a
batch, which is slow but workable.

```sh
brew install uv ffmpeg
git clone https://github.com/F483/vibecheck && cd vibecheck
uv sync
```

The first run downloads the audio model (~2 GB) and writes nothing outside
`<your collection>/.vibecheck/` and the genre tag of tracks it labels.

## Usage

```sh
# point it at your collection (default: ~/Music/Collection)
uv run vibecheck scan

# label some tracks by hand first -- it needs about 200 to be useful
uv run vibecheck status

# then, each round:
uv run vibecheck label 300          # picks 300 unlabelled tracks, labels what
                                    # it can, writes batch-<date>-<time>.m3u8
#   ... import that playlist into your DJ software, correct what is wrong ...
uv run vibecheck sync               # reads your corrections back and retrains
```

Other things you may want:

```sh
uv run vibecheck label 50 --dry-run       # see what it would say, change nothing
uv run vibecheck discard batch-….m3u8     # throw a batch away, free its tracks
uv run vibecheck label 300 --include-unconfirmed   # reuse tracks from a batch
                                                   # you have not corrected yet
```

A batch in progress is excluded from the next one, so batches never overlap and
a new batch cannot overwrite corrections you have not synced. `discard` clears
only tags still holding exactly what the app wrote — anything you have since
edited is treated as a correction and kept.

That loop is the whole product. Each round the model gets better, so each round
you correct less.

**What it writes:** the ID3 genre tag, and nothing else. Track identity is a
hash of the audio only, so writing a label never changes what a track *is* and
never invalidates its cached analysis.

**Configuration** lives in `<your collection>/.vibecheck/config.toml` — copy
[the default](src/vibecheck/default_config.toml) to start. It declares your
label vocabulary, how labels group into coarser levels, and one dial:
`misleading_cost`, how bad it is to be given a wrong label.

## Status

**Prototype built and in use.** The research question — can a model learn one
person's labels from audio well enough to save real work — is answered, and the
loop above is running on the reference collection.

## What it achieves

Measured on 861 tracks never used for any modelling decision, from a reference
collection of 14,194 mp3s with 8,619 hand-applied labels.

**Per 100 new tracks**, with the default setting:

```
55 get exactly the right colour
32 get a colour that is close — right hue pair, or at least right tone
 9 are completely wrong
 4 get a hue family instead of a colour
```

That is **56% less labelling work** than doing it by hand, measured as the
number of binary decisions you still have to make. A more cautious setting
trades exact colours for fewer mistakes and a "don't know" answer:

```
setting              exact   close   wrong   silent   work saved
misleading_cost 1      55      32       9        0       56%
misleading_cost 2      46      21       5        2       42%
misleading_cost 3      33      10       2       16       33%
```

Accuracy by granularity, against always guessing the commonest answer:

```
                      model    baseline
colour  (8 values)    56.0%      26.1%
hue     (4 values)    66.1%      32.8%
tone    (2 values)    80.1%      66.7%
```

## The result that matters most

**The model reproduces the labelling more consistently than the person does.**

300 tracks were cleared and relabelled blind, then compared against their
previous labels:

```
              you vs your past self     model
colour               45.3%              56.0%
hue                  65.7%              66.1%
tone                 70.3%              80.7%
```

So 56% is not a ceiling the model is stuck beneath — it is *above* the noise
floor of the target. Six independent approaches all converged at 56–58%
because there is no further signal to extract, not because the right technique
was missing.

That also changes what the tool is for. It is not only a labour saver: it
applies one stable judgement across 14,000 tracks and several years, which is
something no one can do by hand. See [docs/ceiling.md](docs/ceiling.md).

## How it works

```
mp3 → 24 × 10 s excerpts → CLAP embedding → logistic regression → label
                                                     ↓
                        cheapest of: colour / hue / tone / say nothing
```

- Pretrained audio models are used **only as feature extractors**. They never
  see a label. Everything about your taste lives in a small classifier trained
  on your own tags, which is what lets the same pipeline serve a completely
  different vocabulary.
- There are **no confidence thresholds**. Every option is scored by how much
  work it leaves you — how many binary choices remain — and the cheapest wins.
  Near-misses get partial credit, so the model is pushed toward being close
  rather than boldly wrong, and the colour → hue → tone → silence cascade falls
  out of the arithmetic rather than being hand-tuned.
- **One dial**: `misleading_cost`, how bad it is to be misled by a wrong label.

## Current best setup

```
model        laion/larger_clap_music_and_speech, frozen
audio        48 kHz, 24 windows × 10 s, mean and standard deviation pooled
classifier   standardise → logistic regression, C = 0.001 (chosen by CV)
decision     expected-cost policy over colour / hue / tone / abstain
storage      .vibecheck/ in the collection root: labels.db, cache.db
labels in    rekordbox XML export (colour tag + star rating), or ID3 genre
```

Full detail, including every parameter that matters and why:
[docs/setup.md](docs/setup.md).

## What was ruled out

Recorded so it is not re-attempted. Each was measured, not assumed.

| approach | result |
|---|---|
| 5 encoders: MFCC, MERT, Whisper, CLAP, MuQ | converge within ~2 points |
| ~12 classifiers: kNN, SVC, MLP, one-vs-rest, PCA, per-label thresholds | within 2 points |
| 20× more training data | learning curves flat |
| +1,857 labels imported from rekordbox | +0.3 |
| fine-tuning the encoder | +0.6, inside noise |
| probability calibration | no effect on decisions |
| weighting by DJ play count | no effect once rating is controlled for |
| hearing 4 minutes per track instead of 90 s | +0.2 |

The two changes that *did* matter were both about how the model is used rather
than what it is: tuning regularisation (+11) and replacing confidence
thresholds with the cost policy (17 → 55 exact colours per 100).

## Open, deliberately

- **CLAP has never been exported to ONNX.** It decides whether a packaged app
  is ~500 MB or ~2 GB. Half a day of work, parked until packaging actually
  starts — see [docs/design.md](docs/design.md) §7.
- **Cloud encoders are untested on the current labels.** Worth ~$10 to settle,
  once there are around 1,000 fresh labels — before that it would mostly
  measure noise. See [docs/plan-b-cloud.md](docs/plan-b-cloud.md).
- **The encoder comparison was run against the retired labels.** Re-running it
  on fresh ones is free and may not give the same answer.

## Documentation

- [docs/setup.md](docs/setup.md) — the working configuration, reproducibly
- [docs/accuracy.md](docs/accuracy.md) — what accuracy to expect, and what limits it
- [docs/ceiling.md](docs/ceiling.md) — label consistency, and why the model beats it
- [docs/design.md](docs/design.md) — design decisions and their reasoning
- [docs/phase0-report.md](docs/phase0-report.md) — the backend comparison in full
- [docs/plan-b-cloud.md](docs/plan-b-cloud.md) — contingency, and when *not* to use it

## License

MIT — see [LICENSE](LICENSE).
