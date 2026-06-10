#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Normalize obvious ASR transcript terms in reference audio sidecars.

Usage:
  uv run --script tools/normalize_reference_transcripts.py
  uv run --script tools/normalize_reference_transcripts.py --dry-run
  uv run --script tools/normalize_reference_transcripts.py --root agents --map assets/reference_transcript_replacements.json
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def load_replacements(path: Path) -> dict[str, str]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise SystemExit(f"Replacement map must be an object: {path}")
    out: dict[str, str] = {}
    for k, v in raw.items():
        kk = str(k).strip()
        vv = str(v).strip()
        if kk and vv:
            out[kk] = vv
    return out


def apply_replacements(text: str, replacements: dict[str, str]) -> tuple[str, int]:
    total = 0
    out = text
    for src, dst in replacements.items():
        pattern = re.compile(rf"\b{re.escape(src)}\b", flags=re.IGNORECASE)
        out, n = pattern.subn(dst, out)
        total += n
    return out, total


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=Path, default=Path("agents"), help="Agent root directory")
    ap.add_argument("--map", type=Path, default=Path("assets/reference_transcript_replacements.json"), help="JSON map of replacement pairs")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    replacements = load_replacements(args.map)
    txt_files = sorted(args.root.glob("*/assets/reference_audio/*.wav.txt"))
    changed = 0
    hits = 0

    for p in txt_files:
        original = p.read_text(encoding="utf-8")
        normalized, n = apply_replacements(original, replacements)
        if n > 0:
            hits += n
            changed += 1
            if not args.dry_run:
                p.write_text(normalized, encoding="utf-8")
            print(f"updated {p} ({n} replacements)")

    mode = "dry-run" if args.dry_run else "write"
    print(f"done ({mode}): files_scanned={len(txt_files)} files_changed={changed} replacements={hits}")


if __name__ == "__main__":
    main()
