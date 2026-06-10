#!/usr/bin/env python3
"""Render MGS1 (PSX) sound effects at correct pitch by simulating the game's
sound driver and the PS1 SPU.

MGS1 does not store sound effects as playable audio files. SFX are tiny
sequencer programs (4-byte commands) that play SPU-ADPCM samples from wave
banks with runtime pitch, ADSR envelopes, volume and pan. This tool replays
those programs offline:

- Built-in/global SEs (codec call, frequency tuning, alert, menu, item UI...)
  live as an embedded 128-entry table inside the game executable
  (SLUS_005.94). They are located by signature scan and rendered against the
  resident `init` stage wave banks.
- Stage SEs live in `.se` effect banks inside STAGE.DIR and are rendered
  against that stage's wave banks layered over the resident banks.

Everything is parsed from the user-provided disc extraction; no game data is
bundled. Format knowledge: MGS decompilation (FoxdieTeam/mgs_reversing
source/sound) and psx-spx SPU documentation.

Example:
  uv run tools/render_mgs_sfx.py \
    --filesystem assets/generated/mgs/disc_1/filesystem \
    --out assets/generated/mgs/disc_1/sfx_rendered
"""
from __future__ import annotations

import argparse
import json
import struct
import wave
from dataclasses import dataclass, field
from pathlib import Path

OUT_RATE = 44100
# Sequencer tick rate: 98.4 Hz = 2x the 49.22 Hz SPU-IRQ loop of the driver's
# blank timing voice (512-byte / 896-sample loop at 44100 Hz). Validated three
# independent ways: beat periods of songs rendered through the game's ORIGINAL
# driver via the joshw PSF rip (Encounter and Duel both match within 0.3%),
# a cycle-stamped PCSX-Redux SPU trace of the in-game codec ring (95.3 Hz
# +- emulator clock skew), and the PC port's ring recording (~100 Hz). Do NOT
# calibrate against the PC port's music recordings: that port's music is
# documented as broken (wrong pitch/speed/loops). Pitch is clock-independent.
TICK_HZ = 98.4
MUSIC_TICK_HZ = TICK_HZ
MAX_SECONDS = 30.0
SILENCE_FLOOR = 16  # envelope level below which a released voice is dead

# ---------------------------------------------------------------------------
# Note -> SPU pitch table from the sound driver (freq_tbl). Index is a
# semitone; 0x1000 = 44100 Hz. Entries 72+ are sub-semitone fine values.
# ---------------------------------------------------------------------------
FREQ_TBL = [
    0x010B, 0x011B, 0x012C, 0x013E, 0x0151, 0x0165, 0x017A, 0x0191,
    0x01A9, 0x01C2, 0x01DD, 0x01F9, 0x0217, 0x0237, 0x0259, 0x027D,
    0x02A3, 0x02CB, 0x02F5, 0x0322, 0x0352, 0x0385, 0x03BA, 0x03F3,
    0x042F, 0x046F, 0x04B2, 0x04FA, 0x0546, 0x0596, 0x05EB, 0x0645,
    0x06A5, 0x070A, 0x0775, 0x07E6, 0x085F, 0x08DE, 0x0965, 0x09F4,
    0x0A8C, 0x0B2C, 0x0BD6, 0x0C8B, 0x0D4A, 0x0E14, 0x0EEA, 0x0FCD,
    0x10BE, 0x11BD, 0x12CB, 0x13E9, 0x1518, 0x1659, 0x17AD, 0x1916,
    0x1A94, 0x1C28, 0x1DD5, 0x1F9B, 0x217C, 0x237A, 0x2596, 0x27D2,
    0x2A30, 0x2CB2, 0x2F5A, 0x322C, 0x3528, 0x3850, 0x3BAC, 0x3F36,
    0x0021, 0x0023, 0x0026, 0x0028, 0x002A, 0x002D, 0x002F, 0x0032,
    0x0035, 0x0038, 0x003C, 0x003F, 0x0042, 0x0046, 0x004B, 0x004F,
    0x0054, 0x0059, 0x005E, 0x0064, 0x006A, 0x0070, 0x0077, 0x007E,
    0x0085, 0x008D, 0x0096, 0x009F, 0x00A8, 0x00B2, 0x00BD, 0x00C8,
    0x00D4, 0x00E1, 0x00EE, 0x00FC,
]

PANT = [
    0, 2, 4, 7, 10, 13, 16, 20, 24, 28, 32, 36, 40, 45,
    50, 55, 60, 65, 70, 75, 80, 84, 88, 92, 96, 100, 104, 107,
    110, 112, 114, 116, 118, 120, 122, 123, 124, 125, 126, 127, 127,
]

SE_PANT = [
    0, 2, 4, 6, 8, 10, 14, 18, 22, 28, 34, 40, 46,
    52, 58, 64, 70, 76, 82, 88, 94, 100, 106, 112, 118, 124,
    130, 136, 142, 148, 154, 160, 166, 172, 178, 183, 188, 193, 198,
    203, 208, 213, 217, 221, 224, 227, 230, 233, 236, 238, 240, 242,
    244, 246, 248, 249, 250, 251, 252, 253, 254, 254, 255, 255, 255,
]

VIBX_TBL = [
    0, 32, 56, 80, 104, 128, 144, 160,
    176, 192, 208, 224, 232, 240, 240, 248,
    255, 248, 244, 240, 232, 224, 208, 192,
    176, 160, 144, 128, 104, 80, 56, 32,
]

RDM_TBL = [
    159, 60, 178, 82, 175, 69, 199, 137,
    16, 127, 224, 157, 220, 31, 97, 22,
    57, 201, 156, 235, 87, 8, 102, 248,
    90, 36, 191, 14, 62, 21, 75, 219,
    171, 245, 49, 12, 67, 2, 85, 222,
    65, 218, 189, 174, 25, 176, 72, 87,
    186, 163, 54, 11, 249, 223, 23, 168,
    4, 12, 224, 145, 24, 93, 221, 211,
    40, 138, 242, 17, 89, 111, 6, 10,
    52, 42, 121, 172, 94, 167, 131, 198,
    57, 193, 180, 58, 63, 254, 79, 239,
    31, 0, 48, 153, 76, 40, 131, 237,
    138, 47, 44, 102, 63, 214, 108, 183,
    73, 34, 188, 101, 250, 207, 2, 177,
    70, 240, 154, 215, 226, 15, 17, 197,
    116, 246, 122, 44, 143, 251, 25, 106,
    229,
]

# Names for the 128 built-in SE slots, from the MGS decompilation's se_tbl.
BUILTIN_SE_NAMES = {
    1: "shot", 2: "step_l", 3: "step_r", 4: "karasht", 5: "hohuku_3",
    6: "hohuku_4", 7: "senaka", 8: "stand", 9: "kamae", 10: "heartbeat",
    11: "life_full", 12: "kaihuku", 13: "item", 14: "radar", 15: "game_over",
    16: "radio_receive", 17: "photo", 18: "player_out_3", 19: "sneeze",
    20: "item_decide_3", 21: "item_display", 22: "item_get", 23: "item_select",
    24: "player_damage_1", 25: "player_damage_2", 26: "player_out_1",
    27: "rifle", 28: "step_2", 29: "mouse_step", 30: "water_step",
    31: "cursor", 32: "window_open", 33: "menu_start", 34: "item_decide_2",
    35: "buzzer", 36: "scope_zoom", 37: "hit", 38: "enemy_damage",
    39: "ricochet", 40: "ricochet_drum", 41: "explosion_5", 42: "siren",
    43: "bounce", 44: "grenade_pin", 45: "shot_enemy_3", 46: "shot_enemy_2",
    47: "reload", 48: "famas", 49: "c4_put", 50: "c4_switch", 51: "body_down",
    52: "punch_hit", 53: "kick_hit", 54: "item_select_3", 55: "wall_hit",
    56: "punch_swing", 57: "kick_swing", 58: "chaff_3", 59: "spark",
    60: "panel", 61: "chair", 62: "ricochet_glass", 63: "glass_break",
    64: "paper", 65: "explosion_bakuha", 66: "chaff_2", 67: "back_close_1",
    68: "back_close_2", 72: "se_unk_72", 73: "shot_m2", 74: "face_change",
    75: "run", 76: "nikita_1", 77: "nikita_2", 78: "ninja_1", 79: "lockon",
    80: "ninja_2", 81: "knee", 82: "shot_s1", 83: "alert_bikkuri",
    84: "radio_window_open", 85: "radio_select", 86: "codec_call",
    87: "radio_window_close", 88: "door_close_3", 89: "door_close_4",
    90: "door_close_5", 91: "door_open_3", 92: "door_open_4",
    93: "door_open_5", 94: "camera_7", 95: "camera_3", 96: "button",
    97: "elevator_close", 98: "elevator_open", 99: "sight", 100: "in_elevator",
    101: "shot_s2", 102: "stage_start", 103: "codec_tune",
    104: "radio_cancel", 105: "radio_cursor", 106: "o2_damage",
    107: "player_out_2", 108: "codec_noise", 109: "camera_6",
    110: "shatter_4", 111: "shatter_6", 112: "elevator_chime",
    113: "elevator_stop", 114: "gas_mask", 115: "ration", 116: "signal_2",
    117: "clock", 118: "mine_mask", 119: "signal_4", 120: "radar_3",
    121: "jingle_1", 122: "jingle_2", 123: "char_display", 124: "shatter_b",
    125: "menu_open", 126: "kaihuku_4", 127: "item_decide_4",
}

