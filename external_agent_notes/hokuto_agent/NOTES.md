# Hokuto no Ken II — Mega Drive reverse-engineering notes

**Target:** `Hokuto no Ken - Shin Seikimatsu Kyuuseishu Densetsu (Japan).md`
**Goal:** enough information for a pixel-perfect re-implementation in Rust + Bevy.
**Tool under test:** `mdx` (MCP server `mdx`, 17 tools) — see `MDX_HICCUPPS.md`.

Every claim below is backed by an `mdx` operation or by an explicitly-labelled
independent read of the ROM file. Addresses are **CPU bus addresses**; on this
cartridge bus address == cartridge offset (cartridge is mapped at `0x000000`).

---

## 0. Identity

| Field | Value | Source |
|---|---|---|
| SHA-256 | `3318f7dde3fa54c69b3144a00abe7501c42277609dc0743566f246428180f4a1` | `artifact_import` |
| Size | 524288 B (512 KiB) | `header` |
| Console | `SEGA MEGA DRIVE` | `header` |
| Copyright | `(C)SEGA 1989.APR` | `header` |
| Domestic name | `HOKUTO NO KEN II` | `header` |
| Region note | `J` (Japan) | `header` |
| Overseas name | *(empty — domestic-only ROM)* | `header` |
| Serial | `GM 00004012-00` | `header` |
| SRAM declared | `0xFF0000–0xFFFFFF` (save RAM present) | `header` |

ROM declared range `0x0000–0x7FFF` (32 KiB) does not match the 512 KiB file; mdx
reports `declared_range_matches_file: true` because it computes
`declared_rom_len` from the file, not from the header field. The header field is
simply unused, which is normal for this era of cartridge.

---

## 1. Reset state and vector table

```
0x00  reset SP   = 0x00FFFE00     (top of work RAM — stack lives in WRAM)
0x04  reset PC   = 0x00000200     (cartridge offset 0x200)
```

Full 64-entry vector table, verified two ways (`mdx_header` + independent read
of the file):

| Offset | Index | 68000 meaning | Value |
|---|---|---|---|
| `0x00` | 0 | reset SP | `0x00FFFE00` |
| `0x04` | 1 | reset PC | `0x00000200` |
| `0x08`–`0x3C` | 2–15 | bus error … spurious #8 | `0x00000200` |
| `0x40`–`0x74` | 16–29 | auto-vector 1–14 | `0x00000200` |
| **`0x78`** | **30** | **auto-vector 6 = VBlank** | **`0x00000DB4`** |
| `0x7C`–`0xFF` | 31–63 | auto-vector 15 … IRQ 7 | `0x00000200` |

**The only real interrupt vector in the ROM is index 30 → `0x00000DB4`.**

Index 30 is the auto-vector for interrupt level 6. On a Mega Drive VBlank is
IRQ level 6, and it is taken as an *autovector* precisely when the VDP's IRQ
vector register (VDP reg 23, upper bits, at `$FFFF78`) has not been programmed.
`mdx` confirms reg 23 = `253` (`0xFD`), i.e. only HScroll-base bits are set and
no IRQ vector. So the table is correct hardware behaviour, not a defect.

Everything else pointing at `0x00000200` means "no handler installed": a stray
bus/address error would re-enter the init stub. For re-implementation this
table can be copied verbatim.

---

## 2. Power-on init stub (`0x000200`)

Stock Sega third-party init. Annotated from `mdx_disasm`:

```
0x200  MOVE.B  $A10001,D0        ; Mega Drive hardware version register
0x206  ANDI.B  #$0F,D0
0x20A  BEQ    $216               ; version 0 → no 32X
0x20C  MOVE.L  #$53454741,$A14000  ; "SEGA" → TMSS handshake, skip 32X
0x216  BTST.B  #6,$A1000D        ; region byte: bit6 = overseas flag
0x21E  BNE    $25A               ; overseas → skip checksum (JP cart: continue)
0x222  LEA    $200,A0            ; --- ROM self-test: sum words ---
0x228  LEA    $1A4,A1            ; expected end pointer + expected sum
0x22E  MOVE.L (A1),D0
0x230  MOVEQ  #0,D1
0x232  ADD.W  (A0)+,D1           ;   sum 16-bit words
0x234  CMP.L  A0,D0
0x236  BCC    $232               ;   until A0 reaches the expected end
0x238  LEA    $18E,A1
0x23E  CMP.W  (A1),D1
0x240  BNE    $2FA               ; checksum fail → error path
0x244  LEA    $FE00,A6           ; clear system work RAM (0x80 longwords)
0x248  MOVEQ  #0,D7
0x24A  MOVE.W #$7F,D6
0x24E  MOVE.L D7,(A6)+
0x250  DBRA.W D6,$24E
0x254  MOVE.W #1,$FE24           ; system control: 68k/VRAM enable
0x25A  MOVE.W #$8134,$C00004     ; VDP control: VRAM write, CD 1 odd/even
0x262  MOVE.W #0,$FE26           ; clear work-RAM request flag
0x268  LEA    $FF0000,A6         ; clear all 64 KiB work RAM ($3EFF longwords)
0x26E  MOVEQ  #0,D7
0x270  MOVE.W #$3EFF,D6
0x274  MOVE.L D7,(A6)+
0x276  DBRA.W D6,$274
0x27A  JSR    $A148              ; Mega Drive hardware init
0x280  JSR    $A1A2              ;   (A148/A1A2/A6FA/A1B8 are ROM-local stubs)
0x286  JSR    $A6FA              ; VDP / PSG init
0x28C  LEA    $F330,A3
0x290  JSR    $12266             ; game-specific init
0x296  JSR    $A1B8
0x29C  MOVE.W #0,$F000
0x2A2  TST.W  $FE26
0x2A6  BEQ    $2B4
0x2A8  MOVE.W #4,$F000
0x2AE  MOVE.W #0,$FE26
```

Notes for a re-implementation:

* The self-test sums **16-bit words** from `$200` to a limit read from `$1A4`
  and compares against the word at `$18E`. mdx reports this loop
  (`0x232/0x234/0x236`) as ~262 k hits in a 60-frame run; it is genuinely
  executed and the game proceeds, so the stored checksum matches.
* All `0xA1xx` destinations are the **I/O space** — on the Mega Drive
  `$A00000–$A0001F` is the Z80 bus and `$A10000–$A1001F` the VDP/IO chip, mirrored
  through the `$A11100/$A11200` and `$A15100/$A15200` aliases. `$A148` etc. are
  *RAM-resident* helper stubs the init copies there, not ROM entry points.
* `$FE00`, `$FE24`, `$FE26` are the standard Sega system work-RAM page.

### Sega splash wait routine (`0x000`+`0x10D0`)

```
0x10D0  MOVE.B  #1,$F0A0       ; set "pending" flag
0x10D6  MOVE.W  #$2500,SR     ; supervisor, interrupt mask 5 → VBlank enabled
0x10DA  TST.B  $F0A0          ; spin until the VBlank ISR clears it
0x10DE  BNE    $10DA
0x10E0  RTS
```

Confirmed with `mdx_watch` on WRAM folded offset `61600` (`$00F0A0`):

| Frame | Writer PC | Value | Meaning |
|---|---|---|---|
| 58–64, repeating | `0x10D0` | `1` | routine sets the flag |
| 58–64, same frame | `0x0DC4` | `0` | VBlank handler clears it |

