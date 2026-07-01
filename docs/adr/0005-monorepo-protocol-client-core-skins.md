---
status: accepted
supersedes: 0001 (monorepo and frontend framing)
---

# Foxline monorepo: Protocol source of truth, Client Core, Frontend Skins

Foxline is a monorepo and a voice-assistant platform. It contains the Voice Gateway, the Protocol, the Client Core, and one or more Frontend Skins. The standalone kitt repository is abandoned; its overlay app is copied into foxline as a Frontend Skin renamed Overlayz. The Protocol is the one artifact shared across every frontend, including a future Rust-native (GPUI) frontend.

## Decision

Foxline is the single repository. It holds four kinds of things:

- **Voice Gateway** (`crates/voice_gateway`) — the Rust daemon, the sole gateway and sole Brain path per ADR 0004. Runs standalone; bundling as a Tauri sidecar is a later packaging concern that is invisible to the Client Core.
- **Protocol** (`crates/protocol` in Rust, `packages/protocol` in TypeScript) — the authoritative frame schema. Defined once in Rust as the source of truth; consumed natively by Rust frontends and codegenerated to TypeScript for TS frontends. No frontend hand-mirrors the Protocol. This is the only artifact shared across the Rust/TS language seam.
- **Client Core** (`packages/voice-client`) — the TypeScript module that connects a Frontend Skin to a Voice Gateway: WebSocket lifecycle and reconnect, microphone capture, PCM playback scheduling, capability negotiation, and Persona/Agent session state. It is browser-runtime-clean (WebSocket, AudioContext, AudioWorklet, getUserMedia) and works identically in a plain web app and a Tauri webview. It contains zero Tauri or Node code.
- **Frontend Skins** (`apps/codec`, `apps/overlayz`, future skins) — presentation and Shell. Codec is a fullscreen/second-screen role-play skin, a web app today targeting Tauri. Overlayz is an unobtrusive overlay assistant skin, a Tauri app. KITT is not an app name; it survives as a Visual Theme plus a Persona bundled inside Overlayz.

Extensibility follows from the Client Core, not from a plugin framework. A new Frontend Skin is a new `apps/` member that imports `@foxline/voice-client` and renders its own UI; no runtime skin-loading registry exists. Within a skin, a **Visual Theme** is the in-app extension point: a swappable presentation paired with the active Persona. Overlayz ships two Visual Themes today (LED, Orb), which justifies the theme seam. Runtime loading and distribution of skins or themes is deferred until a concrete second deployment needs it; the stable interface is the extension point that survives, not the machinery.

The overlay app is copied from the standalone kitt repository into `apps/overlayz` as a plain copy (no git history graft) and renamed Overlayz. The kitt repository is abandoned as a historical record, not deleted. On copy, the legacy local STT/TTS/LLM path (eaRS, Kokoro, LM Studio, local VAD turn-commit) is removed; Overlayz is gateway-only. The gateway-vs-legacy A/B switch that existed in the separate kitt repo is deleted. The current `kitt` and `orb` UI modes inside the overlay app become Visual Themes (LED and Orb); KITT continues as a Persona and the LED Visual Theme paired with it.

The Client Core is TypeScript only. A Rust Client Core is deferred until a Rust-native frontend (for example GPUI) is real; with only TypeScript frontends today, a Rust Core would be one adapter against a hypothetical seam. The Protocol, not the Client Core, is the unit that crosses the Rust/TS boundary, so a future GPUI frontend shares the Protocol natively without forcing a shared cross-language Client Core.

OS-integration concerns (global shortcuts, tray, overlay window policy, system audio) belong to each skin's Shell (`src-tauri/` or equivalent), not to the Client Core. The Shell reaches the Client Core only through the Client Core's own interface and never crosses into the Protocol layer.

## Context

