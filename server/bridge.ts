#!/usr/bin/env bun
import { spawn } from 'node:child_process';
import { StringDecoder } from 'node:string_decoder';
import { randomUUID } from 'node:crypto';
import { appendFileSync, existsSync, mkdirSync, readFileSync, readdirSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { once } from 'node:events';

function detectRepoRoot() {
  const fromEnv = process.env.CODEC_REPO_ROOT;
  if (fromEnv && existsSync(`${fromEnv}/agents/manifest.json`)) return fromEnv;

  const cwd = process.cwd();
  if (existsSync(`${cwd}/agents/manifest.json`)) return cwd;

  const fromImport = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  if (existsSync(`${fromImport}/agents/manifest.json`)) return fromImport;

  return fromImport;
}

const repo = detectRepoRoot();
const args = new Set(process.argv.slice(2));
const characterArg = process.argv.find((a) => a.startsWith('--character='));
let character = characterArg?.split('=')[1] || process.env.CODEC_CHARACTER || 'campbell';
let characterCwd = `${repo}/agents/${character}`;
const port = Number(process.env.CODEC_BRIDGE_PORT || 8770);
const ttsBase = process.env.CODEC_TTS_BASE_URL || process.env.VOXCPM2_BASE_URL || 'http://127.0.0.1:8089/v1';
const localQwenModel = `${process.env.HOME || ''}/models/qwen3-tts-mlx-4bit`;
const ttsModel = process.env.CODEC_TTS_MODEL || process.env.QWEN3_TTS_MODEL || (existsSync(localQwenModel) ? localQwenModel : 'mlx-community/Qwen3-TTS-12Hz-0.6B-Base-4bit');
const ttsMode = process.env.CODEC_TTS_MODE || 'worker';
const ttsHttpTimeoutMs = Number(process.env.CODEC_TTS_HTTP_TIMEOUT_MS || 12000);
const ttsWorkerPath = process.env.CODEC_TTS_WORKER_PATH || `${repo}/services/qwen3_tts_worker.py`;
const ttsWorkerSampleRate = Number(process.env.CODEC_TTS_WORKER_SAMPLE_RATE || 24000);
const minTtsChunkChars = Number(process.env.CODEC_TTS_MIN_CHARS || 28);
const maxTtsChunkChars = Number(process.env.CODEC_TTS_MAX_CHARS || 90);
const workerInputSpeak = 1;
const workerInputCancel = 2;
const workerInputShutdown = 3;
const workerOutputReady = 1;
const workerOutputAudioStart = 2;
const workerOutputAudioChunk = 3;
const workerOutputAudioDone = 4;
const workerOutputError = 5;
const frameHeaderBytes = 9;
const sessionStamp = new Date().toISOString().replace(/[:.]/g, '-');
const sessionDir = process.env.CODEC_SESSION_DIR || `${repo}/sessions/codec-${character}-${sessionStamp}`;
mkdirSync(sessionDir, { recursive: true });
const traceFile = process.env.CODEC_TRACE_FILE || `${sessionDir}/trace.jsonl`;
const legacyBenchmarkTraceDir = process.env.CODEC_LEGACY_BENCHMARK_TRACE_DIR || `${repo}/benchmarks/traces/legacy-bridge`;
mkdirSync(legacyBenchmarkTraceDir, { recursive: true });
const legacyBenchmarkTraceFile = process.env.CODEC_LEGACY_BENCHMARK_TRACE_FILE || `${legacyBenchmarkTraceDir}/${character}-${sessionStamp}.jsonl`;
const trajectoryFile = process.env.CODEC_TRAJECTORY_FILE || `${sessionDir}/trajectory.jsonl`;
function writeJsonl(path: string, record: Record<string, unknown>, label: string) {
  try { appendFileSync(path, `${JSON.stringify(record)}\n`); } catch (e) { console.error(`[${label}] write failed`, e); }
}
function trace(event: string, data: Record<string, unknown> = {}) {
  const record = { ts: new Date().toISOString(), t_ms: Date.now(), event, ...data };
  writeJsonl(traceFile, record, 'trace');
  writeJsonl(legacyBenchmarkTraceFile, record, 'legacy-benchmark-trace');
  for (const alias of benchmarkEventAliases(event, data)) {
    writeJsonl(legacyBenchmarkTraceFile, { ...record, event: alias.event, aliasOf: event, ...alias.data }, 'legacy-benchmark-trace');
  }
}
function trajectory(event: string, data: Record<string, unknown> = {}) {
  writeJsonl(trajectoryFile, { ts: new Date().toISOString(), t_ms: Date.now(), event, ...data }, 'trajectory');
}

function benchmarkEventAliases(event: string, data: Record<string, unknown>) {
  const aliases: Array<{ event: string; data?: Record<string, unknown> }> = [];
  if (event === 'tts_cancel') aliases.push({ event: 'tts_cancel_sent', data });
  if (event === 'client_audio_pcm_received' || event === 'client_audio_chunk_received') aliases.push({ event: 'frontend_audio_received', data });
  if (event === 'client_pcm_scheduled' || event === 'client_wav_play_start') aliases.push({ event: 'frontend_audio_play_scheduled', data });
  if (event === 'client_stt_word') aliases.push({ event: 'stt_partial', data });
  if (event === 'client_stt_audio_frame_sent') aliases.push({ event: 'mic_frame_received', data });
  return aliases;
}

function buildPiArgs() {
  const built = ['--mode', 'rpc'];
  if (args.has('--new')) {
    // default persistent new session
  } else if (args.has('--session')) {
    const i = process.argv.indexOf('--session');
    if (process.argv[i + 1]) built.push('--session', process.argv[i + 1]);
  } else {
    built.push('--continue');
  }

  if (piNoExtensions) built.push('--no-extensions');
  if (piNoContextFiles) built.push('--no-context-files');
  if (piSessionDir.trim()) {
    mkdirSync(piSessionDir, { recursive: true });
    built.push('--session-dir', piSessionDir);
  }
  if (piModel) built.push('--model', piModel);
  if (piTools) built.push('--tools', piTools);
  if (piThinking) built.push('--thinking', piThinking);

  const effectiveSystemPrompt = process.env.CODEC_PI_SYSTEM_PROMPT || (piUseDefaultPrompt ? buildDefaultPiSystemPrompt() : '');
  if (effectiveSystemPrompt.trim()) {
    built.push(piPromptMode === 'replace' ? '--system-prompt' : '--append-system-prompt', effectiveSystemPrompt);
  }

  return built;
}


type ClientMsg = { type: 'user_utterance'; text: string; speaker?: string; character?: string } | { type: 'interrupt' } | { type: 'new_session' } | { type: 'switch_character'; character: string } | { type: 'client_trace'; event: string; data?: Record<string, unknown> };
type RpcEvent = { type: string; assistantMessageEvent?: { type: string; delta?: string; content?: string; channel?: string; phase?: string; reasoning?: string; reasoning_content?: string }; data?: any; message?: any; toolCallId?: string; toolName?: string; command?: string; success?: boolean; error?: string };
type Filler = { agent?: string; category: 'tool_start' | 'tool_slow' | 'tool_done'; text: string; path: string };
type ChatRole = 'user' | 'assistant';
type ChatMessage = { role: ChatRole; text: string };

interface Brain {
  prompt(text: string): void;
  abort(): void;
  newSession(): void;
  onClientConnected?(): void;
  stop?(): void;
  info(): string;
}

function loadCharacterManifest() {
  const path = `${repo}/agents/manifest.json`;
  if (!existsSync(path)) return { characters: [] as Array<Record<string, unknown>> };
  try { return JSON.parse(readFileSync(path, 'utf8')); } catch { return { characters: [] as Array<Record<string, unknown>> }; }
}

function enabledCharacters() {
  return (loadCharacterManifest().characters || []).filter((c: any) => c.enabled !== false);
}

function setCharacter(next: string) {
  const allowed = enabledCharacters().map((c: any) => c.id);
  if (allowed.length && !allowed.includes(next)) throw new Error(`Unknown or disabled character: ${next}`);
  const nextCwd = `${repo}/agents/${next}`;
  if (!existsSync(`${nextCwd}/AGENTS.md`)) throw new Error(`Missing AGENTS.md for character: ${next}`);
  character = next;
  characterCwd = nextCwd;
}

const COMMON_ABBREVIATIONS = new Set([
  'mr', 'mrs', 'ms', 'dr', 'prof', 'sr', 'jr', 'st', 'vs', 'etc', 'e.g', 'i.e', 'a.m', 'p.m',
  'u.s', 'u.k', 'u.n', 'nato', 'dept', 'est', 'approx', 'fig', 'no', 'vol', 'sgt', 'lt', 'col', 'gen', 'cmdr',
]);

function loadFillers(): Filler[] {
  const manifest = `${characterCwd}/assets/filler/manifest.json`;
  if (!existsSync(manifest)) return [];
  try {
    const parsed = JSON.parse(readFileSync(manifest, 'utf8')) as Filler[];
    return parsed.filter((f) => f.path && existsSync(`${repo}/${f.path}`));
  } catch (e) {
    console.error('[bridge] failed to load filler manifest', manifest, e);
    return [];
  }
}

let fillers = loadFillers();
function pickFiller(category: Filler['category']) {
  const matches = fillers.filter((f) => f.category === category);
  if (!matches.length) return null;
  return matches[Math.floor(Math.random() * matches.length)];
}

function loadCharacterInstructions() {
  const path = `${characterCwd}/AGENTS.md`;
  if (!existsSync(path)) return '';
  try {
    return readFileSync(path, 'utf8').trim();
  } catch (e) {
    console.error('[bridge] failed to load character instructions', path, e);
    return '';
  }
}

function loadCharacterSystemPrompt() {
  const path = `${characterCwd}/SYSTEM.md`;
  if (!existsSync(path)) return '';
  try {
    return readFileSync(path, 'utf8').trim();
  } catch (e) {
    console.error('[bridge] failed to load character system prompt', path, e);
    return '';
  }
}

function buildDefaultPiSystemPrompt() {
  const systemPrompt = loadCharacterSystemPrompt();
  if (!systemPrompt) console.warn(`[bridge] SYSTEM.md missing for character=${character}; only voice output rules will be applied`);
  return [systemPrompt, voiceSessionPrompt].filter(Boolean).join('\n\n');
}

const voiceSessionPrompt = `You are currently connected through a real-time voice interface. Output must be directly speakable aloud.

Voice output rules:
- Do not output Markdown tables, pipe tables, code fences, headings, horizontal rules, block quotes, or decorative separators.
- Prefer short spoken sentences and compact lists in plain prose.
- For calendar, email, search, or tabular data, summarize the most important entries in words instead of formatting a table.
- Do not include emojis, bullets made from symbols, raw URLs, markup syntax, or bracketed UI labels unless the user explicitly asks for exact text.
- Never reveal hidden reasoning, thought traces, channel markers, tool protocol text, or implementation details. If a tool fails, state the exact visible error plainly.`;

const brainMode = process.env.CODEC_BRAIN_MODE || 'pi';
const piModel = process.env.CODEC_PI_MODEL || 'LM-Studio/gemma-4-26b-a4b-it';
const piTools = process.env.CODEC_PI_TOOLS || 'read,write,edit,bash';
const piThinking = process.env.CODEC_PI_THINKING || 'minimal';
const piNoExtensions = (process.env.CODEC_PI_NO_EXTENSIONS || '1') !== '0';
const piNoContextFiles = (process.env.CODEC_PI_NO_CONTEXT_FILES || '1') !== '0';
const piSessionDir = process.env.CODEC_PI_SESSION_DIR || '';
const piUseDefaultPrompt = (process.env.CODEC_PI_USE_DEFAULT_PROMPT || '1') !== '0';
const piPromptMode = process.env.CODEC_PI_PROMPT_MODE || 'append'; // append|replace
const piEnableFillers = (process.env.CODEC_PI_FILLERS || '0') === '1';

function sanitizeAssistantDelta(text: string) {
  return text
    .replace(/<\|channel\>\s*thought\s*/gi, ' ')
    .replace(/<channel\|>/gi, ' ')
    .replace(/<\/?thinking>/gi, ' ')
    .replace(/<\/?think>/gi, ' ')
    .replace(/\[\s*codec\s*frequency[^\]]*\]/gi, ' ')
    .replace(/\b(?:thought|analysis|reasoning)(?:\s*[:\-])?/gi, ' ');
}

