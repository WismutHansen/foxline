#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="${FOXLINE_SOURCE_DIR:-$ROOT/sources}"
PC_INSTALLER=""
PC_DIR=""
PSX_DISC1=""
PSX_DISC2=""
RUN_SETUP=1
START_SERVICES=1
TRANSCRIBE=1
GENERATE_FILLERS=1
FORCE=0
YES=0
PARAKEET_URL="${PARAKEET_URL:-http://127.0.0.1:8780}"

usage() {
  cat <<'EOF'
Usage: scripts/install-assets.sh [options]

All-in-one Foxline first-setup installer. By default it:
  1. installs local dependencies/models,
  2. starts Parakeet+Silero STT and Parakeet transcript HTTP,
  3. scans ./sources for a GOG installer, PC install dir, or PSX disc images,
  4. extracts user-owned MGS assets,
  5. ensures reference transcripts and filler assets.

The script is idempotent and checks existing outputs before doing heavy work.

Options:
  --source-dir DIR          Directory to scan for source media (default: ./sources)
  --pc-installer FILE       Use this GOG/Inno installer .exe
  --pc-dir DIR              Use this existing PC install/media directory
  --psx-disc1 FILE          Use this PSX disc 1 image (.cue/.iso/.bin/.img/.chd)
  --psx-disc2 FILE          Use this PSX disc 2 image
  --skip-setup              Do not run scripts/setup-deps.sh
  --no-services             Do not start local STT/transcript services
  --no-transcribe           Do not generate reference .wav.txt transcripts
  --no-fillers              Do not generate filler snippets/manifests
  --force                   Force reinstall/regeneration even if outputs already exist
  -y, --yes                 Do not prompt before selected source install
  -h, --help                Show this help

Detection priority when flags are omitted:
  1. PSX images in source dir (preferred: enables high-fidelity SFX/music render)
  2. GOG/Inno .exe in source dir
  3. PC install directory containing face.dat, efx.mgz, or VOX/
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-dir) SOURCE_DIR="${2:-}"; shift ;;
    --pc-installer) PC_INSTALLER="${2:-}"; shift ;;
    --pc-dir) PC_DIR="${2:-}"; shift ;;
    --psx-disc1) PSX_DISC1="${2:-}"; shift ;;
    --psx-disc2) PSX_DISC2="${2:-}"; shift ;;
    --skip-setup) RUN_SETUP=0 ;;
    --no-services) START_SERVICES=0 ;;
    --no-transcribe) TRANSCRIBE=0 ;;
    --no-fillers) GENERATE_FILLERS=0 ;;
    --force) FORCE=1 ;;
    -y|--yes) YES=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

abs() { python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).expanduser().resolve())' "$1"; }
health() { curl -fsS "$1" >/dev/null 2>&1; }
wait_health() {
  local name="$1" url="$2" tries="${3:-90}"
  for _ in $(seq 1 "$tries"); do
    if health "$url"; then echo "$name is healthy: $url"; return 0; fi
    sleep 2
  done
  echo "$name did not become healthy: $url" >&2
  return 1
}

find_pc_installer() {
  find "$SOURCE_DIR" -maxdepth 2 -type f \( -iname '*.exe' -o -iname 'setup_metal_gear_solid*.exe' \) | sort | head -1
}

find_pc_dir() {
  find "$SOURCE_DIR" -maxdepth 3 -type d | while IFS= read -r d; do
    if [[ -f "$d/face.dat" || -f "$d/FACE.DAT" || -f "$d/efx.mgz" || -f "$d/EFX.MGZ" || -d "$d/VOX" ]]; then
      echo "$d"
      break
    fi
  done
}

find_psx_discs() {
  find "$SOURCE_DIR" -maxdepth 2 -type f \( -iname '*.cue' -o -iname '*.chd' -o -iname '*.iso' -o -iname '*.bin' -o -iname '*.img' \) | sort
}

cd "$ROOT"
mkdir -p "$SOURCE_DIR"

if [[ -z "$PC_INSTALLER" && -z "$PC_DIR" && -z "$PSX_DISC1" ]]; then
  # PSX first: only PSX media yields high-fidelity SFX/music rendering.
  mapfile -t discs < <(find_psx_discs)
  PSX_DISC1="${discs[0]:-}"
  PSX_DISC2="${discs[1]:-}"
  if [[ -z "$PSX_DISC1" ]]; then
    PC_INSTALLER="$(find_pc_installer || true)"
    if [[ -z "$PC_INSTALLER" ]]; then PC_DIR="$(find_pc_dir || true)"; fi
  fi
fi

if [[ -n "$PC_INSTALLER" ]]; then PC_INSTALLER="$(abs "$PC_INSTALLER")"; fi
if [[ -n "$PC_DIR" ]]; then PC_DIR="$(abs "$PC_DIR")"; fi
if [[ -n "$PSX_DISC1" ]]; then PSX_DISC1="$(abs "$PSX_DISC1")"; fi
if [[ -n "$PSX_DISC2" ]]; then PSX_DISC2="$(abs "$PSX_DISC2")"; fi

if [[ -z "$PC_INSTALLER" && -z "$PC_DIR" && -z "$PSX_DISC1" ]]; then
  cat >&2 <<EOF
No source media found.
Place one of these in: $SOURCE_DIR
  - PSX disc images (.cue/.chd/.iso/.bin/.img) — preferred, enables SFX/music rendering
  - GOG/Inno MGS installer .exe
  - PC install directory containing face.dat, efx.mgz, VOX/, MDX/
Or pass --pc-installer, --pc-dir, --psx-disc1/--psx-disc2.
EOF
  exit 2
