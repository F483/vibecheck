# Plan B — cloud inference with a local frontend

Contingency. Not the current plan, and not started. This describes what to
build **if local models cannot reach useful accuracy**, and — just as
importantly — the case where Plan B should *not* be attempted either.

---

## 1. When this triggers

Two measurements decide it, and they are not interchangeable.

**The ceiling check** (blind relabel, `scripts/relabel_status.py`) measures how
often the user agrees with their own past labels. That is the hard ceiling: no
model trained on those labels can beat it.

|  | local models plateau ~56% | local models reach ~70%+ |
|---|---|---|
| **self-agreement ~65%** | **Do nothing.** 56% is already near the ceiling. Cloud cannot add consistency to the target. Ship the cascade. | not possible |
| **self-agreement ~85%+** | **Plan B is justified.** There is real headroom and local representations cannot reach it. | Stay local. |

The trap to avoid: spending money because results are disappointing, without
first knowing whether the disappointment is fixable. A larger model cannot
learn a label the labeller does not reproduce.

**Concrete trigger:** self-agreement >= 80%, *and* local colour accuracy still
below ~65% after the per-window and fine-tuning experiments.

## 2. What changes, and what does not

The local-first constraint in the main design (README §3 — "runs entirely on a
laptop, offline, no upload of audio") is **deliberately reversed** under Plan B.
That was decided when local looked sufficient; this document is what happens
when it is not. Everything else survives.

**Changes**
- Audio excerpts leave the machine to be embedded.
- Ongoing cost and a network dependency, where there were none.
- The shipping story gets *simpler*: no ONNX export, no bundled 400 MB model,
  no per-platform inference. The app becomes a thin client.

**Stays the same**
- Labels live in the user's tags; playlists remain work lists.
- `.vibecheck/` in the collection root; the same cache key scheme.
- The cascade (colour -> hue -> tone, finest confident level).
- **The classifier is still trained locally, on the user's machine.** Only
  embeddings come back from the cloud.

## 3. Architecture

```
local                                        cloud
-----                                        -----
scan, hash, decode 10 s windows
encode windows as Opus (~2-5 MB/track)  -->  GPU inference endpoint
                                             big audio encoder, frozen
cache embeddings in cache.db            <--  returns vectors only
train classifier on user's labels
predict, cascade, write tags
```

**The important boundary: labels never leave the machine.** The cloud sees
audio excerpts and returns numbers; it never learns what the user calls them.
That preserves the property the whole project rests on — the model is *this
person's* taste — and keeps the privacy exposure to "excerpts of commercially
released music", not "my taste profile".

## 4. Which models, and why

The class untested locally is the audio-LLM encoders, which are 20-70x larger
than anything an M1 can run:

| model | evidence |
|---|---|
| Qwen2-Audio-7B | 1st on audio-only tasks in MAEB (53 models); 89.99% on GTZAN genre probes vs MERT's 78.96% |
| LCO-Embedding-Omni-7B | leads MAEB overall (52.2% average) |
| Audio Flamingo / Whisper-large encoders | 91.37% GTZAN probe -- captioning-trained encoders do unusually well on genre |

Expected gain if the representation is the binding constraint: **+3 to +10 on
colour**. Unknown if it is not.

## 5. Cost

**Rented GPU, run our own script** (Lambda, RunPod, Vast) — cheapest, most
control, recommended for a single user:

```
subset trial (2,681 tracks, 7B encoder)     ~5 GPU-h    $5-10
full labelled set (8,619 tracks)            ~17 GPU-h   $20-50
whole library (14,194 tracks)               ~28 GPU-h   $35-80
ongoing: new music only                     pennies per track
upload: 10 s Opus windows                   ~5 GB subset / ~20 GB full
```

**Managed endpoint** (Modal, Replicate, HF Inference Endpoints) — simpler, no
machine to babysit, roughly 2-4x the unit cost. Worth it if this ever serves
more than one person.

**Commercial audio-tagging APIs** (Cyanite and similar) — wrong fit. They sell
their own taxonomy and bill per track as a subscription, which is precisely
what this project exists not to use.

## 6. Risks

- **Reproducibility.** A hosted model can change under you, silently
  invalidating every cached embedding. The cache key must include the endpoint
  and its model version, exactly as it already includes backend and version.
  Prefer a pinned self-hosted image over a "latest" endpoint.
- **Ongoing cost** on a growing collection, versus a one-off local batch.
- **Service lifetime.** A tool that stops working when a vendor sunsets an
  endpoint is worse than a slower tool that always works. Keep the local
  backend functional as a fallback, even if less accurate.
- **Upload time** — 20 GB is hours on a domestic connection, though only once.
- **Privacy.** Excerpts of purchased music leave the machine. Modest, but it
  should be a stated choice rather than a silent default.

## 7. Build order, if triggered

1. Subset trial first: rent a GPU, embed 2,681 tracks with one 7B encoder,
   score against the same split. **$10 and an afternoon.** If it does not beat
   local by a clear margin, stop -- the representation was never the problem.
2. If it wins: full labelled set, re-tune, re-measure on the sealed test.
3. Only then build the client -- window extraction, upload, embedding cache,
   retry and resume. The tool itself is a few hundred lines; the risk was never
   in the client.
4. Keep the local CLAP backend selectable, so the tool degrades to a working
   offline mode rather than breaking.

## 8. What this does not fix

Label noise, an 8-way distinction finer than the labeller reliably reproduces,
and the level axis. If the ceiling check says the target is only ~65%
determined, none of the above helps, and the right response is a coarser
vocabulary rather than a bigger model.