function sanitizeAssistantText(text: string) {
  return sanitizeAssistantDelta(text)
    .replace(/\s+/g, ' ')
    .trim();
}

function isHiddenAssistantEvent(event: NonNullable<RpcEvent['assistantMessageEvent']>) {
  const hidden = new Set(['thought', 'thought_delta', 'thinking', 'thinking_delta', 'analysis', 'analysis_delta', 'reasoning', 'reasoning_delta']);
  const fields = [event.type, event.channel, event.phase].map((value) => String(value || '').trim().toLowerCase());
  if (fields.some((value) => hidden.has(value))) return true;
  const visible = `${event.delta || ''}${event.content || ''}`.trim();
  const reasoning = `${event.reasoning || ''}${event.reasoning_content || ''}`.trim();
  return !visible && Boolean(reasoning);
}

function stripMarkdownForTts(text: string) {
  return sanitizeAssistantText(text)
    .replace(/```[\s\S]*?```/g, ' code block omitted. ')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}#{1,6}\s+/gm, '')
    .replace(/^\s{0,3}>\s?/gm, '')
    .replace(/^\s*[-*+]\s+/gm, '')
    .replace(/^\s*\d+[.)]\s+/gm, '')
    .replace(/[*_~]{1,3}/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function extractJsonObject(text: string) {
  const fenced = text.match(/```(?:json)?\s*([\s\S]*?)```/i)?.[1];
  const candidate = (fenced || text).trim();
  const start = candidate.indexOf('{');
  const end = candidate.lastIndexOf('}');
  if (start < 0 || end < 0 || end <= start) throw new Error('No JSON object in model output');
  return JSON.parse(candidate.slice(start, end + 1));
}

function resolvePathSafe(path: string) {
  return resolve(path.startsWith('/') ? path : `${repo}/${path}`);
}

function countOccurrences(haystack: string, needle: string) {
  if (!needle) return 0;
  let count = 0;
  let at = 0;
  while (true) {
    const i = haystack.indexOf(needle, at);
    if (i < 0) break;
    count += 1;
    at = i + needle.length;
  }
  return count;
}

function shellQuote(value: string) {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

async function runBash(command: string, timeout = 120000) {
  const child = spawn('bash', ['-lc', command], { cwd: repo, stdio: ['ignore', 'pipe', 'pipe'] });
  const timer = setTimeout(() => child.kill('SIGTERM'), timeout);
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  child.stdout.on('data', (d) => stdout.push(Buffer.from(d)));
  child.stderr.on('data', (d) => stderr.push(Buffer.from(d)));
  const [code, signal] = await once(child, 'exit') as [number | null, NodeJS.Signals | null];
  clearTimeout(timer);
  return {
    exitCode: code ?? -1,
    signal,
    stdout: Buffer.concat(stdout).toString('utf8'),
    stderr: Buffer.concat(stderr).toString('utf8'),
  };
}

async function executeLocalTool(tool: string, args: any) {
  if (tool === 'read') {
    const path = resolvePathSafe(String(args?.path || ''));
    const offset = Math.max(1, Number(args?.offset || 1));
    const limit = Math.max(1, Number(args?.limit || 400));
    const text = await readFile(path, 'utf8');
    const lines = text.split(/\r?\n/);
    const slice = lines.slice(offset - 1, offset - 1 + limit).join('\n');
    return { ok: true, tool, path, offset, limit, content: slice };
  }
  if (tool === 'write') {
    const path = resolvePathSafe(String(args?.path || ''));
    const content = String(args?.content || '');
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content, 'utf8');
    return { ok: true, tool, path, bytes: Buffer.byteLength(content) };
  }
  if (tool === 'edit') {
    const path = resolvePathSafe(String(args?.path || ''));
    const edits = Array.isArray(args?.edits) ? args.edits : [];
    let text = await readFile(path, 'utf8');
    for (const e of edits) {
      const oldText = String(e?.oldText || '');
      const newText = String(e?.newText || '');
      const count = countOccurrences(text, oldText);
      if (count !== 1) throw new Error(`edit oldText must match exactly once (got ${count})`);
      text = text.replace(oldText, newText);
    }
    await writeFile(path, text, 'utf8');
    return { ok: true, tool, path, edits: edits.length };
  }
  if (tool === 'bash') {
    const command = String(args?.command || '');
    const timeout = Number(args?.timeout || 120000);
    return { ok: true, tool, ...(await runBash(command, timeout)) };
  }
  throw new Error(`Unsupported tool: ${tool}`);
}

function isLikelyBoundary(text: string, i: number) {
  const ch = text[i];
  const prev = text[i - 1] || '';
  const next = text[i + 1] || '';
  if (ch === '.' && /\d/.test(prev) && /\d/.test(next)) return false; // 140.85

  const before = text.slice(0, i).match(/([A-Za-z](?:\.[A-Za-z])+|[A-Za-z]+)$/)?.[1] || '';
  const normalized = before.toLowerCase().replace(/\.$/, '');
  if (ch === '.' && COMMON_ABBREVIATIONS.has(normalized)) return false;
  if (ch === '.' && /^[A-Z](?:\.[A-Z])+$/.test(before)) return false; // U.S. / C.I.A.
  if (ch === '.' && before.length === 1 && /[A-Z]/.test(before)) return false; // initial

  if ('!?'.includes(ch)) return true;
  if (ch === '.') return true;
  if (';:—'.includes(ch)) return true;
  if (ch === ',') return false;
  return false;
}

class SentenceBuffer {
  private text = '';
  constructor(private minChars = minTtsChunkChars) {}
  push(delta: string) {
    this.text += delta;
    const out: string[] = [];
    while (true) {
      let cut = -1;
      for (let i = 0; i < this.text.length; i++) {
        if (!isLikelyBoundary(this.text, i)) continue;
        const next = this.text[i + 1] || '';
        if (next && !/\s|["')\]]/.test(next)) continue;
        const candidate = this.text.slice(0, i + 1).trim();
        if (candidate.length >= this.minChars) { cut = i + 1; break; }
      }
      if (cut < 0 && this.text.length > maxTtsChunkChars) {
        const window = this.text.slice(0, maxTtsChunkChars);
        cut = Math.max(window.lastIndexOf(','), window.lastIndexOf(' '));
        if (cut < this.minChars) cut = maxTtsChunkChars;
      }
      if (cut < 0) break;
      const consumed = cut + (this.text[cut] === ',' ? 1 : 0);
      const sentence = this.text.slice(0, consumed).trim();
      this.text = this.text.slice(consumed).trimStart();
      if (sentence.length >= 8) out.push(sentence);
    }
    return out;
  }
  flush() {
    const t = this.text.trim();
    this.text = '';
    return t || null;
  }
  reset() { this.text = ''; }
}

function resolveVoiceReference() {
  const refDir = `${characterCwd}/assets/reference_audio`;
  const fallbackDir = `${characterCwd}/assets`;
  const searchDirs = [refDir, fallbackDir].filter((d) => existsSync(d));

  let wavPath = '';
  for (const dir of searchDirs) {
    const wav = readdirSync(dir).find((f) => f.toLowerCase().endsWith('.wav'));
    if (wav) {
      wavPath = `${dir}/${wav}`;
      break;
    }
  }

  if (!wavPath) throw new Error(`No reference wav found in ${characterCwd}/assets/reference_audio or ${characterCwd}/assets`);

  const txtPath = existsSync(`${wavPath}.txt`) ? `${wavPath}.txt` : `${wavPath.replace(/\.wav$/i, '.txt')}`;
  if (!existsSync(txtPath)) throw new Error(`No reference transcript found for ${wavPath}`);
  return { wavPath, txtPath };
}

class QwenTtsWorker {
  private proc;
  private stdoutBuffer = Buffer.alloc(0);
  private nextId = 1;
  private active = new Map<number, { turnId: string; index: number; text: string; sampleRate: number; startedAt: number; firstAudioAt?: number; chunks: number; bytes: number; sawAudio: boolean; pendingSilence: Buffer[]; done: () => void; error: (e: Error) => void }>();
  ready: Promise<void>;
  private resolveReady?: () => void;
  private rejectReady?: (e: Error) => void;
  private intentionalExit = false;

  constructor() {
    const voice = resolveVoiceReference();
    this.ready = new Promise((resolve, reject) => { this.resolveReady = resolve; this.rejectReady = reject; });
    const args = [
      'run', '--no-project', '--with', 'speech-to-speech==0.2.9', 'python', ttsWorkerPath,
      '--serve', '--model-name', ttsModel,
      '--ref-audio', voice.wavPath,
      '--ref-text-file', voice.txtPath,
      '--language', process.env.QWEN3_TTS_LANGUAGE || 'auto',
      '--output-sample-rate', String(ttsWorkerSampleRate),
      '--temperature', process.env.QWEN3_TTS_TEMPERATURE || '0.7',
      '--top-k', process.env.QWEN3_TTS_TOP_K || '30',
      '--blocksize', process.env.CODEC_TTS_WORKER_BLOCKSIZE || '2048',
    ];
    const seed = process.env.QWEN3_TTS_SEED;
    if (seed) args.push('--seed', seed);
    this.proc = spawn(process.env.CODEC_TTS_WORKER_COMMAND || 'uv', args, { cwd: repo, stdio: ['pipe', 'pipe', 'pipe'] });
    trace('tts_worker_spawn', { command: process.env.CODEC_TTS_WORKER_COMMAND || 'uv', args: args.join(' ') });
    this.proc.stdout.on('data', (chunk) => this.handleStdout(Buffer.from(chunk)));
    this.proc.stderr.on('data', (chunk) => console.error('[qwen-worker]', String(chunk).trim()));
    this.proc.once('error', (e) => this.rejectReady?.(e));
    this.proc.once('exit', (code, signal) => {
      if (this.intentionalExit) {
        trace('tts_worker_exit_intentional', { code: code ?? 'none', signal: signal ?? 'none' });
        this.active.clear();
        return;
      }
      const err = new Error(`Qwen TTS worker exited code=${code ?? 'none'} signal=${signal ?? 'none'}`);
      this.rejectReady?.(err);
      for (const req of this.active.values()) req.error(err);
      this.active.clear();
    });
  }

  private sendFrame(type: number, id: number, payload = Buffer.alloc(0)) {
    const header = Buffer.allocUnsafe(frameHeaderBytes);
    header.writeUInt8(type, 0);
    header.writeUInt32LE(id >>> 0, 1);
    header.writeUInt32LE(payload.byteLength, 5);
    this.proc.stdin.write(Buffer.concat([header, payload]));
  }

  private handleStdout(chunk: Buffer) {
    this.stdoutBuffer = Buffer.concat([this.stdoutBuffer, chunk]);
    while (this.stdoutBuffer.byteLength >= frameHeaderBytes) {
      const type = this.stdoutBuffer.readUInt8(0);
      const id = this.stdoutBuffer.readUInt32LE(1);
      const len = this.stdoutBuffer.readUInt32LE(5);
      const frameLen = frameHeaderBytes + len;
      if (this.stdoutBuffer.byteLength < frameLen) return;
      const payload = this.stdoutBuffer.subarray(frameHeaderBytes, frameLen);
      this.stdoutBuffer = this.stdoutBuffer.subarray(frameLen);
      this.handleFrame(type, id, payload);
    }
  }

  private handleFrame(type: number, id: number, payload: Buffer) {
    if (type === workerOutputReady) { trace('tts_worker_ready'); this.resolveReady?.(); return; }
    const req = this.active.get(id);
    if (!req) return;
    if (type === workerOutputAudioStart) {
      req.sampleRate = payload.byteLength >= 4 ? payload.readUInt32LE(0) : ttsWorkerSampleRate;
      trace('tts_audio_start', { id, turnId: req.turnId, index: req.index, sampleRate: req.sampleRate, textChars: req.text.length, text: req.text });
      return;
    }
    if (type === workerOutputAudioChunk) {
      this.broadcastPcmChunk(req, payload);
      return;
    }
    if (type === workerOutputAudioDone) {
      this.active.delete(id);
      // Drop buffered silence at the end of each request. Qwen often emits long zero tails;
      // playing them makes the next sentence feel like TTS has jammed.
      const droppedSilenceChunks = req.pendingSilence.length;
      req.pendingSilence = [];
      trace('tts_audio_done', { id, turnId: req.turnId, index: req.index, chunks: req.chunks, bytes: req.bytes, elapsedMs: Date.now() - req.startedAt, firstAudioMs: req.firstAudioAt ? req.firstAudioAt - req.startedAt : null, droppedSilenceChunks });
      req.done();
      return;
    }
    if (type === workerOutputError) {
      this.active.delete(id);
      const message = payload.toString('utf8');
      trace('tts_audio_error', { id, turnId: req.turnId, index: req.index, message });
      req.error(new Error(message));
    }
  }

  private isSilentPcm(payload: Buffer) {
    if (payload.byteLength < 2) return true;
    let peak = 0;
    for (let i = 0; i + 1 < payload.byteLength; i += 2) {
      const v = Math.abs(payload.readInt16LE(i));
      if (v > peak) peak = v;
      if (peak > Number(process.env.CODEC_TTS_SILENCE_PEAK || 2)) return false;
    }
    return true;
  }

  private emitPcm(req: { turnId: string; index: number; text: string; sampleRate: number }, payload: Buffer) {
    broadcast({ type: 'audio_pcm', turnId: req.turnId, index: req.index, text: req.text, sample_rate: req.sampleRate, chunk: payload.toString('base64') });
  }

  private broadcastPcmChunk(req: { turnId: string; index: number; text: string; sampleRate: number; sawAudio: boolean; pendingSilence: Buffer[] }, payload: Buffer) {
    if (this.isSilentPcm(payload)) {
      if (req.sawAudio) req.pendingSilence.push(payload);
      return;
    }
    req.sawAudio = true;
    req.firstAudioAt ??= Date.now();
    req.chunks += 1;
    req.bytes += payload.byteLength;
    trace('tts_audio_chunk', { turnId: req.turnId, index: req.index, chunkBytes: payload.byteLength, chunks: req.chunks, firstAudioMs: req.firstAudioAt - req.startedAt });
    for (const silence of req.pendingSilence) this.emitPcm(req, silence);
    req.pendingSilence = [];
    this.emitPcm(req, payload);
  }

  async speak(turnId: string, index: number, text: string) {
    await this.ready;
    const id = this.nextId++;
    await new Promise<void>((resolve, reject) => {
      trace('tts_request_start', { id, turnId, index, textChars: text.length, text });
      this.active.set(id, { turnId, index, text, sampleRate: ttsWorkerSampleRate, startedAt: Date.now(), chunks: 0, bytes: 0, sawAudio: false, pendingSilence: [], done: resolve, error: reject });
      this.sendFrame(workerInputSpeak, id, Buffer.from(text, 'utf8'));
    });
  }

  cancel(reason = 'cancel') {
    for (const [id, req] of this.active) {
      trace('tts_cancel', { id, turnId: req.turnId, index: req.index, reason, elapsedMs: Date.now() - req.startedAt, chunks: req.chunks, bytes: req.bytes });
      this.sendFrame(workerInputCancel, id);
      req.done();
    }
    this.active.clear();
  }

  stop() {
    this.intentionalExit = true;
    try { this.sendFrame(workerInputShutdown, 0); } catch {}
    this.proc.kill();
  }
}

let qwenTtsWorker: QwenTtsWorker | null = ttsMode === 'worker' ? new QwenTtsWorker() : null;

async function synthesizeTts(turnId: string, index: number, text: string, ttsText: string, shouldContinue: () => boolean = () => true) {
  if (qwenTtsWorker) {
    await qwenTtsWorker.speak(turnId, index, ttsText);
    return;
  }
  trace('tts_http_request_start', { turnId, index, textChars: ttsText.length, text: ttsText, timeoutMs: ttsHttpTimeoutMs });
  const startedAt = Date.now();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ttsHttpTimeoutMs);
  let res: Response;
  try {
    res = await fetch(`${ttsBase}/audio/speech`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: ttsModel, voice: character, input: ttsText, response_format: 'wav', ddpm_steps: Number(process.env.VOXCPM2_DDPM_STEPS || 10) }),
      signal: controller.signal,
    });
  } catch (e) {
    trace('tts_http_request_error', { turnId, index, elapsedMs: Date.now() - startedAt, message: String((e as Error)?.message || e) });
    throw e;
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) throw new Error(`TTS ${res.status}: ${await res.text()}`);
  const audioBuffer = Buffer.from(await res.arrayBuffer());
  trace('tts_http_request_done', { turnId, index, bytes: audioBuffer.byteLength, elapsedMs: Date.now() - startedAt });
  if (!shouldContinue()) {
    trace('tts_http_drop_stale_before_broadcast', { turnId, index });
    return;
  }
  const audio = audioBuffer.toString('base64');
  broadcast({ type: 'audio_chunk', turnId, index, text, ttsText, sample_rate: Number(res.headers.get('X-Codec-Sample-Rate') || 24000), chunk: audio });
}

