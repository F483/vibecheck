# User flows

*2026-11-06. Proposal.*

Labels should count by what the user actually did, not by silence. Today a
track left unchanged after a round counts as confirmed. That works for someone
who reviews every track carefully, but not for a new user who skims. This doc
proposes trust levels plus one guided flow, so the app works well for both.

## The problem

The current loop treats "unchanged" as "confirmed". A user who doesn't review
a playlist closely confirms the model's own guesses. The model then trains on
them and repeats them, and each round drifts further from what the user really
thinks.

This is what happened at the very start: labels were inconsistent, results
were poor, and the only way out was to start over. It shows up in both of
today's use cases:

- **Fresh start:** generate a playlist, review it in rekordbox, sync. Every
  track not corrected becomes a label at full weight.
- **Established library:** the same loop, plus an initial sync of the colours
  and ratings already in rekordbox. Those were often set over years, with no
  consistent rule, and they all count at full weight from day one.

Neither case can tell "I listened and agree" from "I didn't look".

## Trust levels

Every label gets a level from what the user did with it. Training weights each
label by its level. The user never has to choose a mode or understand the
rule; they only see a small indicator per track.

| level | what the user did | training weight | notes |
|---|---|---|---|
| 4: set by hand | labelled it in the app, or changed our colour or rating in rekordbox | 1.0 | the strongest signal there is |
| 3: played and kept | play count went up since we wrote the label, and it is unchanged | about 0.7 | they heard it, likely in a set, and let it stand; editing its cues counts too |
| 2: existing label | it was there before vibecheck | about 0.3 until checked | a library's history can be wildly inconsistent |
| 1: untouched | we wrote it and nothing happened since | 0 | never trained on; it stays pending |

The weights are a starting point to tune, not measured values.

Age alone doesn't lower trust: an old label can be perfectly right. What
matters is age together with disagreement. An old label the current model
strongly disagrees with goes to the review queue, since the user's taste may
have changed or it was a mistake. An old label the model agrees with is left
alone.

Plays only show up after rekordbox has the play count. Plays on CDJs count
once rekordbox imports the USB history.

## The flow

![Silence never counts as a confirmation](img/user-flows/flow.png)

Fresh start and established library become one flow; the established library
only adds the consistency check.

1. **Setup.** Choose the music folder and the rekordbox xml. Scanning takes
   seconds. Vectors are computed in the background, starting with a random
   sample so calibration can begin within about 30 minutes.
2. **Consistency check** (only if labels exist). Import them all at level 2.
   Find near-identical tracks with different labels, and labels the model
   rejects. Show them as "which is right?" cards; every answer becomes level 4.
3. **Calibration.** About 30 tracks chosen to cover the sound of the
   collection, labelled in the app in about 5 minutes. This gives the model a
   clean base before the first round, which is what the fresh start lacked.
4. **Rounds.** Today's loop: write a playlist, play and correct in rekordbox,
   export, sync. Sync assigns levels instead of confirming. Untouched tracks
   stay pending, and the round stays open until they earn trust. "Confirm all
   untouched" is a deliberate button with a warning, not the default.
5. **Review inbox.** Fills up from every sync: conflicts, old labels the model
   now disagrees with, and the tracks it is least sure about. These teach the
   model the most per minute spent.

## What the UI needs

Five screens, plus one indicator that shows up everywhere:

| screen | what it does | why it matters |
|---|---|---|
| setup | choose the music folder and the rekordbox xml; shows vectors being computed in the background | the first two minutes decide whether people stay |
| card review | one track at a time: play a 10 s snippet, pick colour and stars on a grid, next; fully keyboard driven | used for calibration, conflicts and the review queue; the most eye-catching part to animate |
| sync summary | after each sync: e.g. "12 corrected · 30 played and kept · 58 untouched, still pending" | makes the trust rule visible without explaining it |
| review inbox | conflicts, old labels the model now disagrees with, tracks it is least sure about; a count badge | gives a reason to come back between rounds |
| collection | every track with its labels and a trust ring coloured by level | shows progress and where trust is thin |

The trust ring is the one new idea the user has to learn, so it appears the
same way on every screen.

Settings keep today's workflow for experienced users: "treat untouched tracks
as confirmed", off by default.

## What has to change in the core

All of this belongs in the Rust core, so the command line and the app behave
the same.

| change | where | size |
|---|---|---|
| store each label's level and where it came from; snapshot play count and cues per track when a round is written | `store.rs`, a schema v5 migration | small |
| read `PlayCount` from the rekordbox xml | `rekordbox.rs` | small |
| sync assigns levels instead of confirming; untouched tracks stay pending and the round stays open | `commands.rs` | medium |
| weighted training: each label's weight in the classifier's loss | `predict.rs` | small |
| consistency check: near-identical vectors with different labels, and existing labels the model rejects under cross-validation | new | medium |
| selection: mix in the tracks the model is least sure about, not only a random pick | `commands.rs` | small |
| calibration pick: about 30 tracks spread across the sound of the collection | new, uses the vectors | small |
| vectors computed in the background, most useful tracks first | `embed.rs` | medium |

The calibration pick needs vectors, and a whole library takes about 20 hours
at 5 s per track. So the app starts with a random sample of a few hundred
tracks (about 30 minutes) and calibrates from those while the rest continue in
the background.

Schema v5 would no longer match the Python prototype, which reads the same
`labels.db`. Either the prototype learns v5, or from this point on a
collection uses one or the other.

## Open questions

- [ ] Untouched labels: weight 0, or a small weight such as 0.1?
- [ ] "Played and kept": does one play count, or also a minimum time since the round?
- [ ] Calibration size: 30 tracks, or fewer?
- [ ] Is the snippet player in the app part of the first version, or is rekordbox the only place to listen at first?
- [ ] Existing labels that pass the consistency check: do they move up to level 3, or stay at 2?
- [ ] Prototype compatibility after schema v5: teach the prototype v5, or freeze it?
- [ ] Your own collection: re-level its current labels from the round history, or start everyone at level 4?
