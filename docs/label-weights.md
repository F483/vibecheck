# Label weights: trust × rating

*2026-11-12. In core; what remains of the fast track plan (see the end).*

Training weights every label by how it came to be and how much the user
cares about the track: `weight = trust[level] × rating[stars]`, one line.
The numbers are config (`[trust]`, `[rating]` in
`core/src/default_config.toml`), to tune without a rebuild; `trust()`,
`rating_factor()` in `core/src/app.rs`; `Lr::fit_weighted` carries a per-row
weight through the logistic regression.

## What a label records (schema v8)

`labels` gained three columns, so trust can be derived later from what
actually happened rather than stored per level:

- `shown`: the colour on screen when the label was made (palette id), NULL
  when asked blind;
- `shown_stars`: the stars on screen then;
- `card`: which card asked (calibration, unsure, conflict, old label, re-ask,
  blind, relisten, dropped, library); for labels read from rekordbox
  `existing` (there before vibecheck), `corrected` (changed in rekordbox),
  `confirmed` (left as exported). Older rekordbox labels were backfilled from
  the sync history rows.

The player remembers the guess it opened with; `decide()` saves it with the
label. Sync writes existing / corrected / confirmed.

## Trust

Derived at training time (`trust()`):

| trust | from | weight |
|---|---|---|
| 4 played | played with the label on it: a sync saw the play count rise while rekordbox had it (`plays`) | 1.0 |
| 3 confirmed | labelled blind here, corrected here (`colour_id != shown`), or changed in rekordbox | 0.7 |
| 3 kept | the model's guess was on screen and you kept it (`colour_id = shown`) | 0.5 |
| 2 existing | there before vibecheck, never confirmed | 0.3 |
| 1 guessed | the model wrote it and nothing happened since | 0.2, not trained on |

The model's own guesses are not trained on: at 14,000 of them even 0.2 would
outvote 1,300 labels. The weight stays in config for when that is revisited.

The library's trust ring and the stats bar use the same `trust()`.

## Plays (schema v10)

Every sync compares each track's rekordbox play count with the one last seen
(`play_counts`: the last sync, or the export that wrote the track, since its
label changes there). A rise is a play (`plays`), with the label the track
had here, when rekordbox had that same label; NULL when it had another
(labelled here since, not exported yet). So a play counts whichever export
the label went out in, or none:

- a label of yours, from any export: played;
- a guess of ours, played: becomes yours (`card` `played`), played;
- rekordbox's own colour on a track with no label here: taken (`existing`),
  played.

Only a rise counts, not a play count from before vibecheck: an old play may
have been with another colour. Until 2026-11-26 a sync compared only the
last export's tracks, so a play of anything exported earlier was lost (34
tracks of one set, 10 counted); backfilled from the xml's `.bak` before that
set.

## Self-healing: low trust is changed, high trust is asked (2026-11-26)

The same rule for colours and cues, as the model improves or things change
(the user: "I want everything to be aligned and self healing over time"):

| trust | the model now disagrees |
|---|---|
| low: none of yours, existing (before vibecheck), guessed (ours left alone) | **Bulk label changes it**: colour and stars where the model is `[guesses] sure` of another colour; cues where the cue model would set at most a quarter of them and is sure of its own |
| high: kept, confirmed, played | **asked again**: a conflict card (colour), a cue conflict card (cues), not within 60 days of the last |

