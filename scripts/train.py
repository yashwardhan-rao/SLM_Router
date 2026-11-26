#!/usr/bin/env python3
"""Train the Rule 5 classifier on data/rule5_*.jsonl and export it for the router.

    python scripts/train.py      # writes assets/rule5.bin and tests/fixtures/classifier_parity.json

L2 logistic regression by SGD over the hashed features of scripts/features.py: a stratified 80/20
split picks L2 and epochs by validation log-loss, then the model is refit on the whole corpus.
Sparse weights are quantised to int8 with one f32 scale; everything after that (threshold, parity
fixture) uses the dequantised weights, so it matches the router exactly. VARIANTS pin some dense weights at 0;
the one with the best dev recall is exported.
The threshold is calibrated on the dev split only: the lowest t with 0 false-cloud, plus MARGIN.
Holdout prompts never enter training or calibration. Standard library only.
"""
import json
import math
import pathlib
import random
import struct
import sys

import features as fx

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "data"
MARGIN = 0.02
GRID = [(l2, ep) for l2 in (1e-6, 1e-5, 1e-4) for ep in (5, 10, 20)]
DENSE = ["is_code", "big_o", "cue_score", "log2_tokens"]
# Rule 5 already ORs in the cue score, and generated hard prompts run longer than easy ones,
# so those two dense weights may only relearn the keywords or a corpus artefact. Each is tried pinned at 0.
VARIANTS = {"A": (), "B": (2,), "C": (2, 3)}


def jsonl(path):
    return [json.loads(line) for line in path.read_text("utf-8").splitlines() if line.strip()]


def featurize(text):
    """As the router sees a single-turn request: features.rs adds 4 tokens per message."""
    cue = fx.cue_score(text)
    return fx.buckets(text), fx.dense(text, fx.estimate(text) + 4, cue)


def score(model, x):
    sparse, dense, bias = model
    bs, d = x
    s = sum(sparse[b] for b in bs) / math.sqrt(len(bs)) if bs else 0.0
    return bias + s + sum(w * v for w, v in zip(dense, d))


def prob(model, x):
    z = score(model, x)
    return 1 / (1 + math.exp(-z)) if z >= 0 else math.exp(z) / (1 + math.exp(z))


def fit(xs, ys, l2, epochs, frozen=(), seed=0):
    sparse, dense, bias = [0.0] * fx.BUCKETS, [0.0] * 4, 0.0
    order, rng, step = list(range(len(xs))), random.Random(seed), 0
    for _ in range(epochs):
        rng.shuffle(order)
        for i in order:
            step += 1
            lr = 0.5 / (1 + step / len(xs))
            bs, d = xs[i]
            g = prob((sparse, dense, bias), xs[i]) - ys[i]
            inv = 1 / math.sqrt(len(bs)) if bs else 0.0
            for b in bs:  # shortcut: L2 decays only active weights (lazy); fine for this sparsity
                sparse[b] -= lr * (g * inv + l2 * sparse[b])
            for j in (j for j in range(4) if j not in frozen):
                dense[j] -= lr * (g * d[j] + l2 * dense[j])
            bias -= lr * g
    return sparse, dense, bias


def logloss(model, xs, ys):
    eps = 1e-7
    return -sum(y * math.log(max(prob(model, x), eps)) + (1 - y) * math.log(max(1 - prob(model, x), eps))
                for x, y in zip(xs, ys)) / len(xs)


def quantise(model):
    sparse, dense, bias = model
    scale = (max(abs(w) for w in sparse) / 127) or 1.0
    q = [max(-127, min(127, round(w / scale))) for w in sparse]
    return q, scale, (([v * scale for v in q]), dense, bias)


def export(q, scale, dense, bias, threshold):
    head = b"SLR5" + struct.pack("<HIfff4f", 1, fx.BUCKETS, scale, bias, threshold, *dense)
    (ROOT / "assets").mkdir(exist_ok=True)
    (ROOT / "assets" / "rule5.bin").write_bytes(head + struct.pack(f"<{fx.BUCKETS}b", *q))


