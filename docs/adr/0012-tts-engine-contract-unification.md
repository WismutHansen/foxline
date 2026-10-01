---
status: accepted
---

# Unify TTS engines behind a shared worker contract, not a code merge

Foxline's TTS slot becomes engine-interchangeable at the contract level. spqx (Qwen3-TTS, MLX/tch, Apache-2.0) and kokorox (Kokoro-82M, ONNX, GPL-3.0) remain separate repositories, separate licenses, and separate worker processes. Unification happens through four shared surfaces: spqx's binary-framed worker protocol (made engine-agnostic), one OpenAI-compatible HTTP contract (kokorox-openai as the reference), a Persona voice manifest that resolves for either engine, and loadout backend names. Foxline never links kokorox code into its own binaries; the GPL stays confined to the kokorox worker process, where it is mere aggregation.

## Decision

- **Two repositories stay split.** No merge of kokorox into spqx, and no shared engine crate containing engine code. Merging would turn the unified engine GPL-3.0 (kokorox's espeak-rs dependency statically links espeak-ng), forfeiting spqx's Apache-2.0 and infecting any distributed foxline bundle. The split also keeps two unrelated build matrices apart (mlx-c submodule + Metal toolchain vs ONNX Runtime + phonemizer).
- **The worker protocol is the primary contract.** spqx's persistent worker framing (`u8 type, u32 request_id, u32 payload_len` header; `speak`/`cancel`/`shutdown` in; `ready`/`audio_start`/`audio_chunk`/`audio_done`/`error` out; PCM16 LE mono; JSON status on stderr) becomes the engine-agnostic TTS worker protocol. kokorox implements it as a new worker binary (kokorox-7ngw); foxline's `build_tts_adapter` accepts a kokorox backend name (fxl-8bht). Sample-rate negotiation belongs in the `ready` frame because Kokoro is 24 kHz and Qwen3-TTS is configurable.
- **The HTTP contract has one dialect.** kokorox-openai's OpenAI-compatible API is the reference; spqx's early api_server converges onto it (spqx-wr4b). Engine differences (ICL reference voices vs named style packs) appear as data in the voices listing, never as renamed core fields.
- **Voices are declared engine-neutrally.** `persona.toml [voice]` gains engine-keyed resolution sections so one Persona package works under either backend, with loud failure instead of silent fallback (fxl-00hv).
- **License boundary rules.** kokorox runs only as a spawned worker process (GPL process boundary). Foxline binaries must not statically or dynamically link kokorox crates. A small Apache-2.0 contract crate (types and a `TtsEngine` trait only, no engine code, no GPL dependencies) may be extracted later if duplication hurts; it must stay implementation-free.
- **The GPL exit is tracked, not required.** kokorox replaces the espeak-rs static link (subprocess espeak-ng, or Crane's MIT German G2P for German; misaki is Apache but has no German) and can then relicense to Apache-2.0 (kokorox-8j45). That removes the process-boundary caveat and would make a merge possible in the future — but a merge remains unnecessary under this ADR even then.

## Context

Foxline's default TTS is the spqx worker (`rust-mlx`): Qwen3-TTS-12Hz-0.6B via Rust MLX, ~0.20 RTF and ~96 ms time-to-first-audio on Apple Silicon. Two needs crossed:

1. **German voice.** Kokoro-82M has German finetunes with permissive licenses; the most drop-in compatible is `crane-local-ai/Kokoro-82M-v1.0-German-ONNX` (ONNX model + `voices/df_kerstin.bin` in the standard voice-pack format, same `input_ids`/`style`/`speed` contract, Apache-2.0). kokorox already ships the German plumbing (espeak-ng `de` phonemization, German number/unit normalization); only the model was missing. Vanilla Kokoro's German was weak enough that kokorox removed German from its supported-language list.
2. **Non-Mac deployment.** spqx's fast path is MLX (Apple Silicon only); the libtorch `tch` backend exists for Linux/CUDA but is not the tuned path. Cross-platform Qwen3-TTS backends exist (vLLM-Omni officially, ONNX exports of the 0.6B, llama.cpp/GGUF ports) but none currently matches MLX single-stream latency.

kokorox is the cross-platform engine (ONNX Runtime on Linux/macOS/Windows) and is GPL-3.0 for exactly one reason: `espeak-rs` statically linking espeak-ng. Everything else in the Kokoro path is permissive (ort is MIT; Kokoro weights and the German finetunes are Apache-2.0; misaki, the official Kokoro G2P, is Apache-2.0). That makes the license the deciding constraint on unification shape rather than a blocker on unification itself.

The worker-process boundary also keeps the gateway's invariants intact: turn authority, barge-in, cancellation generation fencing (see `fxl-xp29`), and tracing are gateway-owned regardless of which engine produces audio, exactly as ADR 0003 keeps them gateway-owned under client-owned speech. This ADR changes where audio synthesis runs, not who owns the session.

## Consequences

- Foxline loadouts gain a third `tts` backend name for kokorox alongside `rust-mlx` and `qwen3-worker`; docs move to `docs/foxline-loadouts.md`.
- The worker protocol gains normative status: frame types, cancel semantics, and the `ready` sample-rate field must be treated as a versioned contract. Engine-specific flags live in adapter construction, not in the frame stream; anything an engine needs beyond the shared speak payload (voice id, language, speed) must be expressible there or the contract is wrong.
- Benchmark traces must distinguish the engine path so TTFA/RTF comparisons (spqx MLX vs kokorox ONNX CPU/GPU vs spqx tch) stay honest per host.
- kokorox German voices install as voice packs; Persona packages targeting German kokorox must pin the finetune they were validated against, because finetunes are single-speaker and not interchangeable with the multilingual base.
- Distributing foxline with both engines is license-clean: Apache-2.0 gateway plus a GPL worker binary the user's host launches, with kokorox's license and source obligations attached to that binary alone.
- If kokorox relicenses to Apache-2.0, the process-boundary caveat is dropped from the docs and a merge may be reconsidered; nothing else in this ADR changes.
- Cross-platform Qwen3-TTS (ONNX export inside kokorox's runtime, or vLLM-Omni as a remote engine behind the same contract) can be adopted later without touching this decision, because the contract, not the engine, is what foxline depends on.

## Considered options

- **Merge kokorox into spqx as a second engine behind one roof:** rejected; GPL contamination of an Apache engine and a combined build matrix (Metal toolchain + mlx-c submodule + espeak) that is hostile to cross-platform CI. The user-facing goal is interchangeability, which a contract provides without a merge.
- **Merge after the GPL exit:** deferred; possible once kokorox-8j45 lands, and explicitly not required. The contract-first shape already delivers drop-in behavior and keeps CI matrices simple.
- **Keep the status quo (two engines, two protocols):** rejected; every host integration would need engine-specific glue, and switching engines would be a config plus code change instead of a config change.
- **Serve kokorox only via its OpenAI/WebSocket servers:** rejected as the primary path; the binary worker protocol is the low-latency path foxline already runs spqx on, and adding a socket hop for kokorox would make the engines non-equivalent in latency character.
- **Unify at the model layer only (Kokoro in spqx's MLX runtime):** rejected for now; there is no Kokoro MLX implementation, and writing one duplicates kokorox's working ONNX path while remaining Apple-Silicon-only, which is the exact limitation this ADR works around.