Adding the overlay app as a second frontend exposed that the gateway protocol was being written three times: once in Rust (`protocol.rs`) and independently hand-mirrored in each TypeScript client. Reconnect logic, PCM scheduling, and the eight-branch server-event mapping were duplicated verbatim across two repos, and an incoming protocol change (ADR 0003's client STT frame) would have landed in all three copies.

A future Rust-native frontend (GPUI) is a stated direction. That constraint is load-bearing: it is impossible to share one Client Core implementation across a TypeScript/AudioContext frontend and a Rust/cpal frontend. The shareable unit across that language seam is the Protocol, not the Client Core. Because the Gateway is Rust and `protocol.rs` is already the de-facto authority, making Rust the Protocol's source of truth is both correct and free: Rust frontends consume it natively, TS frontends consume a codegenerated derivative.

Codec and Overlayz are two flavors of one frontend over a shared brain, not two products. They are developed in lockstep against one gateway. That plus the shared Client Core and the Protocol question points at one monorepo rather than separate repositories with a published dependency.

The overlay app historically conflated the app identity (KITT) with one of its presentations (the LED bar). Separating the Skin (the app, Overlayz) from its Visual Themes (LED, Orb) makes the app general and lets the LED/Orb pair and the KITT persona be one option among many. Two Visual Themes already exist inside the overlay app, which makes the theme seam real rather than hypothetical.

The Voice Gateway is a long-running process that clients connect to. Standalone daemon is the initial deployment; bundling it as a Tauri sidecar for install-and-go apps is a later packaging step that does not change the Client Core, because the Core talks to a URL and does not care whether that URL is a system daemon or a spawned sidecar.

## Consequences

- Foxline becomes a monorepo holding both a Cargo workspace (`crates/`) and a Bun/TS workspace (`packages/`, `apps/`). `just` recipes and CI must orchestrate both toolchains.
- The kitt repository is abandoned. Its trx issues, history, and assets remain there as a record; ongoing overlay development happens in `apps/overlayz`.
- The overlay app is renamed Overlayz on copy. KITT survives as a Persona plus the LED Visual Theme, not as an app name.
- The legacy local STT/TTS/LLM path and the gateway-vs-legacy A/B switch are deleted on copy. Overlayz is gateway-only.
- `crates/protocol` is extracted from the gateway as a no-dependency serde crate; the gateway depends on it. A codegen step emits `packages/protocol` (TS types) as a build step.
- Both TypeScript clients (Codec, Overlayz) import `@foxline/protocol` and `@foxline/voice-client` and delete their hand-mirrored protocol types and duplicated audio plumbing. Three protocol drift sites collapse to one.
- The Client Core must stay free of Tauri and Node dependencies. Any OS capability a skin needs (global shortcuts, tray, overlay) is implemented in that skin's Shell, not in the Core.
- A Visual Theme seam exists inside Overlayz (LED and Orb are the first two themes). It is the in-skin extension point; it renders the active Persona and does not own it. Runtime theme loading is deferred.
- A Rust Client Core is not built now. It is built only when a Rust-native frontend is real, at which point the Protocol is already shared natively.
- ADR 0001's framing of the repo as a gateway plus a single Codec frontend is superseded by this monorepo, multi-skin, Protocol-centric structure. ADR 0001's core decision (Rust gateway, frame pipeline, gateway-owns-turns, Brain is Pi RPC) stands.
- ARCHITECTURE.md's repo-layout section will be updated when the extraction is performed, so it documents the built structure rather than describing it in advance.

## Considered options

- Keep the overlay app as a separate repository consuming a published `@foxline/*` package: rejected. It is a flavor of one frontend, not a product; a publish step adds ceremony with no external consumer, and the cross-repo dependency (path, published, or submodule) adds drift or friction that a monorepo avoids.
- Graft the overlay app's git history into the monorepo via `git subtree`: rejected. The kitt repository stays as a historical record; only the needed frontend code is copied. History does not need to follow the code.
- Build a runtime skin-loading framework so users can install and switch skins at runtime: rejected for now. Extensibility today is satisfied by the stable Client Core interface (new skin = new `apps/` member) and the Visual Theme seam (new presentation inside a skin). Runtime loading earns its cost only when a concrete deployment needs distribution of skins the app did not ship with.
- Build a Rust Client Core now so all frontends (including a future GPUI one) share one Core: rejected. Today there is only one Client Core platform (TypeScript); a second Rust Core would be a single adapter against a hypothetical seam. The Protocol is the unit that must be shared; the Client Core need not be, until a second platform is real.
- Hand-mirror the Protocol in each client: rejected. Three hand-maintained copies of the frame schema is the status quo this decision removes. The Protocol is the drift-prone, correctness-bearing artifact and must have exactly one source of truth.
