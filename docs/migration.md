# Migration note

Foxline is the active source-of-truth repository for the Codec-style runtime and asset installer.

The older `codec` working directory is an archive/scratch area. It may contain generated local assets, experiments, benchmark reports, and historical cleanup work, but new product development should happen here in `foxline`.

## Current policy

- Make runtime, installer, documentation, and release changes in `foxline`.
- Treat `codec` as read-only unless recovering historical experiments or generated local test assets.
- Do not copy generated copyrighted/runtime outputs into `foxline`.
- Track publish-readiness work under trx epic `fxl-yftd`.

## Active architecture

Foxline uses:

- pi-rpc brain runtime
- parakeet.cpp STT, default model `tdt-0.6b-v3-q8_0.gguf`
- Qwen3-TTS worker-based TTS
- user-provided MGS source media only

See `ARCHITECTURE.md` for details.

## Rust Voice Gateway migration

The accepted target architecture is documented in `docs/adr/0001-rust-voice-gateway-frame-pipeline.md`.

Migration rules:

- Keep `server/bridge.ts` usable until Rust benchmark parity is proven.
- Treat `server/bridge.ts` as the latency and behavior oracle during migration.
- Do not add a second Brain path. The gateway must talk to Pi RPC; it must not call LLM servers directly.
- Keep browser and future Tauri clients on the same WebSocket protocol unless measurement proves otherwise.
- Keep normal audio frames binary in the Rust gateway protocol.
- Keep generated copyrighted/runtime assets out of git.

Migration phases:

1. Baseline: preserve the TypeScript bridge and emit comparable JSONL benchmark traces.
2. Foundation: add the Rust gateway crate, canonical frame model, WebSocket transport, `.foxline` loadouts, and gateway-owned turn management.
3. Adapters: connect Pi RPC Brain lifecycle, Qwen3-TTS worker, and Parakeet/Silero STT through gateway adapter interfaces.
4. Frontend tools and avatar actions: negotiate frontend capabilities from `.foxline` loadouts and route semantic Avatar Actions without Codec-specific coupling.
5. Codec UI migration: connect Foxline Codec UI to the Rust gateway while keeping the bridge path available.
6. Benchmark gate: compare legacy and Rust JSONL traces and flag any end-to-end latency regression greater than +50ms.
7. Cutover: make Rust gateway the default only after the demo path, interruption behavior, tool routing, and benchmark deltas are proven.

## Configuration split

- `.foxline/` is Voice Gateway-owned voice-session configuration: loadouts, voice-only extensions, adapter choices, lifecycle policy, and frontend tool requirements.
- `.pi/` is Pi-owned Brain configuration: Pi defaults, provider/model behavior, tools, skills, prompt templates, and session semantics.

Bundled personas and loadouts can ship as metadata for demos. Project-local personas/loadouts should live in workspaces and use `.foxline` for voice overrides. Generated voice references, fillers, and avatar assets remain local and ignored.

## Brain lifecycle

The Rust gateway binds each Pi RPC Brain to:

- agent id
- workspace path
- voice loadout name
- frontend capability profile hash

This identity controls warm Brain reuse. The gateway may prewarm a Brain when the loadout or global config requests it, can shut down idle Brains, can restart a Brain after failure, and must pass voice-only extensions with explicit `--extension` flags. The gateway must not call a model provider or LLM server directly.

## Future clients

KITT, oqto, Tauri, and other clients should connect as frontend clients to the gateway WebSocket protocol. The gateway should stay theme-agnostic: clients own visuals, playback UX, and any mapping from semantic Avatar Actions to their presentation system.
