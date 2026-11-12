# Fast track

*2026-11-12. Idea, to plan.*

The vibe queue today is a training queue: it asks what teaches the model
most, which is also the slowest for the user to get through. Once the model
is good enough, a **fast track** would do the opposite: offer the tracks the
model is most sure about, so the user mostly listens and confirms, and gets
the library into rekordbox quickly.

## How the queue works today

`queue()` in `core/src/app.rs`. Nothing is random.

- **Below the threshold** (default 100 of your labels, slider 100–300), and
  also while fewer than 20 labelled tracks are analysed: only calibration
  tracks, blind. Each pick is the track that sounds most different from
  everything labelled and already picked (`spread`).
- **From the threshold on**, three lists:
  - **unsure**: least sure first (hue × tone probability), shown with the
    model's guess;
  - **conflict**: your label, while the model is at least 50% sure of another
    colour; most sure first, blind;
  - **old label**: a colour from before vibecheck you never confirmed, the
    model at least 50% sure of another; most sure first, blind.
- **Mix** by queue position (`train_slot`): every 10th place a sure track
  asked blind (added 2026-11-12, see below); every 5th place (3rd, 8th, …) a
  conflict; every 7th place (5th, 12th, …) an old label unless a conflict
  holds it; everything else unsure. Over 35 tracks: 7 conflicts (20%), 4 old labels
  (about 11%), 24 unsure (about 69%). When a list runs out, its places go to
  the next one.
- The settings slider is titled "Calibration ends after" for this reason:
  past it nothing is mixed in, the queue switches over.

## Why it works

- Confident guesses are mostly right: a short listen and Confirm.
- They teach the model almost nothing, which is why the training queue never
  shows them. So the fast track costs the model little and gets the library
  done faster: a second phase, from teaching the model to checking it.

## Risks, and what was decided

1. **Anchoring.** Seeing the guess before deciding pulls the judgement
   toward it: you accept a colour you'd not have picked blind, and the more
   often the guess is right, the less you listen. `user-flows.md` (research
   section) cites studies finding exactly this, without people getting
   faster; hence "ask blind first". The fast track shows the guess on
   purpose, so expect rubber-stamping, mostly on confident-but-wrong tracks.
   - Blind re-asks **detect** it, they don't prevent it: re-ask fast-track
     confirms blind now and then; the disagreement rate between confirm and
     re-ask is the anchoring gauge. If it climbs, the fast track slows itself
     down (more blind cards, or pause).
2. **The model learning its own guesses back.** A plain Confirm of a shown
   guess gets less than a blind label. A correction keeps full weight: a fix
   against a confident guess is the most valuable signal there is. See
   *Weights* below.
3. **Inflated accuracy.** Confirms made with the guess shown would push the
   measured accuracy up. Weighting doesn't help (it changes training, not the
   stat). Decided: **accuracy is measured from blind cards only**
   (calibration, conflicts, old labels, re-asks, and the blind cards mixed
   into the fast track). Fast-track confirms never enter it.
4. **Unlock by measured accuracy, not by count.** Agreed. Unlock once
   confident guesses are measurably right: lower end of the Wilson interval
   (`wilson()` in `app.rs`) on confident blind guesses ≥ 90%. A messy library
   unlocks later, a consistent one sooner; some may get it well before 500
   labels.
5. **Keep measuring.** Agreed. Mix in a blind card at a **fixed rate, 1 in
   10**, in the spirit of "re-ask old tracks now and then" (`user-flows.md`,
   rule 3), so drift still shows and point 3 keeps its data. Log the anchoring
   gauge from the start; let it drive the rate (fewer when low, more or a pause
   when high) only once data shows it actually moves. Start simple, tune from
   evidence, as with the weights.

## Weights: trust × rating

Trust weights are not in core yet; `user-flows.md` has the proposal. This
replaces it with a product of two factors.

**Trust.** The fast track adds one row.

| trust | value |
|---|---|
| 4 played | 1.0 |
| 3 confirmed blind, or corrected (also in the fast track) | 0.7 |
| 3 fast-track confirm (guess shown, kept) | 0.5 |
| 2 existing label | 0.3 |
| 1 guessed | 0.2 |

**Rating.** The user is sloppier with tracks they care less about, so the
whole label act on a low-rated track (colour and stars both) counts less.
`0.5 + 0.1 × stars`, mild on purpose: low-rated tracks often sound different
(filler, tools) and the model still needs examples of them. No stars is not
unknown: in rekordbox 0★ is unrated, so a colour without stars is a half-done
label, the sloppiest signal, and sits at the bottom. In the app Confirm always
carries a rating, so 0★ labels are mostly existing ones from rekordbox.

| rating | × |
|---|---|
| 5★ | 1.0 |
| 4★ | 0.9 |
| 3★ | 0.8 |
| 2★ | 0.7 |
| 1★ | 0.6 |
| none (0★) | 0.5 |

