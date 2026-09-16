# Landscape

Who else does this, what they actually do, and where this project sits.
Researched 2026-09-15. Vendor claims are marked as such -- most of what is
written about these tools is written by the people selling them.

The short version: **the idea is not novel and one competitor already ships it
locally.** What is unusual here is being open source, refusing to guess, and
measuring whether it works.

---

## Four families

Most tools fall into one of four groups. Only the fourth is competition.

| family | what it does | examples |
|---|---|---|
| **library management** | you tag, it organises and exports | [Lexicon](https://www.lexicondj.com/), [One Tagger](https://onetagger.github.io/), [beets](https://beets.io/) |
| **pretrained auto-tagging** | someone else's taxonomy, applied to your audio | [Essentia](https://essentia.upf.edu/), [Cyanite](https://cyanite.ai/), [Djoid](https://www.djoid.io/), [TrackTag](https://tracktag.me/) |
| **audio similarity** | "what sounds like this?", no labels at all | [bliss](https://github.com/Polochon-street/bliss-rs), [Musly](https://www.musly.org/), [music-similarity](https://github.com/CDrummond/music-similarity) |
| **personal-taste learning** | learns *your* vocabulary from *your* corrections | **[Vibes](https://vibesdj.io/), [DaJent](https://www.dajent.co/), this** |

The first three are not competitors in any meaningful sense -- they answer
different questions. The fourth is.

---

## The actual competition

### [Vibes](https://vibesdj.io/) — closed source, commercial

The closest thing that exists. Local library manager for macOS and Windows:
tags by energy, mood and genre, exports to Rekordbox, Serato, Traktor and
Engine DJ. Their own description of the tagging is
*"auto-tag suggests vibes trained on your picks, which you can select or
confirm with keyboard shortcuts"* -- which is this project's loop, with a
real interface on top.

Notably, and unusually for a commercial tool: **library, analysis and tags live
locally, no account, no cloud dependency.** One-time purchase, 14-day trial.

| | [Vibes](https://vibesdj.io/) | this |
|---|---|---|
| learns your labels | yes (vendor claim) | yes, measured |
| runs locally | yes | yes |
| open source | no | yes |
| interface | polished desktop app | a CLI |
| BPM, key, cues | yes | no |
| exports to | rekordbox, serato, traktor, engine | rekordbox |
| says nothing when unsure | unknown | yes, by cost policy |
| publishes its accuracy | no | yes |

Honest reading: if you want this working today with a good interface and are
happy to pay and to trust a black box, Vibes is the answer and this project is
not.

### [DaJent Curate](https://www.dajent.co/) — closed source, commercial

*"Auto-analyzes every track for BPM, key, and energy, and suggests tags based
on your personal taste profile. You approve or correct them, training the
system on your style. Every correction sharpens your personal taste model."*
(vendor copy) -- again, the same loop, bundled into a broader set-prep product.
Whether analysis runs locally is not stated in their public material.

### Everyone else in the DJ space

- **[Lexicon](https://www.lexicondj.com/)**
  ([custom tags](https://www.lexicondj.com/manual/tags)) -- library management
  with fully custom tags and categories, and an energy field its analyser can
  fill on an absolute scale. Tagging is manual or rule-driven; nothing learns
  from you. Strong at organising, not at guessing.
- **[Djoid](https://www.djoid.io/)**
  ([tagging guide](https://www.djoid.io/articles/how-to-tag-your-music-library-as-a-dj-a-comprehensive-guide))
  -- pretrained labels (energy, mood, danceability, emotion) plus a
  graph-based set builder. Fixed taxonomy, no personal model.
- **[Mixed In Key](https://mixedinkey.com/)** -- key, energy curve, cue points.
  Fixed analysis, no taxonomy at all. Relevant here for one reason: the
  in-place Rekordbox XML rewrite that makes a new playlist appear on refresh is
  its mechanism, and this project copies it.
- **[beaTunes](https://www.beatunes.com/)**,
  **[TrackTag](https://tracktag.me/)**,
  **[GreenGo](https://greengomusic.com/)** -- catalogue-scale tagging with
  fixed attributes.

---

## Open source, local

Nothing here learns a personal vocabulary. They are worth knowing anyway --
two of them solve problems this project has.

- **[One Tagger](https://onetagger.github.io/)** -- Rust/Vue, cross-platform,
  built for DJs. Its Quick Tag mode is a fast keyboard-driven tagger writing
  to custom fields. **The best labelling interface in this space**, and
  directly useful: hand-labelling speed is this project's bottleneck, and the
  frontend work has the same problem to solve.
- **[Essentia](https://essentia.upf.edu/)** (MTG-UPF) -- the serious open
  audio-analysis library, with pretrained models for genre (400 Discogs
  classes), mood, danceability, instrumentation. Architecturally our nearest
  twin: frozen embeddings plus a classifier head. The difference is whose
  labels the head was trained on.
- **[beets-xtractor](https://github.com/adamjakab/BeetsPluginXtractor)** /
  **[beets-autogenre](https://github.com/mgoltzsche/beets-autogenre)** /
  **[Essentia-to-Metadata](https://github.com/WB2024/Essentia-to-Metadata)** --
  Essentia wrapped for library tools, writing its taxonomy into tags.
- **[music_classifier](https://github.com/Jacob-Haynes/music_classifier)** --
  Essentia + TensorFlow, genre/energy/danceability into ID3, aimed at DJs.
- **[winson0123/Auto-Tag](https://github.com/winson0123/Auto-Tag)** -- tags a
  DJ library using the SoundCloud API; metadata lookup rather than audio.
- **[bliss](https://github.com/Polochon-street/bliss-rs)**,
  **[Musly](https://www.musly.org/)**,
  **[music-similarity](https://github.com/CDrummond/music-similarity)** --
  similarity for automatic playlists. No labels. Useful speed datapoint from
  music-similarity: 25k tracks takes bliss ~2h, Musly ~1h, Essentia ~20h.
- **[Mixxx](https://mixxx.org/)** ([source](https://github.com/mixxxdj/mixxx))
  -- the FOSS DJ application. BPM and key detection, coloured cues. No learned
  tagging, and no plugin surface for it.
- **[LabelBuddy](https://arxiv.org/html/2603.04293v1)** -- open source audio
  annotation with AI-assisted pre-annotation and a pluggable inference
  backend. Not a DJ tool, but conceptually the same loop: the model
  pre-annotates, the human corrects, the model improves.

## Cloud APIs

**[Cyanite](https://cyanite.ai/)**
([auto-tagging guide](https://cyanite.ai/blog/ai-auto-tagging-music-catalogs/))
is the reference point: 13 moods, 131
advanced moods, 23 genres, 58 sub-genres, 5,000+ free genre tags, from audio
rather than user behaviour. Their own guidance is the interesting part --
they say context-specific taxonomies *cannot* be derived from audio alone and
need human input. That is precisely the gap this project aims at, stated by
someone selling the alternative.

---

## Does the approach hold up?

Yes, and there is published work saying so.
[Music auto-tagging in the long tail: a few-shot approach](https://arxiv.org/abs/2409.07730)
finds that a **linear probe over pretrained embeddings reaches near
state-of-the-art with as few as ~20 examples per tag**. That is this
architecture exactly -- frozen encoder, small supervised head -- and it is the
reason 300 hand-labelled tracks is a plausible starting point rather than
wishful thinking.

The same literature is where the encoder choice comes from: CLAP-Music&Speech
is competitive with much larger models on multi-label few-shot tagging.

---

## Where this is genuinely different

Not "it learns your taste" -- two products already claim that. The differences
that survive scrutiny:

1. **It is open source and local.** Vibes is local but closed; the cloud
   tagging APIs are neither. Nobody else lets you read why a track got the
   colour it got.
2. **It refuses to guess.** Every axis asserts only when
   `(1 - p) × (cost + misleading_cost) < cost` -- when being right is likely
   enough to be worth the risk. There are no confidence thresholds to tune, and
   a prediction that only got as far as the hue says so instead of inventing a
   colour. No competitor documents anything equivalent; the marketing language
   throughout this space is about coverage, never about knowing when to stay
   quiet.
3. **It measures itself, publicly.** Held-out accuracy, decisions-left per
   track, coverage per axis, all recorded per round in the database and
   published in the README. No competitor publishes an accuracy figure at all.
4. **It knows what it cannot do.** The ceiling work found the maintainer
   relabels their own tracks with 45.3% consistency, and the model reaches
   56%. A product would not advertise that.

## Where it loses, today

- No interface. Vibes and Lexicon have had years of interface work.
- One axis of the problem. No BPM, no key, no cue points, no set building.
- Rekordbox only.
- Slow: CLAP is far heavier per track than bliss or Musly, and nothing here is
  optimised for a first-run scan of a large library.
- Needs hundreds of hand labels before it earns its keep, which is a hard
  thing to ask of anyone who is not already invested.

---

## Sources

- [Vibes](https://vibesdj.io/) · [integrations](https://vibesdj.io/integrations) · [their auto-tagging comparison](https://vibesdj.io/best/auto-tagging-software)
- [DaJent](https://www.dajent.co/) · [library organisation guide](https://www.dajent.co/blog/how-to-organize-dj-music-library)
- [Lexicon custom tags](https://www.lexicondj.com/manual/tags) · [Lexicon vs Djoid](https://www.lexicondj.com/lexicon-vs-djoid)
- [Djoid tagging guide](https://www.djoid.io/articles/how-to-tag-your-music-library-as-a-dj-a-comprehensive-guide)
- [Mixed In Key 11](https://mixedinkey.com/learn-more/)
- [Cyanite auto-tagging guide](https://cyanite.ai/blog/ai-auto-tagging-music-catalogs/) · [music tagging](https://cyanite.ai/music-tagging/)
- [One Tagger](https://onetagger.github.io/)
- [beets plugins](https://beets.readthedocs.io/en/stable/plugins/) · [beets-xtractor](https://github.com/adamjakab/BeetsPluginXtractor) · [beets-autogenre](https://github.com/mgoltzsche/beets-autogenre)
- [Essentia-to-Metadata](https://github.com/WB2024/Essentia-to-Metadata) · [music_classifier](https://github.com/Jacob-Haynes/music_classifier) · [Auto-Tag](https://github.com/winson0123/Auto-Tag)
- [music-similarity](https://github.com/CDrummond/music-similarity) · [Musly](https://www.musly.org/)
- [Mixxx](https://mixxx.org/features/)
- [LabelBuddy](https://arxiv.org/html/2603.04293v1)
- [Music auto-tagging in the long tail: a few-shot approach](https://arxiv.org/abs/2409.07730)
