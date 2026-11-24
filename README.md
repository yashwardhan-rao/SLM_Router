# SLM_Router

> Headless, OpenAI-compatible proxy that routes each chat request between a local Ollama model and a cloud model, keeps warm caches by pinning sessions, and logs latency and cloud spend avoided.

The v1 scope is defined in [SPEC.md](SPEC.md). Development rules are in [CLAUDE.md](CLAUDE.md).

## v1 at a glance

- Drop-in `/v1/chat/completions` (streaming and non-streaming) plus `/v1/models`. Point any OpenAI SDK at it.
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

## Roadmap

- [x] Repository setup and development guidelines
- [x] v1 spec
- [ ] Pure routing core (`decide()`) with unit tests
- [ ] axum gateway: `/v1/chat/completions`, `/v1/models`
- [ ] Session pinning and policy
- [ ] Upstream clients, streaming, failover
- [ ] JSONL logging and `stats` CLI
- [ ] Post-v1: ONNX classifier, semantic cache, speculative verification, extra tiers
