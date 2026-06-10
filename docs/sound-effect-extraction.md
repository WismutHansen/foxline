# Metal Gear Solid PSX sound-effect extraction from STAGE.DIR

## Bottom line

I did **not** find a public, already-finished master map that says “the alert sound is exactly bank X, sample Y, effect Z” for the NTSC/US PS1 discs. What the public sources *do* give you is enough to build that map reproducibly: MGS1 keeps **voices** in `VOX.DAT`, but keeps **stage-local audio** inside `STAGE.DIR`, where `*.sw` files are wave archives and `*.se` files are effect banks. That matches your experience of “voices were easy, sound effects were not.” citeturn73view0turn61view0

Your proposed key of **`stage folder + .sw sample index + .se effect index`** is the right abstraction. Public tooling and docs show that MGS1 file names are hash-based or nickname-based, not clean semantic labels, and modern unpackers explicitly warn that the game’s 16-bit hashing is weak and collision-prone. In practice, raw indices are more stable than guessed friendly names. citeturn61view0turn86view2

A separate music follow-up is also justified. Public format notes treat `*.sm` as a different stage file type from `*.sw` and `*.se`, and older community tools include a converter for the same type byte under a different alias, outputting MIDI rather than final rendered audio. So SFX extraction and music/sequence extraction are related, but they are **not the same problem**. citeturn73view0turn83view0turn85view4

## What the stage-audio formats actually are

The strongest public technical reference is Martin Korth’s PSXSPX page on MGS1 archives. It documents `STAGE.DIR` as a foldered archive whose stage files begin on `0x800`-byte boundaries, and it explicitly identifies the important audio-side stage file types as `.se` for sound effects, `.sw` for wave archives, and `.sm` for sound/music. It also notes the older nickname scheme used by some tools: `.se` ↔ `efx`, `.sw` ↔ `wvx`, and `.sm` ↔ `mt3`. citeturn73view0

For `*.sw`, PSXSPX describes a header, a file list of `N*0x10` bytes, and an SPU-ADPCM data area. Each file-list entry contains a little-endian `Offset+Flags` field that points into the shared ADPCM payload. In other words, the wave bank is already organized as a list of indexed sample entries, which is exactly why a “sample index” is a sensible stable identifier for your manifest. citeturn61view0

For `*.se`, PSXSPX describes a fixed entry table of `0x80 * 0x10` bytes, followed by a data area. Each entry has a “number of voices” field and up to three offsets into the data region, with unused voices marked `FFFFFFFFh`. That is strong evidence that a `.se` bank is **not** raw sample storage; it is an effect-program layer that can trigger or combine one to three voices per effect slot. The docs stop short of fully decoding the opcode stream, but structurally this is the level where “effect index” belongs. citeturn61view0

This also explains why speech extraction feels different. PSXSPX documents `VOX.DAT` as containing large blocks of SPU-ADPCM chunks, while stage audio lives inside per-stage archives in `STAGE.DIR`. So “voices” and “famous UI/alert SFX” really do come from different parts of the disc layout. citeturn73view0

## What the public tools already solve

There is a clear public tool lineage for unpacking MGS1 stage audio, even if the modern script names differ from the ones in your note. In the older `metalgeardev/MGS1` toolset, `unStage.rb` extracts `stage.dir` into per-stage `.stg` blobs, and `unStg.rb` then splits those blobs into typed files using a mapping that includes `efx` for effect banks, `wvx` for wave banks, and `mdx` for the music-side type byte. citeturn81view0turn83view0turn74view0

That older toolset also includes **dedicated converters** for the sample side and the sequence side. `unWVX.rb` says it “Extracts wvx to vag,” and the code walks the bank payload, writes numbered `.vag` files, and then emits a `data.txt` that maps each table entry’s relative offset back to the numbered output sample. `unMDX.rb` says it will “Convert mdx to midi,” creates standard `MThd`/`MTrk` MIDI output, and iterates songs and tracks out of the source file. So the public state of the art is not “nothing exists”; it is “wave and sequence extraction exist, but `.efx/.se` is the least-finished layer.” citeturn71view0turn72view0turn85view0turn85view4

Modern extractors solve the archive side more cleanly. `Rex` is a CLI tool that extracts `stage.dir` directly. `tools-mgs` includes a `stage-extract` tool for unpacking MGS1 `STAGE.DIR` and supports dictionaries for naming. DoktorDeSparkle’s `stageDirFileExtractor.py` independently reproduces the stage-table logic, using `0x800`-byte block math and exporting the files of a chosen stage. These tools validate your overall pipeline even if your own local script is named differently. citeturn59view0turn47view1turn86view2turn50view0

