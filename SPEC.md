# SLM_Router v1 Spec

## Problem Statement

Developers who run a local Ollama model and also pay for a cloud model send every request to the cloud, because switching by hand is tedious and risky. They overpay for simple requests, lose privacy on sensitive ones, and have no record of what a local model could have saved them. Naive per-request routing also thrashes the local model's warm prompt/KV cache: a conversation that bounces between targets pays full prefill cost every turn. Existing gateways are heavy, need a UI or database, or translate between provider formats.

## Solution

SLM_Router v1 is a headless, single-binary, OpenAI-compatible proxy. A client points any OpenAI SDK at it and calls `/v1/chat/completions` as normal. The router picks between a local Ollama model and a cloud model (since v0.6.0, any cost-ordered ladder of OpenAI-compatible tiers), all reached over OpenAI-compatible `/v1`, so there is no provider translation. The choice comes from a small ordered set of cheap heuristics and a privacy policy. A session that has started on a target stays pinned to it so warm caches are reused. Every request appends one metadata-only line to a local JSONL log, and the `stats` and `report` commands report latency and the cloud spend avoided. There is no UI, database, or background service. The whole project is checked by one fast, offline command.

## User Stories

1. As an app developer, I want to point my existing OpenAI SDK at the router by changing only `base_url`, so that I adopt it without code changes.
2. As an app developer, I want unknown request fields passed through to the upstream untouched, so that new OpenAI parameters keep working.
3. As an app developer, I want streaming (`stream: true`) responses relayed as-is, so that chat UIs and IDEs feel responsive. (Since v0.5.0 the one exception is the usage chunk the router requests for itself, hidden from clients that did not ask for it.)
4. As an app developer, I want non-streaming responses to work identically to streaming ones in routing and logging, so that I can choose either.
5. As an app developer, I want `/v1/models` to list `slm-router/auto`, `slm-router/local` and `slm-router/cloud`, so that SDKs and model pickers discover the router.
6. As an app developer, I want to force a target by setting `model` to `slm-router/local` or `slm-router/cloud`, so that I can override routing per request.
7. As an app developer, I want any other `model` value treated as auto, so that apps hard-coded to a vendor model name still work.
8. As an app developer, I want simple short prompts answered by the local model, so that I stop paying for trivial requests.
9. As an app developer, I want long prompts and reasoning-style prompts sent to the cloud model, so that quality holds on hard requests.
10. As an app developer, I want requests with images, tools, `tool_choice` or JSON-schema `response_format` sent to cloud by default, so that small local models are not trusted with unreliable features.
11. As a power user with a capable local model, I want a `local_supports_tools` setting, so that I can keep tool-using requests local.
12. As an app developer, I want prompts that exceed the local context limit routed to cloud, so that requests do not fail on the local model.
13. As an app developer, I want every response to say which model answered, so that I can see the routing from the client side.
14. As an app developer, I want multi-turn conversations to stay on the same target, so that the local KV/prompt cache stays warm and follow-ups are fast.
15. As an app developer, I want to send `X-Session-Id` to define a session explicitly, so that pinning is exact.
16. As an app developer, I want a session derived automatically from the system prompt and first user message when I send no header, so that pinning works with unmodified clients.
17. As an app developer, I want a conversation that outgrows the local model to escalate to cloud and stay there, so that I never bounce back to a cold cache.
18. As an app developer, I want a session never to move from cloud back to local, so that quality does not silently degrade mid-conversation.
19. As an app developer, I want pins to expire after a configurable idle TTL, so that they match how long Ollama keeps the model resident.
20. As an operator, I want the pin table bounded in size with LRU eviction, so that memory cannot grow without limit.
21. As an operator, I want pins held only in memory, so that there is no state to migrate or leak.
22. As a privacy-conscious user, I want a `local-only` policy that forbids all cloud calls, so that sensitive data never leaves my machine.
23. As a privacy-conscious user, I want a 503 in the OpenAI error shape when local-only is set and local fails, so that nothing leaks to cloud as a fallback.
24. As a cost-conscious user, I want a `cloud-only` policy, so that I can bypass the local model entirely when I need to.
25. As an operator, I want a config default policy that clients can only tighten via `X-Router-Policy`, never loosen, so that a client cannot break my privacy rule.
26. As an app developer, I want a 400 when my forced model conflicts with the effective policy, so that I learn about the conflict instead of getting a surprise route.
27. As an app developer, I want a request to fail over to cloud automatically when Ollama is down, refuses connections, returns a 5xx, or sends no first byte in time, so that my app stays available.
28. As an operator, I want `local_first_byte_timeout_ms` configurable with a default of 15000, so that cold model loads do not trigger false failovers.
29. As an operator, I want connection-refused to fail over immediately, so that "Ollama isn't running" costs no waiting.
30. As an operator, I want failover only before any bytes reach the client, so that responses are never spliced from two models.
31. As an app developer, I want cloud 429 and 5xx errors passed through with their original status and body, so that my existing retry logic works.
32. As an operator, I want no automatic fallback from cloud to local, so that quality never downgrades silently.
33. As an app developer, I want a mid-stream upstream failure to end the stream with an OpenAI-style error event, so that my client can detect and handle it.
34. As an operator, I want a client disconnect to cancel the upstream request immediately, so that I do not burn tokens or GPU on abandoned requests.
35. As an operator, I want partial token counts logged for disconnected and failed streams, so that the stats stay honest.
36. As an operator, I want the proxy bound to `127.0.0.1` by default, so that I do not expose an open LLM gateway by accident.
37. As an operator, I want the router to refuse to start on a non-loopback address unless `ROUTER_API_KEY` is set, so that a network-reachable router is never unauthenticated.
38. As an operator, I want an optional bearer token for clients of the router, so that I can share it on a trusted network.
39. As an operator, I want cloud API keys read only from environment variables and never forwarded from client headers, so that secrets stay server-side.
40. As an operator, I want one TOML config file located by `--config`, `ROUTER_CONFIG`, or `./slm-router.toml`, so that setup is one file.
41. As an operator, I want a missing or invalid config to exit non-zero with a clear message, so that misconfiguration is caught at startup, not at the first request.
42. As an operator, I want routing thresholds and the reasoning-cue list in config, so that I can tune routing without a rebuild.
43. As an operator, I want a `serve --warmup` flag that sends one tiny request to the local model at startup, so that the first real request is not the cold load.
44. As a user, I want one JSONL line per request with timestamp, request id, session id and source, policy, target, reason, failover and pin flags, so that every routing decision is explainable.
45. As a user, I want TTFB and total latency in each line, so that I can compare local and cloud speed.
46. As a user, I want prompt and completion token counts, flagged when estimated, so that I know how much to trust the numbers.
47. As a user, I want a `saved_usd` figure per request, calculated as what the cloud model would have cost, so that I can see the savings.
48. As a user, I want `saved_usd` left null when no pricing is configured, so that I am never shown invented numbers.
49. As a user, I want `slm-router stats` to print request counts by target, failovers, percent local, tokens kept local, total saved and p50/p95 latency per target, so that I get the picture in one command.
50. As a user, I want `stats --since <duration>`, so that I can look at today or this week.
51. As a privacy-conscious user, I want logs to contain metadata only, never prompt or completion text or keys, so that the log is safe to keep and share.
52. As an operator, I want a log-write failure to warn on stderr without failing the request, so that logging problems never take down traffic.
53. As an operator, I want log lines versioned with `v: 1`, so that the schema can evolve.
54. As a contributor, I want one command (`cargo clippy --all-targets -- -D warnings && cargo test`) that runs offline in seconds, so that I can verify every change.
55. As a contributor, I want the routing decision as a pure function with no I/O, so that every rule is unit-testable.
56. As a contributor, I want every file under 200 lines, so that the codebase stays navigable for people and agents.
57. As a developer embedding the logic, I want `decide()` importable as a library, so that I can reuse the routing without the HTTP server.

