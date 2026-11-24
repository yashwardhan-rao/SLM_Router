# SLM_Router

Headless, OpenAI-compatible LLM router. Routes each `/v1/chat/completions` request to a local Ollama model or a cloud model based on complexity and policy. Both targets speak OpenAI-compatible `/v1/chat/completions`, so there is no provider translation layer. Backend only: no UI, no dashboard.

`README.md` is the architecture reference (4-stage pipeline, model tiers). It is a design doc, not a spec of what exists. Check the code before assuming a stage is implemented.

## Stack

- Rust (stable, 2021+ edition), axum, tokio, reqwest, serde / serde_json, clap
- Upstream calls are plain HTTP to OpenAI-compatible endpoints (Ollama `/v1`, cloud `/v1`). No LiteLLM, no provider SDKs.
- Stage 1 classifier starts as heuristics. ONNX (`ort`) is a later option.

## Conventions

- Validate every request and response at the boundary with typed serde structs. Reject unknown or malformed input with a 4xx.
- Pass unrecognised request fields through to the upstream untouched (`#[serde(flatten)]` extras) so the proxy stays drop-in.
- Return structured errors with correct HTTP status codes (OpenAI-style error body).
- No `TODO: implement later` stubs in core routing paths.
- Keep routing decisions pure and testable: classifier in, policy in, model choice out. No I/O inside the decision function.
- Log every routing decision (chosen model, reason, latency) as structured JSON.

## Modularity

- Keep every file under 200 lines. When one grows past that, split it: upstream clients, routing stages, and schemas each get their own module.

## Verification

- The single fast check is `cargo clippy --all-targets -- -D warnings && cargo test`. Run it before saying a change works. It must need no network and no running Ollama; fake upstreams in tests.
- Do not claim an endpoint works until you have called it.

## Git

- Conventional Commits: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`.
- The commit body explains why the change was made, not only what changed.
- Never commit `.env` or API keys.
