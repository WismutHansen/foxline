# Foxline vision: the on-device voice runtime for agentic assistants

Foxline's differentiator is already decided by its ADRs: a Rust gateway that owns turns,
latency, and speech I/O, with an agentic Brain (Pi) that owns reasoning and tools.
Pipecat and LiveKit Agents are cloud-first Python orchestrators; Foxline can be the
thing they are not — **a single local binary that turns any machine into a private,
sub-second, interruptible voice interface to real agents.**

Everything below serves one sentence: *press-to-talk to first audio in under 300ms,
fully on-device, without leaking a single token of reasoning, and without a Python
process in sight.*

## Where we are

Proven: the Rust gateway beats the legacy TS bridge on every latency metric
(stt_final −260ms, brain_first_token −223ms, tts_audio_start −266ms). The frame
pipeline, trait-based STT/TTS adapters, turn manager, protocol codegen (Rust → TS),
agent/persona split, and loadout system are solid foundations.

Honest gaps, from the code and the commit history:

1. **Legacy defensive filtering survived its own fix.** Six commits (fxl-b7tj,
   fxl-jwv6, fxl-7zzy, fxl-7yqd ×3) fought duplicated deltas, replayed answers, and
   leaked reasoning. Pi now owns per-model normalization and separates thinking and
   text into distinct content blocks — the fxl-7yqd fixes established that structured
   boundary — but the older heuristic layers were never deleted: tag stripping and
   prose-marker heuristics in `brain.rs`, the UI-side dedup, and the unused
   `think_block_filter.rs` module. Redundant filters are not harmless; they are the
   ad-hoc layers that caused the replay/duplication bugs in the first place.
2. **Speech is still sidecar Python.** STT is a Python WebSocket wrapper around
   parakeet.cpp; TTS is a Python MLX worker. Workers die fatally with no restart, no
   health checks, no fallback chain. The "target binary runtime" in ARCHITECTURE.md
   is documented, not built.
3. **Latency has unclaimed headroom.** 800ms silence timeout with no semantic
   endpointing; a 28-char minimum before the first TTS chunk; a 15ms polling tick
   instead of an event-driven pipeline; `barge_in_confirmation_window_ms` is config
   that no code reads; STT connections and Brains cold-start on first use.
4. **Structural debt.** `ws.rs` is a 1,567-line monolith; the Brain has no trait
   (ADR 0003's conditional adapters need one); protocol version is hardcoded to 1;
   no backpressure; traces are per-session JSONL with no aggregated metrics; no
   end-to-end streaming-content regression suite.

## Four pillars

### 1. Trust Pi's contract; delete the legacy filters

Per-model quirks are **Pi's job, and Pi already does it**: the RPC stream separates
thinking and text into distinct content blocks. The gateway's contract is that
structured boundary — filter by block type at the RPC mapper, once, and treat
everything Pi marks as text as speakable.

The remaining work is deletion, not construction: remove the heuristic tag stripping
and prose-marker scanning in `brain.rs`, the unused `think_block_filter.rs` module,
and the frontend-side dedup. Downstream consumers — TTS, frontend, transcript —
receive canonical frames and never re-filter.

Real Pi RPC streams (the fxl-7yqd fixtures are the seed) become a replay regression
suite guarding that single boundary: every past leak is a fixture, and any future
leak is a Pi-side normalization bug caught by fixtures, not a reason to grow a new
gateway filter layer.

### 2. Native, in-process speech — the single binary

Drop Python completely. Replace the sidecars with in-process engines behind the
existing adapter traits:

- **VAD**: Silero via ONNX/candle in-process — no WebSocket hop for the most
  latency-critical signal in the system.
- **STT**: parakeet.cpp via FFI (or a Rust port) streaming in-process.
- **TTS**: the hard constraint is **voice cloning parity** — a Rust backend
  (`qwen3_tts_rs_mario` or others) ships only if it matches the current MLX worker's
  reference-audio cloning quality. Rather than betting on one implementation, the
  TTS trait gets a clean enough seam that candidate backends (rust-mlx, rust-candle,
  cpp-ggml, Kokoro-class fallbacks) are cheap to trial and A/B against the cloning
  baseline. The Python worker stays exactly as long as it is the best cloning
  backend, and not a day longer.
- **Brain**: extract a `Brain` trait; `PiRpcBrain` stays the only shipped path
  (per ADR 0001), but the seam enables supervision, pooling, and testing.

A **model manager** makes on-device real rather than aspirational: declarative model
registry, Hugging Face cache reuse, integrity pinning, device-aware quantization
selection (detect RAM/GPU, pick 4/6/8-bit), lazy load with prewarm, and fallback
chains when a model is missing or too large. First run downloads; every run after is
offline.

Result: `foxline` is one binary plus a model cache. No uv, no tmux, no orphaned
processes, no ports 8796/8780.

### 3. Latency as a measured product feature

- **Semantic endpointing**: commit turns on partial-transcript stability and
  linguistic completeness, not only an 800ms silence clock. Evaluate a small
  turn-detection model as VAD's successor.
- **Event-driven pipeline**: replace the 15ms tick with wakeups; frames flow the
  moment adapters produce them.
- **Streaming-first TTS**: synthesize from the first stable clause instead of a
  28-char buffer; speculative synthesis of the first sentence while the Brain is
  still streaming.
- **Real barge-in**: implement the confirmation window that exists only in config;
  duck output on speech-start evidence, cancel on confirmation.
- **Latency budgets in CI**: the benchmark harness already exists — give each stage
  a budget (VAD decision, stt_final, brain_first_token, tts_first_audio) and fail CI
  on regression. Publish the numbers; make "fastest local voice gateway" a claim with
  receipts.

Target on Apple Silicon: **end-of-speech → first audible sample < 300ms**, barge-in
cancel < 50ms.

### 4. A dependable platform, not a demo

- **Supervision**: adapters restart with backoff, health-checked; a dead TTS engine
  degrades to text, never kills the session.
- **Backpressure and bounded queues** through the pipeline; slow consumers shed
  partials, never audio.
- **Protocol v2 with real version negotiation**, then implement ADR 0003
  (client-owned STT/TTS) — that is the door to mobile and browser-edge deployments
  where the client brings Silero/Kokoro and the gateway stays the turn authority.
- **Observability**: aggregate per-stage latency metrics from the existing traces;
  expose a local metrics endpoint; every bug report is a replayable trace.
- **Packaging**: Tauri sidecar bundling for Codec/Overlayz, one-command install,
  models fetched on first run.

## Sequencing

1. **Foundation** — delete legacy filter layers, keep the single structured RPC
   boundary, build the replay regression suite; extract Brain trait; decompose
   `ws.rs` into session/normalization/routing modules. *(Stops the recurring bug
   class; unblocks everything else.)*
2. **Native speech** — in-process Silero VAD, parakeet FFI STT, modular TTS backend
   trials gated on voice-cloning parity, model manager. *(Deletes Python entirely.)*
3. **Latency** — semantic endpointing, event-driven pipeline, streaming-first TTS,
   real barge-in, CI latency gates. *(Makes the headline numbers.)*
4. **Platform** — protocol v2, ADR 0003 implementation, supervision, metrics,
   packaged installs. *(Makes it shippable beyond this repo.)*

The order matters: pillar 1 is cheap and removes the tax every other change currently
pays; pillar 2 is the strategic moat; pillar 3 is the marketing headline; pillar 4 is
what lets other people build on it.
