#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_EXT="$ROOT_DIR/pi-config/extensions/foxline-tools"

if [[ ! -d "$SOURCE_EXT" ]]; then
  echo "error: source extension not found: $SOURCE_EXT" >&2
  exit 1
fi

resolve_relative() {
  local target="$1"
  local from_dir="$2"
  python3 - <<'PY' "$target" "$from_dir"
import os, sys
print(os.path.relpath(sys.argv[1], sys.argv[2]))
PY
}

link_one() {
  local agent_dir="$1"
  local ext_dir="$agent_dir/.pi/extensions"
  local link_path="$ext_dir/foxline-tools"

  mkdir -p "$ext_dir"

  local rel
  rel="$(resolve_relative "$SOURCE_EXT" "$ext_dir")"

  if [[ -L "$link_path" ]]; then
    local current
    current="$(readlink "$link_path")"
    if [[ "$current" == "$rel" ]]; then
      echo "ok: $link_path"
      return
    fi
    rm "$link_path"
  elif [[ -e "$link_path" ]]; then
    rm -rf "$link_path"
  fi

  ln -s "$rel" "$link_path"
  echo "linked: $link_path -> $rel"
}

if [[ $# -gt 0 ]]; then
  for dir in "$@"; do
    link_one "$dir"
  done
  exit 0
fi

shopt -s nullglob
agent_dirs=("$ROOT_DIR"/agents/*)
shopt -u nullglob

if [[ ${#agent_dirs[@]} -eq 0 ]]; then
  echo "no agent directories found under $ROOT_DIR/agents"
  exit 0
fi

for dir in "${agent_dirs[@]}"; do
  [[ -d "$dir" ]] || continue
  link_one "$dir"
done
