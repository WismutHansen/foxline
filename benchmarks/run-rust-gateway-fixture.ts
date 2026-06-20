#!/usr/bin/env bun

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { resolve } from 'node:path';

type Fixture = {
  character?: string;
  utterance: string;
  timeoutMs?: number;
};

const fixturePath = resolve(argValue('--fixture') || 'benchmarks/fixtures/legacy-bridge-utterance.json');
const url = argValue('--url') || process.env.FOXLINE_GATEWAY_WS_URL || 'ws://127.0.0.1:8780';
const traceDir = resolve(argValue('--trace-dir') || process.env.FOXLINE_GATEWAY_TRACE_DIR || 'benchmarks/traces/rust-gateway');
const workspace = argValue('--workspace') || process.env.FOXLINE_GATEWAY_WORKSPACE || '.';
const loadout = argValue('--loadout') || process.env.FOXLINE_GATEWAY_LOADOUT || 'default';
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8')) as Fixture;
const timeoutMs = Number(argValue('--timeout-ms') || fixture.timeoutMs || 60000);
const pcmBytes = Number(argValue('--pcm-bytes') || 4096);
const character = argValue('--agent') || fixture.character || 'campbell';
const audioFile = argValue('--audio-file');
const rawPcmSampleRate = Number(argValue('--raw-pcm-sample-rate') || 24000);
const chunkMs = Number(argValue('--chunk-ms') || 80);
const tailSilenceMs = Number(argValue('--tail-silence-ms') || 1200);
const waitForAudio = process.argv.includes('--wait-for-audio') || Boolean(audioFile);
const waitForAssistant = process.argv.includes('--wait-for-assistant') || Boolean(audioFile);
const endAfterAudio = process.argv.includes('--end-after-audio') || Boolean(audioFile);

let audio: AudioFixture | undefined;
try {
  audio = audioFile ? loadAudioFixture(resolve(audioFile), rawPcmSampleRate) : undefined;
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(2);
}

if (process.argv.includes('--dry-run')) {
  console.log(JSON.stringify({
    ok: true,
    dryRun: true,
    url,
    fixture: fixturePath,
    character,
    workspace,
    loadout,
    traceDir,
    timeoutMs,
    mode: audio ? 'audio' : 'protocol',
    pcmBytes: audio ? audio.bytes.byteLength : pcmBytes,
    sampleRate: audio?.sampleRate ?? 24000,
    chunkMs,
    tailSilenceMs,
    waitForAssistant,
    waitForAudio,
  }));
  process.exit(0);
}

const before = newestTrace(traceDir);
const startedAt = Date.now();
const ws = new WebSocket(url);
let completed = false;
let sessionStarted = false;
let assistantDelta = false;
let outputAudioBytes = 0;
let turnCompleted = false;

const timer = setTimeout(() => {
  console.error(`rust gateway fixture timed out after ${timeoutMs}ms; ${statusText()}`);
  ws.close();
  process.exit(1);
}, timeoutMs);

ws.addEventListener('open', () => {
  ws.send(JSON.stringify({
    type: 'hello',
    client: 'foxline-benchmark',
    debug_traces: true,
    capabilities: {
      protocol_version: 1,
      audio: { input_pcm: true, output_pcm: true, sample_rates_hz: [audio?.sampleRate ?? 24000] },
      tools: ['codec.display', 'codec.avatar'],
      avatar_actions: ['set_state', 'set_expression', 'focus', 'play_animation', 'clear'],
    },
  }));
  ws.send(JSON.stringify({ type: 'start_session', agent: character, workspace, loadout }));
});

ws.addEventListener('message', (event) => {
  if (typeof event.data !== 'string') {
    const bytes = binaryLength(event.data);
    outputAudioBytes += bytes;
    if (endAfterAudio && canComplete()) endSession();
    return;
  }
  const message = JSON.parse(event.data);
  if (message.type === 'session_started') {
    sessionStarted = true;
    if (audio) {
      void streamAudio(ws, audio, chunkMs);
    } else {
      ws.send(silencePcm16(pcmBytes));
      ws.send(JSON.stringify({ type: 'interrupt' }));
      endSession();
    }
  }
  if (message.type === 'assistant_delta') {
    assistantDelta = true;
  }
  if (message.type === 'turn_completed') {
    turnCompleted = true;
    if (canComplete()) endSession();
  }
  if (message.type === 'session_ended') {
    completed = true;
    clearTimeout(timer);
    ws.close();
  }
  if (message.type === 'error') {
    console.error(`rust gateway fixture error: ${message.code}: ${message.message}`);
    clearTimeout(timer);
    ws.close();
    process.exit(1);
  }
});

ws.addEventListener('close', () => {
  if (!completed) {
    console.error('rust gateway fixture connection closed before session end');
    process.exit(1);
  }
  const after = newestTrace(traceDir);
  console.log(JSON.stringify({
    ok: true,
    url,
    fixture: fixturePath,
    elapsedMs: Date.now() - startedAt,
    traceDir,
    traceFile: after && after !== before ? after : after || null,
    mode: audio ? 'audio' : 'protocol',
    assistantDelta,
    outputAudioBytes,
    turnCompleted,
  }));
});

