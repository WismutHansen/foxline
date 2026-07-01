import type { ClientControl, ServerEvent } from '@foxline/protocol';

export type VoicePhase = 'idle' | 'connecting' | 'listening' | 'thinking' | 'speaking' | 'interrupted' | 'error';

export type VoiceCharacterInfo = {
  id: string;
  displayName: string;
  speakerName: string;
  frequency?: string;
  avatar?: string;
  enabled?: boolean;
};

export type VoiceClientEvent =
  | { type: 'ready'; character: string; characters?: VoiceCharacterInfo[]; model?: string | null }
  | { type: 'character_switched'; character: string; characters?: VoiceCharacterInfo[] }
  | { type: 'phase'; phase: VoicePhase }
  | { type: 'assistant_delta'; delta: string; turnId: string }
  | { type: 'user_transcript'; text: string; final: boolean; confidence?: number | null }
  | { type: 'sentence'; text: string; turnId: string }
  | { type: 'audio_chunk'; chunk: string; text: string; index: number; sample_rate: number; turnId: string }
  | { type: 'audio_pcm'; chunk: string; text: string; index: number; sample_rate: number; turnId: string }
  | { type: 'turn_started' | 'turn_completed'; turnId: string; character?: string | null }
  | { type: 'audio_reset'; reason?: string | null }
  | { type: 'session'; sessionFile?: string; sessionId?: string; sessionName?: string; model?: string | null }
  | { type: 'disconnected' }
  | { type: 'error'; message: string };

export type VoiceGatewayClientOptions = {
  url?: string;
  clientId: string;
  agent: string;
  workspaceOverride?: string;
  loadout?: string;
  persona?: string;
  characters?: VoiceCharacterInfo[];
  agentWorkspaceIds?: Set<string> | string[];
  debugTraces?: boolean;
  protocolVersion?: number;
  inputSampleRatesHz?: number[];
  tools?: string[];
  avatarActions?: string[];
  reconnectInitialDelayMs?: number;
  reconnectMaxDelayMs?: number;
  binaryAudioSampleRateHz?: number;
  unimplementedToolMessage?: string;
};

export class StreamingAudioPlayer {
  private ctx?: AudioContext;
  private analyser?: AnalyserNode;
  private queue: AudioBuffer[] = [];
  private playing = false;
  private current?: AudioBufferSourceNode;
  private scheduled = new Set<AudioBufferSourceNode>();
  private nextPcmStartAt = 0;
  private readonly pcmLookaheadSeconds: number;
  onPlaying?: () => void;
  onStopped?: () => void;
  onDebug?: (event: string, data?: Record<string, unknown>) => void;

  constructor(options: { pcmLookaheadSeconds?: number } = {}) {
    this.pcmLookaheadSeconds = options.pcmLookaheadSeconds ?? 0.75;
  }

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
    for (let i = 0; i < samples; i += 1) channel[i] = view.getInt16(i * 2, true) / 32768;
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
    source.onended = () => {
      if (this.current === source) this.current = undefined;
      this.playNext();
    };
    this.current = source;
    source.start();
  }

  stop() {
    this.onDebug?.('playback_stop_requested', { queuedBuffers: this.queue.length, scheduledSources: this.scheduled.size, bufferAhead: this.ctx ? Math.max(0, this.nextPcmStartAt - this.ctx.currentTime) : 0 });
    this.queue = [];
    try { this.current?.stop(); } catch {}
    for (const source of this.scheduled) {
      try { source.stop(); } catch {}
    }
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
    for (const v of data) {
      const n = (v - 128) / 128;
      sum += n * n;
    }
    return Math.min(1, Math.pow(Math.max(0, Math.sqrt(sum / data.length) - 0.01) * 12, 0.7));
  }
}

export type MicrophoneAudioFrame = {
  chunksSent: number;
  samples: number;
  level: number;
  pcm16: ArrayBuffer;
};

export class MicrophonePcmStreamer {
  private audioContext?: AudioContext;
  private stream?: MediaStream;
  private worklet?: AudioWorkletNode;
  private source?: MediaStreamAudioSourceNode;
  private analyser?: AnalyserNode;
  private listening = false;
  private pendingSamples: number[] = [];
  private chunksSent = 0;
  private readonly sendChunkSamples: number;
  private readonly sampleRate: number;
  private onLevelCb?: (level: number) => void;
  private onAudioFrameCb?: (data: MicrophoneAudioFrame) => void;

  constructor(options: { sampleRate?: number; sendChunkSamples?: number } = {}) {
    this.sampleRate = options.sampleRate ?? 24000;
    this.sendChunkSamples = options.sendChunkSamples ?? 2048;
  }

