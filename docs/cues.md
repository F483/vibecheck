# Cue points

*2026-11-24; the generic model (v2) 2026-11-26. How the cue model learns, kept
in line with colours.*

Two models read the same per-bar features with the same network:

- **v2, generic** (`cues::FORMAT_V2`): learns the cues themselves, hot cue
  slots A–H, beats, loops and memory cues, from yours. The app trains it
  as soon as you have enough cues of your own.
- **v1, shipped**: four **transition points** per track (mix-in, two
  between, mix-out), with the cues **placed** around them by settings
  (`[cues] lead`, `loops`, `memory`). What a new user starts with. The
  prototype's write-up of it is `prototype/docs/cues.md` in `prototype.zip`.

## Learning, as colours do

| | colours | cues |
|---|---|---|
| model | logistic regression on the audio embedding | bidirectional GRU over the track's bars, three averaged |
| learns from | your labels | your cues: set in the player, or in rekordbox (`cue_train::yours`) |
| a guess left alone | not trained on (trust 1) | not trained on: cues this app wrote and you left where they were |
| a guess shown and kept | kept, 0.5 | kept, 0.5: the suggestion confirmed unchanged in the player |
| your correction | confirmed, 0.7 | confirmed, 0.7: changed in the player, or in rekordbox (any cue, yours or ours: a sync sees the change) |
| there before vibecheck | existing, 0.3 | existing, 0.3: your hand-set cues in rekordbox |
| played since | played, 1.0 | played, 1.0: played after the cues were set |
| times | rating (`[rating]`), recency (`[recency]`) | the same, from the track's stars and when you last set, edited or played them |
| asked again | conflict cards: the model sure of another colour | cue conflict cards: the model would put most of your hot cues elsewhere |
| retrains | in the background when behind your labels | in the background once `[cues] retrain_after` (50) tracks are new, or changed since it was trained (set, edited, played) |
| once written | only you change it | only you change it (`cues::writable`) |

**Cue conflicts** (2026-11-26). After training, the model decodes every
track it learned from; where it would set at most a quarter of your hot cues
itself (`CONFLICT_SHARE`), even having learned them, and it is on average
more likely right than not about its own (`CONFLICT_SURE`, 0.5: a colour
conflict needs the model sure too), the track is a cue conflict (60 of 1,859
on the first run) (`.vibecheck/cue_conflicts.json`, surest first: the model's own
cues, how sure it is of them). The queue deals one every 9th card, once a
model of yours found it, not again within 60 days (as colour conflicts), and
not if you changed its cues since. The card opens in Cues with your cues on
the pads and the model's faint on the waveform; keeping yours or moving them
is a cue decision like any (confirmed, 0.7), and the colour stays the label
it was unless you change it.

**Edits in rekordbox.** Every sync keeps each track's cues as it saw them
(`cues_seen`, schema v11); cues different the next time were changed by you,
and count as confirmed from then on (a cue of yours from before vibecheck too).
Hot cues where an export of ours last put them are not an edit: rekordbox only
has them once you import that export.

The one difference, on purpose: cues retrain in steps of 50 tracks, not on
every new one, because training takes minutes (about 6 on an M1 Pro for
1,800 tracks), not seconds. The model it replaces is kept as
`cue_model.prev.bin`.

## Training (`core/src/cue_train.rs`)

The prototype's network (Linear → GELU → 2-layer bidirectional GRU(64)), its
gradients written by hand (checked against finite differences in a test),
AdamW 2e-3 / 1e-2, 60 epochs, batches of 16, three seeds. v2's outputs per
bar (`cues::out`):

| output | loss | used for |
|---|---|---|
| each slot A–H: its bar, or empty (from the mean over the bars) | cross-entropy over the bars + empty | how many hot cues |
| a hot cue on this bar | BCE, positives ×20 | which bars |
| which slot a hot cue here is in | cross-entropy, 8 | which slot |
| the beat in the bar | cross-entropy, 4 | beat |
| loop length, per slot (none, 4 … 256 beats) | cross-entropy, 8 | loop |
| a memory cue on this bar, and its loop | BCE (unweighted), cross-entropy | memory cues |

Each track's loss is weighted by trust × rating, as above.

**Decoding** (`decode_generic`): as many hot cues as it expects slots filled
(the sum over slots of not-empty), on the bars it ranks highest for a hot
cue, at least 4 bars apart; then each of those bars the slot that fits it,
all at once (an exact matching); beat and loop where they sit; memory cues
where one is more likely than not, up to 10. Ranking bars first, rather than
each slot picking its own bar, was worth 5 points (45.1% → 50.1%).

Held out five ways (`cargo run --release --example cue_train`), measured only
on cues set by hand (1,140 tracks; 636 kept suggestions are trained on, never
measured):

| | v2 | v1 |
|---|---|---|
| on v1's 646 tracks: your hot cues reproduced, exact bar | **50.1%** | 50.1% |
| of the hot cues it set, yours | **52.1%** | about 45% (8 a track) |
| tracks with at least half exact | **67.0%** | 62.7% |
| tracks with none exact | **5.5%** | 6.3% |
| the same, setting 8 a track as v1 does | **54.6%** | 50.1% |
| on all 1,140: your hot cues reproduced | 46.7% | not measured |
| on a bar of yours: beat right / loop right | 99.5% / 80.4% | (settings) |
| … in your slot too | 57% of them | |
| memory cues: yours found / of its, yours | 48.1% / 64.6% | (copies of the hot cues) |

v1's 646 are the tracks whose cues fit its pairs; the other 494 tracks set by
hand don't, which is why v2 does less well on all of them. Gate (decided
2026-11-24): v2 replaces v1 only if it reproduces at least v1's 50.1%; it does,
and is better at the same number of cues.

Also from the command line: `vibecheck cues --train`, and `label` / `cues`
learn first when the model is behind.

## Plan: cues from scratch, like colours (2026-11-24; steps 1–4 built)

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

1. Mockup and `ui/decisions.md`: the cue part of the card (board 05).
   Keyboard by key position (Neo 2). **Done.**
2. Data: a cue decision per track (`cue_decisions`, schema v9: cues as set,
   the suggestion shown, which card); Export writes them like labels.
   **Done**, with trust as for labels, and sync reading cue changes in
   rekordbox as corrections (2026-11-26).
3. The editor in the player card (Tab). **Done.**
4. The generic model, weighted, validated against 50.1%. **Done**
   (2026-11-26, above).
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