fi

if [[ -n "$PC_INSTALLER" ]]; then
  echo "Selected PC installer: $PC_INSTALLER"
elif [[ -n "$PC_DIR" ]]; then
  echo "Selected PC directory: $PC_DIR"
else
  echo "Selected PSX disc 1: $PSX_DISC1"
  [[ -n "$PSX_DISC2" ]] && echo "Selected PSX disc 2: $PSX_DISC2"
fi

if [[ $YES -eq 0 ]]; then
  read -r -p "Continue with extraction/generation? [y/N] " reply
  case "$reply" in [Yy]|[Yy][Ee][Ss]) ;; *) echo "Cancelled"; exit 0 ;; esac
fi

if [[ $RUN_SETUP -eq 1 ]]; then
  scripts/setup-deps.sh
fi

# Determine idempotent state from current workspace outputs.
state_file="$(mktemp)"
python3 - <<'PY' >"$state_file"
import json
from pathlib import Path
root = Path('.').resolve()
agents_dir = root / 'agents'
chars = []
manifest_path = agents_dir / 'manifest.json'
if manifest_path.exists():
    data = json.loads(manifest_path.read_text(encoding='utf-8'))
    chars = [c['id'] for c in data.get('characters', []) if c.get('enabled', True)]

core_checks = [
    root / 'assets/generated/mgs_install_manifest.json',
    root / 'assets/generated/mgs_pc/vox-psx-crosswalk.json',
    root / 'assets/generated/mgs_pc/face/manifest.json',
    root / 'assets/generated/mgs_pc/VOX_wav/manifest.csv',
]
core_ready = all(p.exists() for p in core_checks)

ref_wavs = sorted((root / 'agents').glob('*/assets/reference_audio/*_reference.wav'))
ref_txts = sorted((root / 'agents').glob('*/assets/reference_audio/*_reference.wav.txt'))
if len(ref_txts) < 7:
    ref_txts = sorted((root / 'agents').glob('*/assets/reference_audio/*.txt'))
avatars = sorted((root / 'agents').glob('*/assets/avatar/**/*.png'))
refs_ready = len(ref_wavs) >= 7
transcripts_ready = len(ref_txts) >= 7
avatars_ready = len(avatars) > 0

fillers_ready = True
if chars:
    for agent in chars:
        m = root / f'agents/{agent}/assets/filler/manifest.json'
        if not m.exists():
            fillers_ready = False
            break
        try:
            rows = json.loads(m.read_text(encoding='utf-8'))
        except Exception:
            fillers_ready = False
            break
        if not rows:
            fillers_ready = False
            break
        existing = 0
        for row in rows:
            p = root / row.get('path', '')
            if p.exists() and p.stat().st_size > 1000:
                existing += 1
        if existing == 0:
            fillers_ready = False
            break

print(f"CORE_READY={int(core_ready and refs_ready and avatars_ready)}")
print(f"TRANSCRIPTS_READY={int(transcripts_ready)}")
print(f"FILLERS_READY={int(fillers_ready)}")
PY
# shellcheck source=/dev/null
source "$state_file"
rm -f "$state_file"

if [[ $FORCE -eq 0 ]]; then
  if [[ ${TRANSCRIPTS_READY:-0} -eq 1 && $TRANSCRIBE -eq 1 ]]; then
    echo "Transcripts already present; skipping transcript regeneration."
    TRANSCRIBE=0
  fi
  if [[ ${FILLERS_READY:-0} -eq 1 && $GENERATE_FILLERS -eq 1 ]]; then
    echo "Fillers already present; skipping filler regeneration."
    GENERATE_FILLERS=0
  fi
fi

NEED_INSTALL=1
if [[ $FORCE -eq 0 && ${CORE_READY:-0} -eq 1 ]]; then
  NEED_INSTALL=0
  echo "Core install outputs already present; skipping extraction/install step."
fi

if [[ $START_SERVICES -eq 1 && $TRANSCRIBE -eq 1 ]]; then
  scripts/start-services.sh
  wait_health "Parakeet transcript server" "$PARAKEET_URL/health" 120
fi

if [[ $NEED_INSTALL -eq 1 ]]; then
  cmd=(uv run --with pillow tools/install_user_mgs_assets.py)
  if [[ -n "$PC_INSTALLER" ]]; then
    cmd+=(--pc-installer "$PC_INSTALLER")
  elif [[ -n "$PC_DIR" ]]; then
    cmd+=(--pc-dir "$PC_DIR")
  else
    cmd+=(--psx-disc1 "$PSX_DISC1")
    [[ -n "$PSX_DISC2" ]] && cmd+=(--psx-disc2 "$PSX_DISC2")
  fi
  [[ $TRANSCRIBE -eq 1 ]] && cmd+=(--transcribe --parakeet-url "$PARAKEET_URL")
  [[ $GENERATE_FILLERS -eq 1 ]] && cmd+=(--generate-fillers)

  printf 'Running:'
  printf ' %q' "${cmd[@]}"
  printf '\n'
  "${cmd[@]}"
else
  if [[ $TRANSCRIBE -eq 1 ]]; then
    uv run --with pillow python - "$PARAKEET_URL" <<'PY'
import sys
from tools.install_user_mgs_assets import transcribe_reference_audio
transcribe_reference_audio(sys.argv[1])
PY
    uv run --script tools/normalize_reference_transcripts.py
  fi
  if [[ $GENERATE_FILLERS -eq 1 ]]; then
    uv run --script tools/generate_codec_fillers.py --agents all
  fi
fi

echo "Foxline asset installation complete."
