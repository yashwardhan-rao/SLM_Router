#!/usr/bin/env python3
"""Python twin of src/tokens.rs, src/routing/complexity.rs (cue score) and src/routing/features.rs.

The trainer imports this so its features match the router's bit for bit; the Rust test
`matches_the_python_featurizer` checks it against tests/fixtures/featurizer_parity.json.

    python scripts/features.py --parity > tests/fixtures/featurizer_parity.json
"""
import json
import math
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUCKETS = 1 << 16
CODE_MARKERS = ["{", ";", "=>", "->", "::", "def ", "fn ", "#include", "==", "!="]


def _cues(name):
    src = (ROOT / "src" / "routing" / "complexity.rs").read_text("utf-8")
    csv = re.search(rf'const {name}: &str = "([^"]*)";', src).group(1)
    return [c.strip() for c in csv.split(",")]


REASONING, HINTS = _cues("REASONING"), _cues("HINTS")


def estimate(text):
    """src/tokens.rs::estimate"""
    n, prev, i = 0, "\0", 0
    while i < len(text):
        c = text[i]
        if c.isascii() and c.isalnum():
            digits, j = c.isdigit(), i + 1
            while j < len(text) and text[j].isascii() and text[j].isalnum() and text[j].isdigit() == digits:
                j += 1
            run = j - i
            n += (-(-run // 3) + (prev == " ")) if digits else 1 + (run - 1) // 6
            prev, i = "a", j
            continue
        if not c.isspace():
            n += 1
        prev, i = c, i + 1
    return n


# shortcut: Python isalnum/lower match Rust's for ASCII, Latin and CJK, not every Unicode mark;
# extend the parity fixture if non-Latin scripts matter.
def words(text):
    """complexity::words: lowercase, split on anything that is not a letter or digit."""
    return "".join(c if c.isalnum() else " " for c in text.lower()).split()


def has_cue(ws, cue):
    cw = words(cue)
    if not cw:
        return False
    head, last = cw[:-1], cw[-1]
    for i in range(len(ws) - len(cw) + 1):
        w = ws[i:i + len(cw)]
        if w[:-1] == head and (w[-1] == last or (w[-1].endswith("s") and w[-1][:-1] == last)):
            return True
    return False


def big_o(lower):
    for p in ("o(n", "o(log", "o(1)"):
        for m in re.finditer(re.escape(p), lower):
            if m.start() == 0 or not lower[m.start() - 1].isalnum():
                return True
    return False


def _ident(b, mid, needs_after):
    return any(chr(b[i]).isascii() and chr(b[i]).isalnum() and b[i + 1] == mid
               and (not needs_after or (chr(b[i + 2]).isascii() and chr(b[i + 2]).isalnum()))
               for i in range(len(b) - 2))


def is_code(text):
    b = text.encode("utf-8")
    markers = sum(m in text for m in CODE_MARKERS) + _ident(b, ord("_"), True) + _ident(b, ord("("), False)
    return "`" in text or markers >= 2


def cue_score(text):
    """complexity::score with the default config (cue_min_chars = 0)."""
    if not text:
        return 0
    ws = words(text)
    reasoning = any(has_cue(ws, c) for c in REASONING)
    hints = sum(has_cue(ws, c) for c in HINTS)
    return 2 * reasoning + min(hints, 2) + is_code(text) + big_o(text.lower())


def fnv1a(data):
    h = 0x811C9DC5
    for byte in data:
        h = ((h ^ byte) * 0x01000193) & 0xFFFFFFFF
    return h


def buckets(text):
    """Sorted unique active buckets: word unigrams, bigrams, char 3..=5-grams of ' word '."""
    out, ws = set(), words(text)
    for i, w in enumerate(ws):
        out.add(fnv1a(b"w:" + w.encode()) % BUCKETS)
        if i + 1 < len(ws):
            out.add(fnv1a(b"b:" + w.encode() + b" " + ws[i + 1].encode()) % BUCKETS)
        padded = f" {w} "
        for n in (3, 4, 5):
            for j in range(len(padded) - n + 1):
                out.add(fnv1a(b"c:" + padded[j:j + n].encode()) % BUCKETS)
    return sorted(out)


def dense(text, est_tokens, cue):
    return [float(is_code(text)), float(big_o(text.lower())), min(cue, 4) / 4,
            min(math.log2(max(est_tokens, 1)), 12) / 12]


def featurize(text):
    """(buckets, dense) exactly as the router computes them for this last user message."""
    est, cue = estimate(text), cue_score(text)
    return buckets(text), dense(text, est, cue)


PARITY = [
    "", "  ?! ", "Lock-Free queues", "why why why", "Prove that the halting problem is undecidable.",
    "`x` is O(n)", "Refactor this step-by-step: def f(xs): return [x*2 for x in xs]",
    "fn main() { println!(\"hi\"); }", "snake_case_name == other", "What's 1234567 + 89?",
    "Crashes, panics and race conditions!", "Résumé tips for a café job", "東京の天気は？",
    "How would you shard a 50k QPS proxy across TCP clusters?", "O(log n) vs o(n) vs foo(n)",
    "Show that the sum of the first n odd numbers is n^2.", "e1 e2 e3 a_b c(d",
]


def parity():
    cases = json.loads((ROOT / "tests" / "fixtures" / "routing_eval.json").read_text("utf-8"))
    texts = PARITY + [c["prompt"] for c in cases if c["split"] == "dev"][::7]
    rows = []
    for t in texts:
        est, cue = estimate(t), cue_score(t)
        rows.append({"text": t, "est_tokens": est, "cue_score": cue, "buckets": buckets(t),
                     "dense": [round(x, 7) for x in dense(t, est, cue)]})
    return "[\n" + ",\n".join(json.dumps(r, ensure_ascii=False) for r in rows) + "\n]\n"


if __name__ == "__main__":
    if sys.argv[1:] == ["--parity"]:
        sys.stdout.reconfigure(encoding="utf-8", newline="\n")
        sys.stdout.write(parity())
    else:
        sys.exit(__doc__)
