# SLM_Router

> Headless, OpenAI-compatible proxy that routes each chat request between a local Ollama model and a cloud model, keeps warm caches by pinning sessions, and logs latency and cloud spend avoided.

The v1 scope is defined in [SPEC.md](SPEC.md). Development rules are in [CLAUDE.md](CLAUDE.md).

## v1 at a glance

- Drop-in `/v1/chat/completions` and `/v1/responses` (both streaming and non-streaming) plus `/v1/models`. Point any OpenAI SDK at it.
- Two targets, both OpenAI-compatible, so no provider translation: one local Ollama model, one cloud model.
- A pure, ordered-rule `decide()` function picks the target from cheap heuristics and a privacy policy (`auto`, `local-only`, `cloud-only`).
- Sessions are pinned to a target so the local prompt/KV cache stays warm. A pin only ever escalates local to cloud.
- Local failure before the first byte fails over to cloud, unless policy is `local-only`.
- One metadata-only JSONL log line per request, and a `stats` command for latency and savings.
- No UI, database, or background service.

## Tech stack

Rust, axum, tokio, reqwest, serde, clap.

## Check

```
cargo clippy --all-targets -- -D warnings && cargo test
```

Runs offline, with no Ollama and no network.

## Configuration

Copy [`slm-router.example.toml`](slm-router.example.toml) to `slm-router.toml` and edit it. Every option is documented in the file. Secrets (`ROUTER_API_KEY` and the cloud key) come from environment variables, never from the file.

```
slm-router serve --warmup        # run the proxy
slm-router stats --since 24h     # summarise the request log
```

### `/v1/responses` notes

- The body is forwarded as received. Only `model` is rewritten to the chosen target's model. Routing, pinning, failover and logging work the same as for chat. Token usage comes from `usage.input_tokens` / `output_tokens`, or from the `response.completed` event when streaming.
- **Stateful requests always go to cloud.** A request is stateful if it has a non-empty `previous_response_id`, a `conversation`, `store: true`, `background: true` or an `item_reference` input item. Ollama silently ignores these fields, so the router never sends such a request to local. Forced local (`slm-router/local`) or the `local-only` policy returns 400 `stateful_requires_cloud`. A `null` or `false` value is not stateful.
- **Stateful chains must start on cloud.** Response ids issued by Ollama mean nothing to the cloud. If the first turn was served locally and a later turn sends its `previous_response_id`, the cloud's own error (usually a 404) is relayed back unchanged. Once a session reaches the cloud, its pin keeps it there.
- If an upstream does not implement `/v1/responses` (a 404 or 405 without an OpenAI error body), the request fails with 501 `responses_unsupported`. This error never triggers failover.
- If the stream breaks mid-way, the client gets an `event: error` frame with code `stream_interrupted`.
- `GET` and `DELETE /v1/responses/{id}` are not proxied.

## v1 Local Benchmark

One run of `python scripts/bench.py --skip-complex` against a release build, with `slm-router serve --warmup`, auto policy, and a local `llama3.2:1b`. Numbers are from `slm-router stats` and the request log.

| | |
| :--- | :--- |
| Hardware | Ryzen 5 7500X3D (6 cores / 12 threads), 30.9 GB RAM, Radeon RX 9070 XT, Windows 11 |
| Model | `llama3.2:1b` on Ollama 0.40.18, fully in VRAM (`/api/ps`) |
| Requests | 15: 6 single prompts, a 5-turn session, 4 streamed (2 auto, 2 forced local) |
| Routing | 15 local, 0 cloud, 0 failovers; 6 requests were session-pinned |
| Latency, local | p50 **107 ms**, p95 **1,389 ms** (total time, including generation) |
| Streamed time to first chunk | 13 to 18 ms |
| Ollama cache reuse | follow-up turns of the session: 372 of 420 prompt tokens (89%); single prompts: 138 of 207 (67%, mostly a fixed ~20-token template prefix) |
| Binary | 7.1 MiB (release, x86_64 Windows) |

### A/B: pinned session vs unique session IDs

Two 5-turn conversations on the same model, real replies fed back as history (`max_tokens` 120), auto policy, cloud pricing `input_per_mtok = 3.00`, `output_per_mtok = 15.00` (the cloud was never called). One run each.

| | Same `X-Session-Id` | Unique ID per turn |
| :--- | :--- | :--- |
| Routing | 5 local, 0 cloud | 5 local, 0 cloud |
| `saved_usd` | **$0.012882** | **$0.013017** |
| Cache reuse, follow-up turns (2 to 5) | 1,464 of 1,594 prompt tokens (92%) | 1,509 of 1,594 (95%) |
| Latency, local p50 / p95 | 476 / 530 ms | 472 / 564 ms |

No measurable difference. With every turn routed local, a pin changes nothing, and Ollama's cache matches on the prompt prefix, not the session ID. The gaps come from run-to-run noise in the generated text. A pin only pays off when the unpinned conversation would switch targets mid-session, and this run does not exercise that. In a first attempt without `max_tokens`, turn 5 passed the 1,500-token complexity threshold and went to the cloud, which is that case; that attempt was discarded.

Read it with care: it is one run on one machine with a 1B model. The cache figures are Ollama's own `cached_tokens`; every request stayed local, so this run does not measure what pinning saves compared with bouncing between targets. Token counts for the 4 streamed requests are estimates (Ollama sends no usage chunk unless the client asks for it), so "tokens kept local" is partly estimated. No pricing was configured, so there is no dollar figure.

## Roadmap

- [x] Repository setup and development guidelines
- [x] v1 spec
- [x] Pure routing core (`decide()`) with unit tests
- [x] axum gateway: `/v1/chat/completions` (streaming and non-streaming), `/v1/models`
- [x] Session pinning and policy
- [x] Upstream clients, streaming, failover
- [x] JSONL logging and `stats` CLI
- [x] v0.2.0: `/v1/responses` passthrough (streaming, stateful-to-cloud routing)
- [ ] Post-v1: ONNX classifier, semantic cache, speculative verification, extra tiers
