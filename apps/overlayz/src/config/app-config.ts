import { invoke } from '@tauri-apps/api/core';

export interface GlobalShortcut {
  modifiers: string[];
  key: string;
}

export interface GlobalShortcuts {
  toggle_audio: GlobalShortcut;
  show_kitt: GlobalShortcut;
  show_orb: GlobalShortcut;
  show_last_agent: GlobalShortcut;
}

export interface KeyboardShortcuts {
  switch_ui: string;
  clear_history: string;
  toggle_draggable: string;
}

export interface AssistantConfig {
  name: string;
  system_prompt: string;
  voice: string;
  speed: number;
  history_enabled: boolean;
}

export interface StorageConfig {
  history_path: string | null;
}

export interface FoxlineConfig {
  url: string;
  agent: string;
  persona: string;
  workspace: string;
  loadout: string;
}

export interface AppConfig {
  kitt: AssistantConfig;
  orb: AssistantConfig;
  keyboard: KeyboardShortcuts;
  global_shortcuts: GlobalShortcuts;
  storage: StorageConfig;
  foxline: FoxlineConfig;
}

let cachedConfig: AppConfig | null = null;

export async function getConfig(): Promise<AppConfig> {
  if (cachedConfig) return cachedConfig;

  try {
    cachedConfig = await invoke<AppConfig>('get_config');
    return cachedConfig;
  } catch (error) {
    console.error('Failed to load config from backend:', error);
    throw error;
  }
}

export async function setLastAgent(agent: string): Promise<void> {
  try {
    await invoke('set_last_agent', { agent });
  } catch (error) {
    console.error('Failed to set last agent:', error);
  }
}

export async function getAudioEnabled(): Promise<boolean> {
  try {
    return await invoke<boolean>('get_audio_enabled');
  } catch (error) {
    console.error('Failed to get audio enabled state:', error);
    return true;
  }
}

export async function setAudioEnabled(enabled: boolean): Promise<void> {
  try {
    await invoke('set_audio_enabled', { enabled });
  } catch (error) {
    console.error('Failed to set audio enabled state:', error);
  }
}