  async start() {
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        sampleRate: this.sampleRate,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
    });
    this.audioContext = new AudioContext({ sampleRate: this.sampleRate });
    if (this.audioContext.state === 'suspended') await this.audioContext.resume();
    this.analyser = this.audioContext.createAnalyser();
    this.analyser.fftSize = 256;
    this.analyser.smoothingTimeConstant = 0.8;
    await this.audioContext.audioWorklet.addModule(
      URL.createObjectURL(
        new Blob(
          [
            `
        class AudioProcessor extends AudioWorkletProcessor {
          process(inputs) { const input = inputs[0]; if (input.length > 0) this.port.postMessage(input[0]); return true; }
        }
        registerProcessor('foxline-audio-processor', AudioProcessor);
      `,
          ],
          { type: 'application/javascript' },
        ),
      ),
    );
    this.source = this.audioContext.createMediaStreamSource(this.stream);
    this.worklet = new AudioWorkletNode(this.audioContext, 'foxline-audio-processor');
    this.listening = true;
    this.worklet.port.onmessage = (event) => {
      if (!this.listening) return;
      const samples = event.data as Float32Array;
      let sum = 0;
      for (const v of samples) sum += v * v;
      const level = Math.min(1, Math.sqrt(sum / samples.length) * 14);
      this.onLevelCb?.(level);
      this.pendingSamples.push(...samples);
      if (this.pendingSamples.length < this.sendChunkSamples) return;
      const out = new Float32Array(this.pendingSamples.splice(0, this.sendChunkSamples));
      this.chunksSent += 1;
      this.onAudioFrameCb?.({ chunksSent: this.chunksSent, samples: out.length, level, pcm16: float32ToPcm16(out) });
    };
    this.source.connect(this.analyser);
    this.analyser.connect(this.worklet);
    const silent = this.audioContext.createGain();
    silent.gain.value = 0.00001;
    this.worklet.connect(silent);
    silent.connect(this.audioContext.destination);
  }

  stop() {
    this.listening = false;
    this.pendingSamples = [];
    this.worklet?.disconnect();
    this.analyser?.disconnect();
    this.source?.disconnect();
    this.stream?.getTracks().forEach((track) => track.stop());
    if (this.audioContext?.state !== 'closed') void this.audioContext?.close();
  }

  getInputVolume(): number {
    if (!this.analyser || !this.listening) return 0;
    const data = new Uint8Array(this.analyser.frequencyBinCount);
    this.analyser.getByteFrequencyData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i += 1) sum += data[i];
    const volume = sum / data.length / 255;
    return volume < 0.01 ? 0 : volume;
  }

  onLevel(cb: (level: number) => void) {
    this.onLevelCb = cb;
  }

  onAudioFrame(cb: (data: MicrophoneAudioFrame) => void) {
    this.onAudioFrameCb = cb;
  }
}