So a 1★ fast-track confirm lands at 0.3, the same as an untouched existing
label; a 0★ one at 0.25, just above a bare guess; a 5★ correction at 0.7;
played stays the ceiling however the label got there.

**Universal.** The multiplier applies to every level, corrections included:
one line, `weight = trust[level] × rating[stars]`. Trust says how deliberate
the act was, rating how much the user cares about the track; a careless flip
of a 1★ colour is still careless. The product keeps the order that matters
within a band (1★: correction 0.42 > fast-track confirm 0.3 > existing 0.18),
and the one cross-band case, a 1★ correction (0.42) under a 5★ confirm (0.5),
is fine: 5★ tracks are the ones the user knows best.

**Tune from data.** Re-ask consistency per rating band shows whether the user
really is sloppier on 1–2★. If they agree with themselves as often on 2★ as
on 5★, flatten the multiplier; if corrections turn out as consistent on 1★ as
on 5★, flatten it for level ≥ 3 only, with that evidence.

## Group and filter by guess

Agreed, and the biggest accelerator. The user's own flow already is: sort the
export list by colour / rating and go over it once or twice. Do that before
export: put the library's filter row (`filters.gd`: colour, energy, rating,
BPM chips) over the queue when the fast track is on, applied to the model's
**guess**, since these tracks have no label yet. "Guessed Aqua, guessed 4–5★"
keeps one mood, which makes deciding faster, and a track that doesn't fit its
group stands out (the outlier pass, `user-flows.md` rule 4).

## UI

- A toggle in front of QUEUE on the vibe screen switches the queue.
- The line next to it changes with it, e.g. "ordered by what teaches the model
  most" ↔ "Fast track: most sure first, confirm or fix".
- Locked: the toggle is dimmed, with a muted hint on what unlocks it.
- The filter row over the queue when the fast track is on.
- Card tags: "SURE 94%" in place of UNSURE / CONFLICT. The blind card gets its
  own tag, **blind**, like conflict and unsure, and like them it's clickable:
  the tag popup (`WHY` in `player.gd`) gets a row for it, e.g. "a check card:
  asked blind so your answer stays your own, and the model's accuracy stays
  honest" / "Pick its colour and stars as if there were no guess."

## Implementation

What the design above needs that the code doesn't have, and the proposed fix
for each. Checked against `core/src/store.rs` and `app.rs` on 2026-11-12.

### Gaps

1. **Two kinds of blind card, conflated above.** The 1-in-10 blind card serves
   two purposes that need different tracks:
   - *accuracy*: an unlabelled confident track asked blind, compared with the
     guess afterwards;
   - *anchoring gauge*: a track already fast-track-confirmed, re-asked blind
     later.
   Fix: two rates. Accuracy cards 1 in 10; gauge re-asks 1 in 20, drawn from
   fast-track confirms older than a week, tagged **blind** both.
2. **Labels don't record what was shown.** `labels` has `source`, `round_id`,
   `origin` (schema v7), not whether a guess was on screen or which card asked.
   Without that neither the weight, the gauge nor an honest accuracy can be
   computed later. Fix: **v8 migration**, two columns on `labels`:
   `shown` (colour id on screen, NULL = blind) and `card` (the tag:
   calibration, unsure, conflict, old label, re-ask, fast, blind). `decide()`
   gets both from the app, which has them on the queue item.
3. **Accuracy today would be inflated.** `model()` scores a held-out split of
   *all* user labels; fast-track confirms would land in the validation half
   and match by construction. Fix: validation uses only rows with
   `shown IS NULL`. Training may still use everything, weighted. This also
   means **no separate accuracy bookkeeping**: the held-out score restricted
   to blind labels is already the honest number, and the 1-in-10 accuracy
   cards are what keep fresh blind labels flowing into it.
4. **Trust levels don't exist in core.** Build them now, properly, as part of
   this. The data is nearly all there: every level is derivable at training
   time from what the store already holds plus v8's `shown`, so nothing new
   is stored per level:
   - 4 played: the label was exported and a later sync set
     `batch_tracks.fate = 'played'` (exists today);
   - 3 confirmed: `source = 'user'` with `shown IS NULL`, or shown and
     corrected (`colour_id != shown`); also `origin = 'xml'` logged as
     `what = 'corrected'` (changed in rekordbox);
   - 3 fast-track confirm: `source = 'user'`, `colour_id = shown`;
   - 2 existing: `origin = 'xml'` from a sync, never confirmed;
   - 1 guessed: `source = 'model'`.
   One function `trust(label, fate) -> f32` in `app.rs`; `weight = trust ×
   rating(stars)`. `predict::Lr::fit` is plain logistic regression with no
   sample weights; add a per-row weight to its gradient (one argument through
   `Model::train`). Weights are config (`[trust]`, `[rating]` in
   `default_config.toml`) so they can be tuned without a rebuild.
