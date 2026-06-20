#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "fastapi>=0.115",
#   "uvicorn[standard]>=0.30",
#   "numpy>=1.26",
# ]
# ///
"""Codec WebSocket STT server using PiBot's parakeet.cpp + Silero VAD worker.

Protocol-compatible with the Codec browser STT service:
- Browser sends JSON commands (setlanguage/getstatus/stop) and Float32 PCM chunks.
- Server forwards 16 kHz int16 PCM to PiBot's C++ worker.
- Server emits {type:"word"} for final words and {type:"final", text} per utterance.
"""
from __future__ import annotations

import argparse
import asyncio
import json
import os
import struct
import time
from pathlib import Path
from typing import Any

import numpy as np
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.responses import JSONResponse

DEFAULT_WORKER = str(Path.home() / "repos/pibot/native/parakeet-cpp-stt/build/parakeet-cpp-stt-worker")
DEFAULT_MODEL = str(Path.home() / "models/parakeet-cpp-gguf/tdt-0.6b-v3-q8_0.gguf")
DEFAULT_VAD = str(Path.home() / "models/whisper-vad/ggml-silero-v6.2.0.bin")


def resample_linear(x: np.ndarray, src: int, dst: int) -> np.ndarray:
    if src == dst:
        return x.astype(np.float32, copy=False)
    if len(x) == 0:
        return x.astype(np.float32)
    n = max(1, int(round(len(x) * dst / src)))
    old = np.linspace(0.0, 1.0, num=len(x), endpoint=False)
    new = np.linspace(0.0, 1.0, num=n, endpoint=False)
    return np.interp(new, old, x).astype(np.float32)


def pcm16le(samples: np.ndarray) -> bytes:
    return (np.clip(samples, -1.0, 1.0) * 32767.0).astype("<i2").tobytes()


class WorkerSession:
    def __init__(self, worker: str, model: str, vad_model: str, input_rate: int, min_silence_ms: int):
        self.worker = worker
        self.model = model
        self.vad_model = vad_model
        self.input_rate = input_rate
        self.min_silence_ms = min_silence_ms
        self.proc: asyncio.subprocess.Process | None = None
        self.ready = asyncio.Event()
        self.started_at = time.perf_counter()

    async def start(self, ws: WebSocket) -> None:
        for label, path in (("worker", self.worker), ("model", self.model), ("vad", self.vad_model)):
            if not Path(path).exists():
                raise RuntimeError(f"{label} missing: {path}")
        self.proc = await asyncio.create_subprocess_exec(
            self.worker,
            self.model,
            self.vad_model,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env={
                **os.environ,
                "PARAKEET_MIN_SILENCE_MS": str(self.min_silence_ms),
                "PARAKEET_PREROLL_MS": os.environ.get("PARAKEET_PREROLL_MS", "1800"),
            },
        )
        asyncio.create_task(self._stdout_loop(ws))
        asyncio.create_task(self._stderr_loop(ws))

    async def _send(self, ws: WebSocket, obj: dict[str, Any]) -> None:
        try:
            await ws.send_text(json.dumps(obj, ensure_ascii=False))
        except RuntimeError:
            pass

    async def _stdout_loop(self, ws: WebSocket) -> None:
        if not self.proc or not self.proc.stdout:
            return
        while line := await self.proc.stdout.readline():
            try:
                msg = json.loads(line.decode("utf-8", errors="replace").strip())
            except Exception:
                continue
            typ = msg.get("type")
            if typ == "ready":
                self.ready.set()
                await self._send(ws, {"type": "status", "message": f"parakeet-silero ready silence={msg.get('minSilenceMs')}ms vad={msg.get('vadThreshold')}"})
            elif typ == "speech_start":
                await self._send(ws, {"type": "status", "message": f"speech start #{msg.get('index')}"})
            elif typ == "speech_end":
                await self._send(ws, {"type": "status", "message": f"speech end {float(msg.get('duration') or 0):.2f}s"})
            elif typ == "speech_drop":
                await self._send(ws, {"type": "status", "message": f"speech drop {msg.get('reason')}"})
            elif typ == "interim" and msg.get("text"):
                text = str(msg.get("text") or "").strip()
                await self._send(ws, {"type": "interim", "text": text})
            elif typ == "final":
                text = str(msg.get("text") or "").strip()
                if text:
                    # Emit words first so Codec's live-caption/barge-in path sees activity.
                    for word in text.split():
                        await self._send(ws, {"type": "word", "word": word})
                    await self._send(ws, {"type": "final", "text": text, "decodeMs": msg.get("decodeMs"), "duration": msg.get("duration")})
            elif typ == "error":
                await self._send(ws, {"type": "error", "message": str(msg.get("message") or msg)})

    async def _stderr_loop(self, ws: WebSocket) -> None:
        if not self.proc or not self.proc.stderr:
            return
        while line := await self.proc.stderr.readline():
            text = line.decode(errors="replace").strip()
            if text:
                await self._send(ws, {"type": "status", "message": text[-220:]})

    async def feed_float32(self, samples: np.ndarray) -> None:
        if not self.proc or not self.proc.stdin or self.proc.stdin.is_closing():
            return
        block16 = resample_linear(samples, self.input_rate, 16000)
        await self.feed_16k(block16)

    async def feed_16k(self, samples: np.ndarray) -> None:
        if not self.proc or not self.proc.stdin or self.proc.stdin.is_closing():
            return
        pcm = pcm16le(samples)
        self.proc.stdin.write(struct.pack("<I", len(pcm)) + pcm)
        await self.proc.stdin.drain()

    async def flush_silence(self) -> None:
        await self.feed_16k(np.zeros(int(16000 * max(0.9, self.min_silence_ms / 1000)), dtype=np.float32))

    async def close(self) -> None:
        if self.proc and self.proc.stdin and not self.proc.stdin.is_closing():
            self.proc.stdin.close()
            await self.proc.stdin.wait_closed()
        if self.proc:
            try:
                await asyncio.wait_for(self.proc.wait(), timeout=2)
            except asyncio.TimeoutError:
                self.proc.kill()