One important limitation is naming. `tools-mgs` explicitly warns that MGS1 uses an “extremely weak 16bit hashing” scheme that is very collision-prone, and PSXSPX notes that some file names are unknown while others can only be recovered partially from hashes or strings found in `.sb` files. So if you want a durable SFX map, you should treat human-readable names as **helpful annotations**, not as your primary key. citeturn86view2turn61view0

## How to interpret your bank and effect counts

I could not independently verify a public source that publishes the exact aggregate totals `sw banks: 145`, `samples: 2043`, `se banks: 75`, `effects: 4896`. The public docs and repos I found describe the formats and extraction chain, but they do not publish a finished global census of all banks and effects. That means those numbers are best treated as the output of a particular extraction run rather than as a count already canonicalized in public docs. citeturn61view0turn59view0turn86view2

Even so, the numbers are **internally plausible**. A `.se` bank has room for `0x80 = 128` entry slots, so `75` banks would expose a theoretical maximum of `9600` slots. Your `4896` populated effects would therefore represent about **51% occupancy**, or about **65.28 effects per bank on average**, which is perfectly believable for stage-local banks with many unused or reserved slots. citeturn61view0turn80calculator1turn80calculator2turn80calculator3

The wave-bank side is also plausible. PSXSPX explicitly says a stage can have **one or more** `.sw` files, with VR stages specifically called out as having two in some cases. Against that backdrop, `145` wave banks holding `2043` decoded samples works out to about **14.09 samples per bank on average**, which sounds reasonable for stage-scoped banks rather than one giant global archive. citeturn61view0turn80calculator0

The counts should also be treated as **version- and disc-specific** until proven otherwise. PSXSPX shows that archive sizes differ across demo and retail builds, and DoktorDeSparkle’s repo is structured per `usa-d1`, `usa-d2`, `jpn-d1`, and `jpn-d2`, which is a good reminder that a reproducible manifest must record the exact disc/version it came from. citeturn73view0turn62view0

## A reproducible workflow that will actually get you to the alert sounds

The clean public workflow is basically an updated version of the old Ruby chain. First unpack the correct `STAGE.DIR` for the specific NTSC/US disc with a stage extractor such as `Rex`, `tools-mgs stage-extract`, or an equivalent parser like `stageDirFileExtractor.py`. The old `unStage.rb`/`unStg.rb` pair proves the same intermediate model: `stage.dir` becomes per-stage blobs, then each stage blob becomes typed files such as `*.efx`, `*.wvx`, and the music-side `*.mdx`. citeturn59view0turn86view2turn81view0turn83view0

From there, decode the `*.sw`/`*.wvx` banks first. `unWVX.rb` is valuable here because it does more than just dump raw chunks: it writes **numbered `.vag` files** and a `data.txt` whose entries are resolved by comparing each bank-table relative offset against the discovered sample start offsets. That is very close to the “sample index” layer you want in a modern `stage_audio/samples` output tree. citeturn71view0turn72view0

Then parse the `*.se`/`*.efx` banks as effect-index catalogs, not as audio. Public format notes tell you exactly what to preserve in your manifest: bank filename, effect slot `0..127`, voice count, and up to three voice-data offsets per entry. Because the effect opcode stream is only partially documented publicly, the safest reproducible artifact is a manifest that keeps **raw offsets and raw bytes** alongside any audition-based label you assign later. citeturn61view0

At that point the join key should be exactly what you proposed: **stage folder + bank file + sample index + effect index**. That is stronger than any filename-based scheme because MGS1 names are hash-derived and collision-prone, and because some source names are only partially recoverable from the stage `.sb` binaries. If you want nicer labels, use dictionaries and mine `.sb` strings, but keep the raw IDs and indices in your final map. citeturn61view0turn86view2

The practical consequence for “famous alert/error/etc.” is that you should **not** expect one neat global `alert.wav` somewhere on disc. The archive design is stage-scoped, and the public tooling history reflects stage-by-stage unpacking rather than a single universal SFX table. So the right procedure is exactly what you outlined: unpack stages, decode wave banks, enumerate effect banks, audition samples, then annotate the effect/sample combinations that correspond to iconic cues. citeturn61view0turn81view0turn83view0

## Why the music follow-up is worth doing

Yes, the music follow-up is worth triggering, because the public evidence shows a separate and more ambiguous music layer. PSXSPX describes `.sm` as “Sound Music?” and says it resembles a **nested parent/child archive**, while the old `unStg.rb` tool maps the same type byte (`0x6d`, type `m`) to the alias `mdx`, and `unMDX.rb` then converts that file into MIDI. That combination strongly suggests the music-side asset is **sequence-oriented** and custom-wrapped, not just a stock raw-audio container. citeturn73view0turn83view0turn85view4