So a colour conflict is never about a label from before vibecheck any more
(Bulk label's), and Bulk label never touches a label or cues of yours. Bulk
label leaves alone what waits for Export. An export never refreshes cues of
ours left as written by itself, as it never re-guesses a colour: only Bulk
label does, where the model disagrees. `Low` in `core/src/app.rs`,
`cue_train::low`.

## Recency (2026-11-26)

Newer counts more, so a change of taste shows once you have acted on it:
`weight × (floor + (1 − floor) × ½^(age / half-life))`, `[recency]` in the
config (Settings › Weighting: off, 6 months … 5 years), half-life 365 days,
floor 0.5: a year old counts 0.75, ten years
0.5. Age runs from when you made the label or last played it (cues: last
set, edited or played). Labels from before vibecheck have no age to go by:
`[trust]`'s 0.3 already says they are old, so recency leaves them at 1.
The same for cues (`docs/cues.md`).

## Rating

Settings › Weighting › Weigh by stars (`[rating] use`) turns this off: every
track ×1, for whoever uses stars for something else.

The user is sloppier with tracks they care less about, so the whole label act
on a low-rated track (colour and stars both) counts less: `0.5 + 0.1 ×
stars`, mild on purpose, since low-rated tracks often sound different
(filler, tools) and the model still needs examples of them. No stars is not
unknown: in rekordbox 0★ is unrated, so a colour without stars is a
half-done label, the sloppiest signal, and sits at the bottom. In the app
Confirm always carries a rating, so 0★ labels are mostly existing ones from
rekordbox.

| rating | × |
|---|---|
| 5★ | 1.0 |
| 4★ | 0.9 |
| 3★ | 0.8 |
| 2★ | 0.7 |
| 1★ | 0.6 |
| none (0★) | 0.5 |

**Universal.** The multiplier applies to every level, corrections included.
Trust says how deliberate the act was, rating how much the user cares about
the track; a careless flip of a 1★ colour is still careless. Within a band
the order that matters holds (1★: correction 0.42 > kept guess 0.3 >
existing 0.18); the one cross-band case, a 1★ correction (0.42) under a 5★
kept guess (0.5), is fine: 5★ tracks are the ones the user knows best.

**Tune from data.** Re-ask consistency per rating band shows whether the
user really is sloppier on 1–2★. If they agree with themselves as often on
2★ as on 5★, flatten the multiplier; if corrections turn out as consistent on
1★ as on 5★, flatten it for level ≥ 3 only.

## Worked examples

| label | trust | rating | weight |
|---|---|---|---|
| played out after export, 5★ | 1.0 | 1.0 | 1.00 |
| played, 2★ | 1.0 | 0.7 | 0.70 |
| calibration card (blind), 4★ | 0.7 | 0.9 | 0.63 |
| conflict / old label / blind check card, 3★ | 0.7 | 0.8 | 0.56 |
| unsure card, guess Yellow, you picked Red, 5★ | 0.7 | 1.0 | 0.70 |
| unsure card, guess Yellow, you kept Yellow, 5★ | 0.5 | 1.0 | 0.50 |
| kept guess, 1★ | 0.5 | 0.6 | 0.30 |
| kept guess, 0★ | 0.5 | 0.5 | 0.25 |
| changed the colour in rekordbox, 3★ | 0.7 | 0.8 | 0.56 |
| old rekordbox label, untouched, 5★ | 0.3 | 1.0 | 0.30 |
| old rekordbox label, untouched, 0★ | 0.3 | 0.5 | 0.15 |

Edge cases, as the code reads them (`trust()` in `app.rs`):

- **Kept** needs the guess actually on screen (`shown` not NULL): calibration
  and every blind card are confirmed (0.7) whatever you pick. Changing only
  the stars of a shown guess is still kept: the colour matched.
- **Played** wins over everything: a kept guess that was then played in a
  set is 1.0. It means a sync saw a play with the label on it (see Plays),
  not the track's rekordbox play count.
- **Rekordbox labels**: `corrected` and `confirmed` (left as exported, read
  back) are 0.7; `existing` and older rows without a card are 0.3.
- **Pre-v8 labels** have `shown` NULL, so they all count as confirmed:
  0.7 × rating. On the reference collection that is 1,316 of 1,333 labels,
  with one played.
- **Stars**: `shown_stars` is recorded but does not enter the weight; only
  the label's own stars do, through the rating factor.
- The weight multiplies the row's loss and gradient in `Lr::fit_weighted`;
  standardisation is weighted too, so a 0-weight row changes nothing.

Things to check with data: whether re-ask consistency really drops on 1–2★
(else flatten the rating factor), and whether kept guesses disagree with a
later blind answer more than blind labels do (else raise `kept` toward 0.7).

## Accuracy from blind labels only

A label made with the guess on screen matches it by construction, so the
held-out score (`fit()`) uses only rows with `shown IS NULL`. Training uses
everything, weighted. To keep fresh blind labels flowing into it where the
model is sure, the training queue deals a sure track (≥ 0.8, `SURE`) blind
every 10th card, tagged **blind** and explained in the tag popup
(`train_slot`). Sure-and-blind labels otherwise only came by accident (7 in
1,314 on the reference collection).

## Fast track: considered and cut (2026-11-12)

The idea: once the model's sure guesses are measurably right, a toggle
switches the queue to the surest tracks, shown with the guess, to confirm or
fix quickly, narrowed by the library's colour / energy / rating / BPM chips
applied to the guess. Built behind a gate (30 blind held-out sure labels,
Wilson midpoint ≥ 0.9), then cut the same day, because on the reference
collection the gate was out of reach:

- the gate only sees blind labels in the 10% held-out bucket where the model
  is sure: 1 sample per ~100 training cards, so ~2,300 cards to reach 30;
- it needs 29 of 30 right; the model stood at 65% on 7, and the user agrees
  with their own earlier label 85% of the time (766 of 897 repeats), which
  caps what any model can score against them blind.

Others' libraries are unlikely to be cleaner. What stayed: everything above.
The risk the fast track would have added, anchoring on a shown guess, is why
a kept guess weighs 0.5 and why accuracy is blind-only; both hold for the
training queue's unsure cards too, which show the guess.