ADPCM_FILTERS = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)]


def clamp16(v: int) -> int:
    return max(-32768, min(32767, v))


def s8(v: int) -> int:
    return v - 256 if v >= 128 else v


# ---------------------------------------------------------------------------
# STAGE.DIR parsing (psx-spx layout: folder list + 0x800-block aligned files)
# ---------------------------------------------------------------------------

def parse_stage_dir(data: bytes) -> dict[str, list[dict]]:
    """Return {folder_name: [ {id, family, type, offset, size}, ... ]}."""
    list_size = struct.unpack_from("<I", data, 0)[0]
    folders = []
    for pos in range(4, 4 + list_size, 12):
        name = data[pos : pos + 8].rstrip(b"\0").decode("ascii", errors="replace").strip()
        off_blocks = struct.unpack_from("<I", data, pos + 8)[0]
        folders.append((name or "folder", off_blocks * 0x800))

    result: dict[str, list[dict]] = {}
    for name, base in folders:
        if base + 4 > len(data):
            continue
        leading, trailing, c_entries = [], [], []
        c_end = None
        phase = "leading"
        pos = base + 4
        while pos + 8 <= len(data):
            file_id, fam_i, typ_i, value = struct.unpack_from("<HBBI", data, pos)
            pos += 8
            if fam_i == 0 and typ_i == 0 and value == 0:
                continue
            fam, typ = bytes([fam_i]), bytes([typ_i])
            if fam == b"c" and typ_i == 0xFF:
                c_end = value
                phase = "trailing"
                continue
            if fam == b"c":
                c_entries.append((file_id, fam, typ, value))
                phase = "c"
            elif phase == "trailing":
                trailing.append((file_id, fam, typ, value))
            else:
                leading.append((file_id, fam, typ, value))
            if pos >= base + 0x800:
                break

        rows = []
        cur = base + 0x800
        for file_id, fam, typ, size in leading:
            rows.append((file_id, fam, typ, cur, size))
            cur += (size + 0x7FF) & ~0x7FF
        if c_entries:
            c_area = cur
            offs = [e[3] for e in c_entries]
            ends = offs[1:] + [c_end if c_end is not None else 0]
            for (file_id, fam, typ, rel), nxt in zip(c_entries, ends):
                if nxt > rel:
                    rows.append((file_id, fam, typ, c_area + rel, nxt - rel))
            cur = c_area + (((c_end or 0) + 0x7FF) & ~0x7FF)
        for file_id, fam, typ, size in trailing:
            rows.append((file_id, fam, typ, cur, size))
            cur += (size + 0x7FF) & ~0x7FF

        result[name] = [
            {"id": fid, "family": fam.decode(errors="replace"),
             "type": typ.decode(errors="replace"), "offset": off, "size": size}
            for fid, fam, typ, off, size in rows
        ]
    return result


# ---------------------------------------------------------------------------
# Driver memory model: 256-entry voice table + virtual SPU RAM
# ---------------------------------------------------------------------------

@dataclass
class WaveEntry:
    addr: int = 0          # byte offset into virtual SPU wave RAM
    sample_note: int = 0   # signed: root note added to (note_tune >> 8)
    sample_tune: int = 0   # signed: fine tune added to note_tune
    a_mode: int = 0
    ar: int = 0
    dr: int = 0
    s_mode: int = 0
    sr: int = 0
    sl: int = 0
    r_mode: int = 0
    rr: int = 0
    pan: int = 0
    decl_vol: int = 0
    present: bool = False


class SoundRam:
    """Mirrors the driver's wave_header/voice_tbl + SPU sample RAM."""

    def __init__(self) -> None:
        self.voice_tbl = [WaveEntry() for _ in range(256)]
        self.spu_ram = bytearray(512 * 1024)

    def load_sw(self, payload: bytes) -> dict:
        """Apply one .sw/.wvx wave bank exactly like LoadWaveHeader does:
        big-endian [table_dest_offset, table_size], 16-byte WAVE_W entries,
        then big-endian [spu_dest_offset, data_size] and SPU-ADPCM data."""
        tbl_off, tbl_size = struct.unpack_from(">II", payload, 0)
        entries = payload[16 : 16 + tbl_size]
        pos = 16 + tbl_size
        spu_off, data_size = struct.unpack_from(">II", payload, pos)
        adpcm = payload[pos + 16 : pos + 16 + data_size]

        base_index = tbl_off // 16
        count = len(entries) // 16
        for i in range(count):
            raw = entries[i * 16 : i * 16 + 16]
            addr = struct.unpack_from("<I", raw, 0)[0]
            e = self.voice_tbl[(base_index + i) % 256]
            e.addr = addr
            e.sample_note = s8(raw[4])
            e.sample_tune = s8(raw[5])
            e.a_mode, e.ar, e.dr, e.s_mode = raw[6], raw[7], raw[8], raw[9]
            e.sr, e.sl, e.r_mode, e.rr = raw[10], raw[11], raw[12], raw[13]
            e.pan, e.decl_vol = raw[14], raw[15]
            e.present = True
        end = spu_off + len(adpcm)
        if end > len(self.spu_ram):
            self.spu_ram.extend(b"\0" * (end - len(self.spu_ram)))
        self.spu_ram[spu_off:end] = adpcm
        return {"table_index": base_index, "entries": count,
                "spu_offset": spu_off, "data_size": len(adpcm)}


# ---------------------------------------------------------------------------
# SE tables: built-in (game EXE) and stage .se/.efx banks
# ---------------------------------------------------------------------------

@dataclass
class SeEntry:
    pri: int
    tracks: int
    kind: int
    character: int
    seqs: list[bytes]
    name: str = ""


def read_sequence(data: bytes, off: int) -> bytes:
    """Read 4-byte command rows until block_end (mdata1 == 0xFF) inclusive,
    or a zero command byte (driver treats it as end of data)."""
    rows = []
    pos = off
    while pos + 4 <= len(data) and len(rows) < 4096:
        row = data[pos : pos + 4]
        rows.append(row)
        if row[3] == 0xFF or row[3] == 0x00:
            break
        pos += 4
    return b"".join(rows)


def find_builtin_se_tbl(exe: bytes) -> tuple[int, int]:
    """Locate se_tbl[128] in the PS-EXE by signature: entry 0 is
    {pri=1, tracks=1, kind=1, char=0} with three identical pointers to
    se_dummy = {00 00 FE FF}. Returns (file_offset, ram_base)."""
    if exe[:8] != b"PS-X EXE":
        raise SystemExit("not a PS-X EXE (expected SLUS executable)")
    t_addr = struct.unpack_from("<I", exe, 0x18)[0]

    def ram2file(addr: int) -> int:
        return addr - t_addr + 0x800

    sig = bytes([0x01, 0x01, 0x01, 0x00])
    pos = exe.find(sig)
    while pos != -1:
        if pos % 4 == 0 and pos + 16 <= len(exe):
            p0, p1, p2 = struct.unpack_from("<III", exe, pos + 4)
            if p0 == p1 == p2 and 0x80000000 <= p0 < 0x80800000:
                fo = ram2file(p0)
                if 0 <= fo + 4 <= len(exe) and exe[fo : fo + 4] == b"\x00\x00\xfe\xff":
                    return pos, t_addr
        pos = exe.find(sig, pos + 1)
    raise SystemExit("built-in se_tbl not found in executable")