class PiRpc implements Brain {
  proc;
  private seq = 0;
  private sessionName;
  private label;
  private sentenceBuffer = new SentenceBuffer();
  private currentTurnId = '';
  private ttsQueue = Promise.resolve();
  private audioQueueVersion = 0;
  private activeTools = new Set<string>();
  private slowFillerTimer?: ReturnType<typeof setTimeout>;
  private streamedAssistantText = '';
  private sawFirstTokenThisTurn = false;
  private inAngleTag = false;
  private sawInternalTagsThisTurn = false;
  private fillerStartPlayedThisTurn = false;
  private fillerDonePlayedThisTurn = false;
  private lastFillerAt = 0;

  constructor(private spawnArgs: string[], label = 'pi') {
    this.label = label;
    this.sessionName = `codec-${character}-${label}-demo`;
    this.proc = spawn('pi', spawnArgs, { cwd: characterCwd, stdio: ['pipe', 'pipe', 'pipe'] });
    this.proc.stderr.on('data', (d) => console.error(`[${label}]`, String(d).trim()));
    this.attachJsonl(this.proc.stdout, (line) => {
      if (!line.trim()) return;
      let event: RpcEvent;
      try { event = JSON.parse(line); } catch (e) { console.error('[bridge] bad rpc json', line); return; }
      this.handleEvent(event);
    });
    this.send({ type: 'set_session_name', name: this.sessionName });
    this.send({ type: 'get_state' });
  }

