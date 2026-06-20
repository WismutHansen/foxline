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

Start the Rust gateway with benchmark-local traces, then run:

```bash
FOXLINE_GATEWAY_TRACE_DIR=benchmarks/traces/rust-gateway just gateway
just benchmark-rust-gateway
```

The Rust fixture declares the same frontend capability profile as the Codec UI, starts a session, sends a deterministic silent PCM frame, triggers an interrupt, and closes the session. Until the full Rust STT/Brain/TTS path is wired end to end, the latency comparator will report missing downstream milestones instead of treating them as a pass.

For a service-backed smoke run, provide a local ignored PCM16 mono utterance fixture:

```bash
mkdir -p benchmarks/traces/local-fixtures
# Put a short user-owned PCM16 mono WAV or raw .s16le/.pcm utterance here.
# Do not commit this file; benchmarks/traces/ is ignored.
FOXLINE_GATEWAY_TRACE_DIR=benchmarks/traces/rust-gateway just gateway
just benchmark-rust-gateway --audio-file=benchmarks/traces/local-fixtures/utterance.wav --wait-for-assistant --wait-for-audio
```

The audio fixture streams the utterance as binary PCM frames, sends trailing silence to let STT close the turn, then waits for assistant deltas, `turn_completed`, binary output PCM, and trace discovery. Raw `.pcm`/`.s16le` input defaults to 24 kHz and can be overridden with `--raw-pcm-sample-rate=<hz>`.

To exercise cancellation after the first output audio frame:

```bash
just benchmark-rust-gateway --audio-file=benchmarks/traces/local-fixtures/utterance.wav --wait-for-assistant --wait-for-audio --interrupt-on-audio
just benchmark-legacy-bridge --interrupt-on-audio
```

The Rust fixture reports the newest trace that contains the milestones requested by the run, so short protocol-only sessions do not mask a completed audio smoke trace. The legacy fixture writes `benchmark_fixture_start`/`benchmark_fixture_done` client trace markers; the comparator uses the latest completed fixture window in a session-long trace.

## Latency comparator

Compare the newest legacy and Rust traces:

```bash
just benchmark-gateway-latency
```

Or pass explicit trace files:

```bash
just benchmark-gateway-latency --legacy-trace=benchmarks/traces/legacy-bridge/campbell.jsonl --rust-trace=benchmarks/traces/rust-gateway/session.jsonl
```

The comparator writes a machine-readable summary to `benchmarks/traces/latency-summary-<timestamp>.json` and exits non-zero when a metric is missing or the Rust trace is more than 50ms slower than the legacy trace. Override the budget with `--threshold-ms=<ms>`.

Measured metrics:

- `stt_final`
- `brain_first_token`
- `tts_audio_start`
- `frontend_audio_play_scheduled`
- `barge_in_cancel` (`tts_cancel_sent - barge_in_received`)

Current proof status:

- A service-backed Rust gateway smoke run has passed through frontend WebSocket, Parakeet/Silero STT, gateway turn commit, Pi RPC Brain, Qwen TTS worker, and binary output PCM.
- `brain_first_token`, `tts_audio_start`, and `frontend_audio_play_scheduled` measured faster than the available legacy text-injection fixture window in the captured traces.
- `stt_final` is not yet comparable against the legacy bridge fixture because the legacy runner injects text instead of replaying microphone audio through the legacy STT path.
- `barge_in_cancel` remains unproven until both paths produce paired `barge_in_received` and `tts_cancel_sent` markers in clean interrupt fixture windows.

To run fixture commands and then compare in one invocation:

```bash
just benchmark-gateway-latency --legacy-command="just benchmark-legacy-bridge" --rust-command="just benchmark-rust-gateway"
```
