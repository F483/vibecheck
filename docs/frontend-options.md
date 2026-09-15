# Frontend options

**Nothing here is decided.** No frontend has been chosen, no stack committed
to, and none of this constrains the backend. The work has not started, and
deliberately so — the backend is still being used and shaped. This document
exists so the thinking does not have to be redone when the question is
actually open.

Requirements, as stated: lightweight, fast, responsive, cross-platform, a
**stylised, memorable look** -- the reference points are Ableton devices and
guitar pedals, not standard desktop chrome -- and **statistics showing progress
over time** as the library and the labelled set grow. That is a design-led
application with a list and some charts inside it, rather than a list
application with a theme.

---

## The candidates

| | size | design freedom | notes |
|---|---|---|---|
| **Tauri** | 3–10 MB shell | full (HTML/CSS) | system webview, Rust core. Best fit. |
| Electron | ~120 MB | full (HTML/CSS) | same design story, fails "lightweight" |
| Flutter | ~40 MB | full, own idioms | one codebase, Dart, good custom visuals |
| Godot | ~50 MB + sidecar | full, awkward | game engine doing office work (see below) |
| Qt / native | small, fast | real work | custom visual design is expensive |
| SwiftUI | smallest | good | mac only, and cross-platform is open |

## What a device aesthetic changes

Textured panels, knobs, LEDs, bevels, glow: this is a real constraint, not
decoration, and it moves two of the options.

**It partly rehabilitates Godot.** Custom drawing, shaders, sprites and
animation are a game engine's home territory. The earlier objection -- that
Control nodes are weak for dense lists -- still holds, but the chrome around
the list is now a genuine part of the work, and Godot is strong there.

**It strengthens the case for web anyway.** There is a great deal of prior art
in web-based audio plugin interfaces; JUCE ships a webview option precisely
because CSS and SVG turn out to be very good at this. Gradients, inset shadows,
layered textures and transforms produce hardware-looking surfaces without
writing a renderer, and canvas or WebGL is there for anything genuinely
dynamic. Meanwhile the dull half stays free: scrolling several hundred tracks
with selection and keyboard navigation is trivial in HTML and fiddly in a game
engine, and so is the other dull half, charts -- a few hundred points is an
`<svg>` polyline, no library required.

**One real caveat, specific to a design-led app.** Tauri uses the *system*
webview -- WebKit on macOS, WebView2 on Windows, WebKitGTK on Linux. Font
rendering, subpixel behaviour and some CSS details differ between them. Usually
invisible; for a precisely crafted panel it can mean the bevels look right on
one platform and slightly wrong on another.

**Flutter draws every pixel itself**, so it renders identically everywhere.
If pixel-exact consistency across platforms matters more than web ergonomics,
that is the reason to choose it over Tauri -- and its ~40 MB is noise beside a
300 MB model.

## The palette is already decided

Worth noticing before any design work: **the label vocabulary is a visual
identity.** Eight named colours, grouped into four hue families and two tones,
each with the hex rekordbox uses -- and since the schema redesign they are a
table the interface can read rather than a convention it has to know
(`palette.py`, `colours`).

A device-panel design where the colour swatches *are* the interface -- and
where a hue or tone fallback shows as a coarser or dimmer form of the same
colour -- would be both distinctive and honest about what the model is actually
saying. The prediction levels are not an implementation detail to hide; they
are the app telling you how sure it is, and they have an obvious visual grammar.

That is the first thing to prototype in plain HTML, because it decides whether
the look works at all, and it costs an afternoon rather than a packaging
commitment.

## The strongest candidate today: Tauri

On the requirements as they stand — lightweight, cross-platform, a stylised
look, and charts. Worth revisiting whenever any of those change, and it has
not been chosen.

It uses the operating system's own webview instead of bundling a browser, so
HTML/CSS — the best tooling that exists for making something look how you want
— comes at a fraction of Electron's weight.

Were it taken, the end state would be:

```
Tauri shell     HTML/CSS/JS, fully custom, ~5 MB
Rust core       ort (ONNX Runtime) for inference
                symphonia for mp3 decode
                rusqlite, plus ~50 lines of logistic regression
model file      ~300 MB fp16, ~150 MB int8
```

One binary, no Python, no second runtime. Everything in the current backend
maps across: the logic is small, and the parts that are not -- decode,
inference, storage -- all have solid Rust equivalents. That is an argument that
the option is *open*, not that it has been taken.

## If it went that way: three shippable steps

1. **Local web UI served by the existing Python.** Days of work, no packaging.
   This is where the interface gets designed, because nobody yet knows whether
   this wants a list, a queue, or something card-shaped.