## Implementation Decisions

**Stack**
- Rust. axum and tokio for serving, reqwest for upstream calls, serde for typed request/response boundaries, clap for the CLI. No provider SDKs, no LiteLLM.
- Both upstreams are OpenAI-compatible `/v1/chat/completions`: Ollama's OpenAI endpoint and any OpenAI-compatible cloud endpoint (OpenAI, Groq, or Anthropic's compatibility endpoint). No request or response translation.
- Unknown request fields are preserved and forwarded. Known fields the router reads (model, messages, stream, tools, tool_choice, response_format) are typed.

**Tiers (v0.6.0)**
- `[[tier]]` entries form a ladder in config order, cheapest first; it is never re-sorted. Each tier has a name, base URL, model, optional key env var, and flags: `local` (on this machine), `supports_tools`, `max_context_tokens`, `stateful` (keeps `/v1/responses` state), `hard` (may take prompts Rule 5 calls hard), optional input/output prices (both or neither, >= 0), first-byte timeout (default 15000 for local tiers, none otherwise) and its own circuit breaker (`breaker_failures` default 2, 0 disables; `breaker_cooldown_ms` default 30000).
- The legacy `[local]` + `[cloud]` form desugars to a two-tier ladder named `local` and `cloud` with v0.5.0 behaviour: cloud supports tools, any context, stateful and hard, uses `[pricing]`, and has no breaker. The two forms cannot be mixed.
- Routing picks the cheapest tier the policy allows (`local-only`: local tiers, `cloud-only`: the rest) that passes, in order, the stateful, capability, context and Rule 5 checks. The logged reason is the first check the cheapest allowed tier fails. If no tier passes all, Rule 5 is dropped and then capability, taking the highest tier left; if none can serve the stateful and context needs, 400. A policy that allows no tier is a policy conflict, and one that allows exactly one tier logs `policy`.
- `slm-router/<name>` forces a tier. `slm-router/local` and `slm-router/cloud` force the cheapest local or non-local tier, unless a tier has that name, and return 400 `model_not_found` when there is no such tier. `/v1/models` lists `auto`, the aliases that resolve, and every tier name.
- Pins store a tier and only move up. A request below its pin goes to the cheapest tier at or above the pin that can serve it, or ignores the pin when none can.
- Failover (not for forced requests) walks up to the next allowed tier that can serve the request, preferring one with the needed capabilities. The logged reason is the first hop's: `circuit_open`, `local_unavailable` (local tier unreachable or 5xx) or `tier_unavailable` (remote tier). With nothing left above, an open circuit or a dead local tier returns 503; a remote tier's own reply is relayed, or 502 if unreachable.
- Saved USD is the top tier's cost minus the serving tier's. An unpriced local tier costs zero; an unpriced remote tier, or an unpriced top tier, gives null.

