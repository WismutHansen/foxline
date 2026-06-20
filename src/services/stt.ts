export type SttMessage = { type: 'word' | 'final' | 'error' | 'status' | 'interim'; word?: string; text?: string; message?: string; decodeMs?: number; duration?: number };

export class StreamingSttService {
  private ws?: WebSocket;
  private audioContext?: AudioContext;
  private stream?: MediaStream;
  private worklet?: AudioWorkletNode;
  private source?: MediaStreamAudioSourceNode;
  private listening = false;
  private transcript = '';
  private vadTimer?: number;
  private finalTimer?: number;
  private onWordCb?: (word: string) => void;
  private onFinalCb?: (text: string) => void;
  private onErrorCb?: (error: string) => void;
  private onLevelCb?: (level: number) => void;
  private onStatusCb?: (status: string) => void;
  private onAudioFrameCb?: (data: { chunksSent: number; samples: number; level: number; mode: string }) => void;
  private chunksSent = 0;
  private manualMode = false;
  private pendingSamples: number[] = [];
  private readonly sendChunkSamples = 2048;
  private reconnectTimer?: number;
  private reconnecting = false;
  private connectPromise?: Promise<void>;
  private audioSilenceTimer?: number;
  private sawLocalSpeech = false;
  private awaitingServerFinal = false;

  constructor(private url = 'ws://127.0.0.1:8796/ws', private fallbackVadMs = 1800, private finalDebounceMs = 0, private label = 'parakeet-cpp', private sendStopOnStop = true) {}

