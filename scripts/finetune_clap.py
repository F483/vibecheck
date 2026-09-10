"""Fine-tune CLAP's audio encoder on the user's labels.

Every model tested so far was **frozen**: we trained a classifier to read a
representation built for someone else's task. Four such representations
converge at ~56% on 8-way colour, which says no off-the-shelf encoder happens
to encode these distinctions strongly. It does not say the audio lacks them.

Fine-tuning is the one intervention that changes what is *in* the
representation rather than how it is read, and it is the last untested idea
with large upside.

Design notes:
  - trains on individual 10 s windows, not track averages, so the encoder sees
    ~9x more examples and cannot hide behind a blurred mean;
  - evaluates at track level by averaging window logits, which is how the tool
    would actually predict;
  - low LR on the encoder, higher on the head: the encoder already knows a lot
    about audio and should be nudged, not retrained;
  - decodes on the fly in worker processes so the CPU feeds the GPU rather than
    materialising a large mel cache.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import DataLoader, Dataset

sys.path.insert(0, "src")
from vibecheck import audio, evaluate, store  # noqa: E402
from vibecheck.backends.clap import FALLBACK_ID, MODEL_ID  # noqa: E402
from vibecheck.config import DEFAULT  # noqa: E402

SR = 48000
WIN = 10.0
NWIN = 9


class Windows(Dataset):
    def __init__(self, root: Path, items, classes, proc):
        self.root, self.items, self.proc = root, items, proc
        self.cls = {c: i for i, c in enumerate(classes)}
        self.cfg = DEFAULT.__class__(sample_rate=SR, n_excerpts=NWIN,
                                     excerpt_seconds=WIN)

    def __len__(self):
        return len(self.items)

    def __getitem__(self, i):
        rel, label = self.items[i]
        try:
            ws = audio.excerpts(str(self.root / rel), self.cfg)
        except Exception:
            ws = [np.zeros(int(SR * WIN), dtype=np.float32)]
        n = min(len(w) for w in ws)
        ws = [w[:n] for w in ws]
        feats = self.proc(audio=ws, sampling_rate=SR, return_tensors="pt")
        return feats["input_features"], self.cls[label], i


def collate(batch):
    xs = torch.cat([b[0] for b in batch])
    counts = [b[0].shape[0] for b in batch]
    ys = torch.tensor([b[1] for b in batch])
    idx = torch.tensor([b[2] for b in batch])
    return xs, counts, ys, idx


def track_logits(logits, counts):
    """Average window logits back to one prediction per track."""
    out, o = [], 0
    for c in counts:
        out.append(logits[o:o + c].mean(0))
        o += c
    return torch.stack(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--epochs", type=int, default=6)
    ap.add_argument("--batch-tracks", type=int, default=4)
    ap.add_argument("--lr-encoder", type=float, default=1e-5)
    ap.add_argument("--lr-head", type=float, default=1e-3)
    ap.add_argument("--limit", type=int, default=0, help="tracks per split, 0=all")
    ap.add_argument("--unfreeze-last", type=int, default=0,
                    help="0 = whole encoder trainable; N = only last N blocks")
    args = ap.parse_args()

    from transformers import AutoProcessor, ClapModel

    root = Path.home() / "Music" / "Collection"
    lab = store.labels_db(root)
    labels = store.current_labels(lab, source="user")
    hb = dict(lab.execute("SELECT path, hash FROM tracks"))
    paths = sorted(labels)
    parts = evaluate.split([hb[p] for p in paths])
    col = {p: labels[p].split("_")[0] for p in paths}
    classes = sorted(set(col.values()))

    split = {k: [(p, col[p]) for p, s in zip(paths, parts) if s == k]
             for k in ("train", "val")}
    if args.limit:
        for k in split:
            split[k] = split[k][:args.limit]
    print({k: len(v) for k, v in split.items()}, f"{len(classes)} classes", flush=True)

    dev = "mps" if torch.backends.mps.is_available() else "cpu"
    mid = MODEL_ID
    try:
        clap = ClapModel.from_pretrained(mid)
    except Exception:
        mid = FALLBACK_ID
        clap = ClapModel.from_pretrained(mid)
    proc = AutoProcessor.from_pretrained(mid)
    tower = clap.audio_model.to(dev)
    projection = clap.audio_projection.to(dev)
    head = torch.nn.Linear(512, len(classes)).to(dev)

    if args.unfreeze_last:
        for p in tower.parameters():
            p.requires_grad = False
        blocks = [m for m in tower.modules() if isinstance(m, torch.nn.LayerNorm)]
        for m in blocks[-args.unfreeze_last:]:
            for p in m.parameters():
                p.requires_grad = True

    enc_params = [p for p in list(tower.parameters()) + list(projection.parameters())
                  if p.requires_grad]
    opt = torch.optim.AdamW([
        {"params": enc_params, "lr": args.lr_encoder},
        {"params": head.parameters(), "lr": args.lr_head},
    ], weight_decay=0.01)
    lossf = torch.nn.CrossEntropyLoss()

    loaders = {k: DataLoader(Windows(root, v, classes, proc),
                             batch_size=args.batch_tracks, shuffle=(k == "train"),
                             num_workers=4, collate_fn=collate, persistent_workers=True)
               for k, v in split.items()}

    def run(split_name, train: bool):
        tower.train(train); head.train(train)
        tot = corr = 0; loss_sum = 0.0
        for xs, counts, ys, _ in loaders[split_name]:
            xs, ys = xs.to(dev), ys.to(dev)
            with torch.set_grad_enabled(train):
                feats = projection(tower(xs).pooler_output)
                logits = track_logits(head(feats), counts)
                loss = lossf(logits, ys)
                if train:
                    opt.zero_grad(); loss.backward(); opt.step()
            loss_sum += float(loss) * len(ys)
            corr += int((logits.argmax(1) == ys).sum()); tot += len(ys)
        return loss_sum / max(tot, 1), corr / max(tot, 1)

    best = 0.0
    for ep in range(1, args.epochs + 1):
        tl, ta = run("train", True)
        with torch.no_grad():
            vl, va = run("val", False)
        best = max(best, va)
        print(f"epoch {ep}  train loss {tl:.3f} acc {ta*100:.1f}%   "
              f"val loss {vl:.3f} acc {va*100:.1f}%   best {best*100:.1f}%", flush=True)
        torch.save({"tower": tower.state_dict(), "head": head.state_dict()},
                   f"out/finetune_ep{ep}.pt")
    print(f"BEST val accuracy {best*100:.1f}%  (frozen-probe reference: 56.5%)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
