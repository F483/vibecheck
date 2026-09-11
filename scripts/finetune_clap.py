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
from collections import deque
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, "src")
from vibecheck import audio, evaluate, store  # noqa: E402
from vibecheck.backends.clap import FALLBACK_ID, MODEL_ID  # noqa: E402
from vibecheck.config import DEFAULT  # noqa: E402

SR = 48000
WIN = 10.0
NWIN = 9


class Loader:
    """Thread-prefetched batches.

    Not torch's DataLoader: its worker *processes* deadlock against MPS on
    macOS -- the run stalls silently at 0% CPU with no error. Decoding is an
    ffmpeg subprocess and so releases the GIL, which makes threads a better fit
    anyway. Prefetch depth is bounded so decoding cannot run ahead of the GPU
    and exhaust memory.
    """

    def __init__(self, root, items, classes, proc, batch, shuffle, workers=4,
                 depth=6):
        self.root, self.items, self.proc = root, items, proc
        self.batch, self.shuffle = batch, shuffle
        self.workers, self.depth = workers, depth
        self.cls = {c: i for i, c in enumerate(classes)}
        self.cfg = DEFAULT.__class__(sample_rate=SR, n_excerpts=NWIN,
                                     excerpt_seconds=WIN)

    def _prepare(self, chunk):
        feats, counts, ys = [], [], []
        for rel, label in chunk:
            try:
                ws = audio.excerpts(str(self.root / rel), self.cfg)
                n = min(len(w) for w in ws)
                ws = [w[:n] for w in ws]
            except Exception:
                ws = [np.zeros(int(SR * WIN), dtype=np.float32)]
            f = self.proc(audio=ws, sampling_rate=SR,
                          return_tensors="pt")["input_features"]
            feats.append(f); counts.append(f.shape[0]); ys.append(self.cls[label])
        return torch.cat(feats), counts, torch.tensor(ys)

    def __len__(self):
        return (len(self.items) + self.batch - 1) // self.batch

    def __iter__(self):
        items = list(self.items)
        if self.shuffle:
            np.random.shuffle(items)
        chunks = [items[i:i + self.batch] for i in range(0, len(items), self.batch)]
        with ThreadPoolExecutor(self.workers) as ex:
            pending = deque()
            it = iter(chunks)
            for _ in range(self.depth):
                c = next(it, None)
                if c is None:
                    break
                pending.append(ex.submit(self._prepare, c))
            while pending:
                out = pending.popleft().result()
                c = next(it, None)
                if c is not None:
                    pending.append(ex.submit(self._prepare, c))
                yield out


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
    ap.add_argument("--unfreeze-stages", type=int, default=1,
                    help="train only the last N of the encoder's 4 stages "
                         "(0 = all trainable)")
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

    # Full fine-tuning does not fit: 68M params with AdamW needs the weights
    # plus two optimiser state copies plus activations, and on a 16 GB machine
    # that thrashes -- the process ends up at 20% CPU, almost entirely paged
    # out. Training only the top stage (25M) keeps optimiser state near 300 MB.
    if args.unfreeze_stages:
        stages = tower.audio_encoder.layers
        for prm in tower.parameters():
            prm.requires_grad = False
        for st in list(stages)[-args.unfreeze_stages:]:
            for prm in st.parameters():
                prm.requires_grad = True
    n_train = sum(p.numel() for p in tower.parameters() if p.requires_grad)
    print(f"trainable in encoder: {n_train/1e6:.0f}M of "
          f"{sum(p.numel() for p in tower.parameters())/1e6:.0f}M", flush=True)

    enc_params = [p for p in list(tower.parameters()) + list(projection.parameters())
                  if p.requires_grad]
    opt = torch.optim.AdamW([
        {"params": enc_params, "lr": args.lr_encoder},
        {"params": head.parameters(), "lr": args.lr_head},
    ], weight_decay=0.01)
    lossf = torch.nn.CrossEntropyLoss()

    loaders = {k: Loader(root, v, classes, proc, args.batch_tracks,
                         shuffle=(k == "train"))
               for k, v in split.items()}

    def run(split_name, train: bool):
        tower.train(train); head.train(train)
        tot = corr = 0; loss_sum = 0.0
        for n_done, (xs, counts, ys) in enumerate(loaders[split_name], 1):
            xs, ys = xs.to(dev), ys.to(dev)
            with torch.set_grad_enabled(train):
                feats = projection(tower(xs).pooler_output)
                logits = track_logits(head(feats), counts)
                loss = lossf(logits, ys)
                if train:
                    opt.zero_grad(); loss.backward(); opt.step()
            loss_sum += float(loss.detach()) * len(ys)
            corr += int((logits.argmax(1) == ys).sum()); tot += len(ys)
            if train and n_done % 50 == 0 and dev == "mps":
                torch.mps.empty_cache()
            if train and n_done % 200 == 0:
                print(f"    {n_done}/{len(loaders[split_name])} batches  "
                      f"running acc {corr/max(tot,1)*100:.1f}%", flush=True)
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
