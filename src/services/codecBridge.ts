export type CodecPhase = 'idle' | 'connecting' | 'listening' | 'thinking' | 'speaking' | 'interrupted' | 'error';
export type CodecCharacterInfo = { id: string; displayName: string; speakerName: string; frequency?: string; avatar?: string; enabled?: boolean };

export type BridgeEvent =
  | { type: 'ready'; character: string; characters?: CodecCharacterInfo[] }
  | { type: 'character_switched'; character: string; characters?: CodecCharacterInfo[] }
  | { type: 'phase'; phase: CodecPhase }
  | { type: 'assistant_delta'; delta: string; turnId: string }
  | { type: 'user_transcript'; text: string; final: boolean; confidence?: number }
  | { type: 'sentence'; text: string; turnId: string }
  | { type: 'audio_chunk'; chunk: string; text: string; index: number; sample_rate: number; turnId: string }
  | { type: 'audio_pcm'; chunk: string; text: string; index: number; sample_rate: number; turnId: string }
  | { type: 'turn_started' | 'turn_completed'; turnId: string; character?: string }
  | { type: 'audio_reset'; reason?: string }
  | { type: 'session'; sessionFile?: string; sessionId?: string; sessionName?: string }
  | { type: 'disconnected' }
  | { type: 'error'; message: string };

// Same-origin URL (proxied by Vite, see vite.config.ts) so the app works when
// opened from another device, and upgrades to wss:// under HTTPS automatically.
export function defaultWsUrl(path: string) {
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
  return `${proto}://${window.location.host}${path}`;
}

export class StreamingAudioPlayer {
  private ctx?: AudioContext;
  private analyser?: AnalyserNode;
  private queue: AudioBuffer[] = [];
  private playing = false;
  private current?: AudioBufferSourceNode;
  private scheduled = new Set<AudioBufferSourceNode>();
  private nextPcmStartAt = 0;
  private readonly pcmLookaheadSeconds = Number(import.meta.env.VITE_CODEC_PCM_LOOKAHEAD_SECONDS || 0.75);
  onPlaying?: () => void;
  onStopped?: () => void;
  onDebug?: (event: string, data?: Record<string, unknown>) => void;

  private async ensureContext() {
    if (!this.ctx) {
      this.ctx = new AudioContext();
      this.analyser = this.ctx.createAnalyser();
      this.analyser.fftSize = 256;
      this.analyser.smoothingTimeConstant = 0.45;
      this.analyser.connect(this.ctx.destination);
    }
    if (this.ctx.state === 'suspended') await this.ctx.resume();
  }

  // iOS/Android autoplay policies only allow creating/resuming an AudioContext
  // inside a user gesture; call this synchronously from a tap handler.
  unlock() {
    void this.ensureContext().catch(() => {});
  }

  async enqueueBase64Wav(base64: string) {
    await this.ensureContext();
    const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    const buffer = await this.ctx!.decodeAudioData(bytes.buffer.slice(0));
    this.queue.push(buffer);
    this.onDebug?.('wav_enqueued', { duration: buffer.duration, queuedBuffers: this.queue.length });
    if (!this.playing) this.playNext();
  }

  async enqueuePcm16(base64: string, sampleRate: number) {
    await this.ensureContext();
    const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const samples = bytes.byteLength / 2;
    if (samples <= 0) return;
    const buffer = this.ctx!.createBuffer(1, samples, sampleRate);
    const channel = buffer.getChannelData(0);
    for (let i = 0; i < samples; i++) channel[i] = view.getInt16(i * 2, true) / 32768;
    this.schedulePcm(buffer);
  }

  private schedulePcm(buffer: AudioBuffer) {
    if (!this.ctx || !this.analyser) return;
    const source = this.ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(this.analyser);
    const startAt = Math.max(this.ctx.currentTime + this.pcmLookaheadSeconds, this.nextPcmStartAt || 0);
    this.nextPcmStartAt = startAt + buffer.duration;
    const bufferAhead = Math.max(0, this.nextPcmStartAt - this.ctx.currentTime);
    this.scheduled.add(source);
    this.playing = true;
    this.onPlaying?.();
    this.onDebug?.('pcm_scheduled', { duration: buffer.duration, startIn: startAt - this.ctx.currentTime, bufferAhead, scheduledSources: this.scheduled.size });
    source.onended = () => {
      this.scheduled.delete(source);
      this.onDebug?.('pcm_ended', { scheduledSources: this.scheduled.size, bufferAhead: Math.max(0, this.nextPcmStartAt - this.ctx!.currentTime) });
      if (this.scheduled.size === 0 && this.queue.length === 0 && !this.current) {
        this.playing = false;
        this.nextPcmStartAt = 0;
        this.onDebug?.('playback_stopped');
        this.onStopped?.();
      }
    };
    source.start(startAt);
  }

