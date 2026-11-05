# Cue points

Every track in a `label` round also gets **cue points**: hot cues A–H, memory
cues, and optionally loops, written into rekordbox next to its colour, rating,
key and energy.

Only tracks with nothing of yours on them get cues: no cues at all, cues
another tool generated (named "Cue 1", "Cue 2", ...), or a lone memory cue on
the first downbeat. **A track with any cue you set yourself is never touched.**

The work is split in two, so the learned part stays general and the personal
part stays a setting:

| | what it decides | how |
|---|---|---|
| **transition points** | the four bars where the track changes enough to mix around: a mix-in, two between, a mix-out | learned from your own cues |
| **placement** | where cues go around each transition, which are loops, memory cues | plain settings, nothing learned |

**At a glance**

| | |
|---|---|
| hand-set cues reproduced, exact bar | **49%**, held out, 646 tracks |
| tracks with at least half their cues exact | **63%** |
| misses that are exactly 8 or 16 bars off | about half of them |
| speed | ~40 s per 100-track round on an M1 Pro, cached after that |
| learning from your data | yes: retrains on your cues as they accumulate |

Code: [src/vibecheck/cues.py](../src/vibecheck/cues.py) ·
model: [src/vibecheck/cue_model.pt](../src/vibecheck/cue_model.pt)

---

## Transition points

### How they are found

1. **Bars from rekordbox's own grid.** The TEMPO entries in the xml give every
   beat and where each bar starts, so every cue lands on a downbeat rekordbox
   agrees with. Fix the grid in rekordbox and the cues follow. No beat or
   downbeat detection happens here.
2. **What each bar sounds like.** The whole track is decoded once and each bar
   summarised: a 64-band spectrum, the twelve notes, loudness, brightness and
   how busy it is.
3. **The track's sections.** Laplacian structural segmentation (McFee & Ellis
   2014): a bar-by-bar similarity matrix is split into sections, the
   clusters of bars that sound alike, wherever and however long they are.
   This runs at several section counts, and separately on the whole spectrum,
   the bass, the upper spectrum and the notes, because a bassline swap and a
   new melody are different kinds of change.
4. **Reading the whole track.** A small bidirectional recurrent network reads
   every bar at once, with its sound, its sections and its place in the
   track, and scores each bar three ways: as a transition, as the mix-in, and
   as the mix-out.
5. **Choosing four.** The best mix-in, the best mix-out after it, and the two
   best transitions between them, at least 8 bars apart.

### How it learns from you

It learns from the transition points behind your own cues. With
`lead = [16, 8]`, a cue at bar *c* and another at *c* + 8 mean a transition at
*c* + 16, and a track counts once your cues show at least three transitions.

Cues this app wrote are recorded in `labels.db`. A track whose hot cues still
sit exactly where they were written is not learned from, since that would be
learning from its own guesses. Once you move one, the track is yours. `label`
trains again on its own once you have 50 more such tracks than the model in use
was trained on. That takes a few minutes; `vibecheck cues --train` does it on
demand. Your model is saved in `.vibecheck/cue_model.pt` and takes over from
the shipped one.

The shipped model was trained on the author's 646 hand-cued tracks, mostly
techno and house.

---

## Placement

Settings under `[cues]` in `.vibecheck/config.toml`:

| setting | default | |
|---|---|---|
| `write` | `true` | cues with every round, and with `vibecheck cues` |
| `lead` | `[0]` | bars ahead of each transition to cue |
| `loops` | `{}` | hot-cue slots that are loops, and their length in bars |
| `memory` | `true` | a memory cue on every hot cue, and one on the first downbeat |
| `retrain_after` | `50` | more tracks of your own cues before it learns again |

**The default, `lead = [0]`,** puts one cue *at* each change: A is the mix-in,
D the mix-out, B and C between.

**A lead-in pair, `lead = [16, 8]`,** puts two cues 16 and 8 bars *before*
each change. That gives you somewhere to start the next track a phrase or two
ahead, and it fills all eight slots: A/B the mix-in, C–F between in time
order, G/H the mix-out. With loops on the first cue of the mix-in and mix-out
pairs, there is room to wait when two tracks do not line up:

```toml
[cues]
lead = [16, 8]
loops = { A = 16, G = 16 }
```

Hot cues are green, loops orange.

---

## How it was measured

646 tracks whose cues were set by hand, all in one consistent style (16/8-bar
pairs), split five ways by track. Each fifth was predicted by a model trained
on the other four, placed as 16/8 pairs, and compared with the hand-set hot
cues bar for bar.

| | |
|---|---|
| hand-set cues reproduced, exact bar | 49% |
| tracks with at least half exact | 63% |
| tracks with none exact | 7% |
| transition points exact / within one bar | 42% / 45% |

Where the hand-set transitions went that it did not find exactly:

| distance to the nearest predicted transition | share of all hand-set transitions |
|---|---|
| exact | 42% |
| exactly 8 bars | 14% |
| exactly 16 bars | 14% |
| exactly 24 bars | 5% |
| 32 bars or more | 9% |
| anything else | 16% |

A miss is most often a whole phrase or two off: it found a real change in the
track, just not the one that was cued. The mix-in and mix-out are the hardest
part, at 21% and 20% exact. Where to come in and go out depends on more than
the track, such as whether you play its first or second half, and a
neighbouring change is often as usable.

**What this does not show:** whether a predicted cue that differs from yours
would work as well when mixing. Several looked like it when compared by eye.
The numbers only count exact agreement.

---

## Commands

```sh
uv run vibecheck label 100        # cues for the batch, with everything else
uv run vibecheck cues --dry-run   # every track it has labelled: what would change
uv run vibecheck cues             # ... and write them, with a playlist of
                                  # only the tracks that changed
uv run vibecheck cues mine.m3u8   # one playlist
uv run vibecheck cues --all       # the whole collection
uv run vibecheck cues --train     # learn from your cues now
```

**Importing replaces all of a track's cues** with the ones in the xml, hot and
memory, which is why tracks with yours are never written. A track with no cues
in the xml is ignored on import, and its old ones stay.

---

## Credits

- Laplacian structural segmentation: B. McFee and D. Ellis, *Analyzing song
  structure with spectral clustering*, ISMIR 2014.
- [librosa](https://librosa.org) for the spectrum, notes and onsets;
  [scikit-learn](https://scikit-learn.org) for clustering;
  [PyTorch](https://pytorch.org) for the network.