ws.addEventListener('error', () => {
  console.error(`could not connect to Rust Voice Gateway at ${url}`);
  clearTimeout(timer);
  process.exit(1);
});

function argValue(name: string) {
  return process.argv.find((arg) => arg.startsWith(`${name}=`))?.slice(name.length + 1);
}

function silencePcm16(bytes: number) {
  return new Uint8Array(Math.max(2, bytes + (bytes % 2))).buffer;
}

async function streamAudio(ws: WebSocket, audio: AudioFixture, chunkDurationMs: number) {
  const bytesPerSample = 2;
  const samplesPerChunk = Math.max(1, Math.round(audio.sampleRate * (chunkDurationMs / 1000)));
  const bytesPerChunk = samplesPerChunk * bytesPerSample;
  for (let offset = 0; offset < audio.bytes.byteLength; offset += bytesPerChunk) {
    ws.send(audio.bytes.buffer.slice(audio.bytes.byteOffset + offset, audio.bytes.byteOffset + Math.min(audio.bytes.byteLength, offset + bytesPerChunk)));
    await Bun.sleep(Math.max(1, chunkDurationMs));
  }
  const silenceChunks = Math.ceil(tailSilenceMs / chunkDurationMs);
  const silence = new Uint8Array(bytesPerChunk);
  for (let i = 0; i < silenceChunks; i++) {
    ws.send(silence.buffer.slice(0));
    await Bun.sleep(Math.max(1, chunkDurationMs));
  }
  ws.send(JSON.stringify({ type: 'vad_hint', speaking: false, confidence: 0 }));
}

function canComplete() {
  return (!audio || turnCompleted) && (!waitForAssistant || assistantDelta) && (!waitForAudio || outputAudioBytes > 0);
}

function endSession() {
  if (completed || ws.readyState !== WebSocket.OPEN) return;
  ws.send(JSON.stringify({ type: 'end_session' }));
}

function statusText() {
  return `sessionStarted=${sessionStarted} assistantDelta=${assistantDelta} outputAudioBytes=${outputAudioBytes} turnCompleted=${turnCompleted}`;
}

function binaryLength(data: unknown) {
  if (data instanceof ArrayBuffer) return data.byteLength;
  if (ArrayBuffer.isView(data)) return data.byteLength;
  if (data instanceof Blob) return data.size;
  return 0;
}

type AudioFixture = {
  bytes: Uint8Array;
  sampleRate: number;
};

function loadAudioFixture(path: string, fallbackSampleRate: number): AudioFixture {
  if (!existsSync(path)) throw new Error(`audio fixture not found: ${path}`);
  const data = readFileSync(path);
  if (path.toLowerCase().endsWith('.wav')) return readPcm16MonoWav(data, path);
  if (path.toLowerCase().endsWith('.pcm') || path.toLowerCase().endsWith('.s16le')) {
    if (data.byteLength % 2 !== 0) throw new Error(`raw PCM fixture must have an even byte length: ${path}`);
    return { bytes: new Uint8Array(data), sampleRate: fallbackSampleRate };
  }
  throw new Error(`unsupported audio fixture format: ${path}; use .wav, .pcm, or .s16le`);
}

function readPcm16MonoWav(data: Buffer, path: string): AudioFixture {
  if (data.toString('ascii', 0, 4) !== 'RIFF' || data.toString('ascii', 8, 12) !== 'WAVE') {
    throw new Error(`not a RIFF/WAVE file: ${path}`);
  }
  let offset = 12;
  let channels = 0;
  let sampleRate = 0;
  let bitsPerSample = 0;
  let audioFormat = 0;
  let pcm: Uint8Array | undefined;
  while (offset + 8 <= data.byteLength) {
    const id = data.toString('ascii', offset, offset + 4);
    const size = data.readUInt32LE(offset + 4);
    const start = offset + 8;
    const end = start + size;
    if (end > data.byteLength) throw new Error(`truncated WAV chunk ${id} in ${path}`);
    if (id === 'fmt ') {
      audioFormat = data.readUInt16LE(start);
      channels = data.readUInt16LE(start + 2);
      sampleRate = data.readUInt32LE(start + 4);
      bitsPerSample = data.readUInt16LE(start + 14);
    } else if (id === 'data') {
      pcm = new Uint8Array(data.subarray(start, end));
    }
    offset = end + (size % 2);
  }
  if (audioFormat !== 1 || channels !== 1 || bitsPerSample !== 16) {
    throw new Error(`unsupported WAV format for ${path}; expected PCM16 mono, got format=${audioFormat} channels=${channels} bits=${bitsPerSample}`);
  }
  if (!pcm) throw new Error(`WAV file has no data chunk: ${path}`);
  return { bytes: pcm, sampleRate };
}

function newestTrace(dir: string) {
  try {
    return readdirSync(dir)
      .filter((name) => name.endsWith('.jsonl'))
      .map((name) => resolve(dir, name))
      .sort((a, b) => statSync(a).mtimeMs - statSync(b).mtimeMs)
      .at(-1);
  } catch {
    return undefined;
  }
}
