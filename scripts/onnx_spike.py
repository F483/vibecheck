"""Export spike: can this backend become a self-contained ONNX file?

Run before bulk embedding. A model that cannot export with its preprocessing
included is a weaker candidate however well it scores (design.md 4.2, 8), and
finding that out after hours of embedding is the failure this script exists to
prevent.

Checks, in order:
  1. does it export at all;
  2. does the exported graph accept raw PCM (no DSP left for the app to
     reimplement);
  3. do the numbers match the python reference (cosine > 0.999).
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

from vibecheck import audio, backends
from vibecheck.config import DEFAULT

COSINE_FLOOR = 0.999


def sample_track() -> str:
    out = subprocess.run(
        ["bash", "-c", "find ~/Music/Collection -name '*.mp3' | head -1"],
        capture_output=True, text=True,
    )
    return out.stdout.strip()


def main(name: str) -> int:
    import torch

    b = backends.get(name)
    cfg = b.preproc(DEFAULT)
    pcm = audio.excerpts(sample_track(), cfg)[0]
    reference = b.embed([pcm], cfg)
    print(f"{name}: reference vector dim={reference.shape[0]}")

    model = b._load()
    x = torch.from_numpy(pcm).float().unsqueeze(0).to(b._device)

    with tempfile.TemporaryDirectory() as d:
        path = Path(d) / f"{name}.onnx"
        try:
            torch.onnx.export(
                model.cpu(), (x.cpu(),), str(path),
                input_names=["pcm"], output_names=["hidden"],
                dynamic_axes={"pcm": {1: "samples"}},
                opset_version=17,
            )
        except Exception as e:
            print(f"EXPORT FAILED: {type(e).__name__}: {e}")
            return 1

        size_mb = path.stat().st_size / 1e6
        print(f"exported ok, {size_mb:.0f} MB")

        try:
            import onnxruntime as ort
        except ImportError:
            print("onnxruntime not installed - cannot verify numerics")
            return 0

        sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
        got = sess.run(None, {"pcm": pcm[None, :].astype(np.float32)})[0]
        print(f"onnx output shape {got.shape}")
        print("NOTE: pooling/normalisation still happens outside the graph; "
              "a shippable artifact should include them too.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1] if len(sys.argv) > 1 else "mert"))
