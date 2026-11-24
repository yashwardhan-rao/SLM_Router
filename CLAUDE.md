# SLM_Router

Headless, OpenAI-compatible LLM router. Routes each `/v1/chat/completions` request to a small local/cloud model or a frontier model based on complexity, domain, and cost/privacy policy. Backend only: no UI, no dashboard.

`README.md` is the architecture reference (4-stage pipeline, model tiers). It is a design doc, not a spec of what exists. Check the code before assuming a stage is implemented.

## Stack

- Python 3.12+, FastAPI, Pydantic v2, Uvicorn
- LiteLLM for provider calls (Ollama, Groq, OpenAI, Anthropic)
- ONNX Runtime for the Stage 1 classifier (later; start with heuristics)

## Conventions

- Validate every request and response at the boundary with Pydantic models.
- Return structured errors with correct HTTP status codes (OpenAI-style error body).
- No `TODO: implement later` stubs in core routing paths.
- Keep routing decisions pure and testable: classifier in, policy in, model choice out. No I/O inside the decision function.
- Log every routing decision (chosen model, reason, latency) as structured JSON.

## Modularity

- Keep every file under 200 lines. When one grows past that, split it: provider clients, routing stages, and schemas each get their own module.

## Verification

- Run `pytest` and `mypy` (or `ruff check`) before saying a change works.
- Do not claim an endpoint works until you have called it.

## Git

- Conventional Commits: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`.
- The commit body explains why the change was made, not only what changed.
- Never commit `.env` or API keys.
