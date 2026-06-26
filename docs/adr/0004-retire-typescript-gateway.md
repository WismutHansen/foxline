---
status: accepted
supersedes: 0001 (oracle consequences)
---

# Retire the TypeScript gateway; Rust is the sole gateway

The Rust Voice Gateway is the only gateway. `server/bridge.ts` is retired. Frontend clients remain TypeScript: the Rust gateway is a server process, the frontend SDK that talks to it stays TypeScript, and any future client-side speech work (for example a transmission-gating VAD) is TypeScript, not Rust compiled to the browser.

## Decision

Retire `server/bridge.ts`. The Rust Voice Gateway in `crates/voice_gateway` is the sole voice gateway and the sole Brain path. There is no second gateway and no TypeScript gateway.

This also fixes the gateway-language boundary that ADR 0001 left implicit:

- The gateway is Rust. It owns the WebSocket transport, canonical frame pipeline, STT/TTS adapters, Pi RPC Brain lifecycle, turn authority, frontend tool routing, avatar action routing, and tracing.
- Frontend clients are TypeScript and stay TypeScript. KITT, the Foxline Codec UI, oqto, and future clients are React/Tauri (or web) frontends that connect to the gateway over the WebSocket protocol.
- The frontend SDK, including the gateway WebSocket client, the streaming audio player, the microphone PCM streamer, and any future transmission-gating VAD, is TypeScript. It is not Rust compiled to WASM. A full Rust end-to-end rewrite of the frontends was considered and is deferred; it is not this decision.

The codec UI no longer falls back to the TypeScript bridge. `bridge.ts`, the `CodecBridgeClient`, the `VITE_CODEC_BRIDGE_URL` / `/ws/bridge` path, and the `just bridge` recipes are removed. The codec UI's gateway client becomes the only client path.

## Context

ADR 0001 established the Rust gateway as the target but kept `server/bridge.ts` as the latency and behavior oracle, with a +50ms parity budget, until benchmark parity was proven. That parity has since been proven: the Rust gateway passes the +50ms budget for STT final, Brain first token, TTS first audio, frontend audio scheduling, and barge-in cancel, recorded in the Unreleased changelog. The oracle has served its purpose.

Keeping the bridge as a permanent fallback has a concrete cost: it is a second gateway implementation, the codec UI still branches between two clients, and every protocol change must be mirrored in two runtimes. With parity proven, that cost is no longer justified.

Retiring the bridge also forces the long-deferred language-boundary decision. The Rust gateway is a server process; a browser or Tauri frontend is a JavaScript host. Crossing that boundary to make the frontend SDK Rust-to-WASM would be a large rewrite of working React UIs for little benefit, since the frontend's job (WebSocket I/O, audio playback, mic capture, React rendering) is already well-served by TypeScript. The shared frontend SDK therefore stays TypeScript.

## Consequences

- `server/bridge.ts` is deleted.
- `just bridge` and `just bridge-new` recipes are removed.
- The codec UI's `CodecBridgeClient`, the `VoiceClient` union, and the `rustGatewayEnabled ? ... : ...` branch in `main.tsx` are removed. The gateway client is the only client.
- `defaultWsUrl('/ws/bridge')`, the `VITE_CODEC_BRIDGE_URL` env, and any code path that only exists to support the TypeScript bridge are removed.
- `codecrBridge.ts` should be renamed (for example to `gatewayClient.ts`) in a follow-up, since it will no longer contain a codec bridge; the rename is left as cleanup rather than done inline to keep this change reviewable.
- `benchmarks/run-legacy-bridge-fixture.ts` is removed, because it cannot run without the bridge it drives.
- `benchmarks/traces/legacy-bridge/` is kept as a frozen historical latency record. Future regression work compares against captured summaries, not against a live TypeScript gateway.
- ADR 0001's consequences that treat `server/bridge.ts` as the active oracle and that gate the gateway on +50ms deviation from the bridge are superseded. ADR 0001's core decision (Rust gateway with a frame pipeline, gateway-owns-turns, Brain is always Pi RPC) stands unchanged.
- The Rust gateway is the sole benchmark oracle going forward. New latency regressions are measured against the gateway's own baseline, not against a retired TypeScript reference.
- KITT's integration, once done, connects to this single Rust gateway. There is no TypeScript-gateway path for KITT to fall back to.

## Considered options

- Keep `server/bridge.ts` as a permanent fallback: rejected because parity is proven, the bridge is a second gateway to maintain, and the codec UI's two-client branch is accidental complexity.
- Defer retirement until KITT lands: rejected because KITT should integrate against the final architecture, not a transitional two-gateway state, and the retirement is independently justified by proven parity.
- Rewrite the frontends in Rust (Dioxus/Leptos/native) and share a Rust client crate compiled to WASM: rejected for now. It is the only world in which "the WASM VAD lives in Rust code" is coherent, but it rewrites two working React frontends for little gain. Deferred, not abandoned; captured here so the boundary is explicit.
- Keep the legacy bridge benchmark fixture runnable: rejected because it requires keeping `server/bridge.ts`. The captured traces are retained as a frozen historical record instead.
