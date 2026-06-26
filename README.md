![banner](banner.png)

# foxline

Foxline is a local Metal Gear Solid-style codec runtime: voice input (STT), character-driven dialogue via Pi RPC, and streamed voice output (TTS) in a browser UI.

Default runtime models/components:

- Brain (LLM): whatever `CODEC_PI_MODEL` points to in your local Pi model registry (example in this README uses LM Studio `gemma-4-26b-a4b-it`)
- STT: Parakeet + Silero VAD
- TTS: Python Qwen3-TTS worker using MLX (`services/qwen3_tts_worker.py`)

Current platform status:

- Mac-first at the moment due to the Python/MLX Qwen3-TTS worker setup but hopefully won't be too hard to port to other platforms
- Fully local runtime: STT/TTS/LLM can run entirely on your machine (when backed by local providers/models). ~30GB of shared memory required.
- The Rust Qwen3-TTS implementation is not the current default runtime worker in this repo.

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
- Start full Rust gateway app in tmux: `just demo-rust`
- Start legacy bridge app in tmux: `scripts/foxline`

Canonical entrypoints are the scripts in `scripts/`.

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

The Rust Voice Gateway is under active migration in `crates/voice_gateway`. `just demo-rust` is the primary Rust demo path; the legacy TypeScript bridge remains available through `scripts/foxline` while migration work continues. See `ARCHITECTURE.md`, `docs/migration.md`, `docs/benchmarks.md`, and `docs/foxline-loadouts.md`.

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
- [qwen3_tts_rs](https://github.com/badlogic/qwen3_tts_rs): related Rust Qwen3-TTS inference project; not currently wired as Foxline's default TTS worker.