**Targets and model field**
- The original two targets, `local` and `cloud`, each with a base URL and a model name from config, are since v0.6.0 the legacy form of a tier ladder; see **Tiers** above. Both forms are supported in 1.x.
- The client's `model` is rewritten to the chosen target's model. Reserved values: `slm-router/local`, `slm-router/cloud`, `slm-router/auto`. Any other value means auto. The model list endpoint returns `slm-router/auto`, the aliases that resolve, and every tier name.

**Routing core (pure, no I/O)**
- One function takes request features, effective policy and session state and returns a decision (target plus a named reason). Rules are ordered, first match wins:
  1. Forced target (model field or policy). The stricter of the two wins; a conflict returns 400.
  2. Existing pin, with one-way local-to-cloud escalation only.
  3. Capability: images, tools, tool_choice, or JSON-schema response format go to cloud unless `local_supports_tools` is set.
  4. Context: estimated tokens above `local_max_context_tokens` go to cloud.
  5. Complexity: cloud if estimated prompt tokens exceed `complex_prompt_tokens` (default 1500), or the last user message scores at least `complexity_threshold` (default 2): +2 for any reasoning cue, +1 per distinct hint cue (at most +2), +1 for code, +1 for big-O. Cues match whole words, both cue lists are configurable, and they only count in a message longer than `cue_min_chars`. Failing that, cloud with reason `classifier` if the Rule 5 classifier gives the last user message a probability of at least `classifier_threshold` (default: the value calibrated into the model; above 1.0 disables it).
  6. Default: local.
- Token estimate is by symbol density (letter runs, digit runs, symbols) plus 4 per message, documented as approximate. No tokenizer dependency. Rule 4 and the forced-local context check keep 512 tokens free for the reply; `local_max_context_tokens` defaults to 4096.
- The reason string is the rule name and is what gets logged.

