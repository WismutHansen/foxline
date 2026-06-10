// MGS codec UI sound effects, rendered from the user's game disc by
// tools/render_mgs_sfx.py during asset install. If the assets have not been
// installed the glob is empty and every play() is a silent no-op.

// Only the sounds the UI actually plays are globbed so vite does not copy the
// whole rendered SFX library into dist. PSX driver-sim renders are preferred;
// the mgs_pc set is the lower-fidelity fallback converted from the PC port's
// pre-rendered effects (see PC_SFX_FALLBACK in tools/install_user_mgs_assets.py).
const pcSfxModules = import.meta.glob<string>(
  '../../assets/generated/mgs_pc/sfx/builtin/*_{codec_call,codec_tune,codec_noise,radio_receive,radio_select,radio_cancel,radio_cursor,radio_window_open,radio_window_close,alert_bikkuri,cursor,item_select}.wav',
  { eager: true, import: 'default' },
);
const psxSfxModules = import.meta.glob<string>(
  '../../assets/generated/mgs/disc_1/sfx/builtin/*_{codec_call,codec_tune,codec_noise,radio_receive,radio_select,radio_cancel,radio_cursor,radio_window_open,radio_window_close,alert_bikkuri,cursor,item_select}.wav',
  { eager: true, import: 'default' },
);

export type SfxName =
  | 'codec_call'
  | 'codec_tune'
  | 'codec_noise'
  | 'radio_receive'
  | 'radio_select'
  | 'radio_cancel'
  | 'radio_cursor'
  | 'radio_window_open'
  | 'radio_window_close'
  | 'alert_bikkuri'
  | 'cursor'
  | 'item_select';

const TARGET_PEAK = 0.4;
const MAX_NORMALIZE_GAIN = 10;

class SfxPlayer {
  private ctx?: AudioContext;
  private urls = new Map<string, string>();
  private buffers = new Map<string, AudioBuffer>();
  private gains = new Map<string, number>();
  private active = new Map<string, AudioBufferSourceNode>();

  constructor() {
    // PC fallback first, then PSX renders override name-by-name.
    for (const modules of [pcSfxModules, psxSfxModules]) {
      for (const [modulePath, url] of Object.entries(modules)) {
        const m = modulePath.match(/se\d+_(.+?)(?:-[A-Za-z0-9_-]+)?\.wav$/);
        if (m) this.urls.set(m[1], url);
      }
    }
  }

  get available(): boolean {
    return this.urls.size > 0;
  }

  /** Must be called from a user gesture before the first play() on mobile. */
  unlock() {
    if (!this.urls.size) return;
    if (!this.ctx) this.ctx = new AudioContext();
    if (this.ctx.state === 'suspended') void this.ctx.resume();
  }

  private async buffer(name: string): Promise<AudioBuffer | undefined> {
    if (!this.ctx) return undefined;
    const cached = this.buffers.get(name);
    if (cached) return cached;
    const url = this.urls.get(name);
    if (!url) return undefined;
    try {
      const res = await fetch(url);
      const buf = await this.ctx.decodeAudioData(await res.arrayBuffer());
      let peak = 0;
      for (let ch = 0; ch < buf.numberOfChannels; ch += 1) {
        const data = buf.getChannelData(ch);
        for (let i = 0; i < data.length; i += 1) {
          const v = Math.abs(data[i]);
          if (v > peak) peak = v;
        }
      }
      this.gains.set(name, peak > 0 ? Math.min(TARGET_PEAK / peak, MAX_NORMALIZE_GAIN) : 1);
      this.buffers.set(name, buf);
      return buf;
    } catch {
      return undefined;
    }
  }

  async play(name: SfxName, opts: { gain?: number; loop?: boolean } = {}) {
    if (!this.ctx || this.ctx.state !== 'running') return;
    const buf = await this.buffer(name);
    if (!buf) return;
    this.stop(name);
    const src = this.ctx.createBufferSource();
    src.buffer = buf;
    src.loop = !!opts.loop;
    const gain = this.ctx.createGain();
    gain.gain.value = (this.gains.get(name) ?? 1) * (opts.gain ?? 1);
    src.connect(gain).connect(this.ctx.destination);
    src.onended = () => {
      if (this.active.get(name) === src) this.active.delete(name);
    };
    src.start();
    this.active.set(name, src);
  }

  stop(name: SfxName) {
    const src = this.active.get(name);
    if (src) {
      try { src.stop(); } catch { /* already stopped */ }
      this.active.delete(name);
    }
  }
}

export const sfx = new SfxPlayer();
