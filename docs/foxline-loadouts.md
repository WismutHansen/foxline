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
    personas/
      radio-operator/
        assets/
          reference_audio/
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
model = "LM-Studio/gemma-4-26b-a4b-it"
thinking = "minimal"
append_system_prompt_file = "SYSTEM.md"

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

`model`, `thinking`, and prompt fields are optional Pi launch settings for voice loadouts. `append_system_prompt` passes inline text to Pi with `--append-system-prompt`; `append_system_prompt_file` reads a workspace-relative file and passes its contents with `--append-system-prompt`.

## Persona voice references

`start_session` takes an `agent` for the Pi-backed Brain and an optional `persona` for voice/presentation. When `persona` is omitted, it defaults to `agent`.

TTS reference lookup for a persona checks:

1. `<workspace>/.foxline/personas/<persona>/assets/reference_audio`
2. `<workspace>/personas/<persona>/assets/reference_audio`
3. `<workspace>/agents/<persona>/assets/reference_audio`
4. `<repo>/agents/<persona>/assets/reference_audio`

This lets a general UI keep one Agent/Brain active while changing voice or presentation persona.

## Extension references

- `builtin:<name>` resolves to a bundled gateway extension under `crates/voice_gateway/extensions/<name>`.
- `local:<name>` resolves to `.foxline/extensions/<name>` inside the selected workspace.
- A relative path such as `extensions/project-tools` resolves under `.foxline/`.
- Absolute extension paths are rejected.

The gateway will pass resolved voice-only extensions to Pi with explicit `--extension` flags when the Pi RPC Brain adapter is implemented.
