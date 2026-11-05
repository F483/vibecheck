# The ceiling: measuring label consistency

Date: 2026-09-13. This is the measurement the whole investigation was for.

---

## What was done

300 tracks were drawn at random from the **training** split (so nothing here
contaminates validation or test), their genre tags cleared, and the maintainer
relabelled them from scratch in Rekordbox without seeing the previous answers.
Those fresh labels were then compared against what the same tracks had carried
before.

The question: **how often does the labeller agree with their own past
judgement?** That number is a hard ceiling -- no model trained on those labels
can predict them better than they predict themselves.

## Result

```
                    self-agreement      model accuracy
colour  (8 values)       45.3%              56.0%
hue     (4 values)       65.7%              66.1%
tone    (2 values)       70.3%              80.7%
level   (3 values)       47.3%             ~71%
```

**The model reproduces the labelling more reliably than the labeller does.**
On every axis it matches or beats human self-agreement, substantially so for
colour, tone and level.

## What this means

For two weeks 56% was treated as a disappointing ceiling to break through. It
is not a ceiling we are stuck beneath -- **it is above the noise floor of the
target.** The model has learned the central tendency of a noisy judgement, and
a central tendency is more stable than any single instance of that judgement,
including the labeller's own.

Consequences:

1. **Further model work cannot help.** Six independent approaches converged at
   56-58% (README/accuracy.md), and we now know why: there is no more signal to
   extract. A larger encoder, more data, or cloud compute would be spending
   money to predict noise.
2. **Eight colours is finer than the judgement supports.** 45% self-agreement
   means the categories overlap in the labeller's own head, not merely in the
   audio.
3. **The coarser levels are where judgement is stable** -- 66% at hue, 70% at
   tone -- and the model is at or above both.

## It is not a vocabulary problem

The obvious alternative explanation is that the categories were renamed rather
than misapplied. Tested by finding the best possible 1:1 remapping of old
colours onto new ones (optimal assignment on the confusion matrix):

```
raw agreement                    45.3%
best 1:1 remapping               45.3%   (the mapping is the identity)
```

No renaming recovers anything. The largest single movement, Purple -> Pink
(46 tracks), is not a rename: Purple also stayed Purple 26 times. It split.
The remaining disagreements scatter -- Red->Blue 18, Red->Green 13,
Blue->Red 9.

## Part of it is deliberate drift

The relabelling was not a pure repeat measurement. The distribution moved:

```
              old  ->  new
Purple         84  ->   31   (-53)
Pink           24  ->   69   (+45)
Green          19  ->   35   (+16)
Yellow         23  ->   37   (+14)
Red            65  ->   42   (-23)

Dark / Light  202/98  -> 149/151
Level A/B/C   16/96/188 -> 80/132/88
```

Ratings became much more generous (A: 16 -> 80, a five-fold increase, which the
maintainer had said in advance they intended) and the balance shifted toward
Light. So some of the 45% is genuine taste drift rather than inconsistency, and
the true repeat-measurement agreement is somewhat higher than 45%.

This does not change the conclusion. Hue agreement (65.7%) is unaffected by the
rating change and still sits level with the model (66.1%). And drift cuts the
other way for the training corpus: **8,619 labels accumulated across a shifting
scale are a mixed-vintage target**, which is part of why more of them stopped
helping.

## Confirmed: higher-rated labels are more reliable

The maintainer predicted that better-rated tracks carry better labels, having
played them more. Measured:

```
colour agreement by the track's ORIGINAL level
  A:  n= 16   62.5%
  B:  n= 96   56.2%
  C:  n=188   38.3%
```

Confirmed, and the gradient is steep. Note this is the opposite of what play
count suggested when tested directly -- within a level, play count predicted
nothing. The rating carries the information; the play count does not add to it.

## What to do about it

- **Stop trying to improve the model.** It is already past the point where the
  target can reward improvement.
- **Prefer the coarser levels.** Hue and tone are where the labelling is stable
  and where the model is strongest. Colour is better offered as a suggestion to
  correct than as an answer to trust.
- **Consider the model as a consistency tool, not just a labour saver.** It
  applies one stable judgement across the whole library, which is something the
  labeller demonstrably cannot do by hand across thousands of tracks and
  several years.
- **If a better model is ever wanted, it needs better labels, not better
  architecture** -- and "better" here means more self-consistent, which
  probably means fewer categories rather than more effort per track.
