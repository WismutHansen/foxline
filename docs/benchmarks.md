# Benchmarks

Foxline keeps the current TypeScript bridge as the latency and behavior baseline until the Rust Voice Gateway proves parity.

## Legacy bridge trace

`server/bridge.ts` writes its normal session trace and also mirrors benchmark events to:

```text
benchmarks/traces/legacy-bridge/<character>-<timestamp>.jsonl
```

Override the location with:

```bash
CODEC_LEGACY_BENCHMARK_TRACE_DIR=/path/to/traces
CODEC_LEGACY_BENCHMARK_TRACE_FILE=/path/to/trace.jsonl
```

Comparable event names include:

- `mic_frame_received`
- `stt_partial`
- `stt_final`
- `brain_request_start`
- `brain_first_token`
- `tts_request_start`
- `tts_audio_start`
- `frontend_audio_play_scheduled`
- `barge_in_received`
- `tts_cancel_sent`

## Fixture runner

Start the legacy bridge, then run:

```bash
just benchmark-legacy-bridge
```

The runner sends `benchmarks/fixtures/legacy-bridge-utterance.json` to the bridge over WebSocket and waits for `turn_completed`. It expects the bridge, Pi RPC, and TTS/STT services needed by the demo to already be available.

The Rust gateway benchmark harness must compare against these JSONL events and clearly flag any end-to-end latency delta greater than +50ms.
