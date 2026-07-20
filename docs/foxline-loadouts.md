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
        persona.toml
        SYSTEM.md
        voice/
          reference_audio/
        codec/
          persona.toml
          portrait.png
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

[lifecycle]
prewarm = true
keep_warm_ms = 600000
# On prewarm, also run an ephemeral `pi --print --no-session` with the same
# launch args so the provider loads the model and prefills the prompt prefix
# before the first real turn (measured: 10.2s -> 0.8s first audio). Writes no
# session history.
warmup_prompt = true

[adapters]
stt = "parakeet-silero"
# tts backends: "rust-mlx" (native spqx worker; fastest, 0.3s model load) or
# "qwen3-worker" (python MLX worker). Same binary protocol and voice quality.
tts = "rust-mlx"

[tools]
required_frontend = ["codec.display"]
optional_frontend = ["codec.avatar"]
allowed_pi = ["read", "write"]
```

`required_frontend` names must be advertised by the connected frontend or session startup fails. `optional_frontend` names are enabled only when advertised. `allowed_pi` is passed to Pi as the tool allowlist for the Brain process.

`model` and `thinking` are optional Pi launch settings for voice loadouts. Persona character description and speaking style prompt text belongs to the selected Persona package, not the voice loadout.

## Persona packages

`start_session` takes an `agent` for the Pi-backed Brain and an optional `persona` for voice/presentation. When `persona` is omitted, it defaults to `agent`.

Personas are package directories with a `persona.toml` manifest. A package can include prompt files, voice references, and frontend-specific config or assets in one place. The gateway understands only the generic, prompt, and voice fields; frontend-specific sections or files are routed to matching frontends as opaque data.

Example:

```toml
id = "radio-operator"
display_name = "Radio Operator"
prompt_file = "SYSTEM.md"

[voice]
adapter = "qwen3-worker"
reference_audio_dir = "voice/reference_audio"
reference_audio = "voice/reference_audio/radio-operator-a.wav"
reference_text = "voice/reference_audio/reference.txt"

[[voice.variants]]
id = "operator-a"
label = "Operator A"
reference_audio = "voice/reference_audio/radio-operator-a.wav"
reference_text = "voice/reference_audio/reference.txt"
default = true

[frontend.codec]
config = "codec/persona.toml"
portrait = "codec/portrait.png"
```

`voice.reference_audio` and `voice.reference_text` define the default TTS reference used by the gateway. `voice.variants` can list additional shippable references for clients or installers that want to expose voice choices.

Persona package lookup checks:

1. `<workspace>/.foxline/personas/<persona>`
2. `$XDG_CONFIG_HOME/foxline/personas/<persona>`
3. `/etc/foxline/personas/<persona>`
4. app-bundled or repo-bundled demo personas

Workspace-local packages override centrally installed packages. This lets installed systems reuse personas across many Agent workspaces while allowing project-specific Persona overrides.

The repo ships original demo Persona packages under `personas/` for users without game-disc assets. These packages may include generated Codec portraits and Koko/Kokoro-generated reference voices, but must not include copyrighted extracted runtime assets.

For compatibility during migration, TTS reference lookup also accepts legacy sidecar directories:

1. `<workspace>/personas/<persona>/assets/reference_audio`
2. `<workspace>/agents/<persona>/assets/reference_audio`
3. `<repo>/agents/<persona>/assets/reference_audio`

This lets a general UI keep one Agent/Brain active while changing voice or presentation persona.

## Extension references

- `builtin:<name>` resolves to a bundled gateway extension under `crates/voice_gateway/extensions/<name>`.
- `local:<name>` resolves to `.foxline/extensions/<name>` inside the selected workspace.
- A relative path such as `extensions/project-tools` resolves under `.foxline/`.
- Absolute extension paths are rejected.

The gateway passes resolved extensions to Pi with explicit `--extension` flags.

### `builtin:switch-agent`

Bundled but **not** loaded by default (unlike `pi-config/extensions/foxline-tools`,
which every agent gets via `scripts/sync-pi-extensions.sh`). A workspace opts in
by listing it explicitly:

```toml
# .foxline/loadout.toml
extensions = ["builtin:switch-agent"]
```

This registers a `switch_agent` tool that lets Pi switch the live voice session
to a different workspace/agent (and optionally persona) from `[workspaces.registry]`
without ending the session. See
`crates/voice_gateway/extensions/switch-agent/README.md` for the wire protocol.
