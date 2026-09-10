---
status: accepted
---

# Pair a phone client with a work-directory-scoped Foxline server

`foxline setup` records a local server profile for the current canonical work directory; `foxline serve` runs that profile. A short-lived, single-use pairing code exchanges over trusted HTTPS for a revocable device credential. The QR includes the server address and code; manual pairing needs both. No cloud model, central rendezvous, unauthenticated tunnel, or certificate bypass is implied. The gateway remains private on loopback, and its upstream credential never reaches the phone. The paired proxy pins session creation to the profile's work directory and rejects unsupported controls rather than forwarding arbitrary workspace choices.

The canonical web client is also the Tauri mobile client: one UI, with native credential storage and platform permissions. Multiple paired connections are saved separately, but only one captures the microphone at a time. Switching stops capture/playback on the previous connection and does not mix its transcript/draft into the next. Native credentials belong in Apple Keychain; browser fallback credentials are session-local and require pairing again after the page's lifetime. Native installation still requires a trusted device and Apple signing.

Connection, responsibility and presentation remain distinct. An Agent/loadout defines work and tool grants; a Persona supplies voice and character; a Frontend Skin/Visual Theme supplies the look. Future character shortcuts may combine these deliberately (Colonel/Govnr, Otacon/wiki, Mei Ling/issues), but appearance is never authorization. Codec is retained as a specialist Skin and KITT as a Visual Theme/Persona option. Copyrighted game assets remain user-provisioned and are not bundled into distributable phone packages.

This connection boundary is not a sandbox for Pi or proof of async tools. The initial PhoneLLM/Pi/spqx path remains local; whole-process confinement, async operation delivery and Runner attachment are separately verified work. Tracked by `fxl-2nfq`, `fxl-pras`, and `fxl-7snb`.
