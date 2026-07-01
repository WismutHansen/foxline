# Foxline

Foxline is a voice interface system for low-latency conversations with Pi-backed agents. It separates reusable voice orchestration from frontend skins such as the Metal Gear Solid-style codec interface.

## Language

**Voice Gateway**:
A reusable runtime that coordinates low-latency bidirectional voice interaction between a frontend, speech services, and a Pi-backed agent.
_Avoid_: Foxline core, codec server, voice orchestrator

**Foxline Codec UI**:
The Metal Gear Solid-style frontend skin for interacting with a Voice Gateway.
_Avoid_: Codex, Codec as the product name

**Agent**:
A Pi-backed working identity rooted in a workspace and Pi configuration, optionally refined by voice-specific loadouts.
_Avoid_: Character, persona

**Persona**:
The presentational identity used for voice and UI, including display name, voice assets, Persona prompt fragments, and optional frontend-specific representation.
_Avoid_: Agent

Personas can change within a frontend session without changing the Pi-backed Agent. Agent switching changes the Brain/workspace/loadout identity; Persona switching changes voice/reference assets, Persona prompt fragments, and presentation identity. Persona packages can be installed centrally and overridden per workspace.

**Avatar Action**:
A semantic presentation request routed through the Voice Gateway to a frontend, such as changing expression, focus, or animation, without prescribing theme-specific visuals.
_Avoid_: MGS animation, sprite command

**Foxline Directory**:
The `.foxline/` directory inside an Agent workspace that contains optional voice-session configuration and assets for the Voice Gateway.
_Avoid_: .codec, .voice-agents, .agents/voice

**Pi Directory**:
The `.pi/` directory inside an Agent workspace that contains Pi-specific configuration for the Brain.
_Avoid_: .agents for Pi-specific config

**Voice Mode Loadout**:
The agent-specific configuration used only for voice sessions, including allowed tools, extensions, skills, adapter choices, lifecycle policy, and voice interaction constraints.
_Avoid_: Agent config, codec config

**Frontend Tool**:
A capability implemented by the connected frontend that the Brain can request through the Voice Gateway during a voice session.
_Avoid_: Browser tool, UI command

**Speech Ownership**:
Which side runs a speech stage for a session, negotiated in frontend capabilities at `hello`. A stage is either gateway-owned (the default) or client-owned. Client-owned STT or TTS does not change turn authority or the Brain path; the gateway always owns the session. See `docs/adr/0003-client-owned-speech-capabilities.md`.
_Avoid_: Browser STT, WASM mode, client-mode (say client-owned STT/TTS or speech ownership)

**Voice Extension**:
A Pi extension loaded only for voice sessions by the Voice Gateway, either from bundled standard extensions or project-local `.foxline/extensions/` paths.
_Avoid_: Default Pi extension, frontend plugin

**Brain**:
The Pi RPC agent process that owns agent reasoning, tool use, skills, extensions, prompt behavior, and model-provider access for an agent session.
_Avoid_: LLM server, assistant logic

**Protocol**:
The authoritative frame schema for the Voice Gateway WebSocket: control messages, server events, capability negotiation, audio frame format, and session lifecycle. Defined once in Rust (`crates/protocol`) as the source of truth, consumed natively by Rust frontends and codegenerated to TypeScript for TS frontends. Frontends and the gateway never hand-mirror the Protocol independently.
_Avoid_: API, wire format, message types

**Client Core**:
The per-platform module that connects a Frontend Skin to a Voice Gateway: WebSocket lifecycle and reconnect, microphone capture, PCM playback scheduling, capability negotiation, and Persona/Agent session state. A Client Core is platform-bound (TypeScript/AudioContext for web and Tauri; Rust for a future GPUI frontend) and shares the Protocol across that language seam, not its implementation body. There is one Client Core per runtime platform, not one shared across languages.
_Avoid_: client library, SDK (say Client Core), frontend logic

**Frontend Skin**:
The presentation and shell of a voice frontend: visuals, interaction model, and OS integration. Codec is a fullscreen/second-screen role-play skin (web today, Tauri later); KITT is an unobtrusive overlay assistant skin (Siri-shaped, cross-platform, Tauri). A Frontend Skin imports a Client Core and owns only presentation and shell concerns; it must not contain gateway protocol or audio-pipeline logic.
_Avoid_: app, UI, frontend (say Frontend Skin when distinguishing presentation from the Client Core)

**Shell**:
The OS-integration layer of a Frontend Skin: window policy (fullscreen vs always-on overlay), global shortcuts, tray, and any platform audio or accessibility hooks. Both Codec and KITT are Tauri apps; their Shell configuration differs (overlay/tray for KITT, fullscreen window for Codec). Shell concerns reach the Client Core only through the Client Core's own interface and never cross into the Protocol layer.
_Avoid_: native layer, platform code (say Shell)
