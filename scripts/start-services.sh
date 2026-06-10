#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SESSION="${FOXLINE_TMUX_SESSION:-foxline-services}"
STT_PORT="${STT_PORT:-8796}"
TRANSCRIPT_PORT="${TRANSCRIPT_PORT:-8780}"
PARAKEET_LIB="${PARAKEET_LIB:-$HOME/.cache/foxline/parakeet.cpp/build-shared/libparakeet.dylib}"
PARAKEET_MODEL="${PARAKEET_MODEL:-$HOME/models/parakeet-cpp-gguf/tdt-0.6b-v3-q8_0.gguf}"
SILERO_MODEL="${SILERO_MODEL:-$HOME/models/whisper-vad/ggml-silero-v6.2.0.bin}"
PIBOT_STT_WORKER="${PIBOT_STT_WORKER:-$HOME/repos/pibot/native/parakeet-cpp-stt/build/parakeet-cpp-stt-worker}"
PID_DIR="${FOXLINE_PID_DIR:-$HOME/.local/state/foxline/pids}"
START_TRANSCRIPT_SERVER=1
FOREGROUND=0
NO_TMUX=0

usage() {
  cat <<'EOF'
Usage: scripts/start-services.sh [options]

Starts Foxline STT services. App STT is parakeet.cpp + Silero by default.
Qwen3-TTS is owned by server/bridge.ts as a low-latency worker, not a separate HTTP service.

Options:
  --no-transcript-server   Do not start the Parakeet HTTP transcript server
  --foreground             Run parakeet+silero app STT in the current shell
  --no-tmux                Run background services without tmux
  -h, --help               Show this help

Ports:
  STT_PORT=8796            WebSocket app STT port
  TRANSCRIPT_PORT=8780     HTTP Parakeet transcript port for installer
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-transcript-server) START_TRANSCRIPT_SERVER=0 ;;
    --foreground) FOREGROUND=1 ;;
    --no-tmux) NO_TMUX=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

stt_cmd() {
  cat <<EOF
cd "$ROOT" && PORT="$STT_PORT" uv run --script services/parakeet_silero_ws_server.py --host 127.0.0.1 --port "$STT_PORT" --worker "$PIBOT_STT_WORKER" --model "$PARAKEET_MODEL" --vad-model "$SILERO_MODEL"
EOF
}

transcript_cmd() {
  cat <<EOF
cd "$ROOT" && PORT="$TRANSCRIPT_PORT" PARAKEET_LIB="$PARAKEET_LIB" PARAKEET_MODEL="$PARAKEET_MODEL" uv run --script services/parakeet_transcript_server.py --host 127.0.0.1 --port "$TRANSCRIPT_PORT"
EOF
}

start_bg() {
  mkdir -p "$PID_DIR"

  local stt_pid_file="$PID_DIR/stt.pid"
  if [[ -f "$stt_pid_file" ]] && kill -0 "$(cat "$stt_pid_file")" 2>/dev/null; then
    echo "STT service already running (pid $(cat "$stt_pid_file"))."
  else
    nohup bash -lc "$(stt_cmd)" >"$PID_DIR/stt.log" 2>&1 &
    echo $! >"$stt_pid_file"
    echo "Started STT service (pid $!)."
  fi

  if [[ $START_TRANSCRIPT_SERVER -eq 1 ]]; then
    local transcript_pid_file="$PID_DIR/transcript.pid"
    if [[ -f "$transcript_pid_file" ]] && kill -0 "$(cat "$transcript_pid_file")" 2>/dev/null; then
      echo "Transcript service already running (pid $(cat "$transcript_pid_file"))."
    else
      nohup bash -lc "$(transcript_cmd)" >"$PID_DIR/transcript.log" 2>&1 &
      echo $! >"$transcript_pid_file"
      echo "Started transcript service (pid $!)."
    fi
  fi

  echo "Logs: $PID_DIR/stt.log $PID_DIR/transcript.log"
  echo "PIDs: $PID_DIR/*.pid"
}

if [[ $FOREGROUND -eq 1 ]]; then
  eval "$(stt_cmd)"
  exit 0
fi

if [[ $NO_TMUX -eq 1 ]]; then
  start_bg
elif command -v tmux >/dev/null 2>&1; then
  tmux has-session -t "$SESSION" 2>/dev/null || tmux new-session -d -s "$SESSION" -n stt "$(stt_cmd)"
  if [[ $START_TRANSCRIPT_SERVER -eq 1 ]]; then
    tmux new-window -t "$SESSION" -n transcript "$(transcript_cmd)" 2>/dev/null || true
  fi
  echo "Started services in tmux session: $SESSION"
  echo "Attach with: tmux attach -t $SESSION"
else
  echo "tmux not found; starting services without tmux."
  start_bg
fi

echo "Parakeet+Silero STT: ws://127.0.0.1:$STT_PORT/ws"
echo "Transcript server: http://127.0.0.1:$TRANSCRIPT_PORT/health"
