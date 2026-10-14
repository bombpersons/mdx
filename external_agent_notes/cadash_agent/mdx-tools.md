# mdx Tool — Bugs & Feature Requests

Running log of findings while reverse-engineering **Cadash (USA, Korea)** (sha256 `85e07824…`) with `mdx`.
This is the doc the mdx agent will use — evidence, not speculation. Last updated: after input investigation.

> Two gotchas that cost time:
> **`frames` is an absolute target, not a duration.** `boot(frames=N)` stops *at* frame N.
> **`input_events_pending` is not an error** — those events are simply scheduled past the stop frame.
> And every press needs an explicit `{"frame": K, "buttons": []}` release event, otherwise it latches.

---

## BUG 1 — controller replay never reaches Cadash's pad reads (HIGH)

**Symptom.** Cadash sits on the title screen forever regardless of what the replay does. It never
sees a single button press.

**Root cause.** Cadash has **two different controller access methods**, and mdx implements only one:

| method | where | writes TH to | notes |
|---|---|---|---|
| **A** data-port control | `0x716` → `0x737` (the per-frame poller called by the main loop) | `0xA10003` / `0xA10005` — the *same* address it reads data from (`move.b #0x00,(a0)` … `move.b (a0),d0` … `move.b #0x40,(a0)` … `move.b (a0),d0`) | the one the game actually uses every frame |
| **B** separate control port | `0x700` (one-shot init) | `0xA10009` / `0xA1000B` / `0xA1000D` | only run once at boot |

mdx honours **method B** only. Writes to `0xA10003/0xA10005` do not set TH, so Cadash's two reads both
come back with the same TH-independent value, the button byte at `0xFF0208` (P1) / `0xFF0209` (P2)
never changes, and the game sees no input at all.

**Evidence.** With the unmodified ROM, the pad is never seen:

| run | replay | `0xFF0208` (P1 byte) |
|---|---|---|
| boot → frame 2200 | none | `0x0D` |
| boot → frame 2200 | start held 2100–2130 | `0x0D` |
| boot → frame 2200 | **start held frames 0–2199** | `0x0D` |
| watch 2090–2160, one write sample/frame | start/right/left/a/b in turn | `0x0D` every one of 70 samples |

**Confirmation that the replay itself is fine.** With `tools/patch_padctl.py` (routes Cadash's
`move.b #imm,(a0)` writes to `move.b #imm,6(a0)` = `0xA10009/0xA1000B`, and repairs the ROM
checksum at `0x18E`):

| run (patched ROM) | replay | `0xFF0208` |
|---|---|---|
| idle | none | `0x07` |
| idle, port 2 sampled | none | `0x09 = 0x07` |
| start held 0–2199 | start | `0x07` |

**What mdx's pad model actually returns** (same patch, plus a variant that returns the raw first
read instead of Cadash's mix):

| ROM / access | buttons held | byte |
|---|---|---|
| full mix (normal) | any / none | **always `0x07`** |
| raw first read | right | `0x7F` |
| raw first read | start | `0x7F` |
| raw first read | up | `0x7E` (bit 0 cleared) |
| raw first read | left | `0x7B` (bit 2 cleared) |

So mdx's TH=0 read is a **fixed `0x7F`** (independent of the pad), and the TH=1 read exposes only
*up, down, left* (`0x07` idle). Cadash's mixer ORs both, so it always lands on `0x07` — the same
value regardless of what the replay does. THAT is why Cadash never reacts to input under mdx.

**Why the game's own check can't work either.** Cadash explicitly compares the byte to `0x0D`:
```
0xAB6: cmpi.b #0x0D,0xFF0208   ; P1
0xABE: bne.w 0xBC8             ; -> pad-related work
0x6544: cmpi.b #0x0D,0xFF0209   ; P2
```
`0x0D` is what the real hardware produces when the pad read is stuck/latched; a healthy pad gives
`0x07`-ish values. With unpatched mdx the byte is `0x0D` (matches), and with the patched ROM it is
`0x07` — so on real hardware `0x07` and the game works; under mdx, `0x07` never arrives from a real
button state.

**Request — two options, either is enough for Cadash.**
1. Make the pad read TH-independent for 3-button pads: `read(port) = 0xC0 | ~buttons`, i.e.
   idle `0xFF`, up `0xFE`, down `0xFD`, left `0xFB`, right `0xF7`, A `0xBF`, B `0xDF`, C `0xEF`,
   start `0x7F`, mode `0xF7`-ish. Cadash's mixer squashes 8 bits into 6, so it would still see
   up/down/left from the TH=1 nibble — enough to move the cursor on the character-select screen.
2. Honour writes to `0xA10003/0xA10005/0xA10007` as TH control so the two reads differ (this is
   what option 1 makes pointless, but it also fixes 6-button-aware games).

**Reproduction.** `boot(rom=Cadash, frames=2200, input={"events":[{"frame":0,"port":0,"buttons":["start"]},{"frame":3000,"port":0,"buttons":[]}]})`
then read offset `0x208` of the `wram-*.bin` artifact → `0x0D`.

