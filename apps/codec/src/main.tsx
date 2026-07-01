import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import './styles.css';
import { MicrophonePcmStreamer } from './services/stt';
import { RustVoiceGatewayClient, StreamingAudioPlayer, type BridgeEvent, type CodecCharacterInfo, type CodecPhase } from './services/gatewayClient';
import { sfx } from './services/sfx';

type TranscriptLine = { speaker: string; text: string; at: string };
type FaceSet = { base: string; mouth1: string; mouth2: string; eyes1?: string; eyes2?: string };
type CodecCharacter = string;

type AvatarRole = 'base' | 'mouth_1' | 'mouth_2' | 'eyes_1' | 'eyes_2';
type AvatarFile = { modulePath: string; url: string };
const avatarModules = import.meta.glob<string>('../../../agents/*/assets/avatar/*/*.png', { eager: true, import: 'default' });
const personaPortraitModules = import.meta.glob<string>('../../../personas/*/codec/portrait.png', { eager: true, import: 'default' });

function extractFaceId(modulePath: string): string | undefined {
  const m = modulePath.match(/__id_([0-9a-fA-F]+)__/);
  return m?.[1]?.toLowerCase();
}

function extractFrameIndex(modulePath: string): number {
  const m = modulePath.match(/__(\d+)\.png$/);
  return m ? Number(m[1]) : Number.MAX_SAFE_INTEGER;
}

function pickRoleFile(files: AvatarFile[], role: AvatarRole, faceId?: string) {
  const roleFiles = files.filter((f) =>
    f.modulePath.includes(`__${role}__`) && (!faceId || f.modulePath.includes(`__id_${faceId}__`)),
  );
  if (!roleFiles.length) return undefined;
  const preferred = role === 'base'
    ? roleFiles.find((f) => f.modulePath.includes('__source__'))
    : roleFiles.find((f) => f.modulePath.includes('__composed__'));
  return (preferred || roleFiles[0]).url;
}

function pickPrimaryFaceId(files: AvatarFile[]): string | undefined {
  const ids = [...new Set(files.map((f) => extractFaceId(f.modulePath)).filter(Boolean) as string[])];
  const complete = ids.filter((id) =>
    files.some((f) => f.modulePath.includes(`__id_${id}__`) && f.modulePath.includes('__base__')) &&
    files.some((f) => f.modulePath.includes(`__id_${id}__`) && f.modulePath.includes('__mouth_1__')) &&
    files.some((f) => f.modulePath.includes(`__id_${id}__`) && f.modulePath.includes('__mouth_2__')),
  );
  const ranked = (complete.length ? complete : ids)
    .map((id) => ({
      id,
      first: Math.min(
        ...files
          .filter((f) => f.modulePath.includes(`__id_${id}__`))
          .map((f) => extractFrameIndex(f.modulePath)),
      ),
    }))
    .sort((a, b) => a.first - b.first || a.id.localeCompare(b.id));
  return ranked[0]?.id;
}

function buildAvatarFaceMap() {
  const grouped = new Map<string, Map<string, AvatarFile[]>>();
  for (const [modulePath, url] of Object.entries(avatarModules)) {
    const m = modulePath.match(/\.\.\/\.\.\/\.\.\/agents\/([^/]+)\/assets\/avatar\/([^/]+)\/[^/]+\.png$/);
    if (!m) continue;
    const [, characterId, avatarId] = m;
    if (!grouped.has(characterId)) grouped.set(characterId, new Map());
    const avatarMap = grouped.get(characterId)!;
    if (!avatarMap.has(avatarId)) avatarMap.set(avatarId, []);
    avatarMap.get(avatarId)!.push({ modulePath, url });
  }

  const out = new Map<string, Map<string, FaceSet>>();
  for (const [characterId, avatarMap] of grouped) {
    const faceMap = new Map<string, FaceSet>();
    for (const [avatarId, files] of avatarMap) {
      const primaryFaceId = pickPrimaryFaceId(files);
      const base = pickRoleFile(files, 'base', primaryFaceId) || pickRoleFile(files, 'base');
      const mouth1 = pickRoleFile(files, 'mouth_1', primaryFaceId) || pickRoleFile(files, 'mouth_1');
      const mouth2 = pickRoleFile(files, 'mouth_2', primaryFaceId) || pickRoleFile(files, 'mouth_2');
      if (!base || !mouth1 || !mouth2) continue;
      const eyes1 = pickRoleFile(files, 'eyes_1', primaryFaceId) || pickRoleFile(files, 'eyes_1');
      const eyes2 = pickRoleFile(files, 'eyes_2', primaryFaceId) || pickRoleFile(files, 'eyes_2');
      faceMap.set(avatarId, { base, mouth1, mouth2, eyes1, eyes2 });
    }
    out.set(characterId, faceMap);
  }
  for (const [modulePath, url] of Object.entries(personaPortraitModules)) {
    const m = modulePath.match(/\.\.\/\.\.\/\.\.\/personas\/([^/]+)\/codec\/portrait\.png$/);
    if (!m) continue;
    const [, personaId] = m;
    const faceMap = out.get(personaId) || new Map<string, FaceSet>();
    faceMap.set('portrait', { base: url, mouth1: url, mouth2: url });
    out.set(personaId, faceMap);
  }
  return out;
}

