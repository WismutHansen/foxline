---
status: accepted
superseded: 0004 (oracle consequences)
---

# Rust Voice Gateway with frame-based streaming pipeline

Foxline will evolve from a single TypeScript bridge plus codec frontend into a monorepo centered on a Rust Voice Gateway. The gateway is the reusable low-latency runtime for voice sessions, while the Foxline Codec UI is one WebSocket frontend client; KITT, oqto, and future clients should be able to connect to the same gateway protocol.

## Decision

The Rust Voice Gateway is the new runtime core. It owns voice-session orchestration, frontend transport, STT/TTS adapters, Pi RPC Brain lifecycle, frontend tool routing, avatar action routing, and latency tracing. It does not call an LLM server directly: the Brain is always a Pi RPC process, and Pi owns model-provider access, tools, skills, extensions, prompt templates, and session semantics.

The gateway uses a canonical internal frame pipeline inspired by Pipecat. The first implementation is a simple linear pipeline, but frame types and component boundaries must not prevent future graph routing, observers, sidecars, or multi-agent workflows. The initial frontend transport is WebSocket with JSON control frames and binary audio frames; Tauri desktop mode should use the same protocol unless there is a measured reason not to.

## Context

Foxline has two target deployments: a packaged desktop/Tauri app with minimal setup, and a server-side voice interface for agent orchestration that can serve frontends such as Foxline Codec UI, KITT, and oqto. The current `server/bridge.ts` implementation is useful as a latency and behavior baseline, but it couples frontend, Pi RPC, STT/TTS worker lifecycle, and codec-specific behavior too tightly for these goals.

## Consequences

- `server/bridge.ts` becomes the legacy/reference implementation and benchmark oracle during migration.
- The Rust gateway must stay within +50ms end-to-end latency deviation from the current bridge/PiBot-style worker behavior.
- Benchmark traces are JSONL event streams with comparable event names across legacy bridge and Rust gateway.
- Trace events are written server-side by default and exposed to frontends only in debug/benchmark mode.
- STT, TTS, and Brain integrations are capability adapters, not hardcoded product assumptions.
- The gateway owns turn-taking state, using frontend VAD hints, STT/VAD evidence, and configurable turn strategies.
- Frontend tools require two-sided capability negotiation: the effective tool surface is the intersection of the voice loadout allowance and the frontend's advertised capabilities.
- Avatar behavior is expressed as theme-agnostic semantic Avatar Actions; frontend clients own visual implementation.
- Project-local gateway configuration lives under `.foxline/`; Pi-specific default configuration lives under `.pi/`.
- Voice-only Pi extensions may live in bundled gateway extension directories or project-local `.foxline/extensions/`, but are loaded explicitly by the gateway with Pi `--extension` flags only for voice sessions.
- Bundled demo personas can ship in this repo, but voice assets needed for speech behavior are gateway-owned while visual avatar assets are frontend-owned.

## Considered options

- Continue evolving `server/bridge.ts`: rejected because it preserves accidental coupling and makes Tauri/server/KITT/oqto reuse harder.
- Build full Pipecat-style graph framework immediately: rejected because the first hard problem is low-latency Pi-backed voice interaction, not a universal voice framework.
- Make every component an independent network microservice: rejected as the default because local spawned workers can better preserve latency, while server/network deployments remain configurable.
- Let the gateway call LLM servers directly: rejected because it would create a second Brain path outside Pi's tool, extension, skill, and prompt-template model.
