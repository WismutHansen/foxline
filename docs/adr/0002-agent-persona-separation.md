---
status: accepted
---

# Separate Agent identity from Persona identity

The Rust Voice Gateway protocol distinguishes Pi-backed Agent identity from voice and presentation Persona identity. This keeps the gateway general enough for clients that need to change voices, characters, or avatars without changing the Brain session.

## Decision

Gateway sessions carry both `agent` and `persona`.

`agent` selects the Brain-side identity: Pi workspace, loadout, extensions, tool surface, prompt context, and warm Pi lifecycle. Switching the Agent is a Brain/session-level operation and may require a new or restarted Pi-backed session.

`persona` selects the voice and presentation identity used by the gateway and frontend. It controls TTS reference lookup, persona prompt fragments, and can be used by clients for character or avatar presentation. Switching the Persona is not, by itself, an Agent switch.

`persona` is optional in the protocol and defaults to `agent` for compatibility with existing clients and single-character deployments. The Codec UI may continue to expose character-oriented controls, but gateway-facing APIs should treat that as Persona switching. During migration, server turn events include both the legacy `character` field and the explicit `persona` field.

Persona packages are self-contained directories. They contain a `persona.toml` manifest, optional prompt files, voice references, and optional frontend-specific config or assets. The manifest separates generic identity, voice, prompt, and frontend-specific sections. The gateway understands the generic, prompt, and voice fields; frontend-specific sections are passed through only to matching frontends and are not interpreted by the gateway.

Persona packages may be installed centrally and overridden per workspace. For the Codec UI case, Codec-specific config and visual assets may live inside the same Persona package directory. This keeps a user-visible persona in one place while preserving a clean manifest boundary for other frontends.

## Context

Foxline originally used Codec "character" language for both UI presentation and gateway voice identity, while the Rust gateway used `agent` to start the Pi-backed Brain session. That was acceptable for a single UI where one character implied one agent, but it does not support a general gateway.

Future clients such as KITT, oqto, or other voice UIs need two independent operations:

- change the Brain Agent, which changes the work context and tool/runtime behavior;
- change the voice, character, or avatar Persona, while keeping the same Brain Agent.

Conflating those operations would force UIs to model every voice as a separate Agent, restart Brain sessions unnecessarily, and make the gateway less reusable. Splitting the terms also clarifies prompt ownership: Brain/tool/workspace behavior belongs to the Agent and voice loadout, while character description and speaking style belong to the Persona.

## Consequences

- Gateway traces, benchmarks, and loadout resolution should include both `agent` and `persona` when both are known.
- Frontends can swap voice or character presentation while preserving the active Brain Agent.
- Agent switching and Persona switching are distinct client commands and should stay distinct in future protocol work.
- TTS worker, reference audio, cache, and cleanup behavior must be Persona-aware.
- Persona prompt fragments are composed into the Pi session as voice-session context, but remain owned by the Persona package rather than the Agent loadout.
- Persona resolution must support central installed packages and workspace-local overrides.
- Codec-specific Persona assets may live in the Persona package, but the gateway treats them as frontend-specific data rather than generic runtime state.
- Existing clients that send only `agent` continue to work because `persona` defaults to `agent`.
- Existing UI code can keep user-facing "character" wording where that is the right product language, but gateway protocol and docs should prefer "persona" for voice/presentation identity.

## Considered options

- Keep using `character` everywhere: rejected because it is UI-specific terminology and does not describe Brain identity.
- Treat every voice or avatar as a separate Agent: rejected because it couples presentation changes to Pi session lifecycle and loadout behavior.
- Make `persona` mandatory immediately: rejected because compatibility during the Rust gateway migration matters.
- Keep Persona entirely in the frontend: rejected because the gateway owns TTS reference resolution and speech worker orchestration.
- Force visual assets into frontend-owned directories only: rejected for the Codec case because it scatters one user-visible persona across multiple locations. Frontend-specific assets can live in the Persona package when that is the practical packaging model, as long as the manifest keeps the gateway/frontend ownership boundary explicit.
