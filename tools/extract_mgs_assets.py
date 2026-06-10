#!/usr/bin/env python3
"""Extract user-owned Metal Gear Solid (PS1 NTSC/US) disc assets.

This tool intentionally does not ship copyrighted assets. It extracts from local
user-provided disc images into assets/generated/ by default.

Supported inputs:
- .cue + .bin (MODE2/2352, common PSX dumps)
- raw .bin/.img MODE2/2352
- .iso/.bin 2048-byte ISO9660 images
- .chd when `chdman` is installed (converted to temporary cue/bin)

It performs three layers:
1. Extract the ISO9660 filesystem from the PSX data track.
2. Scan extracted files for embedded TIM images and write raw .tim chunks.
3. If jPSXdec is installed, optionally run it for exploratory XA/STR/TIM scanning.

The supported asset installer does not require Java or jPSXdec. Canonical codec
voice extraction is handled by the local Python MGS bank parsers.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import struct
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO

DEFAULT_FACE_NAME_MAP = Path("assets/face-name-map.json")

try:
    from PIL import Image
except ImportError:  # Optional; FACE.DAT extraction needs Pillow.
    Image = None

SECTOR_2048 = 2048
SECTOR_2352 = 2352
MODE2_DATA_OFFSET = 24
PVD_SECTOR = 16


@dataclass
class ImageSource:
    image: Path
    sector_size: int
    data_offset: int
    label: str


def log(msg: str) -> None:
    print(msg, flush=True)


def parse_cue(cue: Path) -> Path:
    text = cue.read_text(errors="replace")
    m = re.search(r'FILE\s+"([^"]+)"\s+BINARY', text, re.I)
    if not m:
        m = re.search(r"FILE\s+(\S+)\s+BINARY", text, re.I)
    if not m:
        raise SystemExit(f"Could not find BIN file in cue: {cue}")
    candidate = cue.parent / m.group(1)
    if candidate.exists():
        return candidate
    # Some dumps have uppercase names in cue and mixed case on disk.
    wanted = m.group(1).lower()
    for p in cue.parent.iterdir():
        if p.name.lower() == wanted:
            return p
    raise SystemExit(f"BIN referenced by cue does not exist: {candidate}")


def convert_chd(chd: Path, tmp: Path) -> Path:
    chdman = shutil.which("chdman")
    if not chdman:
        raise SystemExit("CHD input requires `chdman` in PATH")
    cue = tmp / (chd.stem + ".cue")
    subprocess.run([chdman, "extractcd", "-i", str(chd), "-o", str(cue)], check=True)
    return cue


def sector_payload(f: BinaryIO, sector: int, source: ImageSource) -> bytes:
    f.seek(sector * source.sector_size + source.data_offset)
    return f.read(SECTOR_2048)


def detect_source(path: Path) -> ImageSource:
    suffix = path.suffix.lower()
    label = path.stem
    if suffix == ".cue" or path.name.lower().endswith(".cue.txt"):
        image = parse_cue(path)
        label = path.stem.replace(".cue", "")
    else:
        image = path

    with image.open("rb") as f:
        # 2048-byte sector ISO check.
        f.seek(PVD_SECTOR * SECTOR_2048 + 1)
        if f.read(5) == b"CD001":
            return ImageSource(image, SECTOR_2048, 0, label)
        # 2352-byte raw sector with MODE2 data payload at offset 24.
        f.seek(PVD_SECTOR * SECTOR_2352 + MODE2_DATA_OFFSET + 1)
        if f.read(5) == b"CD001":
            return ImageSource(image, SECTOR_2352, MODE2_DATA_OFFSET, label)
        # Some raw images put ISO payload at offset 16.
        f.seek(PVD_SECTOR * SECTOR_2352 + 16 + 1)
        if f.read(5) == b"CD001":
            return ImageSource(image, SECTOR_2352, 16, label)
    raise SystemExit(f"Could not detect ISO9660 payload in {path}")


def clean_name(raw: bytes) -> str:
    name = raw.decode("ascii", errors="replace")
    if name in ("\x00", "\x01"):
        return name
    name = name.split(";")[0]
    return name.strip()


def parse_dir_records(data: bytes) -> list[dict]:
    records = []
    i = 0
    while i < len(data):
        length = data[i]
        if length == 0:
            i = ((i // SECTOR_2048) + 1) * SECTOR_2048
            continue
        rec = data[i : i + length]
        if len(rec) < 34:
            break
        extent = struct.unpack_from("<I", rec, 2)[0]
        size = struct.unpack_from("<I", rec, 10)[0]
        flags = rec[25]
        name_len = rec[32]
        name = clean_name(rec[33 : 33 + name_len])
        records.append(
            {"name": name, "extent": extent, "size": size, "is_dir": bool(flags & 0x02)}
        )
        i += length
    return records


def read_extent(f: BinaryIO, source: ImageSource, extent: int, size: int) -> bytes:
    chunks = []
    remaining = size
    sector = extent
    while remaining > 0:
        payload = sector_payload(f, sector, source)
        take = min(remaining, SECTOR_2048)
        chunks.append(payload[:take])
        remaining -= take
        sector += 1
    return b"".join(chunks)


def extract_iso(source: ImageSource, out_fs: Path) -> list[dict]:
    out_fs.mkdir(parents=True, exist_ok=True)
    manifest = []
    with source.image.open("rb") as f:
        pvd = sector_payload(f, PVD_SECTOR, source)
        root_len = pvd[156]
        root = parse_dir_records(pvd[156 : 156 + root_len])[0]

        def walk(extent: int, size: int, rel: Path) -> None:
            dir_data = read_extent(f, source, extent, size)
            for rec in parse_dir_records(dir_data):
                if rec["name"] in ("\x00", "\x01", "", ".", ".."):
                    continue
                child = rel / rec["name"]
                if rec["is_dir"]:
                    (out_fs / child).mkdir(parents=True, exist_ok=True)
                    walk(rec["extent"], rec["size"], child)
                else:
                    dest = out_fs / child
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    data = read_extent(f, source, rec["extent"], rec["size"])
                    dest.write_bytes(data)
                    manifest.append(
                        {
                            "path": str(child),
                            "size": rec["size"],
                            "extent": rec["extent"],
                            "source": source.image.name,
                        }
                    )

        walk(root["extent"], root["size"], Path())
    return manifest


def parse_tim_at(data: bytes, off: int) -> tuple[int, dict] | None:
    """Return (total_length, info) for a valid TIM at offset, else None."""
    if off + 20 > len(data) or data[off : off + 4] != b"\x10\x00\x00\x00":
        return None
    flags = struct.unpack_from("<I", data, off + 4)[0]
    # Low 3 bits are bpp mode; bit 3 means CLUT present. Reject impossible flags.
    if flags & ~0x0F:
        return None
    bpp = flags & 0x07
    if bpp > 4:
        return None
    cursor = off + 8
    info = {"flags": flags, "bppMode": bpp, "hasClut": bool(flags & 0x08)}
    if flags & 0x08:
        if cursor + 12 > len(data):
            return None
        clut_len = struct.unpack_from("<I", data, cursor)[0]
        if clut_len < 12 or clut_len > 131072 or cursor + clut_len > len(data):
            return None
        _, _, cw, ch = struct.unpack_from("<HHHH", data, cursor + 4)
        if cw == 0 or ch == 0 or cw > 1024 or ch > 512:
            return None
        info["clut"] = {"w": cw, "h": ch, "bytes": clut_len}
        cursor += clut_len
    if cursor + 12 > len(data):
        return None
    img_len = struct.unpack_from("<I", data, cursor)[0]
    if img_len < 12 or img_len > 2_000_000 or cursor + img_len > len(data):
        return None
    _, _, iw, ih = struct.unpack_from("<HHHH", data, cursor + 4)
    if iw == 0 or ih == 0 or iw > 2048 or ih > 1024:
        return None
    info["image"] = {"wWords": iw, "h": ih, "bytes": img_len}
    return cursor + img_len - off, info


def scan_tim_chunks(root: Path, out_tim: Path) -> list[dict]:
    """Find valid TIM chunks in extracted files and save raw .tim chunks."""
    out_tim.mkdir(parents=True, exist_ok=True)
    found = []
    magic = b"\x10\x00\x00\x00"
    for file in root.rglob("*"):
        if not file.is_file():
            continue
        data = file.read_bytes()
        pos = 0
        idx = 0
        while True:
            off = data.find(magic, pos)
            if off < 0:
                break
            parsed = parse_tim_at(data, off)
            if parsed:
                total_len, info = parsed
                idx += 1
                safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", str(file.relative_to(root)))
                dest = out_tim / f"{safe}__tim_{idx:03d}_0x{off:x}.tim"
                dest.write_bytes(data[off : off + total_len])
                found.append(
                    {
                        "source": str(file.relative_to(root)),
                        "offset": off,
                        "bytes": total_len,
                        "info": info,
                        "file": str(dest.relative_to(out_tim.parent)),
                    }
                )
                pos = off + total_len
            else:
                pos = off + 4
    return found


def psx555_to_rgba(word: int) -> tuple[int, int, int, int]:
    r = (word & 0x1F) << 3
    g = ((word >> 5) & 0x1F) << 3
    b = ((word >> 10) & 0x1F) << 3
    # Palette index 0 is usually transparent/background for overlays.
    return (r | (r >> 5), g | (g >> 5), b | (b >> 5), 255)


def decode_face_bitmap(
    blob: bytes, palette: list[tuple[int, int, int, int]], transparent_zero: bool
) -> tuple[Image.Image, dict] | None:
    if Image is None or len(blob) < 4:
        return None
    ox, oy, w, h = blob[0], blob[1], blob[2], blob[3]
    size = w * h
    if w == 0 or h == 0 or len(blob) < 4 + size or w > 52 or h > 89:
        return None
    img = Image.new("RGBA", (52, 89), (0, 0, 0, 0))
    px = img.load()
    pixels = blob[4 : 4 + size]
    for yy in range(h):
        for xx in range(w):
            idx = pixels[yy * w + xx]
            if transparent_zero and idx == 0:
                continue
            color = palette[idx]
            if transparent_zero and color[:3] == (0, 0, 0):
                continue
            if 0 <= ox + xx < 52 and 0 <= oy + yy < 89:
                px[ox + xx, oy + yy] = color
    return img, {"x": ox, "y": oy, "w": w, "h": h}


def extract_face_file_type0(file_data: bytes, out_dir: Path, prefix: str) -> list[dict]:
    if Image is None or len(file_data) < 0x220:
        return []
    offsets = list(struct.unpack_from("<IIIIIIII", file_data, 0))
    pal_off, main_off = offsets[0], offsets[1]
    # Order from psx-spx: mouth2, mouth3, eyes4, eyes5.
    part_offsets = [
        ("mouth_1", offsets[5]),
        ("mouth_2", offsets[6]),
        ("eyes_1", offsets[2]),
        ("eyes_2", offsets[3]),
    ]
    if (
        pal_off <= 0
        or pal_off + 0x200 > len(file_data)
        or main_off <= 0
        or main_off >= len(file_data)
    ):
        return []
    palette = [
        psx555_to_rgba(struct.unpack_from("<H", file_data, pal_off + i * 2)[0])
        for i in range(256)
    ]
    palette[0] = (0, 0, 0, 0)
    entries = []
    base = decode_face_bitmap(file_data[main_off:], palette, transparent_zero=False)
    if not base:
        return []
    base_img, base_rect = base
    base_file = out_dir / f"{prefix}_base.png"
    base_img.save(base_file)
    entries.append({"role": "base", "file": str(base_file), "rect": base_rect})
    for role, off in part_offsets:
        if off <= 0 or off >= len(file_data):
            continue
        decoded = decode_face_bitmap(file_data[off:], palette, transparent_zero=True)
        if not decoded:
            continue
        img, rect = decoded
        part_file = out_dir / f"{prefix}_{role}.png"
        img.save(part_file)
        comp = base_img.copy()
        comp.alpha_composite(img)
        comp_file = out_dir / f"{prefix}_{role}_composed.png"
        comp.save(comp_file)
        entries.append(
            {
                "role": role,
                "file": str(part_file),
                "composed": str(comp_file),
                "rect": rect,
            }
        )
    return entries


def extract_face_file_type1(file_data: bytes, out_dir: Path, prefix: str) -> list[dict]:
    if Image is None or len(file_data) < 4:
        return []
    n = struct.unpack_from("<I", file_data, 0)[0]
    if n <= 0 or n > 64 or 4 + n * 12 > len(file_data):
        return []
    entries = []
    for i in range(n):
        pal_off, bmp_off, unk = struct.unpack_from("<III", file_data, 4 + i * 12)
        if (
            pal_off <= 0
            or pal_off + 0x200 > len(file_data)
            or bmp_off <= 0
            or bmp_off >= len(file_data)
        ):
            continue
        palette = [
            psx555_to_rgba(struct.unpack_from("<H", file_data, pal_off + j * 2)[0])
            for j in range(256)
        ]
        decoded = decode_face_bitmap(
            file_data[bmp_off:], palette, transparent_zero=False
        )
        if not decoded:
            continue
        img, rect = decoded
        frame_file = out_dir / f"{prefix}_frame_{i + 1:03d}.png"
        img.save(frame_file)
        entries.append(
            {
                "role": "full",
                "frame": i + 1,
                "file": str(frame_file),
                "rect": rect,
                "unknown": unk,
            }
        )
    return entries


def strcode16(text: str) -> int:
    h = 0
    for c in text.encode("ascii", errors="ignore"):
        h = (((h >> 11) | ((h << 5) & 0xFFFF)) + c) & 0xFFFF
    return h


def load_face_dictionary(path: Path | None) -> dict[int, str]:
    """Load tools-mgs compatible mgs1-face dictionary.

    Lines should be filenames like `snake.p` or `solid_snake.p`. The hash is
    computed from the stem and matched with extension `p`, exactly like
    Joy-Division/tools-mgs face-extract. This only works when the guessed stem
    hashes to the in-game ID.
    """
    if not path:
        return {}
    mapping = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.split("//", 1)[0].strip()
        if not line:
            continue
        stem = line.rsplit(".", 1)[0]
        mapping[strcode16(stem)] = line
    return mapping


def load_face_name_map(path: Path | None) -> dict[int, str]:
    """Load canonical id->name JSON exported by the visual mapper.

    Accepted keys are treated as hex by default because the mapper displays IDs
    as four-digit FACE.DAT hashes (`f73b`). Use `dec:63291` for decimal keys.
    Values may be `solid_snake` or `solid_snake.p`.
    """
    if not path:
        return {}
    raw = json.loads(path.read_text(encoding="utf-8"))
    mapping = {}
    for key, value in raw.items():
        if value is None or str(value).strip() == "":
            continue
        k = str(key).lower().strip()
        if k.startswith("dec:"):
            fid = int(k[4:], 10)
        else:
            fid = int(k[2:] if k.startswith("0x") else k, 16)
        name = str(value).strip()
        if not name.endswith(".p"):
            name += ".p"
        mapping[fid] = name
    return mapping


def extract_face_dat(
    face_dat: Path, out_root: Path, face_names: dict[int, str] | None = None
) -> dict:
    if Image is None:
        return {
            "available": False,
            "message": "Install Pillow: uv run --with pillow ...",
        }
    data = face_dat.read_bytes()
    out_root.mkdir(parents=True, exist_ok=True)
    face_names = face_names or {}
    manifest = []
    pos = 0
    block_index = 0
    while pos + 4 < len(data):
        n = struct.unpack_from("<I", data, pos)[0]
        if not (0 < n < 256) or pos + 4 + n * 12 > len(data):
            break
        entries = []
        valid = True
        for i in range(n):
            typ, fid, size, off_minus4 = struct.unpack_from(
                "<HHII", data, pos + 4 + i * 12
            )
            if typ not in (0, 1) or size <= 0 or off_minus4 + size > 4_000_000:
                valid = False
                break
            entries.append((typ, fid, size, off_minus4))
        if not valid:
            break
        block_index += 1
        block_dir = out_root / f"block_{block_index:03d}"
        block_dir.mkdir(parents=True, exist_ok=True)
        for file_index, (typ, fid, size, off_minus4) in enumerate(entries, 1):
            start = pos + 4 + off_minus4
            file_data = data[start : start + size]
            name = face_names.get(fid, f"id_{fid:04x}.p")
            safe_name = re.sub(r"[^A-Za-z0-9_.-]+", "_", name.rsplit(".", 1)[0])
            file_dir = (
                block_dir / f"file_{file_index:03d}_{safe_name}_id_{fid:04x}_type_{typ}"
            )
            file_dir.mkdir(parents=True, exist_ok=True)
            if typ == 0:
                frames = extract_face_file_type0(file_data, file_dir, "face")
            else:
                frames = extract_face_file_type1(file_data, file_dir, "face")
            manifest.append(
                {
                    "block": block_index,
                    "fileIndex": file_index,
                    "type": typ,
                    "id": fid,
                    "name": name,
                    "size": size,
                    "offset": start,
                    "frames": frames,
                }
            )
        last = max(entries, key=lambda e: e[3])
        next_pos = pos + 4 + last[3] + last[2]
        pos = (next_pos + 0x7FF) & ~0x7FF
    (out_root / "manifest.json").write_text(
        json.dumps(manifest, indent=2), encoding="utf-8"
    )
    return {
        "available": True,
        "blocks": block_index,
        "files": len(manifest),
        "manifest": str(out_root / "manifest.json"),
    }


def extract_spu_adpcm_chunks(dat_file: Path, out_dir: Path) -> dict:
    """Extract raw SPU-ADPCM chunks from VOX/DEMO DAT.

    Chunks have header: 01h + 24-bit little-endian size, followed by ADPCM data.
    Decoding to WAV is intentionally separate; raw chunks are useful inputs for
    vgmstream/ffmpeg builds that support PSX ADPCM.
    """
    data = dat_file.read_bytes()
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest = []
    i = 0
    while i + 4 < len(data):
        if data[i] == 0x01:
            size = data[i + 1] | (data[i + 2] << 8) | (data[i + 3] << 16)
            if 0 < size <= 0x2004 and i + 4 + size <= len(data):
                chunk = data[i + 4 : i + 4 + size]
                # PSX ADPCM blocks are 16-byte aligned and usually non-empty.
                if len(chunk) >= 16 and len(chunk) % 16 == 0:
                    dest = (
                        out_dir
                        / f"{dat_file.stem.lower()}_{len(manifest) + 1:05d}_0x{i:x}.spu"
                    )
                    dest.write_bytes(chunk)
                    manifest.append(
                        {"offset": i, "bytes": len(chunk), "file": str(dest)}
                    )
                    i += 4 + size
                    continue
        i += 1
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2), encoding="utf-8"
    )
    return {"chunks": len(manifest), "manifest": str(out_dir / "manifest.json")}


def run_jpsxdec(inputs: list[Path], out_media: Path) -> dict:
    exe = shutil.which("jpsxdec") or shutil.which("jpsxdec.jar")
    if not exe:
        return {"available": False, "message": "jPSXdec not found in PATH"}
    out_media.mkdir(parents=True, exist_ok=True)
    results = []
    for image in inputs:
        # jPSXdec CLI differs by packaging. Keep this best-effort and log output.
        log_file = out_media / f"{image.stem}_jpsxdec.log"
        cmd = [exe, "-x", str(image), "-dir", str(out_media / image.stem)]
        try:
            proc = subprocess.run(cmd, text=True, capture_output=True, timeout=3600)
            log_file.write_text(proc.stdout + "\n" + proc.stderr, encoding="utf-8")
            results.append(
                {
                    "image": str(image),
                    "returncode": proc.returncode,
                    "log": str(log_file),
                }
            )
        except Exception as exc:  # noqa: BLE001
            log_file.write_text(str(exc), encoding="utf-8")
            results.append(
                {"image": str(image), "error": str(exc), "log": str(log_file)}
            )
    return {"available": True, "runs": results}


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Extract local user-owned MGS PSX disc assets"
    )
    parser.add_argument(
        "--disc1", type=Path, required=True, help="Disc 1 .cue/.bin/.img/.iso/.chd"
    )
    parser.add_argument("--disc2", type=Path, help="Disc 2 .cue/.bin/.img/.iso/.chd")
    parser.add_argument("--out", type=Path, default=Path("assets/generated/mgs"))
    parser.add_argument(
        "--no-jpsxdec", action="store_true", help="Skip optional jPSXdec pass"
    )
    parser.add_argument(
        "--face-dictionary",
        type=Path,
        help="Optional tools-mgs compatible mgs1-face.txt dictionary",
    )
    parser.add_argument(
        "--face-name-map",
        type=Path,
        default=DEFAULT_FACE_NAME_MAP,
        help="Canonical JSON map of face id to character/name",
    )
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    summary = {"discs": [], "notes": []}
    face_names = load_face_dictionary(args.face_dictionary)
    face_names.update(load_face_name_map(args.face_name_map))

    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        disc_paths = [args.disc1] + ([args.disc2] if args.disc2 else [])
        normalized = []
        for disc_num, input_path in enumerate(disc_paths, 1):
            if input_path.suffix.lower() == ".chd":
                input_path = convert_chd(input_path, tmp / f"disc_{disc_num}")
            source = detect_source(input_path)
            normalized.append(source.image)
            log(
                f"Disc {disc_num}: {source.image} ({source.sector_size}-byte sectors, offset {source.data_offset})"
            )
            fs_out = args.out / f"disc_{disc_num}" / "filesystem"
            files = extract_iso(source, fs_out)
            tims = scan_tim_chunks(fs_out, args.out / f"disc_{disc_num}" / "tim_chunks")
            face = None
            face_path = fs_out / "MGS" / "FACE.DAT"
            if face_path.exists():
                face = extract_face_dat(
                    face_path, args.out / f"disc_{disc_num}" / "face", face_names
                )
            audio = {}
            for name in ("VOX.DAT", "DEMO.DAT"):
                p = fs_out / "MGS" / name
                if p.exists():
                    audio[name] = extract_spu_adpcm_chunks(
                        p,
                        args.out
                        / f"disc_{disc_num}"
                        / "audio_raw"
                        / name.lower().replace(".dat", ""),
                    )
            (args.out / f"disc_{disc_num}" / "filesystem_manifest.json").write_text(
                json.dumps(files, indent=2), encoding="utf-8"
            )
            (args.out / f"disc_{disc_num}" / "tim_chunks_manifest.json").write_text(
                json.dumps(tims, indent=2), encoding="utf-8"
            )
            summary["discs"].append(
                {
                    "disc": disc_num,
                    "input": str(input_path),
                    "image": str(source.image),
                    "sectorSize": source.sector_size,
                    "dataOffset": source.data_offset,
                    "filesystemFiles": len(files),
                    "timChunks": len(tims),
                    "face": face,
                    "audioRaw": audio,
                }
            )
            log(f"  filesystem files: {len(files)}")
            log(f"  likely TIM chunks: {len(tims)}")
            if face:
                log(
                    f"  FACE.DAT: {face.get('blocks')} blocks, {face.get('files')} face files"
                )
            for k, v in audio.items():
                log(f"  {k}: {v.get('chunks')} raw SPU-ADPCM chunks")

        if not args.no_jpsxdec:
            summary["jpsxdec"] = run_jpsxdec(normalized, args.out / "jpsxdec")
            if not summary["jpsxdec"].get("available"):
                summary["notes"].append(
                    "Install jPSXdec for deeper XA/STR/TIM extraction."
                )

    (args.out / "extraction_summary.json").write_text(
        json.dumps(summary, indent=2), encoding="utf-8"
    )
    log(f"\nWrote extraction summary: {args.out / 'extraction_summary.json'}")


if __name__ == "__main__":
    main()
