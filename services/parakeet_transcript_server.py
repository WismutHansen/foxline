#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "fastapi>=0.115",
#   "uvicorn[standard]>=0.30",
#   "python-multipart>=0.0.9",
#   "huggingface-hub>=0.23",
# ]
# ///
"""Small local Parakeet STT server for Codec.

Phase 1 uses browser-side VAD to send finalized utterance WAVs here. The server
runs parakeet-cli over the WAV and returns JSON text. Phase 2 can replace the
subprocess path with libparakeet streaming EOU.
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
import shutil
import socket
import tempfile
import threading
import time
import wave
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, File, HTTPException, UploadFile
from fastapi.middleware.cors import CORSMiddleware
from huggingface_hub import hf_hub_download

DEFAULT_REPO = os.environ.get("PARAKEET_MODEL_REPO", "mudler/parakeet-cpp-gguf")
DEFAULT_MODEL_FILE = os.environ.get("PARAKEET_MODEL_FILE", "realtime_eou_120m-v1-q8_0.gguf")
DEFAULT_LIB = os.environ.get("PARAKEET_LIB", "/tmp/parakeet.cpp/build-shared/libparakeet.dylib")

_MODEL_PATH: str | None = None
_BACKEND: "ParakeetBackend | None" = None


def model_path() -> str:
    global _MODEL_PATH
    if _MODEL_PATH and Path(_MODEL_PATH).exists():
        return _MODEL_PATH
    explicit = os.environ.get("PARAKEET_MODEL")
    if explicit:
        _MODEL_PATH = explicit
    else:
        _MODEL_PATH = hf_hub_download(DEFAULT_REPO, DEFAULT_MODEL_FILE)
    return _MODEL_PATH


class ParakeetBackend:
    def __init__(self, lib_path: str, gguf_path: str) -> None:
        if not Path(lib_path).exists():
            raise RuntimeError(f"libparakeet not found: {lib_path}")
        self.lib = ctypes.CDLL(lib_path)
        self.lib.parakeet_capi_load.argtypes = [ctypes.c_char_p]
        self.lib.parakeet_capi_load.restype = ctypes.c_void_p
        self.lib.parakeet_capi_free.argtypes = [ctypes.c_void_p]
        self.lib.parakeet_capi_transcribe_path_json.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        self.lib.parakeet_capi_transcribe_path_json.restype = ctypes.c_void_p
        self.lib.parakeet_capi_free_string.argtypes = [ctypes.c_void_p]
        self.lib.parakeet_capi_last_error.argtypes = [ctypes.c_void_p]
        self.lib.parakeet_capi_last_error.restype = ctypes.c_char_p
        self.ctx = self.lib.parakeet_capi_load(str(gguf_path).encode())
        if not self.ctx:
            raise RuntimeError("parakeet_capi_load failed")
        self.lock = threading.Lock()

    def last_error(self) -> str:
        raw = self.lib.parakeet_capi_last_error(self.ctx)
        return raw.decode(errors="replace") if raw else ""

    def transcribe_path_json(self, wav: Path) -> dict[str, object]:
        with self.lock:
            ptr = self.lib.parakeet_capi_transcribe_path_json(self.ctx, str(wav).encode(), 0)
            if not ptr:
                raise RuntimeError(self.last_error() or "parakeet transcription failed")
            try:
                raw = ctypes.string_at(ptr).decode("utf-8", errors="replace")
            finally:
                self.lib.parakeet_capi_free_string(ptr)
        return json.loads(raw)


def backend() -> ParakeetBackend:
    global _BACKEND
    if _BACKEND is None:
        _BACKEND = ParakeetBackend(DEFAULT_LIB, model_path())
    return _BACKEND


def wav_stats(path: Path) -> dict[str, object]:
    try:
        with wave.open(str(path), "rb") as w:
            frames = w.getnframes()
            rate = w.getframerate()
            channels = w.getnchannels()
            width = w.getsampwidth()
            raw = w.readframes(frames)
        if width == 2 and raw:
            total = 0.0
            peak_i = 0
            count = len(raw) // 2
            for i in range(0, len(raw), 2):
                sample = int.from_bytes(raw[i:i + 2], "little", signed=True)
                total += sample * sample
                peak_i = max(peak_i, abs(sample))
            rms = ((total / count) ** 0.5) / 32768 if count else 0.0
            peak = peak_i / 32768
        else:
            rms = 0.0
            peak = 0.0
        return {"seconds": frames / rate if rate else 0.0, "sample_rate": rate, "channels": channels, "sample_width": width, "rms": rms, "peak": peak, "bytes": path.stat().st_size}
    except Exception as exc:
        return {"error": str(exc), "bytes": path.stat().st_size if path.exists() else 0}


@asynccontextmanager
async def lifespan(_app: FastAPI):
    if os.environ.get("PARAKEET_LAZY_LOAD") != "1":
        backend()
    yield


app = FastAPI(title="Codec Parakeet STT", version="0.1.0", lifespan=lifespan)
app.add_middleware(
    CORSMiddleware,
    allow_origins=os.environ.get("PARAKEET_CORS_ORIGINS", "http://localhost:5173,http://127.0.0.1:5173,http://localhost:5174,http://127.0.0.1:5174").split(","),
    allow_credentials=False,
    allow_methods=["GET", "POST", "OPTIONS"],
    allow_headers=["*"],
)


@app.get("/health")
def health() -> dict[str, object]:
    if os.environ.get("PARAKEET_LAZY_LOAD") != "1":
        backend()
    return {"ok": True, "lib": DEFAULT_LIB, "model": model_path(), "loaded": _BACKEND is not None}


@app.post("/transcribe")
async def transcribe(audio: UploadFile = File(...)) -> dict[str, object]:
    data = await audio.read()
    if not data:
        raise HTTPException(status_code=400, detail="empty audio")
    with tempfile.TemporaryDirectory(prefix="codec-parakeet-") as tmp:
        wav = Path(tmp) / "utterance.wav"
        wav.write_bytes(data)
        stats = wav_stats(wav)
        dump_dir = os.environ.get("PARAKEET_DUMP_DIR")
        dump_path = None
        if dump_dir:
            out_dir = Path(dump_dir).expanduser()
            out_dir.mkdir(parents=True, exist_ok=True)
            dump_path = out_dir / f"utterance-{int(time.time() * 1000)}.wav"
            shutil.copyfile(wav, dump_path)
        started = time.perf_counter()
        try:
            parsed = backend().transcribe_path_json(wav)
        except Exception as exc:
            raise HTTPException(status_code=502, detail={"error": str(exc), "audio": stats, "dump_path": str(dump_path) if dump_path else None}) from exc
        elapsed = time.perf_counter() - started
        text = str(parsed.get("text") or "").replace("<EOU>", "").replace("<EOB>", "").strip()
        parsed["text"] = text
        print(f"[parakeet] transcribe elapsed={elapsed:.3f}s audio={stats} text={text!r} dump={dump_path}")
        return {"text": text, "seconds": elapsed, "audio": stats, "dump_path": str(dump_path) if dump_path else None, "raw": parsed}


def port_in_use(host: str, port: int) -> bool:
    probe_host = "127.0.0.1" if host in {"0.0.0.0", "::"} else host
    try:
        with socket.create_connection((probe_host, port), timeout=0.25):
            return True
    except OSError:
        return False


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=os.environ.get("HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.environ.get("PORT", "8780")))
    args = parser.parse_args()
    if port_in_use(args.host, args.port):
        print(f"Parakeet STT already appears to be running at http://{args.host}:{args.port}")
        print(f"Health: curl -s http://{args.host}:{args.port}/health")
        raise SystemExit(0)
    import uvicorn
    uvicorn.run(app, host=args.host, port=args.port)


if __name__ == "__main__":
    main()
