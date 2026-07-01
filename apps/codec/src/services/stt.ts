export class MicrophonePcmStreamer {
  private audioContext?: AudioContext;
  private stream?: MediaStream;
  private worklet?: AudioWorkletNode;
  private source?: MediaStreamAudioSourceNode;
  private listening = false;
  private pendingSamples: number[] = [];
  private chunksSent = 0;
  private readonly sendChunkSamples = 2048;
  private onLevelCb?: (level: number) => void;
  private onStatusCb?: (status: string) => void;
  private onAudioFrameCb?: (data: { chunksSent: number; samples: number; level: number; mode: string; pcm16: ArrayBuffer }) => void;

  async start(manualMode = false) {
    this.stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, sampleRate: 24000, echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    this.audioContext = new AudioContext({ sampleRate: 24000 });
    if (this.audioContext.state === 'suspended') await this.audioContext.resume();
    await this.audioContext.audioWorklet.addModule(URL.createObjectURL(new Blob([`
      class AudioProcessor extends AudioWorkletProcessor {
        process(inputs) { const input = inputs[0]; if (input.length > 0) this.port.postMessage(input[0]); return true; }
      }
      registerProcessor('gateway-audio-processor', AudioProcessor);
    `], { type: 'application/javascript' })));
    this.source = this.audioContext.createMediaStreamSource(this.stream);
    this.worklet = new AudioWorkletNode(this.audioContext, 'gateway-audio-processor');
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
      const pcm16 = float32ToPcm16(out);
      this.chunksSent++;
      this.onAudioFrameCb?.({ chunksSent: this.chunksSent, samples: out.length, level, mode: manualMode ? 'PTT' : 'live', pcm16 });
    };
    this.source.connect(this.worklet);
    const silent = this.audioContext.createGain();
    silent.gain.value = 0.00001;
    this.worklet.connect(silent);
    silent.connect(this.audioContext.destination);
    this.onStatusCb?.(`${manualMode ? 'PTT' : 'live'} mic streaming to Rust Voice Gateway`);
  }

  stop() {
    this.listening = false;
    this.pendingSamples = [];
    this.worklet?.disconnect();
    this.source?.disconnect();
    this.stream?.getTracks().forEach((track) => track.stop());
    if (this.audioContext?.state !== 'closed') void this.audioContext?.close();
  }

  finalize() {}
  onLevel(cb: (level: number) => void) { this.onLevelCb = cb; }
  onStatus(cb: (status: string) => void) { this.onStatusCb = cb; }
  onAudioFrame(cb: (data: { chunksSent: number; samples: number; level: number; mode: string; pcm16: ArrayBuffer }) => void) { this.onAudioFrameCb = cb; }
}

function float32ToPcm16(samples: Float32Array) {
  const out = new ArrayBuffer(samples.length * 2);
  const view = new DataView(out);
  for (let i = 0; i < samples.length; i++) {
    const clamped = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(i * 2, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
  }
  return out;
}
