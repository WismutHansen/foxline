#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARAKEET_DIR="${PARAKEET_DIR:-$HOME/.cache/foxline/parakeet.cpp}"
PARAKEET_BUILD_DIR="${PARAKEET_BUILD_DIR:-$PARAKEET_DIR/build-shared}"
PARAKEET_LIB="${PARAKEET_LIB:-$PARAKEET_BUILD_DIR/libparakeet.dylib}"
PIBOT_DIR="${PIBOT_DIR:-$HOME/repos/pibot}"
PIBOT_STT_WORKER="${PIBOT_STT_WORKER:-$PIBOT_DIR/native/parakeet-cpp-stt/build/parakeet-cpp-stt-worker}"
QWEN3_TTS_REPO="${QWEN3_TTS_REPO:-mlx-community/Qwen3-TTS-12Hz-0.6B-Base-4bit}"
PARAKEET_MODEL_DIR="${PARAKEET_MODEL_DIR:-$HOME/models/parakeet-cpp-gguf}"
PARAKEET_MODEL_REPO="${PARAKEET_MODEL_REPO:-mudler/parakeet-cpp-gguf}"
PARAKEET_MODEL_FILE="${PARAKEET_MODEL_FILE:-tdt-0.6b-v3-q8_0.gguf}"
SILERO_MODEL_DIR="${SILERO_MODEL_DIR:-$HOME/models/whisper-vad}"
SILERO_MODEL_REPO="${SILERO_MODEL_REPO:-ggml-org/whisper-vad}"
SILERO_MODEL_FILE="${SILERO_MODEL_FILE:-ggml-silero-v6.2.0.bin}"
SKIP_BREW=0
SKIP_MODELS=0
SKIP_PARAKEET_BUILD=0
SKIP_SILERO_WORKER_BUILD=0

usage() {
  cat <<'EOF'
Usage: scripts/setup-deps.sh [options]

Installs Foxline runtime dependencies, builds parakeet.cpp shared library, and
prefetches Parakeet/Silero/Qwen models.

Options:
  --skip-brew              Do not install Homebrew packages
  --skip-models            Do not prefetch model files
  --skip-parakeet-build    Do not clone/build parakeet.cpp
  --skip-silero-worker     Do not clone/build PiBot parakeet+Silero worker
  -h, --help               Show this help

Environment overrides:
  QWEN3_TTS_REPO, PARAKEET_DIR, PARAKEET_LIB, PARAKEET_MODEL_DIR,
  PARAKEET_MODEL_FILE, SILERO_MODEL_DIR
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-brew) SKIP_BREW=1 ;;
    --skip-models) SKIP_MODELS=1 ;;
    --skip-parakeet-build) SKIP_PARAKEET_BUILD=1 ;;
    --skip-silero-worker) SKIP_SILERO_WORKER_BUILD=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "Missing required command: $1" >&2; exit 1; }; }

if [[ $SKIP_BREW -eq 0 ]]; then
  if command -v brew >/dev/null 2>&1; then
    brew install bun uv p7zip ffmpeg innoextract cmake git pkg-config tmux || true
  else
    echo "Homebrew not found; install bun, uv, p7zip/7zz, ffmpeg, innoextract, cmake, git manually." >&2
  fi
fi

need bun
need uv
need git
need cmake

cd "$ROOT"
bun install

if [[ $SKIP_SILERO_WORKER_BUILD -eq 0 ]]; then
  if [[ ! -x "$PIBOT_STT_WORKER" ]]; then
    mkdir -p "$(dirname "$PIBOT_DIR")"
    if [[ ! -d "$PIBOT_DIR/.git" ]]; then
      git clone https://github.com/badlogic/pibot.git "$PIBOT_DIR"
    else
      git -C "$PIBOT_DIR" pull --ff-only || true
    fi
    if command -v npm >/dev/null 2>&1; then
      (cd "$PIBOT_DIR" && npm install && npm run build:stt-parakeet-cpp)
    else
      cmake -S "$PIBOT_DIR/native/parakeet-cpp-stt" -B "$PIBOT_DIR/native/parakeet-cpp-stt/build" -DCMAKE_BUILD_TYPE=Release
      cmake --build "$PIBOT_DIR/native/parakeet-cpp-stt/build" --parallel
    fi
  fi
  echo "Parakeet+Silero worker: $PIBOT_STT_WORKER"
fi

if [[ $SKIP_PARAKEET_BUILD -eq 0 ]]; then
  if [[ ! -f "$PARAKEET_LIB" ]]; then
    mkdir -p "$(dirname "$PARAKEET_DIR")"
    if [[ ! -d "$PARAKEET_DIR/.git" ]]; then
      git clone --recursive https://github.com/mudler/parakeet.cpp "$PARAKEET_DIR"
    else
      git -C "$PARAKEET_DIR" pull --ff-only || true
      git -C "$PARAKEET_DIR" submodule update --init --recursive
    fi
    cmake -S "$PARAKEET_DIR" -B "$PARAKEET_BUILD_DIR" -DPARAKEET_SHARED=ON -DCMAKE_BUILD_TYPE=Release
    cmake --build "$PARAKEET_BUILD_DIR" --parallel
  fi
  echo "Parakeet shared library: $PARAKEET_LIB"
fi

if [[ $SKIP_MODELS -eq 0 ]]; then
  uvx --with huggingface-hub --with hf_transfer hf download "$QWEN3_TTS_REPO"
  uvx --with huggingface-hub --with hf_transfer hf download "$PARAKEET_MODEL_REPO" "$PARAKEET_MODEL_FILE" --local-dir "$PARAKEET_MODEL_DIR"
  uvx --with huggingface-hub --with hf_transfer hf download "$SILERO_MODEL_REPO" "$SILERO_MODEL_FILE" --local-dir "$SILERO_MODEL_DIR"
fi

cat <<EOF

Setup complete.
Suggested environment:
  export PARAKEET_LIB="$PARAKEET_LIB"
  export PARAKEET_MODEL="$PARAKEET_MODEL_DIR/$PARAKEET_MODEL_FILE"
  export SILERO_MODEL="$SILERO_MODEL_DIR/$SILERO_MODEL_FILE"
  export QWEN3_TTS_MODEL="$QWEN3_TTS_REPO"
EOF