So `0x0DB4` (the vector target) is the VBlank handler and `0x0DC4` is where it
clears the flag. **This routine is a per-frame VBlank barrier, not a hang** —
`0x10DA/0x10DE` dominating the profile is exactly what a 2-instruction poll loop
looks like when the CPU is waiting on the display.

---

## 3. Boot behaviour observed

`mdx_boot`. The 3-button pad is `up/down/left/right/a/b/c/start`
(`crates/mdcore/src/input.rs`), active-low, ports 0/1.

| Frames | Instructions | VInt req | VInt taken | Screen | Colours |
|---|---|---|---|---|---|
| 60 | 926,154 | 60 | 3 | SEGA splash | 13 |
| 300 | 3,719,280 | 300 | **239** | Japanese **STORY** text screen | 2 |
| 1100 + input | 13,022,359 | 1100 | **984** | **gameplay**, beat-'em-up stage | 32 |

* At 60 frames interrupts are still masked by init (SR `$2700`); they are opened
  at `0x10D6`. That is why `vint_taken` looked broken at first glance.
* By 300 frames the game has left the logo and is rendering Japanese text
  correctly — mdx's Mode 5 renderer, font tiles and palette path all work here.
* The 1100-frame run used `captures/play_input.json`, which presses START for
  60 frames every 100 frames from frame 240. That walks the splash → story
  pages → stage 1. All 17 events applied.
* Audio is live throughout: `z80_cycles: 317,689,526`, `z80_pc: 196`,
  `z80_ram_nonzero: 3369`, `z80_uploads: 11,666` (driver upload),
  `fm_writes: 3,266,370`. The Z80 sound driver is resident and the YM2612 is
  being driven from frame ~1.
* DMA: `dma_requests: 1209`, `dma_bytes: 595,584`,
  `dma_bus_stall_master_cycles: 7,356,681` — the game bulk-loads tilemaps and
  tiles into VRAM.

Screenshots on disk:

| Path | What |
|---|---|
| `captures/boot_f60/frame-ba88cf29….png` | SEGA splash |
| `.mdx/artifacts/captures/frame-539968c7….png` | STORY text screen |
| `captures/boot_play/frame-89226445….png` | **gameplay**: brick wall stage, player sprite, LIFE/Power/SCORE/TIME HUD |

Gameplay frame details:

* Side-scrolling beat-'em-up (Final Fight style). Brick-wall backdrop with two
  shuttered windows and a curtain, a player character standing left, and a HUD
  strip along the bottom with `LIFE`, `Power`, `SCORE` and `TIME`.
* Sprite table base moved: VDP reg 22 = `248` (`0xF8`) → `(0xF8 & 0x7F) << 9`
  = `0xF000` (it was `0x8000` during the story screen). The game relocates the
  SAT between screens.
* `vram_bytes_used_nonzero: 38,565`, `cram_entries_nonzero: 57`,
  `vsram` writes 1240 (sprite attributes are being updated per frame).

---

## 4. VDP state (frame 300, after init)

`registers` is VDP reg 0–31:

| Reg | Value | Meaning |
|---|---|---|
| 0 | `0x04` | Mode 5, display on |
| 1 | `0x74` (`116`) | Mode5, DMA = VRAM+CPU-write, **VInt enable**, HV latch |
| 2 | `0x30` (`48`) | Plane A name table → `0xC000` |
| 3 | `0x2C` (`44`) | Plane B / window name table → `0xB000` |
| 4 | `0x07` | Plane A h-scroll table → `0x0700` |
| 5 | `0x54` (`84`) | Plane B h-scroll table → `0x5400` |
| 12 | `0x81` | HBlank interval = 1 |
| 13 | `0x2B` | Total H per line = 43 |
| 15 | `0x02` | Auto window width |
| 16 | `0x11` (`17`) | Window V position = 17 |
| 17 | `0x00` | **Window H position = 0 → window covers the whole screen** |
| 22 | `0xC0` (`192`) | Sprite attribute base → `(192 & 0x7F) << 9` = `0x8000` |
| 23 | `0xFD` (`253`) | HScroll base → `0xF400`; **no IRQ vector programmed** |

Because reg 17 = 0, the horizontal window is enabled across the full display, so
`mdx.graphics` labels the reg-3 table as `window` and `plane b`. That is correct
behaviour, not a mislabel — on this game the window is a full-screen overlay.

Derived VRAM map:

```
0x0000-0x3FFF   Plane A tiles (reg2 = 0x30)
0x7000          Plane A h-scroll
0x8000          Sprite attribute table (80 sprites)
0xA000-0xAFFF   Sprite pattern data
0xB000          Plane B / window name table
0xC000          Plane A name table
0xD000-0xDFFF   Plane B tiles
0xF400          HScroll table
```

---

## 5. Palette (CRAM, frame 300)

mdx `palette_rgb`, 64 entries. Palette 0 is a Sega-standard blue ramp:

```
 0 000000        1 EEEEEE        2 00CEEE        3 00AAEE
 4 0089EE        5 0065EE        6 0044EE        7 0020EE
 8 0000EE        9 0000CD       10 0000AC       11 00008B
12 000062       13 000000       14 000000       15 000000
16 ACAACA       17..63 = 000000
```

So the story screen uses **palette 0 line 1 (white) on line 0 (black)**, plus a
grey at palette 0 index 16. `distinct_colours: 2` on screen matches. Palettes
2 and 3 are empty at this point (`cram_entries_nonzero: 13`).

---

## 6. Graphics inventory (frame 300)

`mdx_graphics` → `captures/gfx300/`:

* 2048 Mode-5 patterns (full 8 KiB pattern space read from VRAM `0x0000`)
* **189 non-empty tiles** — only these matter for a faithful re-implementation
* 3 complete name tables, 64×64 cells each (window table 64×32)
* 80 sprite slots, 1 linked sprite
* 4 palette sheets, sheet width 32 tiles
* `transparent_index: 0`

> mdx caveat carried into any port: *tile sheets render index 0 as an opaque
> swatch; a native compositor must treat it as transparent.*

---

## 7. I/O observed

From `mdx_boot`'s `io_trace` (frame 60, first 512 entries):

```
0x00A10009=0x40  ×2    ; Version register read
0x00A1000B=0x40  ×2    ; Controller/PIO data port 1
0x00A1000D=0x40  ×2    ; Controller/PIO data port 2  → region/TH bytes
0x00A11100=…             ; Z80 bus data (write)
0x00A11101=…             ; Z80 bus control (write)
0x00A11200=…             ; Z80 bus reset (write)
0x00A10003=0x00/0x40    ; VDP data port (high/low)
0x00A10005=0x00/0x40    ; VDP control port
```

The `0xA10003/05` = `0x00`/`0x40` alternating is the **VRAM DMA length word**
(39-bit DMA length split over the two writes, high byte `0x40`), i.e. the game
is doing 68k→VRAM DMA fills. Consistent with the VRAM-writing loop seen at the
`0x10D0` call site (`a5 = $C00000` data port, `a6` = VRAM destination cursor).

---

## 8. Screen layout from the Plane A name table

Plane A is at `0xC000`, 64 columns wide. The Mega Drive screen is
**320×224 = 40×28 tiles**, so columns 40-63 are the off-screen scroll buffer.

