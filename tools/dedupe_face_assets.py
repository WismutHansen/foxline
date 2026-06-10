#!/usr/bin/env python3
"""Create a deduplicated FACE.DAT asset tree from extracted MGS faces.

Usage:
  uv run tools/dedupe_face_assets.py --mgs-dir assets/generated/mgs

Reads PSX disc_*/face/manifest.json or PC face/manifest.json, hashes PNG
contents, and copies only unique PNGs into unique_faces/<name-or-id>/ while
writing manifest.json.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
from pathlib import Path

DEFAULT_FACE_NAME_MAP = Path("assets/face-name-map.json")

AGENT_BY_FACE_PREFIX = {
    "campbell": "campbell",
    "houseman": "houseman",
    "liquid": "liquid",
    "mei_ling": "mei_ling",
    "meryl": "meryl",
    "meryl2side": "meryl",
    "miller": "miller",
    "miller2liquid": "liquid",
    "naomi": "naomi",
    "nastasha": "nastasha",
    "otacon": "otacon",
    "snake": "snake",
    "wolf": "wolf",
}


def safe(s: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", s).strip("_").lower() or "unknown"


def rel_to_mgs(path_str: str, mgs_dir: Path) -> Path:
    p = Path(path_str)
    if p.is_absolute():
        return p
    if p.exists():
        return p
    marker = "assets/generated/mgs/"
    if marker in path_str:
        return mgs_dir / path_str.split(marker, 1)[1]
    return p


def load_face_name_map(path: Path | None) -> dict[str, str]:
    if not path or not path.exists():
        return {}
    raw = json.loads(path.read_text(encoding="utf-8"))
    mapping = {}
    for key, value in raw.items():
        if value is None or str(value).strip() == "":
            continue
        k = str(key).lower().strip()
        if k.startswith("dec:"):
            face_id = f"{int(k[4:], 10):04x}"
        else:
            face_id = f"{int(k[2:] if k.startswith('0x') else k, 16):04x}"
        mapping[face_id] = safe(str(value).rsplit(".", 1)[0])
    return mapping


def agent_for_face_name(name: str) -> str | None:
    if name in AGENT_BY_FACE_PREFIX:
        return AGENT_BY_FACE_PREFIX[name]
    for prefix in sorted(AGENT_BY_FACE_PREFIX, key=len, reverse=True):
        if name.startswith(prefix + "_"):
            return AGENT_BY_FACE_PREFIX[prefix]
    prefix = name.split("_", 1)[0]
    return AGENT_BY_FACE_PREFIX.get(prefix)


def copy_unique_faces_to_agents(unique_faces_dir: Path, agents_dir: Path) -> int:
    copied = 0
    for face_dir in sorted(p for p in unique_faces_dir.iterdir() if p.is_dir()):
        agent = agent_for_face_name(face_dir.name)
        if not agent:
            print(f"Skipping unmapped avatar directory: {face_dir.name}")
            continue
        dest = agents_dir / agent / "assets" / "avatar" / face_dir.name
        if dest.exists():
            shutil.rmtree(dest)
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(face_dir, dest)
        copied += sum(1 for _ in dest.rglob("*.png"))
    return copied


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mgs-dir", type=Path, default=Path("assets/generated/mgs"))
    ap.add_argument("--out", type=Path)
    ap.add_argument("--face-name-map", type=Path, default=DEFAULT_FACE_NAME_MAP, help="Canonical face id to avatar name map")
    ap.add_argument("--agents-dir", type=Path, default=Path("agents"), help="Copy avatars to this agents directory")
    ap.add_argument("--no-agent-copy", action="store_true", help="Only write unique_faces; do not populate agents/*/assets/avatar")
    args = ap.parse_args()
    mgs_dir = args.mgs_dir.resolve()
    out = args.out.resolve() if args.out else mgs_dir / "unique_faces"
    face_name_map = load_face_name_map(args.face_name_map)
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    seen: dict[str, dict] = {}
    unique = []
    occurrences = []

    manifest_paths = sorted(mgs_dir.glob("disc_*/face/manifest.json"))
    pc_manifest = mgs_dir / "face" / "manifest.json"
    if pc_manifest.exists():
        manifest_paths.append(pc_manifest)

    for manifest_path in manifest_paths:
        disc = manifest_path.parts[-3] if manifest_path.parent.name == "face" and manifest_path.parent.parent.name.startswith("disc_") else "pc"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        for item in manifest:
            face_id = f"{int(item['id']):04x}"
            name = face_name_map.get(face_id) or safe(str(item.get("name", f"id_{item['id']:04x}")).rsplit(".", 1)[0])
            for frame in item.get("frames", []):
                for kind, key in [("source", "file"), ("composed", "composed")]:
                    if key not in frame:
                        continue
                    src = rel_to_mgs(frame[key], mgs_dir)
                    if not src.exists():
                        continue
                    digest = hashlib.sha256(src.read_bytes()).hexdigest()
                    if digest not in seen:
                        role = safe(frame.get("role", kind))
                        dest_dir = out / name
                        dest_dir.mkdir(parents=True, exist_ok=True)
                        dest = dest_dir / f"{name}__id_{face_id}__{role}__{kind}__{len(seen)+1:04d}.png"
                        shutil.copy2(src, dest)
                        seen[digest] = {
                            "hash": digest,
                            "name": name,
                            "faceId": face_id,
                            "role": frame.get("role"),
                            "kind": kind,
                            "file": str(dest.relative_to(out)),
                            "firstSource": str(src),
                        }
                        unique.append(seen[digest])
                    occurrences.append({
                        "hash": digest,
                        "disc": disc,
                        "block": item.get("block"),
                        "fileIndex": item.get("fileIndex"),
                        "faceId": face_id,
                        "name": name,
                        "role": frame.get("role"),
                        "kind": kind,
                        "source": str(src),
                        "uniqueFile": seen[digest]["file"],
                    })

    (out / "manifest.json").write_text(json.dumps({"unique": unique, "occurrences": occurrences}, indent=2), encoding="utf-8")
    print(f"Unique PNGs: {len(unique)}")
    print(f"Occurrences: {len(occurrences)}")
    print(f"Wrote {out}")
    if not args.no_agent_copy:
        copied = copy_unique_faces_to_agents(out, args.agents_dir.resolve())
        print(f"Copied {copied} avatar PNGs to {args.agents_dir}/<agent>/assets/avatar")


if __name__ == "__main__":
    main()
