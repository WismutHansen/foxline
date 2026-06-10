#!/usr/bin/env python3
"""Extract assets from a user-provided MGS PC installer or installed game dir.

This does not execute the Windows installer. For GOG/Inno installers it uses
innoextract to unpack the payload. For existing PC installs, pass the directory
that contains efx.mgz, face.dat, VOX/, MDX/, etc. It uses 7zz to unpack .mgz ZIP
containers.
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

try:
    from tools.extract_mgs_assets import extract_face_dat, load_face_name_map
except Exception:  # Pillow may be missing unless run with --with pillow.
    extract_face_dat = None
    load_face_name_map = None


def require(cmd: str) -> str:
    path = shutil.which(cmd)
    if not path:
        raise SystemExit(f"required command not found: {cmd}")
    return path


def run(cmd: list[str]) -> None:
    print("+", " ".join(cmd))
    subprocess.run(cmd, check=True)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("source", type=Path, help="setup_metal_gear_solid_*.exe or installed PC game directory")
    ap.add_argument("--out", type=Path, default=Path("assets/generated/mgs_pc"))
    ap.add_argument("--work", type=Path, default=Path("/tmp/mgs_pc_extract_work"))
    ap.add_argument("--keep-work", action="store_true")
    ap.add_argument("--face-name-map", type=Path, default=Path("assets/face-name-map.json"))
    ap.add_argument("--skip-face", action="store_true", help="Do not extract PC face.dat avatars")
    args = ap.parse_args()

    require("7zz")
    if not args.source.exists():
        raise SystemExit(f"source not found: {args.source}")

    if args.work.exists():
        shutil.rmtree(args.work)
    args.work.mkdir(parents=True)
    args.out.mkdir(parents=True, exist_ok=True)

    if args.source.is_dir():
        # Normalize an existing PC install/media directory into the work dir.
        for item in args.source.iterdir():
            dst = args.work / item.name
            if item.is_dir():
                shutil.copytree(item, dst)
            else:
                shutil.copy2(item, dst)
    else:
        require("innoextract")
        # Extract the installer payload. The path filters are unreliable across
        # innoextract versions for this installer, so unpack to temp and copy only
        # relevant files into the canonical generated output.
        run(["innoextract", "-d", str(args.work), "--exclude-temp", str(args.source)])

    manifest_lines = []
    for name in ["efx.mgz", "stage.mgz", "stagevr.mgz", "radio.dat", "brf.dat", "brfvr.dat", "face.dat", "tga.mgz"]:
        src = args.work / name
        if src.exists():
            dst = args.out / name
            shutil.copy2(src, dst)
            manifest_lines.append(f"{name}\t{src.stat().st_size}")

    vox_src = args.work / "VOX"
    if vox_src.exists():
        vox_dst = args.out / "VOX"
        if vox_dst.exists():
            shutil.rmtree(vox_dst)
        shutil.copytree(vox_src, vox_dst)
        manifest_lines.append(f"VOX/*.vox\t{len(list(vox_dst.glob('*.vox')))} files")

    vox_dst_for_decode = args.out / "VOX"
    if vox_dst_for_decode.exists():
        vox_wav_out = args.out / "VOX_wav"
        if vox_wav_out.exists():
            shutil.rmtree(vox_wav_out)
        run([
            sys.executable,
            str(Path(__file__).resolve().parent / "decode_mgs_pc_vox.py"),
            "--input", str(vox_dst_for_decode),
            "--out", str(vox_wav_out),
        ])
        manifest_lines.append(f"VOX decoded wavs\t{len(list(vox_wav_out.glob('*.wav')))} files")
        crosswalk_script = Path(__file__).resolve().parent / "build_mgs_pc_vox_crosswalk.py"
        psx_root = Path("assets/generated/mgs/audio_wav_banks")
        psx_index = Path("assets/mgs-vox-bank-index.json")
        if crosswalk_script.exists() and (psx_root.exists() or psx_index.exists()):
            run([
                sys.executable,
                str(crosswalk_script),
                "--pc-manifest", str(vox_wav_out / "manifest.csv"),
                "--psx-root", str(psx_root),
                "--psx-index", str(psx_index),
                "--out", str(args.out / "vox-psx-crosswalk.json"),
            ])
            manifest_lines.append("VOX PSX crosswalk\tvox-psx-crosswalk.json")

    mdx_src = args.work / "MDX"
    if mdx_src.exists():
        mdx_dst = args.out / "MDX"
        if mdx_dst.exists():
            shutil.rmtree(mdx_dst)
        shutil.copytree(mdx_src, mdx_dst)
        manifest_lines.append(f"MDX/*.wav\t{len(list(mdx_dst.glob('*.wav')))} files")

    face_dat = args.out / "face.dat"
    if face_dat.exists() and not args.skip_face:
        if extract_face_dat is None or load_face_name_map is None:
            manifest_lines.append("face.dat extraction\tskipped: install Pillow / run with uv --with pillow")
        else:
            face_names = load_face_name_map(args.face_name_map)
            face_out = args.out / "face"
            if face_out.exists():
                shutil.rmtree(face_out)
            result = extract_face_dat(face_dat, face_out, face_names)
            manifest_lines.append(
                f"face.dat extracted\t{result.get('blocks', 0)} blocks, {result.get('files', 0)} face files"
            )

    efx_mgz = args.out / "efx.mgz"
    if efx_mgz.exists():
        efx_out = args.out / "efx"
        if efx_out.exists():
            shutil.rmtree(efx_out)
        efx_out.mkdir(parents=True)
        run(["7zz", "x", "-y", f"-o{efx_out}", str(efx_mgz)])
        wavs = list(efx_out.rglob("*.wav"))
        manifest_lines.append(f"efx extracted wavs\t{len(wavs)} files")

    (args.out / "manifest.txt").write_text("\n".join(manifest_lines) + "\n", encoding="utf-8")
    print(f"wrote {args.out}")

    if not args.keep_work:
        shutil.rmtree(args.work)


if __name__ == "__main__":
    main()
