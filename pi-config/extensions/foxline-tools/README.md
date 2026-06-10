# foxline-tools pi extension

This extension is the canonical foxline Pi policy + local search tool implementation.

## Features

- Does not manage `bash` policy (configure via Pi CLI args)
- Registers `pi_file_searcher` tool with actions:
  - `capabilities`
  - `find_paths`
  - `grep_content`
  - `list_dir`
- Registers `pi_internet_search` tool with actions:
  - `providers_status`
  - `search_web`
  - `fetch_url`
- Search fallback chains:
  - local content: `rg -> grep -> builtin`
  - local paths: `fd -> find -> builtin`
  - web search: `exa_mcp -> duckduckgo`
  - page extraction: `jina reader`

## Bash policy

Use Pi CLI to block bash globally for a run:

- `--exclude-tools bash`

Example:

- `pi --no-extensions -e ./pi-config/extensions/foxline-tools/index.ts --exclude-tools bash`

## Exa MCP

Default Exa MCP URL:

- `https://mcp.exa.ai/mcp`

Override with:

- `EXA_MCP_URL`

Optional latency tuning env vars:

- `FOXLINE_EXA_TIMEOUT_MS` (default: `900`)
- `FOXLINE_DDG_TIMEOUT_MS` (default: `3000`)
- `FOXLINE_JINA_TIMEOUT_MS` (default: `6000`)
- `FOXLINE_FETCH_MAX_CHARS` (default: `20000`)

## Deployment model

This directory is meant to be symlinked into each agent extension directory using relative symlinks.
Use `scripts/sync-pi-extensions.sh` from repo root.
