#!/usr/bin/env python3
"""Build the Rule 5 training corpus in three resumable, append-only stages:

    python scripts/corpus.py generate --pairs 750   # easy/hard pairs per domain -> data/rule5_prompts.jsonl
    python scripts/corpus.py answer                 # llama3.2:1b answers -> data/rule5_answers.jsonl (untracked)
    python scripts/corpus.py label --judge-base URL --judge-model M   # judge -> data/rule5_labels.jsonl

Every stage skips ids already written, so it can be stopped and rerun in short batches
(--limit N). Run `scripts/judge.py --calibrate` with the same judge before `label`.
`label` grades --judge-batch answers per judge call; ids the judge skips are retried next run.
Generated prompts that overlap any fixture prompt (word-bigram Jaccard >= 0.5) are dropped.
"""
import argparse
import json
import pathlib
import re
import sys

import judge

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "data"
PROMPTS, ANSWERS, LABELS = (DATA / f"rule5_{n}.jsonl" for n in ("prompts", "answers", "labels"))
DOMAINS = [
    "Linux, shell and DevOps", "Docker and Kubernetes", "cryptography and TLS", "web security",
    "GPU programming and CUDA", "performance tuning and profiling", "regular expressions and text processing",
    "probability and statistics", "algebra, number theory and proofs", "algorithms and data structures",
    "SQL and database internals", "distributed systems", "networking and TCP/IP", "frontend web development",
    "Rust", "Python", "JavaScript and TypeScript", "Java, Go and C/C++", "machine learning",
    "git and version control", "everyday writing and editing", "history, science and general knowledge",
    "personal finance and everyday maths", "cooking, travel and daily life",
]
GEN = """Write {k} pairs of questions a user might ask an AI assistant about {domain}.
In each pair, EASY has a short, well-known answer that a small 1-billion-parameter model gets right: a definition, \
a single fact, one command, a simple conversion or a short creative request. HARD is on the same topic but needs \
expert knowledge, multi-step reasoning, a proof, a careful calculation, real debugging or non-trivial code.
Vary phrasing, length and style: casual one-liners, questions with a pasted error or snippet, multi-sentence context.
Do not reuse these: {avoid}
Output only {k} lines, each a JSON object: {{"easy": "...", "hard": "..."}}"""


def rows(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text("utf-8").splitlines() if line.strip()]


def append(path, row):
    DATA.mkdir(exist_ok=True)
    with path.open("a", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(row, ensure_ascii=False) + "\n")


def bigrams(text):
    w = re.findall(r"[a-z0-9]+", text.lower())
    return {tuple(w[i:i + 2]) for i in range(len(w) - 1)} or {tuple(w)}


def overlaps(text, seen):
    g = bigrams(text)
    return any(len(g & s) / len(g | s) >= 0.5 for s in seen)


def parse_pairs(reply):
    """Every {"easy","hard"} object in the reply, even pretty-printed or fenced; junk is skipped."""
    out, dec, i = [], json.JSONDecoder(), reply.find("{")
    while i != -1:
        try:
            d, end = dec.raw_decode(reply, i)
        except json.JSONDecodeError:
            i = reply.find("{", i + 1)
            continue
        if isinstance(d, dict) and all(isinstance(d.get(k), str) and d[k].strip() for k in ("easy", "hard")):
            out.append((d["easy"].strip(), d["hard"].strip()))
        i = reply.find("{", end)
    return out


def generate(args):
    fixtures = json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))
    seen = [bigrams(c["prompt"]) for c in fixtures]
    have = rows(PROMPTS)
    seen += [bigrams(r["prompt"]) for r in have]
    per_domain = -(-args.pairs // len(DOMAINS))
    calls = 0
    for d_i, domain in enumerate(DOMAINS):
        mine = [r for r in have if r["domain"] == domain]
        batch = len({r["id"].rsplit("-", 2)[0] for r in mine})
        while len(mine) < 2 * per_domain and calls < args.limit:
            avoid = "; ".join(r["prompt"] for r in mine[-6:]) or "(none yet)"
            reply = judge.ask(args.gen_base, args.gen_model, GEN.format(k=args.batch, domain=domain, avoid=avoid),
                              max(4096, args.batch * 200), args.judge_key, temperature=0.9)
            calls += 1
            kept = 0
            for p_i, pair in enumerate(parse_pairs(reply)):
                if any(overlaps(p, seen) for p in pair):
                    continue
                for intent, prompt in zip(("easy", "hard"), pair):
                    r = {"id": f"d{d_i:02d}b{batch:03d}-{p_i:02d}-{intent}", "domain": domain,
                         "intent": intent, "prompt": prompt}
                    append(PROMPTS, r)
                    mine.append(r)
                    seen.append(bigrams(prompt))
                kept += 1
            batch += 1
            print(f"{domain[:28]:28} batch {batch:3} kept {kept} pairs, {len(mine)} prompts", flush=True)
    print(f"{len(rows(PROMPTS))} prompts in {PROMPTS.name}")


def stage(args, src, dst, fn, size=1):
    """Apply fn to chunks of rows of src whose id is not yet in dst, at most --limit rows.
    fn returns output rows; rows it leaves out are retried on the next run."""
    done = {r["id"] for r in rows(dst)}
    todo = [r for r in rows(src) if r["id"] not in done][:args.limit]
    n = 0
    for part in judge.chunks(todo, size):
        for out in fn(part):
            append(dst, out)
            n += 1
            print(f"{n}/{len(todo)} {out['id']} {out.get('label', '')}", flush=True)
    print(f"{len(done) + n} rows in {dst.name}")


def label(args, part, prompts):
    got = judge.grade_batch(args, [(prompts[r["id"]], r["answer"]) for r in part])
    return [{"id": r["id"], "label": got[i], "judge": args.judge_model} for i, r in enumerate(part) if i in got]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("stage", choices=["generate", "answer", "label"])
    judge.add_args(ap)
    ap.add_argument("--gen-base", default=judge.OLLAMA)
    ap.add_argument("--gen-model", default="qwen2.5-coder:7b")
    ap.add_argument("--pairs", type=int, default=750, help="total easy/hard pairs (default: %(default)s)")
    ap.add_argument("--batch", type=int, default=16, help="pairs per generator call")
    ap.add_argument("--limit", type=int, default=10**9, help="max calls or rows this run")
    args = ap.parse_args()
    judge.PACE[0] = args.pace
    if args.stage == "generate":
        generate(args)
    elif args.stage == "answer":
        stage(args, PROMPTS, ANSWERS, lambda p: [{"id": r["id"], "answer": judge.answer(args, r["prompt"])} for r in p])
    else:
        prompts = {r["id"]: r["prompt"] for r in rows(PROMPTS)}
        stage(args, ANSWERS, LABELS, lambda p: label(args, p, prompts), args.judge_batch)


def self_check():
    assert parse_pairs('{"easy": "a", "hard": "b"},\nnoise\n{"easy": ""}\n{bad') == [("a", "b")]
    assert parse_pairs('```json\n{\n  "easy": "a",\n  "hard": "b"\n}\n```') == [("a", "b")]
    assert overlaps("How do I exit vim?", [bigrams("how do i exit vim")])
    assert not overlaps("What is TLS?", [bigrams("How do I exit vim?")])


if __name__ == "__main__":
    self_check()
    sys.exit(main())