Reading the gameplay snapshot's `0xC000` table:

| Plane A rows | Content | Tile indices seen | Palette line |
|---|---|---|---|
| 0-2 | sky / empty above the wall | `6000` (tile 0 => transparent => backdrop) | 6 |
| 3-18 | brick wall, two shuttered windows, curtain | `6010`-`6068` | 6 |
| 19-21 | ground / floor, alternating pair | `6063` / `6064` | 6 |
| 22 | separator line | `E063` / `E064` | **0xE (line 14)** |
| 23-26 | **HUD**: LIFE / Power bars, SCORE digits | `A500`-`A53A`, `AD00`-`AD16` | **0xA (line 10)** |
| 27 | HUD second line | `B500` / `B501`, `BD00` | **0xB (line 11)** |

* **The HUD is tilemap-based, not sprites** - it lives in Plane A rows 22-27,
  columns 0-39, and switches to its own palette lines (0xA/0xB/0xE) rather than
  sprite palettes. `AD00`, `AD02`, `AD16`, `BD00` at column 39 are the
  `LIFE` / `Power` / `SCORE` / `TIME` labels.
* The full-screen horizontal window (reg 17 = 0) means Plane B is never visible;
  `mdx.graphics` therefore reports the reg-3 table as `window` and also invents a
  `plane b` at `0xE000` - see hiccup #9.
* The window nametable at `0xB000` contains a completely different, much denser
  pattern (indexes like `B111`, `6616`, `1BA9`, high palette bits set). **122 of
  its 125 64-byte windows appear verbatim in the ROM at `0x016366`** - it is a
  static screen held in ROM, most likely the title / character-select background.

---

## 9. Graphics decompression - recovered algorithm

No VRAM tile matched the ROM byte-for-byte, but ~373 KB of the ROM is
graphics-shaped (`0x24000`-`0x7F000`). The missing step is this routine, found
via `hottest_addresses` and read with `mdx_disasm`.

**`0x00A440` - 32-byte block RLE decoder**

```
0xA440  SUBQ.W  #1,D2
0xA442  MOVE.L  A4,A2            ; A2 = write cursor into the staging buffer
0xA444  MOVE.B  (A0)+,D3         ; [0] FILL byte
0xA446  MOVE.B  (A0)+,D4         ; [1] mask byte 0
0xA448  LSL.W   #8,D4
0xA44A  MOVE.B  (A0)+,D4         ; [2] mask byte 1
0xA44C  SWAP    D4
0xA44E  MOVE.B  (A0)+,D4         ; [3] mask byte 2
0xA450  LSL.W   #8,D4
0xA452  MOVE.B  (A0)+,D4         ; [4] mask byte 3
0xA454  OR.L    D4,D0            ; accumulate "all fill" test
0xA456  MOVEQ   #$1F,D7          ; 32 outputs per block
0xA458  LSL.L   #1,D4            ; shift the top bit out
0xA45A  BCC.S   0xA45E           ; bit clear -> position is NOT a fill
0xA45C  MOVE.B  D3,(A2)
0xA45E  ADDQ.W  #1,A2
0xA460  DBRA.W  D7,0xA458
0xA464  DBRA.W  D2,0xA442         ; repeat for the next block
0xA468  MOVEQ   #$FF,D1
0xA46A  XOR.L   D0,D1            ; D1 = ~D0 ; == 0 when every bit was set
0xA46C  BEQ.S   0xA47E           ; all-fill block -> no literals to consume
0xA46E  MOVE.L  A4,A3
0xA470  MOVEQ   #$1F,D7
0xA472  ADD.L   D0,D0
0xA474  BCS.S   0xA478           ; bit set -> already filled above
0xA476  MOVE.B  (A0)+,(A3)       ; bit clear -> consume a literal byte
0xA478  ADDQ.W  #1,A3
0xA47A  DBRA.W  D7,0xA472
0xA47E  MOVE.L  A4,A6
0xA480  MOVE.L  (A6)+,(A5)       ; 8 x longword -> 32 bytes to the VDP
       ... x8
0xA490  BRA.S   0xA436
0xA492  RTS
```

`A0` = compressed source (ROM), `A4` = staging buffer, `A5` = `0xC00000` VDP
data port.

### Stream format

```
per 32-byte output block:
    byte[0]      FILL value
    byte[1..4]   32-bit mask, BIG-ENDIAN, bit 31 = first output pixel byte
    then N literal bytes, N = popcount(~mask), in output order

    if mask == 0xFFFFFFFF the literal pass is skipped entirely
    (a solid block costs exactly 5 bytes)
```

Bits are consumed **MSB-first** (`LSL.L #1` walks bit 31 -> bit 0).

### Verification

* Solid-fill headers `[00 FF FF FF FF]` occur **114x** in the ROM; the same
  pattern with fills `0x11` (5x), `0x88` (3x), `0x22` (1x), `0xAA` (2x).
* `0x025B29` decodes byte-exactly to VRAM tile 467.
* Inverting the transform over every non-empty VRAM tile (trying the 3 most
  common bytes as the FILL candidate) locates **702 of 1621 tiles** in the ROM -
  a lower bound, since the heuristic misses tiles whose FILL byte also occurs
  among their literals.

### Where the tiles live

| ROM range | ~tiles | Notes |
|---|---|---|
| `0x015867`-`0x01599A` | 8 | small set, likely font/marker tiles |
| `0x023E26`-`0x025B4E` | 201 | |
| `0x02B824`-`0x0301AE` | 508 | largest block |
| `0x034E04`-`0x037A27` | 305 | |
| `0x04CF45`-`0x0508EB` | 398 | |
| `0x055C8B`-`0x05816F` | 255 | |
| `0x079DC5`-`0x07F210` | 583 | |

Machine-readable map: **`captures/tile_to_rom.json`** (`{vram_tile: rom_offset}`).

The ~919 VRAM tiles not yet located come from elsewhere - sprite patterns, the
static window map at `0x016366`, tiles produced by other decoder variants, or
runtime composition. Those still need tracing.

### Implication for the port

The graphics pipeline is: **RLE-decode 32-byte blocks from ROM -> 68k writes them
to `0xC00000`**. A Rust/Bevy port does not need the 68k; it can run this decoder
directly on the ROM and upload decoded blocks as textures. Exactness requires
this format (or an equivalent decode of the same bytes) plus the CRAM values in
section 5.

---

## 10. Backing assets already extracted

`captures/gfx_play/` (gameplay snapshot):

| File | What |
|---|---|
| `palette-0...png` ... `palette-3...png` | all 2048 tiles x 4 palettes, 32 tiles per row |
| `tiles-...indices.bin` | raw tile index per pixel (131 072 B = 2048 x 64) |
| `layout-...json` | full sprite / nametable / tile index structure |

`captures/gfx300/` - the same for the story screen.

Remember mdx's caveat: **tile index 0 is drawn opaque in the sheets but is
transparent in hardware.**

---

## 11. RAM map and game state

WRAM is 64 KiB folded at `0xFF0000-0xFFFFFF` (WRAM accepts folded offsets or
full CPU aliases in `mdx_memory_read`). Derived from the gameplay capture plus a
WRAM diff between *idle* (frame 1100) and *walked right* (frame 1400) — 358
bytes differ, in 60 runs.

