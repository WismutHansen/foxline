# ARCHIVED: voice-cascade extension (spike outcome)

Scrapped 2026-08-25 after end-to-end measurement (trx fxl-xwd9 / fxl-exmc / fxl-cnfq).

## What it was
A cascaded multi-model brain for voice mode: tiny drafter (gemma4:e2b) writes the
opening sentence, main speech model (gemma-4-26B-A4B) continues, orchestrator
(deepseek-v4-flash) escalates hard queries via tool call.

## Why scrapped (measured, n=7 per arm, streamed-audio E2E)
- Phase 1 (drafter steer): strictly worse latency (+~1s first-audio), unproven quality gain.
- Phase 2 (speculative TTS): ceiling collapsed under measurement — commit->audio is only
  ~830ms clean, so max savings ~0.5s, not worth spoken-hallucination risk + a second warm model.
- Bitter lesson: "brain too slow" is a moving target; gemma raw TTFT already 169ms.
  Latency cascades are scaffolding that obsoletes themselves.
- Root cause of perceived slowness was COMPUTE CONTENTION (1.0s -> 3.8s swings from
  co-located workloads), not model speed. Fix = placement/dedicated compute, not cascades.

## What survives
- `ask_orchestrator` escalation-tool pattern (judgment delegated to the model; near-zero cost).
- TurnManager commit_grace_ms fix (post-commit decoder-flush partials were killing turns).
- Channels-as-data concept, reframed: tiers = placement decisions across machines/compute
  pools (oqto fleet), not model stacks.
- The streamed-audio E2E harness (benchmarks/run-rust-gateway-fixture-spike.ts +
  agents/spike-campbell workspace).

Full details: git history of this folder + wiki research note "voice cascade post-mortem".
