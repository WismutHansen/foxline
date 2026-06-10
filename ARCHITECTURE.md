# Foxline architecture

Foxline follows PiBot's low-latency worker architecture rather than HTTP microservices.

```text
browser UI
  -> server/bridge.ts
      -> pi-rpc process
      -> Qwen3-TTS worker process
      -> parakeet.cpp + Silero STT worker process/service
```

## Runtime defaults

- Brain: pi-rpc only.
- TTS: Qwen3-TTS only.
- STT: parakeet.cpp by default, currently with Silero VAD for turn detection.

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
- `services/` - worker/service adapters that are runtime-adjacent.
- `tools/` - install-time/extraction utilities only.
- `scripts/` - user-facing shell entrypoints.
