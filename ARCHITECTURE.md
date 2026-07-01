# Foxline architecture

Foxline follows PiBot's low-latency worker architecture rather than HTTP microservices. The Rust Voice Gateway is the sole gateway and Brain path; the legacy TypeScript bridge has been retired.

```text
Frontend Skin (apps/codec, future apps/overlayz)
  -> @foxline/voice-client
      -> foxline-voice-gateway WebSocket
          -> canonical frame pipeline
          -> Pi RPC Brain adapter
          -> STT adapter
          -> TTS adapter
          -> frontend tool/avatar action routers
```

The gateway never calls LLM servers directly. Pi is always the Brain path and owns model/provider access, tools, skills, extensions, prompt templates, and session behavior.

The Rust gateway binds each warm Brain process to an identity made from agent, workspace, voice loadout, and frontend capability profile. That identity determines the Pi RPC session name and prevents one frontend/tool profile from accidentally reusing another profile's warm Brain.

## Runtime defaults

- Brain: pi-rpc only.
- TTS: Qwen3-TTS only.
- STT: parakeet.cpp by default, currently with Silero VAD for turn detection.

## Product boundaries

- Voice Gateway: reusable Rust runtime for voice sessions, frame routing, adapters, Pi RPC lifecycle, frontend tools, avatar actions, and tracing.
- Foxline Codec UI: browser frontend skin that maps the gateway protocol to the Metal Gear Solid-style Codec experience.
- Brain: Pi RPC process. It owns reasoning, model/provider access, tool use, skills, extensions, prompt templates, and session behavior.
- Agent: Pi-backed working identity rooted in a workspace and Pi configuration.
- Persona: presentational voice/UI identity for a frontend. A bundled demo persona may include metadata and instructions, but generated voice and avatar assets stay local and out of git.

## Rust Voice Gateway protocol

The gateway frontend transport is WebSocket:

- JSON text messages carry control and event frames such as `hello`, `start_session`, `end_session`, `vad_hint`, `interrupt`, and errors.
- Binary messages carry PCM audio frames on the normal path. Audio must not be base64 encoded in JSON.
- Frontends declare capabilities before session startup. The current shell records this profile and follow-up work will negotiate tool and avatar action surfaces against `.foxline` loadouts.
- Debug trace streaming is gated by configuration; server-side JSONL traces remain the default.

Trace events use names comparable to the legacy bridge path, including `mic_frame_received`, `stt_partial`, `stt_final`, `brain_request_start`, `brain_first_token`, `tts_request_start`, `tts_audio_start`, `frontend_audio_play_scheduled`, `barge_in_received`, and `tts_cancel_sent`.

Legacy bridge benchmark traces are mirrored under `benchmarks/traces/legacy-bridge/` and can be driven with `just benchmark-legacy-bridge`. See `docs/benchmarks.md`.

## Turn management

The Rust gateway owns authoritative turn state. Frontend VAD and noise-gate messages are accepted as hints, but they do not commit a user turn by themselves. STT/VAD evidence frames drive `turn.user_started`, `turn.user_committed`, and `turn.interrupted` frames inside the canonical pipeline.

Turn strategy is configurable under `[turn]` in the gateway config:

- `silence_timeout_ms`
- `min_speech_duration_ms`
- `max_utterance_duration_ms`
- `barge_in_confirmation_window_ms`

Barge-in can interrupt local gateway output immediately through a `turn.interrupted` frame while later STT/VAD evidence confirms the user speech boundary.

## Configuration

The Rust gateway reads XDG-compliant config from `$XDG_CONFIG_HOME/foxline/config.toml` or `~/.config/foxline/config.toml`, creating defaults on first run. Command-line flags override the config file and environment variables can be used for deployment overrides.

Generate the JSON schema with:

```bash
just gateway --print-config-schema
```

## `.foxline` loadouts

Project-local voice configuration lives under `.foxline/`; Pi-owned defaults remain under `.pi/`. The gateway resolves loadouts from `.foxline/loadouts/<name>.toml` or `.foxline/loadout.toml` for the default loadout, then resolves voice-only extensions from bundled `builtin:` references or project-local `.foxline/extensions/` paths.

See `docs/foxline-loadouts.md` for the current schema.

Bundled loadouts/personas are examples and shared defaults. Project-local `.foxline` loadouts are the override point for workspace-specific voice constraints, frontend tool requirements, adapter selection, and voice-only extensions.

## Pi RPC Brain adapter

The Rust gateway's Brain adapter launches only `pi --mode rpc`. Launch arguments are derived from the selected `.foxline` loadout:

- Pi profile/config/session directory fields become explicit Pi arguments.
- Allowed Pi tools become `--tools`.
- Resolved voice-only extensions become explicit `--extension` flags.
- `--no-context-files` is enabled by default from gateway config to keep voice startup behavior explicit.

The adapter maps Pi RPC JSONL events into canonical Brain frames:

- prompt send -> `brain.request_start`
- first text delta handling is measured by downstream trace consumers as `brain_first_token`
- text deltas -> `brain.text_delta`
- tool start/end -> `brain.tool_call` and `brain.tool_result`
- agent done -> `brain.done`
- Pi errors or rejected prompts -> `brain.error`

