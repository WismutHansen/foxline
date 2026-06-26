---
status: accepted
---

# Client-owned STT/TTS via capability negotiation

STT and TTS are capability-negotiated ownership slots, not hardcoded server adapters. A frontend client can declare at `hello` that it owns STT and/or TTS, in which case the gateway runs that speech stage on the client instead of in a gateway-owned worker. This is accepted as a design direction and a target protocol shape; it is not yet implemented. The first implementation lands only when a concrete deployment needs it (offline Tauri bundle, edge, or mobile).

## Decision

Speech I/O ownership is negotiated as part of frontend capabilities, the same place audio format and frontend tools are negotiated today. When a client owns a stage, the gateway does not instantiate that adapter and instead routes the corresponding data through the protocol.

Client-owned STT:

- The client declares an STT ownership mode in its capabilities.
- On `start_session`, the gateway skips `build_stt_adapter` and does not launch or poll the Parakeet/Silero worker for that session.
- The client performs recognition locally and sends a new authoritative client control message carrying partial and final transcripts.
- The gateway injects that message into the canonical pipeline as the same `Frame::Stt(SttFrame::Partial)` / `Frame::Stt(SttFrame::Final)` frames the server adapter would have produced.
- The turn manager consumes those frames unchanged. Turn commitment stays gateway-authoritative; a client-owned STT is a different producer of STT evidence, not a different turn authority.

Client-owned TTS:

- The client declares a TTS ownership mode in its capabilities.
- On `start_session`, the gateway skips `build_tts_adapter` and does not launch or prewarm the Qwen3 worker for that session.
- The gateway emits `assistant_delta` text as usual but suppresses binary output PCM. The client synthesizes audio locally from the text it receives.
- Barge-in still works: a client `interrupt` cancels the active turn, and the client is responsible for stopping local playback on the resulting `audio_reset` or phase change.

The gateway always owns, regardless of speech ownership:

- Turn management and turn strategy thresholds.
- The Brain path: Pi RPC lifecycle, model/provider access, tools, skills, extensions, prompt behavior.
- Frontend tool routing and avatar action routing.
- Latency tracing and session state.

A client-owned speech session is still a gateway-mediated session. It never collapses to the client talking to Pi directly, and it never lets client VAD commit a turn on its own; that invariant is already enforced for `VadHint` today.

## Context

Foxline has two target deployments. The server-side deployment serves heavy, high-quality speech models from gateway-owned workers, which is where Qwen3-TTS and Parakeet belong. The packaged Tauri/desktop deployment targets minimal setup and, in the limit, no Python workers at all, which matches the stated target binary runtime and Python-boundary goals in `ARCHITECTURE.md`.

A thick-client deployment is therefore legitimate, and KITT is already one: today it runs eaRS, Kokoro, and a local LLM and wants to keep that shape while gaining a shared gateway protocol. Treating STT/TTS as capability slots lets a client bring its own speech stack without forking the gateway or inventing a second protocol.

The architecture already implies this direction:

- The canonical pipeline is source-agnostic. It consumes `Frame::Stt`, `Frame::Tts`, and `Frame::Vad` without caring who produced them.
- `SttAdapter` and `TtsAdapter` are traits, and `build_stt_adapter` / `build_tts_adapter` are the only hardcoded instantiation points.
- `FrontendCapabilities` already advertises audio format at `hello`, and the reserved TTS backend names document that the adapter is a swappable slot.
- Frontend tools already prove the two-sided capability-negotiation pattern the gateway wants for all client-declared surfaces.

What is missing is only the ownership flag on the capability profile and an authoritative client STT control message.

## Consequences

- `FrontendCapabilities` gains speech ownership fields (for example an optional `speech` section with `stt` and `tts` owner slots). Defaults preserve current behavior: server-owned STT and TTS.
- A new client control message carries authoritative STT results. Client `vad_hint` remains advisory and never commits a turn, as the existing turn-manager test guarantees.
- `build_stt_adapter` and `build_tts_adapter` become conditional on the negotiated capability for that session, and the gateway WebSocket loop must tolerate a session with no server STT adapter, no server TTS adapter, or neither.
- When the client owns TTS, the gateway suppresses binary output PCM and relies on text deltas; benchmark traces must distinguish server-TTS and client-TTS audio paths.
- Turn authority, the Brain path, frontend tools, avatar actions, and tracing remain gateway-owned. Client-owned speech changes where audio is processed, not who owns the session.
- Benchmarking must treat the client-owned path as a separate latency profile, because it removes the server STT/TTS network hops but adds client synthesis cost.
- No browser WASM STT/TTS is added now. Implementation is deferred until a deployment needs it.

Model-size reality constrains what client-owned speech can realistically mean today:

- Qwen3-TTS at roughly 1.2 GB in 4-bit is impractical to run in-browser and is slower than the server worker. Moving it client-side would defeat the reason foxline uses it.
- Streaming Parakeet is borderline for browser WASM on CPU and battery.
- Silero VAD and Kokoro TTS are feasible in-browser and are the realistic first client-owned candidates, not the current server models.

## Considered options

- Hardcode server-owned STT and TTS forever: rejected because it blocks offline, edge, and bundled-Tauri deployments and the no-Python target binary runtime.
- Full client-only, with the client owning turns: rejected because it re-creates the frontend/Pi/STT-TTS coupling that ADR 0001 rejected, and it loses gateway-owned barge-in, tools, tracing, and Brain lifecycle.
- Let the client talk to Pi directly, bypassing the gateway: rejected because it creates a second Brain path outside Pi's tool, extension, skill, and prompt-template model, which ADR 0001 rejects.
- Implement browser WASM STT/TTS now: rejected because Qwen3-TTS is too heavy for the browser, no deployment has proven the need yet, and the current server-worker architecture is the right default for the high-quality voice models foxline ships today. Designing for client-owned speech now, without building it, captures the option without paying the cost.