const avatarFaceMap = buildAvatarFaceMap();
const fallbackCharacter: CodecCharacter = 'alex-grant';
const defaultSupportFace = avatarFaceMap.get(fallbackCharacter)?.values().next().value || { base: '', mouth1: '', mouth2: '' };
const snake = avatarFaceMap.get('snake')?.get('snake_normal') || defaultSupportFace;

function displaySpeakerName(characterId: string, characters: CodecCharacterInfo[]) {
  if (characterId === 'campbell') return 'Colonel';
  return characters.find((c) => c.id === characterId)?.speakerName || characterId;
}

function supportFaceFor(characterId: string, characters: CodecCharacterInfo[]) {
  const character = characters.find((c) => c.id === characterId);
  const avatarId = character?.avatar;
  const perCharacter = avatarFaceMap.get(characterId);
  if (avatarId && perCharacter?.has(avatarId)) return perCharacter.get(avatarId)!;
  if (perCharacter && perCharacter.size > 0) return [...perCharacter.values()][0];
  return defaultSupportFace;
}

const now = () => new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });

const ASSISTANT_DEDUPE_MIN_CHARS = 12;

function suffixPrefixOverlap(left: string, right: string): number {
  const max = Math.min(left.length, right.length);
  for (let length = max; length >= ASSISTANT_DEDUPE_MIN_CHARS; length -= 1) {
    if (left.endsWith(right.slice(0, length))) return length;
  }
  return 0;
}

function appendAssistantDelta(current: string, delta: string): string {
  if (!delta) return current;
  if (!current) return delta;
  if (current.length >= ASSISTANT_DEDUPE_MIN_CHARS && delta.startsWith(current)) return delta;
  if (delta.trim().length >= ASSISTANT_DEDUPE_MIN_CHARS && current.endsWith(delta)) return current;
  const overlap = suffixPrefixOverlap(current, delta);
  return current + delta.slice(overlap);
}

function faceFor(set: FaceSet, level: number, active: boolean, tick: number) {
  if (tick % 240 > 232 && set.eyes1) return set.eyes1;
  if (!active || level < 0.08) return set.base;
  return level > 0.38 ? set.mouth2 : set.mouth1;
}

