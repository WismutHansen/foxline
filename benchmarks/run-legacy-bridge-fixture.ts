#!/usr/bin/env bun

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

type Fixture = {
  character?: string;
  utterance: string;
  timeoutMs?: number;
};

const fixturePath = resolve(process.argv.find((arg) => arg.startsWith('--fixture='))?.split('=')[1] || 'benchmarks/fixtures/legacy-bridge-utterance.json');
const url = process.argv.find((arg) => arg.startsWith('--url='))?.split('=')[1] || process.env.CODEC_BRIDGE_WS_URL || 'ws://127.0.0.1:8770';
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8')) as Fixture;
const timeoutMs = Number(process.argv.find((arg) => arg.startsWith('--timeout-ms='))?.split('=')[1] || fixture.timeoutMs || 60000);

if (process.argv.includes('--dry-run')) {
  console.log(JSON.stringify({
    ok: true,
    dryRun: true,
    url,
    fixture: fixturePath,
    character: fixture.character,
    utteranceChars: fixture.utterance.length,
    timeoutMs,
  }));
  process.exit(0);
}

const startedAt = Date.now();
const ws = new WebSocket(url);
let activeTurnId = '';
let completed = false;

const timer = setTimeout(() => {
  console.error(`legacy bridge fixture timed out after ${timeoutMs}ms`);
  ws.close();
  process.exit(1);
}, timeoutMs);

ws.addEventListener('open', () => {
  ws.send(JSON.stringify({
    type: 'client_trace',
    event: 'benchmark_fixture_start',
    data: { fixture: fixturePath, utteranceChars: fixture.utterance.length },
  }));
  if (fixture.character) ws.send(JSON.stringify({ type: 'switch_character', character: fixture.character }));
  ws.send(JSON.stringify({ type: 'user_utterance', speaker: 'snake', text: fixture.utterance }));
});

ws.addEventListener('message', (event) => {
  const message = JSON.parse(String(event.data));
  if (message.type === 'turn_started') activeTurnId = message.turnId;
  if (message.type === 'error') {
    console.error(`legacy bridge fixture error: ${message.message}`);
    clearTimeout(timer);
    ws.close();
    process.exit(1);
  }
  if (message.type === 'turn_completed' && (!activeTurnId || message.turnId === activeTurnId)) {
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
