# Frontend options

Not started, and deliberately so — the backend is still being used and shaped.
This records the thinking so it does not have to be redone.

Requirements, as stated: lightweight, fast, responsive, cross-platform, and a
**stylised, memorable look** -- the reference points are Ableton devices and
guitar pedals, not standard desktop chrome. That is a design-led application
with a list inside it, rather than a list application with a theme.

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
engine.

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
identity.** Eight named colours, grouped into four hue families and two tones.
That is a palette, a hierarchy and a metaphor handed over by the problem
itself.

A device-panel design where the colour swatches *are* the interface -- and
where a hue or tone fallback shows as a coarser or dimmer form of the same
colour -- would be both distinctive and honest about what the model is actually
saying. The prediction levels are not an implementation detail to hide; they
are the app telling you how sure it is, and they have an obvious visual grammar.

That is the first thing to prototype in plain HTML, because it decides whether
the look works at all, and it costs an afternoon rather than a packaging
commitment.

## Recommended: Tauri

Uses the operating system's own webview instead of bundling a browser, so
HTML/CSS — the best tooling that exists for making something look how you want
— comes at a fraction of Electron's weight.

End state:

```
Tauri shell     HTML/CSS/JS, fully custom, ~5 MB
Rust core       ort (ONNX Runtime) for inference
                symphonia for mp3 decode
                rusqlite, plus ~50 lines of logistic regression
model file      ~300 MB fp16, ~150 MB int8
```

One binary, no Python, no second runtime. Everything in the current backend
maps across: the logic is small, and the parts that are not -- decode,
inference, storage -- all have solid Rust equivalents.

## Path, in three shippable steps

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
- show progress: labels so far, and how much the model has improved

The playlist round-trip through DJ software exists only because there is no
interface. A UI with inline playback removes the export, the import, the tag
reload, and the `#`-in-path failures, and is likely to halve the time per
track -- which matters more than anything else, because labelling is the
bottleneck for the model, the ceiling, and whether cloud is ever worth it.
