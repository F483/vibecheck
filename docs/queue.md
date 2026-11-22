# Queue: labelling faster without teaching less

*2026-11-12. Decided and built the same day; see Done at the end.*

The fast track is cut (`label-weights.md`, last section). The vibe queue
stays a training queue: "ordered by what teaches the model most". That is
also the slowest order for the user, who wants the library labelled. This
doc looks at where the time goes and what can be changed without the model
learning less.

## Two kinds of speed

- **Per card**: seconds from a card landing in the player to Confirm.
- **Labels needed**: how many cards until the model labels the rest well
  enough to export. The user never labels 14,000 tracks; the model writes
  most of them. Fewer labels for the same accuracy is the bigger lever, and
  it is invisible per card.

Everything below is sorted by which of the two it moves.

## How the queue works today

`queue()` in `core/src/app.rs`; the app asks for 200 cards, asks again after
every 10 decisions (or under 20 left), and when the model is behind the
labels it retrains in the background and asks again. So the model is at most
~10 labels stale, and only the first ~10 of the 200 are ever consumed before
the order is recomputed.

Past calibration, four lists, dealt by position (`train_slot`):

| every | card | what it is | shown guess | new track? |
|---|---|---|---|---|
| 10th | **blind** | a track the model is ≥ 0.8 sure of, asked blind: the accuracy check | no | yes |
| 5th | **conflict** | your label, the model ≥ 0.5 sure of another colour, surest first | no | **no** |
| 7th | **old label** | a rekordbox colour from before vibecheck, the model ≥ 0.5 sure of another; surest first | no | yes (to the app) |
| rest | **unsure** | least sure first | yes | yes |

Over 70 cards: 7 blind, 14 conflicts, 7 old labels, 42 unsure. So 20% of
cards re-ask a track you already labelled, 40% are blind, 60% show a guess.

Nothing rests a conflict: once you re-confirm your colour, the track conflicts
again after the next retrain if the model still disagrees, and comes back in
the next session (only the running session's `_answered` skips it).

## Where a card's time goes

1. **Listening.** Playback starts at 0:00; 1–5 jump to sections. An unsure
   track is one the model finds ambiguous, which usually means you do too,
   so it gets listened to longer than a clear one.
2. **Deciding.** 8 colours × 5 stars. Deciding is fastest when the previous
   card was the same mood (your own observation: sorting the export by colour
   makes the second pass quick); the unsure order jumps moods every card.
3. **Blind cards** (40%) have no guess to confirm with Enter: a full
   decision every time. Conflicts add: it is your track, you are asked to
   defend a label against a model that, where it is surest, is right 65% of
   the time (reference collection, 2026-11-12).

## Proposals

### 1. Conflicts only when the model has earned them

**Labels needed, and per card.** A conflict card is worth asking when the
model's disagreement is likely right. Today it fires at ≥ 0.5 sure, while
the measured accuracy where the model is ≥ 0.8 sure is 65%. Below ~75% most
conflicts end with you repeating yourself: a blind card spent, no new track
labelled, nothing learned (the label gets a fresh timestamp and card, same
colour and weight).

- Scale the conflict rate with the sure-accuracy stat: none below 75%, every
  10th between 75% and 90%, every 5th above. Old labels keep their rate: they
  turn a 0.3 label into 0.7 and bring a track into the app, so they pay even
  when the model is wrong.
- Raise the conflict bar from 0.5 to `SURE` (0.8). A 55% disagreement is a
  coin toss.
- **Rest a re-confirmed conflict.** A conflict you answered with the same
  colour is not asked again for 60 days or until your labels have grown 50%,
  whichever first. Today it can come back next session.

Effect on the reference collection: ~14 of 70 cards become unsure cards (new
tracks, guess shown), the conflict list returns as the model improves.

Where: `queue()`, the `conflicts` filter and `train_slot`; the rest needs
the last `card = 'conflict'` label's `ts` per track (`labels`, v8), one query.

### 2. Deal unsure cards in runs of one mood

**Per card.** Same 200 tracks, different order: group the unsure cards by the
model's guessed hue (or by nearest neighbour in embedding space) and deal
5–10 of one group in a row, then the next. The decision narrows to "is this
one also Aqua, and how good is it", which is the pass you already do by hand
on the export list.

Cost to learning: none in what is asked; a little in order, since within one
retrain window (10 labels) all 10 may be one hue. Mitigation: runs of 5, or
rotate hues per run. The model retrains every 10 labels anyway.

Where: `queue()` after `unsure` is sorted, before dealing: bucket by
`best_colour(p).0`'s hue, deal round-robin in runs. Tags unchanged.

### 3. Pick unsure tracks for coverage, not only uncertainty

**Labels needed.** The 10 least sure tracks are often near-duplicates (same
release, same mix). Labelling the first settles the others; the next nine
labels buy little. Take the 300 least sure and pick the 200 (or the next 10)
that are spread across them, as calibration does (`spread`): one label per
neighbourhood.

Cost: nothing asked is less informative; a few of the very least sure tracks
wait a round. This is standard active learning (uncertainty + diversity) and
the one proposal that should cut the total number of labels.

Where: `queue()`, `spread` over the embeddings of the unsure head. Needs the
embeddings already loaded there (`embed::load`).

### 4. Start playback where the track starts

**Per card, small, every card.** Start at section 2 (after the intro) or at
the loudest 30 s window, with 1 to go back to the start. Saves a keypress and
an intro per card; no effect on the model.

Where: `player.gd` `show_track`, `pos = 0.0`; the sections come from the
waveform analysis already.

### 5. Say when you can stop

**Labels needed, by making it visible.** The queue line could say how much
of the library the model would already label with confidence: "model sure on
8,120 of 12,860". When that number stops moving per 100 labels, further
labelling is for the stats page, not for rekordbox, and the user can export
and go play. Today the only signal is the accuracy percentage, which does not
say how much is covered.

Where: `queue()` already predicts the pool; count `pg >= SURE`. One number on
`Queue`, and on the stats page over time.

## Not proposed

- **Fewer blind checks.** The 1-in-10 blind card is the accuracy stat; without
  it every number on the stats page is inflated by shown guesses.
- **Hiding the guess on unsure cards.** Enter-to-confirm is already the fast
  path; a kept guess weighs 0.5, which is the price of that speed.
- **A second queue mode.** That was the fast track; the point here is one
  queue that wastes less.

## Order

1. Proposal 1 (rest re-confirmed conflicts, conflicts by accuracy, bar at
   0.8): small, measurable at once, removes the most wasted cards.
2. Proposal 2 (runs of one mood): small, per-card speed every card.
3. Proposal 4 (start position): trivial.
4. Proposal 3 (coverage): medium; check it on the learning curve before
   keeping it (labels to reach the same accuracy, `curve` in `app.rs`).
5. Proposal 5 (coverage number): small, after 3 so the number is honest.

## Decided

- Conflicts on/off at one number: dealt every 5th only when the model is
  right ≥ 75% where it is sure, on at least `[guesses] measured` blind labels.
- A track asked as a conflict rests 60 days (labels +50% dropped: simpler).
- Runs of 5, by colour (8).
- Playback starts at the second section; 1 goes back.
- Coverage: spread n over the 2n least sure.

## Bulk label: the bulk export

The end of the loop. You label until "model 80% sure on N" flattens, then
write the guesses and go play; rekordbox corrections and plays come back
through sync, the sure set grows, you write again.

- **What**: tracks with no colour in rekordbox and no label here that the
  model is at least `sure` of, colour and stars, at most `limit` (100, picked in
  the dialog, the last pick kept) per run, surest first, added to the export list. Nothing is written to
  rekordbox until Export, which writes them with key, energy and cues like
  the queue's labels. Nothing with a colour is touched: your labels and old
  ones stay, disagreements remain the queue's job. (First built as a direct
  write of its own playlist into the xml; changed 2026-11-22, the export list
  did not show it.)
