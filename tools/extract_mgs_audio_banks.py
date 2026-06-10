#!/usr/bin/env python3
"""Extract and decode MGS VOX.DAT/DEMO.DAT audio banks correctly.

MGS DAT audio is chunked records, not standalone 0x01 blobs. This parser follows
known unDemoVox structure:
- code 0x10: section init
- code 0x02: audio header (sample-rate/channel hints)
- code 0x01: SPU ADPCM payload chunk
- code 0xF0: section/bank end, then align to 0x800

It concatenates code 0x01 payloads per audio section and decodes those as one
continuous PSX SPU-ADPCM stream.
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


def clamp(v):
    return max(-32768, min(32767, v))


def sn(n):
    return n - 16 if n >= 8 else n


def collapse_duplicate_blocks(data: bytes, max_repeats: int = 1):
    """Collapse consecutive identical 16-byte ADPCM blocks.

    Some MGS banks contain long runs of duplicated ADPCM frames, which creates
    severe stuttering when decoded literally. Keep up to max_repeats copies.
    """
    out = bytearray()
    prev = None
    run = 0
    removed = 0
    for i in range(0, len(data) - len(data) % 16, 16):
        b = data[i : i + 16]
        if b == prev:
            run += 1
        else:
            prev = b
            run = 1
        if run <= max_repeats:
            out.extend(b)
        else:
            removed += 1
    return bytes(out), removed


def decode_block(b: bytes, hist):
    s1, s2 = hist
    sf = b[0]
    shift = sf & 0xF
    filt = (sf >> 4) & 0xF
    if filt >= len(FILTERS):
        filt = 0
    f0, f1 = FILTERS[filt]
    out = []
    for by in b[2:]:
        for nib in (by & 0xF, by >> 4):
            sample = sn(nib) << 12
            sample >>= shift
            sample += ((s1 * f0) + (s2 * f1) + 32) >> 6
            sample = clamp(sample)
            out.append(sample)
            s2, s1 = s1, sample
    return out, (s1, s2)


def decode_spu(data: bytes, channels: int = 1):
    blocks = [data[i : i + 16] for i in range(0, len(data) - len(data) % 16, 16)]
    if channels != 2:
        hist = (0, 0)
        out = []
        for b in blocks:
            samples, hist = decode_block(b, hist)
            out.extend(samples)
        return out
    # Stereo streams are interleaved as alternating 16-byte ADPCM blocks.
    hist = [(0, 0), (0, 0)]
    chan_samples = [[], []]
    for bi, b in enumerate(blocks):
        ch = bi % 2
        samples, hist[ch] = decode_block(b, hist[ch])
        chan_samples[ch].extend(samples)
    n = min(len(chan_samples[0]), len(chan_samples[1]))
    out = []
    for i in range(n):
        out.extend((chan_samples[0][i], chan_samples[1][i]))
    return out


def rms(samples):
    return math.sqrt(sum(s * s for s in samples) / len(samples)) if samples else 0


def wav(path, samples, rate, channels=1):
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"".join(struct.pack("<h", s) for s in samples))


def params_from_header(chunk: bytes):
    # Observed sample code at offset+10 and channels at offset+12 in old scripts.
    rate = 22050
    channels = 1
    if len(chunk) > 12:
        code = chunk[10]
        if code == 8:
            rate = 22050
        elif code == 12:
            rate = 32000
        elif code == 16:
            rate = 44100
        if chunk[12] in (1, 2):
            channels = chunk[12]
    return rate, channels


def parse(
    dat: Path,
    out: Path,
    min_rms: float = 25.0,
    limit: int | None = None,
    dedupe_repeats: int | None = None,
    force_channels: int | None = None,
    force_rate: int | None = None,
):
    data = dat.read_bytes()
    pos = 0
    bank = -1
    audio_active = False
    buf = bytearray()
    rate = 22050
    channels = 1
    manifest = []

    def flush(endpos):
        nonlocal buf, audio_active, manifest, bank, rate, channels
        if not audio_active or not buf:
            return
        raw = bytes(buf)
        removed = 0
        if dedupe_repeats is not None:
            raw, removed = collapse_duplicate_blocks(raw, dedupe_repeats)
        out_channels = force_channels or channels
        out_rate = force_rate or rate
        samples = decode_spu(raw, out_channels)
        level = rms(samples)
        frames = len(samples) // out_channels
        if level >= min_rms and (limit is None or len(manifest) < limit):
            dest = (
                out / f"{dat.stem.lower()}_bank_{bank:04d}_0x{endpos - len(buf):x}.wav"
            )
            wav(dest, samples, out_rate, out_channels)
            manifest.append(
                {
                    "bank": bank,
                    "file": str(dest),
                    "bytes": len(buf),
                    "dedupedBytes": len(raw),
                    "duplicateBlocksRemoved": removed,
                    "samples": len(samples),
                    "frames": frames,
                    "duration": frames / out_rate,
                    "sampleRate": out_rate,
                    "headerSampleRate": rate,
                    "channels": out_channels,
                    "headerChannels": channels,
                    "rms": level,
                }
            )
        buf = bytearray()
        audio_active = False

    while pos + 4 <= len(data):
        code = data[pos]
        size = data[pos + 1] | (data[pos + 2] << 8)
        if size < 4 or pos + size > len(data):
            break
        chunk = data[pos : pos + size]
        if code == 0x10:
            ready = struct.unpack_from("<I", chunk, 4)[0] if len(chunk) >= 8 else 0
            if ready == 1:
                bank += 1
                audio_active = True
                buf = bytearray()
                rate = 22050
                channels = 1
        elif code == 0x02 and audio_active:
            rate, channels = params_from_header(chunk)
        elif code == 0x01 and audio_active:
            buf.extend(chunk[4:])
        elif code == 0xF0:
            flush(pos)
            pos += size
            if pos % 0x800:
                pos += 0x800 - (pos % 0x800)
            continue
        pos += size
    flush(pos)
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    with (out / "manifest.csv").open("w", newline="", encoding="utf-8") as f:
        wr = csv.DictWriter(
            f,
            fieldnames=[
                "bank",
                "file",
                "bytes",
                "dedupedBytes",
                "duplicateBlocksRemoved",
                "samples",
                "frames",
                "duration",
                "sampleRate",
                "headerSampleRate",
                "channels",
                "headerChannels",
                "rms",
            ],
        )
        wr.writeheader()
        wr.writerows(manifest)
    return manifest


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dat", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--min-rms", type=float, default=25)
    ap.add_argument("--limit", type=int)
    ap.add_argument(
        "--dedupe-repeats",
        type=int,
        help="Collapse consecutive identical ADPCM blocks, keeping N repeats (try 1 or 2)",
    )
    ap.add_argument(
        "--force-channels",
        type=int,
        choices=[1, 2],
        help="Override header channel count",
    )
    ap.add_argument("--force-rate", type=int, help="Override header sample rate")
    a = ap.parse_args()
    m = parse(
        a.dat,
        a.out,
        a.min_rms,
        a.limit,
        a.dedupe_repeats,
        a.force_channels,
        a.force_rate,
    )
    print(f"Decoded {len(m)} banks to {a.out}")


if __name__ == "__main__":
    main()
