#!/usr/bin/env bun

import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';
import { spawn } from 'node:child_process';

type TraceRecord = {
  event: string;
  t_ms: number;
  [key: string]: unknown;
};

type MetricName = 'stt_final' | 'brain_first_token' | 'tts_audio_start' | 'frontend_audio_play_scheduled' | 'barge_in_cancel';

const thresholdMs = Number(argValue('--threshold-ms') || 50);
const legacyDir = resolve(argValue('--legacy-dir') || 'benchmarks/traces/legacy-bridge');
const rustDir = resolve(argValue('--rust-dir') || 'benchmarks/traces/rust-gateway');
const summaryPath = resolve(argValue('--json-out') || `benchmarks/traces/latency-summary-${stamp()}.json`);

if (process.argv.includes('--dry-run')) {
  console.log(JSON.stringify({
    ok: true,
    dryRun: true,
    thresholdMs,
    legacyTrace: argValue('--legacy-trace') || latestJsonl(legacyDir) || null,
    rustTrace: argValue('--rust-trace') || latestJsonl(rustDir) || null,
    summaryPath,
  }));
  process.exit(0);
}

await runOptionalCommand('--legacy-command');
await runOptionalCommand('--rust-command');

const legacyTrace = resolveTrace('--legacy-trace', legacyDir, 'legacy');
const rustTrace = resolveTrace('--rust-trace', rustDir, 'rust');
const legacyRecords = readTrace(legacyTrace);
const rustRecords = readTrace(rustTrace);
const legacyMetrics = collectMetrics(legacyRecords);
const rustMetrics = collectMetrics(rustRecords);
const comparisons = compareMetrics(legacyMetrics, rustMetrics);
const ok = comparisons.every((metric) => metric.status === 'pass');

mkdirSync(resolve('benchmarks/traces'), { recursive: true });
writeFileSync(summaryPath, `${JSON.stringify({
  ok,
  thresholdMs,
  legacyTrace,
  rustTrace,
  generatedAt: new Date().toISOString(),
  metrics: comparisons,
}, null, 2)}\n`);

printSummary(comparisons, legacyTrace, rustTrace);
console.log(`JSON summary: ${summaryPath}`);
process.exit(ok ? 0 : 1);

function compareMetrics(legacy: Map<MetricName, number>, rust: Map<MetricName, number>) {
  const names: MetricName[] = ['stt_final', 'brain_first_token', 'tts_audio_start', 'frontend_audio_play_scheduled', 'barge_in_cancel'];
  return names.map((name) => {
    const legacyMs = legacy.get(name);
    const rustMs = rust.get(name);
    const deltaMs = legacyMs === undefined || rustMs === undefined ? undefined : rustMs - legacyMs;
    const status = deltaMs === undefined ? 'missing' : deltaMs <= thresholdMs ? 'pass' : 'fail';
    return { name, legacyMs, rustMs, deltaMs, thresholdMs, status };
  });
}

function collectMetrics(records: TraceRecord[]) {
  const origin = firstTime(records, ['benchmark_fixture_start']) ?? records[0]?.t_ms ?? 0;
  const out = new Map<MetricName, number>();
  setElapsed(out, 'stt_final', records, origin);
  setElapsed(out, 'brain_first_token', records, origin);
  setElapsed(out, 'tts_audio_start', records, origin);
  setElapsed(out, 'frontend_audio_play_scheduled', records, origin);
  const bargeIn = firstTime(records, ['barge_in_received']);
  const cancel = firstTime(records, ['tts_cancel_sent']);
  if (bargeIn !== undefined && cancel !== undefined) out.set('barge_in_cancel', cancel - bargeIn);
  return out;
}

function setElapsed(out: Map<MetricName, number>, name: MetricName, records: TraceRecord[], origin: number) {
  const t = firstTime(records, [name]);
  if (t !== undefined) out.set(name, t - origin);
}

function firstTime(records: TraceRecord[], events: string[]) {
  return records.find((record) => events.includes(record.event))?.t_ms;
}

function readTrace(path: string) {
  return readFileSync(path, 'utf8')
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line, index) => {
      const parsed = JSON.parse(line) as Record<string, unknown>;
      if (typeof parsed.event !== 'string' || typeof parsed.t_ms !== 'number') {
        throw new Error(`${path}:${index + 1} is missing event or numeric t_ms`);
      }
      return parsed as TraceRecord;
    });
}

function resolveTrace(argName: string, dir: string, label: string) {
  const explicit = argValue(argName);
  if (explicit) return resolve(explicit);
  const latest = latestJsonl(dir);
  if (!latest) throw new Error(`No ${label} trace found in ${dir}; pass ${argName}=path or run a fixture command first`);
  return latest;
}

function latestJsonl(dir: string) {
  if (!existsSync(dir)) return undefined;
  const files = readdirSync(dir)
    .filter((name) => name.endsWith('.jsonl'))
    .map((name) => resolve(dir, name))
    .sort((a, b) => statSync(a).mtimeMs - statSync(b).mtimeMs);
  return files.at(-1);
}

async function runOptionalCommand(argName: string) {
  const command = argValue(argName);
  if (!command) return;
  const proc = spawn(command, { shell: true, stdio: 'inherit', env: process.env });
  const code = await new Promise<number | null>((resolveCode) => proc.on('close', resolveCode));
  if (code !== 0) throw new Error(`${argName} exited with ${code}`);
}

function printSummary(rows: ReturnType<typeof compareMetrics>, legacyTrace: string, rustTrace: string) {
  console.log(`Legacy trace: ${basename(legacyTrace)}`);
  console.log(`Rust trace:   ${basename(rustTrace)}`);
  console.log(`Threshold:    +${thresholdMs}ms`);
  console.log('');
  console.log('metric                         legacy   rust   delta   status');
  for (const row of rows) {
    console.log(`${row.name.padEnd(30)} ${fmt(row.legacyMs).padStart(6)} ${fmt(row.rustMs).padStart(6)} ${fmt(row.deltaMs).padStart(7)} ${row.status}`);
  }
}

function fmt(value: number | undefined) {
  return value === undefined ? 'missing' : `${Math.round(value)}ms`;
}

function argValue(name: string) {
  return process.argv.find((arg) => arg.startsWith(`${name}=`))?.slice(name.length + 1);
}

function stamp() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}
