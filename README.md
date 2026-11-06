# Vibe Check

Learns how *you* label music, from the audio, and applies it to the rest of
your collection: colour, rating, key, energy and cue points, written into
rekordbox.

A native version for macOS, Windows and Linux, without the Python
dependency setup, is in the works.

The working version is in [`prototype/`](prototype/): see its
[README](prototype/README.md) to install and use it.

## The native version, so far

`core/` is the engine in Rust, with the same commands as the prototype
(`scan`, `status`, `label`, `sync`, `discard`, `analyse`, `cues`, `migrate`).
It reads and writes the same `.vibecheck/` files, so either can be used on
the same collection. `ext/` exposes it to Godot, and `app/` is the Godot app.

```sh
cargo build --release
./target/release/vibecheck status
./target/release/vibecheck label 100
```

It needs the audio model as an ONNX file (145 MB, weights in half
precision), made once from the prototype by
`prototype/scripts/export_clap_onnx.py`, at
`~/Library/Application Support/vibecheck/models/clap_audio.onnx` on macOS.
`scripts/bundle.sh` puts the program and the model in one folder to share or
back up.
No Python, ffmpeg or torch at runtime. Not yet ported: learning cue
placement from your own cues (it uses the model shipped with it).

MIT licensed.