def load_builtin_se(exe: bytes) -> list[SeEntry | None]:
    tbl_off, t_addr = find_builtin_se_tbl(exe)

    def ram2file(addr: int) -> int:
        return addr - t_addr + 0x800

    entries: list[SeEntry | None] = []
    for i in range(128):
        off = tbl_off + i * 16
        pri, tracks, kind, character = exe[off : off + 4]
        ptrs = struct.unpack_from("<III", exe, off + 4)
        seqs = []
        for idx in range(tracks):
            fo = ram2file(ptrs[idx])
            if not (0 <= fo < len(exe)):
                seqs = []
                break
            seqs.append(read_sequence(exe, fo))
        if not seqs or all(s == b"\x00\x00\xfe\xff" for s in seqs):
            entries.append(None)
            continue
        entries.append(SeEntry(pri, tracks, kind, character, seqs,
                               BUILTIN_SE_NAMES.get(i, f"se_{i:03d}")))
    return entries


def load_efx_bank(payload: bytes) -> list[SeEntry | None]:
    """Stage .se/.efx: 0x800-byte header of 128 16-byte SETBL entries with
    offsets (relative to the data area at +0x800); 0xFFFFFFFF = unused."""
    entries: list[SeEntry | None] = []
    data_base = 0x800
    for i in range(128):
        off = i * 16
        if off + 16 > len(payload) or off + 16 > data_base:
            entries.append(None)
            continue
        pri, tracks, kind, character = payload[off : off + 4]
        ptrs = struct.unpack_from("<III", payload, off + 4)
        if tracks == 0 or tracks > 3:
            entries.append(None)
            continue
        seqs = []
        for idx in range(tracks):
            rel = ptrs[idx]
            if rel == 0xFFFFFFFF or data_base + rel >= len(payload):
                continue
            seqs.append(read_sequence(payload, data_base + rel))
        if not seqs:
            entries.append(None)
            continue
        entries.append(SeEntry(pri, tracks, kind, character, seqs, f"efx_{i:03d}"))
    return entries


# ---------------------------------------------------------------------------
# SPU reverb (psx-spx "Reverb Formula"), preset Studio Large = libspu
# SPU_REV_MODE_STUDIO_C, which sd_init() selects with depth 0x4000 on all
# music channels. Runs at 22050 Hz like the hardware.
# ---------------------------------------------------------------------------

_STUDIO_LARGE = [
    0x00E3, 0x00A9, 0x6F60, 0x4FA8, 0xBCE0, 0x4510, 0xBEF0, 0xA680,
    0x5680, 0x52C0, 0x0DFB, 0x0B58, 0x0D09, 0x0A3C, 0x0BD9, 0x0973,
    0x0B59, 0x08DA, 0x08D9, 0x05E9, 0x07EC, 0x04B0, 0x06EF, 0x03D2,
    0x05EA, 0x031D, 0x031C, 0x0238, 0x0154, 0x00AA, 0x8000, 0x8000,
]


def _s16(v: int) -> int:
    return v - 0x10000 if v >= 0x8000 else v


class SpuReverb:
    def __init__(self, depth: int = 0x4000) -> None:
        r = _STUDIO_LARGE
        (dAPF1, dAPF2, vIIR, vC1, vC2, vC3, vC4, vWALL, vAPF1, vAPF2,
         mLSAME, mRSAME, mLC1, mRC1, mLC2, mRC2, dLSAME, dRSAME,
         mLDIFF, mRDIFF, mLC3, mRC3, mLC4, mRC4, dLDIFF, dRDIFF,
         mLAPF1, mRAPF1, mLAPF2, mRAPF2, vLIN, vRIN) = r
        # volumes signed; addresses/offsets are in 8-byte units -> halfwords*4
        self.vIIR, self.vWALL = _s16(vIIR), _s16(vWALL)
        self.vC = [_s16(vC1), _s16(vC2), _s16(vC3), _s16(vC4)]
        self.vAPF1, self.vAPF2 = _s16(vAPF1), _s16(vAPF2)
        self.vLIN, self.vRIN = _s16(vLIN), _s16(vRIN)
        self.vOUT = depth
        h = lambda x: x * 4
        self.dAPF1, self.dAPF2 = h(dAPF1), h(dAPF2)
        self.mLSAME, self.mRSAME = h(mLSAME), h(mRSAME)
        self.dLSAME, self.dRSAME = h(dLSAME), h(dRSAME)
        self.mLDIFF, self.mRDIFF = h(mLDIFF), h(mRDIFF)
        self.dLDIFF, self.dRDIFF = h(dLDIFF), h(dRDIFF)
        self.mLC = [h(mLC1), h(mLC2), h(mLC3), h(mLC4)]
        self.mRC = [h(mRC1), h(mRC2), h(mRC3), h(mRC4)]
        self.mLAPF1, self.mRAPF1 = h(mLAPF1), h(mRAPF1)
        self.mLAPF2, self.mRAPF2 = h(mLAPF2), h(mRAPF2)
        self.size = max(self.mLSAME, self.mRSAME, *self.mLC, *self.mRC,
                        self.mLDIFF, self.mRDIFF, self.mLAPF1, self.mRAPF1,
                        self.mLAPF2, self.mRAPF2) + 4
        self.buf = [0] * self.size
        self.pos = 0

    def step(self, lin: float, rin: float) -> tuple[float, float]:
        """One 22050 Hz reverb cycle. Inputs/outputs are sample-scaled floats."""
        buf, size, pos = self.buf, self.size, self.pos
        rd = lambda o: buf[(pos + o) % size]
        def wr(o: int, v: int) -> None:
            buf[(pos + o) % size] = max(-32768, min(32767, v))
        mul = lambda a, v: (int(a) * v) >> 15

        Lin = mul(lin, self.vLIN)
        Rin = mul(rin, self.vRIN)
        wr(self.mLSAME, mul(Lin + mul(rd(self.dLSAME), self.vWALL) - rd(self.mLSAME - 1), self.vIIR) + rd(self.mLSAME - 1))
        wr(self.mRSAME, mul(Rin + mul(rd(self.dRSAME), self.vWALL) - rd(self.mRSAME - 1), self.vIIR) + rd(self.mRSAME - 1))
        wr(self.mLDIFF, mul(Lin + mul(rd(self.dRDIFF), self.vWALL) - rd(self.mLDIFF - 1), self.vIIR) + rd(self.mLDIFF - 1))
        wr(self.mRDIFF, mul(Rin + mul(rd(self.dLDIFF), self.vWALL) - rd(self.mRDIFF - 1), self.vIIR) + rd(self.mRDIFF - 1))
        Lout = sum(mul(rd(m), v) for m, v in zip(self.mLC, self.vC))
        Rout = sum(mul(rd(m), v) for m, v in zip(self.mRC, self.vC))
        Lout -= mul(rd(self.mLAPF1 - self.dAPF1), self.vAPF1)
        wr(self.mLAPF1, Lout)
        Lout = mul(Lout, self.vAPF1) + rd(self.mLAPF1 - self.dAPF1)
        Rout -= mul(rd(self.mRAPF1 - self.dAPF1), self.vAPF1)
        wr(self.mRAPF1, Rout)
        Rout = mul(Rout, self.vAPF1) + rd(self.mRAPF1 - self.dAPF1)
        Lout -= mul(rd(self.mLAPF2 - self.dAPF2), self.vAPF2)
        wr(self.mLAPF2, Lout)
        Lout = mul(Lout, self.vAPF2) + rd(self.mLAPF2 - self.dAPF2)
        Rout -= mul(rd(self.mRAPF2 - self.dAPF2), self.vAPF2)
        wr(self.mRAPF2, Rout)
        Rout = mul(Rout, self.vAPF2) + rd(self.mRAPF2 - self.dAPF2)
        self.pos = (pos + 1) % size
        return mul(Lout, self.vOUT), mul(Rout, self.vOUT)


# ---------------------------------------------------------------------------
# SPU voice simulation (psx-spx semantics)
# ---------------------------------------------------------------------------

