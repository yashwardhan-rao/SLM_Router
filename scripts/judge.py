#!/usr/bin/env python3
"""Label prompts by whether the local model's own answer is good enough, and check the judge.

A prompt is `local` when the judge passes llama3.2:1b's answer and `cloud` when it fails it, which
is what "hard" means for this router. Before labelling a corpus, check the judge on dev:

    python scripts/judge.py --calibrate            # judge = qwen2.5-coder:7b on local Ollama
    python scripts/judge.py --calibrate --judge-base https://api.example.com/v1 --judge-model m

The judge grades --judge-batch answers per call, the same way in calibration and labelling.
Cloud calls (any base other than local Ollama) send $JUDGE_API_KEY and reasoning_effort=minimal,
are paced --pace seconds apart; every call backs off on 429, 502 and 503. Agreement below --min-agreement exits 2.
Standard library only; corpus.py imports ask(), answer() and grade_batch() from here.
"""
import argparse
import json
import os
import pathlib
import random
import re
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OLLAMA = "http://127.0.0.1:11434/v1"
PACE = [4.5, 0.0]  # [seconds between cloud calls, time of the last one]
LOOPED = "[no answer: the model looped on a repeated token and was aborted]"
GRADE = """You grade answers from a small assistant model. Apply the rule for the kind of question:
- Creative or open-ended requests (names, short messages, rewording, ideas): PASS if the answer is on-task and reasonable.
- Factual questions and how-to steps: FAIL if any stated fact, number or command is wrong.
- Proofs, derivations, calculations and complexity analyses: FAIL unless every step is correct and justified. A confident sketch that skips or fudges the key argument FAILS.
- Code, queries and refactors: FAIL unless the code would actually work and does everything asked.
- Design, debugging and trade-off questions: FAIL if it misses the main cause or trade-off an expert would name first.
Grade every item below independently. Return one {{"id", "grade"}} entry per item, grade PASS or FAIL.

{items}"""
SCHEMA = {"type": "json_schema", "json_schema": {"name": "grades", "strict": True, "schema": {
    "type": "object", "required": ["grades"], "properties": {"grades": {"type": "array", "items": {
        "type": "object", "required": ["id", "grade"],
        "properties": {"id": {"type": "string"}, "grade": {"type": "string", "enum": ["PASS", "FAIL"]}}}}}}}}


def ask(base, model, prompt, max_tokens, key=None, timeout=300, temperature=0, tries=6, cut_ok=False, schema=None):
    """One chat completion; returns the reply text. Raises on an empty reply, or a truncated one unless cut_ok."""
    cloud = base != OLLAMA
    body = {"model": model, "messages": [{"role": "user", "content": prompt}],
            "max_tokens": max_tokens, "temperature": temperature}
    if schema:
        body["response_format"] = schema
    headers = {"content-type": "application/json"}
    if cloud:
        body["reasoning_effort"] = "minimal"
        if key:
            headers["authorization"] = "Bearer " + key
    req = urllib.request.Request(base.rstrip("/") + "/chat/completions", json.dumps(body).encode(), headers)
    for attempt in range(tries):
        if cloud:
            time.sleep(max(0.0, PACE[1] + PACE[0] - time.monotonic()))
            PACE[1] = time.monotonic()
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                choice = json.load(resp)["choices"][0]
            break
        except urllib.error.HTTPError as e:
            # 429 can also mean the daily quota is gone, so give up after `tries`; every stage resumes.
            if e.code not in (429, 502, 503) or attempt == tries - 1:
                raise
            wait = float(e.headers.get("retry-after") or PACE[0] * 2 ** (attempt + 1))
            print(f"HTTP {e.code}, retrying in {wait:.0f}s", file=sys.stderr, flush=True)
            time.sleep(wait)
    text = choice["message"].get("content") or ""
    if not text.strip() or (choice.get("finish_reason") == "length" and not cut_ok):
        raise RuntimeError(f"empty or truncated reply (finish_reason={choice.get('finish_reason')})")
    return text


def verdicts(reply, n):
    """{index: 'local' | 'cloud'} for each of n items a SCHEMA reply grades; others are left out."""
    m = re.search(r"\{.*\}", reply, re.S)
    try:
        got = json.loads(m.group(0)).get("grades", []) if m else []
    except (json.JSONDecodeError, AttributeError):
        return {}
    out = {}
    for g in got if isinstance(got, list) else ():
        k, v = (str(g.get(x, "")).strip().upper() for x in ("id", "grade")) if isinstance(g, dict) else ("", "")
        if k.isdigit() and int(k) < n and v in ("PASS", "FAIL"):
            out[int(k)] = "local" if v == "PASS" else "cloud"
    return out


