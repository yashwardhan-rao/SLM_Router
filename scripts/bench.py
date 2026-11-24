#!/usr/bin/env python3
"""Send up to 20 mixed requests (15 with --skip-complex) to a running slm-router, to generate real
`slm-router stats` numbers.

Standard library only. Start the router first (`slm-router serve --warmup`), then:

    python scripts/bench.py                 # auto routing: complex prompts may call the CLOUD (costs money)
    python scripts/bench.py --local-only    # forbid the cloud for every request
    python scripts/bench.py --skip-complex  # auto routing without the prompts that go to the cloud
    python scripts/bench.py --ab --log slm-router.jsonl
                                            # A/B: 5-turn session with one X-Session-Id vs unique ids;
                                            # set [pricing] in the config to get saved_usd (README table)

Afterwards run `slm-router stats` to see the routing split, latency and savings.
"""
import argparse
import json
import os
import sys
import time
import uuid
import urllib.error
import urllib.request

SIMPLE = [
    "What is the capital of France?",
    "Give me three synonyms for 'quick'.",
    "Convert 72 degrees Fahrenheit to Celsius.",
    "Write a one-line commit message for fixing a typo in the README.",
    "Is 91 a prime number? Answer yes or no.",
    "Translate 'good morning' to Spanish.",
]

# Each is over 200 characters and contains a reasoning cue, so auto routing sends it to the cloud.
COMPLEX = [
    "Analyze the trade-offs between optimistic and pessimistic locking for a ticket-booking service that "
    "sees bursts of 5,000 concurrent buyers, and explain step by step which one you would choose and why.",
    "Prove that the sum of the first n odd numbers equals n squared, and then derive the closed form for "
    "the sum of the first n cubes, showing each step of the induction and the algebra clearly.",
    "Refactor this design: a single 2,000-line service class handles authentication, billing, email and "
    "reporting. Describe, step by step, how you would split it and in which order you would do the work.",
    "Analyze why a hash map with a poor hash function degrades to linear time, derive the expected number "
    "of probes under load factor 0.9 with linear probing, and explain what changes with double hashing.",
    "Explain step by step how a distributed lock based on a lease can still allow two writers, analyze the "
    "role of clock drift and long pauses, and derive a fencing-token scheme that prevents the problem.",
]

TURNS = [
    "I'm planning a small vegetable garden. What should I plant first?",
    "I only get about four hours of sun a day. Does that change it?",
    "Which of those are easiest for a beginner?",
    "How often should I water them?",
    "Thanks. Give me a two-line summary.",
]

COMPLEX = [p + " Be thorough and show your reasoning." for p in COMPLEX]
assert all(len(p) > 200 for p in COMPLEX), "complex prompts must exceed the 200-char cue threshold"


def post(base, key, body, headers, stream=False, timeout=120):
    """Return (status, seconds_to_first_byte, seconds_total, model, detail)."""
    req = urllib.request.Request(
        base + "/chat/completions",
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json", **headers},
    )
    if key:
        req.add_header("authorization", "Bearer " + key)
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if stream:
                events, first = 0, None
                for line in resp:
                    if line.startswith(b"data:"):
                        first = first or time.perf_counter() - start
                        events += 1
                return resp.status, first, time.perf_counter() - start, "-", "%d events" % events
            data = json.load(resp)
            took = time.perf_counter() - start
            # Ollama reports how much of the prompt came from its KV cache; show it when present.
            usage = data.get("usage") or {}
            cached = (usage.get("prompt_tokens_details") or {}).get("cached_tokens")
            detail = "" if cached is None else "prompt %s cached %s" % (usage.get("prompt_tokens", "?"), cached)
            return resp.status, took, took, data.get("model", "?"), detail
    except urllib.error.HTTPError as e:
        took = time.perf_counter() - start
        return e.code, took, took, "-", e.read(200).decode(errors="replace").strip()


def chat(content, **extra):
    return {"model": "slm-router/auto", "messages": [{"role": "user", "content": content}], **extra}


