# Changelog

## Unreleased

- Added the initial Rust Voice Gateway crate with XDG config loading, JSON schema output, canonical frame types, a linear frame pipeline, JSONL tracing, and a WebSocket protocol shell using JSON control messages plus binary PCM audio frames.
- Added the first gateway-owned turn manager with configurable turn strategy thresholds, frontend VAD hints as advisory input, STT/VAD-driven turn frames, and barge-in interruption propagation.
- Added `.foxline` voice loadout resolution with bundled and project-local voice extension references, adapter selection defaults, lifecycle policy fields, and schema documentation.
- Added `just` recipes for running, checking, and testing the Rust gateway while preserving the existing TypeScript bridge path as the migration baseline.
