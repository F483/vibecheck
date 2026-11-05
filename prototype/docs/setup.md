# The working setup

The exact configuration that produced 45.6% on the sealed test set
(`docs/phase0-report.md`). Written to be reproducible: every number that
affects the result is stated, because several of them turned out to matter far
more than expected.

---

## Environment

```
machine        MacBook Pro 18,1 — Apple M1 Pro, 16 GB unified memory
os             macOS 15.6.1, arm64
python         3.12.7          (pinned by uv; system python is 3.14 and lacks wheels)
torch          2.14.0          (MPS backend, GPU available)
transformers   5.16.1
scikit-learn   1.9.0
numpy          2.5.3
librosa        1.0.0           (MFCC baseline only; not used by the winning setup)
ffmpeg         7.1.1           (external binary, brew)
```

`uv` manages both the interpreter version and the lockfile. `uv sync` reproduces
the environment exactly; `uv run <cmd>` runs inside it.

## Data layout

State lives in the collection root so it travels with the music:

```
~/Music/Collection/.vibecheck/
  labels.db     tracks table + append-only label log      ~7 MB
  cache.db      embeddings, one row per (audio, model)    ~90 MB
```

Neither file is required to exist beforehand; scanning creates them.

## Stage 1 — identify

- Walk the collection for `*.mp3` (14,194 files, ~199 GB).
- **Identity is a hash of the audio region only**, skipping the leading ID3v2
  tag and any trailing ID3v1 block: `sha256(audio_length + first 256 KB of audio
  + last 256 KB of audio)`, truncated to 32 hex characters.
  Tags must be excluded because writing a label rewrites the tag; hashing it
  would change a track's identity on every write and orphan its cached
  embedding. It also prevents two copies of one track from colliding when a
  large embedded artwork pushes the genre frame past a fixed head window.
- Read the ID3 genre field via `ffprobe`. A value differing from the last
  recorded one is logged as a user correction.

## Stage 2 — excerpt

- **9 windows x 10 s**, centred at evenly spaced points through the track
  (`i/(n+1)` for i in 1..9), so a ~6 minute track contributes 90 s spread across
  its length rather than 30 s from the front. DJ tracks have long intros and
  outros; a single leading window is unrepresentative.
- Decoded by `ffmpeg` seeking directly to each window: ~1 MB read per file
  rather than 14 MB.
- Seeking in mp3 is frame-aligned and the bit reservoir makes the first frames
  after a seek inexact, so each window is decoded 0.5 s early and that lead-in
  is discarded.
- Output: mono float32 at the model's required rate (48 kHz for CLAP).

Window length is not a free parameter for transformer backends: attention cost
is quadratic in it. 3 x 30 s windows drove this machine to 23 GB of swap and the
run was killed; 9 x 10 s covers the same 90 s, costs ~3x less compute, and peaks
around 1 GB RSS.

## Stage 3 — embed

```
model     laion/larger_clap_music_and_speech   (HuggingFace, ClapModel)
input     48 kHz mono, 10 s per window
call      model.get_audio_features(**processor(audio=..., sampling_rate=48000))
output    pooler_output — the projected audio embedding, 512 dims per window
pooling   mean across the 9 windows, then L2 normalise
device    MPS, torch.inference_mode(), empty_cache() after each track
```

The model is **frozen**. It never sees a label. Everything the system knows
about the user's taste lives in the classifier of stage 4, which is what lets
the same pipeline serve a completely different vocabulary.

L2 normalisation is correct here because all 512 dimensions share units. It is
*not* safe on heterogeneous feature vectors — applying it to the MFCC statistics
vector let the Hz-scaled dimensions annihilate the rest.

### Cache key

```
(audio hash, backend name, backend version, preprocessing digest)
```

The preprocessing digest is a hash of the full `Preproc` dataclass — sample
rate, window count and length, FFT settings. Any change to how audio is
prepared lands in a separate keyspace, so two configurations can never be
silently mixed inside one comparison. Bumping a backend's `version` string has
the same effect, which is how the broken and fixed MFCC vectors coexisted
without contaminating anything.

## Stage 4 — classify

```
sklearn pipeline: StandardScaler  ->  LogisticRegression(C=0.001, max_iter=5000)
```

- **Standardisation is per-dimension and fitted on training data only**, so no
  information leaks from held-out tracks.
- **`C=0.001`** was chosen by 5-fold cross-validation on the training split, not
  by scoring the holdout. This single number is worth roughly **10 points**: at
  512 dimensions and a few thousand examples, the default `C=1.0` overfits
  badly. The optimum is a broad plateau from about 1e-3 to 5e-3.
- **No class weighting.** `class_weight="balanced"` costs 10-18 points at this
  imbalance (412x). It optimises balanced accuracy, which is not the objective.

Training takes seconds on 6,891 x 512 floats, which is why the design retrains
from scratch on every change rather than using an incremental algorithm.

## Stage 5 — split and score

Tracks are bucketed deterministically by audio hash, `int(hash[:8], 16) % 10`:

```
bucket 0        test          never touched until the configuration was locked
bucket 1        validation    used while exploring options
buckets 2-9     training
```

Deterministic bucketing means the split is stable as labels accumulate, survives
renames, is reproducible after deleting the database, and cannot drift through a
bookkeeping bug. Because the bucket derives from the audio hash, identical
copies of a track land in the same bucket automatically — they cannot straddle
the split and inflate the score.

Reported metrics: accuracy against the majority-class baseline, per-label recall,
top-k accuracy, and a precision-versus-coverage curve over confidence
thresholds.

## Reproducing it

```sh
uv sync

# index the collection and pick up labels (~2 min for 14k files)
uv run python -c "from pathlib import Path; from vibecheck.index import scan; \
  print(scan(Path.home()/'Music'/'Collection'))"

# embed all labelled tracks (~2.5 h for 8.6k tracks)
bash scripts/embed_run.sh clap labelled 300 40

# score against validation (test stays sealed unless --test is passed)
uv run python -m vibecheck.report ~/Music/Collection clap
```

`embed_run.sh` runs one process per 300-track chunk. A transformer on a 16 GB
machine gets OOM-killed eventually, and one process per chunk means a kill costs
at most that chunk — the content-addressed cache lets the next process resume.
Wrap it in `caffeinate -i` for unattended runs.

## Measured cost

```
scan + tag read        ~2 min      14,194 files, 12 threads
embedding              0.7 tracks/s, ~2.5 h for 8,619 tracks, ~1 GB RSS
training               seconds
prediction             milliseconds once embedded
storage                ~90 MB of embeddings for the labelled set
```

## Known weaknesses of this exact setup

- **The classifier is tuned for the 24-way task.** Colour accuracy is read off a
  model that also has to separate levels, splitting each colour across three
  classes. A model trained directly on colour would pool those examples.
- **It collapses onto frequent labels.** `Purple` and `Red` are over-predicted;
  `Green` and `Blue` sit at ~6% recall. Strong regularisation plus 412x
  imbalance is the likely cause, and no remedy has been tried since the initial
  class-weighting attempt was made with an untuned `C`.
- **Level labels are contaminated** by the practice of downgrading duplicates,
  which encodes a fact about the library rather than the music.
- **CLAP is mel-based**, so a native port has to reproduce its full
  preprocessing chain exactly. MERT, which takes raw waveform, would have been
  easier to ship — but it scores 5 points worse.
