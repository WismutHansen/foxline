#!/usr/bin/env bun

import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

type Fixture = {
  character?: string;
  utterance: string;
  timeoutMs?: number;
};

const fixturePath = resolve(process.argv.find((arg) => arg.startsWith('--fixture='))?.split('=')[1] || 'benchmarks/fixtures/legacy-bridge-utterance.json');
const url = process.argv.find((arg) => arg.startsWith('--url='))?.split('=')[1] || process.env.CODEC_BRIDGE_WS_URL || 'ws://127.0.0.1:8770';
const sttUrl = argValue('--stt-url') || process.env.CODEC_STT_WS_URL || 'ws://127.0.0.1:8796/ws';
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8')) as Fixture;
const timeoutMs = Number(argValue('--timeout-ms') || fixture.timeoutMs || 60000);
const interruptOnAudio = process.argv.includes('--interrupt-on-audio');
const audioFile = argValue('--audio-file');
const rawPcmSampleRate = Number(argValue('--raw-pcm-sample-rate') || 24000);
const chunkMs = Number(argValue('--chunk-ms') || 80);
const tailSilenceMs = Number(argValue('--tail-silence-ms') || 1200);

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
    character: fixture.character,
    utteranceChars: fixture.utterance.length,
    timeoutMs,
    interruptOnAudio,
    mode: audio ? 'audio' : 'text',
    sttUrl: audio ? sttUrl : undefined,
    pcmBytes: audio?.bytes.byteLength,
    sampleRate: audio?.sampleRate,
    chunkMs,
    tailSilenceMs,
  }));
  process.exit(0);
}

const startedAt = Date.now();
const ws = new WebSocket(url);
let activeTurnId = '';
let completed = false;
let tracedPlayback = false;

const timer = setTimeout(() => {
  console.error(`legacy bridge fixture timed out after ${timeoutMs}ms`);
  ws.close();
  process.exit(1);
}, timeoutMs);

ws.addEventListener('open', () => {
  ws.send(JSON.stringify({
    type: 'client_trace',
    event: 'benchmark_fixture_start',
    data: { fixture: fixturePath, utteranceChars: fixture.utterance.length, mode: audio ? 'audio' : 'text' },
  }));
  if (fixture.character) ws.send(JSON.stringify({ type: 'switch_character', character: fixture.character }));
  if (audio) {
    void transcribeAudioFixture(audio, ws)
      .then((text) => {
        ws.send(JSON.stringify({ type: 'user_utterance', speaker: 'snake', text }));
      })
      .catch((error) => {
        console.error(`legacy audio fixture STT error: ${error instanceof Error ? error.message : String(error)}`);
        clearTimeout(timer);
        ws.close();
        process.exit(1);
      });
  } else {
    ws.send(JSON.stringify({ type: 'user_utterance', speaker: 'snake', text: fixture.utterance }));
  }
});

ws.addEventListener('message', (event) => {
  const message = JSON.parse(String(event.data));
  if (message.type === 'turn_started') activeTurnId = message.turnId;
  if ((message.type === 'audio_pcm' || message.type === 'audio_chunk') && !tracedPlayback) {
    tracedPlayback = true;
    ws.send(JSON.stringify({
      type: 'client_trace',
      event: 'pcm_scheduled',
      data: { turnId: message.turnId, index: message.index, sampleRate: message.sample_rate },
    }));
    if (interruptOnAudio) {
      ws.send(JSON.stringify({ type: 'interrupt' }));
      setTimeout(() => {
        completed = true;
        ws.send(JSON.stringify({
          type: 'client_trace',
          event: 'benchmark_fixture_done',
          data: { elapsedMs: Date.now() - startedAt, turnId: message.turnId, interrupted: true },
        }));
        clearTimeout(timer);
        ws.close();
      }, 500);
    }
  }
  if (message.type === 'error') {
    console.error(`legacy bridge fixture error: ${message.message}`);
    clearTimeout(timer);
    ws.close();
    process.exit(1);
  }
  if (!interruptOnAudio && message.type === 'turn_completed' && (!activeTurnId || message.turnId === activeTurnId)) {
    completed = true;
    ws.send(JSON.stringify({
      type: 'client_trace',
      event: 'benchmark_fixture_done',
      data: { elapsedMs: Date.now() - startedAt, turnId: message.turnId },
    }));
    clearTimeout(timer);
    ws.close();
  }
});

ws.addEventListener('close', () => {
  if (!completed) {
    console.error('legacy bridge fixture connection closed before turn completion');
    process.exit(1);
  }
  console.log(JSON.stringify({
    ok: true,
    url,
    fixture: fixturePath,
    elapsedMs: Date.now() - startedAt,
    traceDir: process.env.CODEC_LEGACY_BENCHMARK_TRACE_DIR || 'benchmarks/traces/legacy-bridge',
  }));
});

