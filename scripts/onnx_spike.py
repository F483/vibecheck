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

    model = b._load().cpu().eval()
    x = torch.from_numpy(pcm.copy()).float().unsqueeze(0)
    with torch.no_grad():
        reference = model(x).last_hidden_state.mean(dim=1).squeeze(0).numpy()
    print(f"{name}: torch reference dim={reference.shape[0]}")

    out_dir = Path("out/onnx") / name
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{name}.onnx"
    try:
        torch.onnx.export(
            model, (x,), str(path),
            input_names=["pcm"], output_names=["hidden"],
            dynamic_axes={"pcm": {1: "samples"}},
            opset_version=17,
        )
    except Exception as e:
        print(f"EXPORT FAILED: {type(e).__name__}: {e}")
        return 1

    total = sum(f.stat().st_size for f in out_dir.rglob("*")) / 1e6
    files = sorted(f.name for f in out_dir.iterdir())
    print(f"exported ok: {total:.0f} MB across {len(files)} file(s): {files}")

    import onnxruntime as ort

    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    got = sess.run(None, {"pcm": pcm[None, :].astype(np.float32)})[0]
    pooled = got.mean(axis=1).squeeze(0)

    cos = float(np.dot(pooled, reference) /
                (np.linalg.norm(pooled) * np.linalg.norm(reference)))
    print(f"onnx output {got.shape}, pooled cosine vs torch = {cos:.6f}")
    ok = cos >= COSINE_FLOOR
    print("PARITY OK" if ok else f"PARITY FAIL (floor {COSINE_FLOOR})")
    print("NOTE: resampling and pooling are still outside the graph; a shippable "
          "artifact should include them (design.md 4.2).")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1] if len(sys.argv) > 1 else "mert"))
