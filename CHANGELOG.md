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
- Added opt-in Codec UI wiring for the Rust Voice Gateway WebSocket protocol, including frontend capability declaration, binary microphone PCM streaming, binary PCM playback handling, session lifecycle mapping, and legacy bridge fallback.
- Added a Rust gateway latency benchmark harness with fixture runners, canonical JSONL trace comparison, +50ms budget reporting, and machine-readable summaries under `benchmarks/traces/`.
- Wired the Rust gateway WebSocket loop into loadout-selected STT, Pi RPC Brain, and TTS adapters so live sessions can route binary microphone PCM through turn commits, assistant deltas, TTS requests, binary output PCM, interrupts, and comparable traces.
- Added a service-backed Rust gateway audio smoke fixture path that streams local ignored PCM16/WAV utterances and waits for assistant text, turn completion, binary output audio, and trace discovery.
- Added `just` recipes for running, checking, and testing the Rust gateway while preserving the existing TypeScript bridge path as the migration baseline.
- Updated the Parakeet/Silero STT adapter to convert gateway PCM16 input into the Float32 wire format expected by the current worker.
- Disabled global Pi extensions by default for gateway-launched Brain RPC processes with `brain.no_extensions = true`, while preserving explicit `.foxline` loadout extension flags.
- Prewarmed the Qwen TTS worker on Rust gateway session startup to avoid first-response cold-start latency.
- Added benchmark fixture support for interrupt-on-audio runs, corrected Rust trace discovery to select traces with the requested milestones, and made the latency comparator use completed fixture windows inside session-long traces.
- Proved a service-backed Rust gateway smoke path through frontend WebSocket, STT, turn commit, Pi RPC Brain, Qwen TTS, and binary frontend audio; full latency parity remains open for real legacy audio/STT and paired barge-in cancellation traces.
- Added legacy audio/STT benchmark replay, Pi model/thinking/prompt loadout fields, and a Campbell voice loadout that prewarms Pi with the legacy baseline settings; the live comparator now passes the +50ms budget for STT final, Brain first token, TTS first audio, frontend audio scheduling, and barge-in cancel.
- Updated the Codec UI Rust gateway client to default voice sessions to the selected agent workspace, matching the proven `agents/<agent>` Pi launch path.
- Separated Rust gateway Agent identity from Persona voice/presentation identity so clients can switch voices or characters without changing the Pi-backed Brain workspace.
- Documented Persona packages as self-contained directories with generic identity, prompt, voice, and frontend-specific config sections, including centrally installed packages and workspace overrides.
- Added two shippable original demo Persona packages, Alex Grant and Mira Chen, with generated Codec portraits and Koko/Kokoro reference voices for no-disc setups.
- Shipped all generated Koko voice variants for the generic demo Personas, defaulting Alex Grant to the Onyx/Daniel blend and Mira Chen to the Sarah/Isabella blend.
- Exposed the enabled Codec character catalog in the Rust Voice Gateway UI client so the Memory menu can switch Personas instead of being limited to the startup Persona.
- Added `just demo-rust` as a one-command tmux launcher for Parakeet/Silero STT, the Rust Voice Gateway, and the Codec frontend configured to open directly against the gateway.
- Added a status bar action to copy the dialogue transcript verbatim to the clipboard.
- Fixed Rust Codec Memory switching so agent-backed characters switch both the Pi-backed Agent and Persona instead of only changing the voice/presentation Persona.
- Fixed Codec markdown table rendering by preserving GFM table rows during display cleanup and stripping leaked Pi/model channel markers before transcript and TTS output.
- Fixed Rust gateway interruption handling so Codec interrupt and barge-in paths send abort to the active Pi RPC Brain and cancel TTS through one runtime handler.
- Fixed voice-session output hygiene by appending a spoken-output contract to Pi launches, stripping leaked thought-trace paragraphs at the Brain boundary, and normalizing Markdown/table-shaped assistant text before TTS.