**Rule 5 classifier**
- L2 logistic regression trained offline by `scripts/train.py` on synthetic prompts, each labelled by a judge model on whether the local model's own answer was good enough. It is embedded in the binary (`assets/rule5.bin`) and decoded once, so scoring does no I/O.
- Features of the last user message, identical in `src/routing/features.rs` and `scripts/features.py` (pinned by parity fixtures):
  - Words: lowercase, split on anything that is not a letter or digit.
  - Sparse: word unigrams (`w:`+word), word bigrams (`b:`+w1+` `+w2) and char 3- to 5-grams of ` `+word+` ` (`c:`+gram), each hashed with FNV-1a 32 into 2^16 buckets and counted once. The sparse dot product is divided by sqrt(active buckets), and is 0 when there are none.
  - Dense: `is_code`, `big_o`, `min(cue_score, 4) / 4`, `min(log2(max(tokens, 1)), 12) / 12`, where tokens is the last message's estimate + 4, never the conversation's.
- Probability: `sigmoid(bias + sparse + dense . w_dense)`.
- Binary format, little-endian, 65,574 bytes: magic `SLR5`, u16 version (1), u32 bucket count (65536), f32 sparse scale, f32 bias, f32 calibrated threshold, 4 x f32 dense weights, then 65536 x i8 sparse weights (weight = i8 x scale).
- The threshold is calibrated on the dev split only: the lowest value that sends no easy prompt to cloud, plus 0.02. The holdout split is never used to train or tune.
- `slm-router calibrate --labels FILE` (v0.7.0) applies the same rule to the user's own prompts. Input is JSONL `{"prompt", "label": "local" | "cloud"}`, other fields ignored, blank lines skipped; a malformed line or unknown label is an error naming its 1-based line. Each prompt is scored as a one-message request, exactly as the log's `classifier_score` (even when the configured threshold disables the classifier). Prompts over `complex_prompt_tokens` are skipped and counted. The suggestion is the highest `local` score plus 0.02, rounded up to 3 decimals; no scored `local` row is an error. It prints the share of `cloud` rows rule 5 catches at that threshold (cues or classifier), notes when the value is above 1.0 (disables the classifier), and warns under 20 `local` rows. It never writes the config and does no I/O beyond reading the two files.

**Policy**
- Values: `auto` (default), `local-only`, `cloud-only`. Set in config and tightened per request via `X-Router-Policy`. A header can never loosen the config.
- Under `local-only`, capability and context rules do not push to cloud; context overflow returns 400 with a context-length error, and local failure returns 503.

**Sessions and pinning**
- Session id comes from `X-Session-Id` when present, otherwise a one-way hash of the system prompt plus first user message. The log records which source was used.
- In-memory table with idle TTL (config, default 300 s, documented as "match your OLLAMA_KEEP_ALIVE"), max size (default 10,000) and LRU eviction. TTL refreshes on each request. Concurrent requests on one session are allowed; first writer wins the pin. Nothing is persisted.
- A pin to local may move to cloud by escalation. Failovers do not pin. A pin to cloud never moves to local.

**Failure handling**
- Local connection refused, 5xx, or no first byte within `local_first_byte_timeout_ms` (default 15000) fails over to cloud, only before any bytes are sent to the client. The reason is `local_unavailable`, and the session is not pinned.
- A circuit breaker opens after `breaker_failures` (default 2) consecutive local failures (unreachable or 5xx). For `breaker_cooldown_ms` (default 30000) local is skipped, then one probe is let through. A skipped auto request fails over with reason `circuit_open` and `failed_over` set. A skipped forced-local or `local-only` request returns 503.
- Errors from the chosen target are the client's errors: status and body pass through unchanged, with no retry and no cross-target fallback. The only exception is the local-to-cloud failover above.
- Mid-stream upstream failure ends the SSE stream with an OpenAI-style error event and logs a partial record. Client disconnect cancels the upstream request and logs tokens counted so far. There is no mid-stream failover.
- All router-originated errors use the OpenAI error body shape with correct HTTP status codes.

