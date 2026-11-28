#!/usr/bin/env python3
"""Live routing eval: send the labelled prompts in tests/fixtures/routing_eval.json, plus cases only a
live run can check (tools, JSON schema, long context, a 4-turn escalation), through a running
slm-router, then join the router's log to report accuracy, misroutes, latency and saved USD.

Standard library only. Requests alternate /v1/chat/completions and /v1/responses, plain and streaming.

    python scripts/eval.py --log slm-router.jsonl
    python scripts/eval.py --log slm-router.jsonl --drill-base http://127.0.0.1:8081/v1 --drill-log dead.jsonl
                           # also time requests against a second router whose local model is down
    python scripts/eval.py --bench   # offline: baselines vs the release binary on routing_bench.json
"""
import argparse
import json
import math
import os
import pathlib
import time
import urllib.error
import urllib.request
import uuid

FIXTURE = pathlib.Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "routing_eval.json"
LOG_TEXT = "\n".join(
    f"2026-11-24T10:{i // 60:02d}:{i % 60:02d}Z INFO worker-{i % 7} handled job {1000 + i} in {(i * 37) % 900}ms status=ok"
    for i in range(140)) + "\n2026-11-24T12:20:00Z ERROR worker-3 job 1139 failed: connection reset by peer"
PARAMS = {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}
PERSON = {"type": "object", "properties": {"name": {"type": "string"}, "age": {"type": "integer"}},
          "required": ["name", "age"], "additionalProperties": False}
# (id, expect, api, prompt, chat extras, responses extras)
LIVE = [
    ("T1", "cloud", "chat", "What's the weather in Paris right now?",
     {"tools": [{"type": "function", "function": {"name": "get_weather", "parameters": PARAMS}}]}, {}),
    ("T2", "cloud", "resp", "What's the weather in Tokyo right now?",
     {}, {"tools": [{"type": "function", "name": "get_weather", "parameters": PARAMS}]}),
    ("J1", "cloud", "chat", "Extract the person: 'Ada Lovelace was 36 when she died.'",
     {"response_format": {"type": "json_schema", "json_schema": {"name": "person", "schema": PERSON, "strict": True}}}, {}),
    ("J2", "cloud", "resp", "Extract the person: 'Alan Turing was 41 when he died.'",
     {}, {"text": {"format": {"type": "json_schema", "name": "person", "schema": PERSON, "strict": True}}}),
    ("L1", "cloud", "chat", f"Here is a log:\n{LOG_TEXT}\n\nWhich job failed and why?", {}, {}),
    ("L2", "cloud", "resp", f"Here is a log:\n{LOG_TEXT}\n\nHow many distinct workers appear?", {}, {}),
]
CONVO = [("C1", "local", "Hi! What's a Python list?"), ("C2", "local", "How do I append to it?"),
         ("C3", "cloud", "Now refactor this step by step so it runs in O(n) instead of O(n^2), and analyze the "
                         "invariant: `def dedupe(xs):\n    out = []\n    for x in xs:\n        if x not in out:\n"
                         "            out.append(x)\n    return out` Keep the original order."),
         ("C4", "cloud", "Thanks! How do I print it?")]


def send(args, base, api, sid, messages, stream, extra):
    """POST one request; returns (status, seconds, assistant text)."""
    if api == "chat":
        path, body = "/chat/completions", {"model": "slm-router/auto", "messages": messages, "max_tokens": args.max_tokens}
    else:
        path, body = "/responses", {"model": "slm-router/auto", "input": messages, "max_output_tokens": args.max_tokens}
    body.update(extra, stream=stream)
    headers = {"content-type": "application/json", "x-session-id": sid}
    if args.key:
        headers["authorization"] = "Bearer " + args.key
    req = urllib.request.Request(base + path, json.dumps(body).encode(), headers)
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=args.timeout) as resp:
            raw = resp.read().decode("utf-8", "replace")
            status = resp.status
    except urllib.error.HTTPError as e:
        return e.code, time.perf_counter() - start, e.read(200).decode("utf-8", "replace")
    took = time.perf_counter() - start
    if stream:
        return status, took, ""
    d = json.loads(raw)
    text = d["choices"][0]["message"].get("content") if api == "chat" else d.get("output_text", "")
    return status, took, text or ""


def plan(split):
    rows = [(c["id"], c["split"], c["expect"], c["prompt"], {}, {}) for c in json.loads(FIXTURE.read_text("utf-8"))
            if split in ("all", c["split"])]
    return rows + [(i, "live", e, p, ce, re) for i, e, _, p, ce, re in LIVE]


def read_log(path, prefix, want):
    for _ in range(50):  # streamed requests are logged once the stream ends
        recs = {}
        for line in open(path, encoding="utf-8"):
            r = json.loads(line)
            if r.get("session_id", "").startswith(prefix):
                recs.setdefault(r["session_id"], []).append(r)
        if len(recs) >= want:
            return recs
        time.sleep(0.1)
    return recs


def pct(xs, p):
    xs = sorted(xs)
    return xs[max(0, math.ceil(p * len(xs)) - 1)] if xs else 0


