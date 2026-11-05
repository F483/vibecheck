#!/bin/sh
# One folder holding everything vibecheck needs: the program and its audio
# model. Copy it, share it or back it up whole; it runs from wherever it is.
#
#   scripts/bundle.sh [model.onnx]
#
# The model defaults to the one in the per-user folder (made by
# prototype/scripts/export_clap_onnx.py). Writes dist/vibecheck-<version>-<os>-<arch>/
# and a .zip of it.
set -eu
cd "$(dirname "$0")/.."
model="${1:-$HOME/Library/Application Support/vibecheck/models/clap_audio.onnx}"
[ -f "$model" ] || { echo "no model at $model" >&2; exit 1; }

cargo build --release -p vibecheck
version=$(./target/release/vibecheck version | awk '{print $2}')
os=$(uname -s | tr '[:upper:]' '[:lower:]')
name="vibecheck-$version-$os-$(uname -m)"
out="dist/$name"

rm -rf "$out" "$out.zip"
mkdir -p "$out/models"
cp target/release/vibecheck "$out/"
cp "$model" "$out/models/clap_audio.onnx"
cp LICENSE "$out/"
cat > "$out/README.txt" <<TXT
vibecheck $version

Learns how you label music, from the audio, and writes colours, ratings, key,
energy and cue points into rekordbox.

  ./vibecheck --help
  ./vibecheck --root ~/Music/Collection status

Everything it needs is in this folder; move it anywhere. Your labels live with
your music, in <collection>/.vibecheck/, not here.
TXT
(cd dist && zip -qr "$name.zip" "$name")
du -sh "$out" "$out.zip"