Contiguous non-zero regions at frame 1100:

| Folded | CPU | Size | Role |
|---|---|---|---|
| `0x0060`-`0x0480` | `$FF0060` | 1056 B | Sega system work RAM / misc globals |
| `0x5000`-`0x6A00` | `$FF5000` | 6144 B | **main game state block** |
| `0xEA00`-`0xEC00` | `$FFEA00` | 512 B | ROM pointer table |
| `0xF360`-`0xF460` | `$FFF360` | 256 B | counters / timers |
| `0xF700`-`0xF850` | `$FFF700` | 336 B | **object table** |
| `0xFB80`-`0xFBE0` | `$FFFB80` | 96 B | sprite staging |
| `0xFD80`-`0xFE30` | `$FFFD80` | 176 B | stack + system vars |

Everything else in WRAM is zero — the game uses a small, clean data layout.

### Identified variables

* **`0xFF5000`-`0xFF505E` (94 B) — name-table strip buffer.**
  Contains Mode 5 name-table words being composed for the scroll. Idle:
  `00 60 00 60 …` (word `0x6000` repeated); walking right:
  `15 60 1F 60 15 60 1F 60 …` → `0x1560` (palette 2, tile `0x560`) alternating
  with `0x601F` (priority 1, tile `0x01F`). This is the code that streams new
  columns of the stage in as the camera advances.

* **`0xFF740 + 10*n` — object/enemy table, 10-byte stride.**
  Ten entries materialise when the player walks right:

  | n | addr | idle | walking right |
  |---|---|---|---|
  | 0 | `0xFFF740` | `00 00 00` | `FF FC 30` |
  | 1 | `0xFFF74A` | `00 00 00` | `FF F8 60` |
  | 2 | `0xFFF754` | `00 00 00` | `FF F4 90` |
  | 3 | `0xFFF75E` | `00 00 00` | `FF F0 C0` |
  | 4 | `0xFFF768` | `00 00 00` | `FF EC F0` |
  | 5 | `0xFFF772` | `00 00 00` | `FF E9 20` |
  | 6 | `0xFFF77C` | `00 00 00` | `FF E5 50` |
  | 7 | `0xFFF786` | `00 00 00` | `FF E1 80` |
  | 8 | `0xFFF790` | `00 00 00` | `FF DD B0` |
  | 9 | `0xFFF79A` | `00 00 00` | `FF D9 E0` |

  Byte 0 is `0xFF` for all live entries (likely a type/active flag), and byte 1
  steps down by ~4 per entry — evenly spaced scenery objects across the screen.
  This is the game's own 3-byte live object record; the 68k converts it into
  hardware sprite-table entries later.

* **`0xFFE000`-`0xFFE300` — ROM pointer table.**
  Holds live pointers into the cartridge, e.g. `0x0583EE` at `0xFFE305`,
  `0x058394` at `0xFFE329`. These advance while scrolling: they are the
  read cursors for the next slice of compressed graphics and/or level data.
  Good anchors for tracing the load path.

* **`0xFFF831`-`0xFFF87F` (79 B) — streaming block.**
  Zero while idle, densely populated while walking. Another cursor-followed
  data slice.

* **`0xFFFBC3`, `0xFFFBFC` etc. (`0xFFFB80`, `0xFFFBB8`)** — repeated `FF Cx`
  pairs appearing as the player moves: sprite-attribute staging.

* **VBlank handshake**: `$FF0A0` set by `0x10D0`, cleared by the IRQ at
  `0x0DC4`. Nearby `$FF0A3`-`$FF0A6` change every frame — likely frame/animation
  timers.

---

## 12. Verified behaviour

Confirmed by two independent replays (`captures/play_input.json`,
`captures/play_right.json`, all events applied):

| Input | Result |
|---|---|
| none | SEGA splash → STORY text screen, then idle |
| START mash | walks through the story pages into stage 1 |
| RIGHT held | player walks right, **stage scrolls**, new scenery enters |
| RIGHT + enemy present | second character sprite engages the player |

Frame 1400 (`captures/boot_right/frame-edac68213….png`) shows the brick-wall
stage scrolled to a section with two doorways, the player mid-swing against a
blue/red enemy, and a live HUD: `LIFE`, `Power`, `SCORE 0186593900`, `TIME 58`.

Live VDP state at frame 1400:

* `vint_requests: 1400`, `vint_taken: 1284` — interrupts healthy.
* `dma_requests: 1999`, `dma_bytes: 822,752`.
* `z80_cycles: 403,950,028`, `z80_uploads: 11,666`, `fm_writes: 4,194,312`.
* VDP regs written every frame: `15=1, 19=255, 20=3, 23=128, 15=2` — the
  horizontal/vertical scroll registers for the stage plus window adjustments.
* `vram_data_counts: cram 64, vram 38740, vsram 1740`.

---

## 13. Object system, RNG, and corrected VDP addressing

### Entity update loop — `0x78A6`

```
0x78A6  LEA.L  64(A0),A0         ; advance to the next record (0x40 stride)
0x78AA  MOVEQ  #$20,D7           ; 33 iterations
0x78AC  MOVE.L  D7,-(A7)         ; save loop counter
0x78AE  MOVEQ  #0,D0
0x78B0  MOVE.B  0(A0),D0         ; record byte 0 = entity type
0x78B4  BEQ.B   0x78E6           ; type 0 -> inactive, skip
0x78B6  ANDI.W  #$7F,D0
0x78BA  ASL.W   #2,D0            ; index = (type & 0x7F) * 4
0x78BC  JSR     62(PC,D0.W)      ; dispatch through the thunk table at 0x78FC
0x78C0  BSR.W   0x7834           ; shared per-entity update
0x78C4  TST.B   0(A0)
0x78C8  BEQ.B   0x78E6
0x78CA  TST.W   $F0B8            ; global "camera moving" flag
0x78CE  BEQ.B   0x78E2
0x78D0  BTST.B  #2,1(A0)         ; per-entity opt-out flag
0x78D6  BNE.B   0x78E2
0x78D8  MOVE.L  $E01C,D0         ; NOTE: $0000E01C is CARTRIDGE (see below)
0x78DC  NEG.L   D0
0x78DE  ADD.L   D0,20(A0)        ; bias record+0x14 while the camera scrolls
0x78E2  JSR     -278(PC)         ; -> 0x77B0
0x78E6  LEA.L   64(A0),A0
0x78EA  MOVE.L  (A7)+,D7
0x78EC  DBRA.W  D7,0x78AC
0x78F0  MOVEA.W $F0A2,A2
0x78F4  MOVE.L  #0,(A2)          ; clear a counter beside the VBlank flag
0x78FA  RTS
```

* **33 entity slots, 64-byte (`0x40`) records.**
* Record byte 0 is the type (0 = free), byte 1 bit 2 is a "don't scroll-bias me"
  flag, record+`0x14` is a position biased while the camera moves.