  private playNext() {
    if (!this.ctx || !this.analyser || this.queue.length === 0) {
      if (this.scheduled.size === 0) {
        this.playing = false;
        this.current = undefined;
        this.nextPcmStartAt = 0;
        this.onStopped?.();
      }
      return;
    }
    this.playing = true;
    this.onPlaying?.();
    this.onDebug?.('wav_play_start', { queuedBuffers: this.queue.length });
    const source = this.ctx.createBufferSource();
    source.buffer = this.queue.shift()!;
    source.connect(this.analyser);
    source.onended = () => { if (this.current === source) this.current = undefined; this.playNext(); };
    this.current = source;
    source.start();
  }

  stop() {
    this.onDebug?.('playback_stop_requested', { queuedBuffers: this.queue.length, scheduledSources: this.scheduled.size, bufferAhead: this.ctx ? Math.max(0, this.nextPcmStartAt - this.ctx.currentTime) : 0 });
    this.queue = [];
    try { this.current?.stop(); } catch {}
    for (const source of this.scheduled) { try { source.stop(); } catch {} }
    this.scheduled.clear();
    this.current = undefined;
    this.nextPcmStartAt = 0;
    this.playing = false;
    this.onStopped?.();
  }

  getLevel() {
    if (!this.analyser || !this.playing) return 0;
    const data = new Uint8Array(this.analyser.fftSize);
    this.analyser.getByteTimeDomainData(data);
    let sum = 0;
    for (const v of data) { const n = (v - 128) / 128; sum += n * n; }
    return Math.min(1, Math.pow(Math.max(0, Math.sqrt(sum / data.length) - 0.01) * 12, 0.7));
  }
}

export class CodecBridgeClient {
  private ws?: WebSocket;
  private handlers = new Set<(event: BridgeEvent) => void>();
  private reconnectTimer?: number;
  private reconnectDelayMs = 1000;
  constructor(private url = import.meta.env.VITE_CODEC_BRIDGE_URL || defaultWsUrl('/ws/bridge')) {
    // Mobile browsers drop WebSockets on tab backgrounding and network switches;
    // reconnect as soon as the app is usable again instead of waiting out backoff.
    window.addEventListener('online', () => this.connect());
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') this.connect();
    });
  }
  connect() {
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) return;
    window.clearTimeout(this.reconnectTimer);
    const ws = new WebSocket(this.url);
    this.ws = ws;
    ws.onopen = () => { this.reconnectDelayMs = 1000; };
    ws.onmessage = (e) => {
      const event = JSON.parse(String(e.data)) as BridgeEvent;
      for (const h of this.handlers) h(event);
    };
    ws.onerror = () => this.emit({ type: 'error', message: `Could not connect to codec bridge at ${this.url}` });
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.emit({ type: 'disconnected' });
      this.reconnectTimer = window.setTimeout(() => this.connect(), this.reconnectDelayMs);
      this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, 15000);
    };
  }
  private emit(event: BridgeEvent) { for (const h of this.handlers) h(event); }
  onEvent(handler: (event: BridgeEvent) => void) { this.handlers.add(handler); return () => this.handlers.delete(handler); }
  sendUtterance(text: string) {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify({ type: 'user_utterance', speaker: 'snake', text }));
    return true;
  }
  interrupt() { this.ws?.send(JSON.stringify({ type: 'interrupt' })); }
  newSession() { this.ws?.send(JSON.stringify({ type: 'new_session' })); }
  switchCharacter(character: string) { this.ws?.send(JSON.stringify({ type: 'switch_character', character })); }
  trace(event: string, data?: Record<string, unknown>) { this.ws?.send(JSON.stringify({ type: 'client_trace', event, data })); }
}

type GatewayServerEvent =
  | { type: 'hello'; protocol_version: number; binary_audio: boolean }
  | { type: 'session_started'; session_id: string }
  | { type: 'session_ended' }
  | { type: 'phase'; phase: CodecPhase }
  | { type: 'turn_started'; turn_id: string; character?: string; persona?: string }
  | { type: 'assistant_delta'; turn_id: string; delta: string }
  | { type: 'user_transcript'; text: string; final: boolean; confidence?: number }
  | { type: 'turn_completed'; turn_id: string }
  | { type: 'audio_reset'; reason?: string }
  | { type: 'error'; code: string; message: string }
  | { type: 'trace'; event: string; data: Record<string, unknown> }
  | { type: 'avatar_action'; action: Record<string, unknown> }
  | { type: 'frontend_tools_negotiated'; tools: string[] }
  | { type: 'frontend_tool_call'; name: string; arguments: Record<string, unknown> }
  | { type: 'frontend_tool_rejected'; name: string; reason: string };

export class RustVoiceGatewayClient {
  private ws?: WebSocket;
  private handlers = new Set<(event: BridgeEvent) => void>();
  private sessionId = '';
  private reconnectTimer?: number;
  private reconnectDelayMs = 1000;
  private readonly agent = import.meta.env.VITE_FOXLINE_GATEWAY_AGENT || 'campbell';
  private readonly workspace = import.meta.env.VITE_FOXLINE_GATEWAY_WORKSPACE || `agents/${this.agent}`;
  private readonly loadout = import.meta.env.VITE_FOXLINE_GATEWAY_LOADOUT || 'default';
  private persona = import.meta.env.VITE_FOXLINE_GATEWAY_PERSONA || this.agent;

