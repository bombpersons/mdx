# Investigation log

## 2026-10-11: resume at `2199521`

**Objective:** fix the boot divergence before starting the native game.

**Baseline:** `cargo test --workspace --release` passes (98 passed, 8 ignored).
The existing modified `caller_probe` runs successfully, but it is a diagnostic
that prints state rather than asserting hardware correctness.

Initial caller output captured with:

```sh
cargo test -p mdcore --release caller_probe -- --ignored --nocapture > /tmp/reverse-kings-caller-baseline.txt 2>&1
```

CPU suite is at `cache/680x0/tests/68000/v1/`. The CLI expects its parent:

```sh
target/release/mdx cpu conformance --suite cache/680x0/tests/68000 --filter MOVE --failures 3 > /tmp/reverse-kings-move-baseline.json
```

The conformance harness currently rewrites fixture memory by default and does
not compare PC/prefetch. Its interpretation must be checked against the corpus
before relying on a pass/fail verdict. In particular, the comment claiming that
the suite's prefetch model is unlike hardware is not independently established.

### Confirmed CPU/harness corrections

- GPGX `core/m68k/m68kops.h:7334`, handler `m68k_op_dbf_16`, explicitly masks
  the decrement to 16 bits and preserves the upper word. The matching dispatch
  record is `{m68k_op_dbf_16, 0xfff8, 0x51c8, 12}`. Corrected mdcore's word
  counter and all sixteen DBcc conditions; changed the old tests that asserted
  a nonexistent DBRA.L. Existing tests pass after this correction.
- The SingleStepTests README says PC points at the opcode and prefetch holds
  the two already-fetched words. The old default relocation overwrote the
  second prefetched word. Canonical mode now preserves inputs and compares
  PC and prefetch, with an explicit limitation that cycles are not compared.
- The sparse conformance bus now masks external addresses to 24 bits; random
  fixtures preserve full 32-bit address registers but declare physical RAM.
- `docs/evidence/dbcc-after-width-fix.json`: 6,101 / 8,065 canonical DBcc cases
  pass; first failures require address-error exception handling (not yet
  implemented). Do not call this full conformance.
- `docs/evidence/boot-after-dbcc.json`: at 3M instructions the ROM still has
  zero CRAM writes and is in the same fill path. The DBcc fix is necessary but
  not sufficient; it does not establish the cause of the remaining failure.

### Next experiment: instruction-aligned oracle state

Added `tools/build_trace_oracle.py` and `tools/oracle_trace.inc`: they make a
private copy of the existing GPGX build and insert an observation-only hook.
The original oracle source/core remain unchanged. `MDX_ORACLE_CORE` selects
the private core in the existing driver. `MDX_ORACLE_TRACE` selects a compact
pre-instruction CPU state file and `MDX_ORACLE_TRACE_LIMIT` bounds its size.

`crates/mdcore/examples/state_trace.rs` produces the same layout: eighteen
little-endian u32 words (PC, SR, D0..D7, A0..A7) per pre-instruction event.
This should identify the first differing instruction instead of extrapolating
from a later loop's final state. Device-read timing may differ even when the
CPU semantics agree; classify such differences explicitly.

### Earliest shipped-ROM differences, now observed directly

- Oracle controlled replay is retained in `docs/evidence/rea-oracle.json`
  (`ev_216d479c03d42aaf387ca8d50bdb2314acfe83772ca87fdb01891331b6ef557c`).
  Private core provenance is in `cache/oracle/trace-build/trace-manifest.json`.
- `docs/evidence/first-divergence-lea.json`: excluding only reset record 0
  (CCR differs before the first instruction), first difference is record 5.
  At `$210`, PC-relative LEA produces A5=$290 in mdcore vs $28E in GPGX.
  Fix: base displacement at the extension word, not after consuming it.
- `docs/evidence/first-divergence-after-lea.json`: next difference is record 6,
  following MOVEM.W at `$214`: D5=$00008000 vs $FFFF8000. Word MOVEM loads
  require sign extension. Reference handler bodies also confirm mask reversal
  for predecrement and postincrement overriding a listed base register.
- These observations overturn the historical claim that the core was already
  essentially correct. Boot-table pointers differed almost immediately, long
  before the final fill loop. Do not infer correctness from a sparse RAM diff.

Reproduction:

```sh
python3 tools/build_trace_oracle.py  # fresh output only; then reuse built core
MDX_ORACLE_CORE="$PWD/cache/oracle/trace-build/genesis_plus_gx_libretro.so" MDX_ORACLE_TRACE=/tmp/gpgx-initial.bin MDX_ORACLE_TRACE_LIMIT=1000000 python3 tools/oracle_gpgx.py rom/original/kings_bounty.md --frames 100 --out /tmp/oracle-frame100.png
cargo run --release -p mdcore --example state_trace -- rom/original/kings_bounty.md 1000000 /tmp/mdcore-current.bin
python3 tools/compare_cpu_traces.py /tmp/mdcore-current.bin /tmp/gpgx-initial.bin --start-index 1
```

Trace state is pre-instruction; a mismatch at record N was produced by the
instruction at record N-1. Small difference reports include hashes and preceding
states; the full 72 MB traces are regenerable and currently live in `/tmp/`.

### Further exact differences and corrections

- Record 11: an initial VDP status read differs ($3400 vs $7208); the next
  MOVEQ overwrites D0. This is a device/status difference, not a CPU result.
  Subsequent comparison starts at record 12 and records that excluded prefix.
- After masking narrow arithmetic operands before calculating carry/borrow,
  the trace agrees from records 12 through 115. At `$24A`, dynamic BTST was
  misclassified as an immediate operation. Corrected bit operation selection,
  opcode-based operation field, and register-vs-memory bit-number modulo.
- The trace then agrees through record 33,085. At `$28A`, MOVE EA,SR was
  implemented as MOVE SR,EA. Corrected the `40C0`/`44C0`/`46C0` directions.
- Next difference (record 33,093) is CMP changing X; comparisons preserve X.
  Also corrected BSR.B's stacked return address (it incorrectly added one).
- GPGX handlers and canonical fixtures confirm ordinary mode 000 is Dn for
  all registers, mode 7 / reg 2 is PC displacement, reg 3 PC index, reg 4
  immediate. Indexed word values sign-extend; W/L is extension bit 11 and
  the 68000 does not scale indexes. Byte stack accesses advance by two.
- Two synthetic MOVEM round-trip tests used the same mask for predecrement
  and postincrement; corrected the save masks against the reference. The old
  default fill test asserted a seed caused by the incorrect core, so it is
  retained as an ignored historical diagnostic rather than a fidelity gate.
- The 12 frozen, unchanged SingleStepTests LEA/MOVEM/DBcc cases pass. These
  supplement, rather than merely mirror, the corrected synthetic tests.

**Still in progress:** subsequent CPU differences, then VDP correctness and
timing. No first-image success has been claimed.

### Exact agreement through the checksum and RAM clear

`docs/evidence/first-divergence-after-quick.json` locates the next difference at
record **693,772**, during VDP initialization. All CPU state matches from record
12 through 693,771 (660k+ consecutive instructions). The final checksum branch
therefore follows the unmodified ROM's genuine path; no branch override is
needed in these runs. Quick address arithmetic had incorrectly changed flags
at `$7FA3C`; fixing it exposed this much longer matching prefix.

MOVE.B and MOVEQ now pass all 8,065 canonical cases each (including exact
PC/prefetch); see `docs/evidence/move-suite-current.json`. Other families remain
partial: word/long address-error exceptions, privilege/stack transitions, and
MOVEP are among the missing cases. Avoid advertising complete CPU conformance.

### Instrument correction: GPGX has two execution dispatch sites

The apparent skipped instruction at record 693,772 was a missing oracle event,
not a CPU defect. `m68k_set_irq_delay` executes one instruction inline while
enabling VDP interrupts, in addition to the normal `m68k_run` loop. The first
hook only covered the latter. The builder now instruments both exact dispatch
sites and fails if their expected count changes. V2 is built separately in
`cache/oracle/trace-build-v2/`; the earlier build/trace reports remain available
and are identified by their hashes. Use `/tmp/gpgx-v2.bin` for further alignment.

This preserves the 660k-instruction matching prefix but withdraws the claim that
record 693,772 identifies a genuine CPU/control-flow difference.

### Fill and interrupt frontier reached

- V2 trace's next actual CPU difference is record 729,455 at `$27F3E`: EXT.W
  had been interpreted as NBCD (and as MOVEM by the disassembler). Corrected
  EXT.W/EXT.L sign extension and excluded register-direct operands from MOVEM.
