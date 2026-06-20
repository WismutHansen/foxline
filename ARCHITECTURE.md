# Foxline architecture

Foxline follows PiBot's low-latency worker architecture rather than HTTP microservices.

```text
browser UI
  -> server/bridge.ts
      -> pi-rpc process
      -> Qwen3-TTS worker process
      -> parakeet.cpp + Silero STT worker process/service
```

During the Rust Voice Gateway migration, `server/bridge.ts` remains the latency and behavior baseline. The Rust gateway lives in `crates/voice_gateway` and is introduced behind a separate WebSocket endpoint until benchmark parity is proven.

```text
frontend client
  -> foxline-voice-gateway WebSocket
      -> canonical frame pipeline
      -> Pi RPC Brain adapter
      -> STT adapter
      -> TTS adapter
      -> frontend tool/avatar action routers
```

The gateway never calls LLM servers directly. Pi is always the Brain path and owns model/provider access, tools, skills, extensions, prompt templates, and session behavior.

## Runtime defaults

- Brain: pi-rpc only.
- TTS: Qwen3-TTS only.
- STT: parakeet.cpp by default, currently with Silero VAD for turn detection.

## Rust Voice Gateway protocol

The gateway frontend transport is WebSocket:

- JSON text messages carry control and event frames such as `hello`, `start_session`, `end_session`, `vad_hint`, `interrupt`, and errors.
- Binary messages carry PCM audio frames on the normal path. Audio must not be base64 encoded in JSON.
- Frontends declare capabilities before session startup. The current shell records this profile and follow-up work will negotiate tool and avatar action surfaces against `.foxline` loadouts.
- Debug trace streaming is gated by configuration; server-side JSONL traces remain the default.

Trace events use names comparable to the legacy bridge path, including `mic_frame_received`, `stt_partial`, `stt_final`, `brain_request_start`, `brain_first_token`, `tts_request_start`, `tts_audio_start`, `frontend_audio_play_scheduled`, `barge_in_received`, and `tts_cancel_sent`.

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

- `server/` - app runtime server.
- `crates/voice_gateway/` - Rust Voice Gateway runtime core and WebSocket protocol.
- `services/` - worker/service adapters that are runtime-adjacent.
- `tools/` - install-time/extraction utilities only.
- `scripts/` - user-facing shell entrypoints.