  private attachJsonl(stream: NodeJS.ReadableStream, onLine: (line: string) => void) {
    const decoder = new StringDecoder('utf8');
    let buffer = '';
    stream.on('data', (chunk) => {
      buffer += typeof chunk === 'string' ? chunk : decoder.write(chunk);
      for (;;) {
        const i = buffer.indexOf('\n');
        if (i < 0) break;
        let line = buffer.slice(0, i);
        buffer = buffer.slice(i + 1);
        if (line.endsWith('\r')) line = line.slice(0, -1);
        onLine(line);
      }
    });
  }

  send(obj: Record<string, unknown>) {
    this.proc.stdin.write(`${JSON.stringify({ id: `req-${++this.seq}`, ...obj })}\n`);
  }

  info() { return `${this.label}:${this.spawnArgs.join(' ')}`; }
  stop() { this.proc.kill(); }

  onClientConnected() {
    this.send({ type: 'get_state' });
  }

  prompt(text: string) {
    this.currentTurnId = randomUUID();
    this.sentenceBuffer.reset();
    this.ttsQueue = Promise.resolve();
    this.audioQueueVersion += 1;
    this.streamedAssistantText = '';
    this.inAngleTag = false;
    this.sawInternalTagsThisTurn = false;
    this.sawFirstTokenThisTurn = false;
    qwenTtsWorker?.cancel('pi_prompt');
    broadcast({ type: 'audio_reset', reason: 'pi_prompt' });
    this.activeTools.clear();
    this.fillerStartPlayedThisTurn = false;
    this.fillerDonePlayedThisTurn = false;
    if (this.slowFillerTimer) clearTimeout(this.slowFillerTimer);
    broadcast({ type: 'turn_started', turnId: this.currentTurnId, character });
    broadcast({ type: 'phase', phase: 'thinking' });
    const message = `Snake: ${text}`;
    trace('brain_request_start', { turnId: this.currentTurnId, textChars: text.length });
    this.send({ type: 'prompt', message, streamingBehavior: 'followUp' });
  }