- Next difference is a zero-count shift's N flag at `$27CE2`. Zero shifts still
  update NZVC; rotate-through-X copies X to C at zero count, plain rotates
  preserve X. Corrected these and word-only memory shifts.
- Exact CPU agreement then reaches record **765,119**, where the game lowers
  SR's interrupt mask. The reference enters its VINT handler at `$2A060`.
- Corrected the six-byte interrupt frame against GPGX's stack_frame_3word:
  SR at SP-6, next PC at SP-4; entry sets supervisor and the serviced priority,
  clears trace, and wakes STOP. RTE now restores SR with stack-mode transitions.
- The compact mdcore tracer now excludes standalone interrupt-entry steps, so
  its records have the same instruction-only meaning as the oracle hook.

### VDP corrections (historical VDP claims were also wrong)

Reference `vdp_ctrl.c` tests display enable with **reg1 & $40**, DMA enable with
reg1 & $10, and VINT enable with reg1 & $20. Interlace is in reg12. The old
notes asserting display bit4 and interlace in reg1 are false.

The old port implementation did not latch the two-word VDP command and used
the wrong data destinations. Correct codes are 1=VRAM write, 3=CRAM write,
5=VSRAM write; second word supplies high address/code bits. Corrected command
latching, auto-increment, address wrapping, destination counters, and status
reads. A port test now writes nonzero sentinels to each distinct target and
checks high address bits (the former test only checked an already-zero palette).

Added a proper packed-4bpp mode-5 snapshot renderer with corrected table bases,
tile attributes, scrolling, window selection, linked sprites, and priority.
The former renderer's 2bpp tile interpretation and table layouts were unrelated
to Mega Drive mode 5. Unsupported DMA, interlace, and shadow/highlight remain
reported; the old private renderer helpers are pending removal after validation.

The oracle PNG writer also misread libretro pixel format **2 (RGB565)** as a
32-bit format, distorting the old reference image. Fixed RGB565/1555/XRGB8888
decoding and generated **`testdata/oracle_kb_frame400_rgb.png`**. Preserve the
old image as historical evidence, but use the new one for comparisons.

## 2026-10-12: title-screen fidelity and independent snapshot rendering

The resume baseline passed 101 normal tests (94 mdcore, 7 mdx/integration),
but a later frame-1200 capture was corrupted at 71,089 of 71,680 pixels.
Zero CPU exceptions did not imply that the title loader was working.

### Confirmed corrections

- **PEA control EAs:** the implementation always pushed An and consumed no
  extensions. The ROM's `$4878 $0300` and `$4878 $5000` arguments at `$18AE0`
  were therefore wrong and the following words were executed as opcodes.
  GPGX's mask/pattern records and PEA handler bodies establish all seven control
  EA forms. All 8,065 canonical PEA cases now pass, including total cycles.
- **EXG:** `$C141` in the palette fade at `$13644` / `$13688` was falling through
  to AND, discarding red/blue channels. Correct masks are `$F1F8` matching
  `$C140` (Dn/Dn), `$C148` (An/An), `$C188` (Dn/An). All 8,065 canonical cases
  now pass, including flags, prefetch, stack-register exchanges and total cycles.
- **EOR / CMPM:** corrected line-B dispatch, CMPM operand order, sequential
  postincrement and A7 byte stride. All 8,065 EOR.B cases pass including cycles;
  word/long suites still include unsupported address-error cases.
- **MULU / MULS:** signedness is bit 8, NZVC must clear V/C, and cycle count
  depends on source bit population/transitions. Full-suite passes rose from
  1,497 to 9,935 / 16,130; remaining address-error cases are not conformance.
- **Clocking:** NTSC lines contain 3420 master clocks and the 68000 divisor is
  **7**, not 4. One frame is ~128,005.7 CPU clocks. The normalized 896-dot line
  coordinate now advances with the exact rational ratio and retains remainders
  across instructions. Corrected EA costs in MOVE, ALU, quick/immediate forms,
  MOVEM, LEA, memory shifts, and non-taken word branches. MOVE.B passes all
  8,065 canonical cases including total cycles. DMA/FIFO scheduling is still
  approximate; this is not a general cycle-accuracy claim.
- Disassembler now distinguishes PEA/SWAP, EXG, proper LEA masks, JSR, RTE,
  OR/AND/EOR, MUL and address arithmetic without losing loader boundaries.

