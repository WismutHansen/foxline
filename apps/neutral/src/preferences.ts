export const visuals = ["kitt", "phosphor", "relay"] as const;
export type VisualTheme = (typeof visuals)[number];
export const phosphorModes = ["cloud", "torus", "metamorph", "medusa"] as const;
export type PhosphorMode = (typeof phosphorModes)[number];
export type MotionPreference = "system" | "full" | "reduce";
export type Preferences = {
  visual: VisualTheme;
  mode: PhosphorMode;
  motion: MotionPreference;
};
export type ConnectionSettings = {
  url: string;
  agent: string;
  workspace: string;
  persona: string;
};
export const visualNames = {
  kitt: "KITT",
  phosphor: "Phosphor",
  relay: "Relay",
};
export const modeNames = {
  cloud: "Point cloud",
  torus: "Torus knot",
  metamorph: "Metamorph",
  medusa: "Medusa",
};

// Keep the existing keys: becoming canonical must not erase anyone's preferences.
const connectionKey = "foxline.neutral.connection";
const visualKey = "foxline.neutral.visual";
function load(key: string): Record<string, unknown> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? "{}");
    return value && typeof value === "object"
      ? (value as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}
function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* Storage is optional; private browsing must remain usable. */
  }
}
export function normalizePreferences(
  value: Partial<Record<keyof Preferences, unknown>>,
): Preferences {
  return {
    visual: visuals.find((item) => item === value.visual) ?? "phosphor",
    mode: phosphorModes.find((item) => item === value.mode) ?? "metamorph",
    motion:
      (["system", "full", "reduce"] as const).find(
        (item) => item === value.motion,
      ) ?? "system",
  };
}
export const loadPreferences = () => normalizePreferences(load(visualKey));
export const savePreferences = (preferences: Preferences) =>
  save(visualKey, preferences);
export function connectionProblem(
  value: ConnectionSettings,
  secure = location.protocol === "https:",
): string {
  try {
    const url = new URL(value.url);
    if (!["ws:", "wss:"].includes(url.protocol))
      return "Use a ws:// or wss:// gateway address.";
    if (url.username || url.password || url.search || url.hash)
      return "Keep credentials out of the URL. Use the access token field.";
    if (secure && url.protocol !== "wss:")
      return "This HTTPS page needs a wss:// gateway. The built-in /gateway connection supports this.";
    return "";
  } catch {
    return "Enter a complete gateway address, starting with ws:// or wss://.";
  }
}
export function loadConnection(): ConnectionSettings {
  const value = load(connectionKey);
  const defaults = `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/gateway`;
  const settings = {
    url: typeof value.url === "string" ? value.url : defaults,
    agent: typeof value.agent === "string" ? value.agent : "",
    workspace: typeof value.workspace === "string" ? value.workspace : "",
    persona: typeof value.persona === "string" ? value.persona : "",
  };
  // Never restore an old credential-bearing URL into a visible/persisted field.
  if (connectionProblem(settings)) {
    settings.url = defaults;
    saveConnection(settings);
  }
  return settings;
}
export const saveConnection = (settings: ConnectionSettings) =>
  save(connectionKey, settings);
export function sameConnection(a: ConnectionSettings, b: ConnectionSettings) {
  return (
    a.url === b.url &&
    a.agent === b.agent &&
    a.workspace === b.workspace &&
    a.persona === b.persona
  );
}
