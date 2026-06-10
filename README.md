![banner](banner.png)

# foxline

Foxline is a local Metal Gear Solid-style codec runtime: voice input (STT), character-driven dialogue via Pi RPC, and streamed voice output (TTS) in a browser UI.

Default runtime models/components:
- Brain (LLM): whatever `CODEC_PI_MODEL` points to in your local Pi model registry (example in this README uses LM Studio `gemma-4-26b-a4b-it`)
- STT: Parakeet + Silero VAD
- TTS: Qwen3-TTS worker (MLX)

Current platform status:
- Mac-first at the moment due to the Qwen3-TTS MLX worker setup
- Fully local runtime: STT/TTS/LLM can run entirely on your machine (when backed by local providers/models)

> [!IMPORTANT]
> This repo does not ship copyrighted Metal Gear Solid assets. You must provide legally obtained source media (PS1 NTSC/US discs, GOG installer, or an existing PC install directory).

## Quick start

1. Put source media in `./sources/`.
2. Run installer:

```bash
scripts/install-assets.sh -y
```

3. Start full runtime (tmux + bridge + frontend):

```bash
scripts/foxline
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
- Start full app in tmux: `scripts/foxline`

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
```

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