**Side note (not a bug, but required for ROM patching).** Cadash verifies its own checksum at boot:
sum of big-endian words from `0x20A` for `((long@0x1A4 + 1) - 0x20A) >> 1` words, masked to 16 bits,
compared with the word at `0x18E`. On mismatch it jumps to `0x846` and hangs in the loop at
`0x854`. Any patched ROM must recompute that word (`tools/patch_padctl.py` does it automatically).

---

## BUG 3 — the artifact store is global, rooted in whichever project launched mdx first

`mdx` writes every snapshot, memory buffer and graphics export to
`<dir mdx was launched from>/.mdx/artifacts/objects/<sha256>` and `.../.mdx/graphics/<prefix>/`.
It currently lives in `/home/maisy/opencode_projects/hokuto_reverse/.mdx/` — a *different* project
(Hokuto no Ken). While reverse-engineering Cadash, every artifact I produced landed in that other
project's directory.

**Proof of concurrent use** (I made no mdx call between 12:15:56 and 12:25):

```
12:20:12  1c785ed0… 203534 B   5f9f3530… 154691 B   97d0748a… 155326 B
12:20:12  018678bb… 182235 B   9aee742a… 773997 B   afdd1bc5… 131072 B
12:20:03  8203d77f…  32356 B   29ad8805… 109728 B   5a2de3ba…  65536 B
12:15:56  ad335d1f… 194772 B   3795d7a9… 220279 B   <- my mdx_graphics call
```

**Does it corrupt anything?** Not observed. The store is **content-addressed by sha256**, so two
different ROMs/states produce different object names; re-running the same input overwrites the same
name with identical bytes. My Cadash snapshots still decode correctly (checked the RAM dump against
the original frame PNG and the `subject` sha256 in each response).

**What is genuinely broken / risky:**
1. **Provenance is invisible.** `artifacts/objects/7cae09b4…` tells you nothing about which project,
   ROM or session wrote it; the only way to tell is to re-read the `subject` field of the response
   that referenced it. That cost me a round of verification when I found myself reading paths under
   `hokuto_reverse/`.
2. **Store rooting is an accident of launch order.** Two agents on two ROMs share one store; the
   first project to launch mdx owns it.
3. **Fragility.** If the owning project's directory is cleaned, moved, or its `.mdx/` pruned, every
   `sha256:` reference from the other project breaks.
4. **Unbounded growth.** 158 objects, 32 KB–774 KB each, no eviction visible, accumulated across both
   projects.

**Requests.** (a) A configurable store path (`MDX_STORE` env var or a CLI/config flag) so each
project owns its artifacts. (b) Include the producing project/ROM/session identity in the response
(`provenance: {rom, rom_sha256, session}`), or at minimum an `mdx_where` command that prints the
store root. (c) A way to list which artifacts belong to a given ROM.

---

## BUG 2 — `mdx_boot` on a ROM whose size changed by patching: silent path confusion

If a patched ROM is not exactly 524 288 bytes, boot reports `entry_region: wram`, the reset vector
decodes to `0xFF0200`, and the CPU immediately fault-loops (`illegal_instruction: 0x4741` at
`0x01000102`, ~40 M repeats) with `display_enabled: false`. Nothing in the first lines says
"your ROM is the wrong size" — it took a few minutes to notice the size had grown from an
off-by-two in my patch. A size/power-of-two check (the tool clearly knows — `header` reports
`is_power_of_two`) on `boot` would be a cheap guard.

---

## Feature requests

1. **Disassembly entry-point list / alignment control.** Seed a listing at function starts
   (vectors `0x200-0x3FF`, reset `0x20A`, the mode jump table at `0x3C06`) so linear disassembly
   doesn't walk into data.
2. **Follow a branch.** Let `mdx_disasm` follow `bra/jmp/jsr` targets so a real CFG exists instead of a
   flat listing.
3. **`stopAtInvalid` flag** to end a listing at the first undecodable opcode. (`4E 66` at `0x23E` is
   data inside the boot block; the tool correctly reports `??` and advances two bytes.)
4. **Repeat screenshots without a full re-run.** `boot` captures one image at the end; a
   `frame_capture` option (every K frames, or a list) would make menu-flow tracing much cheaper.
5. **Warn when a `watch` address lands in a mirrored alias.** A watch on `0xFF0200` folded to offset
   `0x0200` and returned zero records with no hint the address was wrong.

## VDP register decode (verified against `graphics`)

`vdp.registers[]` in `boot` output is `[reg0, reg1, … reg23]`:
`[2]=48, [3]=60, [4]=7, [5]=108, [7]=32, [12]=129, [13]=55` →
plane A `0xC000`, window `0xF000`, plane B `0xE000`, SAT `0xD800`, bg colour 32, H32 mode,
hscroll `0xDC00`. This agrees with `mdx_graphics`' `planes` output, so the decode is right.