  async connect() {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: 'getstatus' }));
      return;
    }
    if (this.connectPromise) {
      await this.connectPromise;
      return;
    }

    this.connectPromise = new Promise<void>((resolve, reject) => {
      this.ws = new WebSocket(this.url);
      this.ws.binaryType = 'arraybuffer';
      this.ws.onopen = () => {
        this.onStatusCb?.(`${this.label} connected ${this.url}`);
        this.ws?.send(JSON.stringify({ type: 'setlanguage', lang: 'en' }));
        this.ws?.send(JSON.stringify({ type: 'getstatus' }));
        this.connectPromise = undefined;
        resolve();
      };
      this.ws.onerror = () => {
        this.onStatusCb?.(`${this.label} websocket error`);
        this.connectPromise = undefined;
        reject(new Error(`Could not connect to ${this.label} at ${this.url}`));
      };
      this.ws.onclose = (event) => {
        this.onStatusCb?.(`${this.label} closed code=${event.code}`);
        this.connectPromise = undefined;
        if (this.listening) this.scheduleReconnect();
      };
      this.ws.onmessage = (event) => {
        this.onStatusCb?.(`${this.label} message ${String(event.data).slice(0, 80)}`);
        try {
          this.handleMessage(JSON.parse(String(event.data)) as SttMessage);
        } catch (e) {
          this.onStatusCb?.(`${this.label} parse error`);
          this.onErrorCb?.(e instanceof Error ? e.message : String(e));
        }
      };
    });

    await this.connectPromise;
  }

  private handleMessage(message: SttMessage) {
    if (message.type === 'word' && message.word) {
      window.clearTimeout(this.finalTimer);
      this.transcript += `${this.transcript ? ' ' : ''}${message.word}`;
      this.onStatusCb?.(`word: ${message.word}`);
      this.onWordCb?.(message.word);
      window.clearTimeout(this.vadTimer);
      if (!this.manualMode) this.vadTimer = window.setTimeout(() => this.flushTranscript(), this.fallbackVadMs);
    } else if (message.type === 'final') {
      window.clearTimeout(this.vadTimer);
      window.clearTimeout(this.finalTimer);
      const text = (message.text || this.transcript).trim();
      if (text) {
        this.transcript = text;
        this.onStatusCb?.(`server final: ${text}`);
        if (!this.manualMode) this.finalTimer = window.setTimeout(() => this.flushTranscript(), this.finalDebounceMs);
        else this.flushTranscript();
      }
    } else if (message.type === 'interim' && message.text) {
      this.onStatusCb?.(`interim: ${message.text}`);
    } else if (message.type === 'status') {
      this.onStatusCb?.(message.message || `${this.label} status`);
    } else if (message.type === 'error') {
      this.onErrorCb?.(message.message || `${this.label} error`);
    }
  }

  private flushTranscript() {
    const text = this.transcript.trim();
    this.transcript = '';
    if (text) { this.onStatusCb?.(`${this.manualMode ? 'manual' : 'vad'} final: ${text}`); this.onFinalCb?.(text); }
    else this.onStatusCb?.(`finalize: no transcript received from ${this.label} yet`);
  }

  async start(manualMode = true) {
    this.manualMode = manualMode;
    await this.connect();
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) throw new Error(`${this.label} WebSocket is not open`);
    this.onStatusCb?.('requesting microphone');
    this.stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, sampleRate: 24000, echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    const track = this.stream.getAudioTracks()[0];
    this.onStatusCb?.(`microphone granted: ${track?.label || 'unknown'}`);
    this.audioContext = new AudioContext({ sampleRate: 24000 });
    if (this.audioContext.state === 'suspended') await this.audioContext.resume();
    this.onStatusCb?.(`audio context ${this.audioContext.sampleRate}Hz ${this.audioContext.state}`);
    await this.audioContext.audioWorklet.addModule(URL.createObjectURL(new Blob([`
      class AudioProcessor extends AudioWorkletProcessor {
        process(inputs) { const input = inputs[0]; if (input.length > 0) this.port.postMessage(input[0]); return true; }
      }
      registerProcessor('audio-processor', AudioProcessor);
    `], { type: 'application/javascript' })));
    this.source = this.audioContext.createMediaStreamSource(this.stream);
    this.worklet = new AudioWorkletNode(this.audioContext, 'audio-processor');
    this.listening = true;
    this.worklet.port.onmessage = (event) => {
      if (!this.listening) return;
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        this.scheduleReconnect();
        return;
      }
      const samples = event.data as Float32Array;
      let sum = 0;
      for (const v of samples) sum += v * v;
      const level = Math.min(1, Math.sqrt(sum / samples.length) * 14);
      this.onLevelCb?.(level);

      if (!this.manualMode) {
        const speechThreshold = 0.06;
        if (level > speechThreshold) {
          this.sawLocalSpeech = true;
          this.awaitingServerFinal = false;
          window.clearTimeout(this.audioSilenceTimer);
          this.audioSilenceTimer = undefined;
        } else if (this.sawLocalSpeech && !this.awaitingServerFinal && !this.audioSilenceTimer) {
          this.audioSilenceTimer = window.setTimeout(() => {
            this.audioSilenceTimer = undefined;
            if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
            this.awaitingServerFinal = true;
            this.sawLocalSpeech = false;
            this.onStatusCb?.(`${this.label} local silence detected -> stop`);
            this.ws.send(JSON.stringify({ type: 'stop' }));
          }, this.fallbackVadMs);
        }
      }

      this.pendingSamples.push(...samples);
      if (this.pendingSamples.length < this.sendChunkSamples) return;
      const out = new Float32Array(this.pendingSamples.splice(0, this.sendChunkSamples));
      this.ws.send(out.buffer);
      this.chunksSent++;
      const mode = this.manualMode ? 'PTT' : 'live';
      this.onAudioFrameCb?.({ chunksSent: this.chunksSent, samples: out.length, level, mode });
      if (this.chunksSent === 1 || this.chunksSent % 20 === 0 || level > 0.08) this.onStatusCb?.(`${mode} ${this.label} stream: ${this.chunksSent} chunks level=${level.toFixed(2)}`);
    };
    this.source.connect(this.worklet);
    const silent = this.audioContext.createGain();
    silent.gain.value = 0.00001;
    this.worklet.connect(silent);
    silent.connect(this.audioContext.destination);
    this.onStatusCb?.(`${this.manualMode ? 'PTT' : 'live'} mic streaming to ${this.label}; final debounce ${this.finalDebounceMs}ms, fallback ${this.fallbackVadMs}ms`);
  }

  private scheduleReconnect() {
    if (!this.listening || this.reconnecting) return;
    if (this.reconnectTimer) return;
    this.reconnectTimer = window.setTimeout(async () => {
      this.reconnectTimer = undefined;
      if (!this.listening) return;
      this.reconnecting = true;
      try {
        this.onStatusCb?.(`${this.label} reconnecting...`);
        await this.connect();
        if (this.ws?.readyState === WebSocket.OPEN) this.onStatusCb?.(`${this.label} reconnected`);
      } catch (e) {
        this.onStatusCb?.(`${this.label} reconnect failed: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        this.reconnecting = false;
        if (this.listening && (!this.ws || this.ws.readyState !== WebSocket.OPEN)) this.scheduleReconnect();
      }
    }, 600);
  }

  stop() {
    this.listening = false;
    if (this.sendStopOnStop && this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify({ type: 'stop' }));
    window.clearTimeout(this.vadTimer);
    window.clearTimeout(this.finalTimer);
    window.clearTimeout(this.audioSilenceTimer);
    window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.reconnecting = false;
    this.connectPromise = undefined;
    this.pendingSamples = [];
    this.sawLocalSpeech = false;
    this.awaitingServerFinal = false;
    this.worklet?.disconnect();
    this.source?.disconnect();
    this.stream?.getTracks().forEach((t) => t.stop());
    if (this.audioContext?.state !== 'closed') void this.audioContext?.close();
  }

  finalize() { window.clearTimeout(this.vadTimer); window.clearTimeout(this.finalTimer); this.flushTranscript(); }
  disconnect() { this.stop(); this.ws?.close(); }
  onWord(cb: (word: string) => void) { this.onWordCb = cb; }
  onFinal(cb: (text: string) => void) { this.onFinalCb = cb; }
  onError(cb: (error: string) => void) { this.onErrorCb = cb; }
  onLevel(cb: (level: number) => void) { this.onLevelCb = cb; }
  onStatus(cb: (status: string) => void) { this.onStatusCb = cb; }
  onAudioFrame(cb: (data: { chunksSent: number; samples: number; level: number; mode: string }) => void) { this.onAudioFrameCb = cb; }
}
