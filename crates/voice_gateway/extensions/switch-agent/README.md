# switch-agent pi extension

Registers a single `switch_agent` tool that lets Pi switch the live Foxline
voice session to a different workspace/agent (and optionally persona) without
ending the session — no reconnect, no dropped WebSocket.

## Opt-in only

Unlike `pi-config/extensions/foxline-tools` (symlinked into every agent via
`scripts/sync-pi-extensions.sh`), this extension is **not** loaded by
default. A workspace opts in explicitly by referencing `builtin:switch-agent`
in its `.foxline/loadout.toml` (or `.foxline/loadouts/<name>.toml`)
`extensions = [...]` list. The gateway resolves that reference to this
directory and passes it to Pi via `--extension` (see
`crates/voice_gateway/src/loadout.rs::resolve_extension` and
`crates/voice_gateway/src/brain.rs::PiLaunch::build`).

## How it talks to the gateway

`switch_agent`'s `execute()` runs in-process inside the Pi child process. It
does **not** rely on the gateway parsing Pi's own tool-call stream — instead
it opens a raw TCP connection directly to the gateway's loopback-only control
listener (`crates/voice_gateway/src/control.rs`) and speaks a tiny JSON-lines
protocol:

1. Connect to `FOXLINE_CONTROL_ADDR` (injected by the gateway at Pi launch).
2. Write one JSON line: `{"token": "<FOXLINE_CONTROL_TOKEN>", "workspace": "<id>", "persona": "<optional>"}`.
3. Read one JSON line back: `{"ok": true, "workspace": ..., "agent": ..., "persona": ..., "loadout": ...}` or `{"ok": false, "error": "..."}`.
4. Close.

`FOXLINE_CONTROL_TOKEN` is the calling Pi process's own deterministic
`BrainIdentity::session_name()`, so it survives `BrainPool` reuse of an
already-warm process without needing a fresh per-connection secret. The
gateway swaps its live `brain`/`stt`/`tts` state in place for the WS
connection that owns that session and emits a `ServerEvent::AgentSwitched`
(or `AgentSwitchFailed`) event to the frontend.

## Tool parameters

- `workspace` (required): a workspace id from the gateway's
  `[workspaces.registry]` (see `config.rs::WorkspacesConfig`) — not an
  arbitrary filesystem path.
- `persona` (optional): overrides the target workspace's configured persona.

## Example loadout opt-in

```toml
# .foxline/loadout.toml
extensions = ["builtin:switch-agent"]
```