Warm Brain lifecycle supports prewarm, idle shutdown, restart, and full shutdown. Live prompt-to-TTS routing is completed in the STT/TTS/frontend wiring tasks.

## TTS adapter

The Rust gateway exposes TTS through an adapter trait. The first backend is `qwen3-worker`, which preserves the current `services/qwen3_tts_worker.py` binary protocol:

- input `speak`, `cancel`, and `shutdown` frames use the existing 9-byte `<type,u32 request_id,u32 payload_len>` header.
- worker `audio_start`, `audio_chunk`, `audio_done`, and `error` frames map to canonical TTS frames.
- PCM chunks also emit canonical binary audio frames for frontend transport.
- request start, first audio, and cancel events use trace names comparable to the legacy bridge.

Reserved backend names are `rust-mlx`, `rust-candle`, `cpp-ggml`, `elevenlabs`, and `openai`; they are explicit future adapter slots, not active Brain paths.

## STT adapter

The Rust gateway exposes STT through an adapter trait. The first backend is `parakeet-silero`, matching the current `services/parakeet_silero_ws_server.py` WebSocket protocol:

- binary messages carry PCM audio to the STT service.
- `word` and `interim` messages map to canonical STT partial frames.
- `final` messages map to VAD speech-stop evidence plus canonical STT final frames.
- status messages such as speech start/end map to VAD evidence frames.
- errors map to canonical STT error frames.

The turn manager consumes these VAD/STT frames as authoritative turn evidence. Trace events for first partial and final transcript use names comparable to the legacy bridge.

## Frontend clients

Foxline Codec UI, KITT, oqto, and future clients should all connect through the same gateway WebSocket protocol. Client-specific visuals stay in the client. The gateway only emits semantic events and Avatar Actions, and the client maps those to its own presentation.

Browser and Tauri clients should use the same WebSocket transport unless a measured latency or packaging issue requires a separate transport later.

## Avatar Actions

Avatar Actions are semantic presentation requests, not Codec/MGS sprite commands. Frontends advertise supported action names in their capability profile. The gateway routes only supported actions as `avatar_action` server events:

- `set_state`
- `set_expression`
- `focus`
- `play_animation`
- `clear`

The Foxline Codec UI maps these semantic actions to its own visual language; KITT, oqto, and future clients can map the same actions differently.

## Frontend tools

Frontend tools are negotiated at session startup. The effective tool surface is:

```text
(loadout.required_frontend + loadout.optional_frontend) ∩ frontend.advertised_tools
```

Missing required frontend tools fail session startup with a clear error. Optional tools are available only when the frontend advertises them. Tool calls that are not part of the negotiated surface are rejected by the gateway before reaching the frontend.

The gateway emits `frontend_tools_negotiated`, `frontend_tool_call`, `frontend_tool_rejected`, and accepts `frontend_tool_result` control messages. Pi can reach these through the voice-only frontend-tools extension path once the extension implementation is filled in.

## Why worker processes instead of HTTP services

- Lower latency: PCM can stream as soon as the worker emits it.
- Better interruption/barge-in: cancellation is a direct worker frame.
- Fewer ports and health checks.
- One runtime server owns character state, session state, brain, and TTS lifecycle.

## Python boundary

Python should be treated as a compatibility/development boundary, not the long-term runtime shape.

Currently Python remains for:

- `services/qwen3_tts_worker.py` until Rust Qwen3-TTS is production-ready.
- installer/extraction scripts under `tools/`.
- transcript helper service during asset installation.

Target binary runtime:

```text
foxline                       # Bun-compiled TypeScript runtime
foxline-stt-parakeet-silero   # native parakeet.cpp/Silero worker
foxline-tts-qwen3             # Rust Qwen3-TTS worker
```

## Model provisioning

Models should be downloaded at setup/first run and should reuse Hugging Face cache when possible. Release binaries should not embed large model weights by default.

Important models:

- Qwen3-TTS: `mlx-community/Qwen3-TTS-12Hz-0.6B-Base-4bit` for Python/MLX compatibility today.
- Future Rust Qwen3-TTS: track `/Users/tommyfalkowski/repos/qwen3_tts_rs_mario` and its supported model layout.
- Parakeet: `mudler/parakeet-cpp-gguf`.
- VAD: `ggml-org/whisper-vad` Silero GGML, unless the newer parakeet.cpp streaming model makes external VAD unnecessary.

## Repo layout

- `crates/protocol/` - Rust source of truth for Foxline Gateway protocol types.
- `crates/voice_gateway/` - Rust Voice Gateway runtime core.
- `packages/protocol/` - generated TypeScript protocol package (`@foxline/protocol`).
- `packages/voice-client/` - browser-clean TypeScript Client Core (`@foxline/voice-client`).
- `apps/codec/` - Codec Frontend Skin (web app today, Tauri target later).
- `services/` - worker/service adapters that are runtime-adjacent.
- `tools/` - install-time/extraction utilities only.
- `scripts/` - user-facing shell entrypoints.
- `agents/` - bundled demo agent/persona metadata only; generated voice/avatar assets remain local.
- `docs/` - architecture, migration, benchmark, extraction, and loadout notes.
