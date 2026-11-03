# Key and energy

Every track a `label` round picks also gets a **key** and an **energy level**,
written into the rekordbox xml with its colour and rating: the key into
rekordbox's own key field, and `<key> - Energy <n>` at the start of the
comment. `vibecheck analyse` does the same for chosen tracks, or all of them.

Unlike colour and rating, neither is learned from you. Both are fixed
functions of the audio: the same file always gets the same answer, on any
machine, until the method's version changes. That is what makes them
consistent across a collection and across years.

Code: [src/vibecheck/keyenergy.py](../src/vibecheck/keyenergy.py).

---

## Key

**Method.** Three steps, all from the published literature:

1. **A pitch profile of the track** -- how much of each of the twelve notes it
   contains -- from 24 ten-second windows spread through it, 100 Hz to 5 kHz.
   Only spectral *peaks* count, so drums and noise add almost nothing, and
   every frame counts equally. The track's tuning is estimated first, so a
   record pitched a little off still lands on the right notes. (Harmonic
   pitch class profile: Gómez 2006, after Fujishima 1999.)
2. **Score all 24 keys with templates** (Krumhansl & Schmuckler 1990): the
   profile, in log, is rotated so the candidate tonic comes first and weighed
   by the mode's template, one weight per interval above the tonic.
3. **A fixed preference for minor**, because dance music is mostly in minor
   keys -- a prior over keys, as in Temperley 2002.

The two templates and the minor preference -- 25 numbers -- are fitted to
human-annotated electronic tracks by multinomial logistic regression, so that
the annotated key scores highest, rather than by averaging profiles. On the
training set alone that is worth about four points of accuracy.

Written in alphanumeric notation: `8A` is A minor, `8B` its relative C major;
moving one number is a fifth.

**Accuracy**, against keys annotated by people, on a test set the method never
saw while being built:

