---
status: accepted
---

# Gateway Personas and client-owned presentations

## Decision

Foxline keeps spoken identity and visual presentation on opposite sides of the Protocol seam.

A **Persona** is gateway-owned and determines what a Voice Session says and sounds like: a fixed `PROMPT.md`, a TTS voice reference, and optional canned spoken responses. Persona packages resolve from the selected Work Directory, the user's XDG data directory, then bundled distributable packages.

A **Frontend Skin** owns how a Persona looks. Codec portraits and mouth frames remain Codec data; Overlayz Orb/LED animation remains Overlayz data. Frontends map the stable Persona id to their own presentation and use a neutral fallback when no specialized presentation exists. The gateway does not distribute or interpret visual assets.

Gateway-owned STT and TTS remain the default, including for thin and mobile clients. ADR 0003's client-owned speech slots remain an optional future capability, not a requirement for Persona operation.

## Package layout

```text
$XDG_DATA_HOME/foxline/personas/<id>/
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

A Work Directory may override an installed Persona at `.foxline/personas/<id>/`.

`PROMPT.md` is conventional rather than configurable. Its content and the manifest contribute to the warm Brain identity, preventing a process initialized for one Persona from being reused by another active Voice Session.

## Consequences

- Persona prompt, voice, and canned speech can be synchronized independently of the Foxline source repository.
- Frontends remain deliberately opinionated and need no common avatar schema.
- Multiple clients can select the same Persona while retaining independent active Pi RPC processes and event streams.
- Canned speech is selected and streamed by the gateway; clients receive ordinary audio and semantic phase/tool events.
- Copyrighted or private voice and visual assets remain user data and must not be committed to this repository.

## Rejected alternatives

- A universal cross-frontend Character package: rejected because Codec portrait animation and Overlayz procedural animation do not share a useful representation.
- Client-provided Persona prompts on every session: deferred because it complicates trust and warm-session identity while gateway-installed Personas already support thin clients.
- Gateway distribution of frontend visual assets: rejected because clients may run remotely and already own their presentation data.