ws.addEventListener('error', () => {
  console.error(`could not connect to legacy bridge at ${url}`);
  clearTimeout(timer);
  process.exit(1);
});

async function transcribeAudioFixture(audio: AudioFixture, bridge: WebSocket) {
  return await new Promise<string>((resolveText, rejectText) => {
    const stt = new WebSocket(sttUrl);
    const words: string[] = [];
    let finalText = '';
    let settled = false;

    const fail = (error: Error) => {
      if (settled) return;
      settled = true;
      try { stt.close(); } catch {}
      rejectText(error);
    };
    const succeed = (text: string) => {
      if (settled) return;
      settled = true;
      try { stt.close(); } catch {}
      resolveText(text);
    };

    stt.addEventListener('open', () => {
      stt.send(JSON.stringify({ type: 'setlanguage', lang: 'en' }));
      stt.send(JSON.stringify({ type: 'getstatus' }));
      void streamAudioToStt(stt, bridge, audio).catch(fail);
    });
    stt.addEventListener('message', (event) => {
      if (typeof event.data !== 'string') return;
      const message = JSON.parse(event.data);
      if (message.type === 'word' && typeof message.word === 'string') {
        words.push(message.word);
        bridge.send(JSON.stringify({
          type: 'client_trace',
          event: 'stt_word',
          data: { word: message.word, words: words.length },
        }));
      }
      if ((message.type === 'interim' || message.type === 'final') && typeof message.text === 'string' && message.text.trim()) {
        if (message.type === 'final') {
          finalText = message.text.trim();
          succeed(finalText);
        }
      }
      if (message.type === 'error') {
        fail(new Error(String(message.message || 'STT error')));
      }
    });
    stt.addEventListener('close', () => {
      if (settled) return;
      const fallback = finalText || words.join(' ').trim();
      if (fallback) succeed(fallback);
      else fail(new Error('STT connection closed before final transcript'));
    });
    stt.addEventListener('error', () => fail(new Error(`could not connect to STT service at ${sttUrl}`)));
  });
}

async function streamAudioToStt(stt: WebSocket, bridge: WebSocket, audio: AudioFixture) {
  const bytesPerSample = 2;
  const samplesPerChunk = Math.max(1, Math.round(audio.sampleRate * (chunkMs / 1000)));
  const bytesPerChunk = samplesPerChunk * bytesPerSample;
  let chunksSent = 0;
  for (let offset = 0; offset < audio.bytes.byteLength; offset += bytesPerChunk) {
    const chunk = audio.bytes.subarray(offset, Math.min(audio.bytes.byteLength, offset + bytesPerChunk));
    stt.send(pcm16leToFloat32Buffer(chunk));
    chunksSent++;
    bridge.send(JSON.stringify({
      type: 'client_trace',
      event: 'stt_audio_frame_sent',
      data: { chunksSent, samples: Math.floor(chunk.byteLength / bytesPerSample), mode: 'fixture' },
    }));
    await Bun.sleep(Math.max(1, chunkMs));
  }
  const silenceChunks = Math.ceil(tailSilenceMs / chunkMs);
  const silence = new Uint8Array(bytesPerChunk);
  for (let i = 0; i < silenceChunks; i++) {
    stt.send(pcm16leToFloat32Buffer(silence));
    chunksSent++;
    bridge.send(JSON.stringify({
      type: 'client_trace',
      event: 'stt_audio_frame_sent',
      data: { chunksSent, samples: Math.floor(silence.byteLength / bytesPerSample), mode: 'fixture_silence' },
    }));
    await Bun.sleep(Math.max(1, chunkMs));
  }
  stt.send(JSON.stringify({ type: 'stop' }));
}

function argValue(name: string) {
  return process.argv.find((arg) => arg.startsWith(`${name}=`))?.slice(name.length + 1);
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

function pcm16leToFloat32Buffer(pcm: Uint8Array) {
  if (pcm.byteLength % 2 !== 0) throw new Error('PCM16 input must have an even byte length');
  const out = new ArrayBuffer((pcm.byteLength / 2) * 4);
  const view = new DataView(out);
  for (let source = 0, target = 0; source < pcm.byteLength; source += 2, target += 4) {
    const sample = (pcm[source] | (pcm[source + 1] << 8));
    const signed = sample & 0x8000 ? sample - 0x10000 : sample;
    view.setFloat32(target, signed / 32768.0, true);
  }
  return out;
}
