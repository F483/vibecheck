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

## Plan: cues from scratch, like colours (2026-11-24, not built)

Four transitions with a 16 + 8 bar pair each is one user's taste, and even
for them it depends on the track. Many users will want 8 hot cues on 8
important parts, others any pattern of their own. So: no styles to define,
cues edited per track in the app, and a model that learns positions directly.

### Decided

- **Edited in the card**, in one configurable spot with colour, stars, key
  and energy: what the card shows and asks is a setting, so a user who
  doesn't care about cues keeps fast cards.
- **Generic, no styles.** A track's cues are what rekordbox has: up to 8 hot
  cues (slot A–H, position, optional loop length) and memory cues (up to 10,
  position, optional loop). "4 pairs" or "8 marks" are not settings; they
  are patterns the model picks up from what you do, per track.
- **Your old 646 hand-cued tracks stay, at low weight**, as an old rekordbox
  colour does (existing, 0.3). To be checked on other users' libraries too.
- **Trust as colours** (`label-weights.md`), times the rating factor:

  | cues | trust |
  |---|---|
  | your old hand-set cues, from rekordbox | existing, 0.3 |
  | suggestion shown in the card, confirmed as it was | kept, 0.5 |
  | moved, added, deleted or loop changed in the card, or changed in rekordbox | confirmed, 0.7 |
  | written and never shown or touched | guessed, not trained on |
  | played with those cues since | played, 1.0 |

  This removes today's two differences from colours (no weighting; steps of
  50 stay only because training takes minutes).

### The model, generic

Same network over the track's bars (it already reads the whole track), new
outputs per bar:

- **per hot cue slot, A to H, where it goes or that it stays empty**: eight
  outputs, each a choice over the bars plus "empty", as the mix-in and mix-out
  are chosen today. Slots are not ordered or named: a user who fills them in
  time order and one who keeps A for the drop wherever it is are both
  learned, per track;
- **which beat of the bar** for each cue (1–4 in 4/4), so cues can sit on
  beats, not only on downbeats; the network still steps by bar;
- **loop length** if it is a loop (none / 4 / 8 / 16 / 32 bars);
- **memory cues** as a set of positions (up to 10, no slots), with beat and
  loop the same way.

Trained on positions directly instead of transitions + lead: a pair 16 and 8
bars before a change is just two positions it learns to put there. The
transition split was for a fixed placement setting, which goes away.
Validation as before, held out five ways on the 646: it replaces today's
model only if hot cues exact is not below today's **50.1%**.

### Order

1. Mockup and `ui/decisions.md`: the cue part of the card, and the card's
   configuration. Keyboard by key position (Neo 2).
2. Data: a cue decision per track (cues as set, what was shown, which card),
   trust from it as for labels; Export writes cue decisions like labels;
   sync reads cue changes in rekordbox as corrections.
3. The editor in the player card.
4. The generic model, weighted, validated against 50.1%.
5. Settings: placement settings go; `cues::writable` reworked (a cue decision
   made in the card is yours to write, whatever rekordbox had).

Decided 2026-11-24: slots learned per slot, not fixed to time order (tastes
differ, keep it generic); cues on beats too.

Also decided 2026-11-24:

- **A fresh start in the card.** A track's cues in the card start from the
  model's suggestion, not from the cues it has in rekordbox; once you
  confirm them there, Export replaces what rekordbox had.
- **The model in use is kept, and can be reset.** Today's model (the shipped
  one, trained on the 646) stays the starting point. Training never throws
  the previous model away: it is kept next to the new one, and Settings can
  go back to it or to the shipped one.
- **A new user's first suggestions: decided later**, from how your own
  editing goes. Both get built and compared: the shipped model (one user's
  pairs) and a neutral one (one cue per strong change).
- **Memory cues learned too**, like hot cues (positions, beats, loops), not
  a copy of the hot cues plus the first downbeat; that setting goes.
