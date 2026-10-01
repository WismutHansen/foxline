---
status: accepted
---

# Unify TTS engines behind a shared worker contract, not a code merge

Foxline's TTS slot becomes engine-interchangeable at the contract level. spqx (Qwen3-TTS, MLX/tch, Apache-2.0) and kokorox (Kokoro-82M, ONNX, GPL-3.0) remain separate repositories, separate licenses, and separate worker processes. Unification happens through four shared surfaces: spqx's binary-framed worker protocol (made engine-agnostic), one OpenAI-compatible HTTP contract (kokorox-openai as the reference), a Persona voice manifest that resolves for either engine, and loadout backend names. Foxline never links kokorox code into its own binaries. Separate processes preserve a technical boundary; whether a distributed combination is legally separate requires a distribution-specific license review, not an automatic exemption.

## Decision

- **Two repositories stay split.** No merge of kokorox into spqx, and no shared engine crate containing engine code. Linking GPL-covered dependencies into a combined engine can impose GPL distribution obligations. Avoid such linkage in spqx; existing Apache-licensed components do not lose their own license merely because they are combined. The split also keeps two unrelated build matrices apart (mlx-c submodule + Metal toolchain vs ONNX Runtime + phonemizer).
- **The worker protocol is the primary contract.** spqx's persistent worker framing (`u8 type, u32 request_id, u32 payload_len` header; `speak`/`cancel`/`shutdown` in; `ready`/`audio_start`/`audio_chunk`/`audio_done`/`error` out; PCM16 LE mono; diagnostics/status on stderr) becomes the engine-agnostic TTS worker protocol. kokorox implements it as a new worker binary (kokorox-7ngw); foxline's `build_tts_adapter` accepts a kokorox backend name (fxl-8bht). Preserve the existing wire contract: `speak` carries raw UTF-8 text, `ready` has an empty payload, and each `audio_start` carries a four-byte little-endian sample rate. Voice, language and speed are launch configuration. Changing these payloads requires an explicit versioned extension, not a silent JSON dialect.
- **The HTTP contract has one dialect.** kokorox-openai's OpenAI-compatible API is the reference; spqx's api_server converges onto it (spqx-wr4b), with actual behavior verified by conformance tests rather than inferred from README maturity claims. Unsupported engine capabilities must be explicit errors or advertised differences, not silently ignored options. Engine differences (ICL reference voices vs named style packs) appear as data in the voices listing, never as renamed core fields.
- **Voices are declared engine-neutrally.** `persona.toml [voice]` gains engine-keyed resolution sections so one Persona package works under either backend, with loud failure instead of silent fallback (fxl-00hv).
- **License boundary rules.** kokorox runs only as a spawned worker process (implementation boundary). Foxline binaries must not statically or dynamically link kokorox crates. A small Apache-2.0 contract crate (types and a `TtsEngine` trait only, no engine code, no GPL dependencies) may be extracted later if duplication hurts; it must stay implementation-free.
- **License cleanup is tracked, not required.** kokorox-8j45 audits dependencies and source provenance, evaluates a subprocess or permissive G2P, and determines what can lawfully be distributed under Apache-2.0. Removing a dependency alone does not authorize relicensing existing GPL-covered code; contributor rights and retained notices also matter. Misaki's own permissive license does not imply all optional backends are permissive. Browser packaging requires a separate license review; GPL does not categorically prohibit a browser implementation.

## Implementation shape

The gateway selects `tts = "kokorox"` and optionally `[adapters.kokorox].worker`; a Persona declares `[voice.kokorox]` with confined local `model`/`voices` files, exact `voice`, explicit `language`, and positive `speed`. `[voice.spqx]` supplies reference audio/transcript to either Qwen worker. Legacy reference fields remain compatible; missing mappings and unsupported nested options fail without engine fallback. See [loadouts](../foxline-loadouts.md) and [verification scope](../tts-engine-verification.md).

The worker adapter preserves cancellation-generation fencing and waits for valid empty ready before prewarm succeeds. The real native Martin adapter turn/cancel/recovery is verified; full gateway WebSocket conversation/playback and real same-Persona synthesis through both engines remain outstanding. Neither worker startup nor synthesis completion is evidence of frontend playback completion.

## Context

Foxline's default TTS is the spqx worker (`rust-mlx`): Qwen3-TTS-12Hz-0.6B via Rust MLX, ~0.20 RTF and ~96 ms time-to-first-audio on Apple Silicon. Two needs crossed:

1. **German voice.** Kokoro-82M has German finetunes with permissive licenses; the most drop-in compatible is `crane-local-ai/Kokoro-82M-v1.0-German-ONNX` (ONNX model + `voices/df_kerstin.bin` as raw float32 style rows, same declared `input_ids`/`style`/`speed` contract, Apache-2.0 model card). kokorox already ships the German plumbing (espeak-ng `de` phonemization, German number/unit normalization); model compatibility still needs a real run. kokorox's existing `.bin` packs are NPZ archives containing named three-dimensional arrays; Crane's raw voice file must be converted, whereas Martin's NPZ still requires shape validation. Vanilla Kokoro's German was weak enough that kokorox removed German from its supported-language list.
2. **Non-Mac deployment.** spqx's fast path is MLX (Apple Silicon only); the libtorch `tch` backend exists for Linux/CUDA but is not the tuned path. Cross-platform Qwen3-TTS backends exist (vLLM-Omni officially, ONNX exports of the 0.6B, llama.cpp/GGUF ports) but no controlled same-model, same-workload comparison was performed here; relative speed remains an experiment rather than an established result.

kokorox targets cross-platform ONNX Runtime (Linux/macOS/Windows); its README attributes GPL-3.0 to `espeak-rs` statically linking espeak-ng. That is not proof that dependency replacement alone clears retained-source licensing. ort and the inspected model cards declare permissive licenses. No complete transitive-dependency or source-provenance audit has been performed; the README's GPL statement and Cargo metadata also disagree and require reconciliation. That makes the license the deciding constraint on unification shape rather than a blocker on unification itself.

The worker-process boundary also keeps the gateway's invariants intact: turn authority, barge-in, cancellation generation fencing (see `fxl-xp29`), and tracing are gateway-owned regardless of which engine produces audio, exactly as ADR 0003 keeps them gateway-owned under client-owned speech. This ADR changes where audio synthesis runs, not who owns the session.

## Consequences

- Foxline loadouts gain a third `tts` backend name for kokorox alongside `rust-mlx` and `qwen3-worker`; docs move to `docs/foxline-loadouts.md`.
- The worker protocol gains normative status: raw-text speak, empty ready, sample rate in audio_start, frame types and cancellation behavior must remain compatible. Engine-specific voice/language/speed flags live in adapter construction. Queued and in-flight requests must be fenced after cancellation; an ONNX inference call need not be preemptible, but its obsolete output must not escape.
- Benchmark traces must distinguish the engine path so TTFA/RTF comparisons (spqx MLX vs kokorox ONNX CPU/GPU vs spqx tch) stay honest per host.
- kokorox German voices install as voice packs; Persona packages targeting German kokorox must pin the finetune they were validated against, because finetunes are single-speaker and not interchangeable with the multilingual base.
- Before distributing a bundle containing both engines, review the combination, retain notices and fulfill applicable source/license obligations. A subprocess boundary is not by itself proof of compliance.
- If a provenance/dependency audit establishes a lawful permissive distribution path, a merge may be reconsidered; it is still not required for interchangeability.
- Other spqx execution backends, or a remote Qwen3-TTS service behind the same contract, can be adopted without touching this decision. Controlled off-Mac benchmarking is tracked by `spqx-wzrp`; this ADR does not move Qwen inference into kokorox just because both could use ONNX.

## Considered options

- **Merge kokorox into spqx as a second engine behind one roof:** rejected; linked GPL dependency obligations in a combined engine and a combined build matrix (Metal toolchain + mlx-c submodule + espeak) that is hostile to cross-platform CI. The user-facing goal is interchangeability, which a contract provides without a merge.
- **Merge after the GPL exit:** deferred; subject to the provenance and licensing outcome of kokorox-8j45, and explicitly not required. The contract-first shape already delivers drop-in behavior and keeps CI matrices simple.
- **Keep the status quo (two engines, two protocols):** rejected; every host integration would need engine-specific glue, and switching engines would be a config plus code change instead of a config change.
- **Serve kokorox only via its OpenAI/WebSocket servers:** rejected as the primary path; the binary worker protocol is the low-latency path foxline already runs spqx on, and adding a socket hop for kokorox would make the engines non-equivalent in latency character.
- **Unify at the model layer only (Kokoro in spqx's MLX runtime):** rejected for this milestone; spqx currently has no Kokoro implementation. Adding another model runtime is unnecessary to establish interchangeability and does not by itself validate other-platform performance.
