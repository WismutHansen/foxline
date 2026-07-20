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

Overlayz uses the same gateway Agent and Persona system as every other Frontend Skin; it does not maintain a separate Codec character registry.

- `agent` selects the Pi workspace, tools, loadout, and Brain personality.
- `persona` selects the gateway-owned TTS reference voice and presentation identity. It defaults to `agent` when empty.
- `workspace` is the Agent workspace path.

To give KITT both a distinct Brain personality and voice, use a KITT Agent workspace plus a local, ignored reference recording:

```text
agents/kitt/
├── AGENTS.md
├── SYSTEM.md
├── .foxline/
│   └── loadout.toml
└── assets/
    └── reference_audio/
        ├── kitt.wav
        └── kitt.wav.txt
```

Files under `agents/*/assets/reference_audio/` are intentionally gitignored because they may be generated, private, or copyrighted. The transcript in `kitt.wav.txt` must exactly match the spoken content of `kitt.wav`. Commit only the Agent metadata and prompts you have the right to distribute.

Then select both identities in Overlayz:

```toml
[foxline]
url = "ws://127.0.0.1:8780"
agent = "kitt"
persona = "kitt"
workspace = "agents/kitt"
loadout = "default"
```

The Agent personality currently comes from the selected workspace and loadout, including `SYSTEM.md` when configured through `pi.append_system_prompt_file`. Persona `prompt_file` composition and automatic Persona-to-Visual-Theme selection are not yet implemented; select the LED or Orb Visual Theme in Overlayz itself.

See `docs/foxline-loadouts.md` for loadout and Persona lookup rules.
