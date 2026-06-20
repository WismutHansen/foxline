# Changelog

## Unreleased

- Added the initial Rust Voice Gateway crate with XDG config loading, JSON schema output, canonical frame types, a linear frame pipeline, JSONL tracing, and a WebSocket protocol shell using JSON control messages plus binary PCM audio frames.
- Added the first gateway-owned turn manager with configurable turn strategy thresholds, frontend VAD hints as advisory input, STT/VAD-driven turn frames, and barge-in interruption propagation.
- Added `.foxline` voice loadout resolution with bundled and project-local voice extension references, adapter selection defaults, lifecycle policy fields, and schema documentation.
- Added legacy bridge benchmark trace mirroring, comparable event aliases, and a repeatable WebSocket fixture runner for the TypeScript bridge baseline.
- Documented the Rust gateway migration phases, monorepo boundaries, `.foxline`/`.pi` split, bundled versus project-local loadouts, and future frontend client model.
- Added the Rust Pi RPC Brain adapter boundary with identity-bound warm process lifecycle, explicit `pi --mode rpc` launch construction, `--extension` routing, JSONL event mapping, and WebSocket startup prewarm binding.
- Added semantic Avatar Action routing with frontend capability checks and theme-agnostic `avatar_action` server events.
- Added the TTS adapter interface and Qwen3 worker backend protocol mapping for speak, audio_start, audio_chunk, audio_done, error, cancel, shutdown, binary audio frames, and comparable trace events.
- Added the STT adapter interface and Parakeet/Silero WebSocket backend protocol mapping for PCM input, VAD evidence, partial/final transcripts, errors, reset, and comparable trace events.
- Added frontend tool capability negotiation with required/optional loadout tools, advertised frontend tool intersection, startup failure for missing required tools, negotiated tool events, result frames, and unauthorized call rejection.
- Added `just` recipes for running, checking, and testing the Rust gateway while preserving the existing TypeScript bridge path as the migration baseline.
