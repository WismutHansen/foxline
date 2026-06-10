#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
AGENTS_DIR = ROOT / "agents"
WORKER = ROOT / "services" / "qwen3_tts_worker.py"

# Character-specific lines where we have tuned style.
CUSTOM_LINES: dict[str, list[tuple[str, str]]] = {
    "campbell": [
        ("tool_start", "Hold on, Snake. I'm checking it now."),
        ("tool_start", "Stand by. I'm pulling that up."),
        ("tool_start", "Keep your head down while I verify this."),
        ("tool_start", "Give me a second, Snake. I'm looking into it."),
        ("tool_start", "Stay on the channel. I'll check the data."),
        ("tool_start", "Don't move yet. Let me confirm that."),
        ("tool_start", "I'm accessing the records now."),
        ("tool_start", "One moment, Snake. I need to verify this."),
        ("tool_start", "Hold your position. I'm checking our intel."),
        ("tool_start", "Stay alert while I run this down."),
        ("tool_slow", "This is taking longer than expected. Stay sharp."),
        ("tool_slow", "I'm still working on it, Snake. Keep watch."),
        ("tool_slow", "The channel is slow. Don't let your guard down."),
        ("tool_slow", "Almost there. Keep your head low."),
        ("tool_slow", "Stand by, Snake. I'm not done yet."),
        ("tool_done", "I've got it, Snake."),
        ("tool_done", "Listen carefully, Snake."),
        ("tool_done", "The information is coming through now."),
        ("tool_done", "All right, Snake. Here's what I found."),
        ("tool_done", "Confirmed. I'm sending it to you now."),
    ],
    "mei_ling": [
        ("tool_start", "Just a moment, Snake. I'll look it up."),
        ("tool_start", "Hold on. I'm checking the system now."),
        ("tool_start", "Stay there, Snake. I'll find it for you."),
        ("tool_start", "Let me search the records. This should only take a second."),
        ("tool_start", "Keep the codec open. I'm checking."),
        ("tool_start", "One moment, Snake. I'll verify it."),
        ("tool_start", "Don't worry, I'll look into it right away."),
        ("tool_start", "I'm pulling up the data now."),
        ("tool_start", "Give me a second. I think I can find it."),
        ("tool_start", "Stay calm, Snake. Let me check."),
        ("tool_slow", "It's taking a little longer than I thought."),
        ("tool_slow", "Almost there. Please wait just a bit longer."),
        ("tool_slow", "The connection is slow, but I'm still checking."),
        ("tool_slow", "Don't rush. Sometimes patience saves your life."),
        ("tool_slow", "I'm still here, Snake. Keep listening."),
        ("tool_done", "I found it, Snake."),
        ("tool_done", "Okay, I've got the information."),
        ("tool_done", "Here it is. Listen carefully."),
        ("tool_done", "All right, Snake. This should help."),
        ("tool_done", "The answer is ready now."),
    ],
}

# Generic fallback for other characters so manifests exist and generation is possible.
DEFAULT_LINES: list[tuple[str, str]] = [
    ("tool_start", "Stand by. I'm checking that now."),
    ("tool_start", "One moment. I'm looking it up."),
    ("tool_start", "Hold on while I verify this."),
    ("tool_start", "Give me a second, I'm pulling the data."),
    ("tool_start", "Stay on this channel while I check."),
    ("tool_slow", "Still working on it. Please hold."),
    ("tool_slow", "This is taking a bit longer than expected."),
    ("tool_slow", "Almost there. I'm still checking."),
    ("tool_slow", "Connection is slow, but I'm still on it."),
    ("tool_slow", "I'm here. Keep listening."),
    ("tool_done", "I've got it."),
    ("tool_done", "Confirmed. Here's what I found."),
    ("tool_done", "Done. Sending the result now."),
    ("tool_done", "All right, the answer is ready."),
    ("tool_done", "Information complete."),
]


def slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_")[:42]


def load_enabled_agents() -> list[str]:
    manifest = AGENTS_DIR / "manifest.json"
    if not manifest.exists():
        return sorted([p.name for p in AGENTS_DIR.iterdir() if p.is_dir()])
    data = json.loads(manifest.read_text(encoding="utf-8"))
    chars = data.get("characters", [])
    return [c["id"] for c in chars if c.get("enabled", True)]


def reference_for(agent: str) -> tuple[Path | None, Path | None]:
    ref_dir = AGENTS_DIR / agent / "assets" / "reference_audio"
    if not ref_dir.exists():
        return None, None
    wavs = sorted(ref_dir.glob("*_reference.wav"))
    if not wavs:
        return None, None
    wav = wavs[0]
    txt = wav.with_suffix(wav.suffix + ".txt")
    if not txt.exists():
        alt = wav.with_suffix(".txt")
        txt = alt if alt.exists() else None
    return wav, txt


def run_worker(text: str, out_path: Path, ref_wav: Path, ref_txt: Path, model: str, output_sr: int) -> None:
    cmd = [
        "uv",
        "run",
        "--no-project",
        "--with",
        "speech-to-speech==0.2.9",
        "python",
        str(WORKER),
        "--text",
        text,
        "--output",
        str(out_path),
        "--model-name",
        model,
        "--ref-audio",
        str(ref_wav),
        "--ref-text-file",
        str(ref_txt),
        "--language",
        "auto",
        "--output-sample-rate",
        str(output_sr),
        "--temperature",
        "0.7",
        "--top-k",
        "30",
    ]
    subprocess.run(cmd, check=True)


def main() -> None:
    ap = argparse.ArgumentParser(description="Generate codec filler WAVs using Qwen worker and write filler manifests.")
    ap.add_argument("--agents", default="all", help="Comma-separated agent ids, or 'all' (default).")
    ap.add_argument("--model", default="mlx-community/Qwen3-TTS-12Hz-0.6B-Base-4bit")
    ap.add_argument("--output-sr", type=int, default=24000)
    ap.add_argument("--ensure-manifests-only", action="store_true", help="Only write manifest.json files; do not synthesize audio.")
    args = ap.parse_args()

    enabled = load_enabled_agents()
    selected = enabled if args.agents == "all" else [a.strip() for a in args.agents.split(",") if a.strip()]

    for agent in selected:
        out_dir = AGENTS_DIR / agent / "assets" / "filler"
        out_dir.mkdir(parents=True, exist_ok=True)
        lines = CUSTOM_LINES.get(agent, DEFAULT_LINES)
        ref_wav, ref_txt = reference_for(agent)

        manifest: list[dict[str, str]] = []
        for i, (category, text) in enumerate(lines, 1):
            path = out_dir / f"{category}_{i:02d}_{slug(text)}.wav"
            if path.exists() and path.stat().st_size > 1000:
                print(f"skip {path}")
            elif args.ensure_manifests_only:
                print(f"manifest-only {agent} {i:02d}: {text}")
            elif not ref_wav or not ref_txt:
                print(f"skip-generate {agent}: missing reference wav/txt")
            else:
                print(f"generate {agent} {i:02d}: {text}")
                run_worker(text, path, ref_wav, ref_txt, args.model, args.output_sr)
                print(f"  -> {path}")
            manifest.append({"agent": agent, "category": category, "text": text, "path": str(path.relative_to(ROOT))})

        (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        print(f"wrote {out_dir / 'manifest.json'}")


if __name__ == "__main__":
    main()
