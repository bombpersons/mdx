# Finite cartridge fidelity checks

The game-independent engine is tested with authored fixtures in CI. External
cartridges and independent Genesis Plus GX pixel captures are kept in the
research workspace, not distributed with this toolkit. These are finite
behavioural checks, not claims of whole-game or cycle equivalence.

## Cadash (USA, Korea)

**2026-10-13 update:** the formerly phase-shifted unattended title600 now matches
exactly, alongside all five original gates and an extended7000 replay. Shared
reset/refresh/bit/FIFO/VINT scheduling corrections are described below. The
older residual discussion is retained as investigation history.

ROM SHA-256: `85e0782411cf50ea92a9982dac1942679f738d46a58fdda5d7d82b943512afd8`.

The late-boot vertical-bars failure had an instruction-aligned cause:
`CMPA.L A1,A3` at `$2F20` matched the broader CMPM dispatch mask and incorrectly
post-incremented both address registers. Signed `DIVS` at `$8B6C` then exposed
a separate impossible opcode-mask comparison when entering gameplay. Both
families have synthetic, register-independent regression cases.

Internal DMA fill/copy now exposes a timed busy interval while leaving the CPU
running. It is separate from external DMA's CPU-bus wait. Data effects remain
instruction-boundary approximations, not sub-instruction FIFO simulation.

Frame replay/capture boundaries now follow completed visible frames at the
VBLANK edge. Reset begins after VBLANK rises, with no synthetic initial VINT;
scanline-zero wrap is not a completed-frame boundary. This removes the measured
one-frame input-phase/camera discrepancy. NTSC VBLANK lasts through line 260.

Finite external checks exercise Start, player selection, name entry, castle
dialogue, rightward movement, attack/jump inputs and extended leftward camera
scrolling. Exact image checks are maintained in the research project's
`cadash_boot` test target using the standalone binary.

### Historical timing investigation

The unattended title wave is still shifted in phase at frame 600; the measured
snapshot matches a nearby oracle frame rather than the requested frame. This
is not included among the exact pixel gates. Likewise CPU conformance remains
partial: signed-division exception/undefined-flag cases are not generally
equivalent to the full SingleStepTests corpus.

A reference-derived bit-operation timing experiment was not integrated: it
regressed the measured camera checkpoint. An explicit ignored timing probe
retains the independently known cycle expectations for a wider scheduling
investigation. Neither the oracle nor expected pixel captures were modified to
agree with the engine.

## King's Bounty

Seven existing oracle gates cover EA logo, title, character selection,
difficulty selection, overworld entry and two directional replays. They remain
the cross-cartridge regression checks in the research workspace.

Credits1800 and extended-world6000 now match the independent oracle exactly.
Four-entry write-FIFO backpressure and periodic CPU refresh waits close the
credits gap; delayed VINT preserves the original difficulty-cursor gate.

## Earthworm Jim and Hokuto no Ken — updated 2026-10-13

Fresh checks and extended replays have zero changed pixels and CPU exceptions:

| Cartridge / checkpoint | Changed pixels (320×224) |
| --- | ---: |
| Earthworm Jim title, frame 1200 | 0 |
| Earthworm Jim difficulty selection, frame 2800 | 0 |
| Earthworm Jim first level, frame 6000 | 0 |
| Earthworm Jim extended replay, frames 7000 / 8000 | 0 / 0 |
| Hokuto splash/story/intro, frames 60 / 300 / 1200 | 0 / 0 / 0 |
| Hokuto first level after Start/action replay, frame 2800 | 0 |
| Hokuto extended replay, frames 4000 / 6000 | 0 / 0 |

The research workspace freezes17 survey references and preserves12 original
gates, representing27 distinct frame/input checkpoints. At all17 surveyed
checkpoints VRAM/CRAM/VSRAM and VDP registers also match; non-video WRAM deltas
remain in some snapshots. These are exact finite image gates, not whole-game,
audio or cycle equivalence.

### Shared timing mechanisms

- Bit-operation costs distinguish immediate/dynamic, register/memory, modifier
  and effective-address costs. The title's absolute-long static BTST costs20
  CPU clocks, not8. The formerly ignored reference test is now enabled.
- A hardware-derived NTSC hard reset releases the CPU at master784462 relative
  to the initial192-line Mode4 frame origin. Frame geometry latches at the
  completed-frame boundary; replay frame numbers/inputs stay unchanged.
- Console refresh waits cost14 master clocks at dispatch deadlines896 clocks
  apart. Device clocks advance during the waits.
- A four-word write FIFO uses independent H32/H40 slot tables, two slots per
  VRAM word and one per CRAM/VSRAM word, with live empty/full status and an
  explicit `fifo_stall_master_cycles` diagnostic separate from external DMA.
- VINT rises770/788 master clocks after VBLANK in H32/H40. Frame completion
  and replay application remain on the VBLANK edge.

FIFO data updates and accesses remain instruction-granular. Long-instruction
refresh suppression and exact intra-instruction bus timestamps are not claimed.
No original oracle execution, old reference PNG or old replay timing was changed.

## Oracle adapter integrity

The old Python adapter declared `retro_system_av_info` as 28 bytes. The public
libretro ABI requires geometry plus two doubles (40 bytes on x64), so the native
core wrote beyond the Python buffer. This caused the SIGBUS previously attributed
to ROM filenames. `libretro_abi.py` now declares the actual structures and is
covered by an execution-free layout test. Original long/spaced ROM filenames now
load normally. `GET_VARIABLE` also writes the value pointer and retains its
buffer rather than overwriting the key pointer.

All five Cadash references were re-created with this corrected adapter and had
zero changed pixels against their frozen captures. Oracle execution code was
not altered.
