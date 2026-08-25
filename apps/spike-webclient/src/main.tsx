import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ThinkingOrb } from 'thinking-orbs';
import {
  MicrophonePcmStreamer,
  StreamingAudioPlayer,
  VoiceGatewayClient,
  type VoiceClientEvent,
} from '@foxline/voice-client';

// WS URL resolution:
//   ?ws=wss://host/path  explicit override
//   served over https    -> wss://<same host>/gateway   (tailscale serve route)
//   local dev            -> ws://127.0.0.1:8783
const params = new URLSearchParams(location.search);
const wsUrl =
  params.get('ws') ??
  (location.protocol === 'https:' ? `wss://${location.host}/gateway` : 'ws://127.0.0.1:8783');

const PHASE_ORB: Record<string, { state: Parameters<typeof ThinkingOrb>[0]['state']; label: string }> = {
  idle: { state: 'breathing', label: '' },
  connecting: { state: 'connecting', label: 'connecting…' },
  listening: { state: 'listening', label: 'listening' },
  thinking: { state: 'working', label: 'thinking' },
  speaking: { state: 'composing', label: 'speaking' },
  interrupted: { state: 'solving', label: 'interrupted' },
  error: { state: 'shaping', label: 'error' },
};

function App() {
  const clientRef = useRef<VoiceGatewayClient | null>(null);
  const playerRef = useRef<StreamingAudioPlayer | null>(null);
  const micRef = useRef<MicrophonePcmStreamer | null>(null);
  const [started, setStarted] = useState(false);
  const [phase, setPhase] = useState('connecting');
  const [lines, setLines] = useState<{ who: string; text: string }[]>([]);
  const [input, setInput] = useState('');
  const transcriptRef = useRef<HTMLDivElement>(null);

  useEffect(() => { transcriptRef.current?.scrollTo(0, 1e6); }, [lines]);
  const say = (who: string, text: string) => setLines((l) => [...l, { who, text }]);
  const assistantBuf = useRef('');

  useEffect(() => {
    if (!started) return;

    const player = new StreamingAudioPlayer();
    player.onPlaying = () => setPhase('speaking');
    player.onStopped = () => setPhase('listening');
    playerRef.current = player;

    const client = new VoiceGatewayClient({
      url: wsUrl,
      clientId: 'govnr-web-spike',
      agent: 'campbell',
      workspaceOverride: '/Users/tommyfalkowski/byteowlz/govnr',
      loadout: 'default',
      token: new URLSearchParams(location.search).get('token') ?? '',
      tools: [],
      avatarActions: [],
    });
    clientRef.current = client;

    client.onEvent((ev: VoiceClientEvent) => {
      switch (ev.type) {
        case 'ready': setPhase('idle'); break;
        case 'session': say('System', 'session ready'); break;
        case 'phase': setPhase(ev.phase); break;
        case 'user_transcript':
          if (ev.final) say('You', ev.text);
          break;
        case 'assistant_delta':
          assistantBuf.current += ev.delta;
          break;
        case 'turn_completed':
          if (assistantBuf.current.trim()) say('Govnr', assistantBuf.current.trim());
          assistantBuf.current = '';
          break;
        case 'audio_pcm': void player.enqueueBase64Wav(ev.chunk); break;
        case 'error': say('System', `error: ${ev.message}`); break;
      }
    });

    client.connect();

    const mic = new MicrophonePcmStreamer();
    mic.onAudioFrame((frame) => client.sendAudioPcm16(frame.pcm16));
    mic.start().catch((e) => say('System', `mic failed: ${String(e)} — HTTPS required for microphone`));
    micRef.current = mic;

    return () => {
      mic.stop();
      client.disconnect();
      player.stop();
    };
  }, [started]);

  if (!started) {
    return (
      <>
        <div id="orbWrap"><ThinkingOrb state="breathing" size={128} theme="dark" /></div>
        <button style={{ marginTop: 24 }} onClick={() => setStarted(true)}>Start voice session</button>
        <div id="micHint">connects to the foxline gateway over the tailnet</div>
      </>
    );
  }

  const orb = PHASE_ORB[phase] ?? PHASE_ORB.idle;
  return (
    <>
      <div id="orbWrap"><ThinkingOrb state={orb.state} size={160} theme="dark" /></div>
      <div id="phaseLabel">{orb.label}</div>
      <div id="transcript" ref={transcriptRef}>
        {lines.map((l, i) => (
          <div key={i} className={`line ${l.who.toLowerCase()}`}><b>{l.who}:</b> {l.text}</div>
        ))}
      </div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!input.trim() || !clientRef.current) return;
          say('You', input.trim());
          clientRef.current.sendUtterance(input.trim());
          setInput('');
        }}
      >
        <input value={input} onChange={(e) => setInput(e.target.value)} placeholder="Type instead…" />
        <button type="submit">Send</button>
        <button type="button" className="secondary" onClick={() => clientRef.current?.interrupt()}>Stop</button>
      </form>
      <div id="micHint">voice always-on · speak, pause, it sends</div>
    </>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