class SpuVoice:
    def __init__(self, ram: bytearray) -> None:
        self.ram = ram
        self.vol_l = 0
        self.vol_r = 0
        self.pitch = 0
        self.start = 0
        self.adsr1 = 0
        self.adsr2 = 0
        self.on = False
        self.env_level = 0
        self.env_phase = "off"  # attack/decay/sustain/release/off
        self.env_counter = 0
        self.reverb = False
        # decoder state
        self.pos = 0
        self.repeat = 0
        self.hist = (0, 0)
        self.block: list[int] = []
        self.block_idx = 0
        self.frac = 0
        self.h = [0, 0, 0, 0]  # 4-point interpolation window
        self.ended = True

    def key_on(self) -> None:
        self.on = True
        self.ended = False
        self.env_level = 0
        self.env_phase = "attack"
        self.env_counter = 0
        self.pos = self.start
        self.repeat = self.start
        self.hist = (0, 0)
        self.block = []
        self.block_idx = 0
        self.frac = 0
        self.h = [0, 0, 0, 0]

    def key_off(self) -> None:
        if self.env_phase not in ("off",):
            self.env_phase = "release"
            self.env_counter = 0

    def _decode_next_block(self) -> bool:
        if self.pos + 16 > len(self.ram):
            return False
        blk = self.ram[self.pos : self.pos + 16]
        flags = blk[1]
        if flags & 0x04:  # loop start
            self.repeat = self.pos
        shift = blk[0] & 0x0F
        filt = min((blk[0] >> 4) & 0x0F, 4)
        f0, f1 = ADPCM_FILTERS[filt]
        s1, s2 = self.hist
        out = []
        for b in blk[2:16]:
            for nib in (b & 0x0F, b >> 4):
                v = (nib - 16 if nib >= 8 else nib) << 12
                if shift <= 12:
                    v >>= shift
                else:
                    v = 0
                v += ((s1 * f0) + (s2 * f1) + 32) >> 6
                v = clamp16(v)
                out.append(v)
                s2, s1 = s1, v
        self.hist = (s1, s2)
        self.block = out
        self.block_idx = 0
        if flags & 0x01:  # end block
            if flags & 0x02:
                self.pos = self.repeat
            else:
                self.ended = True  # mute after this block (no repeat)
                self.pos = self.repeat
        else:
            self.pos += 16
        return True

    def _advance_sample(self) -> None:
        h = self.h
        h[0], h[1], h[2] = h[1], h[2], h[3]
        if self.block_idx >= len(self.block):
            if self.ended or not self._decode_next_block():
                h[3] = 0
                return
        h[3] = self.block[self.block_idx]
        self.block_idx += 1
        if self.block_idx >= len(self.block) and self.ended:
            self.block = []

    def _env_step(self) -> None:
        # psx-spx ADSR: rate -> shift/step; counter waits cycles, adds step.
        phase = self.env_phase
        if phase == "off":
            return
        if phase == "attack":
            rate = (self.adsr1 >> 8) & 0x7F
            exp = bool(self.adsr1 & 0x8000)
            decrease = False
        elif phase == "decay":
            rate = ((self.adsr1 >> 4) & 0x0F) << 2
            exp = True
            decrease = True
        elif phase == "sustain":
            rate = (self.adsr2 >> 6) & 0x7F
            exp = bool(self.adsr2 & 0x8000)
            decrease = bool(self.adsr2 & 0x4000)
        else:  # release
            rate = (self.adsr2 & 0x1F) << 2
            exp = bool(self.adsr2 & 0x0020)
            decrease = True

        shift = rate >> 2
        step = (-8 + (rate & 3)) if decrease else (7 - (rate & 3))
        cycles = 1 << max(0, shift - 11)
        step <<= max(0, 11 - shift)
        if exp and not decrease and self.env_level > 0x6000:
            cycles *= 4
        if exp and decrease:
            step = (step * self.env_level) >> 15

        self.env_counter += 1
        if self.env_counter < cycles:
            return
        self.env_counter = 0
        self.env_level = max(0, min(0x7FFF, self.env_level + step))

        if phase == "attack" and self.env_level >= 0x7FFF:
            self.env_phase = "decay"
        elif phase == "decay":
            sustain_level = ((self.adsr1 & 0x0F) + 1) * 0x800
            if self.env_level <= sustain_level:
                self.env_phase = "sustain"
        elif phase == "release" and self.env_level <= 0:
            self.env_phase = "off"
            self.on = False

    def render(self, n: int, out_l: list[float], out_r: list[float], base: int,
               wet_l: list[float] | None = None, wet_r: list[float] | None = None) -> None:
        if self.env_phase == "off":
            return
        pitch = min(self.pitch, 0x4000)
        send = self.reverb and wet_l is not None
        for i in range(n):
            self._env_step()
            if self.env_phase == "off":
                break
            self.frac += pitch
            while self.frac >= 0x1000:
                self.frac -= 0x1000
                self._advance_sample()
            t = self.frac / 0x1000
            p0, p1, p2, p3 = self.h
            # Catmull-Rom between p1..p2 (the SPU uses a gaussian 4-tap)
            s = p1 + 0.5 * t * (p2 - p0 + t * (2 * p0 - 5 * p1 + 4 * p2 - p3 + t * (3 * (p1 - p2) + p3 - p0)))
            s = s * self.env_level / 0x8000
            sl = s * self.vol_l / 0x8000
            sr = s * self.vol_r / 0x8000
            out_l[base + i] += sl
            out_r[base + i] += sr
            if send:
                wet_l[base + i] += sl
                wet_r[base + i] += sr

    def is_audible(self) -> bool:
        if self.env_phase == "off":
            return False
        if self.env_phase == "release" and self.env_level <= SILENCE_FLOOR:
            return False
        return True


# ---------------------------------------------------------------------------
# Sequencer (port of sd_sub1/sd_sub2/sd_ioset track logic)
# ---------------------------------------------------------------------------