* The `JSR 62(PC,D0.W)` target is `0x78FC + 4*(type & 0x7F)`, and each slot holds a
  4-byte `BRA.W` thunk. Because `JSR` pushes `0x78C0`, the thunk's `BRA` redirects
  to the real handler and the handler's `RTS` unwinds straight back to `BSR.W
  0x7834` — no stack fixup needed.

**Dispatch table: 92 entries (`type & 0x7F` = 0…91) at `0x078FC`…`0x07A28`.**
First targets (corrected for the mdx branch bug — mdx reports each exactly 2 low):

| type | handler | | type | handler | | type | handler |
|---|---|---|---|---|---|---|---|
| 0 | `0x07A6E` | | 14 | `0x07A6E` | | 60 | `0x0BB80` |
| 1 | `0x07C8C` | | 15 | `0x07A6E` | | 61 | `0x0C1DC` |
| 2 | `0x07C84` | | 16 | `0x039E6` | | 62 | `0x0BBAC` |
| 3 | `0x01F8C` | | 17 | `0x03C04` | | 63 | `0x0C4A4` |
| 4 | `0x01C3E` | | 18 | `0x03C90` | | 64 | `0x0C994` |
| 5 | `0x0994E` | | 19 | `0x06ECC` | | 65 | `0x0CEBE` |
| 6 | `0x07A74` | | 20 | `0x07246` | | 66 | `0x0D38C` |
| 7 | `0x024A8` | | 21 | `0x07580` | | 67 | `0x0D4F4` |
| 8 | `0x09810` | | 22 | `0x075FE` | | 68 | `0x0D980` |
| 9 | `0x0987C` | | 23 | `0x07246` | | 69 | `0x0DDFA` |
| 10 | `0x02478` | | 56 | `0x0AA2A` | | 70 | `0x0E298` |
| 11 | `0x098B8` | | 57 | `0x0AE5A` | | 71 | `0x0E574` |
| 12 | `0x0094E` | | 58 | `0x0B1E2` | | 72 | `0x0EAE6` |
| 13 | `0x00ABE` | | 59 | `0x0B6B8` | | 73 | `0x0EC5E` |

Handlers are spread from `0x0094E` to `0x0EAE6` — the entity behaviours occupy
most of the first `0x0F000` of the cartridge.

### Graphics job table at `$FFA800`

Not the sprite table (see hiccup 12). 8-byte records `[w0][w1][w2][dst]` where
`dst` is a VRAM destination word. At frame 1100 the first entries were

```
00E0 0401 0580 00B8
00E8 0F02 0582 00B0
0100 0A03 05A0 00B4
0118 0A04 05A9 00B4
0170 0005 26EE 00D0
0170 0006 2000 00F0
```

and after walking right the `dst` words advanced by `+0x80`
(`00B8→0138`, `00B0→0130`, `00B4→0134`). This is the per-frame tile-upload
queue, consistent with the `A6 = 0x15860` staging cursor seen in CPU traces.

### RNG — `0x10F0`

```
0x10F0  MOVE.L  $FE00,D1                 ; previous seed
0x10F4  BNE.B   0x10FC
0x10F6  MOVE.L  #$2A6D365A,D1            ; fallback seed on first call
0x10FC  MOVE.L  D1,D0
0x10FE  ASL.L   #2,D1                    ; d1 = d0*4
0x1100  ADD.L   D0,D1                    ; d1 = d0*5
0x1102  ASL.L   #3,D1                    ; d1 = d0*40
0x1104  ADD.L   D0,D1                    ; d1 = d0*41
0x1106  MOVE.W  D1,D0
0x1108  SWAP    D1
0x110A  ADD.W   D1,D0                    ; fold the high half back in
0x110C  MOVE.W  D0,D1
0x110E  SWAP    D1
0x1110  MOVE.L  D1,$FE00                 ; store new state
0x1114  RTS
```

State lives in the Sega system work RAM at `$FFFE00`; the 16-bit result is
returned in the high half of the returned longword.

### VDP register → VRAM address (correct units)

Reg 2/3/4/5/23 use unit `0x400`; **reg 13 uses `0x200`**; reg 22 is the sprite
attribute table with unit `0x200` and only 7 bits. With the gameplay register
set `[…, 48, 44, 7, 84, …, 129, 43, …, 248, 253, …]`:

| Register | Value | Address | Contents |
|---|---|---|---|
| 2 | 48 | `0x0C000` | Plane A name table |
| 3 | 44 | `0x0B000` | Plane B / window name table |
| 4 | 7 | `0x01C00` | Plane A h-scroll |
| 5 | 84 | `0x15000` | Plane B h-scroll |
| 13 | 43 | `0x05600` | h-scroll counter table (unit `0x200`) |
| 22 | 248 | `0x0F000` | sprite attribute table (`(248 & 0x7F) << 9`) |
| 23 | 253 | `0x0F400` | window h-scroll (253·`0x400` masked to 16 bits) |
| 12 | 129 | — | RS1 + LS set; **bit 2 clear ⇒ reg 13 is the HSCR value, not a table base** |

Reg 12 bit 2 clear means the whole-screen horizontal scroll is the constant
`reg13 = 43`. Combined with the observation that the h-scroll tables are
byte-identical between the idle and walking frames, **the stage is not scrolled
by hardware scroll at all** — only ~6 tiles per name-table row are rewritten per
frame. The scrolling is therefore done by editing the name table directly.

`$0000E01C` (read by the entity loop) resolves to **cartridge**, because region
decode keys off address bits: `addr & 0x00C0_0000 == 0` selects cartridge, else
`addr & 0x00E0_0000 == 0x00E0_0000` selects work RAM. So that instruction reads
the ROM longword `0x303C0024`, not a variable.

---

## 14. Open questions / next steps

1. **Reach gameplay.** Boot with a controller replay (START press) and capture
   the title screen and in-game frames. This is the single highest-value next
   step for pixel accuracy.
2. **VRAM usage diff.** `memory_diff` between the STORY snapshot and a gameplay
   snapshot to see which planes/tiles actually change.
3. **ROM asset layout.** Locate in the 512 KiB ROM the source tiles, tilemaps
   and palettes that get DMA'd into VRAM — `memory_search` the ROM artifact for
   tile-shaped data or follow the DMA source pointers in `$FFxxxx`.
4. **Main loop / state machine.** Disassemble from the caller of `0x10D0`.
5. **Audio.** The Z80 driver is resident in Z80 RAM; dump and disassemble it.
6. **WRAM variable map.** `memory_search` / `ram_dump` over the snapshot WRAM to
   locate the game-state block.

---

## Appendix — reproduction

```bash
cd /home/maisy/opencode_projects/hokuto_reverse
MDX=/home/maisy/opencode_projects/mdx/target/release/mdx
ROM="extracted/Hokuto no Ken - Shin Seikimatsu Kyuuseishu Densetsu (Japan).md"

# identity
$MDX call artifact.import --args "{\"path\":\"$ROM\"}"
$MDX call header --args "{\"rom\":\"$ROM\"}"

# disassembly (note: `from` is a CPU BUS address; cart maps at 0x000000)
$MDX call disasm --args "{\"rom\":\"$ROM\",\"from\":512,\"count\":40}"
$MDX call disasm --args "{\"rom\":\"$ROM\",\"from\":4304,\"count\":24}"

