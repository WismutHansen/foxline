#!/usr/bin/env python3
"""Build a crosswalk from MGS PC VOX filenames to canonical PSX VOX bank ids.

PC/GOG uses filenames like VC000101.VOX while this project selected voice
snippets by canonical PSX extraction paths like
assets/generated/mgs/audio_wav_banks/disc_1/vox/vox_bank_0343_0x52551e4.wav.

The decoded PC records are equivalent enough to match by extracted ADPCM byte
count, decoded frame count, sample rate, and channel count.
"""
from __future__ import annotations

import argparse
import csv
import json
from collections import defaultdict
from pathlib import Path


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def key(row: dict[str, str]) -> tuple[int, int, int, int]:
    bytes_ = int(float(row.get("bytes") or row.get("dedupedBytes") or 0))
    frames = int(float(row.get("frames") or row.get("samples") or 0))
    rate = int(float(row.get("sampleRate") or 0))
    channels = int(float(row.get("channels") or 1))
    return bytes_, frames, rate, channels


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pc-manifest", type=Path, default=Path("assets/generated/mgs_pc/VOX_wav/manifest.csv"))
    ap.add_argument("--psx-root", type=Path, default=Path("assets/generated/mgs/audio_wav_banks"))
    ap.add_argument("--psx-index", type=Path, default=Path("assets/mgs-vox-bank-index.json"), help="Non-audio bank metadata index for clean PC-only installs")
    ap.add_argument("--out", type=Path, default=Path("assets/generated/mgs_pc/vox-psx-crosswalk.json"))
    args = ap.parse_args()

    psx_by_key: dict[tuple[int, int, int, int], list[dict[str, object]]] = defaultdict(list)
    manifests = sorted(args.psx_root.glob("disc_*/vox/manifest.csv"))
    if manifests:
        for manifest in manifests:
            disc_name = manifest.parts[-3]
            disc = int(disc_name.split("_")[-1])
            for row in read_csv(manifest):
                k = key(row)
                psx_by_key[k].append({
                    "disc": disc,
                    "bank": int(row["bank"]),
                    "file": row["file"],
                    "bytes": int(float(row["bytes"])),
                    "frames": int(float(row.get("frames") or row.get("samples") or 0)),
                    "sampleRate": int(float(row["sampleRate"])),
                    "channels": int(float(row.get("channels") or 1)),
                    "duration": float(row["duration"]),
                })
    elif args.psx_index.exists():
        index = json.loads(args.psx_index.read_text(encoding="utf-8"))
        for row in index.get("banks", []):
            k = (int(row["bytes"]), int(row["frames"]), int(row["sampleRate"]), int(row.get("channels", 1)))
            psx_by_key[k].append({
                "disc": int(row["disc"]),
                "bank": int(row["bank"]),
                "file": None,
                "bytes": int(row["bytes"]),
                "frames": int(row["frames"]),
                "sampleRate": int(row["sampleRate"]),
                "channels": int(row.get("channels", 1)),
                "duration": float(row["duration"]),
            })
    else:
        raise SystemExit(f"no PSX manifests under {args.psx_root} and no index at {args.psx_index}")

    psx_rows = [item for items in psx_by_key.values() for item in items]

    entries = []
    matched = ambiguous = fuzzy = unmatched = 0
    for row in read_csv(args.pc_manifest):
        k = key(row)
        candidates = psx_by_key.get(k, [])
        status = "unmatched"
        match = None
        if len(candidates) == 1:
            status = "matched"
            match = candidates[0]
            matched += 1
        elif len(candidates) > 1:
            status = "ambiguous"
            match = candidates[0]
            ambiguous += 1
        else:
            # Some PC voice files include/omit a single terminal ADPCM block vs
            # the PSX DAT extraction. Recover those with a very tight fuzzy
            # match: same rate/channels, within one 16-byte ADPCM block and its
            # 28 decoded samples.
            near = [
                p for p in psx_rows
                if p["sampleRate"] == k[2]
                and p["channels"] == k[3]
                and abs(int(p["bytes"]) - k[0]) <= 16
                and abs(int(p["frames"]) - k[1]) <= 28
            ]
            if len(near) == 1:
                status = "fuzzy"
                match = near[0]
                candidates = near
                fuzzy += 1
            elif len(near) > 1:
                status = "ambiguous_fuzzy"
                match = near[0]
                candidates = near
                ambiguous += 1
            else:
                unmatched += 1
        entries.append({
            "pcSource": row.get("source"),
            "pcWav": row.get("file"),
            "pcStem": Path(row.get("source") or row.get("file") or "").stem.lower(),
            "status": status,
            "match": match,
            "candidates": candidates if status == "ambiguous" else None,
            "bytes": k[0],
            "frames": k[1],
            "sampleRate": k[2],
            "channels": k[3],
            "duration": float(row.get("duration") or 0),
        })

    result = {
        "pcManifest": str(args.pc_manifest),
        "psxRoot": str(args.psx_root),
        "psxIndex": str(args.psx_index),
        "summary": {
            "entries": len(entries),
            "matched": matched,
            "fuzzy": fuzzy,
            "ambiguous": ambiguous,
            "unmatched": unmatched,
        },
        "entries": entries,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2), encoding="utf-8")

    csv_out = args.out.with_suffix(".csv")
    with csv_out.open("w", newline="", encoding="utf-8") as f:
        fields = ["pcStem", "pcSource", "pcWav", "status", "disc", "bank", "psxFile", "bytes", "frames", "duration", "sampleRate", "channels"]
        wr = csv.DictWriter(f, fieldnames=fields)
        wr.writeheader()
        for e in entries:
            m = e.get("match") or {}
            wr.writerow({
                "pcStem": e["pcStem"],
                "pcSource": e["pcSource"],
                "pcWav": e["pcWav"],
                "status": e["status"],
                "disc": m.get("disc"),
                "bank": m.get("bank"),
                "psxFile": m.get("file"),
                "bytes": e["bytes"],
                "frames": e["frames"],
                "duration": e["duration"],
                "sampleRate": e["sampleRate"],
                "channels": e["channels"],
            })
    print(json.dumps(result["summary"], indent=2))
    print(f"wrote {args.out}")
    print(f"wrote {csv_out}")


if __name__ == "__main__":
    main()
