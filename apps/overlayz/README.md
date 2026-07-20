# Overlayz

Overlayz is the Foxline ambient overlay Frontend Skin. It runs as a Tauri shell, connects to the Rust Foxline Voice Gateway, and renders swappable Visual Themes such as Orb and the KITT LED theme.

## Architecture

```text
Overlayz Tauri shell
  -> @foxline/voice-client
      -> foxline-voice-gateway WebSocket
```

The Rust Voice Gateway owns STT, TTS, turn authority, and the Pi Brain path. Overlayz owns only shell integration, microphone capture, audio playback, and visuals.

## Run Overlayz

From the repository root, start the complete development stack:

```bash
just run overlayz
```

This installs workspace dependencies, then starts:

1. Parakeet/Silero STT on `127.0.0.1:8796`.
2. The Rust Voice Gateway on `127.0.0.1:8780`.
3. The Overlayz Tauri development app.

Stopping the command also stops the STT service and gateway it started.

For individual development tasks:

```bash
just overlayz-typecheck
just overlayz-build         # frontend build only
just overlayz-tauri-check
```

From `apps/overlayz`:

```bash
bun tauri dev       # full Tauri app; also starts the Vite frontend
bun tauri build     # build and package the full desktop app
bun run dev:web     # Vite frontend only
bun run build       # frontend build only
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

Overlayz has no frontend-owned STT, TTS, or Brain path. When launching components manually, start the Parakeet/Silero service and Voice Gateway before Overlayz. Prefer `just run overlayz`, which starts the complete stack.

## Agents, Personas, and reference voices

Overlayz selects gateway Agents and Personas but owns only their visual presentation.

- `agent` selects the Pi Work Directory, tools, loadout, and operational identity.
- `persona` selects the gateway-owned `PROMPT.md`, voice, and canned speech. It defaults to `agent` when empty.
- `workspace` currently carries the registered Work Directory path in Protocol v1.

Install a KITT Persona on the gateway machine:

```text
~/.local/share/foxline/personas/kitt/
├── persona.toml
├── PROMPT.md
├── voice/
│   ├── reference.wav
│   └── reference.txt
└── canned/
    ├── tool-started/
    ├── tool-slow/
    └── tool-completed/
```

The reference transcript must exactly match the WAV. Keep copyrighted or private recordings in user data and out of this repository. Overlayz maps the `kitt` Persona id to its local LED presentation; the gateway never reads Overlayz visual assets.

Then select the Agent and Persona in Overlayz:

```toml
[foxline]
url = "ws://127.0.0.1:8780"
agent = "assistant"
persona = "kitt"
workspace = "/Users/you/work/your-project"
loadout = "default"
```

The gateway composes the Work Directory/Agent instructions with the selected Persona's `PROMPT.md`. Select the LED or Orb Visual Theme in Overlayz; presentation remains client-local.

See `docs/foxline-loadouts.md` for loadout and Persona lookup rules.
