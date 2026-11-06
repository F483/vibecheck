# Parity checks

Each compares the Rust core with the Python prototype on the collection at
`~/Music/Collection` (reading only; nothing is written except cache rows).

| example | checks | last result |
|---|---|---|
| `embed_check N` | CLAP vectors against the prototype's cached ones | cosine ≥ 0.99997 |
| `lr_check <json>` | logistic regression against scikit-learn (`prototype/scripts/parity/lr_dump.py`) | p within 5e-4, decisions 500/500 |
| `ke_check N` | key and energy against the prototype's cache | 224/224 identical |
| `xml_check …` | rekordbox xml writes against the prototype's (`xml_make.py`, `xml_compare.py`) | 14,580 tracks identical |
| `cue_check <json>` | cue network and features (`cue_dump.py`) | network 4e-6; transitions 39/40 on the same features |
| `cue_quality N` | cue transitions against your own hand-set cues | 87.7% (prototype features 90.0%) |
| `precision_check <fp32> <fp16> N <out.jsonl>` | half against full precision model, same audio | 500 tracks: cosine ≥ 0.9999995, decisions 499/500 (one tie) |
| `model_check N` | a candidate model (`VIBECHECK_MODELS=<dir>`) against the cached vectors | |

Timings: `bench <tracks.json>` (the same tracks as the prototype's bench),
`load_time` (loading the audio model).

    cargo run --release --example ke_check 60
