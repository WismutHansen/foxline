# Migration note

Foxline is the active source-of-truth repository for the Codec-style runtime and asset installer.

The older `codec` working directory is an archive/scratch area. It may contain generated local assets, experiments, benchmark reports, and historical cleanup work, but new product development should happen here in `foxline`.

## Current policy

- Make runtime, installer, documentation, and release changes in `foxline`.
- Treat `codec` as read-only unless recovering historical experiments or generated local test assets.
- Do not copy generated copyrighted/runtime outputs into `foxline`.
- Track publish-readiness work under trx epic `fxl-yftd`.

## Active architecture

Foxline uses:

- pi-rpc brain runtime
- parakeet.cpp STT, default model `tdt-0.6b-v3-q8_0.gguf`
- Qwen3-TTS worker-based TTS
- user-provided MGS source media only

See `ARCHITECTURE.md` and `STRUCTURE.md` for details.
