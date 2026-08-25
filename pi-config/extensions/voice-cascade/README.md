# voice-cascade pi extension

Foxline "channels" spike (trx: fxl-exmc, epic fxl-xwd9): a cascaded multi-model brain for
voice mode. Pi stays the only Brain path (ADR-0001); the gateway is untouched.

## Bitter-lesson factorization

This extension is a **dumb, deterministic runner**. All judgment lives in data:

- **Channel topology** (tiers, endpoints, models, budgets) → `ChannelDef`, overridable via
  `$FOXLINE_CHANNEL_FILE` (JSON). Better models arrive → edit JSON or delete a tier row.
- **Every prompt and the steer template** → authored data (`draft.system`, `steerTemplate`,
  `orchestrator.system`), not code. Phase-2 safe-openers belong here too, not in a code branch.
- **Escalation** is the speech model's judgment via the `ask_orchestrator` tool — never a
  code heuristic.

The code only does mechanism: fetch-with-timeout, deterministic sentence truncation,
silent degrade on drafter failure, fail-loud on a broken channel file. The built-in
`DEFAULT_CHANNEL` is a disposable example, not policy.

## Cascade tiers (local, measured warm)

| tier | model | server | measured |
|------|-------|--------|----------|
| drafter | smollm2:135m | Ollama `:11434` | TTFT ~59 ms, ~367 tok/s |
| speech (main brain) | gemma-4-26B-A4B-it Q4_K_M | llama-server `:8001` | content TTFT ~169 ms with thinking off |
| orchestrator | deepseek-v4-flash | ds4-serve `:8000` | TTFT ~3.7 s, ~35 tok/s (prefill-bound) |

Gemma must run with thinking disabled or it burns the token budget in `reasoning_content`
before any speakable text: pass `--chat_template_kwargs '{"enable_thinking": false}'` per
request, or start llama-server with `--reasoning-budget 0`.

## What it does

- **Drafter steer (`input` hook):** every user turn gets a one-sentence spoken opening
  drafted by smollm2 and injected into the prompt as a `[voice-cascade]` note; gemma
  continues seamlessly from it. Drafter failure/timeout degrades silently to a normal turn.
- **`ask_orchestrator(question, context?)`:** tool for hard queries; ds4 answers in concise
  spoken prose. On failure returns an error result so the speech model self-answers.

## Configuration

Channel file (recommended): `$FOXLINE_CHANNEL_FILE` → JSON with `enabled`, `draft`
(url/model/maxTokens/timeoutMs/system/maxSentences/maxInputChars), `steerTemplate`
(`${opening}` placeholder), `orchestrator` (tier + system). Env fallbacks:

- `FOXLINE_CASCADE=0` disables everything (default on).
- `FOXLINE_DRAFTER_URL`, `FOXLINE_DRAFTER_MODEL`, `FOXLINE_ORCHESTRATOR_URL`, `FOXLINE_ORCHESTRATOR_MODEL`
- `FOXLINE_DRAFT_TIMEOUT_MS` (1500), `FOXLINE_DRAFT_MAX_TOKENS` (60), `FOXLINE_DRAFT_MAX_SENTENCES` (2)
- `FOXLINE_ORCHESTRATOR_TIMEOUT_MS` (120000), `FOXLINE_ORCHESTRATOR_MAX_TOKENS` (400)
- `FOXLINE_CASCADE_DEBUG=1` logs the draft to stderr

## Phase 2 (not in this extension)

Stream drafter audio to TTS immediately while gemma regenerates a better continuation
(speculative TTS at the gateway / Brain trait seam fxl-ma47.2), adaptive cascade keyed on
context length + recent TTFT, and generalizing the pattern into "channels" — per-interaction-mode
compositions of cooperating pi sessions.