def make_app(args: argparse.Namespace) -> FastAPI:
    app = FastAPI(title="Codec Parakeet+Silero STT", version="0.1.0")

    @app.get("/health")
    def health() -> JSONResponse:
        return JSONResponse({"ok": True, "backend": "parakeet-silero", "worker": args.worker, "model": args.model, "vad": args.vad_model})

    @app.websocket("/")
    @app.websocket("/ws")
    async def ws_root(ws: WebSocket) -> None:
        await ws.accept()
        session = WorkerSession(args.worker, args.model, args.vad_model, args.input_rate, args.min_silence_ms)
        try:
            await session.start(ws)
            await asyncio.wait_for(session.ready.wait(), timeout=args.ready_timeout)
            while True:
                msg = await ws.receive()
                if "bytes" in msg:
                    await session.feed_float32(np.frombuffer(msg["bytes"], dtype=np.float32).copy())
                elif "text" in msg:
                    try:
                        obj = json.loads(msg["text"])
                    except Exception:
                        continue
                    typ = obj.get("type")
                    if typ == "stop":
                        await session.flush_silence()
                    elif typ == "getstatus":
                        await ws.send_text(json.dumps({"type": "status", "message": "parakeet-silero connected"}))
                    elif typ == "setlanguage":
                        await ws.send_text(json.dumps({"type": "status", "message": "parakeet-silero language auto"}))
        except WebSocketDisconnect:
            pass
        except RuntimeError as exc:
            if "disconnect message has been received" not in str(exc):
                try:
                    await ws.send_text(json.dumps({"type": "error", "message": str(exc)}))
                except RuntimeError:
                    pass
        except Exception as exc:
            try:
                await ws.send_text(json.dumps({"type": "error", "message": str(exc)}))
            except RuntimeError:
                pass
        finally:
            await session.close()

    return app


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=os.environ.get("HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.environ.get("PORT", "8796")))
    parser.add_argument("--worker", default=os.environ.get("PIBOT_PARAKEET_CPP_WORKER", DEFAULT_WORKER))
    parser.add_argument("--model", default=os.environ.get("PARAKEET_CPP_MODEL_PATH", DEFAULT_MODEL))
    parser.add_argument("--vad-model", default=os.environ.get("SILERO_VAD_GGML_MODEL_PATH", DEFAULT_VAD))
    parser.add_argument("--input-rate", type=int, default=24000)
    parser.add_argument("--min-silence-ms", type=int, default=int(os.environ.get("PARAKEET_MIN_SILENCE_MS", "800")))
    parser.add_argument("--ready-timeout", type=float, default=30.0)
    args = parser.parse_args()
    import uvicorn
    uvicorn.run(make_app(args), host=args.host, port=args.port)


if __name__ == "__main__":
    main()