  abort() {
    this.send({ type: 'abort' });
    this.sentenceBuffer.reset();
    this.ttsQueue = Promise.resolve();
    this.audioQueueVersion += 1;
    this.streamedAssistantText = '';
    this.inAngleTag = false;
    this.sawInternalTagsThisTurn = false;
    this.sawFirstTokenThisTurn = false;
    qwenTtsWorker?.cancel('pi_abort');
    broadcast({ type: 'audio_reset', reason: 'pi_abort' });
    this.activeTools.clear();
    this.fillerStartPlayedThisTurn = false;
    this.fillerDonePlayedThisTurn = false;
    if (this.slowFillerTimer) clearTimeout(this.slowFillerTimer);
    broadcast({ type: 'phase', phase: 'interrupted' });
  }

  newSession() {
    this.sentenceBuffer.reset();
    this.ttsQueue = Promise.resolve();
    this.audioQueueVersion += 1;
    this.streamedAssistantText = '';
    this.inAngleTag = false;
    this.sawInternalTagsThisTurn = false;
    this.sawFirstTokenThisTurn = false;
    qwenTtsWorker?.cancel('pi_new_session');
    broadcast({ type: 'audio_reset', reason: 'pi_new_session' });
    this.activeTools.clear();
    this.fillerStartPlayedThisTurn = false;
    this.fillerDonePlayedThisTurn = false;
    if (this.slowFillerTimer) clearTimeout(this.slowFillerTimer);
    broadcast({ type: 'phase', phase: 'idle' });
    this.send({ type: 'new_session' });
    this.send({ type: 'set_session_name', name: this.sessionName });
  }

