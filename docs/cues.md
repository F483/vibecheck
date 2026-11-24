# Cue points

*2026-11-24. How the cue model learns, kept in line with colours.*

The work is split in two: **transition points** (four bars per track where it
changes enough to mix around: mix-in, two between, mix-out) are learned;
**placement** (where cues go around them, loops, memory cues) is settings,
`[cues]` in `config.toml`. The prototype's write-up of the method and its
measurement is `prototype/docs/cues.md` in `prototype.zip`.

## Learning, as colours do

| | colours | cues |
|---|---|---|
| model | logistic regression on the audio embedding | bidirectional GRU over the track's bars, three averaged |
| learns from | your labels | your own cues (`cue_train::yours`) |
| a guess left alone | not trained on (trust 1) | not trained on: cues this app wrote and you left where they were |
| your correction | trained on | trained on: move a written cue in rekordbox and the track is yours |
| retrains | in the background when behind your labels | in the background when behind your cues by `[cues] retrain_after` (50) tracks |
| once written | only you change it | only you change it (`cues::writable`) |

The one difference, on purpose: cues retrain in steps of 50 tracks, not on
every new one, because training takes minutes (about 2.5 on an M1 Pro), not
seconds. Not weighted by trust or rating yet: every track of your cues counts
the same.

Until cues can be edited in the app, rekordbox is the only place a correction
comes from, so the model only moves once you move cues there.

## Training (`core/src/cue_train.rs`)

The prototype's `fit` (PyTorch), with the gradients written by hand: same
network, losses (transition BCE with positives ×20; cross-entropy over the bars
for the mix-in and the mix-out), AdamW 2e-3 / 1e-2, 60 epochs, batches of 16,
three seeds. A unit test checks the gradients against finite differences. The
model is saved as `.vibecheck/cue_model.bin` and takes over from the shipped
one (trained on the same 646 tracks in the prototype).

Held out five ways on the 646 tracks (`cargo run --release --example cue_train`):

| | Rust | prototype |
|---|---|---|
| your cues reproduced, exact bar | 50.1% | 49% |
| tracks with at least half exact | 62.7% | 63% |
| tracks with none exact | 6.3% | 7% |
| transition points exact | 42.8% | 42% |
| mix-in / mix-out exact | 21.2% / 19.0% | 21% / 20% |

Also from the command line: `vibecheck cues --train`, and `label` / `cues`
learn first when the model is behind.

## Later

- Edit cues in the player, before export: the source of corrections.
- Simpler cue settings.
- `cues::writable` treats this app's imported, untouched cues as yours, so they
  are never rewritten and the player shows them as your own; revisit with the
  two above.