export function float32ToPcm16(samples: Float32Array): ArrayBuffer {
  const buffer = new ArrayBuffer(samples.length * 2);
  const view = new DataView(buffer);
  for (let i = 0; i < samples.length; i += 1) {
    const sample = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(i * 2, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
  }
  return buffer;
}

export class VoiceGatewayClient {
  private ws?: WebSocket;
  private handlers = new Set<(event: VoiceClientEvent) => void>();
  private sessionId = '';
  private reconnectTimer?: number;
  private reconnectDelayMs: number;
  private agent: string;
  private readonly url: string;
  private readonly clientId: string;
  private readonly workspaceOverride: string;
  private readonly loadout: string;
  private persona: string;
  private readonly characters: VoiceCharacterInfo[];
  private readonly agentWorkspaceIds: Set<string>;
  private readonly debugTraces: boolean;
  private readonly protocolVersion: number;
  private readonly inputSampleRatesHz: number[];
  private readonly tools: string[];
  private readonly avatarActions: string[];
  private readonly reconnectInitialDelayMs: number;
  private readonly reconnectMaxDelayMs: number;
  private readonly binaryAudioSampleRateHz: number;
  private readonly unimplementedToolMessage: string;

  constructor(options: VoiceGatewayClientOptions) {
    this.url = options.url || 'ws://127.0.0.1:8780';
    this.clientId = options.clientId;
    this.agent = options.agent;
    this.workspaceOverride = options.workspaceOverride || '';
    this.loadout = options.loadout || 'default';
    this.persona = options.persona || this.agent;
    this.characters = options.characters || [];
    this.agentWorkspaceIds = new Set(options.agentWorkspaceIds || []);
    this.debugTraces = options.debugTraces ?? false;
    this.protocolVersion = options.protocolVersion ?? 1;
    this.inputSampleRatesHz = options.inputSampleRatesHz || [24000];
    this.tools = options.tools || [];
    this.avatarActions = options.avatarActions || [];
    this.reconnectInitialDelayMs = options.reconnectInitialDelayMs ?? 1000;
    this.reconnectMaxDelayMs = options.reconnectMaxDelayMs ?? 15000;
    this.reconnectDelayMs = this.reconnectInitialDelayMs;
    this.binaryAudioSampleRateHz = options.binaryAudioSampleRateHz ?? 24000;
    this.unimplementedToolMessage = options.unimplementedToolMessage || 'Frontend tool execution is not implemented yet';

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
      this.reconnectDelayMs = this.reconnectInitialDelayMs;
      this.sendControl({
        type: 'hello',
        client: this.clientId,
        debug_traces: this.debugTraces,
        capabilities: {
          protocol_version: this.protocolVersion,
          audio: { input_pcm: true, output_pcm: true, sample_rates_hz: this.inputSampleRatesHz },
          tools: this.tools,
          avatar_actions: this.avatarActions,
        },
      });
      this.sendControl({
        type: 'start_session',
        agent: this.agent,
        persona: this.persona,
        workspace: this.workspace(),
        loadout: this.loadout,
      });
    };
    ws.onmessage = async (e) => this.handleMessage(e);
    ws.onerror = () => this.emit({ type: 'error', message: `Could not connect to Rust Voice Gateway at ${this.url}` });
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.emit({ type: 'disconnected' });
      this.reconnectTimer = window.setTimeout(() => this.connect(), this.reconnectDelayMs);
      this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, this.reconnectMaxDelayMs);
    };
  }

  onEvent(handler: (event: VoiceClientEvent) => void) {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  disconnect() {
    window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    const ws = this.ws;
    this.ws = undefined;
    ws?.close();
  }

  sendUtterance(_text: string) {
    return false;
  }

  sendAudioPcm16(pcm: ArrayBuffer) {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(pcm);
    return true;
  }

  interrupt() {
    this.sendControl({ type: 'interrupt' });
  }

  newSession() {
    this.restartSession();
  }

  switchPersona(persona: string) {
    if (!this.characters.some((character) => character.id === persona)) return;
    this.persona = persona;
    this.emit({ type: 'character_switched', character: persona, characters: this.characters });
    this.restartSession();
  }

  switchCharacter(character: string) {
    if (!this.characters.some((entry) => entry.id === character)) return;
    if (!this.workspaceOverride && this.agentWorkspaceIds.has(character)) this.agent = character;
    this.switchPersona(character);
  }

  trace(_event: string, _data?: Record<string, unknown>) {}

  private async handleMessage(e: MessageEvent) {
    if (typeof e.data !== 'string') {
      const bytes = e.data instanceof ArrayBuffer ? new Uint8Array(e.data) : new Uint8Array(await (e.data as Blob).arrayBuffer());
      const chunk = uint8ToBase64(bytes);
      this.emit({ type: 'audio_pcm', turnId: this.sessionId || 'gateway', index: Date.now(), text: '', sample_rate: this.binaryAudioSampleRateHz, chunk });
      return;
    }

    const event = JSON.parse(e.data) as ServerEvent;
    if (event.type === 'session_started') {
      this.sessionId = event.session_id;
      this.emit({ type: 'ready', character: this.persona, characters: this.characters, model: event.model });
      this.emit({ type: 'session', sessionId: event.session_id, sessionName: `gateway:${this.agent}:${this.persona}`, model: event.model });
    } else if (event.type === 'session_ended') {
      this.emit({ type: 'phase', phase: 'idle' });
    } else if (event.type === 'phase') {
      this.emit({ type: 'phase', phase: event.phase as VoicePhase });
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
      this.trace('avatar_action_received', { action: event.action });
    } else if (event.type === 'frontend_tools_negotiated') {
      this.trace('frontend_tools_negotiated', { tools: event.tools });
    } else if (event.type === 'frontend_tool_call') {
      this.sendControl({
        type: 'frontend_tool_result',
        call_id: `${event.name}-${Date.now()}`,
        result: { ok: false, error: this.unimplementedToolMessage },
      });
    }
  }

  private emit(event: VoiceClientEvent) {
    for (const handler of this.handlers) handler(event);
  }

  private sendControl(control: ClientControl) {
    this.ws?.send(JSON.stringify(control));
  }

  private workspace() {
    return this.workspaceOverride || `agents/${this.agent}`;
  }

  private restartSession() {
    this.sendControl({ type: 'end_session' });
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