That old converter is important because it means the music problem is not starting from zero. `unMDX.rb` treats the source as a chunked multi-song file, emits MIDI headers, loops over songs, and writes up to 24 tracks, including handling custom loop-like events such as `CueIni` and `CueEnd`. So a specialized follow-up should absolutely include the historical `mdx` alias in addition to your newer `.sm / mt3` terminology. citeturn85view0turn85view3

What is *not* solved by that older MIDI conversion is final authentic playback. MIDI extraction gives you note/sequence structure, but the actual PS1 timbre still depends on the game’s sample banks and playback driver. That is why PSF and miniPSF are relevant in a follow-up: PSF is specifically designed to package PlayStation sequenced music together with the original playback code and needed sample data for emulated reproduction. citeturn85view0turn51search0

So the split between the two research tracks is now pretty clear. For **SFX**, the public path is mature enough to justify your bank/sample/effect census workflow right now. For **music**, the right follow-up question is narrower and more technical: reconcile the `.sm` / `mt3` / `mdx` naming, determine how the sequence files relate to `*.sw` banks, and see whether the old MIDI path can be modernized or cross-checked against PSF-style playback. citeturn73view0turn83view0turn85view4turn51search0
## Update (2026-06-10): solved — driver simulation renders SFX at correct pitch

The `.se/.efx` opcode layer is no longer a blocker. The MGS1 decompilation
([FoxdieTeam/mgs_reversing](https://github.com/FoxdieTeam/mgs_reversing),
`source/sound/`) contains the complete sound driver in C, which settles every
open question:

- SFX are **4-byte sequencer commands** `[vel, gate, len, op]`: `op < 0x80` is
  a note (pitch via the driver's 108-entry `freq_tbl`, semitone-indexed,
  `0x1000` = 44100 Hz), `op >= 0x80` indexes a 128-entry control table
  (`0xD0` tempo, `0xD2` sample select, `0xD7/D8/D9` ADSR, `0xE7/E8` loops,
  vibrato/portamento/pan/etc., `0xFF` end).
- The previously-unknown 12 bytes per `.sw` sample entry are `WAVE_W`: root
  note, fine tune, ADSR (attack/decay/sustain/release + modes), pan, declick
  volume.
- The **global UI sounds (codec call, tuning, alert, menu, item)** never were
  in STAGE.DIR: they are a 128-entry `se_tbl` compiled into the game
  executable, with sequences pointing at the resident `init`-stage wave banks.
  This is why no amount of stage unpacking could find them.

`tools/render_mgs_sfx.py` reimplements the sequencer plus a software SPU
(ADPCM decode, pitch, full ADSR envelope, pan) and renders every SE offline
from a user-provided disc extraction — no emulator, no bundled game data. The
built-in table is located in `SLUS_005.94` by signature scan; stage `.se`
banks render the same way layered over the resident banks.

Validation: a cycle-stamped PCSX-Redux SPU register trace of the in-game codec
call shows voice pitches alternating `0x12CB/0x17AD` with ADSR1 `0x007C` —
exactly what the rendered `codec_call` sequence produces (notes 0x32/0x36 →
`freq_tbl[50]/[54]`, `ads_set 0C,08,7F`).

Sequencer timing uses two empirically calibrated rates: SE tracks tick at
~95.3 Hz (cycle-stamped PCSX-Redux trace of the in-game codec ring,
corroborated by the PC port's official ring recording), while music tracks
run 1.935x faster (~184.4 Hz), calibrated by onset-flux alignment against
the PC port's official music recordings across multiple songs. The
mechanism behind the differing rates lives in the game's MTS interrupt
plumbing (unrecovered symbols in the decompile); `--tick-hz` and
`--music-tick-hz` override the measured defaults.

Usage: `just mgs-sfx-render` (or automatically via `just mgs-install-psx ...`);
`uv run tools/render_mgs_sfx.py --list ...` shows all named built-ins.

Music (`.sm`/`.mdx`) uses the **same command format** on 13 song tracks — the
sequencer in `render_mgs_sfx.py` is the bulk of a future music renderer; see
the phase-2 issue.

### PC port source media

The renderer is PSX-only by necessity: the PC port's `stage.mgz` retains the
`.wvx/.efx/.mdx` file entries but **all of them are 0-byte stubs** — the
sequencer audio system was removed and replaced with pre-rendered files
(`efx/` = unsigned 8-bit 11 kHz mono WAVs, `MDX/` = 8-bit 22 kHz stereo
music). PC installs therefore get only the `normalize_mgs_pc_efx.py` cleanup
path; the PSX driver-sim renders are higher fidelity than the PC port's own
shipped effects.

## Update: music rendering + frontend integration

**Music** now renders through the same driver simulation. `.mdx` files (one
per stage in STAGE.DIR, 35 unique) hold up to 8 songs each: byte 0 = song
count, per-song 16-bit table offset, then 13 track pointers (24-bit) into the
same 4-byte command stream the SFX use. `render_mgs_sfx.py --music` renders
every unique song (84 on disc 1) against the stage's wave banks layered over
the `init` residents, in parallel workers, stopping after two passes of each
track's infinite `l3` loop (`looping: true` in the manifest, `peak` included
for normalization). Runs by default in `just mgs-install-psx`
(`--skip-music` to opt out).

**Frontend**: `src/services/sfx.ts` Web-Audio player globs the rendered
`builtin` WAVs (only the ones the UI uses get bundled) with per-sound peak
normalization and graceful no-op when assets are not installed. Wired in
`src/main.tsx`: codec_call on first activation, codec_tune on character
switch, radio_window_open/close + radio_cursor + radio_select in the memory
overlay, radio_cancel on interrupt, codec_noise on bridge loss.

### Disc 2

Disc 1 is sufficient for all SFX and music: STAGE.DIR is fully duplicated on
disc 2 (verified by hash — all 72 `.m`, 145 `.w`, 75 `.e` payloads are
byte-identical) and the built-in `se_tbl` matches between SLUS_005.94 and
SLUS_007.76. The discs differ only in streamed voice/cutscene data
(VOX.DAT/DEMO.DAT/ZMOVIE.STR), which is why only Naomi's voice reference
needs disc 2.

### SE name provenance

The built-in SE names in `render_mgs_sfx.py` (`BUILTIN_SE_NAMES`) are NOT
official Konami asset names. They come from the C identifiers the
decompilation project assigned to each sequence blob in `se_tbl.c`
(`r_snd0100`, `bikkuri00`, `kaihuku100`, ... — at least one is commented
"guessed name" upstream), normalized and translated here (r_* = radio/codec,
kaihuku = recovery, bikkuri = the surprise "!" alert). `codec_call` (se086,
`r_snd0100`) is additionally verified against the PCSX-Redux SPU trace of the
real in-game codec ring. The PC-port names in `assets/efx-name-map.json` were
confirmed by user audition. Treat the rest as informed labels; the manifest
keeps the stable numeric codes.

### PC fallback for frontend sounds

PC-only installs get a degraded-but-working UI sound set:
`pc_install_sfx_fallback` converts the user-auditioned PC efx WAVs
(8-bit/11kHz originals) to `assets/generated/mgs_pc/sfx/builtin/` using the
same `seNNN_<name>.wav` naming, and `src/services/sfx.ts` globs both
locations with the PSX renders taking precedence. The PC port's music is
pre-rendered too (the `MDX/` folder kept the PSX sequence format's name
but contains 41 unsigned 8-bit 22kHz stereo WAV recordings — no sequence
data); `pc_install_music` converts them to 16-bit/44.1kHz under
`assets/generated/mgs_pc/sfx/music/`. `scripts/install-assets.sh` now
prefers PSX media when both are present in `sources/`.

### Render quality: reverb + interpolation (and tempo ground truth)

Music tempo was definitively validated against the joshw PSF rip: rendering
"03 DISCOVERY.psf" through the game's original driver (Mednafen + PS1 BIOS,
`-psx.bios_sanity 0`) and aligning onset flux against our render of the same
song converges at stretch 1.00 for the 184.4 Hz music tick. The PC port's
official music recordings agree; the PC port's ring recording and the PCSX
trace agree on ~95 Hz for SE — the two-rate split is real.

