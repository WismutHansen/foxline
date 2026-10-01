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
        PROMPT.md
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
# TTS: "rust-mlx" (native spqx), "qwen3-worker" (Python MLX),
# or "kokorox" (independent Kokoro ONNX worker). Default is unchanged.
tts = "rust-mlx"

[tools]
required_frontend = ["codec.display"]
optional_frontend = ["codec.avatar"]
allowed_pi = ["read", "write"]
```

`required_frontend` names must be advertised by the connected frontend or session startup fails. `optional_frontend` names are enabled only when advertised. `allowed_pi` is passed to Pi as the tool allowlist for the Brain process.

`model` and `thinking` are optional Pi launch settings for voice loadouts. Persona character description and speaking style prompt text belongs to the selected Persona package, not the voice loadout.

## Kokorox backend

Install/build `kokorox-tts-worker` independently in the kokorox repository (`just worker-build`). Foxline does not link kokorox crates or auto-download this worker.

```toml
# .foxline/loadout.toml
[adapters]
tts = "kokorox"

[adapters.kokorox]
worker = "/absolute/path/to/kokorox-tts-worker"
```

`FOXLINE_TTS_BACKEND` overrides the selected backend. For kokorox, worker lookup is `FOXLINE_TTS_KOKOROX_WORKER`, then `adapters.kokorox.worker`, then the sibling `kokorox/target/release/kokorox-tts-worker`, then `kokorox-tts-worker` on PATH. Explicit worker settings accept an absolute file path or a program name on PATH; invalid settings fail instead of being substituted. Relative executable paths are rejected.

Model/voice/language/speed come from the selected Persona's `[voice.kokorox]` mapping below. Qwen model, reference, temperature, blocksize and output-rate environment overrides do not apply to kokorox. Initial support is v1.0-vocabulary models, one exact voice, and 24 kHz PCM16 LE mono; no resampling, blended styles or Chinese v1.1 switching. The worker validates its archive/voice/model at startup.

Both engines use the existing binary worker adapter despite its historical `QwenWorker*` names. Prewarm waits up to 120 seconds for empty `ready`; startup errors, premature EOF and invalid frames fail clearly. Stderr is continuously drained, with a bounded diagnostic tail retained for startup failures. Sample rate comes from `audio_start`, never `ready`. Cancellation fences old queued/late frames; it cannot preempt an ONNX kernel or recall already-sent audio.

`just gateway-loadout-schema` prints the loadout JSON schema for validation/LSP setup. [Verification and reproduction](tts-engine-verification.md) distinguish component proof from full conversation/playback proof.

## Persona packages

`start_session` takes an `agent` for the Pi-backed Brain and an optional `persona` for gateway-owned prompt, voice, and canned speech. When `persona` is omitted, it defaults to `agent`. Frontend presentation is client-local and keyed by the Persona id.

Personas are gateway-owned spoken identities. A package contains a fixed `PROMPT.md`, a `persona.toml` voice/canned-speech manifest, and optional audio. Frontend portraits and animations remain in each client and are mapped by Persona id.

Example:

```text
$XDG_DATA_HOME/foxline/personas/radio-operator/
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

```toml
id = "radio-operator"
display_name = "Radio Operator"

[voice]
reference_audio = "voice/reference.wav"
reference_text = "voice/reference.txt"

[canned.tool_started]
directory = "canned/tool-started"

[canned.tool_slow]
directory = "canned/tool-slow"

[canned.tool_completed]
directory = "canned/tool-completed"
```

`PROMPT.md` is appended to the Pi Brain prompt for that Voice Session. Its content and the manifest contribute to Brain identity so concurrent or warm sessions cannot cross Persona prompts. `voice.reference_audio` and `voice.reference_text` must be configured together. Canned directories contain signed PCM16 mono or stereo WAV files at 24 kHz; files are selected deterministically per session and streamed through the normal frontend audio path.

### Engine-specific voices

A Persona can declare both mappings once; select its runtime engine in the loadout, not legacy `voice.adapter` metadata:

```toml
[voice.spqx]
reference_audio = "voice/reference.wav"
reference_text = "voice/reference.txt"

[voice.kokorox]
model = "voice/kokoro-martin.onnx"
voices = "voice/voices-martin.npz"
voice = "martin"
language = "de"
speed = 1.0 # optional; defaults to 1.0; must be finite and positive
```

`voice.spqx` serves both `rust-mlx` and `qwen3-worker`. Explicit spqx mapping takes precedence over legacy top-level `voice.reference_audio/reference_text`. Those legacy fields remain valid; directory discovery remains available only when no engine-keyed sections exist. A kokorox-only package does **not** implicitly acquire a spqx voice from nearby WAV files. Missing selected-engine mappings fail; there is no cross-engine fallback. Complete Qwen reference environment overrides remain intentional host overrides; incomplete overrides now fail rather than silently using the Persona.

Nested engine sections reject unknown fields. Assets must exist as files inside the Persona package; absolute/traversing paths and symlinks escaping it are rejected, including legacy reference discovery. Copy or hardlink externally cached assets into installed packages rather than linking outside them. Existing descriptive voice metadata and variants remain compatible. Voice ids do not imply matching timbre between engines.

For German, install the **matched** model and named NPZ pack from the validated public `Godelaune/Kokoro-82M-ONNX-German-Martin` revision `a1cba7fbf0e72fbae38f0a3a48ce0dc8e6077804`; archive entry `martin` is float32 `(510,1,256)`. Keep model, archive and provenance in your private/local package, not this source repository. A `.bin` extension does not prove format: kokorox legacy packs are NPZ archives, while Crane's raw `df_kerstin.bin` needs conversion and is not validated here.

Standalone synthesis still requires no Persona or Foxline session:

```bash
kokorox-tts-worker --serve --model /local/model.onnx --voices /local/voices.npz \
  --voice martin --language de --speed 1.0 --output-sample-rate 24000
```

The worker accepts raw UTF-8 `speak`, empty `cancel`/`shutdown`, and the nine-byte LE frame header; it is not a JSON-line or HTTP interface. Separate HTTP convergence and license/provenance review remain open. A subprocess boundary alone is not distribution-compliance clearance.

Current Persona lookup checks:

1. `<work-directory>/.foxline/personas/<persona>`
2. `$XDG_DATA_HOME/foxline/personas/<persona>` or `~/.local/share/foxline/personas/<persona>`
3. `<repo>/personas/<persona>` for bundled distributable Personas
4. Legacy Work Directory/repository Agent sidecars during migration

The first matching package wins. Paths in `persona.toml` must stay confined to the package.

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
