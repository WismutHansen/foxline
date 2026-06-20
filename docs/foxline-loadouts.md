# Foxline loadouts

The Rust Voice Gateway owns `.foxline/` voice-session configuration. Pi-specific defaults remain in `.pi/` and are consumed by Pi, not by the gateway.

## Directory layout

```text
<workspace>/
  .foxline/
    loadout.toml
    loadouts/
      default.toml
      field.toml
    extensions/
      project-tools/
```

Loadout selection order:

1. `.foxline/loadouts/<name>.toml`
2. `.foxline/loadout.toml` when `<name>` is `default`
3. Built-in gateway defaults when no file exists

## Loadout schema

```toml
extensions = ["builtin:frontend-tools", "local:project-tools"]

[pi]
profile = "voice"
config = ".pi/config.toml"
session_dir = ".pi/sessions"

[lifecycle]
prewarm = true
keep_warm_ms = 600000

[adapters]
stt = "parakeet-silero"
tts = "qwen3-worker"

[tools]
required_frontend = ["codec.display"]
optional_frontend = ["codec.avatar"]
allowed_pi = ["read", "write"]
```

`required_frontend` names must be advertised by the connected frontend or session startup fails. `optional_frontend` names are enabled only when advertised. `allowed_pi` is passed to Pi as the tool allowlist for the Brain process.

## Extension references

- `builtin:<name>` resolves to a bundled gateway extension under `crates/voice_gateway/extensions/<name>`.
- `local:<name>` resolves to `.foxline/extensions/<name>` inside the selected workspace.
- A relative path such as `extensions/project-tools` resolves under `.foxline/`.
- Absolute extension paths are rejected.

The gateway will pass resolved voice-only extensions to Pi with explicit `--extension` flags when the Pi RPC Brain adapter is implemented.
