# Changelog

Notable changes to SLM_Router. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). From 1.0.0 on, the surface listed under Stability in [SPEC.md](SPEC.md) only changes in a major version; before 1.0.0 a minor version could change defaults or config.

## [1.0.0] - 2026-11-28

### Added
- `slm-router report [--since] [--json]` prints the log summary as a Markdown report (overview, latency, reasons, tiers, errors), or with `--json` as JSON (schema version 1) for scripts. It uses the same aggregation as `stats`, so the numbers always match. A missing log gives a zero summary.
- Optional top-level config `version`. Absent or `1` is accepted, and any other value is rejected at startup with a clear message. The example config now sets `version = 1`.
- SPEC lists the stable 1.x surface: the endpoints, config schema 1, log schema 1 (its 21 fields, reasons and outcomes, pinned by a test), `report --json` schema 1, the CLI and `decide()`.

### Changed
- `stats` and `report` skip log lines whose `v` is not 1 instead of misreading them. Lines without `v` are still read as version 1.
- Semantic versioning applies from this release.

## [0.8.0] - 2026-11-28

### Added
- `slm-router init [--path FILE] [--force]` writes the documented example config, which is compiled into the binary. It refuses to overwrite an existing file without `--force`, and writes through a temp file in the same directory so a failed write never leaves half a config.
- `[[tier]]` presets in the example config for local Ollama, Groq, Anthropic's OpenAI-compatible endpoint and OpenAI, with placeholder prices to check against each provider.
- macOS (Apple silicon) binary `slm-router-macos-aarch64` on each release.
- `Dockerfile` (non-root Debian image, `serve --bind 0.0.0.0:8080`) and `contrib/slm-router.service`, an example systemd unit. No image is published.

## [0.7.0] - 2026-11-28

### Added
- `slm-router calibrate --labels FILE`: scores your own prompts labelled `local` or `cloud` with the embedded classifier and suggests a `classifier_threshold` (highest `local` score plus 0.02, the rule the shipped model was calibrated with), with the share of `cloud` prompts rule 5 would then catch. Prompts over `complex_prompt_tokens` are skipped, as rule 5 never scores them. The config is never rewritten.

## [0.6.0] - 2026-11-28

