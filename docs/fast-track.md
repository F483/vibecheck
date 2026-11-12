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
- **Mix** by queue position: every 5th place (3rd, 8th, …) is a conflict;
  every 7th place (5th, 12th, …) is an old label unless a conflict holds it;
  everything else is unsure. Over 35 tracks: 7 conflicts (20%), 4 old labels
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

## Open questions

- [x] Unlock: measured confident accuracy (Wilson lower bound ≥ 90%).
- [x] Weight of a fast-track Confirm: trust 0.5, × rating.
- [x] Blind checks inside the fast track: yes, about 1 in 10–20.
- [x] Group by guessed colour: yes, the library's filters applied to the guess.
- [x] Rating multiplier: universal, corrections included.
- [x] Blind-card rate: fixed 1 in 10 first; gauge-driven later, with data.
- [x] Settings slider retitled "Calibration ends after": past it the queue switches
      over (about 69% unsure, 20% conflicts, 11% old labels), nothing is mixed in.
