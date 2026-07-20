#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-${XDG_DATA_HOME:-$HOME/.local/share}/foxline}"
MANIFEST="$ROOT/agents/manifest.json"
SCHEMA_URL="https://raw.githubusercontent.com/byteowlz/foxline/main/schemas/persona.schema.json"

command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
mkdir -p "$DEST/personas"

for source in "$ROOT"/agents/*; do
  [[ -d "$source" && -f "$source/PROMPT.md" ]] || continue
  id="${source##*/}"
  target="$DEST/personas/$id"
  display_name="$(jq -r --arg id "$id" '.characters[]? | select(.id == $id) | .displayName' "$MANIFEST" | head -1)"
  [[ -n "$display_name" && "$display_name" != "null" ]] || display_name="$id"

  mkdir -p "$target/voice" "$target/canned/tool-started" "$target/canned/tool-slow" "$target/canned/tool-completed"
  cp "$source/PROMPT.md" "$target/PROMPT.md"

  reference="$(find "$source/assets/reference_audio" -maxdepth 1 -type f -iname '*.wav' ! -name '*.bak' 2>/dev/null | sort | head -1 || true)"
  has_voice=false
  if [[ -n "$reference" ]]; then
    transcript=""
    for candidate in "$reference.txt" "${reference%.wav}.txt" "$(dirname "$reference")/reference.txt"; do
      if [[ -f "$candidate" ]]; then transcript="$candidate"; break; fi
    done
    if [[ -z "$transcript" ]]; then
      echo "Missing transcript for $reference" >&2
      exit 1
    fi
    cp "$reference" "$target/voice/reference.wav"
    cp "$transcript" "$target/voice/reference.txt"
    metadata="${reference%.wav}.json"
    [[ -f "$metadata" ]] && cp "$metadata" "$target/voice/reference.json"
    rm -f "$target/voice/.gitkeep"
    has_voice=true
  fi

  for wav in "$source"/assets/filler/*.wav; do
    [[ -f "$wav" ]] || continue
    name="${wav##*/}"
    case "$name" in
      tool_start_*) group="tool-started" ;;
      tool_slow_*) group="tool-slow" ;;
      tool_done_*) group="tool-completed" ;;
      *) continue ;;
    esac
    cp "$wav" "$target/canned/$group/$name"
    rm -f "$target/canned/$group/.gitkeep"
  done
  [[ -f "$source/assets/filler/manifest.json" ]] && cp "$source/assets/filler/manifest.json" "$target/canned/manifest.json"

  has_started=false; compgen -G "$target/canned/tool-started/*.wav" >/dev/null && has_started=true
  has_slow=false; compgen -G "$target/canned/tool-slow/*.wav" >/dev/null && has_slow=true
  has_completed=false; compgen -G "$target/canned/tool-completed/*.wav" >/dev/null && has_completed=true

  {
    printf '"$schema" = "%s"\n\n' "$SCHEMA_URL"
    printf 'id = "%s"\n' "$id"
    printf 'display_name = "%s"\n' "${display_name//\"/\\\"}"
    if [[ "$has_voice" == true ]]; then
      printf '\n[voice]\nreference_audio = "voice/reference.wav"\nreference_text = "voice/reference.txt"\n'
    fi
    if [[ "$has_started" == true ]]; then
      printf '\n[canned.tool_started]\ndirectory = "canned/tool-started"\n'
    fi
    if [[ "$has_slow" == true ]]; then
      printf '\n[canned.tool_slow]\ndirectory = "canned/tool-slow"\n'
    fi
    if [[ "$has_completed" == true ]]; then
      printf '\n[canned.tool_completed]\ndirectory = "canned/tool-completed"\n'
    fi
  } > "$target/persona.toml"

  printf 'Synced %-12s prompt=%s voice=%s canned=%s/%s/%s\n' \
    "$id" yes "$has_voice" "$has_started" "$has_slow" "$has_completed"
done

printf 'Foxline Persona data synchronized to %s\n' "$DEST"