def run(args):
    run_id = uuid.uuid4().hex[:6]
    prefix, sent = f"eval-{run_id}-", []
    for n, (cid, split, expect, prompt, chat_x, resp_x) in enumerate(plan(args.split)):
        live = next((x for x in LIVE if x[0] == cid), None)
        api = live[2] if live else ("chat", "resp")[n % 2]
        stream = (n // 2) % 2 == 1 and not (chat_x or resp_x)
        sid = prefix + cid
        status, took, _ = send(args, args.base, api, sid, [{"role": "user", "content": prompt}], stream,
                               chat_x if api == "chat" else resp_x)
        sent.append((cid, split, expect, sid))
        print(f"{cid:4} {split:7} {api:4} {'stream' if stream else 'plain':6} HTTP {status} {took:6.2f}s", flush=True)
    history = []
    for cid, expect, prompt in CONVO:  # one session: C3 escalates, C4 must stay pinned to cloud
        history.append({"role": "user", "content": prompt})
        status, took, text = send(args, args.base, "chat", prefix + "convo", history, False, {})
        history.append({"role": "assistant", "content": text or "(no reply)"})
        sent.append((cid, "convo", expect, prefix + "convo"))
        print(f"{cid:4} convo   chat plain  HTTP {status} {took:6.2f}s", flush=True)
    report(read_log(args.log, prefix, len({s for *_, s in sent})), sent)


def report(recs, sent):
    convo_i, rows = 0, []
    for cid, split, expect, sid in sent:
        lines = recs.get(sid, [])
        r = lines[convo_i] if split == "convo" and convo_i < len(lines) else (lines[0] if lines else None)
        convo_i += split == "convo"
        rows.append((cid, split, expect, r))
    print("\nsplit    correct  false-cloud  missed-hard")
    for split in ("dev", "holdout", "live", "convo"):
        rs = [x for x in rows if x[1] == split and x[3]]
        if rs:
            ok = sum(r["target"] == e for _, _, e, r in rs)
            fc = sum(e == "local" and r["target"] == "cloud" for _, _, e, r in rs)
            mh = sum(e == "cloud" and r["target"] == "local" for _, _, e, r in rs)
            print(f"{split:8} {ok:3}/{len(rs):<4} {fc:11}  {mh:11}")
    print("\nmisroutes:")
    for cid, split, e, r in rows:
        if r is None:
            print(f"  {cid}: no log line found")
        elif r["target"] != e:
            print(f"  {cid} ({split}) expected {e}, got {r['target']} reason={r['reason']}")
    for t in ("local", "cloud"):
        ms = [r["total_ms"] for *_, r in rows if r and r["target"] == t]
        print(f"latency_ms p50/p95 {t}: {pct(ms, 0.5)}/{pct(ms, 0.95)}  (n={len(ms)})")
    saved = sum(r.get("saved_usd") or 0 for *_, r in rows if r)
    # A hard prompt answered by the small model is not a real saving, so it is left out here.
    real = sum(r.get("saved_usd") or 0 for _, _, e, r in rows if r and e == "local" and r["target"] == "local")
    print(f"saved_usd: {saved:.6f}   misroute-adjusted: {real:.6f}")
    odd = [r["reason"] for *_, r in rows if r and r["reason"] in ("local_unavailable", "circuit_open")]
    if odd:
        print(f"WARNING: {len(odd)} failovers in the main run ({set(odd)}); warm both models and rerun")


def drill(args):
    print(f"\nfailover drill against {args.drill_base} (local should be down)")
    run_id = uuid.uuid4().hex[:6]
    for i in range(args.drill_n):
        status, took, _ = send(args, args.drill_base, "chat", f"drill-{run_id}-{i}",
                               [{"role": "user", "content": f"What is {i} + 2?"}], False, {})
        print(f"  #{i + 1} HTTP {status} {took * 1000:7.0f} ms")
    if args.drill_log:
        for sid, (r, *_) in sorted(read_log(args.drill_log, f"drill-{run_id}-", args.drill_n).items()):
            print(f"  {sid}: target={r['target']} reason={r['reason']} failed_over={r['failed_over']} total_ms={r['total_ms']}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default="http://127.0.0.1:8080/v1", help="router base URL (default: %(default)s)")
    ap.add_argument("--key", default=os.environ.get("ROUTER_API_KEY"), help="router bearer token (default: $ROUTER_API_KEY)")
    ap.add_argument("--bench", action="store_true", help="offline benchmark instead: scripts/routing_bench.py")
    ap.add_argument("--log", help="the router's JSONL log, joined by session id (required unless --bench)")
    ap.add_argument("--split", default="all", choices=["all", "dev", "holdout"], help="fixture split to send")
    ap.add_argument("--max-tokens", type=int, default=128, help="reply cap per request (default: %(default)s)")
    ap.add_argument("--timeout", type=float, default=300, help="seconds per request (default: %(default)s)")
    ap.add_argument("--drill-base", help="a second router whose local model is down")
    ap.add_argument("--drill-log", help="that router's JSONL log")
    ap.add_argument("--drill-n", type=int, default=5, help="drill requests (default: %(default)s)")
    args = ap.parse_args()
    if args.bench:
        import routing_bench
        return routing_bench.main()
    if not args.log:
        ap.error("--log is required unless --bench")
    run(args)
    if args.drill_base:
        drill(args)


if __name__ == "__main__":
    main()