# boot + capture
$MDX call boot --args "{\"rom\":\"$ROM\",\"frames\":300,\"instructions\":20000000,\"trace\":0}"
```

MCP equivalents: `mdx_header`, `mdx_disasm`, `mdx_boot`, `mdx_watch`,
`mdx_graphics`, `mdx_render`, `mdx_memory_read`, `mdx_memory_search`,
`mdx_memory_diff`, `mdx_trace`, `mdx_ram_dump`.
---

## 14. Graphics resource loader — `0x2CA0`

This is the routine that loads a stage's graphics. It is the master key to the
asset format.

```
0x2CA2  LEA.L  0xFF7000,A6
0x2CA8  MOVE.W #$7FF,D7
0x2CAC  MOVE.L D6,(A6)+          ; clear 2048 longs
0x2CAE  DBRA.W D7,0x2CAC          ; -> zeroes $FF7000-$FF8FFF (8 KB object area)
0x2CB2  MOVE.B #$40,$F7F5        ; constant 64
0x2CB8  MOVE.W $F0B2,D7          ; resource number
0x2CBC  BSR.W  0x32C4            ; (mdx says 0x32C2)
0x2CC0  LEA.L  0x4B47C,A2        ; resource pointer table
0x2CC6  LSL.W  #2,D7
0x2CC8  MOVEA.L 0(A2,D7.W),A2    ; -> record for that resource
0x2CCC  BSR.W  0x9AA2            ; (mdx 0x9AA0) parse header + build actor tables
0x2CD0  MOVE.W (A2)+,D2          ; palette-block count
0x2CD2  MOVEA.L(A2)+,A6         ; destination
0x2CD4  BSR.W  0xA54A            ; (mdx 0xA546) palette/VSRAM upload
0x2CD8  DBRA.W D2,0x2CD2
0x2CDC  MOVE.W (A2)+,D0          ; dma-block count
0x2CDE  MOVE.L (A2)+,$C00004     ; VDP control longword
0x2CE4  MOVEA.L(A2)+,A0         ; compressed source
0x2CE6  MOVEM.L D0/A2,-(A7)
0x2CEA  BSR.W  0xA430            ; (mdx 0xA42C) blit
0x2CEE  MOVEM.L (A7)+,D0/A2
0x2CF2  DBRA.W D0,0x2CDE
0x2CF6  MOVE.L (A2)+,$F702       ; stream cursor 1  (ROM pointer)
0x2CFA  MOVE.L (A2)+,$F724
0x2CFE  MOVE.L (A2)+,$F70A
```

So a **resource record** is:

```
[header, consumed by 0x9AA2 — size is NOT constant]
word  paletteBlockCount
  { long dest }            x paletteBlockCount
word  dmaBlockCount
  { long $C00004 ; long src }  x dmaBlockCount
long  -> $F702
long  -> $F724
long  -> $F70A
```

The **resource pointer table lives at ROM `0x4B47C`** and is indexed by the
resource number in `$F0B2`. Scanning it: entries run `0x4B47C`…`0x4B56C`
(39 pointers before the first non-pointer word), covering **20 non-empty
resources** interleaved with empty ones (a record whose first word is 0 is
skipped outright by `0x9AA0`). Records span ROM `0x10108`–`0x4C62C`.
Partial machine-readable dump: `captures/graphics_resources.json`.

> The header length could not be pinned down statically: `0x9AA0` returns via
> `MOVEA.L A6,A2`, and `0x9AE8`/`0x9B22` clobber both `A2` and `A6`, so how far
> the record cursor advances depends on the actor-template copy performed
> inside. Best resolved at runtime by watching `A2` across the `BSR 0x9AA2`.

### Blitter — `0xA42C`

```
0xA42C  LEA.L  0xFAC0,A4      ; 32-byte staging buffer at WRAM $FFFAC0
0xA430  LEA.L  0xC00000,A5    ; VDP data port
0xA436  MOVEQ  #0,D0
0xA438  MOVEQ  #0,D2
0xA43A  MOVE.B (A0)+,D2       ; command byte
0xA43C  BEQ.B  0xA46E         ; 0x00 -> end of blit
0xA43E  BMI.B  0xA492         ; 0x80+ -> different opcode, not decoded here
0xA440  SUBQ.W #1,D2
0xA442  ... the RLE decoder (§9) ...
0xA46E  MOVE.L A4,A6
0xA470  MOVE.L (A6)+,(A5)     x8   ; push 32 bytes per tile
0xA490  BRA.B  0xA436         ; next command
```

* The compressed stream is a sequence of **command blocks**: a count byte
  `1..0x7F` = tiles in this block, then that many RLE tiles. `0x00` terminates
  the whole blit, `>= 0x80` is a second opcode.
* **Correction to §11:** the staging buffer is WRAM `$FFFAC0` (32 bytes, one
  tile), not `$FFF000`.
* Decoded tiles go straight out through `$C00000` a tile at a time, so the
  source bytes are consumed strictly sequentially.

### Actor definition tables — `0x9AE8` / `0x9B22`

```
0x9AE8  LEA.L 0xEA00,A1  ; then fill 35 longwords from ROM 0x24542,
                         ; + 3 groups of 3 longwords from 0x245CE..0x245E6,
                         ; + 7 longwords from 0x245EA