  private normalizeDeltaSpacing(delta: string) {
    if (!delta) return delta;
    if (!this.streamedAssistantText) return delta;
    const prev = this.streamedAssistantText.slice(-1);
    const next = delta[0];
    if (!prev || !next) return delta;
    if (/\s/.test(prev) || /\s/.test(next)) return delta;

    // Be conservative: do NOT inject spaces across alphanumeric boundaries,
    // otherwise chunk splits like "L" + "oud" become "L oud".
    // Only repair obvious missing space after punctuation.
    if (/[.!?;:,)\]]/.test(prev) && /[A-Za-z0-9"'“‘(\[]/.test(next)) return ` ${delta}`;
    return delta;
  }

  private stripAngleTagsStreaming(delta: string) {
    let out = '';
    for (const ch of delta) {
      if (this.inAngleTag) {
        this.sawInternalTagsThisTurn = true;
        if (ch === '>') this.inAngleTag = false;
        continue;
      }
      if (ch === '<') {
        this.inAngleTag = true;
        this.sawInternalTagsThisTurn = true;
        continue;
      }
      out += ch;
    }
    return out;
  }

  private handleEvent(event: RpcEvent) {
    if (event.type === 'response') {
      if (event.data?.sessionFile) broadcast({ type: 'session', character, ...event.data });
      if (event.command === 'prompt' && event.success === false) {
        const message = event.error || 'Pi RPC prompt rejected';
        broadcast({ type: 'error', turnId: this.currentTurnId, message });
        broadcast({ type: 'turn_completed', turnId: this.currentTurnId });
      }
      return;
    }
    if (event.type === 'error') {
      const message = String(event.error || event.message || 'Pi RPC error');
      broadcast({ type: 'error', turnId: this.currentTurnId, message });
      broadcast({ type: 'turn_completed', turnId: this.currentTurnId });
      return;
    }
    if (event.type === 'agent_end') {
      const tail = this.sentenceBuffer.flush();
      if (tail) this.enqueueTts(tail);
      this.ttsQueue.finally(() => broadcast({ type: 'turn_completed', turnId: this.currentTurnId }));
      return;
    }
    if (event.type === 'tool_execution_start') {
      if (event.toolCallId) this.activeTools.add(event.toolCallId);
      if (!this.fillerStartPlayedThisTurn) {
        this.playFiller('tool_start', event.toolName);
        this.fillerStartPlayedThisTurn = true;
      }
      if (this.slowFillerTimer) clearTimeout(this.slowFillerTimer);
      this.slowFillerTimer = setTimeout(() => {
        if (this.activeTools.size > 0) this.playFiller('tool_slow', event.toolName);
      }, Number(process.env.CODEC_TOOL_SLOW_FILLER_MS || 2500));
      return;
    }
    if (event.type === 'tool_execution_end') {
      if (event.toolCallId) this.activeTools.delete(event.toolCallId);
      if (this.activeTools.size === 0) {
        if (this.slowFillerTimer) clearTimeout(this.slowFillerTimer);
        if (!this.fillerDonePlayedThisTurn) {
          this.playFiller('tool_done', event.toolName);
          this.fillerDonePlayedThisTurn = true;
        }
      }
      return;
    }
    if (event.type !== 'message_update') return;
    const u = event.assistantMessageEvent;
    if (!u) return;
    if (isHiddenAssistantEvent(u)) return;
    if (u.type === 'text_delta' && u.delta) {
      const noTags = this.stripAngleTagsStreaming(u.delta);
      const cleanDelta = sanitizeAssistantDelta(noTags);
      if (!cleanDelta.trim()) return;
      let spacedDelta = this.normalizeDeltaSpacing(cleanDelta);
      if (this.sawInternalTagsThisTurn && !this.streamedAssistantText.trim()) {
        spacedDelta = spacedDelta.replace(/^\s*(?:thought|analysis|reasoning)\b\s*[:\-]?\s*/i, '');
      }
      if (!spacedDelta.trim()) return;
      if (!this.sawFirstTokenThisTurn) {
        this.sawFirstTokenThisTurn = true;
        trace('brain_first_token', { turnId: this.currentTurnId });
      }
      this.streamedAssistantText += spacedDelta;
      broadcast({ type: 'assistant_delta', turnId: this.currentTurnId, delta: spacedDelta });
      for (const sentence of this.sentenceBuffer.push(spacedDelta)) this.enqueueTts(sentence);
    } else if (u.type === 'done') {
      const tail = this.sentenceBuffer.flush();
      if (tail) this.enqueueTts(tail);
    }
  }