**Streaming**
- Server-sent events are relayed as received for both targets. Token counts come from the final usage chunk, or the local estimate (flagged) if the provider omits it.
- Streamed chat requests get `stream_options.include_usage: true` (other `stream_options` keys are kept), so the upstream sends a usage chunk. If the client did not set it, that `choices: []` chunk and its blank line are dropped from the relayed stream; no other byte is changed. On a streamed request, a `stream_options` that is not an object or null is rejected with 400; non-streamed bodies are forwarded as received. `server.request_stream_usage = false` turns all of this off. `/v1/responses` streams already carry usage and are untouched.

**Security**
- Binds to loopback by default. Binding elsewhere is an explicit flag and requires `ROUTER_API_KEY`, otherwise startup fails.
- Optional bearer token protects the router. Cloud keys are read from the environment variable named in config. The client's Authorization header is never forwarded upstream.

**Config and CLI**
- Single TOML file with an optional top-level `version` (absent or 1; any other value is rejected at load, since v1.0.0) and sections for server, local + cloud or `[[tier]]`, routing, pricing, session and log. Secrets are never in the file; the config names the env var. Loaded once at startup into typed, validated structs; no hot reload. Invalid config exits non-zero with a clear message.
- CLI subcommands: `serve` (with `--warmup` and `--bind`), `stats`, `report [--since] [--json]` (since v1.0.0), `calibrate --labels FILE` (since v0.7.0), and `init [--path] [--force]` (since v0.8.0), which writes the example config compiled into the binary. It refuses an existing file without `--force`, never creates directories, and writes a temp file in the target's directory that is renamed into place.
- `--warmup` sends one tiny request to the local model at startup.

**Logging and stats**
- Append-only JSONL, one line per request, schema version 1. Fields: timestamp, request id, session id, session source, policy, target (`local` or `cloud`: whether the serving tier is local), tier (its name; additive in v0.6.0, still version 1), reason, failed-over flag, pinned flag, stream flag, returned status, outcome (ok, upstream_error, client_disconnect, stream_error), TTFB ms, total ms, prompt tokens, completion tokens, tokens-estimated flag, saved USD, and (additive, still version 1) the Rule 5 classifier score for the last user message to 3 decimals, null when the classifier is disabled or the prompt is over `complex_prompt_tokens` (rule 5 never scores those).
- Saved USD is the counterfactual cloud cost for requests served locally, using optional configured cloud input/output prices per million tokens. Local cost is treated as zero (electricity and hardware ignored). Null when pricing is absent; cloud-served requests save zero.
- Metadata only. Never prompt or completion text, never keys. A write failure warns on stderr and never fails the request. No rotation in v1.
- `stats` aggregates the file: counts by target and by tier, failovers, percent local, tokens kept local, total saved, p50/p95 latency per target, with an optional `--since` window.
- `report` (v1.0.0) renders the same aggregation as Markdown (overview, latency, reasons, tiers, errors), or as JSON with `--json`. A missing log gives a zero summary. Log lines with a `v` other than 1 are counted as skipped, never misread; a line without `v` is read as version 1.

**Verification**
- The single fast check is `cargo clippy --all-targets -- -D warnings && cargo test`. It needs no network and no Ollama.
- Files stay under 200 lines. Module split follows: schemas, routing rules, session table, upstream clients, server handlers, logging, stats, config.

## Testing Decisions

- A good test asserts external behaviour only: what the client receives, what each upstream receives, and what lands in the log. It does not assert on internal modules, struct layout or call order.
- **Primary seam:** the real axum application driven in-process over HTTP, with two fake OpenAI-compatible upstreams (local and cloud) on ephemeral ports. Fakes can be told to refuse connections, hang, return errors, stream slowly, or die mid-stream.
- **Secondary seam:** direct unit tests of the pure routing function, one or more per rule, including rule precedence and policy conflicts.
- Acceptance coverage: each routing rule; forced model and policy conflicts; local-down failover (refused, 5xx, first-byte timeout); local-only returns 503; cloud 429 passthrough; mid-stream upstream death; client disconnect cancels the upstream; stream and non-stream parity; the models endpoint; router auth and the non-loopback refusal; pin sticking, expiring, escalating and never downgrading; pin table eviction; unknown-field passthrough; client Authorization never forwarded; log line contents and absence of bodies and keys; stats totals and percentiles; missing pricing yields null; log write failure does not fail the request; invalid config fails startup.
- Timing-sensitive tests (timeouts, TTL) use short configured values or a paused tokio clock, never real sleeps of seconds.
- Shared helpers live in `tests/common/` (fake upstreams that can refuse, hang, error, stream slowly or die mid-stream, a router rig, log readers); new tests reuse them.
- A manual smoke test against real Ollama and a real cloud key is documented but is not part of the fast check.

