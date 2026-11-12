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
| 4 played | an export of ours holding the label was played since (sync's fate) | 1.0 |
| 3 confirmed | labelled blind here, corrected here (`colour_id != shown`), or changed in rekordbox | 0.7 |
| 3 kept | the model's guess was on screen and you kept it (`colour_id = shown`) | 0.5 |
| 2 existing | there before vibecheck, never confirmed | 0.3 |
| 1 guessed | the model wrote it and nothing happened since | 0.2, not trained on |

The model's own guesses are not trained on: at 14,000 of them even 0.2 would
outvote 1,300 labels. The weight stays in config for when that is revisited.

The library's trust ring and the stats bar use the same `trust()`, so
"played" means an export of ours was played since, not any play count.

## Rating

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