Twenty-two unchanged frozen SingleStepTests cases are checked in under
`crates/mdx/testdata/`, with original source-file SHA-256s. The regression test
now also compares their measured cycle totals. `cpu conformance --compare-cycles`
exposes that check to agents; bus ordering remains explicitly unverified.

### Finite visual gates

Both **frame 400 (EA logo)** and **frame 1200 (full title)** match all 71,680 RGB
pixels of the independent RGB565-decoded captures. The live gates pass:

```sh
cargo test -p mdx --release --test kings_bounty_boot -- --ignored
```

The title reference is `testdata/oracle_kb_frame1200_rgb.png`. The repaired
capture is `.mdx/frame1200-exg/frame-1200.png`; its report is
`docs/evidence/kb-frame1200-exg.json`.

At frame 1800, VRAM, CRAM and all 32 VDP registers are **byte-identical** to
the independently normalized oracle state. Only one VSRAM byte differs:
plane-B vertical scroll is 89 vs 76. This produces 4,627 differing text pixels.
Rendering the **oracle's own** frame-1800 buffers with mdcore matches all pixels,
so this difference is upstream scheduling rather than the snapshot renderer.
The scroll discrepancy remains open; no credits-frame fidelity is claimed.

### Reusable investigation tools

- `mdx render --snapshot REPORT.json --out DIR` verifies memory artifact sizes
  and SHA-256s, then renders a boot-data/snapshot JSON without executing CPUs.
- `tools/oracle_vdp_snapshot.py STATE ABSENT_DIR` converts the pinned GPGX
  1.7.6 little-endian MD state layout into canonical buffers and a hashed manifest.
  `oracle_gpgx.py --state PATH` now actually writes its state bytes.
- `state_trace` accepts an optional exact start PC; the private oracle hook
  supports `MDX_ORACLE_TRACE_START_PC`. V3 trace core provenance is in
  `docs/evidence/oracle-trace-v3-build.json`. The graphics decompressor has a
  matching exact prefix of 2,897 instructions before interrupt timing differs.
- `watch_ram` observes a folded RAM range at instruction boundaries, retaining
  the latest 64 changes and reporting discarded counts. It is explicitly a
  net-change observer, **not** a bus-write tracer. It identified `$1369E` as the
  palette entry writer and allowed direct examination of its consuming registers.

New initial traces still diverge at record 768,522 on the next VINT. Reference
DMA rates are in `core/vdp_ctrl.c:664-801`: external bus DMA stalls the 68000,
VRAM costs two access slots per word, and rates depend on blanking/H32/H40.
Our immediate DMA currently models transfer data, not those stalls.

### Controlled controller replay: into the overworld

Added active-low three-button pad state for both ports, respecting the TH
selection phase and output-direction pins. `mdx boot --input FILE` consumes
ordered JSON state events indexed by **completed frames, starting at zero**.
Each event replaces the complete pressed-button state for one port; `[]`
releases it. Ambiguous same-frame/port events, invalid ports, and unknown buttons
are rejected. The boot report retains the input file SHA-256, full event list,
applied/pending counts and final masks.

The independent oracle driver now has correctly typed polling/state callbacks
and consumes the same replay format. Its Genesis mappings were checked against
`libretro.c:722-731`: libretro Y→A, B→B, A→C, Start→Start.

New independently verified checkpoints, all **zero differing RGB pixels and
zero CPU exceptions**:

| Frame | Screen | Input fixture |
| ---: | --- | --- |
| 3600 | Sir Crimsaun / character selection | `kb_character_replay.json` |
| 3900 | Difficulty selection | `kb_new_game_replay.json` |
| 4300 | Initial overworld, 600 days remaining | `kb_world_replay.json` |

The five live oracle gates now include these three plus the EA logo and title.
All pass. Copies of the actual mdcore renders are in `docs/screenshots/`.

The next replay, `testdata/kb_move_replay.json`, presses Right then Down in the
overworld. The frame-4500 camera/terrain/UI match, but **49 flag-animation pixels**
differ, confined to x=116..129, y=105..112. This is a recorded unresolved animation
phase difference, not an exact movement fidelity gate. The reports and enlarged
flag crops are in `docs/evidence/`; no tolerance was added to the exact tests.

Independent controlled replay of character selection was captured through REA
as `ev_5c31a651285c845ce8ea43cd35b02393e4cb8d645929b842b2f1701332029090`.
The process capture records the input identity and created screenshot digest;
it does not itself establish pixel equivalence. The exported bundle is
`docs/evidence/controlled-replay-bundle.json`.

