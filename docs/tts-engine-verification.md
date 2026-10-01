# TTS engine interchangeability: verification scope

Implementation is on `feat/tts-kokorox-contract`, based on `73187ff`. The existing cancellation fence was isolated as prerequisite `af64e1b`; unrelated main-checkout changes were not altered. This is component evidence, not a merged/deployed or physically audible conversation claim.

## Automated checks

```bash
just gateway-check
just gateway-test
just gateway-loadout-schema
```

Gateway check passes. The default gateway suite passes 96 library tests and nine executable/CLI tests; two opt-in library tests remain ignored. Tests cover:

- Independent worker framing, partial headers/bodies, exact raw German UTF-8, empty ready and sample rate in audio_start (including 48 kHz despite a 24 kHz configured hint).
- Ready timeout, startup protocol errors and bounded stderr-only diagnostics; draining 1 MiB of library chatter without pipe backpressure.
- Malformed/truncated output, EOF and PCM before audio_start reporting terminal errors rather than abandoning pending requests.
- Per-request error followed by success, cancellation of late output, already-queued completion/audio/error fences, failed cancel writes, shutdown/restart and dropping a pending prewarm future.
- Independent spqx/kokorox mappings, legacy Persona fixtures and metadata, mapping precedence, missing mappings, incomplete host overrides, unsupported fields/options and confined existing assets (traversal/symlink/relative-work-directory checks).
- CLI loadout schema includes native worker configuration.

Executable fixtures are std-only Rust programs compiled with the installed Rust toolchain. Lifecycle cancellation waits for a child-entry signal instead of racing cold OS executable launch. The executable suite passed three additional parallel repeat runs before the final restart test was added.

Unfiltered `cargo clippy -p foxline-voice-gateway --all-targets -- -D warnings` is blocked by pre-existing lint classes in Brain/config/control/frame/STT/WS. A focused run with those six classes allowed passes; this is not a clean-repository lint claim. Ripwire contract/review checks were run. Quality delta still flags constructor/test-scaffolding clones and test-module growth; no blanket acknowledgement was written and no clean quality-gate claim is made.

## Native German worker

Requires an independently built kokorox worker and compatible, locally installed assets:

```bash
export KOKOROX_TEST_WORKER=/absolute/path/to/kokorox-tts-worker
export KOKOROX_TEST_MODEL=/absolute/path/to/kokoro-martin.onnx
export KOKOROX_TEST_VOICES=/absolute/path/to/voices-martin.npz
export KOKOROX_TEST_VOICE=martin
export KOKOROX_TEST_LANGUAGE=de
export KOKOROX_TEST_TEXT='Guten Morgen. Dies ist ein echter Integrationstest mit einer deutschen Stimme.'
just gateway-kokorox-test
```

The ignored test installs assets into a temporary confined Persona package using hardlinks (copy fallback), resolves its mapping, builds native launch flags and drives the actual persistent process through gateway TTS frames. It checks non-empty/non-silent 24 kHz PCM16 mono, a cancelled long request, a subsequent successful request with no old-session frames, shutdown and real invalid-voice startup rejection. Request/cancel trace records are checked. No model/audio assets are checked in.

Recorded on Apple M2 Ultra, macOS arm64; debug gateway test harness, release kokorox worker `ef18d594`, ONNX CPU. Martin model/NPZ public revision: `Godelaune/Kokoro-82M-ONNX-German-Martin` at `a1cba7fbf0e72fbae38f0a3a48ce0dc8e6077804`.

| Component observation | Result |
| --- | --- |
| Ready/startup | 394 ms |
| Initial request PCM | 217,200 bytes (4.525 seconds) |
| Initial actual-PCM TTFA / synthesis RTF | 637 ms / 0.1409 |
| Post-cancel recovery actual-PCM TTFA / RTF | 3,566 ms / 0.7881 |
| Missing voice | Startup rejected with diagnostic |

These are single-run observations, not controlled comparative benchmarks. TTFA ends at the first `AudioFrame::OutputPcm`, not `audio_start`; RTF is request-to-done elapsed time divided by PCM duration. Recovery includes waiting behind cancelled, non-preemptible engine work. Fencing suppresses obsolete output; it does not promise instant kernel interruption. Martin is stochastic, so independent PCM equality is not an appropriate oracle.

## Outstanding proof

Both `fxl-8bht` and `fxl-00hv` remain open:

- Full gateway WebSocket/Pi/turn/barge-in/playback-completion proof, and integration with the unrelated pending gateway changes in the original checkout.
- One unchanged dual-mapped Persona synthesizing through both real engines. Current dual-mapping tests validate resolution/configuration, not real spqx synthesis.
- Correlated production per-request TTFA/RTF traces. WS audio-start/scheduling records identify the selected engine, including backend overrides; synthesis completion is not playback completion.
- Physical microphone/playback, other operating systems/GPU providers and cross-engine quality/performance comparisons.
- HTTP conformance (`spqx-wr4b`), license/provenance/distribution review (`kokorox-8j45`) and controlled off-Mac Qwen benchmarking (`spqx-wzrp`).

Standalone workers remain independently consumable. No linked kokorox dependency, shared speech SDK, HTTP rewrite or Qwen backend port was introduced.
