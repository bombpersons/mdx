# Finite cartridge fidelity checks

The game-independent engine is tested with authored fixtures in CI. External
cartridges and independent Genesis Plus GX pixel captures are kept in the
research workspace, not distributed with this toolkit. These are finite
behavioural checks, not claims of whole-game or cycle equivalence.

## Cadash (USA, Korea)

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

### Residual timing limits

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

## Earthworm Jim and Hokuto no Ken

Fresh replay checks reached gameplay at frame 6000 (Earthworm Jim) and 2800
(Hokuto), with zero reported CPU exceptions. Pixel fidelity is partial:

| Cartridge / checkpoint | Changed pixels (320×224) |
| --- | ---: |
| Earthworm Jim title, frame 1200 | 3422 |
| Earthworm Jim difficulty selection, frame 2800 | 2296 |
| Earthworm Jim first level, frame 6000 | 1582 |
| Hokuto timed intro, frame 1200 | 5782 |
| Hokuto first level after Start/action replay, frame 2800 | 17 |

These observations establish execution progress, not pixel-perfect or whole-game
support. The residuals require their own consumed-state investigations.

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
