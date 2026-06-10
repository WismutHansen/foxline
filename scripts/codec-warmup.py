#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["websockets>=15.0"]
# ///
from __future__ import annotations

import argparse
import asyncio
import json
import time

import websockets

PROMPTS = [
    "Colonel, confirm radio check in one short line.",
    "Give me one tactical reminder before I move.",
    "Search online for latest AI news and summarize quickly.",
]


async def wait_ready(ws_url: str, timeout_s: float = 45.0) -> None:
    started = time.perf_counter()
    while True:
        try:
            async with websockets.connect(ws_url, max_size=8_000_000) as ws:
                _ = json.loads(await asyncio.wait_for(ws.recv(), timeout=10))
                return
        except Exception:
            if time.perf_counter() - started > timeout_s:
                raise TimeoutError(f"Bridge not ready at {ws_url} after {timeout_s}s")
            await asyncio.sleep(0.25)


async def run_turn(ws, prompt: str, timeout_s: float = 180.0) -> tuple[float, float]:
    started = time.perf_counter()
    await ws.send(json.dumps({"type": "user_utterance", "text": prompt}))

    turn_id = None
    first_delta = None
    completed = None

    while True:
        msg = json.loads(await asyncio.wait_for(ws.recv(), timeout=timeout_s))
        now = time.perf_counter()
        mtype = msg.get("type")

        if mtype == "turn_started" and turn_id is None:
            turn_id = msg.get("turnId")
            continue

        if turn_id is not None and msg.get("turnId") not in (turn_id, None):
            continue

        if mtype == "assistant_delta" and first_delta is None:
            first_delta = now
        if mtype == "turn_completed":
            completed = now
            break

    ttft = (first_delta - started) if first_delta else float("nan")
    total = (completed - started) if completed else float("nan")
    return ttft, total


async def main() -> None:
    parser = argparse.ArgumentParser(description="Warm up Codec bridge model path")
    parser.add_argument("--ws", default="ws://127.0.0.1:8770")
    parser.add_argument("--turns", type=int, default=2)
    parser.add_argument("--new-session", action="store_true")
    args = parser.parse_args()

    await wait_ready(args.ws)
    async with websockets.connect(args.ws, max_size=8_000_000) as ws:
        _ = json.loads(await asyncio.wait_for(ws.recv(), timeout=10))

        for i in range(args.turns):
            prompt = PROMPTS[i % len(PROMPTS)]
            ttft, total = await run_turn(ws, prompt)
            print(f"warmup {i+1}/{args.turns}: ttft_text={ttft:.3f}s total={total:.3f}s")

        if args.new_session:
            await ws.send(json.dumps({"type": "new_session"}))
            await asyncio.sleep(0.2)
            print("sent new_session")


if __name__ == "__main__":
    asyncio.run(main())
