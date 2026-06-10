#!/usr/bin/env python3
"""Install user-provided MGS assets into this repo.

Accepts either:
- PSX NTSC/US disc images (`--psx-disc1`, optional `--psx-disc2`)
- PC/GOG installer exe (`--pc-installer`)
- existing PC game directory (`--pc-dir`)

Generated copyrighted outputs stay under assets/generated/. App-facing agent
assets are copied/generated under agents/*/assets/.
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

REFS = {
    "campbell": {"out": "Campbell_reference.wav", "sources": [(1, 343)]},
    "mei_ling": {"out": "Mei_Ling_reference.wav", "sources": [(1, 571)]},
    "meryl": {"out": "Meryl_reference.wav", "sources": [(1, 842), (1, 848)]},
    "miller": {"out": "Miller_reference.wav", "sources": [(1, 1066)]},
    "naomi": {"out": "Naomi_reference.wav", "sources": [(2, 213), (2, 211), (2, 198)]},
    "nastasha": {"out": "Nastasha_reference.wav", "sources": [(1, 651)]},
    "otacon": {"out": "Otacon_reference.wav", "sources": [(1, 868)]},
}


def run(cmd: list[object], *, cwd: Path = ROOT) -> None:
    print("+", " ".join(map(str, cmd)))
    subprocess.run([str(c) for c in cmd], cwd=cwd, check=True)


def require(cmd: str) -> None:
    if shutil.which(cmd) is None:
        raise SystemExit(f"required command not found: {cmd}")


def wav_concat(inputs: list[Path], out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    if len(inputs) == 1:
        shutil.copy2(inputs[0], out)
        return
    require("ffmpeg")
    list_file = out.with_suffix(".concat.txt")
    list_file.write_text("".join(f"file '{p.resolve()}'\n" for p in inputs), encoding="utf-8")
    try:
        run(["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", list_file, "-c", "copy", out])
    finally:
        list_file.unlink(missing_ok=True)


def psx_bank_wav(mgs_out: Path, disc: int, bank: int) -> Path:
    matches = sorted((mgs_out / f"disc_{disc}" / "vox").glob(f"vox_bank_{bank:04d}_*.wav"))
    if not matches:
        raise FileNotFoundError(f"missing PSX decoded bank disc {disc} bank {bank} in {mgs_out}")
    return matches[0]


def load_pc_crosswalk(pc_out: Path) -> dict[tuple[int, int], Path]:
    data = json.loads((pc_out / "vox-psx-crosswalk.json").read_text(encoding="utf-8"))
    result: dict[tuple[int, int], Path] = {}
    for e in data.get("entries", []):
        m = e.get("match") or {}
        if not m:
            continue
        key = (int(m["disc"]), int(m["bank"]))
        if key not in result or e.get("status") in {"matched", "fuzzy"}:
            result[key] = ROOT / e["pcWav"] if not Path(e["pcWav"]).is_absolute() else Path(e["pcWav"])
    return result


def install_reference_audio(source: str, psx_out: Path, pc_out: Path, install_manifest: dict[str, object]) -> None:
    pc_map = load_pc_crosswalk(pc_out) if source == "pc" else {}
    refs = []
    for agent, spec in REFS.items():
        inputs: list[Path] = []
        for disc, bank in spec["sources"]:
            if source == "pc":
                try:
                    inputs.append(pc_map[(disc, bank)])
                except KeyError as exc:
                    raise FileNotFoundError(f"PC crosswalk missing disc {disc} bank {bank} for {agent}") from exc
            else:
                inputs.append(psx_bank_wav(psx_out, disc, bank))
        out_dir = ROOT / "agents" / agent / "assets" / "reference_audio"
        out = out_dir / spec["out"]
        wav_concat(inputs, out)
        meta = {
            "character": agent,
            "installedFrom": source,
            "sources": [{"disc": d, "bank": b} for d, b in spec["sources"]],
            "sourceFiles": [str(p.relative_to(ROOT) if p.is_relative_to(ROOT) else p) for p in inputs],
            "selection": "project_confirmed_reference",
        }
        out.with_suffix(".json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        refs.append({**meta, "target": str(out.relative_to(ROOT))})
        print(f"reference {agent}: {out}")
    install_manifest["referenceAudio"] = refs


def transcribe_reference_audio(base_url: str) -> None:
    for wav in sorted((ROOT / "agents").glob("*/assets/reference_audio/*_reference.wav")):
        req = urllib.request.Request(f"{base_url.rstrip('/')}/transcribe", method="POST")
        boundary = "codec-boundary"
        data = wav.read_bytes()
        body = (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="audio"; filename="{wav.name}"\r\n'
            "Content-Type: audio/wav\r\n\r\n"
        ).encode() + data + f"\r\n--{boundary}--\r\n".encode()
        req.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
        req.add_header("Content-Length", str(len(body)))
        with urllib.request.urlopen(req, data=body, timeout=180) as res:
            parsed = json.loads(res.read().decode())
        text = str(parsed.get("text") or "").strip()
        wav.with_suffix(wav.suffix + ".txt").write_text(text + "\n", encoding="utf-8")
        print(f"transcript {wav}: {text}")


def psx_extract(args: argparse.Namespace) -> None:
    cmd: list[object] = [sys.executable, "tools/extract_mgs_assets.py", "--disc1", args.psx_disc1, "--out", args.psx_out]
    if args.psx_disc2:
        cmd.extend(["--disc2", args.psx_disc2])
    run(cmd)
    for disc in [1, 2]:
        dat = Path(args.psx_out) / f"disc_{disc}" / "filesystem" / "MGS" / "VOX.DAT"
        if dat.exists():
            run([sys.executable, "tools/extract_mgs_audio_banks.py", "--dat", dat, "--out", Path(args.psx_out) / f"disc_{disc}" / "vox"])
    run([sys.executable, "tools/dedupe_face_assets.py", "--mgs-dir", args.psx_out, "--face-name-map", "assets/face-name-map.json"])


def psx_render_sfx(args: argparse.Namespace, install_manifest: dict[str, object]) -> None:
    """Render game sound effects (codec call, tuning, alert, UI...) and music
    at correct pitch by simulating the game's sound driver against the
    extracted disc."""
    fs = Path(args.psx_out) / "disc_1" / "filesystem"
    out = Path(args.psx_out) / "disc_1" / "sfx"
    if not (fs / "MGS" / "STAGE.DIR").exists():
        print("warning: STAGE.DIR not found; skipping SFX render")
        return
    cmd: list[object] = [sys.executable, "tools/render_mgs_sfx.py", "--filesystem", fs, "--out", out]
    if not args.skip_music:
        cmd.append("--music")
    run(cmd)
    install_manifest["sfx"] = str(out)


def pc_extract(args: argparse.Namespace) -> None:
    source = args.pc_installer or args.pc_dir
    run([sys.executable, "tools/extract_mgs_pc_assets.py", source, "--out", args.pc_out])
    if (Path(args.pc_out) / "face" / "manifest.json").exists():
        run([sys.executable, "tools/dedupe_face_assets.py", "--mgs-dir", args.pc_out, "--face-name-map", "assets/face-name-map.json"])


# Frontend SFX names -> (PSX builtin SE code, name in assets/efx-name-map.json).
# The PC port ships only pre-rendered 8-bit effects; these user-auditioned
# matches give PC-only installs working (lower-fidelity) codec UI sounds.
PC_SFX_FALLBACK = {
    "codec_call": (86, "codec_ring_outgoing"),
    "radio_receive": (16, "codec_ring_incoming"),
    "codec_tune": (103, "codec_frequency_change"),
    "radio_window_open": (84, "codec_menu"),
    "radio_window_close": (87, "codec_conversation_closed"),
    "radio_select": (85, "main_menu_selection"),
    "item_select": (23, "main_menu_selection"),
    "radio_cursor": (105, "item_menu_move"),
    "cursor": (31, "item_menu_move"),
    "radio_cancel": (104, "access_denied"),
    "alert_bikkuri": (83, "enemy_discovered"),
}


def pc_install_sfx_fallback(args: argparse.Namespace, install_manifest: dict[str, object]) -> None:
    name_map_path = ROOT / "assets" / "efx-name-map.json"
    if not name_map_path.exists():
        print("warning: assets/efx-name-map.json missing; skipping PC SFX fallback")
        return
    require("ffmpeg")
    name_map = json.loads(name_map_path.read_text(encoding="utf-8"))
    by_name = {v["name"]: k for k, v in name_map.items()}
    out_dir = Path(args.pc_out) / "sfx" / "builtin"
    out_dir.mkdir(parents=True, exist_ok=True)
    installed = []
    for ui_name, (code, pc_name) in PC_SFX_FALLBACK.items():
        rel = by_name.get(pc_name)
        src = Path(args.pc_out) / "efx" / rel if rel else None
        if src is None or not src.exists():
            print(f"  pc sfx fallback: no source for {ui_name} ({pc_name})")
            continue
        dest = out_dir / f"se{code:03d}_{ui_name}.wav"
        run(["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-i", src,
             "-af", "aformat=sample_fmts=s16,aresample=44100", dest])
        installed.append({"name": ui_name, "source": str(src.relative_to(ROOT) if src.is_relative_to(ROOT) else src), "file": str(dest)})
    (out_dir / "manifest.json").write_text(json.dumps(installed, indent=2), encoding="utf-8")
    install_manifest["sfxFallback"] = str(out_dir)
    print(f"PC SFX fallback: {len(installed)} sounds -> {out_dir}")


def pc_install_music(args: argparse.Namespace, install_manifest: dict[str, object]) -> None:
    """The PC port ships the soundtrack pre-rendered (MDX/*.wav, unsigned
    8-bit 22kHz stereo — the folder kept the PSX sequence format's name but
    holds recordings). Convert to 16-bit/44.1kHz for app use; fidelity is
    capped by the 8-bit originals."""
    src_dir = Path(args.pc_out) / "MDX"
    if not src_dir.is_dir():
        print("note: no MDX/ folder in PC extraction; skipping music")
        return
    require("ffmpeg")
    out_dir = Path(args.pc_out) / "sfx" / "music"
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    for src in sorted(src_dir.glob("*.wav")):
        dest = out_dir / f"pc_track_{src.stem}.wav"
        run(["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-i", src,
             "-af", "aformat=sample_fmts=s16,aresample=44100", dest])
        rows.append({"name": dest.stem, "file": dest.name, "source": src.name})
    (out_dir / "manifest.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
    install_manifest["musicFallback"] = str(out_dir)
    print(f"PC music: {len(rows)} pre-rendered tracks -> {out_dir}")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    src = p.add_mutually_exclusive_group(required=True)
    src.add_argument("--pc-installer", type=Path)
    src.add_argument("--pc-dir", type=Path)
    src.add_argument("--psx-disc1", type=Path)
    p.add_argument("--psx-disc2", type=Path)
    p.add_argument("--psx-out", type=Path, default=Path("assets/generated/mgs"))
    p.add_argument("--pc-out", type=Path, default=Path("assets/generated/mgs_pc"))
    p.add_argument("--transcribe", action="store_true", help="Generate .wav.txt transcripts through Parakeet STT server")
    p.add_argument("--parakeet-url", default="http://127.0.0.1:8780")
    p.add_argument("--generate-fillers", action="store_true", help="Generate filler snippets through the current Qwen3-TTS filler generator")
    p.add_argument("--skip-sfx", action="store_true", help="Skip rendering game sound effects from the PSX disc")
    p.add_argument("--skip-music", action="store_true", help="Skip rendering game music (saves several minutes)")
    args = p.parse_args()

    manifest: dict[str, object] = {"sourceType": "psx" if args.psx_disc1 else "pc", "generated": {}}
    if args.psx_disc1:
        if not args.psx_disc2:
            print("warning: --psx-disc2 omitted; Naomi reference requires disc 2")
        psx_extract(args)
        manifest["generated"] = {"psx": str(args.psx_out)}
        install_reference_audio("psx", args.psx_out, args.pc_out, manifest)
        if not args.skip_sfx:
            psx_render_sfx(args, manifest)
    else:
        pc_extract(args)
        manifest["generated"] = {"pc": str(args.pc_out)}
        install_reference_audio("pc", args.psx_out, args.pc_out, manifest)
        if not args.skip_sfx:
            pc_install_sfx_fallback(args, manifest)
        if not args.skip_music:
            pc_install_music(args, manifest)
        print("note: PC media ships SFX and music as pre-rendered 8-bit audio; installing")
        print("      from a PSX disc image instead (just mgs-install-psx) renders both at")
        print("      full fidelity via sound-driver simulation")

    if args.transcribe:
        transcribe_reference_audio(args.parakeet_url)
        run(["uv", "run", "--script", "tools/normalize_reference_transcripts.py"])
        manifest["transcripts"] = "generated"
    if args.generate_fillers:
        run(["uv", "run", "--script", "tools/generate_codec_fillers.py"])
        manifest["fillers"] = "generated"

    out_manifest = ROOT / "assets" / "generated" / "mgs_install_manifest.json"
    out_manifest.parent.mkdir(parents=True, exist_ok=True)
    out_manifest.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"wrote {out_manifest}")
    print("MGS user asset installation complete")


if __name__ == "__main__":
    main()
