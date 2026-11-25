#!/usr/bin/env python3
"""Label prompts by whether the local model's own answer is good enough, and check the judge.

A prompt is `local` when the judge passes llama3.2:1b's answer and `cloud` when it fails it, which
is what "hard" means for this router. Before labelling a corpus, check the judge on dev:

    python scripts/judge.py --calibrate            # judge = qwen2.5-coder:7b on local Ollama
    python scripts/judge.py --calibrate --judge-base https://api.example.com/v1 --judge-model m

Agreement below --min-agreement (0.8) exits 2: get a stronger judge before labelling.
Standard library only; corpus.py imports ask() and label() from here.
"""
import argparse
import json
import os
import pathlib
import re
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OLLAMA = "http://127.0.0.1:11434/v1"
GRADE = """You grade answers from a small assistant model. Apply the rule for the kind of question:
- Creative or open-ended requests (names, short messages, rewording, ideas): PASS if the answer is on-task and reasonable.
- Factual questions and how-to steps: FAIL if any stated fact, number or command is wrong.
- Proofs, derivations, calculations and complexity analyses: FAIL unless every step is correct and justified. A confident sketch that skips or fudges the key argument FAILS.
- Code, queries and refactors: FAIL unless the code would actually work and does everything asked.
- Design, debugging and trade-off questions: FAIL if it misses the main cause or trade-off an expert would name first.
Reply with exactly one word, PASS or FAIL.

Question:
{q}

Answer:
{a}"""


def ask(base, model, prompt, max_tokens, key=None, timeout=300, temperature=0):
    """One chat completion; returns the reply text."""
    body = {"model": model, "messages": [{"role": "user", "content": prompt}],
            "max_tokens": max_tokens, "temperature": temperature}
    headers = {"content-type": "application/json"}
    if key:
        headers["authorization"] = "Bearer " + key
    req = urllib.request.Request(base.rstrip("/") + "/chat/completions", json.dumps(body).encode(), headers)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.load(resp)["choices"][0]["message"].get("content") or ""


def verdict(text):
    """True for PASS, False for FAIL; the first of the two words wins, anything else fails."""
    m = re.search(r"\b(PASS|FAIL)\b", text.upper())
    return bool(m) and m.group(1) == "PASS"


def answer(args, prompt):
    """The local model's own answer, capped at --answer-tokens."""
    return ask(args.local_base, args.local_model, prompt, args.answer_tokens)


def grade(args, prompt, reply):
    """'local' if the judge passes the local model's reply, else 'cloud'."""
    graded = ask(args.judge_base, args.judge_model, GRADE.format(q=prompt, a=reply), 8, args.judge_key)
    return "local" if verdict(graded) else "cloud"


def label(args, prompt):
    """('local' | 'cloud', local answer)."""
    reply = answer(args, prompt)
    return grade(args, prompt, reply), reply


def add_args(ap):
    ap.add_argument("--local-base", default=OLLAMA)
    ap.add_argument("--local-model", default="llama3.2:1b")
    ap.add_argument("--judge-base", default=OLLAMA)
    ap.add_argument("--judge-model", default="qwen2.5-coder:7b")
    ap.add_argument("--judge-key", default=os.environ.get("JUDGE_API_KEY"), help="default: $JUDGE_API_KEY")
    ap.add_argument("--answer-tokens", type=int, default=384, help="cap on the local answer")


def calibrate(args):
    cases = [c for c in json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))
             if c["split"] == "dev"]
    half = args.n // 2
    pick = [c for c in cases if c["expect"] == "local"][args.offset::3][:half] + \
           [c for c in cases if c["expect"] == "cloud"][args.offset::3][:args.n - half]
    agree = 0
    for c in pick:
        got, _ = label(args, c["prompt"])
        agree += got == c["expect"]
        print(f"{c['id']:4} expect {c['expect']:5} judged {got:5} {'ok' if got == c['expect'] else 'DISAGREE'}",
              flush=True)
    rate = agree / len(pick)
    print(f"agreement {agree}/{len(pick)} = {rate:.2f} (min {args.min_agreement})")
    return 0 if rate >= args.min_agreement else 2


def self_check():
    assert verdict("PASS") and verdict("pass.") and verdict("Verdict: PASS")
    assert not verdict("FAIL") and not verdict("FAIL, not PASS") and not verdict("") and not verdict("passable")


if __name__ == "__main__":
    self_check()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_args(ap)
    ap.add_argument("--calibrate", action="store_true", help="grade a balanced slice of dev")
    ap.add_argument("--n", type=int, default=30, help="calibration prompts (default: %(default)s)")
    ap.add_argument("--offset", type=int, default=0, help="slice offset (0-2) into dev for calibration")
    ap.add_argument("--min-agreement", type=float, default=0.8)
    a = ap.parse_args()
    sys.exit(calibrate(a) if a.calibrate else ap.print_help())