function useCodecDemo() {
  const autoMicEnabled = import.meta.env.VITE_CODEC_AUTO_MIC !== 'false';
  const [phase, setPhase] = useState<CodecPhase>('idle');
  const [transcript, setTranscript] = useState<TranscriptLine[]>([{ speaker: 'System', text: 'Codec receiver ready. Open Memory to choose support.', at: now() }]);
  const [assistantResponse, setAssistantResponse] = useState(autoMicEnabled ? 'Mic auto-starting...' : 'Awaiting frequency activation.');
  const [supportCharacter, setSupportCharacter] = useState<CodecCharacter>(fallbackCharacter);
  const [characters, setCharacters] = useState<CodecCharacterInfo[]>([]);
  const charactersRef = useRef<CodecCharacterInfo[]>([]);
  const [snakeLevel, setSnakeLevel] = useState(0);
  const [campbellLevel, setCampbellLevel] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [liveCaption, setLiveCaption] = useState('');
  const [sttStatus, setSttStatus] = useState('PTT idle');
  const [llmModel, setLlmModel] = useState('');
  const [pttActive, setPttActive] = useState(false);
  const [showStatus, setShowStatus] = useState(false);
  const gatewayMic = useRef<MicrophonePcmStreamer | undefined>(undefined);
  const bridge = useRef<RustVoiceGatewayClient | undefined>(undefined);
  const player = useRef<StreamingAudioPlayer | undefined>(undefined);
  const assistantText = useRef('');
  const assistantLineIndex = useRef<number | null>(null);
  const phaseRef = useRef<CodecPhase>('idle');
  const supportCharacterRef = useRef<CodecCharacter>(fallbackCharacter);
  const activeTurnId = useRef<string | null>(null);
  const bargeInMinLevel = Number(import.meta.env.VITE_CODEC_BARGE_IN_MIN_LEVEL || 0.18);
  const autoResumeListeningAfterSwitch = useRef(false);
  const listeningActive = useRef(false);
  const startListeningInFlight = useRef(false);
  const lastHighSnakeLevelAt = useRef(0);

  useEffect(() => { phaseRef.current = phase; }, [phase]);
  useEffect(() => { supportCharacterRef.current = supportCharacter; }, [supportCharacter]);
  useEffect(() => { charactersRef.current = characters; }, [characters]);

  useEffect(() => {
    player.current = new StreamingAudioPlayer();
    player.current.onPlaying = () => setPhase('speaking');
    player.current.onStopped = () => setPhase('listening');
    bridge.current = new RustVoiceGatewayClient();
    player.current.onDebug = (event, data) => bridge.current?.trace(event, data);
    const off = bridge.current.onEvent(async (event: BridgeEvent) => {
      if (event.type === 'ready') {
        const character = event.character || fallbackCharacter;
        setSupportCharacter(character);
        setCharacters(event.characters || []);
        setLlmModel(event.model || '');
        setConnected(true);
        setTranscript((t) => [...t, { speaker: 'System', text: `${displaySpeakerName(character, event.characters || [])} Rust Voice Gateway online.`, at: now() }]);
        autoStartListening('bridge_ready');
      }
      if (event.type === 'character_switched') {
        void sfx.play('codec_tune');
        const character = event.character || fallbackCharacter;
        setSupportCharacter(character);
        setCharacters(event.characters || characters);
        assistantText.current = '';
        assistantLineIndex.current = null;
        setAssistantResponse(autoMicEnabled ? 'Mic auto-starting...' : 'Awaiting frequency activation.');
        setLiveCaption('');
        setPhase('idle');
        setTranscript([{ speaker: 'System', text: `Memory tuned to ${displaySpeakerName(character, event.characters || charactersRef.current)}. Fresh channel opened.`, at: now() }]);
        if (autoResumeListeningAfterSwitch.current) {
          autoResumeListeningAfterSwitch.current = false;
          void startListening(false);
        }
      }
      if (event.type === 'phase') setPhase(event.phase);
      if (event.type === 'audio_reset') {
        activeTurnId.current = null;
        player.current?.stop();
        bridge.current?.trace('audio_reset_received', { reason: event.reason });
      }
      if (event.type === 'turn_started') {
        activeTurnId.current = event.turnId;
        assistantText.current = '';
        setAssistantResponse('...');
        setTranscript((t) => {
          assistantLineIndex.current = t.length;
          const character = event.character || supportCharacterRef.current;
          return [...t, { speaker: displaySpeakerName(character, charactersRef.current), text: '', at: now() }];
        });
      }
      if (event.type === 'assistant_delta') {
        const nextAssistantText = appendAssistantDelta(assistantText.current, event.delta);
        if (nextAssistantText === assistantText.current) return;
        assistantText.current = nextAssistantText;
        setAssistantResponse(assistantText.current);
        setTranscript((t) => {
          const idx = assistantLineIndex.current;
          if (idx === null || idx >= t.length) return t;
          const next = [...t];
          next[idx] = { ...next[idx], text: assistantText.current.trimStart() };
          return next;
        });
      }
      if (event.type === 'user_transcript') {
        const text = event.text.trim();
        if (!text) return;
        if (event.final) {
          setLiveCaption('');
          setTranscript((t) => [...t, { speaker: 'Snake', text, at: now() }]);
        } else {
          setLiveCaption(text);
        }
      }
      if (event.type === 'audio_chunk') {
        if (event.turnId !== activeTurnId.current) {
          bridge.current?.trace('audio_chunk_ignored_stale', { turnId: event.turnId, activeTurnId: activeTurnId.current, index: event.index });
          return;
        }
        bridge.current?.trace('audio_chunk_received', { turnId: event.turnId, index: event.index, textChars: event.text.length, base64Chars: event.chunk.length });
        await player.current?.enqueueBase64Wav(event.chunk);
      }
      if (event.type === 'audio_pcm') {
        bridge.current?.trace('audio_pcm_received', { turnId: event.turnId, index: event.index, sampleRate: event.sample_rate, textChars: event.text.length, base64Chars: event.chunk.length });
        await player.current?.enqueuePcm16(event.chunk, event.sample_rate);
      }
      if (event.type === 'turn_completed') {
        setTranscript((t) => {
          const idx = assistantLineIndex.current;
          assistantLineIndex.current = null;
          if (idx === null || idx >= t.length || !assistantText.current.trim()) return t;
          const next = [...t];
          next[idx] = { ...next[idx], text: assistantText.current.trim() };
          return next;
        });
      }
      if (event.type === 'session' && event.sessionId) {
        setLlmModel(event.model || '');
        setTranscript((t) => [...t, { speaker: 'System', text: `Pi session ${event.sessionName || event.sessionId}`, at: now() }]);
      }
      if (event.type === 'disconnected') {
        setConnected(false);
        void sfx.play('codec_noise', { gain: 0.5 });
        setTranscript((t) => {
          if (t[t.length - 1]?.text === 'Codec bridge link lost. Re-establishing...') return t;
          return [...t, { speaker: 'System', text: 'Codec bridge link lost. Re-establishing...', at: now() }];
        });
      }
      if (event.type === 'error') { setError(event.message); setPhase('error'); }
    });
    bridge.current.connect();
    const autoMicTimer = window.setTimeout(() => autoStartListening('mount'), 300);
    const timer = window.setInterval(() => setCampbellLevel(player.current?.getLevel() || 0), 33);
    return () => {
      off();
      window.clearTimeout(autoMicTimer);
      window.clearInterval(timer);
      gatewayMic.current?.stop();
      player.current?.stop();
    };
  }, []);

  function autoStartListening(reason: string) {
    if (!autoMicEnabled || listeningActive.current || startListeningInFlight.current) return;
    bridge.current?.trace('auto_mic_start', { reason });
    void startListening(false);
  }

  async function startListening(manualMode = false) {
    if (manualMode ? pttActive : listeningActive.current || startListeningInFlight.current) return;
    startListeningInFlight.current = true;
    player.current?.unlock(); // must run synchronously inside the tap gesture for mobile audio output
    sfx.unlock();
    if (phaseRef.current === 'idle') void sfx.play('codec_call');
    setError(null);
    setPhase('connecting');
    setTranscript((t) => [...t, { speaker: 'System', text: manualMode ? 'PTT open. Speak as Snake, then release/send.' : 'VAD listening. Speak as Snake; silence will send automatically.', at: now() }]);
    if (!gatewayMic.current) {
      gatewayMic.current = new MicrophonePcmStreamer();
      gatewayMic.current.onLevel((level) => {
        setSnakeLevel(level);
        if (level >= bargeInMinLevel) lastHighSnakeLevelAt.current = Date.now();
      });
      gatewayMic.current.onStatus((status) => {
        setSttStatus(status);
        bridge.current?.trace('gateway_mic_status', { status });
      });
      gatewayMic.current.onAudioFrame((data) => {
        bridge.current?.trace('gateway_audio_frame_sent', {
          chunksSent: data.chunksSent,
          samples: data.samples,
          level: data.level,
          mode: data.mode,
        });
        (bridge.current as RustVoiceGatewayClient | undefined)?.sendAudioPcm16(data.pcm16);
      });
    }
    try {
      bridge.current?.trace('gateway_mic_start_attempt', { manualMode });
      await gatewayMic.current.start(manualMode);
      listeningActive.current = true;
      setPttActive(manualMode);
      setPhase('listening');
      setSttStatus('Rust Voice Gateway mic streaming');
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      bridge.current?.trace('gateway_mic_start_failed', { manualMode, error: msg });
      listeningActive.current = false;
      setSttStatus(msg);
      setError(msg);
      setPhase('error');
    } finally {
      startListeningInFlight.current = false;
    }
  }

  const activateColonel = () => startListening(false);
  const startPushToTalk = () => startListening(true);
  function stopTalking() {
    setPttActive(false);
    bridge.current?.trace('stt_stop_manual');
    listeningActive.current = false;
    gatewayMic.current?.stop();
    gatewayMic.current?.finalize();
    setSttStatus('PTT released; finalizing transcript');
  }
  function interrupt() {
    bridge.current?.trace('ui_interrupt');
    void sfx.play('radio_cancel');
    activeTurnId.current = null;
    player.current?.stop();
    gatewayMic.current?.stop();
    bridge.current?.interrupt();
    listeningActive.current = false;
    setPttActive(false);
    setPhase('interrupted');
  }
  function newCall() {
    bridge.current?.trace('ui_new_call');
    activeTurnId.current = null;
    setLiveCaption('');
    setPttActive(false);
    bridge.current?.trace('stt_stop_new_call');
    listeningActive.current = false;
    gatewayMic.current?.stop();
    player.current?.stop();
    bridge.current?.newSession();
    assistantText.current = '';
    assistantLineIndex.current = null;
    setAssistantResponse(autoMicEnabled ? 'Mic auto-starting...' : 'Awaiting frequency activation.');
    setPhase('idle');
    setTranscript([{ speaker: 'System', text: `Fresh ${displaySpeakerName(supportCharacterRef.current, charactersRef.current)} session requested.`, at: now() }]);
    if (autoMicEnabled) void startListening(false);
  }
  function testCall() {
    const text = `${displaySpeakerName(supportCharacterRef.current, charactersRef.current)}, can you hear me?`;
    setTranscript((t) => [...t, { speaker: 'Snake', text, at: now() }]);
    bridge.current?.trace('ui_test_call', { text });
    bridge.current?.sendUtterance(text);
  }
  function toggleStatus() { setShowStatus((x) => !x); }
  function switchCharacter(character: string) {
    activeTurnId.current = null;
    const wasListening = phaseRef.current === 'listening';
    autoResumeListeningAfterSwitch.current = wasListening || autoMicEnabled;
    bridge.current?.trace('stt_stop_switch_character', { character, wasListening });
    listeningActive.current = false;
    gatewayMic.current?.stop();
    setPttActive(false);
    setLiveCaption('');
    player.current?.stop();
    bridge.current?.switchCharacter(character);
  }

  return { phase, transcript, assistantResponse, supportCharacter, characters, snakeLevel, campbellLevel, error, connected, liveCaption, sttStatus, llmModel, pttActive, showStatus, activateColonel, startPushToTalk, stopTalking, interrupt, newCall, testCall, toggleStatus, switchCharacter };
}

