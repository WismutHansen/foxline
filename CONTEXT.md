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
The presentational identity used for voice and UI, including display name, voice assets, and frontend-specific representation.
_Avoid_: Agent

Personas can change within a frontend session without changing the Pi-backed Agent. Agent switching changes the Brain/workspace/loadout identity; Persona switching changes voice/reference assets and presentation identity.

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
The agent-specific configuration used only for voice sessions, including allowed tools, extensions, skills, prompts, and voice interaction constraints.
_Avoid_: Agent config, codec config

**Frontend Tool**:
A capability implemented by the connected frontend that the Brain can request through the Voice Gateway during a voice session.
_Avoid_: Browser tool, UI command

**Voice Extension**:
A Pi extension loaded only for voice sessions by the Voice Gateway, either from bundled standard extensions or project-local `.foxline/extensions/` paths.
_Avoid_: Default Pi extension, frontend plugin

**Brain**:
The Pi RPC agent process that owns agent reasoning, tool use, skills, extensions, prompt behavior, and model-provider access for an agent session.
_Avoid_: LLM server, assistant logic