[GiantSteps Key Dataset](https://zenodo.org/record/1095691): 600 two-minute
previews of electronic tracks across ten genres, keys annotated by people. 567
have a clear major or minor key; the rest (no key, modal, "other") are left
out, and where annotators gave two keys either counts.

| | exact | MIREX weighted | mix-compatible |
|---|--:|--:|--:|
| **this method** | **78.1%** | **83.8%** | **89.9%** |
| on minor-key tracks (n=447) | 81.2% | | |
| on major-key tracks (n=120) | 66.7% | | |
| classic Krumhansl templates (in Essentia), for reference | 66.0% | 74.0% | 79.4% |

- **exact**: the right key.
- **MIREX weighted**: the standard research score -- 1 for exact, 0.5 a fifth
  away, 0.3 the relative key, 0.2 the parallel key.
- **mix-compatible**: exact, a fifth away or the relative key -- a neighbour
  on the alphanumeric wheel, so it still mixes harmonically.

That is on par with the best open-source key detector measured the same way
on the same tracks (Essentia with its EDM profiles: 77.1-78.5% exact), and
above the 74% reported in research for this dataset (Korzeniowski & Widmer
2017, under a slightly different protocol).

The templates were fitted on a *different* dataset -- the Beatport EDM Key
Dataset, 1,486 tracks, no overlap -- and every choice (features, the minor
preference, regularisation) was made by cross-validation on that set alone.
The test set was only measured, never tuned on. On the training set itself,
cross-validated, it scores 58% exact: that set is deliberately hard, with many
tracks whose key even annotators disagreed on.

Speed: about 2.5 tracks a second on an M1 Pro, so ~40 s for a 100-track
batch, and roughly 1.5 hours for 14,000 tracks once. Cached after that.

**Where it is weak.** Tracks with little harmonic content -- drum tracks,
much techno -- have no strong key, and any key detector is close to guessing
on them. Major keys are harder than minor ones because the minor preference
pulls borderline cases towards minor; that trade was chosen because it is
right more often on dance music.

## Energy

**Method.** Three standard audio descriptors (MPEG-7, 2002), each taken as
the median over the same 24 windows:

| descriptor | measures |
|---|---|
| loudness | how loud the track is, in dB |
| brightness | spectral centroid: how much of the sound is high-frequency |
| noisiness | spectral flatness: noise and distortion against clean tones |

Each is put in units of its spread across a reference set of tracks, the three
are added, and the sum is placed on 1–10 so that a typical club track is **6**
and one standard deviation is **one step**: 7 and 8 are more intense than
most, 3–5 calmer, 1–2 and 9–10 rare.

The reference set is about 2,600 tracks (a sample of the reference collection
plus the GiantSteps set). Only audio statistics come from it -- no ratings,
labels or anyone's judgement of energy went into the scale. The constants are
in `keyenergy.py`, produced by
[scripts/fit_energy_scale.py](../scripts/fit_energy_scale.py), and never
change unless the method's version does.

**What can and cannot be claimed.** There is no ground truth for energy: it is
a perceptual judgement and no public set of human energy ratings is available
for this use. So the claim is consistency, which can be checked:

1. **Same track, different encoding.** 100 random tracks re-encoded at
   128 kbps: the key stayed the same on all 100, the energy on 99, and the
   other was one step off.
2. **Determinism.** The same audio gives the same key and energy on every
   run; the method has no randomness and no state.
3. **Genres come out in a sensible order** (GiantSteps, mean energy):

   | genre | mean energy |
   |---|--:|
   | Electronica / Downtempo | 5.4 |
   | Techno | 5.8 |
   | Tech House, Deep House | 5.9 |
   | House | 6.3 |
   | Progressive House | 6.5 |
   | Dubstep, Trance | 6.6 |
   | Drum & Bass, Electro House | 6.8 |

   Not a measure of accuracy -- every genre spans calm and intense tracks --
   but the scale runs the way people would expect.

On the reference set the levels fall: 1-2 about 1%, 3 2%, 4 5%, 5 19%, 6 47%,
7 24%, 8 1%, 9-10 almost never. So 6 is ordinary, 7 is a lift, 8 is rare, and
4 or below is a noticeably calm track.

## What it writes

| field | written | setting |
|---|---|---|
| rekordbox key (`Tonality`) | `8A` | `write_tonality = true` |
| comment | `8A - Energy 6`, replacing an earlier `<key> - Energy <n>` at the start; anything else in the comment, `/* tags */` included, is kept | `write_comment = true` |

Both settings live under `[rekordbox]` in `.vibecheck/config.toml`. Nothing is
written to the music files, and nothing changes in rekordbox until a playlist
is imported.

## Reproducing it

```sh
# key templates: fit on one dataset, test once on the other
uv run python scripts/fit_key_profiles.py ~/tmp/beatport-key --test ~/tmp/giantsteps-key

# energy scale: audio statistics of reference tracks
uv run python scripts/fit_energy_scale.py --collection ~/Music/Collection \
    --sample 2000 --extra ~/tmp/giantsteps-key/audio

# consistency checks
uv run python scripts/check_key_energy.py --collection ~/Music/Collection \
    --genres ~/tmp/giantsteps-key
```

The datasets are not part of this repository; download them from their
sources below.

## Credits and licences

- **Key templates** (`src/vibecheck/key_profiles.json`): statistics fitted on
  the [Beatport EDM Key Dataset](https://zenodo.org/record/1101082), Ángel
  Faraldo et al., licensed CC BY-SA 4.0.
- **Test set**: the [GiantSteps Key Dataset](https://zenodo.org/record/1095691)
  (Knees et al. 2015), used for measurement only.
- **Methods**: Krumhansl & Schmuckler key-finding (1990); Fujishima pitch
  class profiles (1999); Temperley key priors (2002); Gómez harmonic pitch
  class profiles (2006); MPEG-7 audio descriptors (2002). All implemented here
  from the publications; no third-party key or energy code is used.
- No other software's key or energy output is read, shipped, or used to
  compute any constant here: the key templates come from human annotations,
  the energy scale from audio statistics alone.