0x9AF8  LEA.L 0xEB00,A1  ; same shape, second table
0x9B08  CMPI.W #3,$F112
0x9B10  LEA.L 0xEC00,A1  ; third table, only when $F112 == 3
```

Fixed **actor/enemy definition blocks** are copied out of ROM `0x24542`…`0x24606`
into WRAM `0xEA00`, `0xEB00` and `0xEC00` — exactly the 768-byte non-zero
region spotted in the WRAM map. `0x9B22` then indexes a pointer table at ROM
`0x24606` by a value from `$F116`/`$F118`, dereferences it into `A0`, and runs
`0x42` (66) iterations — the per-actor initialisation loop.

**Practical consequence:** enemy/actor behaviour is table-driven from
`0x24542`, and `$F112` selects which of three actor banks is in play.

---

## 15. Corrections to earlier sections

* **§11 staging buffer** — `$FFFAC0`, not `$FFF000` (see §14).
* **§11 sprite table** — mdx's `layout.sprite_table_base = 0xA800` is wrong; the
  hardware address from `reg22 = 248` is `0xF000`, and `0xA800` is really the
  tile-upload job table. `0xF000` holds raw 4bpp pixel data
  (`4444 4414 1144 1414 …`), not sprite attributes. On this evidence the stage
  appears to be drawn almost entirely in the name tables rather than through the
  sprite layer; the per-frame name-table edits (≈6 tiles × 64 rows) are the
  moving characters. **Unresolved:** where the hardware sprites actually are, if
  any are used.
* **VDP address table in §13** is the authoritative one — `reg 13` uses unit
  `0x200`, not `0x400`.

---

## 16. Reference implementation

`tools/hokuto_gfx.py` implements the codec described above and is the artefact a
port should start from:

* `decode_tile(src, pos, blocks=1)` — 32-byte RLE block decoder.
* `iter_blits(src)` — walks the command-block framing (`0x00` ends, `0x01-0x7F`
  is a tile count, `0x80+` raises rather than guessing).
* Self-tested: 500/500 random round-trips byte-exact with exact stream positions,
  solid blocks encode to exactly 5 bytes, framing and the opcode guard verified.
* Run it as a script against a ROM + a VRAM capture to re-derive the tile map:

```
python3 tools/hokuto_gfx.py <rom> captures/boot_right/vram-<sha>.bin
```

Current result: 680 compressed tiles located by inverting the codec (an earlier
pass with a slightly different fill-candidate heuristic found 702; the number
is a lower bound, not a limit).

---

## 20. Open questions / next steps

* ~~Find the consumer of the `$F702` map cursor~~ — **done**, see section 21.
* ~~Map the index array at `$FF0000`~~ — **located and verified**, section 22.
  (Residual: reconcile the `48*R` source advance with the `($F70E-$20) = 64`
  advance the code appears to use; the empirical result is unambiguous but the
  two disagree, so one of my readings of `$F70E` is still off.)
* **Proposed mdx feature:** a `watch_registers` option on `mdx_trace`, so
  register-level tracing is possible. Two open questions still hinge on it.
* Locate the hardware sprite table, if the game uses one at all.
* Locate the remaining ~940 live VRAM tiles — likely the actor-definition tiles
  from ROM `0x24542` and whatever the `>=0x80` blit opcode emits.
* Pin the resource-record header size (needs an `A2` watch across `BSR 0x9AA2`;
  `mdx_watch` currently only observes WRAM, not registers).
* Z80 sound driver — mdx has no Z80 disassembler.


---

## 18. The level map

**The map stream lives at the `$F702` cursor — ROM `0x35B70`** (captured value
`$FF702 = 0x00035B70` during gameplay, and `A4` held the same value in CPU
traces). Evidence:

1. `$F702` is set at the tail of the loader (`0x2CF6`) as a ROM pointer, and the
   same routine stores two more cursors at `$F724` and `$F70A`.
2. The bytes at `0x35B70` are name-table words with palette 6 already applied:

```
0x35B70  6000 6000 6000 6000 6000 6000 6010 6011 6000 6000 6012 6000
0x35B7C  6010 6011 6000 6000 6012 6000 6013 6014 6015 6016 6000 6000
0x35B88  6013 6014 6015 6016 6000 6000 6017 6018 6000 6000 6019 601A
```

3. The tile-index vocabulary agrees with the live name table: 118 of the 150
   distinct indices in Plane A also occur in the first 2 KiB of the stream. The
   32 values in the name table but not the stream are all HUD/overlay tiles
   (`0x0A5`, `0x0BD`, `0x0E0`, …, i.e. indices ≥ 0x0AD), which the map does not
   describe.

### Scrolling is name-table rewriting, not hardware scroll

Comparing the idle frame (1100) with the walked frame (1400):

* **Every h-scroll table is byte-identical** between the two frames — Plane A
  `0x01C00`, Plane B `0x15000`, the counter table `0x05600`, and the window
  table `0x0F400`.
* Reg 12 is `0x81`, so **bit 2 is clear and reg 13 is the whole-screen HSCR
  value**, not a table base. It is pinned at `43`.
* Only ~6 cells at the start of each of the 64 name-table rows changed.

So the stage advances by **writing new name-table cells**, which is why the
source data is a stream rather than a flat array: the game pulls cells from the
`$F702` cursor as the camera moves.

Rows 9–19 are a repeating band (`1f 20 25 26 25 26 …`) that appears at a
different phase in the two frames, and rows 0–8 differ structurally (all-`6000`
at frame 1100, populated at frame 1400) — consistent with the camera having
reached a different part of the stage.

> **Not yet determined:** the exact per-cell layout of the stream. It is not
> row-major (`64`-byte runs: 0 matches) and not column-major (`32`-byte runs:
> 13 matches, all short). The stored order interleaves rows — e.g. the stream
> holds `6010 6011`, then `6012`, then `6013 6014`, then `6015 6016`, which in
> the name table are `r03 c12`, `r05 c10`, `r04 c12`, `r05 c12` respectively.
> Resolving this needs the code that consumes `$F702`, which has not been
> located yet.

---

## 19. Codec visual proof — `captures/codec_proof.png`

48 tiles were located by inverting the codec, decoded from the ROM, and rendered
alongside the same tiles read straight out of live VRAM using the same CRAM
palette. All 48 matched byte-for-byte, and the image shows the window frames,
brick textures, sconces and door tiles rendering identically on both sides.

This is the strongest single piece of evidence that the recovered RLE format is
correct and complete.

---

## 21. Map expansion solved — `0x36C0`

The consumer of `$F702` is `0x36C0`, called from `0x2D70` and `0x2E88` right
after `MOVEA.L 0xF702,A3`.

```
0x36C0  MOVE.W $F728,D2
0x36C4  NEG.W D2
0x36C6  ASR.W #4,D2              ; column offset
0x36C8  MOVE.W $F718,D0
0x36CC  ASR.W #4,D0
0x36CE  ADD.W D2,D0              ; row = ($F718 - $F728) >> 4
0x36D2  MULS.W $0100,D7          ; build a VDP control longword:
0x36D6  ANDI.W #$0F80,D7         ;   addr = (row * 256) & 0x0F80
0x36DA  ORI.W  #$E000,D7         ;   | 0xE000
0x36DE  BSR.W 0xA58E             ; set VDP address
0x36E2  MOVE.W $F70E,D1          ; name-table row stride in bytes
0x36E6  MULS.W D1,D0
0x36E8  LEA.L 0(A2,D0.W),A2      ; seek into the index array
0x36EC  LEA.L 0(A2,D2.W),A2
0x36F0  SUBI.W #$20,D1
0x36F4  MOVEQ #$0F,D5            ; 16 rows
0x36F6  MOVEQ #$1F,D6            ; 32 columns
0x36F8  MOVEA.L A3,A4            ; a4 = table at $F702 (ROM 0x35B70)
0x36FC  MOVE.B (A2)+,D0          ; ONE INDEX BYTE from the index array
0x36FE  ASL.W #3,D0              ; * 8  -> 8-byte record
0x3700  LEA.L 0(A4,D0.W),A4
0x3704  MOVE.L (A4)+,(A1)+       ; write 4 name-table bytes
0x3706  MOVE.L (A4),124(A1)      ; ...and the same 4 again 31 cells later
0x370A  DBRA.W D6,0x36F8
0x370E  LEA.L 0(A2,D1.W),A2      ; next index row
0x3712  LEA.L 128(A1),A1         ; output row stride = 128 bytes = 64 words
0x3716  DBRA.W D5,0x36F6          ; 16 rows
```

### The format

The level is **two structures**, not one:

1. **An index array**, in WRAM at `$FF0000 + offset`. One **byte per cell**.
   (Built at load time; the offset comes from the row/column arithmetic above.)
2. **A shared cell table** at the `$F702` cursor — ROM **`0x35B70`** — of
   **8-byte records**. Each record supplies **two 4-byte name-table writes**:
   one at the cursor and the identical one 124 bytes (= 31 words) later, which
   is how the stage wraps horizontally at the screen edge.

Output geometry: **64 words per row, 128-byte row stride, 16 rows per pass.**

This finally explains why searching the ROM for name-table bytes found nothing:
the ROM table *is* name-table words, but the map is expressed as 8-byte
*indices* into it, one index per cell. Reading `0x35B70` as words gives

```
entry 0: 6000 6000 6000 6000
entry 1: 6000 6000 6010 6011
entry 2: 6000 6000 6012 6000
...
```

which is exactly the "interleaved" appearance that defeated the earlier
row-major and column-major searches.

`$F718` and `$F728` are the camera row/column parameters (both set from the
resource record at `0x2DC0`/`0x2DC8`), and `$F70E` is the index-array row
stride.

### Also in this area

* `0x2E42` indexes a second ROM pointer table at **`0x4C828`** by
  `($F0B0 - 1) * 4`, then again by `$F734` — actor/object definition selection.
* `0x2EB0` special-cases resources `9` and `0x10` (boss/ending variants),
  consulting flags at `$FE19`/`$FE1A` and loading from ROM `0x6B4A0`.
* `0x2E18`-`0x2E36` builds a table with an **80-byte (`0x50`) stride**, copying
  four longs per entry and duplicating each entry 80 bytes ahead, over 14 rows.
  That is the **hardware sprite attribute list** — so sprites *are* used, built
  from `0xFFBA00`. This resolves the open question from §15: sprites exist, but
  the sprite table is assembled in WRAM and only later pushed to VRAM.

---

## 22. Index array located; end-to-end check partly passes

Reading the captured runtime parameters during gameplay:

| Variable | Value | Meaning |
|---|---|---|
| `$F702` | `0x00035B70` (longword) | cell-table base — ROM `0x35B70` |
| `$F70E` | `96` | index-array row width in bytes |
| `$F718` | `0` | camera row |
| `$F728` | `0` | camera column |
| `$F724` | `0` idle / `0x0000FFC3` walked | secondary scroll state |
| `$F70A` | `0x0000FB40` | tertiary cursor |

**The staging buffer at `$FF5000` is byte-identical to the VRAM Plane A name
table** — all 16 rows, 128 bytes each. That is the 6 KB "game state block" from
§12: rows 0–15 are the name-table mirror, the rest is other state. It is the
output target of the `0x36C0` expansion, which is what makes the whole pipeline
observable.

### The map format — SOLVED AND VERIFIED

The real routine is **`0x3668`**, reached by `BSR.W` at `0x2D70` — *not* the
near-identical-looking `0x36C0`, which is a second variant called from `0x2E88`
and which never runs in this stage. (See "Two errors I made" below: mdx reports
word-branch targets 2 bytes low, so `BSR` at `0x2D70` — which mdx prints as
`0x3666` — actually enters at `0x3668`, and I initially decoded the *wrong*
routine as a result.)

```
0x366A  MOVE.W  $F70E,D1            ; 96 - the map row width
0x366E  NEG.W   D0
0x3670  ASR.W   #4,D0               ; d0 = -(row / 16)
0x3672  LEA.L   0(A2,D0.W),A2       ; index array + row offset
0x3676  ANDI.W  #$1F,D0
0x367A  MULS.W  #4,D0               ; column phase: (col & 31) * 4 bytes
0x367E  LEA.L   128(A1),A5          ; end of this 128-byte output row
0x3682  LEA.L   0(A1,D0.W),A1
0x3686  SUBI.W  #$20,D1             ; 96 - 32 = 64
0x368A  MOVEQ   #$0D,D7             ; 14 rows
0x368C  MOVEQ   #$1F,D6             ; 32 cells per row
0x368E  MOVEA.L A3,A4               ; a4 = cell table  ($F702 = ROM 0x35B70)
0x3692  MOVE.B  (A2)+,D0            ; one index byte
0x3694  ASL.W   #3,D0               ; x 8
0x3696  LEA.L   0(A4,D0.W),A4
0x369A  MOVE.L  (A4)+,(A1)+         ; 4 name-table bytes
0x369C  MOVE.L  (A4),124(A1)        ; the same 4 again, 31 words later
0x36A0  CMPA.L  A5,A1
0x36A2  BCS.B   0x36AA
0x36A4  SUBA.L  #$80,A1             ; wrap: back up 128 bytes
0x36AA  DBRA.W  D6,0x368E
0x36AE  LEA.L   256(A1),A1          ; NEXT OUTPUT ROW: +256, not +128
0x36B2  LEA.L   128(A1),A5
0x36B6  LEA.L   0(A2,D1.W),A2       ; index advance += 64
0x36BA  DBRA.W  D7,0x368C           ; 14 rows
```

**The whole level format:**

```
index[R][C]  = wram[0xFF0000 + 96*R + C]        # one byte per cell
cell         = rom[0x35B70 + 8 * index]          # 8-byte records; first 4 used
output       = wram[0xFF5000 + 256*R + C*4]      # 4 bytes per cell
              plus a copy at +124 bytes (31 words), wrapping with SUBA.L #$80