What made correct-tempo renders sound "garbled with repeated sounds" was
missing post-processing, now implemented: the SPU reverb unit (psx-spx
formula, Studio Large preset = libspu SPU_REV_MODE_STUDIO_C, depth 0x4000,
22050 Hz) applied to all music channels and to SEs that issue `eon` (codec
ring!), plus 4-point Catmull-Rom interpolation instead of linear, plus
per-song -1 dBFS makeup gain (`gain` recorded in the manifest). Flux match
vs the PSF ground truth improved from 0.365 (dry) to 0.486.

### Correction: ONE tick rate (98.4 Hz); never trust PC-port music timing

The earlier "music ticks 1.935x faster" conclusion was wrong — it came from
calibrating against the PC port's MDX recordings, whose music is documented
as broken (wrong pitch/speed/loops; the port team never had Konami's sound
mixer). Re-validating against songs rendered through the game's ORIGINAL
driver (joshw PSF rip, which is root-counter timed and player-independent):
both Encounter and Duel beat grids match a single sequencer tick of
**98.4 Hz** to within 0.3% — for SFX and music alike. 98.4 Hz = 2x the
49.22 Hz blank-voice SPU-IRQ loop, and is consistent with the PCSX trace
(95.3 measured, emulator clock skew) and the PC ring (~100 Hz). The PSF set
is the only trustworthy external tempo reference.
