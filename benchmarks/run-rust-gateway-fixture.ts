#!/usr/bin/env bun

import { readFileSync, readdirSync, statSync } from 'node:fs';
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

if (process.argv.includes('--dry-run')) {
  console.log(JSON.stringify({ ok: true, dryRun: true, url, fixture: fixturePath, character, workspace, loadout, traceDir, timeoutMs, pcmBytes }));
  process.exit(0);
}

const before = newestTrace(traceDir);
const startedAt = Date.now();
const ws = new WebSocket(url);
let completed = false;

const timer = setTimeout(() => {
  console.error(`rust gateway fixture timed out after ${timeoutMs}ms`);
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
      audio: { input_pcm: true, output_pcm: true, sample_rates_hz: [24000] },
      tools: ['codec.display', 'codec.avatar'],
      avatar_actions: ['set_state', 'set_expression', 'focus', 'play_animation', 'clear'],
    },
  }));
  ws.send(JSON.stringify({ type: 'start_session', agent: character, workspace, loadout }));
});

ws.addEventListener('message', (event) => {
  if (typeof event.data !== 'string') return;
  const message = JSON.parse(event.data);
  if (message.type === 'session_started') {
    ws.send(silencePcm16(pcmBytes));
    ws.send(JSON.stringify({ type: 'interrupt' }));
    ws.send(JSON.stringify({ type: 'end_session' }));
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