```

### Verification

Expanding the captured WRAM through the ROM cell table and comparing against the
captured staging buffer:

```
448/448 cells byte-exact — every one of 14 rows x 32 cells
per row: [32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32]
```

This is the strongest result in the document: the map format is not inferred,
it reproduces the live name table exactly from a single index byte per cell.

### Two errors I made, and what they teach

1. **Output row stride is 256 bytes, not 128.** I initially read the
   near-identical `0x36C0` variant, which uses `LEA.L 128(A1),A1` and 16 rows.
   Using that model against the real 256-byte layout produced the "rows 5-15
   fail" pattern that I spent a long stretch trying to explain. The corruption
   was in my model, not in the ROM.
2. **mdx's word-branch targets are 2 bytes low** (known finding #2). `BSR.W` at
   `0x2D70` is reported as `0x3666` but enters at `0x3668`. Decoding from the
   reported address walked 2 bytes into the previous instruction and landed in
   the *wrong* function. Rule for this project: **correct every reported
   `bsr.w` / `bra.w` target by +2 before decoding anything from it.**

Finding the right routine took tracing: `mdx_trace` with `start_pc: 0x2D0E` and
`start_frame: 474` captured the whole stage load (17,040 records). A PC histogram
of that artifact showed the hot loop at `0x368E`–`0x36AA` and **zero** hits at
`0x36C2`, which is what identified `0x3668` as the live path.

**This needs runtime observation of `A1`/`A2` across `BSR 0x36C0`, which
`mdx_watch` cannot currently provide** — it folds every range through
`fold_to_wram`, so neither registers nor VRAM are observable. Adding a
`watch_registers` option to `mdx_trace` would settle it.

---

## 23. Tooling notes learned the hard way

**Always correct `bsr.w` / `bra.w` targets by +2 before decoding from them.**
mdx reports word-branch targets 2 bytes low (finding #2). Decoding from a
reported target can walk into the middle of a neighbouring instruction and land
you in a completely different — but plausible-looking — function. `0x36C0` vs
`0x3668` is exactly that trap: two structurally similar map builders, one live
and one dead, differing by 2 bytes.

**Use a PC histogram to find the live routine before decoding it.**
`mdx_trace(start_pc=…, start_frame=…)` captures a whole window to a JSONL
artifact. Histogramming the PCs is cheap and immediately tells you which routine
actually executes:

```
mdx_trace(start_pc=0x2D0E, start_frame=474, frames=477, max_records=30000)
  -> 17,040 records, 90 distinct PCs
     0xA52E/0xA530/0xA534  2928 each   <- decompressor inner loop
     0x368E..0x36AA         ~314 each  <- the live map builder
     0x36C2                  0 hits     <- the decoy variant, never executed
```

**Watch hex conversions.** I briefly believed `JSR $A504` had an off-by-16 bug
because `next_pc` was 42244 and I read that as `0xA4F4`. 42244 *is* 0xA504.
Verify before reporting a bug — a false finding costs more than a missed one.
