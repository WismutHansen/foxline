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

For an audio/STT baseline, provide the same local ignored PCM16 mono utterance used by the Rust fixture:

```bash
just benchmark-legacy-bridge --audio-file=benchmarks/traces/local-fixtures/utterance.wav --interrupt-on-audio
```

The legacy bridge does not accept binary microphone frames directly; in the demo, browser STT turns microphone audio into a final `user_utterance`. The audio fixture reproduces that boundary by streaming Float32 PCM to the Parakeet/Silero STT WebSocket, writing `client_stt_audio_frame_sent` trace markers for comparable `mic_frame_received` aliases, then sending the final transcript to the bridge.

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

The audio fixture streams the utterance as binary PCM frames, sends trailing silence to let STT close the turn, then waits for assistant deltas, `turn_completed`, binary output PCM, and trace discovery. Raw `.pcm`/`.s16le` input defaults to 24 kHz and can be overridden with `--raw-pcm-sample-rate=<hz>`. Pass `--agent=<agent>` and `--persona=<persona>` to keep the Brain agent fixed while changing the TTS voice/persona.

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

- A service-backed Rust gateway smoke run has passed through frontend WebSocket, Parakeet/Silero STT, gateway turn commit, Pi RPC Brain, Qwen TTS worker, binary output PCM, and frontend interrupt cancellation.
- The legacy audio baseline fixture now replays the same ignored utterance through the Parakeet/Silero STT service before sending `user_utterance` to the bridge.
- A clean comparator run passed the default +50ms budget with `benchmarks/traces/legacy-bridge/campbell-2026-06-20T23-23-09-307Z.jsonl` and `benchmarks/traces/rust-gateway/569b58ad-d621-431b-80e5-cc5a8ec6a718.jsonl`: `stt_final` -260ms, `brain_first_token` -223ms, `tts_audio_start` -266ms, `frontend_audio_play_scheduled` -273ms, and `barge_in_cancel` 0ms.

To run fixture commands and then compare in one invocation:

```bash
just benchmark-gateway-latency --legacy-command="just benchmark-legacy-bridge" --rust-command="just benchmark-rust-gateway"
```