function normalizeMarkdownForDisplay(text: string) {
  let out = text.replace(/\r\n?/g, '\n');

  out = out
    .replace(/<\|channel\|>\s*thought\s*<\|channel\|>/gi, '')
    .replace(/<\|channel\|>\s*thought/gi, '')
    .replace(/<channel\|>\s*thought/gi, '')
    .replace(/<\|?channel\|?>/gi, '')
    .replace(/\bthought(?:thought)+\b/gi, '');

  out = out
    .split('\n')
    .map((line) => {
      if (/^\s*\|.*\|\s*$/.test(line)) return line;
      return line
        .replace(/:\s*(?=\*{1,3}\s*[A-Z])/g, ':\n\n')
        .replace(/\*{3}\s*(?=[A-Z])/g, '\n\n- ')
        .replace(/\*{2}\s*(?=[A-Z])/g, '\n- ')
        .replace(/\s-\s(?=[A-Z])/g, '\n- ')
        .replace(/([.!?])\s*(?=-\s|\*{1,3}\s*[A-Z])/g, '$1\n\n');
    })
    .join('\n');

  // Keep markdown legible without collapsing intentional breaks.
  out = out
    .replace(/\n{3,}/g, '\n\n')
    .replace(/[ \t]{2,}/g, ' ')
    .trim();

  return out;
}

