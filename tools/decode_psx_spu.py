#!/usr/bin/env python3
"""Decode raw PlayStation SPU-ADPCM chunks to WAV.

Input files are the `.spu` chunks produced by tools/extract_mgs_assets.py from
MGS VOX.DAT/DEMO.DAT. The decoder handles standard 16-byte PSX ADPCM blocks
(28 mono samples per block).

Example:
  uv run tools/decode_psx_spu.py \
    --input assets/generated/mgs/disc_1/audio_raw/vox \
    --out assets/generated/mgs/audio_wav/vox \
    --rate 22050
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import struct
import wave
from pathlib import Path

FILTERS = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)]


def clamp16(v: int) -> int:
    return max(-32768, min(32767, v))


def nibble_to_signed(n: int) -> int:
    return n - 16 if n >= 8 else n


def decode_block(block: bytes, hist: tuple[int, int]) -> tuple[list[int], tuple[int, int]]:
    if len(block) != 16:
        return [], hist
    shift_filter = block[0]
    shift = shift_filter & 0x0F
    filt = (shift_filter >> 4) & 0x0F
    if filt >= len(FILTERS):
        filt = 0
    f0, f1 = FILTERS[filt]
    s1, s2 = hist
    samples = []
    for b in block[2:16]:
        for nib in (b & 0x0F, b >> 4):
            sample = nibble_to_signed(nib) << 12
            sample >>= shift
            sample += ((s1 * f0) + (s2 * f1) + 32) >> 6
            sample = clamp16(sample)
            samples.append(sample)
            s2, s1 = s1, sample
    return samples, (s1, s2)


def decode_spu(data: bytes) -> list[int]:
    hist = (0, 0)
    out: list[int] = []
    for i in range(0, len(data) - len(data) % 16, 16):
        block = data[i : i + 16]
        samples, hist = decode_block(block, hist)
        out.extend(samples)
    return out


def rms(samples: list[int]) -> float:
    if not samples:
        return 0.0
    return math.sqrt(sum(s * s for s in samples) / len(samples))


def write_wav(path: Path, samples: list[int], rate: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(rate)
        wf.writeframes(b"".join(struct.pack("<h", s) for s in samples))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", type=Path, required=True, help="A .spu file or directory containing .spu files")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--rate", type=int, default=22050)
    ap.add_argument("--min-rms", type=float, default=25.0, help="Skip near-silent chunks below this RMS")
    ap.add_argument("--limit", type=int, help="Decode at most N files")
    args = ap.parse_args()

    files = [args.input] if args.input.is_file() else sorted(args.input.rglob("*.spu"))
    manifest = []
    decoded = 0
    for src in files:
        if args.limit and decoded >= args.limit:
            break
        samples = decode_spu(src.read_bytes())
        level = rms(samples)
        if level < args.min_rms:
            continue
        rel = src.name if args.input.is_file() else str(src.relative_to(args.input))
        dest = args.out / Path(rel).with_suffix(".wav")
        write_wav(dest, samples, args.rate)
        decoded += 1
        manifest.append({
            "source": str(src),
            "file": str(dest),
            "samples": len(samples),
            "duration": len(samples) / args.rate,
            "sampleRate": args.rate,
            "rms": level,
        })
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    with (args.out / "manifest.csv").open("w", newline="", encoding="utf-8") as f:
        wr = csv.DictWriter(f, fieldnames=["source", "file", "samples", "duration", "sampleRate", "rms"])
        wr.writeheader(); wr.writerows(manifest)
    print(f"Decoded {decoded}/{len(files)} files to {args.out}")


if __name__ == "__main__":
    main()
