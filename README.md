![banner](banner.png)

# foxline

Foxline is a local Metal Gear Solid-style codec runtime: voice input (STT), character-driven dialogue via Pi RPC, and streamed voice output (TTS) in a browser UI.

Default runtime models/components:

- Brain (LLM): whatever `CODEC_PI_MODEL` points to in your local Pi model registry (example in this README uses LM Studio `gemma-4-26b-a4b-it`)
- STT: Parakeet + Silero VAD
- TTS: native [spqx](https://github.com/byteowlz/spqx) Qwen3-TTS worker (`rust-mlx` backend; ~96ms to first audio, 0.3s model load). The Python MLX worker (`services/qwen3_tts_worker.py`) remains as a fallback via `FOXLINE_TTS_BACKEND=qwen3-worker` or a loadout's `[adapters] tts` entry.

Current platform status:

- Mac-first at the moment (MLX-based STT/TTS workers) but hopefully won't be too hard to port to other platforms
- Fully local runtime: STT/TTS/LLM can run entirely on your machine (when backed by local providers/models). ~30GB of shared memory required.
- The `rust-mlx` TTS backend expects the spqx worker binary: a sibling `../spqx` release build is found automatically; otherwise set `FOXLINE_TTS_RUST_WORKER` or put `spqx-tts-worker` on `PATH`.

> [!IMPORTANT]
> This repo does not ship copyrighted Metal Gear Solid assets. You must provide legally obtained source media (PS1 NTSC/US discs, GOG installer, or an existing PC install directory).

## Quick start

1. Put source media in `./sources/`.
2. Run installer (takes a while for all steps, be patient):

```bash
scripts/install-assets.sh -y
```

3. Start full Rust gateway runtime (tmux + STT + Rust gateway + frontend):

```bash
just demo-rust
```

## Source media options

```bash
scripts/install-assets.sh --pc-installer "/path/to/setup_metal_gear_solid_1.0.exe" -y
scripts/install-assets.sh --pc-dir "/path/to/MGS PC directory" -y
scripts/install-assets.sh --psx-disc1 "/path/to/disc1.cue" --psx-disc2 "/path/to/disc2.cue" -y
```

Useful flags:

```bash
scripts/install-assets.sh --skip-setup
scripts/install-assets.sh --no-services
scripts/install-assets.sh --no-transcribe
scripts/install-assets.sh --no-fillers
scripts/install-assets.sh --force
```

## Runtime entrypoints

- Setup deps only: `scripts/setup-deps.sh`
- Start STT/transcript services only: `scripts/start-services.sh`
- Start Codec, STT, and the Rust gateway in tmux: `just demo-rust`
- Start Overlayz, STT, and the Rust gateway: `just run overlayz`
- Run the installed gateway/STT stack in the foreground: `foxline service run`
- Manage macOS login startup: `foxline service enable --now`, `status`, `restart`, or `disable --now`

Install the management CLI from a checkout:

```bash
cargo install --path crates/foxline_cli
scripts/sync-personas-to-xdg.sh
```

The sync command treats `agents/` as the reference source, copies prompts plus any local ignored voice/canned audio into the user's XDG Persona store, and never stages those assets in this repository.

Run in the foreground while testing, or enable the macOS LaunchAgent:

```bash
foxline service run
foxline service enable --now
foxline service status
```

`foxline service run` supervises gateway-owned STT and the Voice Gateway; frontends start separately and any number may connect. `start` requires an enabled LaunchAgent, while `disable --now` stops and removes login startup.

The legacy TypeScript bridge is retired (ADR 0004); the Rust Voice Gateway is the sole gateway.

## Prerequisites (macOS manual)

```bash
brew install bun uv p7zip ffmpeg innoextract cmake git tmux
brew install rom-tools   # for chdman (.chd support)
```

## Pi model config

Foxline uses `pi --mode rpc` as the brain. Configure models in:

- `~/.pi/agent/models.json`

Set runtime model:

```bash
export CODEC_PI_MODEL="<provider>/<model-id>"
```

Minimal LM Studio provider example:

```json
{
  "providers": {
    "LM-Studio": {
      "baseUrl": "http://localhost:1234/v1",
      "api": "openai-completions",
      "apiKey": "not-needed",
      "models": [{ "id": "gemma-4-26b-a4b-it" }]
    }
  }
}
```

## Development

```bash
bun install
bun run dev
```

Optional shortcuts:

```bash
just install
just dev
just demo-rust
just gateway-check
just gateway -- --bind 127.0.0.1:8780
```

The Rust Voice Gateway in `crates/voice_gateway` is the sole gateway and Brain path (ADR 0004); `just demo-rust` is the primary demo path. See `ARCHITECTURE.md`, `docs/vision.md`, `docs/benchmarks.md`, and `docs/foxline-loadouts.md`.

## Generated outputs

- `assets/generated/mgs/`
- `assets/generated/mgs_pc/`
- `agents/<agent>/assets/avatar/`
- `agents/<agent>/assets/reference_audio/`
- `agents/<agent>/assets/filler/`

Installer manifests/crosswalks:

- `assets/generated/mgs_install_manifest.json`
- `assets/generated/mgs_pc/vox-psx-crosswalk.json`
- `assets/generated/mgs_pc/vox-psx-crosswalk.csv`

Shipped metadata only (no audio/game art):

- `assets/mgs-vox-bank-index.json`
- `assets/face-name-map.json`
- `assets/reference_transcript_replacements.json`

## Attributions

Foxline builds on and learns from these upstream projects:

- [Pi coding agent (pi.dev)](https://pi.dev) by Mario Zechner and contributors: local coding-agent runtime used in RPC mode as Foxline's character brain interface.
- [PiBot / Pipi](https://github.com/badlogic/pibot) by Mario Zechner: worker-based local voice assistant architecture, parakeet.cpp STT worker integration, and Qwen3-TTS worker design.
- [parakeet.cpp](https://github.com/mudler/parakeet.cpp) by Ettore Di Giacinto and contributors: local C/C++ Parakeet/Nemotron ASR inference and GGUF model support.
- [qwen3_tts_rs](https://github.com/badlogic/qwen3_tts_rs) by Mario Zechner: origin of [spqx](https://github.com/byteowlz/spqx), the byteowlz fork that is now Foxline's default TTS engine (`rust-mlx` backend).