function MarkdownText({ text }: { text: string }) {
  const normalized = normalizeMarkdownForDisplay(text);
  return <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
    a: ({ children }) => <>{children}</>,
    img: ({ alt }) => <>{alt || ''}</>,
  }}>{normalized}</ReactMarkdown>;
}

function Portrait({ side, set, level, active, tick }: { side: 'left' | 'right'; set: FaceSet; level: number; active: boolean; tick: number }) {
  return <aside className={`portraitPane ${side} ${active ? 'active' : ''}`}><span className="faceFrame"><img className="face" src={faceFor(set, level, active, tick)} /></span></aside>;
}

function SignalMeter({ level }: { level: number }) {
  const rows = [
    'M943.95,231.55h703.88v31.7H943.95z',
    'M943.95,275.42v31.7h210.49c31.42-18.71,72.74-27.06,143.2-31.7H943.95z',
    'M943.95,319.29h193.15c-9.11,7.74-17.16,18.63-24.28,31.7H943.95z',
    'M943.95,363.15h162.79c-4.5,9.78-8.57,20.45-12.27,31.7H943.95z',
    'M943.95,407.02h146.73c-3.03,10.27-5.77,20.9-8.25,31.7H943.95z',
    'M943.95,450.88h135.8c-2.22,10.55-4.21,21.18-5.98,31.7H943.95z',
    'M943.95,494.75h127.86c-1.68,10.84-3.15,21.48-4.42,31.7H943.95z',
    'M943.95,538.62h121.99c-1.3,11.35-2.35,22.03-3.21,31.7H943.95z',
    'M943.95,582.48h117.76c-1.11,14-1.76,25.01-2.12,31.7H943.95z',
  ];
  const litFrom = Math.max(0, Math.floor((1 - level) * rows.length));
  return <svg className="meter" viewBox="880 170 790 455" aria-label="PTT signal meter">
    <text x="943" y="213" className="meterPtt">PTT</text>
    <text x="1588" y="306" className="meterMax">max</text>
    {rows.map((d, i) => <g key={d} className={i >= litFrom ? 'lit' : 'dim'}><rect x="912.76" y={231.55 + i * 43.865} width="16.09" height="31.7"/><path d={d}/></g>)}
  </svg>;
}