## Stability (v1.0.0)

From 1.0.0 the project follows semver. Within 1.x the following are stable. They may gain optional additions, but they never lose a field or change meaning without a major version:

- **HTTP:** `POST /v1/chat/completions`, `POST /v1/responses`, `GET /v1/models`, the reserved model ids, the `X-Session-Id` and `X-Router-Policy` headers, and the OpenAI error body shape.
- **Config schema version 1:** every key documented in `slm-router.example.toml`, in both the legacy and the `[[tier]]` form. A config that declares another `version` is rejected.
- **Log schema version 1:** exactly these fields, pinned by `tests/log_schema.rs`: `v`, `ts`, `request_id`, `session_id`, `session_source`, `policy`, `target`, `tier`, `reason`, `failed_over`, `pinned`, `stream`, `status`, `outcome`, `ttfb_ms`, `total_ms`, `prompt_tokens`, `completion_tokens`, `tokens_estimated`, `saved_usd`, `classifier_score`. Reasons: `forced`, `policy`, `stateful`, `pinned`, `capability`, `context`, `complexity`, `classifier`, `default`, `local_unavailable`, `tier_unavailable`, `circuit_open`. Outcomes: `ok`, `upstream_error`, `client_disconnect`, `stream_error`. Adding a field or value bumps `v`.
- **`report --json` schema version 1:** the summary object (`v`, `requests`, `local`, `cloud`, `failovers`, `pct_local`, `tokens_kept_local`, `saved_usd`, `latency_ms.{local,cloud}.{p50,p95}`, `reasons`, `tiers`, `errors` as `[{name, count}]`, `client_disconnects`, `tokens_estimated`, `skipped_lines`).
- **CLI:** the subcommands and flags above. The `stats` text and the Markdown `report` are for people, not for parsing; scripts should use `--json`.
- **Library:** the `decide()` signature. Its input and output types may gain fields or `Reason` variants in a minor release, so build `Features` with `..Default::default()` and match `Reason` with a wildcard arm.

## Out of Scope

- Any frontend, dashboard or UI.
- ONNX classifier, semantic response cache, and Stage 3 speculative verification or quality-based escalation (post-1.0, opt-in).
- Provider-format translation (non-OpenAI-compatible APIs).
- Per-user budgets, quotas, multi-tenant auth.
- Hot config reload.
- Prometheus or other metrics endpoints.
- Embeddings, legacy completions and other OpenAI endpoints besides chat completions and the models list.
- Log rotation, log shipping, persistent pins.
- Windows-service packaging, installers and published container images (a `Dockerfile` and an example systemd unit ship in the repo since v0.8.0).
- A published client SDK (the importable routing function is the only library surface).
- Counting electricity or hardware cost in savings.

## Road to 1.0 (done)

- v0.6.0 generalised the two targets to N cost-ordered tiers.
- v0.7.0 added `slm-router calibrate`, which tunes the threshold only; retraining stays in `scripts/train.py`.
- v0.8.0 added `init` and the install tooling.
- v1.0.0 froze the schemas above and added `report`.

Per-tier thresholds, speculative verification, a multi-turn classifier and a semantic cache come after 1.0, and only as opt-in features.

## Further Notes

- Assumption to verify during implementation: Ollama's OpenAI-compatible endpoint ignores a per-request `keep_alive`, so the router cannot control model residency. The pin TTL is therefore a config value the user aligns with their Ollama keep-alive setting.
- The 15000 ms default for the local first-byte timeout is deliberately long: a short timeout would fire during cold model loads and send requests to cloud that should stay local.
- The reasoning-cue list and thresholds are crude by design and live in config so they can be tuned from the log data. Since v0.4.0 the Rule 5 classifier covers hard prompts that carry no cue word; the cues stay as the first, explainable path.
- Savings are a counterfactual estimate. They are only as accurate as the configured prices and the token counts, and the log flags estimated counts.