5. **"Confident" has no number.** Conflicts use ≥ 0.5. Fix: fast-track pool
   and unlock sample use **≥ 0.8** (hue × tone); unlock needs the Wilson lower
   bound ≥ 0.9 over **at least 30** blind held-out rows that were predicted at
   ≥ 0.8. Colour only; stars confidence is a filter, not a gate.
6. **Filtering 200 items app-side leaves a handful.** `queue()` returns the
   next 200. Fix: `queue()` takes a mode and the guess filter
   (colours, stars, energy, BPM) and filters the confident pool in core before
   cutting to 200, most sure first within the filter.
7. **Small.** Toggle persists as a setting (`fast_track`), shortcut ⌥F; the
   gauge (confirm vs re-ask disagreement) shows on the stats page next to
   "your consistency"; `user-flows.md`'s weight table points here so there's
   one source.

### Done

- **Steps 1–2 (2026-11-12).** Schema v8 (`shown`, `shown_stars`, `card`, with
  older rekordbox labels backfilled from the sync history rows); `decide`
  records them from the app (the player remembers the guess it opened with;
  `card` is the queue tag, or relisten / dropped / library); sync writes
  existing / corrected / confirmed. `trust()` + `rating_factor()` in `app.rs`,
  weights in config, `Lr::fit_weighted` and `Model::train_weighted`; the
  library's trust ring and the stats bar use the same `trust()`, so "played"
  now means an export of ours was played since (sync's fate), not any play
  count. Validation uses blind labels only. The model's own guesses are not
  trained on: at 14,000 of them even 0.2 would outvote 1,300 labels; the
  `guessed` weight stays in config for when that is revisited.

- **Steps 3–6 (2026-11-12).** `queue(.., fast, filter)`: when open, the
  sure pool (≥ 0.8) most sure first within the guess filter, `fast_slot`
  putting a blind sure card every 10th place and a kept guess older than a
  week every 20th (`store::kept_guesses_before`). `Score.sure` / `sure_n`:
  colour right where sure, on blind held-back labels; `fast_ready` = n ≥ 30
  and the **middle** of the 95% range ≥ 0.9 (the low end cannot reach 0.9 on
  30: Wilson gives 0.886 for 30/30, so the doc's "lower bound" was unbuildable
  as written; the curve reads the middle too). App: Fast track chip + ⌥F
  before QUEUE, dimmed with the gate's state while locked; guess-only
  `Filters` row over the cards; tags `SURE 94%` / `BLIND`, both explained in
  the tag popup; `fast_track` saved as an app setting. Stats: "Kept guesses,
  asked again blind" under Trust (`store::kept_then_blind`). Not done:
  gauge-driven blind rate (needs data).
- **The training queue checks itself.** Sure-and-blind labels only came by
  accident (7 in 1,314 labels on the reference collection: the training queue
  asks unsure tracks, calibration ends at 100), so the gate was unreachable.
  Now every 10th training card is a sure track asked blind (`BLIND`), the
  same rule as on the fast track; it costs a little teaching and buys an
  honest measure of the model where it is sure.

### Build order

1. v8 migration: `shown`, `card`; `decide()` records them; validation excludes
   `shown IS NOT NULL`.
2. Trust levels and weighted training: `trust()`, `rating()`, weights in
   config, per-row weight in `Lr::fit`. Stats page shows labels per level
   (`user-flows.md` already asks for that).
3. `queue()` mode `fast`: confident pool ≥ 0.8, guess filter, accuracy cards
   1 in 10, gauge re-asks 1 in 20, tags `fast` / `blind`.
4. App: toggle before QUEUE, queue line text, `blind` tag + `WHY` row,
   "SURE 94%" tag, filter row over the queue applied to the guess.
5. Unlock: Wilson lower bound ≥ 0.9, n ≥ 30, on blind confident held-out rows;
   locked toggle with hint.
6. Gauge on stats; gauge-driven blind rate last, with data.

## Open questions

- [x] Unlock: measured confident accuracy (Wilson lower bound ≥ 90%).
- [x] Weight of a fast-track Confirm: trust 0.5, × rating.
- [x] Blind checks inside the fast track: yes, about 1 in 10–20.
- [x] Group by guessed colour: yes, the library's filters applied to the guess.
- [x] Rating multiplier: universal, corrections included.
- [x] Blind-card rate: fixed 1 in 10 first; gauge-driven later, with data.
- [x] Settings slider retitled "Calibration ends after": past it the queue switches
      over (about 69% unsure, 20% conflicts, 11% old labels), nothing is mixed in.
- [x] Trust levels built in core now, weighted training with them (gap 4).
- [x] Confidence 0.8 and n ≥ 30 for the unlock (gap 5).
- [x] Gauge re-asks 1 in 20, from confirms older than a week (gap 1).
- [x] Gauge visible on stats (gap 7).
