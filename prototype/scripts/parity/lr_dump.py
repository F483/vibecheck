import sys, json, time, numpy as np
from pathlib import Path
from vibecheck import cli, embed, store
from vibecheck.config import DEFAULT
from vibecheck.predict import Model, targets_for
root = Path.home() / "Music/Collection"
cfg = cli.load_config(root)
lab = store.current(store.labels_db(root), source="user")
paths, X = embed.load(root, "clapw", DEFAULT, sorted(lab))
m = Model(cli.build_axes(cfg)); t0 = time.time(); m.train(X, targets_for([lab[p] for p in paths])); print("fit", time.time() - t0, file=sys.stderr)
live = [p for (p,) in store.labels_db(root).execute("SELECT path FROM tracks WHERE missing_at IS NULL ORDER BY path")]
cand = [p for p in live if p not in lab]
q, Q = embed.load(root, "clapw", DEFAULT, cand)
q, Q = q[:500], Q[:500]
out = {"paths": q, "axes": {}}
preds = m.predict(Q)
for name, mod in m.models.items():
    out["axes"][name] = {"classes": [str(c) for c in mod.classes_], "proba": mod.predict_proba(Q).tolist()}
out["values"] = [{k: str(v) for k, v in p.values.items()} for p in preds]
json.dump(out, open(sys.argv[1], "w"))