- **Guard**, Settings › Bulk label (`[guesses]`): past calibration, and
  `must_pass` blind tests (held-back labels where the model is ≥ `sure`)
  passed: `right` (0.7) of those taken, counted as at least `measured` (10).
  So 7 passed unlocks before the 10th test; past 10 the pass rate counts.
  Conflict cards use the same rule at 0.75. Not a hard 90% gate: the fast
  track showed a measured gate starves on real labels. The dialog shows the
  count and a checklist (calibration, blind tests passed · needs N).
- **Meter**: the queue line's "model 80% sure on N" (whole pool) and the
  dialog's count (blank in rekordbox), both from per-band histograms the
  queue returns, so the Settings bar moves them without a round trip.
- **Trust**: source `model`, card `guessed`, no round: not trained on,
  trust ring 1. They sit in `batch()` / `unsent` like a dropped track's
  guess, so Export writes them; after it, sync reads an untouched one as
  still a guess (only rounds are confirmed by being left alone) and a
  changed one as `corrected` (0.7).
- **Not done**: play tracking for guesses. Sync follows play counts only for
  the last export, so a played guess stays a guess. A `plays` column on
  `batch_tracks` and a sync over all out batches would fix that for every
  export; worth it once guesses are out in a real set.

## Done (2026-11-12)

In `core/src/app.rs` (`queue()`, `bulk_label()`), `ext`, and the app:

1. Conflicts: bar 0.8 (`SURE`); dealt only when `right_at(by_sure, SURE)` ≥
   `CONFLICT_RIGHT` (0.75) on ≥ `[guesses] measured` labels; a track asked as
   a conflict rests `CONFLICT_REST` (60 days, `store::recent_cards`).
2. Runs: the dealt unsure cards come in runs of `RUN` (5) of one guessed
   colour (`runs`), round-robin over colours, least sure first within one.
3. Coverage: the 2n least sure thinned to n with `spread` against the
   labelled tracks, then ordered as above; the rest follow least sure first.
4. Player: once the sections arrive, a track starts at its second section
   unless you already moved (`pos < 2 s`); 1 goes back.
5. `Score::by_sure` (right, asked per 5% band from 0.5; model cache key m6);
   the queue returns `sure_hist`, `blank_hist`, `right_hist`; the queue line
   shows "model 80% sure on N" and the Bulk label button (⌥G).
6. Bulk label (was Write guesses): `bulk_label(root, xml, cfg, threshold)`, the dialog in
   `main.gd` (count, accuracy, lock reason; Cancel / Write, ENTER writes),
   the Settings card, `[guesses]` defaults in `default_config.toml`.
