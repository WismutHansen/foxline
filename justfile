set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default: dev

install:
    bun install

install-cli:
    cargo install --path crates/foxline_cli

service *args:
    cargo run -p foxline-cli -- service {{args}}

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

rust-demo: demo-rust

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

protocol-generate:
    bun run protocol:generate

protocol-typecheck:
    bun run protocol:typecheck

voice-client-typecheck:
    bun run voice-client:typecheck

overlayz-typecheck:
    bun run overlayz:typecheck

overlayz-build:
    bun run overlayz:build

overlayz-tauri-check:
    bun run overlayz:tauri-check

# Run a frontend skin together with the Voice Gateway.
run frontend: install
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "{{frontend}}" != "overlayz" ]]; then
        echo "Unknown frontend: {{frontend}}" >&2
        echo "Usage: just run overlayz" >&2
        exit 2
    fi

    scripts/start-services.sh --no-transcript-server --foreground &
    stt_pid=$!
    cargo run -p foxline-voice-gateway -- --bind 127.0.0.1:8780 &
    gateway_pid=$!
    cleanup() {
        kill "$gateway_pid" "$stt_pid" 2>/dev/null || true
        wait "$gateway_pid" 2>/dev/null || true
        wait "$stt_pid" 2>/dev/null || true
    }
    trap cleanup EXIT INT TERM

    cd apps/overlayz
    bun tauri dev

gateway *args:
    cargo run -p foxline-voice-gateway -- {{args}}

gateway-check:
    cargo check -p foxline-voice-gateway

gateway-test:
    cargo test -p foxline-voice-gateway

# Run the gateway with segment tracing on, e.g. `just gateway-debug-tts`
gateway-debug-tts *args:
    FOXLINE_GATEWAY_DEBUG_TRACES=true cargo run -p foxline-voice-gateway -- {{args}}

# Tail exact strings sent to the TTS worker from the newest session trace
tts-trace:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="${XDG_STATE_HOME:-$HOME/.local/state}/foxline/traces/rust-gateway"
    latest=$(ls -t "$dir"/*.jsonl | head -1)
    echo "tailing $latest" >&2
    tail -f -n +1 "$latest" | jq -r --unbuffered 'select(.event == "tts_segment_queued") | "[\(.data.segment_index)] \(.data.reason)\n  raw:  \(.data.raw_text)\n  sent: \(.data.normalized_text)"'

benchmark-rust-gateway *args:
    bun run benchmarks/run-rust-gateway-fixture.ts {{args}}

benchmark-gateway-latency *args:
    bun run benchmarks/run-gateway-latency-benchmark.ts {{args}}

preview:
    bun run preview

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