function LevelBar({ label, level, cells = 9 }: { label: string; level: number; cells?: number }) {
  const lit = Math.min(cells, Math.round(Math.min(1, Math.max(0, level)) * cells));
  return <span className="statusLevel" role="img" aria-label={`${label} level ${Math.round(Math.min(1, Math.max(0, level)) * 100)}%`}>
    <span className="statusLabel">{label}</span>
    <span className="levelBar">{Array.from({ length: cells }, (_, i) => <span key={i} className={`cell ${i < lit ? 'lit' : ''}`} />)}</span>
  </span>;
}

function speakerClass(speaker: string) {
  return speaker.toLowerCase().replace(/[^a-z0-9]+/g, '_');
}

function speakerTag(line: TranscriptLine) {
  return line.speaker.toUpperCase();
}

function transcriptToVerbatimText(lines: TranscriptLine[]) {
  return lines
    .map((line) => `${speakerTag(line)}\n${line.text}`)
    .join('\n\n');
}

function App() {
  const codec = useCodecDemo();
  const [tick, setTick] = useState(0);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [memoryActiveIndex, setMemoryActiveIndex] = useState(0);
  const [transcriptInsets, setTranscriptInsets] = useState({ left: 0, right: 0 });
  const [tuneStripInsets, setTuneStripInsets] = useState({ left: 0, right: 0 });
  const [copyNotice, setCopyNotice] = useState('');
  const codecRef = useRef<HTMLElement | null>(null);
  const logRef = useRef<HTMLDivElement | null>(null);
  const memoryOptionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  useEffect(() => { const t = window.setInterval(() => setTick((x) => x + 1), 50); return () => window.clearInterval(t); }, []);
  useEffect(() => {
    const log = logRef.current;
    if (!log) return;
    log.scrollTop = log.scrollHeight;
  }, [codec.transcript, codec.liveCaption, codec.assistantResponse]);

  useEffect(() => {
    const updateInsets = () => {
      const codecEl = codecRef.current;
      if (!codecEl) return;
      const leftAvatar = codecEl.querySelector<HTMLImageElement>('.portraitPane.left .face');
      const rightAvatar = codecEl.querySelector<HTMLImageElement>('.portraitPane.right .face');
      if (!leftAvatar || !rightAvatar) return;
      const codecRect = codecEl.getBoundingClientRect();
      const leftRect = leftAvatar.getBoundingClientRect();
      const rightRect = rightAvatar.getBoundingClientRect();
      const left = Math.max(0, Math.round(leftRect.left - codecRect.left));
      const right = Math.max(0, Math.round(codecRect.right - rightRect.right));
      setTranscriptInsets({ left, right });

      const controlPane = codecEl.querySelector<HTMLElement>('.controlPane');
      const meter = codecEl.querySelector<SVGElement>('.meter');
      const frequency = codecEl.querySelector<HTMLElement>('.frequency');
      if (controlPane && meter && frequency) {
        const paneRect = controlPane.getBoundingClientRect();
        const meterRect = meter.getBoundingClientRect();
        const freqRect = frequency.getBoundingClientRect();
        const tuneLeft = Math.max(0, Math.round(meterRect.left - paneRect.left));
        const tuneRight = Math.max(0, Math.round(paneRect.right - freqRect.right));
        setTuneStripInsets({ left: tuneLeft, right: tuneRight });
      }
    };

    updateInsets();
    const onResize = () => updateInsets();
    window.addEventListener('resize', onResize);

    const codecEl = codecRef.current;
    const avatars = codecEl ? Array.from(codecEl.querySelectorAll<HTMLImageElement>('.portraitPane .face')) : [];
    avatars.forEach((img) => img.addEventListener('load', onResize));

    const raf1 = window.requestAnimationFrame(updateInsets);
    const raf2 = window.requestAnimationFrame(() => window.requestAnimationFrame(updateInsets));

    return () => {
      window.cancelAnimationFrame(raf1);
      window.cancelAnimationFrame(raf2);
      avatars.forEach((img) => img.removeEventListener('load', onResize));
      window.removeEventListener('resize', onResize);
    };
  }, [codec.characters.length, codec.supportCharacter]);
  const status = useMemo(() => codec.error ?? codec.phase.toUpperCase(), [codec.phase, codec.error]);
  const dialogLines = useMemo(() => codec.transcript.filter((line) => line.speaker.toLowerCase() !== 'system'), [codec.transcript]);
  const systemNotice = useMemo(() => {
    if (copyNotice) return copyNotice;
    for (let i = codec.transcript.length - 1; i >= 0; i -= 1) {
      const line = codec.transcript[i];
      if (line.speaker.toLowerCase() === 'system') return line.text;
    }
    return codec.sttStatus;
  }, [codec.transcript, codec.sttStatus, copyNotice]);

  const copyTranscript = async () => {
    const text = transcriptToVerbatimText(dialogLines);
    if (!text) {
      setCopyNotice('No dialogue transcript to copy.');
      return;
    }
    try {
      await navigator.clipboard.writeText(text);
      setCopyNotice(`Copied ${dialogLines.length} transcript ${dialogLines.length === 1 ? 'line' : 'lines'} verbatim.`);
    } catch (error) {
      setCopyNotice(`Copy failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  useEffect(() => {
    if (!memoryOpen || !codec.characters.length) return;
    const selectedIndex = Math.max(0, codec.characters.findIndex((c) => c.id === codec.supportCharacter));
    setMemoryActiveIndex(selectedIndex);
    window.requestAnimationFrame(() => {
      const option = memoryOptionRefs.current[selectedIndex];
      option?.focus();
      option?.scrollIntoView({ block: 'nearest' });
    });
  }, [memoryOpen, codec.characters, codec.supportCharacter]);

  const openMemory = () => {
    sfx.unlock();
    void sfx.play('radio_window_open');
    setMemoryOpen(true);
  };

  const closeMemory = () => {
    void sfx.play('radio_window_close');
    setMemoryOpen(false);
  };

  const selectMemoryCharacter = (index: number) => {
    const character = codec.characters[index];
    if (!character) return;
    void sfx.play('radio_select');
    codec.switchCharacter(character.id);
    setMemoryOpen(false);
  };

  const onMemoryListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!codec.characters.length) {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeMemory();
      }
      return;
    }

    let next = memoryActiveIndex;
    if (event.key === 'ArrowDown') next = Math.min(codec.characters.length - 1, memoryActiveIndex + 1);
    if (event.key === 'ArrowUp') next = Math.max(0, memoryActiveIndex - 1);
    if (event.key === 'Home') next = 0;
    if (event.key === 'End') next = codec.characters.length - 1;

    if (next !== memoryActiveIndex) {
      event.preventDefault();
      void sfx.play('radio_cursor');
      setMemoryActiveIndex(next);
      const option = memoryOptionRefs.current[next];
      option?.focus();
      option?.scrollIntoView({ block: 'nearest' });
      return;
    }

    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      selectMemoryCharacter(memoryActiveIndex);
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      closeMemory();
    }
  };

  return <main className="shell">
    <section className="codec" aria-label="MGS style codec overlay" ref={codecRef} style={{ ['--transcript-left' as string]: `${transcriptInsets.left}px`, ['--transcript-right' as string]: `${transcriptInsets.right}px`, ['--tune-left' as string]: `${tuneStripInsets.left}px`, ['--tune-right' as string]: `${tuneStripInsets.right}px` } as React.CSSProperties}>
      <div className="topFrame">
        <Portrait side="left" set={supportFaceFor(codec.supportCharacter, codec.characters)} level={codec.campbellLevel} active={codec.phase === 'speaking'} tick={tick} />
        <section className="controlPane">
          <div className="frequencyPane"><SignalMeter level={Math.max(codec.snakeLevel, codec.campbellLevel, 0.2)} /><button className="frequency" onClick={codec.activateColonel}>{codec.characters.find((c) => c.id === codec.supportCharacter)?.frequency || '140.85'}</button></div>
          <div className="tuneStrip"><button onClick={codec.newCall}>new call</button><button onClick={openMemory}>memory</button><button onClick={codec.interrupt}>interrupt</button></div>
        </section>
        <Portrait side="right" set={snake} level={codec.snakeLevel} active={codec.phase === 'listening'} tick={tick} />
      </div>
      <section className="transcriptBox">
        <div className="log" ref={logRef}>
          {dialogLines.map((line, i) => (
            <div key={i} className={`line ${speakerClass(line.speaker)} speakerLine`}>
              <div className="speakerBadge">{speakerTag(line)}</div>
              <div className="textCol"><MarkdownText text={line.text} /></div>
            </div>
          ))}
          {codec.liveCaption && <div className="line liveCaption systemRow"><div className="textCol"><span>{codec.liveCaption}</span></div></div>}
        </div>
        {!codec.showStatus && <button className="statusToggle" onClick={codec.toggleStatus}>status</button>}
        {codec.showStatus && <div className="statusBar">
          <div className="statusGroup">
            <span className={`statusDot ${codec.connected ? 'on' : ''}`} />
            <span className="statusLabel">link</span>
            <span className="statusValue">{codec.connected ? 'online' : 'offline'}</span>
            <span className="statusLabel">phase</span>
            <span className="statusValue">{status.toLowerCase()}</span>
            <span className="statusLabel">model</span>
            <span className="statusValue" title={codec.llmModel || 'unknown'}>{codec.llmModel || 'unknown'}</span>
          </div>
          <div className="statusGroup">
            <LevelBar label="mic" level={codec.snakeLevel} />
            <LevelBar label="out" level={codec.campbellLevel} />
          </div>
          <div className="statusGroup statusNotice"><span className="noticeText">{systemNotice}</span></div>
          <div className="statusGroup statusActions">
            <button className={codec.pttActive ? 'armed' : ''} onClick={codec.startPushToTalk}>{codec.pttActive ? 'recording' : 'ptt'}</button>
            <button onClick={codec.stopTalking}>send</button>
            <button onClick={codec.testCall}>test</button>
            <button onClick={copyTranscript}>copy transcript verbatim</button>
            <button onClick={codec.toggleStatus}>hide</button>
          </div>
        </div>}
      </section>
    </section>
    {memoryOpen && <section className="memoryOverlay" role="dialog" aria-modal="true" aria-label="Codec memory">
      <div className="memoryModal">
        <header><h2>Memory</h2><button onClick={closeMemory}>close</button></header>
        <div className="memoryList" role="listbox" aria-label="Codec memory list" onKeyDown={onMemoryListKeyDown}>
          {codec.characters.map((c, index) => <button
            key={c.id}
            ref={(node) => {
              memoryOptionRefs.current[index] = node;
            }}
            role="option"
            aria-selected={index === memoryActiveIndex}
            className={`memoryRow ${c.id === codec.supportCharacter ? 'selected' : ''} ${index === memoryActiveIndex ? 'active' : ''}`}
            onFocus={() => setMemoryActiveIndex(index)}
            onClick={() => selectMemoryCharacter(index)}
          >
            <span className="memoryLine">
              <span className="memoryName">{c.displayName}</span>
              <span className="memoryMeta">{c.frequency || 'codec'}</span>
            </span>
          </button>)}
          {!codec.characters.length && <p>No codec memories received from bridge.</p>}
        </div>
      </div>
    </section>}
  </main>;
}

createRoot(document.getElementById('root')!).render(<App />);