def answer(args, prompt):
    """The local model's own answer, capped at --answer-tokens."""
    try:
        return ask(args.local_base, args.local_model, prompt, args.answer_tokens, cut_ok=True)
    except urllib.error.HTTPError as e:
        # Ollama aborts a 1B answer stuck repeating a token; that is a failed answer, not a crash.
        if e.code == 500 and b"repeat limit" in e.read():
            return LOOPED
        raise


def grade_batch(args, pairs):
    """Grade [(prompt, reply), ...] in one judge call; see verdicts() for the result."""
    items = [{"id": str(i), "question": q, "answer": a} for i, (q, a) in enumerate(pairs)]
    reply = ask(args.judge_base, args.judge_model, GRADE.format(items=json.dumps(items, ensure_ascii=False, indent=1)),
                args.judge_tokens, args.judge_key, schema=SCHEMA)
    return verdicts(reply, len(pairs))


def add_args(ap):
    ap.add_argument("--local-base", default=OLLAMA)
    ap.add_argument("--local-model", default="llama3.2:1b")
    ap.add_argument("--judge-base", default=OLLAMA)
    ap.add_argument("--judge-model", default="qwen2.5-coder:7b")
    ap.add_argument("--judge-key", default=os.environ.get("JUDGE_API_KEY"), help="default: $JUDGE_API_KEY")
    ap.add_argument("--judge-batch", type=int, default=12, help="answers graded per judge call")
    ap.add_argument("--judge-tokens", type=int, default=2048, help="cap on each judge reply")
    ap.add_argument("--answer-tokens", type=int, default=384, help="cap on the local answer")
    ap.add_argument("--pace", type=float, default=PACE[0], help="seconds between cloud calls")


def chunks(seq, size):
    return [seq[i:i + size] for i in range(0, len(seq), size)]


def calibrate(args):
    cases = [c for c in json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))
             if c["split"] == "dev"]
    half = args.n // 2
    pick = [c for c in cases if c["expect"] == "local"][args.offset::3][:half] + \
           [c for c in cases if c["expect"] == "cloud"][args.offset::3][:args.n - half]
    random.Random(5).shuffle(pick)  # mix easy and hard inside each judge batch
    replies = [answer(args, c["prompt"]) for c in pick]
    agree = graded = 0
    for part in chunks(list(zip(pick, replies)), args.judge_batch):
        got = grade_batch(args, [(c["prompt"], r) for c, r in part])
        for i, (c, _) in enumerate(part):
            g = got.get(i, "-")
            graded += i in got
            agree += g == c["expect"]
            print(f"{c['id']:4} expect {c['expect']:5} judged {g:5} {'ok' if g == c['expect'] else 'DISAGREE'}",
                  flush=True)
    rate = agree / len(pick)
    print(f"agreement {agree}/{len(pick)} = {rate:.2f} ({graded} graded, min {args.min_agreement})")
    return 0 if rate >= args.min_agreement else 2


def self_check():
    g = [{"id": "0", "grade": "PASS"}, {"id": "1", "grade": "fail"}, {"id": "2", "grade": "?"}, {"id": "9"}, 3]
    assert verdicts("```json\n" + json.dumps({"grades": g}) + "\n```", 3) == {0: "local", 1: "cloud"}
    assert verdicts("no json", 2) == {} and verdicts("{bad", 2) == {} and verdicts('{"0": "PASS"}', 1) == {}
    assert chunks([1, 2, 3], 2) == [[1, 2], [3]]


if __name__ == "__main__":
    self_check()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_args(ap)
    ap.add_argument("--calibrate", action="store_true", help="grade a balanced slice of dev")
    ap.add_argument("--n", type=int, default=30, help="calibration prompts (default: %(default)s)")
    ap.add_argument("--offset", type=int, default=0, help="slice offset (0-2) into dev for calibration")
    ap.add_argument("--min-agreement", type=float, default=0.8)
    a = ap.parse_args()
    PACE[0] = a.pace
    sys.exit(calibrate(a) if a.calibrate else ap.print_help())