def build_plan(skip_complex=False):
    plan = [("simple", chat(p), {}, False) for p in SIMPLE]
    history = []
    for turn in TURNS:  # one session: every turn after the first should be pinned
        history.append({"role": "user", "content": turn})
        body = {"model": "slm-router/auto", "messages": list(history)}
        plan.append(("pinned", body, {"x-session-id": "bench-pin"}, False))
        history.append({"role": "assistant", "content": "(earlier answer)"})
    for p in SIMPLE[:2]:
        plan.append(("stream-auto", chat(p, stream=True), {}, True))
    for p in SIMPLE[2:4]:
        plan.append(("stream-local", chat(p, stream=True, model="slm-router/local"), {}, True))
    if not skip_complex:
        plan += [("complex", chat(p), {}, False) for p in COMPLEX]
    return plan


def ab_arm(base, key, arm, timeout):
    """One 5-turn conversation, real replies fed back. Returns (session id prefix, follow-up cached, prompt)."""
    nonce = uuid.uuid4().hex[:8]  # keeps the first turn from hitting the other arm's Ollama cache
    hist, rows = [], []
    for i, turn in enumerate(TURNS):
        hist.append({"role": "user", "content": ("[%s] " % nonce if i == 0 else "") + turn})
        sid = "ab-%s-%s" % (nonce, "fixed" if arm == "pinned" else "u%d" % i)
        body = {"model": "slm-router/auto", "messages": list(hist), "max_tokens": 120}
        req = urllib.request.Request(base + "/chat/completions", data=json.dumps(body).encode(),
                                     headers={"content-type": "application/json", "x-session-id": sid})
        if key:
            req.add_header("authorization", "Bearer " + key)
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            d = json.load(resp)
        u = d["usage"]
        rows.append((u["prompt_tokens"], (u.get("prompt_tokens_details") or {}).get("cached_tokens", 0)))
        hist.append({"role": "assistant", "content": d["choices"][0]["message"]["content"]})
    return "ab-%s-" % nonce, sum(c for _, c in rows[1:]), sum(p for p, _ in rows[1:])


def saved_usd(log, prefix):
    total, n = 0.0, 0
    for line in open(log, encoding="utf-8"):
        r = json.loads(line)
        if str(r.get("session_id", "")).startswith(prefix):
            n += 1
            total += r.get("saved_usd") or 0.0
    return total, n


def run_ab(args):
    print("A/B: 5 turns, same session id vs unique ids  (needs the local model; keep [pricing] set for saved_usd)")
    for arm in ("pinned", "unique"):
        prefix, cached, prompt = ab_arm(args.base, args.key, arm, args.timeout)
        usd = "n/a (pass --log)" if not args.log else "$%.6f over %d logged requests" % saved_usd(args.log, prefix)
        print("%-7s follow-up cache reuse %d of %d (%.0f%%)   saved_usd %s" % (arm, cached, prompt, 100 * cached / prompt, usd))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default="http://127.0.0.1:8080/v1", help="router base URL (default: %(default)s)")
    ap.add_argument("--key", default=os.environ.get("ROUTER_API_KEY"), help="router bearer token (default: $ROUTER_API_KEY)")
    ap.add_argument("--local-only", action="store_true", help="send X-Router-Policy: local-only so nothing reaches the cloud")
    ap.add_argument("--skip-complex", action="store_true", help="leave out the 5 reasoning prompts that auto routing sends to the cloud")
    ap.add_argument("--timeout", type=float, default=120, help="seconds per request (default: %(default)s)")
    ap.add_argument("--ab", action="store_true", help="run the pinned vs unique-session A/B instead of the normal plan")
    ap.add_argument("--log", help="the router's JSONL log, read by --ab to sum saved_usd")
    args = ap.parse_args()
    if args.ab:
        return run_ab(args)

    plan = build_plan(args.skip_complex)
    print("%d requests -> %s%s" % (len(plan), args.base, "  [local-only]" if args.local_only else ""))
    failures = 0
    for i, (kind, body, headers, stream) in enumerate(plan, 1):
        if args.local_only:
            headers = {**headers, "x-router-policy": "local-only"}
        try:
            status, first, total, model, detail = post(args.base, args.key, body, headers, stream, args.timeout)
        except (urllib.error.URLError, OSError) as e:
            sys.exit("cannot reach the router at %s: %s\nStart it with: slm-router serve --warmup" % (args.base, e))
        failures += status != 200
        first = "%.2fs" % first if first is not None else "-"
        print("%2d %-12s HTTP %-3s first byte %-7s total %5.2fs  model=%s %s" % (i, kind, status, first, total, model, detail))
    print("\n%d/%d succeeded. Now run: slm-router stats" % (len(plan) - failures, len(plan)))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