def train(xs, ys, tr, val, frozen):
    """Grid-search on the split, refit on everything; returns (raw model, quantised (q, scale, model))."""
    best = None
    for l2, ep in GRID:
        m = fit([xs[i] for i in tr], [ys[i] for i in tr], l2, ep, frozen)
        best = min(best or (9.0, l2, ep), (logloss(m, [xs[i] for i in val], [ys[i] for i in val]), l2, ep))
    loss, l2, ep = best
    raw = fit(xs, ys, l2, ep, frozen)
    print(f"  best l2 {l2:g}, epochs {ep}: val log-loss {loss:.4f}")
    return raw, quantise(raw)


def calibrate(model, dev):
    """Lowest threshold with 0 dev false-cloud, plus MARGIN; and dev hard recall with the cue OR."""
    p = {c["id"]: prob(model, featurize(c["prompt"])) for c in dev}
    threshold = max(p[c["id"]] for c in dev if c["expect"] == "local") + MARGIN
    hard = [c for c in dev if c["expect"] == "cloud"]
    caught = sum(fx.cue_score(c["prompt"]) >= 2 or p[c["id"]] >= threshold for c in hard)
    return threshold, caught, len(hard)


def main():
    prompts = {r["id"]: r["prompt"] for r in jsonl(DATA / "rule5_prompts.jsonl")}
    labels = jsonl(DATA / "rule5_labels.jsonl")
    xs = [featurize(prompts[r["id"]]) for r in labels]
    ys = [1.0 if r["label"] == "cloud" else 0.0 for r in labels]
    print(f"corpus: {len(ys)} rows, {int(sum(ys))} cloud")
    rng = random.Random(7)
    val = set()
    for cls in (0.0, 1.0):
        idx = [i for i, y in enumerate(ys) if y == cls]
        val |= set(rng.sample(idx, len(idx) // 5))
    tr = [i for i in range(len(ys)) if i not in val]
    cases = json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))
    assert all(c["split"] in ("dev", "holdout") for c in cases)
    dev = [c for c in cases if c["split"] == "dev"]  # holdout never enters training or calibration
    hard = [c for c in dev if c["expect"] == "cloud"]
    print(f"dev hard recall, cue only: {sum(fx.cue_score(c['prompt']) >= 2 for c in hard)}/{len(hard)}")

    results = []
    for name, frozen in VARIANTS.items():
        print(f"variant {name}: dense weights pinned at 0: {[DENSE[j] for j in frozen] or 'none'}")
        raw, (q, scale, model) = train(xs, ys, tr, val, frozen)
        worst = max(abs(prob(raw, x) - prob(model, x)) for x in xs)
        assert worst < 1e-2, f"quantisation moved a probability by {worst}"
        threshold, caught, n = calibrate(model, dev)
        print(f"  threshold {threshold:.4f}; dev hard recall cue+classifier {caught}/{n}")
        results.append(((caught, len(frozen)), name, q, scale, model, threshold))
    # Fixed before any run: most dev hard prompts caught; ties go to fewer active dense features.
    _, name, q, scale, model, threshold = max(results, key=lambda r: r[0])
    print(f"selected variant {name}")
    export(q, scale, model[1], model[2], threshold)

    texts = fx.PARITY + [c["prompt"] for c in dev][::2]
    rows = [{"text": t, "est_tokens": fx.estimate(t) + 4, "cue_score": fx.cue_score(t),
             "prob": round(prob(model, featurize(t)), 6)} for t in texts[:50]]
    (ROOT / "tests" / "fixtures" / "classifier_parity.json").write_text(
        "[\n" + ",\n".join(json.dumps(r, ensure_ascii=False) for r in rows) + "\n]\n", "utf-8", newline="\n")
    print(f"wrote assets/rule5.bin ({(ROOT / 'assets' / 'rule5.bin').stat().st_size} B) and {len(rows)} parity rows")


def self_check():
    q, scale, m = quantise(([0.5, -1.27] + [0.0] * (fx.BUCKETS - 2), [0.1] * 4, 0.0))
    assert q[:2] == [50, -127] and abs(m[0][0] - 0.5) < scale
    assert abs(prob(m, ([], [0.0] * 4)) - 0.5) < 1e-9 and prob(m, ([1], [0.0] * 4)) < 0.3


if __name__ == "__main__":
    self_check()
    sys.exit(main())