  constructor(private url = import.meta.env.VITE_FOXLINE_GATEWAY_URL || 'ws://127.0.0.1:8780') {
    window.addEventListener('online', () => this.connect());
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') this.connect();
    });
  }

  connect() {
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) return;
    window.clearTimeout(this.reconnectTimer);
    const ws = new WebSocket(this.url);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.onopen = () => {
      this.reconnectDelayMs = 1000;
      ws.send(JSON.stringify({
        type: 'hello',
        client: 'foxline-codec-ui',
        debug_traces: false,
        capabilities: {
          protocol_version: 1,
          audio: { input_pcm: true, output_pcm: true, sample_rates_hz: [24000] },
          tools: ['codec.display', 'codec.avatar'],
          avatar_actions: ['set_state', 'set_expression', 'focus', 'play_animation', 'clear'],
        },
      }));
      ws.send(JSON.stringify({
        type: 'start_session',
        agent: this.agent,
        persona: this.persona,
        workspace: this.workspace,
        loadout: this.loadout,
      }));
    };
    ws.onmessage = async (e) => {
      if (typeof e.data !== 'string') {
        const bytes = e.data instanceof ArrayBuffer ? new Uint8Array(e.data) : new Uint8Array(await (e.data as Blob).arrayBuffer());
        const chunk = uint8ToBase64(bytes);
        this.emit({ type: 'audio_pcm', turnId: this.sessionId || 'gateway', index: Date.now(), text: '', sample_rate: 24000, chunk });
        return;
      }
      const event = JSON.parse(e.data) as GatewayServerEvent;
      if (event.type === 'session_started') {
        this.sessionId = event.session_id;
        this.emit({ type: 'ready', character: this.persona, characters: [] });
        this.emit({ type: 'session', sessionId: event.session_id, sessionName: `gateway:${this.agent}:${this.persona}` });
      } else if (event.type === 'session_ended') {
        this.emit({ type: 'phase', phase: 'idle' });
      } else if (event.type === 'phase') {
        this.emit({ type: 'phase', phase: event.phase });
      } else if (event.type === 'turn_started') {
        this.emit({ type: 'turn_started', turnId: event.turn_id, character: event.persona || event.character });
      } else if (event.type === 'assistant_delta') {
        this.emit({ type: 'assistant_delta', turnId: event.turn_id, delta: event.delta });
      } else if (event.type === 'user_transcript') {
        this.emit({ type: 'user_transcript', text: event.text, final: event.final, confidence: event.confidence });
      } else if (event.type === 'turn_completed') {
        this.emit({ type: 'turn_completed', turnId: event.turn_id });
      } else if (event.type === 'audio_reset') {
        this.emit({ type: 'audio_reset', reason: event.reason });
      } else if (event.type === 'error') {
        this.emit({ type: 'error', message: `${event.code}: ${event.message}` });
      } else if (event.type === 'avatar_action') {
        this.trace('avatar_action_received', event.action);
      } else if (event.type === 'frontend_tools_negotiated') {
        this.trace('frontend_tools_negotiated', { tools: event.tools });
      } else if (event.type === 'frontend_tool_call') {
        this.ws?.send(JSON.stringify({
          type: 'frontend_tool_result',
          call_id: `${event.name}-${Date.now()}`,
          result: { ok: false, error: 'Codec UI tool execution is not implemented yet' },
        }));
      }
    };
    ws.onerror = () => this.emit({ type: 'error', message: `Could not connect to Rust Voice Gateway at ${this.url}` });
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.emit({ type: 'disconnected' });
      this.reconnectTimer = window.setTimeout(() => this.connect(), this.reconnectDelayMs);
      this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, 15000);
    };
  }

  private emit(event: BridgeEvent) { for (const h of this.handlers) h(event); }
  onEvent(handler: (event: BridgeEvent) => void) { this.handlers.add(handler); return () => this.handlers.delete(handler); }
  sendUtterance(_text: string) { return false; }
  sendAudioPcm16(pcm: ArrayBuffer) {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(pcm);
    return true;
  }
  interrupt() { this.ws?.send(JSON.stringify({ type: 'interrupt' })); }
  newSession() { this.restartSession(); }
  switchPersona(persona: string) {
    this.persona = persona;
    this.emit({ type: 'character_switched', character: persona, characters: [] });
    this.restartSession();
  }
  switchCharacter(character: string) { this.switchPersona(character); }
  trace(_event: string, _data?: Record<string, unknown>) {}

  private restartSession() {
    this.ws?.send(JSON.stringify({ type: 'end_session' }));
    this.ws?.close();
    this.ws = undefined;
    this.connect();
  }
}

function uint8ToBase64(bytes: Uint8Array) {
  let binary = '';
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}
