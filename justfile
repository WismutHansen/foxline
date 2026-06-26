set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default: dev

install:
    bun install

setup:
    scripts/setup-deps.sh

install-assets *args:
    scripts/install-assets.sh {{args}}

services *args:
    scripts/start-services.sh {{args}}

demo-tmux:
    scripts/foxline

demo-rust:
    scripts/foxline-rust

foxline *args:
    scripts/foxline {{args}}

foxline-rust *args:
    scripts/foxline-rust {{args}}

dev:
    bun run dev

build:
    bun run build

typecheck:
    bun run typecheck

gateway *args:
    cargo run -p foxline-voice-gateway -- {{args}}

gateway-check:
    cargo check -p foxline-voice-gateway

gateway-test:
    cargo test -p foxline-voice-gateway

benchmark-legacy-bridge *args:
    bun run benchmarks/run-legacy-bridge-fixture.ts {{args}}

benchmark-rust-gateway *args:
    bun run benchmarks/run-rust-gateway-fixture.ts {{args}}

benchmark-gateway-latency *args:
    bun run benchmarks/run-gateway-latency-benchmark.ts {{args}}

preview:
    bun run preview

bridge character="campbell":
    CODEC_BRAIN_MODE=pi CODEC_TTS_MODE=worker bun run server/bridge.ts --character={{character}} --continue

bridge-new character="campbell":
    CODEC_BRAIN_MODE=pi CODEC_TTS_MODE=worker bun run server/bridge.ts --character={{character}} --new

reference-transcripts-normalize *args:
    uv run --script tools/normalize_reference_transcripts.py {{args}}

mgs-install *args:
    uv run --with pillow tools/install_user_mgs_assets.py {{args}}

mgs-install-pc-installer installer *args:
    uv run --with pillow tools/install_user_mgs_assets.py --pc-installer "{{installer}}" {{args}}

mgs-install-pc-dir dir *args:
    uv run --with pillow tools/install_user_mgs_assets.py --pc-dir "{{dir}}" {{args}}

mgs-install-psx disc1 disc2="" *args:
    if [ -n "{{disc2}}" ]; then uv run --with pillow tools/install_user_mgs_assets.py --psx-disc1 "{{disc1}}" --psx-disc2 "{{disc2}}" {{args}}; else uv run --with pillow tools/install_user_mgs_assets.py --psx-disc1 "{{disc1}}" {{args}}; fi

mgs-sfx-render filesystem="assets/generated/mgs/disc_1/filesystem" out="assets/generated/mgs/disc_1/sfx" *args:
    uv run tools/render_mgs_sfx.py --filesystem "{{filesystem}}" --out "{{out}}" {{args}}

mgs-pc-assets-extract source out="assets/generated/mgs_pc" *args:
    uv run --with pillow tools/extract_mgs_pc_assets.py "{{source}}" --out "{{out}}" {{args}}

mgs-pc-efx-normalize input="assets/generated/mgs_pc/efx/efx" out="assets/generated/mgs_pc/efx_normalized" preset="denoise" *args:
    uv run tools/normalize_mgs_pc_efx.py --input "{{input}}" --out "{{out}}" --preset "{{preset}}" {{args}}

mgs-pc-vox-decode input="assets/generated/mgs_pc/VOX" out="assets/generated/mgs_pc/VOX_wav" *args:
    uv run tools/decode_mgs_pc_vox.py --input "{{input}}" --out "{{out}}" {{args}}

mgs-pc-vox-crosswalk pc_manifest="assets/generated/mgs_pc/VOX_wav/manifest.csv" psx_root="assets/generated/mgs/audio_wav_banks" out="assets/generated/mgs_pc/vox-psx-crosswalk.json":
    uv run tools/build_mgs_pc_vox_crosswalk.py --pc-manifest "{{pc_manifest}}" --psx-root "{{psx_root}}" --psx-index assets/mgs-vox-bank-index.json --out "{{out}}"
