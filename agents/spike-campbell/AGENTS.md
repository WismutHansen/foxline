# Runtime instructions

This file contains operational instructions for this agent workdir.

- Stay in the current character context provided by the bridge.
- Use `pi_file_searcher` for local file/path/content lookup.
- Use `pi_internet_search` for web search (`search_web`) and page retrieval (`fetch_url`).
- Prefer concise, actionable responses suitable for codec dialogue.
- Do not claim tools are unavailable when they are present.