Final normal release suite: **104 passed, 14 ignored, zero failed** (99 mdcore
including port integration tests, four mdx unit tests, one snapshot-integrity
integration test). The five visual gates
pass when explicitly selected with `--ignored`. Snapshot-integrity tests reject
wrong digest, wrong byte count, and wrong register count before writing output.

## 2026-10-12: external DMA waits, exact walking, native-ready graphics

Reproduced the 49-pixel flag difference at frame 4500. The independent oracle's
normalized VRAM differed at 30 bytes in the rider pattern area; CRAM, VSRAM and
registers matched. Rendering the oracle buffers produced zero different pixels.
This isolates the discrepancy upstream of the snapshot renderer.

Added external-bus DMA stalls using unmodified GPGX `core/vdp_ctrl.c:664-801`:
16/18 access slots per active H32/H40 line, 166/204 during blanking; VRAM halves
the word rate, while blanked CRAM/VSRAM uses 161/198 for refresh-slot losses.
Waits cross scanline/blanking boundaries and advance device time in master clocks
before the next interrupt can be accepted. Boot reports now expose DMA requests,
transferred bytes and accumulated bus-wait master clocks. At frame 4500 the short
replay has 533 DMA requests, 450,903 transferred bytes and 30,943,936 wait clocks.
Data is still applied immediately, and FIFO/sub-instruction/fill/copy/sound
scheduling remains approximate; this is not a general cycle-accuracy result.

**Observed result:** short directional replay frame 4500 now matches all 71,680
oracle pixels, with zero exceptions. All five earlier exact gates continue to
pass. Credits frame 1800 remains different at 4,412 pixels: B-scroll is 87 vs
oracle 76, improved from 89 but still unresolved. The historical CPU-trace
divergence has not been rerun after this DMA correction.

**Replay correction:** the four-frame pulses in `kb_move_replay.json` did not
produce visible traversal. Added `kb_walk_replay.json` with 60-frame Right/Down
holds. Terrain visibly moves, and its frame-4500 image also matches every oracle
pixel. These are two separate tests with distinct output directories, bringing
the exact live gates to **seven**. References were captured with the independent
regular GPGX core, not generated from mdcore. The sustained oracle replay is
retained as `ev_bf4c19764d7fc4959b683f508ce2994622989cbb16058ca225a59394392cbd82`
in `docs/evidence/dma-walking-bundle.json`.

Added `mdx graphics --snapshot REPORT --out DIR`, sharing digest-verified snapshot
loading with `mdx render`. Exports every mode-5 pattern as unpacked indices, four
palette sheets, complete name tables and sprite metadata to hashed artifacts.
`mdx schema graphics --snapshot REPORT` exposes the response shape. Malformed
snapshots, unsupported interlace/mode and invalid plane dimensions fail before
creating output; new boundary tests verify nibble order, sprite tile order,
flips, palette/priority, full inventory sizes, hashes and repeat determinism.

Overworld mapping and native-ready rider:

- Window at `$D000`, A/B alias at `$E000`, sprites at `$CC00`.
- Rider = sprites 14/15, tiles `$328..$33F`, palette 0, x104/128 y104.
- `tools/kb_rider_assets.py` exports RGBA and verifies all **615 opaque pixels**
  against the independent walking reference, with zero differences.
- Extended `watch_ram` with validated replay input and a starting-frame filter.
  `$E54C` is the observed rider phase byte (0..3, ten-frame steps);
  `$14854` increments it and `$14864` wraps it. `$E54A` is its countdown.
  The phase probe retains all 25 changes, dropping none. The initially suspected
  `$E548` field is a separate animation counter, not a verified coordinate.
- Details and regeneration commands: `docs/overworld-map.md`.
- Inspectable images: `docs/screenshots/walking4500.png`, `movement4500.png`,
  and `rider4500.png`.

Next: full rider phases/source pointers and world-coordinate/terrain mapping
before the first native overworld slice. No native game crate exists yet.

Final verification: `cargo test --workspace --release` **107 passed, 16 ignored,
zero failed**. `cargo test -p mdx --release --test kings_bounty_boot -- --ignored`
**7 passed, zero failed**. Hashed before/after visual comparisons, including the
explicit unresolved credits result, are in `docs/evidence/dma-visual-comparison.json`.
