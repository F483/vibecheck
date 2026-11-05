"""Convert a cue model (cue_model.pt) to the native app's format (cue_model.bin).

    uv run python -I scripts/export_cue_model.py <in.pt> <out.bin>

The format: one line of JSON (the metadata and, per tensor, its shape and
offset in floats), a newline, then every tensor as little-endian float32, in
the order listed. Nothing else; readable without PyTorch.
"""

import io
import json
import sys

import numpy as np
import torch

state = torch.load(io.BytesIO(open(sys.argv[1], "rb").read()), weights_only=True)
tensors, blobs, off = [], [], 0


def add(name, t):
    global off
    a = np.ascontiguousarray(np.asarray(t, dtype=np.float32))
    tensors.append({"name": name, "shape": list(a.shape), "offset": off})
    blobs.append(a.tobytes())
    off += a.size


add("mu", state["mu"])
add("sd", state["sd"])
for i, net in enumerate(state["nets"]):
    for k, v in net.items():
        add(f"net{i}.{k}", v.numpy())
head = {"format": "vibecheck-cue-model/1", "d": int(state["d"]), "n_tracks": int(state["n_tracks"]),
        "source": str(state["source"]), "version": str(state.get("version", "")),
        "nets": len(state["nets"]), "tensors": tensors}
with open(sys.argv[2], "wb") as f:
    f.write(json.dumps(head).encode() + b"\n")
    for b in blobs:
        f.write(b)
print(f"wrote {sys.argv[2]}: {len(state['nets'])} networks, {off} floats, trained on {head['n_tracks']} ({head['source']})")
