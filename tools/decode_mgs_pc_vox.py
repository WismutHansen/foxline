#!/usr/bin/env python3
"""Decode MGS PC .vox voice files to WAV.

PC/GOG .vox files use the same record structure as PS1 VOX.DAT/DEMO.DAT:
0x10 section init, 0x02 audio header, 0x01 SPU ADPCM payload chunks, and 0xF0
bank end/alignment. Decode them with the canonical PSX bank parser per file.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.extract_mgs_audio_banks import parse


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", type=Path, required=True, help="VOX directory or single .vox file")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--force-rate", type=int, help="Override header sample rate")
    ap.add_argument("--force-channels", type=int, choices=[1, 2], help="Override header channel count")
    ap.add_argument("--dedupe-repeats", type=int, help="Collapse consecutive duplicate ADPCM blocks")
    ap.add_argument("--limit", type=int)
    args = ap.parse_args()

    files = [args.input] if args.input.is_file() else sorted(args.input.rglob("*.vox"))
    rows = []
    decoded = 0
    for src in files:
        if args.limit and decoded >= args.limit:
            break
        rel = Path(src.name if args.input.is_file() else src.relative_to(args.input)).with_suffix("")
        with tempfile.TemporaryDirectory() as td:
            tmp_out = Path(td)
            manifest = parse(
                src,
                tmp_out,
                min_rms=0,
                dedupe_repeats=args.dedupe_repeats,
                force_channels=args.force_channels,
                force_rate=args.force_rate,
            )
            for item in manifest:
                wav_src = Path(item["file"])
                suffix = "" if len(manifest) == 1 else f"_bank_{item['bank']:04d}"
                dst = args.out / rel.parent / f"{rel.name}{suffix}.wav"
                dst.parent.mkdir(parents=True, exist_ok=True)
                wav_src.replace(dst)
                out_item = dict(item)
                out_item["source"] = str(src)
                out_item["file"] = str(dst)
                rows.append(out_item)
        decoded += 1

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "manifest.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
    fieldnames = [
        "source", "file", "bank", "bytes", "dedupedBytes",
        "duplicateBlocksRemoved", "samples", "frames", "duration",
        "sampleRate", "headerSampleRate", "channels", "headerChannels", "rms",
    ]
    with (args.out / "manifest.csv").open("w", newline="", encoding="utf-8") as f:
        wr = csv.DictWriter(f, fieldnames=fieldnames, extrasaction="ignore")
        wr.writeheader(); wr.writerows(rows)
    print(f"decoded {decoded}/{len(files)} VOX files to {args.out}; wavs={len(rows)}")


if __name__ == "__main__":
    main()
