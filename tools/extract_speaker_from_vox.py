#!/usr/bin/env python3
"""Programmatically isolate one speaker from a multi-speaker VOX clip.

Metal Gear Solid codec exchanges (e.g. disc-1 `vox-0021.wav`) contain several
characters in one file, and the low-fi codec filter makes off-the-shelf
diarization collapse them into a single speaker. This tool instead classifies
each transcript segment by *nearest enrolled anchor voice*: given a clean
reference for the target speaker and one or more references for the other
speaker(s), it embeds every segment (Resemblyzer / GE2E) and keeps only the
segments closest to the target, then concatenates them into a clean reference.

This is how Snake's reference is derived: his codec voice is not a contiguous
VOX bank, but `Snake ↔ Campbell` in vox-0021 separates cleanly with Snake and
Campbell as the two anchors (~0.9 vs ~0.75 cosine per segment).

Segment boundaries come from a trnscrb transcript (`transcript.json` with
`start`/`end`); produce one first with:

    trnscrb run <vox.wav> --language en   # then pass its transcript.json

Requires (kept out of the repo's core deps; run via uv):

    uv run --python 3.12 --with 'setuptools<81' --with resemblyzer \\
        --with soundfile --with scipy tools/extract_speaker_from_vox.py ...
"""
from __future__ import annotations

import argparse
import json
from math import gcd
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly


def load(path: str, sr: int) -> np.ndarray:
    x, s = sf.read(path, dtype="float32")
    if x.ndim > 1:
        x = x.mean(1)
    if s != sr:
        g = gcd(s, sr)
        x = resample_poly(x, sr // g, s // g)
    return x


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--vox", required=True, help="Multi-speaker source WAV")
    p.add_argument("--transcript", required=True, help="trnscrb transcript.json with segment start/end")
    p.add_argument("--target-ref", required=True, help="Clean reference clip of the speaker to keep")
    p.add_argument("--anchor-ref", action="append", default=[], required=True,
                   help="Clean reference of another speaker (repeatable)")
    p.add_argument("--out", required=True, help="Output WAV of the isolated speaker")
    p.add_argument("--out-sr", type=int, default=24000, help="Output sample rate (default 24000)")
    p.add_argument("--pad-ms", type=int, default=80, help="Silence padding between kept segments")
    p.add_argument("--min-seg-ms", type=int, default=300, help="Ignore segments shorter than this")
    args = p.parse_args()

    from resemblyzer import VoiceEncoder, preprocess_wav

    enc = VoiceEncoder(verbose=False)
    target_a = enc.embed_utterance(preprocess_wav(args.target_ref))
    anchor_as = [enc.embed_utterance(preprocess_wav(a)) for a in args.anchor_ref]

    sr = 16000
    vox = load(args.vox, sr)
    data = json.loads(Path(args.transcript).read_text())
    segs = data if isinstance(data, list) else data.get("segments") or next(
        v for v in data.values() if isinstance(v, list)
    )

    def cos(a: np.ndarray, b: np.ndarray) -> float:
        return float(np.dot(a, b))

    kept: list[tuple[float, float]] = []
    print(f"{'seg':>3} {'start':>6} {'end':>6} {'target':>6} {'other':>6}  keep  text")
    for i, s in enumerate(segs):
        a, b = float(s.get("start")), float(s.get("end"))
        clip = vox[int(a * sr):int(b * sr)]
        if len(clip) < sr * args.min_seg_ms / 1000:
            continue
        e = enc.embed_utterance(clip)
        ts = cos(e, target_a)
        os_ = max(cos(e, x) for x in anchor_as)
        keep = ts > os_
        if keep:
            kept.append((a, b))
        text = str(s.get("text", "")).strip()[:52]
        print(f"{i:>3} {a:>6.1f} {b:>6.1f} {ts:>6.3f} {os_:>6.3f}  {'YES' if keep else '  .':>4}  {text}")

    voxo = load(args.vox, args.out_sr)
    pad = np.zeros(int(args.pad_ms / 1000 * args.out_sr), dtype="float32")
    parts: list[np.ndarray] = []
    for a, b in kept:
        parts.append(voxo[int(a * args.out_sr):int(b * args.out_sr)])
        parts.append(pad)
    out = np.concatenate(parts) if parts else np.zeros(1, dtype="float32")
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    sf.write(args.out, out, args.out_sr, subtype="PCM_16")
    print(f"\nkept {len(kept)} segments -> {args.out} ({len(out) / args.out_sr:.2f}s)")


if __name__ == "__main__":
    main()
