# Overlayz

Overlayz is the Foxline ambient overlay Frontend Skin. It runs as a Tauri shell, connects to the Rust Foxline Voice Gateway, and renders swappable Visual Themes such as Orb and the KITT LED theme.

## Architecture

```text
Overlayz Tauri shell
  -> @foxline/voice-client
      -> foxline-voice-gateway WebSocket
```

The Rust Voice Gateway owns STT, TTS, turn authority, and the Pi Brain path. Overlayz owns only shell integration, microphone capture, audio playback, and visuals.

## Development

From the repository root:

```bash
just overlayz-typecheck
just overlayz-build
just overlayz-tauri-check
```

From this directory:

```bash
bun run dev        # Tauri dev
bun run dev:web    # Vite-only frontend
bun run build      # frontend build
```

## Configuration

Overlayz reads `config.toml` from the app config directory and creates defaults on first run. Gateway settings live under `[foxline]`:

```toml
[foxline]
url = "ws://127.0.0.1:8780"
agent = "campbell"
persona = "campbell"
workspace = "agents/campbell"
loadout = "default"
```

There is no local STT/TTS/LLM path. Run `foxline-voice-gateway` separately before using Overlayz.