### Added
- N tiers: a `[[tier]]` list of any number of OpenAI-compatible upstreams, cheapest first, each with `local`, `supports_tools`, `max_context_tokens`, `stateful`, `hard`, optional prices, a first-byte timeout and its own circuit breaker. Each request goes to the cheapest tier the policy allows that passes the stateful, capability, context and Rule 5 checks.
- `slm-router/<tier name>` forces a tier; `/v1/models` lists every tier name.
- `tier` field in every log line (the serving tier's name) and a `tiers:` line in `stats`.
- `tier_unavailable` routing reason, for a remote tier that was unreachable or answered 5xx and was failed over.
- Config checks: prices must be >= 0, and a tier sets both prices or neither.
- Startup warnings, never fatal: `[routing] local_supports_tools` / `local_max_context_tokens` set with `[[tier]]` (they are ignored; set `supports_tools` / `max_context_tokens` on each tier), and a cheaper tier priced above the top tier (its `saved_usd` would be negative).

### Changed
- Failover and pins move up the ladder only. A failed-over request logs the first hop's reason.
- `serve --warmup` warms every local tier in parallel.
- `slm-router/local` or `slm-router/cloud` with no matching tier returns 400 `model_not_found` instead of routing automatically.
- The legacy `[local]` + `[cloud]` config keeps working unchanged as a two-tier ladder named `local` and `cloud`; the log's `target` field still says `local` or `cloud`.

## [0.5.0] - 2026-11-26

### Added
- `classifier_score` in every log line: the Rule 5 classifier's probability for the last user message (3 decimals, `null` when the classifier is disabled or the prompt is long enough that rule 5 skips it), so `classifier_threshold` can be tuned on real traffic.
- `stats` reports routing reasons by count, errors by outcome (client disconnects on their own line), and how many requests have estimated token counts.
- `400 invalid_request_error` when a streamed chat request's `stream_options` is not an object.
- `server.request_stream_usage` (default `true`) to turn the usage request off for an upstream that rejects `stream_options`.

### Changed
- Streamed chat requests are sent with `stream_options.include_usage: true`, so token counts and `saved_usd` are exact instead of estimated. The extra usage chunk is hidden from clients that did not ask for it; clients that asked get the stream unchanged.
- README roadmap now runs to v1.0.0 (N tiers, `calibrate`, install tooling, stable schemas), and SPEC's out-of-scope list points to those milestones.

## [0.4.0] - 2026-11-26

### Added
- Embedded Rule 5 classifier: a 64 KB int8 logistic-regression model over hashed word and character n-grams, compiled into the binary. It scores a 2 KB prompt in under 50 µs with no I/O.
- `classifier_threshold` config (default 0.78, calibrated on the dev split only) and a new `classifier` routing reason in the log.
- Training pipeline: `scripts/corpus.py`, `scripts/judge.py` and `scripts/train.py`, plus a judged corpus of 1,808 synthetic prompts in `data/`.
- Parity fixtures that check Rust and Python featurize and score identically.
- Blind holdout v3 with a CI gate: the classifier must raise hard-prompt recall by at least 0.25 over keyword cues alone.

### Changed
- Rule 5 sends a prompt to cloud if it is long, if keyword cues reach `complexity_threshold`, or if the classifier reaches `classifier_threshold`. Hard prompts caught on holdout v3 rose from 4/20 to 16/20, with zero easy prompts sent to cloud.
- **The classifier is on by default**, so some prompts with no cue words now go to cloud. Set `classifier_threshold` above 1.0 to turn it off.

## [0.3.0] - 2026-11-25

### Added
- Local circuit breaker: after `breaker_failures` (2) consecutive unreachable or 5xx replies, local is skipped for `breaker_cooldown_ms` (30 s), then one probe is let through. Skipped auto requests fail over with reason `circuit_open` (counted as failovers in `stats`); forced-local and `local-only` requests get 503 while it is open. `breaker_failures = 0` disables it.
- `scripts/eval.py`: a repeatable live routing eval that joins the router's log by session id.
- Offline routing eval fixture with dev and holdout splits, plus llama3.2 token ground truth.

### Changed
- Token estimate uses symbol density instead of chars/4. It lands within 0.90–1.45x of llama3.2:1b's real counts, compared with 0.33–1.41x before.
- Rule 5 scores the last user message on reasoning, debugging and code signals instead of only checking long messages for reasoning cues. New keys: `complexity_threshold` (default 2) and `hint_cues`; the built-in `reasoning_cues` list grew (adds `show that`, `justify`, `trade off`).
- **Default `cue_min_chars` is now 0** (was 200), so keyword cues also count in short messages; set it back to 200 for the old behaviour.
- The context check keeps 512 tokens free for the reply.
- **Default `local_max_context_tokens` is now 4096** (was 8000), matching what Ollama actually loads for llama3.2:1b.
- **Failovers no longer pin the session to cloud**, so one local blip no longer moves a whole conversation off the local model.

## [0.2.0] - 2026-11-25

### Added
- `POST /v1/responses` passthrough, streaming and non-streaming, through the same routing pipeline as chat.
- Stateful Responses requests (`previous_response_id`, `conversation`, `store: true`, `background: true`, item references) always go to cloud.
- `501 responses_unsupported` when an upstream has no `/v1/responses`.

### Changed
- Routing, failover, pinning and relay are shared between endpoints in `app/dispatch.rs`.

## [0.1.1] - 2026-11-24

### Added
- Release workflow that attaches Linux and Windows binaries to each `v*` tag.
- `scripts/bench.py --ab`: a pinned vs unique-session benchmark.

### Changed
- Pin eviction is amortised O(1) with a deterministic order.

### Fixed
- `ROUTER_API_KEY` is compared in constant time.
- `stats` streams the log line by line instead of reading it whole. A line with invalid UTF-8 now counts as unreadable instead of failing the command.

## [0.1.0] - 2026-11-24

### Added
- Headless OpenAI-compatible proxy: `/v1/chat/completions` (streaming and non-streaming) and `/v1/models`.
- Pure `decide()` routing: forced model or policy, session pin, capability (images, tools, JSON schema), context size, complexity, then local by default.
- `auto`, `local-only` and `cloud-only` policies. Clients can tighten the policy with `X-Router-Policy` but never loosen it.
- Session pinning by `X-Session-Id` or prompt fingerprint, with an idle TTL and a bounded in-memory LRU table. Pins only escalate from local to cloud.
- Failover from local to cloud before the first byte reaches the client.
- Unknown request fields pass through to the upstream untouched.
- Bearer auth, OpenAI-style error bodies, and a configurable `max_body_bytes` (default 64 MiB).
- One metadata-only JSONL log line per request, and `slm-router stats [--since]`.
- `slm-router.example.toml`, CI (fmt, clippy, tests) and `scripts/bench.py`.

[Unreleased]: https://github.com/yashwardhan-rao/SLM_Router/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.8.0...v1.0.0
[0.8.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/yashwardhan-rao/SLM_Router/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/yashwardhan-rao/SLM_Router/releases/tag/v0.1.0