  private playFiller(category: Filler['category'], toolName?: string) {
    if (!piEnableFillers) return;
    const nowMs = Date.now();
    const minGapMs = Number(process.env.CODEC_FILLER_MIN_GAP_MS || 6000);
    if (nowMs - this.lastFillerAt < minGapMs) return;
    const filler = pickFiller(category);
    if (!filler) return;
    this.lastFillerAt = nowMs;
    const turnId = this.currentTurnId || randomUUID();
    const index = ++this.seq;
    const queueVersion = this.audioQueueVersion;
    this.ttsQueue = this.ttsQueue.then(async () => {
      if (queueVersion !== this.audioQueueVersion) return;
      const audio = readFileSync(`${repo}/${filler.path}`).toString('base64');
      if (queueVersion !== this.audioQueueVersion) return;
      broadcast({
        type: 'audio_chunk',
        turnId,
        index,
        text: filler.text,
        sample_rate: 24000,
        chunk: audio,
        filler: true,
        category,
        toolName,
      });
    }).catch((e) => broadcast({ type: 'error', message: `Filler ${category}: ${String(e?.message || e)}` }));
  }

  private enqueueTts(text: string) {
    const turnId = this.currentTurnId;
    const index = ++this.seq;
    const ttsText = stripMarkdownForTts(text);
    if (!ttsText) return;
    const queueVersion = this.audioQueueVersion;
    broadcast({ type: 'sentence', turnId, text });
    this.ttsQueue = this.ttsQueue.then(async () => {
      if (queueVersion !== this.audioQueueVersion) return;
      broadcast({ type: 'phase', phase: 'speaking' });
      await synthesizeTts(turnId, index, text, ttsText, () => queueVersion === this.audioQueueVersion);
      if (queueVersion !== this.audioQueueVersion) return;
    }).catch((e) => broadcast({ type: 'error', message: String(e?.message || e) }));
  }
}

