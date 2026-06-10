#!/usr/bin/env python3
"""Normalize and lightly denoise MGS PC efx WAVs.

The PC port stores many effects as noisy unsigned 8-bit PCM. This script makes
agent-friendly 16-bit/44.1 kHz WAV derivatives while preserving originals.
"""
from __future__ import annotations

import argparse
import csv
import subprocess
from pathlib import Path


def run(cmd: list[str]) -> None:
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", type=Path, default=Path("assets/generated/mgs_pc/efx/efx"))
    ap.add_argument("--out", type=Path, default=Path("assets/generated/mgs_pc/efx_normalized"))
    ap.add_argument("--preset", choices=["normalize", "denoise"], default="denoise")
    ap.add_argument("--limit", type=int)
    args = ap.parse_args()

    if not args.input.exists():
        raise SystemExit(f"input not found: {args.input}")

    files = sorted(args.input.rglob("*.wav"))
    if args.limit:
        files = files[: args.limit]

    # Keep this conservative: convert away from 8-bit PCM, resample, remove DC,
    # gently denoise, and normalize peaks/loudness. Over-aggressive restoration
    # destroys short UI bleeps.
    filters = {
        "normalize": "aformat=sample_fmts=s16:channel_layouts=mono,aresample=44100,dynaudnorm=f=75:g=5:p=0.95",
        "denoise": "aformat=sample_fmts=s16:channel_layouts=mono,aresample=44100,highpass=f=35,afftdn=nf=-30:tn=1,dynaudnorm=f=75:g=5:p=0.95",
    }[args.preset]

    rows = []
    for src in files:
        rel = src.relative_to(args.input)
        dst = args.out / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        run(["ffmpeg", "-y", "-i", str(src), "-af", filters, "-ac", "1", "-ar", "44100", "-sample_fmt", "s16", str(dst)])
        rows.append({"source": str(src), "file": str(dst), "preset": args.preset})

    args.out.mkdir(parents=True, exist_ok=True)
    with (args.out / "manifest.csv").open("w", newline="", encoding="utf-8") as f:
        wr = csv.DictWriter(f, fieldnames=["source", "file", "preset"])
        wr.writeheader(); wr.writerows(rows)
    print(f"normalized {len(rows)} files to {args.out}")


if __name__ == "__main__":
    main()
