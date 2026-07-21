---
status: accepted
---

# eaRS as the Foxline STT backend, with managed and remote transports

The Foxline Voice Gateway gets its speech-to-text from eaRS, integrated at the eaRS server boundary rather than at any single engine. Foxline speaks eaRS's WebSocket protocol and never links an ASR engine directly. Engine choice, and the set of available engines, belongs to eaRS.

## Decision

Add an `ears` STT backend to the gateway (`crates/voice_gateway/src/stt.rs`) that connects to an eaRS server over its existing WebSocket protocol and maps eaRS messages onto the canonical Foxline frame pipeline. This backend replaces the `parakeet-silero` path (the Python `services/parakeet_silero_ws_server.py` server plus the PiBot `parakeet-cpp-stt-worker` binary), which is retired once the `ears` backend reaches turn-and-latency parity.

The backend is engine-agnostic. eaRS builds expose engines behind cargo features and selects one per session via `--engine` at launch or the `SetEngine` WebSocket command at runtime. Foxline forwards a configured engine string and treats the available engine set as advertised by eaRS (`GetStatus`/`Status`), not assumed. All current eaRS engines are supported through this one integration: `kyutai`, `parakeet-rs`, and `transcribe-cpp`, plus any engine eaRS adds later.

The backend supports two transports behind one shared protocol-mapping layer:

- **Managed** — the gateway spawns a local `ears-server` binary, owns its lifecycle (start, health-check, shutdown on session release), and connects to it on loopback. This is the low-latency, zero-configuration default for a single-machine deployment. Its lifecycle mirrors the spqx TTS worker: a fetched, checksum-verified binary the gateway launches on demand.
- **Remote** — the gateway connects to an already-running `ears-server` at a configured `ws://host:port` and does not manage its lifecycle. This lets STT run on a separate machine when services are split.

Only transport and process lifecycle differ between the two modes; the eaRS-message-to-Foxline-frame mapping is identical and lives in one place.

For engines with revisable streaming hypotheses (notably transcribe.cpp), eaRS emits `Interim { text }` containing the authoritative `committed + tentative` preview. Append-only clients continue consuming `Word`; revisable UI clients replace their preview from `Interim`. At an ingress-VAD end-of-turn, transcribe.cpp finalizes and restarts its native stream, emits its complete snapshot, then emits `Speech { active: false }`. This ordering prevents trailing words from arriving after Foxline commits the turn and allows multiple utterances on one WebSocket.

## Context

Foxline's turn strategy (`turn.rs`) is authoritative over barge-in and interruption and depends on explicit speech-boundary events. Today those come from the Python silero layer as `{"type":"status"}` messages carrying `speech_start` / `speech_stop`, which `stt.rs` maps to `VadFrame::SpeechStarted` / `SpeechStopped`.

eaRS already emits transcripts as internally tagged messages (`{"type":"final","text":...,"words":...}`, `{"type":"word",...}`), which are directly compatible with the gateway's existing `ParakeetMessage` decoding — the gateway ignores the extra `words` field. eaRS performs VAD internally (webrtc-vad and an engine-driven silence timeout) but consumes it to drive endpointing (`Pause`/`Final`) and does not expose speech boundaries on the wire. That is the one protocol gap.

To close it, eaRS gains one additive WebSocket message, `Speech { active: bool }`, emitted by a single engine-agnostic boundary VAD at the server audio ingress — before audio is handed to any engine. This one acoustic detector emits speech boundaries uniformly for every engine (kyutai, parakeet-rs, transcribe-cpp, and any future engine), rather than relying on each engine's internal endpointing, which varies (moshi has linguistic end-of-turn, parakeet-rs gates on silence, transcribe-cpp has no VAD at all). The message is additive to an internally tagged enum whose consumers all tolerate unknown variants (same-version clients match with a catch-all; cross-version clients drop messages that fail to deserialize), so it does not affect eaRS dictation or any existing eaRS client.

Integrating at the eaRS boundary — rather than linking parakeet-rs or any single engine into the gateway — keeps ASR engines, their model management, and their build/acceleration features inside eaRS. Foxline stays a thin STT client and inherits every eaRS engine through the same code.

## Consequences

- A new `EarsSttBackend` in `crates/voice_gateway/src/stt.rs` with a transport enum (`Managed` / `Remote`) and a shared eaRS-message mapping. `SttFrame` and `VadFrame` outputs are unchanged, so `turn.rs`, `pipeline.rs`, and tracing are unaffected.
- STT config gains an `[stt.ears]` section: `mode` (`managed` | `remote`), `engine` (`kyutai` | `parakeet-rs` | `transcribe-cpp`), and `url` (remote only). `backend = "parakeet-silero"` remains selectable during migration; the default flips to `ears`/`managed` once at parity.
- eaRS adds the additive `Speech { active }` message, emitted by a single boundary VAD at the audio ingress so all engines get uniform speech boundaries with no per-engine wiring. Per-engine boundary emission (moshi end-of-turn, parakeet silence gate) was considered and removed in favor of the single ingress owner. This is the only cross-repo change. Existing eaRS clients, including dictation, are unaffected.
- The managed `ears-server` binary is fetched and checksum-verified like the spqx TTS worker. Which engine cargo features that distributed binary ships determines which engines managed mode can run; `parakeet-rs` requires `ORT_DYLIB_PATH` at runtime, and `transcribe-cpp` can add `metal`/`cuda`. Remote mode sidesteps this because the remote host owns its own eaRS build and features.
- Engine availability is negotiated, not assumed: Foxline queries eaRS status and falls back to the server's default engine when a requested engine is not loaded.
- `services/parakeet_silero_ws_server.py` and the PiBot `parakeet-cpp-stt-worker` dependency are retired once the `ears` backend is at parity. `scripts/setup-deps.sh` stops cloning and building the PiBot worker and the parakeet.cpp shared library for the STT path, removing the Python STT server from the runtime and advancing the Rust-first, zero-Python direction.
- Minimum-latency managed transport ships as loopback TCP first. A Unix-domain-socket or stdio transport for `ears-server` is a later optimization, taken only if measurements justify it; it is not built speculatively.

## Considered options

- Point the existing `parakeet-silero` client straight at `ears-server` with no adapter: rejected. Transcripts are compatible, but speech boundaries are missing, which would break barge-in and interruption.
- Wrap an eaRS engine as a PiBot-style worker inside the Python `parakeet_silero_ws_server.py`, swapping only the inner engine binary: rejected. It keeps Python in the runtime and duplicates VAD (silero in Python and webrtc-vad in eaRS), which is the worst architecture for the least conceptual change.
- Link parakeet-rs (or another single engine) directly into the gateway: rejected. It couples the gateway to one engine and to that engine's model management and build/acceleration features, and it discards eaRS's runtime engine switching and multi-engine support.
- Managed transport only, or remote transport only: rejected. Managed alone cannot split STT across machines; remote alone forfeits the zero-configuration, low-latency single-machine default. One adapter with two transports serves both without duplicating the protocol mapping.
- Derive speech boundaries gateway-side from `Pause`/`Final` plus a second local VAD, instead of adding `Speech` to eaRS: rejected. It reintroduces a second VAD owner and duplicates endpointing logic that eaRS already performs; the additive eaRS message keeps VAD single-owned.