class Track:
    """One SE track: SOUND_W + SPU_TRACK_REG pending-register state."""

    def __init__(self, sound: SoundRam, voice: SpuVoice, se_kind: int) -> None:
        self.sound = sound
        self.voice = voice
        self.se_kind = se_kind
        self.data = b""
        self.mptr = 0
        self.done = True
        self.key_on_req = False
        self.key_off_req = False
        # SOUND_W fields (sng_track_init defaults)
        self.ngc = 1
        self.ngo = 0
        self.ngs = 0
        self.ngg = 0
        self.vol = 127
        self.pvod = 127
        self.pvoc = 0
        self.pvoad = 0
        self.pvom = 0
        self.pand = 2560
        self.panf = 10
        self.panc = 0
        self.panad = 0
        self.panm = 0
        self.panoff = 0
        self.panmod = 0
        self.lp1_cnt = 0
        self.lp2_cnt = 0
        self.lp1_vol = 0
        self.lp2_vol = 0
        self.lp1_freq = 0
        self.lp2_freq = 0
        self.lp1_addr = 0
        self.lp2_addr = 0
        self.lp3_addr = 0
        self.kakfg = 0
        self.kak1ptr = 0
        self.kak2ptr = 0
        self.swpc = 0
        self.swphc = 0
        self.swpd = 0
        self.swpad = 0
        self.swpm = 0
        self.swsc = 0
        self.swshc = 0
        self.swsk = 0
        self.swss = 0
        self.vibhc = 0
        self.vib_tmp_cnt = 0
        self.vib_tbl_cnt = 0
        self.vib_tc_ofst = 0
        self.vibcc = 0
        self.vibd = 0
        self.vibdm = 0
        self.vibhs = 0
        self.vibcs = 0
        self.vibcad = 0
        self.vibad = 0
        self.rdmc = 0
        self.rdmo = 0
        self.rdms = 0
        self.rdmd = 0
        self.trec = 0
        self.trehc = 0
        self.tred = 0
        self.trecad = 0
        self.trehs = 0
        self.ptps = 0
        self.dec_vol = 0
        self.tund = 0
        self.tmpd = 1
        self.tmp = 255
        self.tmpad = 0
        self.tmpc = 0
        self.tmpw = 0
        self.tmpm = 0
        self.rest_fg = 0
        self.macro = 0
        self.micro = 0
        self.rrd = 0
        self.wavs = 0x4F
        # pending command bytes
        self.m1 = self.m2 = self.m3 = self.m4 = 0
        # pending SPU register writes (spu_tr_wk)
        self.p_vol: tuple[int, int] | None = None
        self.p_pitch: int | None = None
        self.p_addr: int | None = None
        self.p_a_mode = 1
        self.p_ar = 0
        self.p_dr = 0
        self.p_s_mode = 1
        self.p_sr = 0
        self.p_sl = 0
        self.p_r_mode = 3
        self.p_rr = 0
        self.p_env1 = False
        self.p_env2 = False
        self.p_env3 = False

    def start(self, seq: bytes, offset: int = 0) -> None:
        self.data = seq
        self.mptr = offset
        self.done = False
        self.l3_jumps = 0

    # --- register staging (spuwr equivalent) ---

    def flush_registers(self) -> None:
        v = self.voice
        if self.key_off_req:
            v.key_off()
            self.key_off_req = False
        if self.p_vol is not None:
            v.vol_l, v.vol_r = self.p_vol
            self.p_vol = None
        if self.p_pitch is not None:
            v.pitch = self.p_pitch
            self.p_pitch = None
        if self.p_addr is not None:
            v.start = self.p_addr
            self.p_addr = None
        if self.p_env1:
            exp_a = 1 if self.p_a_mode == 5 else 0
            v.adsr1 = (exp_a << 15) | ((self.p_ar & 0x7F) << 8) | ((self.p_dr & 0xF) << 4) | (v.adsr1 & 0xF)
            v.adsr1 = (v.adsr1 & ~0xF) | (self.p_sl & 0xF)
            self.p_env1 = False
        if self.p_env2:
            # s_mode: 1=LinInc, 3=LinDec, 5=ExpInc, 7=ExpDec
            exp_s = 1 if self.p_s_mode in (5, 7) else 0
            dec_s = 1 if self.p_s_mode in (3, 7) else 0
            v.adsr2 = (v.adsr2 & 0x003F) | (exp_s << 15) | (dec_s << 14) | ((self.p_sr & 0x7F) << 6)
            self.p_env2 = False
        if self.p_env3:
            exp_r = 1 if self.p_r_mode == 7 else 0
            v.adsr2 = (v.adsr2 & ~0x3F) | (exp_r << 5) | (self.p_rr & 0x1F)
            self.p_env3 = False
        if self.key_on_req:
            v.key_on()
            self.key_on_req = False

    # --- tone / freq / vol (sd_ioset port) ---

    def tone_set(self, n: int) -> None:
        e = self.sound.voice_tbl[n % 256]
        self.p_addr = e.addr
        self.macro = e.sample_note
        self.micro = e.sample_tune
        self.p_a_mode = 5 if e.a_mode else 1
        self.p_ar = ~e.ar & 0x7F
        self.p_dr = ~e.dr & 0xF
        self.p_env1 = True
        self.p_s_mode = {0: 3, 1: 7, 2: 1}.get(e.s_mode, 5)
        self.p_sr = ~e.sr & 0x7F
        self.p_sl = e.sl & 0xF
        self.p_env2 = True
        self.p_r_mode = 3 if not e.r_mode else 7
        self.p_rr = self.rrd = ~e.rr & 0x1F
        self.p_env3 = True
        if not self.panmod:
            self.pan_set2(e.pan)
        self.dec_vol = e.decl_vol

    def pan_set2(self, x: int) -> None:
        if not self.panoff:
            self.panf = 2 * x
            self.pand = x << 9

    def freq_set(self, note_tune: int) -> None:
        note_tune += self.micro
        temp4 = note_tune & 0xFF
        temp3 = (((note_tune >> 8) & 0xFF) + self.macro) & 0x7F
        # the real driver indexes past freq_tbl for out-of-range notes; clamp
        temp3 = min(temp3, len(FREQ_TBL) - 2)
        nxt = FREQ_TBL[temp3 + 1]
        freq = (nxt - FREQ_TBL[temp3]) & 0xFFFF
        if freq & 0x8000:
            freq = 0xC9
        lo, hi = freq & 0xFF, (freq >> 8) & 0xFF
        freq = ((lo * temp4) >> 8) + (hi * temp4) + FREQ_TBL[temp3]
        self.p_pitch = freq & 0xFFFF

    def vol_set(self, vol_data: int, se_vol: int, se_pan: int) -> None:
        if vol_data >= self.dec_vol:
            vol_data -= self.dec_vol
        else:
            vol_data = 0
        if self.se_kind == 0:
            pan = min(self.pand >> 8, 40)
            self.p_vol = (vol_data * PANT[40 - pan], vol_data * PANT[pan])
        else:
            vol_data = (vol_data * se_vol) >> 16
            self.p_vol = (vol_data * SE_PANT[64 - se_pan], vol_data * SE_PANT[se_pan])

    def volxset(self, depth: int, se_vol: int, se_pan: int) -> None:
        vol_data = self.vol - depth + self.lp1_vol + self.lp2_vol
        vol_data = max(0, min(127, vol_data))
        pvod_w = (self.pvod >> 8) & 0xFF
        self.vol_set(((pvod_w * vol_data) >> 8) & 0xFF, se_vol, se_pan)


