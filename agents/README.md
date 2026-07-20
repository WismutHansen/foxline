# Foxline Agent and Persona reference source

This directory is the repository reference used to seed a user's Foxline data directory. Commit only distributable Agent metadata, prompts, loadouts, and manifests here.

Local voice references, canned speech, portraits, and extracted/generated runtime assets remain gitignored. Never commit copyrighted assets to this repository.

Synchronize the available local data into the gateway-owned XDG Persona store with:

```bash
scripts/sync-personas-to-xdg.sh
```

The default destination is:

```text
$XDG_DATA_HOME/foxline
```

or `~/.local/share/foxline` when `XDG_DATA_HOME` is unset. The script maps:

- `agents/<id>/PROMPT.md` to `personas/<id>/PROMPT.md`;
- `assets/reference_audio` to the Persona `voice/` directory;
- `assets/filler/tool_start_*` to `canned/tool-started/`;
- `assets/filler/tool_slow_*` to `canned/tool-slow/`;
- `assets/filler/tool_done_*` to `canned/tool-completed/`.

Visual assets are deliberately not copied because frontend presentation remains client-owned.