2. **The same HTML/CSS inside Tauri, Python as a sidecar.** Works immediately.
   Fat package, but a real app.
3. **Replace the sidecar with the Rust core.** Drops Python, roughly halves the
   bundle.

Each step is usable on its own, and the design work from step 1 carries through
unchanged -- the CSS written in an afternoon is the CSS the final product
ships. Only step 3 depends on the parked ONNX export question (design.md §7).

Step 1 is also the step that is worth doing under *any* of these options: a
local web UI served by the existing Python commits to nothing, and answers the
question this document cannot -- whether the interface wants a list, a queue,
or something card-shaped.

## On Godot

Raised because it is familiar, which is not a small consideration -- a UI that
gets finished beats a better one that does not.

It would work, as a pure client over the Python core, talking HTTP or stdio.
But GDScript has no numerical stack, no ONNX runtime, no ID3 library, and no
clean way to decode an mp3 to a float array, so anything beyond the UI needs a
C++ GDExtension -- writing most of the application in C++ with Godot as a
shell. And the interface wanted here is a dense list with colour swatches, a
play button and keyboard shortcuts, which is where Godot's Control nodes are
weakest and a browser is strongest.

Charts are a second instance of the same problem. `draw_polyline` exists, but
axes, ticks, labels, hover and tooltips are all hand-rolled, and they are free
in a browser.

If Godot is what gets built, the sidecar architecture is sound; the cost is
packaging two runtimes per platform rather than anything fundamental.

## The floor on "lightweight"

The app cannot be smaller than the model: ~300 MB at fp16, ~150 MB quantised to
int8 at some accuracy cost we have not measured. The alternative is downloading
it on first run, trading bundle size for a network dependency and hosting.
Whatever the frontend, that term dominates -- the UI is noise beside it.

## What the interface actually has to do

From how the tool is used now, rather than from imagination:

- show a batch, each track with its predicted label and how sure it is
- play a track instantly, from the middle rather than the intro
- accept or correct in one keystroke, without reaching for a mouse
- make the level of the prediction visible -- a full label, a colour, a hue, a
  tone, or nothing -- since that is the app's honest statement of confidence
- show progress over time (below), because the reason to label another batch is
  that the last one moved the number

## Statistics, and what they need from the backend

Five things are worth plotting, each because it answers a question that comes
up in practice:

| chart | question |
|---|---|
| cost per track, per round | is this working? -- the headline, already the objective metric |
| labels confirmed / predicted / untouched | how far through the collection am I? |
| per-axis coverage and accuracy, per round | are the two dials set right? |
| label distribution | which colours is the collection made of, and which are starved of examples? |
| correction matrix | where do the model and the ear actually disagree? |

The first is the only one that decides anything on its own. The last is the one
that has historically been most informative, and it also says what to go label
next.

**What already exists.** `labels` is append-only and nothing is overwritten, and
it is typed -- `colour_id` and `stars`, with hue and tone joinable from the
`colours` table -- so a chart never has to parse a string against a config file
to find out what it is looking at. Label counts over time, correction counts
over time, and the full correction matrix all fall out of it. `tracks.first_seen` / `missing_at` give the library's
size at any past date, which is the other axis of "progress as the collection
grows".

**What used not to exist, and was being thrown away every round.** Confidence
at the moment of prediction, which axis spoke, which batch a row belonged to,
how many tracks the model was trained on, and the held-out scores from each
fit. All now recorded (design.md §7): `rounds` as a first-class row that `labels`
points back at, one wide `predictions` row per track per round -- carrying every
axis including the ones that stayed silent, so a different `misleading_cost` can
be replayed against history -- and `fits` / `fit_axes`, one row per fit, each
stamped with the encoder fingerprint so a moving curve can be attributed.

Done before the UI rather than after, because a chart can only reach as far
back as its data, and the rounds that pass in the meantime are rounds the graph
will not have.

**One caution about honesty.** Only ~10% of a round lands in the validation
slice, so a 300-track round is scored on about 30 tracks: a few points of noise,
and any curve drawn through it will look like progress. `fit_axes.n` is stored
for exactly this reason -- show it alongside every point, and do not draw a
trend through the first few rounds.

The playlist round-trip through DJ software exists only because there is no
interface. A UI with inline playback removes the export, the import, the tag
reload, and the `#`-in-path failures, and is likely to halve the time per
track -- which matters more than anything else, because labelling is the
bottleneck for the model, the ceiling, and whether cloud is ever worth it.