class Sequencer:
    """Runs up to 3 tracks of one SE through the driver tick loop."""

    def __init__(self, sound: SoundRam, entry: SeEntry, se_code_vol: int = 0x3F,
                 se_pan: int = 32, music: bool = False) -> None:
        self.sound = sound
        self.entry = entry
        self.se_vol = 2 * ((se_code_vol & 0x3F) << 8)
        self.se_pan = se_pan & 0x3F
        self.tracks: list[Track] = []
        for seq in entry.seqs:
            t = Track(sound, SpuVoice(sound.spu_ram), entry.kind)
            t.voice.reverb = music  # sd_init enables reverb on channels 0-12
            if isinstance(seq, tuple):
                t.start(seq[0], seq[1])
            else:
                t.start(seq)
            self.tracks.append(t)

    # --- command dispatch (cntl_tbl port); returns "note"/"stop"/"end"/None ---

    def control(self, t: Track) -> str | None:
        op = t.m1 - 0x80
        if op == 0x50:  # tempo_set
            t.tmp = t.m2
        elif op == 0x51:  # tempo_move
            t.tmpc = t.m2
            t.tmpm = t.m3
            t.tmpw = t.tmp << 8
            d = (t.tmpm & 0xFF) - (t.tmp & 0xFF)
            d = max(-127, min(127, d))
            if t.tmpc:
                t.tmpad = (d << 8) // t.tmpc if d >= 0 else -((-d << 8) // t.tmpc)
        elif op in (0x52, 0x53, 0x54):  # sno_set/svl_set/svp_set
            t.key_off_req = True
            t.tone_set(t.m2)
        elif op == 0x55:  # vol_chg
            t.pvod = t.m2 << 8
            t.pvoc = 0
        elif op == 0x56:  # vol_move
            t.pvoc = t.m2
            t.pvom = t.m3
            d = (t.m3 << 8) - t.pvod
            if t.pvoc:
                t.pvoad = max(-2032, min(0x7F0, d // t.pvoc if d >= 0 else -((-d) // t.pvoc)))
        elif op == 0x57:  # ads_set
            t.p_a_mode = 1
            t.p_ar = ~t.m2 & 0x7F
            t.p_dr = ~t.m3 & 0xF
            t.p_sl = t.m4 & 0xF
            t.p_env1 = True
        elif op == 0x58:  # srs_set
            t.p_s_mode = 3
            t.p_sr = ~t.m2 & 0x7F
            t.p_env2 = True
        elif op == 0x59:  # rrs_set
            t.p_r_mode = 3
            t.p_rr = t.rrd = ~t.m2 & 0x1F
            t.p_env3 = True
        elif op == 0x5D:  # pan_set (panf is a char in the driver: wraps)
            t.panmod = t.m2
            t.panf = (t.m3 + 20) & 0xFF
            t.pand = t.panf << 8
            t.panc = 0
        elif op == 0x5E:  # pan_move
            t.panc = t.m2
            target = (t.m3 + 0x14) & 0xFF
            t.panm = target << 8
            d = target - t.panf
            if t.panc:
                t.panad = max(-2032, min(2032, (d << 8) // t.panc if d >= 0 else -(((-d) << 8) // t.panc)))
        elif op == 0x5F:  # trans_set
            t.ptps = s8(t.m2)
        elif op == 0x60:  # detune_set
            t.tund = s8(t.m2) << 2
        elif op == 0x61:  # vib_set
            t.vibhs = t.m2
            cad = t.m3
            if cad < 32:
                t.vib_tc_ofst, cad = 1, cad << 3
            elif cad < 64:
                t.vib_tc_ofst, cad = 2, cad << 2
            elif cad < 128:
                t.vib_tc_ofst, cad = 4, cad << 1
            elif cad != 255:
                t.vib_tc_ofst = 8
            else:
                t.vib_tc_ofst = 16
            t.vibcad = cad
            t.vibd = t.m4 << 8
            t.vibdm = t.m4 << 8
        elif op == 0x62:  # vib_change
            t.vibcs = t.m2
            if t.m2:
                t.vibad = t.vibdm // t.m2
        elif op == 0x63:  # rdm_set
            t.rdms = t.m2
            t.rdmd = (t.m3 << 8) + t.m4
            t.rdmc = 0
            t.rdmo = 0
        elif op == 0x65:  # sws_set
            t.swsk = 0
            t.swshc = t.m2
            t.swsc = t.m3
            t.swss = t.m4 << 8
        elif op == 0x66:  # por_set
            t.swshc = 0
            t.swsc = t.m2
            t.swsk = 1 if t.m2 else 0
        elif op == 0x67:  # lp1_start
            t.lp1_addr = t.mptr
            t.lp1_cnt = 0
            t.lp1_freq = 0
            t.lp1_vol = 0
        elif op == 0x68:  # lp1_end
            t.lp1_cnt = (t.lp1_cnt + 1) & 0xFF
            if t.lp1_cnt != t.m2 or t.lp1_cnt == 0:
                t.lp1_vol += s8(t.m3)
                t.lp1_freq += s8(t.m4) * 8
                t.mptr = t.lp1_addr
            else:
                t.lp1_vol = 0
                t.lp1_freq = 0
        elif op == 0x69:  # lp2_start
            t.lp2_addr = t.mptr
            t.lp2_cnt = 0
            t.lp2_freq = 0
            t.lp2_vol = 0
        elif op == 0x6A:  # lp2_end
            t.lp2_cnt = (t.lp2_cnt + 1) & 0xFF
            if t.lp2_cnt != t.m2 or t.lp2_cnt == 0:
                t.lp2_vol += s8(t.m3)
                t.lp2_freq += s8(t.m4) * 8
                t.mptr = t.lp2_addr
        elif op == 0x6B:  # l3s_set
            t.lp3_addr = t.mptr
        elif op == 0x6C:  # l3e_set (infinite loop; counted for music rendering)
            if t.lp3_addr:
                t.mptr = t.lp3_addr
                t.l3_jumps += 1
            else:
                t.key_off_req = True
        elif op == 0x6D:  # kakko_start
            t.kak1ptr = t.mptr
            t.kakfg = 0
        elif op == 0x6E:  # kakko_end
            if t.kakfg == 0:
                t.kakfg = 1
            elif t.kakfg == 1:
                t.kakfg = 2
                t.kak2ptr = t.mptr
                t.mptr = t.kak1ptr
            else:
                t.kakfg = 1
                t.mptr = t.kak2ptr
        elif op == 0x72:  # rest_set
            t.rest_fg = 1
            t.key_off_req = True
            t.ngs = t.m2
            t.ngg = 0
            t.vol = 0
            t.ngc = t.ngs
            t.ngo = 0
            return "stop"
        elif op == 0x73:  # tie_set
            t.rest_fg = 1
            t.ngs = t.m2
            t.ngg = t.m3
            t.ngc = t.ngs
            t.ngo = max(1, (t.ngg * t.ngc) // 100)
            return "stop"
        elif op == 0x7F:  # block_end
            t.key_off_req = True
            return "end"
        elif op == 0x76:  # eon_set (driver applies it only for kind==0 SEs;
            if t.se_kind == 0:  # music channels have reverb on from boot)
                t.voice.reverb = True
        elif op == 0x77:  # eof_set
            if t.se_kind == 0:
                t.voice.reverb = False
        # 0x71 use_set, 0x74/0x75 echo, others: no-op
        return None

    # --- sd_sub1 port ---

    def note_set(self, t: Track) -> None:
        t.ngs = t.m2
        t.ngg = t.m3
        t.vol = t.m4 & 0x7F
        self.note_compute(t)
        t.ngc = t.ngs
        t.ngo = max(1, (t.ngg * t.ngc) // 100)

    def note_compute(self, t: Track) -> None:
        if t.m1 >= 0x48:
            t.key_off_req = True
            t.tone_set((t.m1 + ((t.wavs + 0xB8) & 0xFF)) & 0xFF)
            x = 0x24
        else:
            x = t.m1
        x += t.ptps
        x = (x << 8) + t.tund + t.lp1_freq + t.lp2_freq
        while x >= 0x6000:
            x -= 0x6000
        swp_ex = t.swpd
        t.vibcc = 0
        t.vibhc = 0
        t.swpd = x
        t.vib_tmp_cnt = 0
        t.vib_tbl_cnt = 0
        t.trehc = 0
        t.trec = 0
        t.vibd = 0
        t.p_rr = t.rrd
        t.p_env3 = True
        t.swpc = t.swsc
        if t.swpc:
            t.swphc = t.swshc
            if not t.swsk:
                x0 = t.swpd
                if t.swss >= 0x7F01:
                    t.swpd += 0x10000 - (t.swss & 0xFFFF)
                else:
                    t.swpd -= t.swss
                self.swpadset(t, x0)
            else:
                t.swpm = t.swpd
                t.swpd = swp_ex
        self.entry_freq_set(t, t.swpd)

    def entry_freq_set(self, t: Track, val: int) -> None:
        t.freq_set(val)

    def swpadset(self, t: Track, xfreq: int) -> None:
        if t.swpc:
            flame = (t.swpc << 8) // max(1, t.tmp)
            xfreq = max(0, min(0x5FFF, xfreq))
            t.swpm = xfreq
            xfreq -= t.swpd
            if flame:
                t.swpad = xfreq // flame if xfreq >= 0 else -((-xfreq) // flame)

    def vib_compute(self, t: Track) -> int:
        t.vib_tbl_cnt = (t.vib_tbl_cnt + t.vib_tc_ofst) & 0x3F
        tbl = VIBX_TBL[t.vib_tbl_cnt & 0x1F]
        tmp = t.vibd
        if tmp <= 0x7FFF:
            vd = ((tmp >> 7) & 0xFE)
            vd = (vd * tbl) >> 8
        else:
            vd = ((tmp >> 8) & 0x7F) + 2
            vd = (vd * tbl) >> 1
        if (t.vib_tbl_cnt & 0xFF) >= 32:
            vd = -vd
        return vd

    def por_compute(self, t: Track) -> None:
        d = t.swpm - t.swpd
        if d == 0:
            t.swpc = 0
            return
        mag = abs(d)
        lo = ((mag & 0xFF) * t.swsc) >> 8
        hi = (mag >> 8) * t.swsc
        step = max(1, hi + lo)
        t.swpd += step if d > 0 else -step

    def random(self, t: Track) -> int:
        if not t.rdms:
            return 0
        t.rdmc += t.rdms
        if t.rdmc > 256:
            t.rdmc &= 255
            t.rdmo = (t.rdmo + 1) & 0x7F
            v = RDM_TBL[t.rdmo] + (RDM_TBL[t.rdmo + 1] << 8)
            return v & t.rdmd
        return 0

    def keych(self, t: Track) -> None:
        if t.ngg < 0x64 and t.ngc == 1 and t.rrd >= 8:
            t.p_rr = 7
            t.p_env3 = True
        if t.ngo:
            t.ngo -= 1
            if not t.ngo:
                t.key_off_req = True
        set_fg = False
        if t.swpc:
            if t.swphc:
                t.swphc -= 1
            else:
                if not t.swsk:
                    t.swpc -= 1
                    if not (t.swpc & 0xFF):
                        t.swpd = t.swpm
                    else:
                        t.swpd += t.swpad
                else:
                    self.por_compute(t)
                set_fg = True
        vib_data = 0
        if t.vibdm:
            if (t.vibhc & 0xFF) != (t.vibhs & 0xFF):
                t.vibhc += 1
            else:
                if t.vibcc == t.vibcs:
                    t.vibd = t.vibdm
                else:
                    t.vibd = t.vibad if not t.vibcc else t.vibd + t.vibad
                    t.vibcc += 1
                t.vib_tmp_cnt += t.vibcad
                if t.vib_tmp_cnt >= 256:
                    t.vib_tmp_cnt &= 0xFF
                    vib_data = self.vib_compute(t)
                    set_fg = True
        rdm = self.random(t)
        if rdm:
            vib_data += rdm
            set_fg = True
        if set_fg:
            t.freq_set(t.swpd + vib_data)

    def note_cntl(self, t: Track) -> None:
        if t.vol and t.tred and t.trehs == t.trehc:
            t.trec += (t.trecad * s8(t.tmpd & 0xFF)) >> 8
            if t.trec < 0:
                depth = t.tred * -t.trec
            elif t.trec == 0:
                depth = 1
            else:
                depth = t.tred * t.trec
            t.volxset(depth >> 8, self.se_vol, self.se_pan)
        fset = False
        frq = t.swpd
        if t.swpc and not t.swphc:
            fset = True
            if not t.swsk:
                t.swpd += t.swpad
            else:
                self.por_compute(t)
            frq = t.swpd
        if t.vibd and t.vibhs == t.vibhc:
            t.vib_tmp_cnt += t.vibcad
            if t.vib_tmp_cnt >= 256:
                t.vib_tmp_cnt &= 0xFF
                frq += self.vib_compute(t)
                fset = True
        rdm = self.random(t)
        if rdm:
            fset = True
            frq += rdm
        if fset:
            t.freq_set(frq)

    def vol_compute(self, t: Track) -> None:
        if t.pvoc:
            t.pvoc -= 1
            if not t.pvoc:
                t.pvod = t.pvom << 8
            else:
                t.pvod += t.pvoad
        if t.vol:
            if not t.tred:
                depth = 0
            else:
                if t.trehs == t.trehc:
                    t.trec += t.trecad
                    if t.trec < 0:
                        depth = t.tred * -t.trec
                    elif t.trec == 0:
                        depth = 1
                    else:
                        depth = t.tred * t.trec
                else:
                    t.trehc += 1
                    depth = 0
            t.volxset(depth >> 8, self.se_vol, self.se_pan)
        if t.panc:
            t.panc -= 1
            if not t.panc:
                t.pand = t.panm
            else:
                t.pand += t.panad
            t.panf = t.pand >> 8

    def bendch(self, t: Track) -> None:
        if not t.swpc and t.mptr + 4 <= len(t.data) and t.data[t.mptr + 3] == 0xE4:
            t.swphc = t.data[t.mptr + 2]
            t.swpc = t.data[t.mptr + 1]
            bend = t.data[t.mptr]
            t.mptr += 4
            bend = ((bend + t.ptps) << 8) + t.tund
            self.swpadset(t, bend)

    def tx_read(self, t: Track) -> tuple[bool, bool]:
        """Returns (track_finished, key_on)."""
        key_fg = False
        for _ in range(256):
            if t.mptr + 4 > len(t.data):
                return True, key_fg
            m4, m3, m2, m1 = t.data[t.mptr : t.mptr + 4]
            if not m1:
                return True, key_fg
            t.m1, t.m2, t.m3, t.m4 = m1, m2, m3, m4
            t.mptr += 4
            if m1 >= 0x80:
                res = self.control(t)
                if res == "end":
                    return True, key_fg
                if res == "stop":
                    return False, key_fg
            else:
                if t.ngg < 0x64 and t.m4:
                    key_fg = True
                t.rest_fg = 0
                self.note_set(t)
                return False, key_fg
        return True, key_fg

    def tick_track(self, t: Track) -> None:
        if t.done:
            return
        t.tmpd += t.tmp
        if t.tmpd >= 256:
            t.tmpd &= 0xFF
            t.ngc = (t.ngc - 1) & 0xFF
            key_fg = False
            if t.ngc:
                self.keych(t)
            else:
                finished, key_fg = self.tx_read(t)
                if finished:
                    t.key_off_req = True
                    t.done = True
                    t.flush_registers()
                    return
            if t.tmpc:
                t.tmpc -= 1
                if not t.tmpc:
                    t.tmpw = (t.tmpm & 0xFF) << 8
                else:
                    t.tmpw += t.tmpad
                t.tmp = (t.tmpw >> 8) & 0xFF
            self.bendch(t)
            self.vol_compute(t)
            if key_fg:
                t.key_on_req = True
        else:
            self.note_cntl(t)
        t.flush_registers()

    def render(self, tick_hz: float = TICK_HZ, max_seconds: float = MAX_SECONDS,
               loop_stop: int = 0) -> tuple[list[float], list[float]]:
        """loop_stop > 0: stop once every track has either finished or jumped
        through its infinite l3 loop that many times (music mode)."""
        out_l: list[float] = []
        out_r: list[float] = []
        wet_l: list[float] = []
        wet_r: list[float] = []
        sample_pos = 0.0
        base = 0
        wet_used = False
        n_ticks = int(max_seconds * tick_hz)
        samples_per_tick = OUT_RATE / tick_hz
        for tick in range(n_ticks):
            for t in self.tracks:
                self.tick_track(t)
            sample_pos += samples_per_tick
            n = int(sample_pos) - base
            out_l.extend([0.0] * n)
            out_r.extend([0.0] * n)
            wet_l.extend([0.0] * n)
            wet_r.extend([0.0] * n)
            for t in self.tracks:
                t.voice.render(n, out_l, out_r, base, wet_l, wet_r)
                wet_used = wet_used or t.voice.reverb
            base += n
            if all(t.done for t in self.tracks) and not any(
                t.voice.is_audible() for t in self.tracks
            ):
                break
            if loop_stop and all(
                t.done or t.l3_jumps >= loop_stop for t in self.tracks
            ):
                break
        if wet_used:
            # offline reverb pass (linear+causal, so identical to inline)
            rev = SpuReverb()
            tail = int(1.5 * OUT_RATE)
            out_l.extend([0.0] * tail)
            out_r.extend([0.0] * tail)
            wet_l.extend([0.0] * tail)
            wet_r.extend([0.0] * tail)
            for i in range(0, len(out_l) - 1, 2):
                l, r = rev.step((wet_l[i] + wet_l[i + 1]) * 0.5,
                                (wet_r[i] + wet_r[i + 1]) * 0.5)
                out_l[i] += l
                out_l[i + 1] += l
                out_r[i] += r
                out_r[i + 1] += r
        return out_l, out_r


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

def trim_silence(l: list[float], r: list[float]) -> tuple[list[float], list[float]]:
    last = 0
    for i in range(len(l) - 1, -1, -1):
        if abs(l[i]) > 1.0 or abs(r[i]) > 1.0:
            last = i + 1
            break
    pad = min(len(l), last + OUT_RATE // 20)
    return l[:pad], r[:pad]


def write_wav_stereo(path: Path, l: list[float], r: list[float], gain: float = 1.0) -> float:
    path.parent.mkdir(parents=True, exist_ok=True)
    frames = bytearray()
    # master volume 0x3FFF/0x8000 like the driver's SpuSetCommonAttr
    g = 0.5 * gain
    for a, b in zip(l, r):
        frames += struct.pack("<hh", clamp16(int(a * g)), clamp16(int(b * g)))
    with wave.open(str(path), "wb") as wf:
        wf.setnchannels(2)
        wf.setsampwidth(2)
        wf.setframerate(OUT_RATE)
        wf.writeframes(bytes(frames))
    return len(l) / OUT_RATE


def peak(l: list[float], r: list[float]) -> float:
    m = 0.0
    for v in l:
        m = max(m, abs(v))
    for v in r:
        m = max(m, abs(v))
    return m


# ---------------------------------------------------------------------------
# Main pipeline
# ---------------------------------------------------------------------------

def load_stage_waves(stage_data: bytes, folders: dict, names: list[str],
                     sound: SoundRam) -> list[dict]:
    loaded = []
    for fname in names:
        for entry in folders.get(fname, []):
            if entry["type"] != "w":
                continue
            payload = stage_data[entry["offset"] : entry["offset"] + entry["size"]]
            try:
                info = sound.load_sw(payload)
            except (struct.error, IndexError):
                continue
            loaded.append({"folder": fname, "id": f"{entry['id']:04x}", **info})
    return loaded


def render_entries(entries: list[SeEntry | None], sound: SoundRam, out_dir: Path,
                   prefix: str, tick_hz: float, only: set[str] | None,
                   manifest: list[dict]) -> int:
    count = 0
    for code, entry in enumerate(entries):
        if entry is None:
            continue
        if only and entry.name not in only and str(code) not in only:
            continue
        seq = Sequencer(sound, entry)
        l, r = seq.render(tick_hz=tick_hz)
        l, r = trim_silence(l, r)
        if not l or peak(l, r) < 8.0:
            continue
        name = f"{prefix}{code:03d}_{entry.name}.wav"
        dur = write_wav_stereo(out_dir / name, l, r)
        manifest.append({
            "code": code, "name": entry.name, "file": name,
            "duration": round(dur, 3), "tracks": len(entry.seqs),
            "kind": entry.kind, "pri": entry.pri,
            "looping": dur >= MAX_SECONDS - 1.0,
            "peak": round(peak(l, r) * 0.5 / 32768, 4),
        })
        count += 1
    return count


# ---------------------------------------------------------------------------
# Music (.mdx): same sequencer, 13 song tracks per song (sd_drv.c sng_adrs_set)
# ---------------------------------------------------------------------------

MUSIC_MAX_SECONDS = 240.0


def parse_mdx_songs(mdx: bytes) -> list[list[tuple[bytes, int]]]:
    """MDX layout: byte 0 = song count; song N's 16-bit offset at N*4 points
    to 13 track pointers (24-bit offsets into the file), 4 bytes apart."""
    if not mdx:
        return []
    count = mdx[0]
    songs = []
    for s in range(1, count + 1):
        if s * 4 + 2 > len(mdx):
            break
        off = mdx[s * 4] | (mdx[s * 4 + 1] << 8)
        if off == 0 or off + 13 * 4 > len(mdx):
            continue
        tracks: list[tuple[bytes, int]] = []
        for tno in range(13):
            p = off + tno * 4
            a = mdx[p] | (mdx[p + 1] << 8) | (mdx[p + 2] << 16)
            if a and a + 4 <= len(mdx):
                tracks.append((mdx, a))
        if tracks:
            songs.append(tracks)
    return songs


def _render_music_job(job: dict) -> dict | None:
    sound = SoundRam()
    for payload in job["sw_payloads"]:
        try:
            sound.load_sw(payload)
        except (struct.error, IndexError):
            continue
    songs = parse_mdx_songs(job["mdx"])
    idx = job["song_index"]
    if idx >= len(songs):
        return None
    entry = SeEntry(pri=0, tracks=13, kind=0, character=0,
                    seqs=songs[idx], name=job["name"])
    seq = Sequencer(sound, entry, music=True)
    l, r = seq.render(tick_hz=job["tick_hz"], max_seconds=job["max_seconds"],
                      loop_stop=2)
    l, r = trim_silence(l, r)
    if not l or peak(l, r) < 8.0:
        return None
    # makeup gain: normalize each song to -1 dBFS peak (mix is authentic but quiet)
    pk = peak(l, r) * 0.5 / 32768
    gain = min(8.0, 0.9 / pk) if pk > 0 else 1.0
    dur = write_wav_stereo(Path(job["out_path"]), l, r, gain=gain)
    looping = any(t.l3_jumps for t in seq.tracks)
    return {"name": job["name"], "stage": job["stage"], "mdxId": job["mdx_id"],
            "song": idx + 1, "file": Path(job["out_path"]).name,
            "duration": round(dur, 3), "looping": looping,
            "peak": round(pk, 4), "gain": round(gain, 2)}


def render_music(stage_data: bytes, folders: dict, resident_stage: str,
                 out_dir: Path, tick_hz: float, max_seconds: float,
                 stages: list[str] | None, jobs: int) -> list[dict]:
    def payloads(stage_name: str, typ: str) -> list[bytes]:
        return [stage_data[e["offset"] : e["offset"] + e["size"]]
                for e in folders.get(stage_name, []) if e["type"] == typ]

    resident_sw = payloads(resident_stage, "w")
    seen: set[tuple[int, int]] = set()
    job_list: list[dict] = []
    for stage_name, entries in folders.items():
        if stages and stage_name not in stages:
            continue
        for e in entries:
            if e["type"] != "m":
                continue
            mdx = stage_data[e["offset"] : e["offset"] + e["size"]]
            n_songs = len(parse_mdx_songs(mdx))
            for s in range(n_songs):
                key = (e["id"], s)
                if key in seen:
                    continue
                seen.add(key)
                name = f"{stage_name}_{e['id']:04x}_song{s + 1}"
                job_list.append({
                    "name": name, "stage": stage_name, "mdx_id": f"{e['id']:04x}",
                    "mdx": mdx, "song_index": s,
                    "sw_payloads": resident_sw + payloads(stage_name, "w"),
                    "out_path": str(out_dir / f"{name}.wav"),
                    "tick_hz": tick_hz, "max_seconds": max_seconds,
                })
    print(f"music: {len(job_list)} unique songs, {jobs} workers")
    results: list[dict] = []
    if jobs > 1:
        from concurrent.futures import ProcessPoolExecutor
        with ProcessPoolExecutor(max_workers=jobs) as pool:
            for res in pool.map(_render_music_job, job_list):
                if res:
                    results.append(res)
                    print(f"  {res['name']}: {res['duration']}s"
                          + (" (loop)" if res["looping"] else ""))
    else:
        for job in job_list:
            res = _render_music_job(job)
            if res:
                results.append(res)
                print(f"  {res['name']}: {res['duration']}s")
    return results


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--filesystem", type=Path, required=True,
                    help="Extracted disc filesystem root (contains MGS/STAGE.DIR and SLUS_*)")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--stage", default="init",
                    help="Stage folder whose wave banks are resident (default: init)")
    ap.add_argument("--efx-stages", nargs="*", default=[],
                    help="Additional stage folders whose .se effect banks to render")
    ap.add_argument("--only", nargs="*", default=None,
                    help="Render only these SE names or codes (e.g. codec_call 103)")
    ap.add_argument("--tick-hz", type=float, default=TICK_HZ)
    ap.add_argument("--list", action="store_true", help="List built-in SEs and exit")
    ap.add_argument("--music", action="store_true",
                    help="Also render all .mdx music (two loop passes per song)")
    ap.add_argument("--music-only", action="store_true")
    ap.add_argument("--music-stages", nargs="*", default=None,
                    help="Limit music to these stage folders")
    ap.add_argument("--music-max-seconds", type=float, default=MUSIC_MAX_SECONDS)
    ap.add_argument("--music-tick-hz", type=float, default=MUSIC_TICK_HZ)
    ap.add_argument("--jobs", type=int, default=max(1, (__import__("os").cpu_count() or 2) - 1))
    args = ap.parse_args()

    fs = args.filesystem
    exe_path = next((p for p in [fs / "SLUS_005.94", fs / "MGS" / "SLUS_005.94",
                                 fs / "SLUS_007.76", fs / "MGS" / "SLUS_007.76"]
                     if p.exists()), None)
    if exe_path is None:
        candidates = sorted(fs.rglob("SLUS_*"))
        exe_path = candidates[0] if candidates else None
    if exe_path is None:
        raise SystemExit(f"no SLUS executable found under {fs}")
    stage_path = fs / "MGS" / "STAGE.DIR"
    if not stage_path.exists():
        raise SystemExit(f"missing {stage_path}")

    exe = exe_path.read_bytes()
    builtin = load_builtin_se(exe)

    if args.list:
        for code, e in enumerate(builtin):
            if e:
                print(f"{code:3d}  {e.name:24s} tracks={len(e.seqs)} pri=0x{e.pri:02x} kind={e.kind}")
        return

    stage_data = stage_path.read_bytes()
    folders = parse_stage_dir(stage_data)
    if args.stage not in folders:
        raise SystemExit(f"stage folder '{args.stage}' not in STAGE.DIR "
                         f"(has: {', '.join(sorted(folders))})")

    sound = SoundRam()
    loaded = load_stage_waves(stage_data, folders, [args.stage], sound)
    print(f"exe: {exe_path.name} | resident waves from '{args.stage}': "
          f"{len(loaded)} banks, {sum(x['entries'] for x in loaded)} samples")

    only = set(args.only) if args.only else None
    manifest: list[dict] = []
    out_dir = args.out
    if not args.music_only:
        n = render_entries(builtin, sound, out_dir / "builtin", "se", args.tick_hz,
                           only, manifest)
        print(f"rendered {n} built-in SEs -> {out_dir / 'builtin'}")

    for stage_name in args.efx_stages:
        if stage_name not in folders:
            print(f"skip unknown stage '{stage_name}'")
            continue
        stage_sound = SoundRam()
        load_stage_waves(stage_data, folders, [args.stage, stage_name], stage_sound)
        stage_manifest: list[dict] = []
        for entry in folders[stage_name]:
            if entry["type"] != "e":
                continue
            payload = stage_data[entry["offset"] : entry["offset"] + entry["size"]]
            bank = load_efx_bank(payload)
            cnt = render_entries(bank, stage_sound,
                                 out_dir / "stage" / stage_name, f"{entry['id']:04x}_",
                                 args.tick_hz, only, stage_manifest)
            print(f"stage {stage_name} bank {entry['id']:04x}: {cnt} effects")
        manifest.extend({"stage": stage_name, **m} for m in stage_manifest)

    if args.music or args.music_only:
        music_rows = render_music(stage_data, folders, args.stage,
                                  out_dir / "music", args.music_tick_hz,
                                  args.music_max_seconds, args.music_stages,
                                  args.jobs)
        manifest.extend({"category": "music", **m} for m in music_rows)
        print(f"rendered {len(music_rows)} songs -> {out_dir / 'music'}")

    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"wrote {out_dir / 'manifest.json'} ({len(manifest)} sounds)")


if __name__ == "__main__":
    main()