const clients = new Set<ServerWebSocket<unknown>>();
function broadcast(obj: Record<string, unknown>) {
  const line = JSON.stringify(obj);
  for (const ws of clients) if (ws.readyState === 1) ws.send(line);
  if (obj.type !== 'audio_pcm' && obj.type !== 'audio_chunk') console.log('[bridge]', line);
}

function createBrain(): Brain {
  if (brainMode !== 'pi') {
    throw new Error(`Unsupported CODEC_BRAIN_MODE=${brainMode}. Foxline runtime uses pi-rpc only.`);
  }
  return new PiRpc(buildPiArgs(), 'pi-rpc');
}

let brain: Brain = createBrain();

function switchCharacter(next: string) {
  if (next === character) return;
  brain.abort();
  brain.stop?.();
  qwenTtsWorker?.stop();
  setCharacter(next);
  fillers = loadFillers();
  qwenTtsWorker = ttsMode === 'worker' ? new QwenTtsWorker() : null;
  brain = createBrain();
  broadcast({ type: 'audio_reset', reason: 'switch_character' });
  broadcast({ type: 'character_switched', character, characters: enabledCharacters() });
  broadcast({ type: 'phase', phase: 'idle' });
  trace('character_switched', { character });
  trajectory('character_switched', { character, characterInstructions: loadCharacterInstructions() });
}

Bun.serve({
  port,
  fetch(req, server) {
    if (server.upgrade(req)) return undefined;
    return new Response('codec bridge ok\n');
  },
  websocket: {
    open(ws) {
      clients.add(ws);
      console.log(`[bridge] client connected (${clients.size})`);
      ws.send(JSON.stringify({ type: 'ready', character, characters: enabledCharacters(), model: piModel }));
      brain.onClientConnected?.();
    },
    close(ws) {
      clients.delete(ws);
      console.log(`[bridge] client disconnected (${clients.size})`);
    },
    message(_ws, data) {
      const msg = JSON.parse(String(data)) as ClientMsg;
      console.log('[bridge] client message', msg.type, 'text' in msg ? msg.text : '');
      if (msg.type === 'client_trace') { trace(`client_${msg.event}`, msg.data || {}); return; }
      if (msg.type === 'user_utterance') {
        trace('stt_final', { text: msg.text, source: 'frontend_user_utterance' });
        brain.prompt(msg.text);
      }
      if (msg.type === 'interrupt') {
        trace('barge_in_received', { source: 'frontend_interrupt' });
        brain.abort();
      }
      if (msg.type === 'new_session') brain.newSession();
      if (msg.type === 'switch_character') {
        try { switchCharacter(msg.character); }
        catch (e) { broadcast({ type: 'error', message: String((e as Error)?.message || e) }); }
      }
    },
  },
});

console.log(`[bridge] listening ws://127.0.0.1:${port} cwd=${characterCwd} brain=${brain.info()} session=${sessionDir} trace=${traceFile} trajectory=${trajectoryFile}`);
trace('bridge_start', { port, character, brain: brain.info(), sessionDir, traceFile, trajectoryFile, legacyBenchmarkTraceFile });
trajectory('session_start', { port, character, brain: brain.info(), sessionDir, traceFile, trajectoryFile, characterInstructions: loadCharacterInstructions() });
