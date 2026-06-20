# Foxline Voice Gateway

Rust runtime core for Foxline voice sessions.

Run locally:

```bash
just gateway-check
just gateway --bind 127.0.0.1:8780
```

The gateway creates an XDG config file on first run at:

```text
$XDG_CONFIG_HOME/foxline/config.toml
```

or `~/.config/foxline/config.toml` when `XDG_CONFIG_HOME` is unset.

The first protocol pass exposes a WebSocket server with JSON control/events and binary PCM input frames. It does not call LLM servers directly. Pi RPC, STT, and TTS are adapter boundaries that will be wired in follow-up `trx` issues.
