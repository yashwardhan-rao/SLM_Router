#!/usr/bin/env python3
"""Offline routing benchmark on tests/fixtures/routing_bench.json (held out from training and tuning).

    python scripts/eval.py --bench        # or: python scripts/routing_bench.py

Compares three routers on the same labelled prompts: always cloud, the best single character-length
cutoff for this dataset, and SLM_Router itself. The router is the compiled `--release` binary's
`slm-router route`, which runs the shipped features(), decide() and classifier, so nothing is
reimplemented here. Prints one Markdown table. Standard library only; needs cargo, no network.
"""
import json
import pathlib
import re
import statistics
import subprocess
import sys
import tempfile

import features as fx

ROOT = pathlib.Path(__file__).resolve().parent.parent
BENCH = ROOT / "tests" / "fixtures" / "routing_bench.json"
CONFIG = ROOT / "slm-router.example.toml"
# Pricing tier for the cost column: the example config's top tier, USD per million tokens.
PRICE = ("gpt-6.2-sol", 2.00, 10.00)
# Assumed reply length; a counterfactual, since nothing is generated here.
OUT_TOKENS = 300


def bigrams(text):
    w = re.findall(r"[a-z0-9]+", text.lower())
    return {tuple(w[i:i + 2]) for i in range(len(w) - 1)} or {tuple(w)}


def leakage(rows):
    """Max word-bigram Jaccard of each bench prompt against everything used to train or tune."""
    train = [json.loads(l)["prompt"] for l in (ROOT / "data" / "rule5_prompts.jsonl").read_text("utf-8").splitlines() if l.strip()]
    tune = [c["prompt"] for c in json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))]
    seen = [bigrams(p) for p in train + tune]
    worst = 0.0
    for r in rows:
        g = bigrams(r["prompt"])
        worst = max(worst, max(len(g & s) / len(g | s) for s in seen))
    return worst, len(train), len(tune)


def route(rows):
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    exe = ROOT / "target" / "release" / ("slm-router.exe" if sys.platform == "win32" else "slm-router")
    with tempfile.TemporaryDirectory() as d:
        path = pathlib.Path(d) / "prompts.jsonl"
        path.write_text("".join(json.dumps({"prompt": r["prompt"]}) + "\n" for r in rows), "utf-8")
        out = subprocess.run([str(exe), "route", "--config", str(CONFIG), "--prompts", str(path)],
                             cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return [json.loads(l) for l in out.splitlines() if l.strip()]


def cost(prompt):
    return ((fx.estimate(prompt) + 4) * PRICE[1] + OUT_TOKENS * PRICE[2]) / 1e6


def metrics(rows, local):
    """`local`: one bool per row, True when that router keeps the prompt local."""
    n_cloud = sum(r["label"] == "cloud" for r in rows)
    false_local = sum(k and r["label"] == "cloud" for r, k in zip(rows, local))
    correct = sum(k == (r["label"] == "local") for r, k in zip(rows, local))
    full = sum(cost(r["prompt"]) for r in rows)
    paid = sum(cost(r["prompt"]) for r, k in zip(rows, local) if not k)
    # A wrongly local answer gets re-asked on cloud, so it is charged at cloud price too.
    redo = sum(cost(r["prompt"]) for r, k in zip(rows, local) if k and r["label"] == "cloud")
    return {"local": sum(local) / len(rows), "false_local": false_local / n_cloud,
            "accuracy": correct / len(rows), "saved": 1 - paid / full, "saved_adj": 1 - (paid + redo) / full}


def best_cutoff(rows):
    """Local when len(prompt) <= L, with L chosen on `rows` for best accuracy."""
    best = None
    for cut in [-1] + sorted({len(r["prompt"]) for r in rows}):
        local = [len(r["prompt"]) <= cut for r in rows]
        m = metrics(rows, local)
        if best is None or m["accuracy"] > best[1]["accuracy"]:
            best = (cut, m, local)
    return best


def pct(v):
    return f"{v * 100:.1f}%"


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    rows = json.loads(BENCH.read_text("utf-8"))
    worst, n_train, n_tune = leakage(rows)
    print(f"leakage check: max word-bigram Jaccard {worst:.2f} against {n_train} training and {n_tune} tuning prompts")
    if worst >= 0.5:
        sys.exit("a bench prompt overlaps training or tuning data (Jaccard >= 0.5); fix the fixture first")
    routed = route(rows)
    assert len(routed) == len(rows) and not any("error" in r for r in routed), routed
    cut, cut_m, _ = best_cutoff(rows)
    # The same heuristic tuned out of sample, on the older fixture, then applied here.
    eval_rows = [{"prompt": c["prompt"], "label": c["expect"]}
                 for c in json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))]
    oos = best_cutoff(eval_rows)[0]
    ours = [r["target"] == "local" for r in routed]
    us = statistics.median(r["route_us"] for r in routed)
    table = [("Always cloud", metrics(rows, [False] * len(rows)), "n/a"),
             (f"Length cutoff tuned on this dataset (local if <= {cut} chars)", cut_m, "n/a"),
             (f"Length cutoff tuned on routing_eval.json (local if <= {oos} chars)",
              metrics(rows, [len(r["prompt"]) <= oos for r in rows]), "n/a"),
             ("SLM_Router (rules + classifier)", metrics(rows, ours), f"{us:.1f}")]
    n_local = sum(r["label"] == "local" for r in rows)
    mean = {k: statistics.mean(len(r["prompt"]) for r in rows if r["label"] == k) for k in ("local", "cloud")}
    print(f"\n{len(rows)} prompts ({n_local} local, {len(rows) - n_local} cloud); cost at {PRICE[0]} "
          f"${PRICE[1]:.2f} / ${PRICE[2]:.2f} per 1M tokens, {OUT_TOKENS} output tokens per request; "
          f"mean prompt length {mean['local']:.0f} chars local, {mean['cloud']:.0f} cloud\n")
    print("| Router | Routed local | False-local rate | Accuracy | Cost reduction vs always-cloud (raw / misroute-adjusted) | Median routing latency (µs) |")
    print("| :--- | ---: | ---: | ---: | ---: | ---: |")
    for name, m, lat in table:
        print(f"| {name} | {pct(m['local'])} | {pct(m['false_local'])} | {pct(m['accuracy'])} | "
              f"{pct(m['saved'])} / {pct(m['saved_adj'])} | {lat} |")
    print("\nSLM_Router accuracy by category:", ", ".join(
        f"{c} {pct(sum(k == (r['label'] == 'local') for r, k in zip(rows, ours) if r['category'] == c) / sum(r['category'] == c for r in rows))}"
        for c in dict.fromkeys(r["category"] for r in rows)))
    reasons = statistics.multimode([r["reason"] for r in routed])
    print("SLM_Router reasons:", ", ".join(f"{k} {sum(r['reason'] == k for r in routed)}" for k in sorted({r["reason"] for r in routed})),
          f"(most common: {', '.join(reasons)})")


if __name__ == "__main__":
    main()
