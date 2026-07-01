import agentsManifest from '../../agents/manifest.json';
import { StreamingAudioPlayer, VoiceGatewayClient, type VoiceCharacterInfo, type VoiceClientEvent, type VoicePhase } from '@foxline/voice-client';

export { StreamingAudioPlayer };
export type CodecPhase = VoicePhase;
export type CodecCharacterInfo = VoiceCharacterInfo;
export type BridgeEvent = VoiceClientEvent;

const codecCharacters = (agentsManifest.characters as CodecCharacterInfo[]).filter((character) => character.enabled !== false);
const agentWorkspaceModules = import.meta.glob<string>('../../agents/*/SYSTEM.md', { eager: true, query: '?raw', import: 'default' });
const agentWorkspaceIds = new Set(
  Object.keys(agentWorkspaceModules)
    .map((path) => path.match(/\.\.\/\.\.\/agents\/([^/]+)\/SYSTEM\.md$/)?.[1])
    .filter(Boolean) as string[],
);

export class RustVoiceGatewayClient extends VoiceGatewayClient {
  constructor(url = import.meta.env.VITE_FOXLINE_GATEWAY_URL || 'ws://127.0.0.1:8780') {
    const agent = import.meta.env.VITE_FOXLINE_GATEWAY_AGENT || 'campbell';
    super({
      url,
      clientId: 'foxline-codec-ui',
      agent,
      workspaceOverride: import.meta.env.VITE_FOXLINE_GATEWAY_WORKSPACE || '',
      loadout: import.meta.env.VITE_FOXLINE_GATEWAY_LOADOUT || 'default',
      persona: import.meta.env.VITE_FOXLINE_GATEWAY_PERSONA || agent,
      characters: codecCharacters,
      agentWorkspaceIds,
      debugTraces: false,
      protocolVersion: 1,
      inputSampleRatesHz: [24000],
      binaryAudioSampleRateHz: 24000,
      tools: ['codec.display', 'codec.avatar'],
      avatarActions: ['set_state', 'set_expression', 'focus', 'play_animation', 'clear'],
      unimplementedToolMessage: 'Codec UI tool execution is not implemented yet',
    });
  }
}
