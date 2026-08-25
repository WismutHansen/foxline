# govnr voice spike — foxline + neutral web client over the tailnet

Talk to a govnr orchestrator assistant from your iPhone. Channels placement:
the voice brain is **local gemma (light lifting only)** — heavy reasoning
escalates to the orchestrator model (deepseek-v4-flash) via the
`ask_orchestrator` tool (`.foxline/extensions/govnr-escalate` in the govnr repo).

## One-time prerequisite: tailnet HTTPS

iOS Safari only allows microphone access on secure pages. Enable it once:

1. https://login.tailscale.com/admin/settings/dns → **HTTPS Certificates** → Enable.
2. Verify: `tailscale cert mac.wismut.ts.net` should succeed.

(Without this, use the Vite basic-ssl fallback: accept the self-signed warning on the phone.)

## Run

```bash
# 1. Gateway, LAN/tailnet reachable, token-authed (repo root):
FOXLINE_TTS_MODEL_PATH="$(ls -d ~/.cache/huggingface/hub/models--mlx-community--Qwen3-TTS-12Hz-0.6B-Base-4bit/snapshots/*/)" \
FOXLINE_STT_BACKEND=ears FOXLINE_STT_WS_URL=ws://127.0.0.1:8798/ws \
  ./target/release/foxline-voice-gateway --config apps/spike-webclient/gateway.govnr.toml &

#    (start ears-server first if not running:)
nohup ears-server --bind 127.0.0.1:8798 >/tmp/ears.log 2>&1 &

# 2. Neutral web client (thinking-orbs UI):
pnpm --filter @foxline/spike-webclient dev   # serves on :5174, host-exposed

# 3. Tailnet HTTPS routes (after enabling HTTPS certs):
tailscale serve --bg --https=443 / http://127.0.0.1:5173
```

Then open `https://mac.wismut.ts.net/?token=govnr-voice-spike` on the iPhone,
tap Start, allow the microphone. The client connects its WS to
`wss://<host>/gateway` — add that route if serving the app on a different path:

```bash
tailscale serve --bg --set-path /gateway http://127.0.0.1:8783
```

## Escalation placement (channels)

The voice brain stays light. Heavy reasoning goes to the orchestrator:

```bash
# same machine (Mac Studio runs ds4):        default http://127.0.0.1:8000
# from oqto-dev or other tailnet machines:   FOXLINE_ORCHESTRATOR_URL=http://mac:8000
```

Set `FOXLINE_ORCHESTRATOR_URL` when starting the gateway so brain children inherit it.

## Notes

- The voice session's pi workspace IS the govnr repo — pi reads govnr AGENTS.md,
  so spoken questions get answered with full fleet/repo context.
- `commit_grace_ms = 2500`: swallows post-commit STT decoder flushes and boundary-VAD
  chatter so they don't kill turns (see archive/voice-cascade-extension/OUTCOME.md).
- thinking-orbs states map to voice phases: listening/thinking(working)/speaking(composing)/idle(breathing).
