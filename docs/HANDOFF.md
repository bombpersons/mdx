> Current checkpoint: [ACTIVE_HANDOFF.md](ACTIVE_HANDOFF.md) and
> [timing-investigation-2026-10-13.md](timing-investigation-2026-10-13.md).
> The material below is historical. Toolkit development and validation now
> live here; native game research is in `../kings_bounty/`.

> Layout update (2026-10-13): this is now the mdx workspace. Toolkit source,
> tests, ROMs, oracle caches and development evidence are under this directory.
> Native game research is in `../kings_bounty/`. Old absolute paths and
> `../mdx` references below describe the original layout; use the current
> `target/release/mdx` and [reorganisation guide](reorganisation.md).

# Current handoff

**Active work:** read [`ACTIVE_HANDOFF.md`](ACTIVE_HANDOFF.md) first. It records
the latest-Bevy template/MDX generator, completed2774-archive ROM library, and
current live-RAM/HV/Z80 investigation after the previous finite-video milestone.

Updated: 2026-10-13. Start here; `notes/first-image-frontier.md` is historical
and contains many superseded conclusions.

## Goal and architecture

Build an agent-first, reusable Mega Drive reverse-engineering toolkit (`mdx`)
to support an idiomatic native Rust reconstruction of the Mega Drive King's
Bounty. The emulator is an analysis oracle, not the final game implementation.
The workspace currently contains `mdcore` and `mdx`; no game crate exists yet.

## Latest milestone: four-cartridge finite fidelity gates are exact

The user asked to review `external_agent_notes`, verify its claims, and run/fix
all available ROMs against the independent Genesis Plus GX oracle. Implemented
changes are in canonical **`../mdx`**, currently uncommitted, with the rebuilt
release binary at `../mdx/target/release/mdx`. The local historical engine was
not substituted for the canonical source. Restart an already-running MCP to
load the revised binary/API.

**27 distinct same-frame/input image checkpoints now match every pixel.** The
12 original gates pass unchanged; 17 additional captured checkpoints pass (two
overlap original gates). Fresh comparisons also show equal VRAM, CRAM, VSRAM
and VDP registers across the 17 surveys, with no CPU exceptions. Extended
replays reach Hokuto6000, Cadash7000, Earthworm Jim8000 and King's Bounty6000.

Closed former visual residuals: Hokuto intro1200/gameplay2800, Cadash title600,
Earthworm Jim title1200/menu2800/level6000, and King's Bounty credits1800.
Shared fixes: reference bit-operation costs, periodic CPU refresh waits,
hardware-derived hard-reset phase with initial Mode4 geometry, four-word VDP
FIFO backpressure/status, and master-precise delayed VINT. Verified sound-bus/FM
corrections and the preserved BCD-family work also pass regressions.

The notes' PC+4 branch correction, VDP register map, and cartridge read at
Hokuto `$78D8` were wrong. Absolute-short `$E01C` sign-extends into WRAM;
disassembly now makes that explicit. Accepted tooling changes cover vector
visibility, cartridge offsets, reached-stop coverage, RAM aliases, annotated
replay/schema discovery, and offline bundled conformance. Read
[`external-agent-review.md`](external-agent-review.md) before using those notes.

Evidence: [`evidence/fidelity-review.json`](evidence/fidelity-review.json),
[`screenshots/fidelity-review.png`](screenshots/fidelity-review.png), and
`testdata/oracle_fidelity/manifest.json`. Larger timing evidence is under
`.mdx/fidelity-review/{kb,cadash}-investigation/`; final fresh captures are under
`.mdx/fidelity-review/release-verified/` (all17 final-binary comparisons).

Verification: 156 canonical release Rust tests, 4 Python observer tests, all
12 original gates, and the new 17-case `cartridge_fidelity` gate. Run:

```sh
MDX_GOLDEN_BINARY=/home/maisy/opencode_projects/mdx/target/release/mdx \
  cargo test -p mdx --release --test kings_bounty_boot --test cadash_boot \
  --test cartridge_fidelity -- --ignored
.venv/bin/python tools/cartridge_survey.py --verify --out .mdx/NEW-DIRECTORY
.venv/bin/python tools/cartridge_survey.py --extended --verify --out .mdx/OTHER-NEW-DIRECTORY
```

These are finite video gates, not whole-game/audio/cycle equivalence. Non-video
WRAM differences persist in some KB/EWJ snapshots; do not silently classify
them as unused without tracing. Timing remains instruction-granular, including
FIFO data effects, intra-instruction port timestamps and long-instruction
refresh suppression. Sound synthesis is not implemented. Native game work has
not begun. Historical results below predate this milestone.

## Historical milestone: toolkit and MCP ready

### Cadash and four-cartridge checks (2026-10-12)

The canonical toolkit checkout `../mdx` now repairs the Cadash late-boot failure:
CMPA.L An,An no longer matches CMPM's post-increment handler, signed DIVS uses
the correct discriminator, internal fill/copy keeps a timed DMA-busy interval,
and completed frames/replay boundaries use the visible-end VBLANK edge. NTSC
blanking continues through line 260. Synthetic regressions cover these cases.
Published privately as commit **`e8e8dcb`** in `bombpersons/mdx`.
Its independent GitHub CI run also passed.

Verification: **127 release tests passed, 0 failed, 10 ignored** in `../mdx`;
**12 explicit pixel gates passed** here (7 King's Bounty + 5 Cadash). Four Python
observer tests pass. Cadash checks player selection1000, name entry1600, castle
dialogue2300, rightward/action replay4500 and leftward camera scrolling5500.
All are exact at the same requested frames and original replay inputs. A fresh
canonical MCP connection also matched the5500 image and queried its snapshot RAM.

`opencode.json` now launches **`../mdx/target/release/mdx`**, retaining this
workspace as the input/artifact root. **Quit/restart OpenCode** to use the new
binary in a session whose MCP process was already running. The old local crates
are a historical working snapshot; use `MDX_GOLDEN_BINARY` for King's Bounty
gates and the new `cadash_boot` gate's canonical-binary default.

The other two available cartridges were checked more deeply. Earthworm Jim
reaches its title, difficulty menu and first level with no CPU exceptions, but
the frame1200 title differs3422 pixels and frame6000 level differs1582. Hokuto
reaches its first level after the same Start/action replay and differs17 pixels
at frame2800; its timed intro differs5782 at1200. **Do not call all games fully
correct**. Cadash's unattended title wave still has a finite timing-phase gap.
A broader bit-operation timing experiment was set aside after regressing the
camera checkpoint; an explicit ignored reference probe records its expectations.

The historical SIGBUS attributed to long ROM filenames was an oracle-wrapper
ABI overrun: `retro_system_av_info` needs 40 bytes on x64, not 28. The new shared
`tools/libretro_abi.py` declarations and layout test fix this. Long filenames
now work. Environment variable callbacks now update `value`, not `key`, and
retain their buffer lifetime. All five Cadash references were recaptured with
the corrected adapter and matched the frozen references exactly.

Evidence: [`evidence/cartridge-coverage.json`](evidence/cartridge-coverage.json),
[`screenshots/cadash-gameplay5500.png`](screenshots/cadash-gameplay5500.png),
`../mdx/docs/verified-cartridges.md`. Source/binary/ROM/input/reference digests
and partial-result labels are recorded; no cycle/whole-game/audio claim is made.
All four cartridges can now be inspected and exercised, but only the listed
12 image gates prove exact fidelity. Remaining Earthworm Jim/Hokuto and Cadash
title timing work is recorded as an active follow-up; do not silently adjust
replay inputs or reference frame numbers to turn a partial result into a pass.

The reusable tool now also has its own **private** GitHub repository:
[`bombpersons/mdx`](https://github.com/bombpersons/mdx), local checkout
`/home/maisy/opencode_projects/mdx`, initial commit `972f247`. It builds and passes
120 Rust tests plus 3 Python observer tests without game ROMs or sibling-project
assets. Its default hardware/header tests use authored synthetic fixtures;
game-specific captures and pixel gates remain here. The standalone repository
is the toolkit source of truth; this game's existing workspace and MCP launch
configuration remain intact as a working snapshot.

The user's current priority was to finish the reusable toolkit and have its MCP
working before native implementation. That milestone is complete. Start with
[`mdx.md`](mdx.md) and the root README for the current interface; the older boot
investigation below is retained as historical evidence.

- 17 typed operations shared by CLI and the official Rust-SDK stdio MCP.
- Complete, portable input/output schemas; static discovery needs no ROM and
  has no filesystem side effects. Large results are immutable artifacts with
  JSON-Pointer/record-page queries.
- Content-addressed ROMs, memory, PNGs, snapshots and trace datasets; atomic
  publication and digest verification. Explicit snapshot IDs remain reusable
  after deleting their original named capture directory.
- Memory read/search/diff, all-width folded RAM watching, instruction tracing,
  per-stream omission counts, unhandled read/write diagnostics, batched surveys.
- Execution attempts count exceptions; cancellation, timeout, worker concurrency
  and disconnect lifecycle are tested through a real independent stdio client.
- `opencode.json` registers server `mdx`. `opencode mcp list` reports it connected
  without numeric-schema warnings. Reconnect/restart OpenCode to expose the new
  `mdx_*` tools in a session that started before registration.

Verification: **122 release tests passed, 0 failed, 16 ignored**; the seven
explicit King's Bounty image gates also pass. Three Python observer tests pass.
All 17 operations have CLI/MCP parity and schema-validation coverage. A real MCP
driver imports/inspects all four cartridges and exercises the walking replay,
memory query, graphics extraction and RAM writer capture. Its walking screenshot
has **0 changed pixels** against the independent reference and retains all
**25 phase writes**, none omitted.

Evidence: [`evidence/mdx-readiness.json`](evidence/mdx-readiness.json),
[`screenshots/mcp-walk4500.png`](screenshots/mcp-walk4500.png), and the larger
regenerable `.mdx/artifacts/mcp-smoke/report.json`.

Reviewed the parallel agent's Cadash work and adopted corrected/tested trace
start-count gating, conversion and bus diagnostics. See
[`other-agent-review.md`](other-agent-review.md). The other tree is preserved;
the proposed open-bus CPU change was not copied without extension-word/byte-read
tests. Cadash's reported late-boot fidelity discrepancy remains a follow-up,
not something proven fixed by our frame-1 smoke checks.

GPGX WRAM is host-word storage with compensated byte accesses, **not an oracle
CPU byte-order defect**. Our exporter and saved-state converter now normalize
it to 68000 byte order. Older raw WRAM dumps need pair swapping on little-endian
hosts. Oracle execution itself remains unchanged. Existing private trace builds
predate the corrected hook; rebuild a fresh private directory before using its
new count gate. mdx's `start_step` counts system attempts, not oracle dispatches.

Next: use the MCP for the native-overworld preparation described below, with the
existing asset/state map. Credits frame 1800 and the reported Cadash transition
remain finite fidelity gaps; no general cycle/whole-game/sound equivalence is
claimed. No native game crate has been started.

## Historical starting state for the boot investigation

- HEAD: `2199521` (disassembler MOVE operands / BRA mnemonic fixes).
- Existing user/session work: uncommitted `caller_probe` diagnostic in
  `crates/mdcore/src/cpu/exec.rs`. Preserve it while investigating.
- `cargo test --workspace --release`: 98 passed, 8 ignored, 0 failed.
- Independent oracle: `tools/oracle_gpgx.py`, backed by the locally built
  Genesis Plus GX core in `cache/oracle/`.
- Reference image: `testdata/oracle_kb_frame400.png`.
- Last historical lead: our boot takes the fill path near `$027F3E`; the oracle
  has a different scratch value consistent with the sibling path near
  `$027D7A`. The first divergence is not yet located. An end-state scratch
  value alone is not proof of the oracle's exact execution path.

## Boot and overworld investigation history

Baseline and reproducible oracle traces are established. The first non-reset
state difference was **instruction 5**, PC-relative LEA at `$210`, loading A5
two bytes too high. Fixing it exposed MOVEM.W's missing sign extension at
instruction 6. These are confirmed by full pre-instruction state in both cores,
not inferred from a later scratch marker.

Current milestone reaches the overworld via deterministic controller replay.
Independent frozen fixtures are in `crates/mdx/testdata/*-regressions.json`;
their original source-file hashes are included. Twenty-two frozen cases also
compare measured total cycles. **The first-image and title milestones pass exact
independent gates.**

Latest milestone: frames **400, 1200, 3600, 3900, 4300 and both frame-4500 replays**
have **zero exceptions** and the complete EA logo, title, character selection,
difficulty selection, initial overworld and sustained walking respectively.
All **71,680 RGB pixels match at each of these seven checkpoints**
against the independent oracle after applying
the reference's declared RGB565 colour profile. This proves that finite image,
not general cycle accuracy or full game correctness. Correct VDP command and
mode-5 rendering now replace the previous unrelated layouts.
Use `mdx boot --frames 400 --instructions 20000000`
for aligned captures; it writes hashed WRAM/VRAM/CRAM/VSRAM/Z80 snapshots and
the screenshot to the selected output directory.

**Reference correction:** use `testdata/oracle_kb_frame400_rgb.png` and
`testdata/oracle_kb_frame1200_rgb.png`. The earlier
PNG decoded RGB565 as 32-bit pixels and is not a fidelity reference.
Controlled-screen references are `testdata/oracle_kb_{character3600,difficulty3900,world4300,move4500,walk4500}_rgb.png`.

Live gate (passed):

```sh
cargo test -p mdx --release --test kings_bounty_boot -- --ignored
```

Current verification: normal release workspace suite **107 passed, 16 ignored,
zero failed**; all **seven** explicitly selected visual gates pass. Hashed
before/after comparisons are in `docs/evidence/dma-visual-comparison.json`.

At frame 1800 the independently normalized oracle's VRAM, CRAM and registers
match ours byte for byte; one VSRAM byte differs (B-scroll **87 vs 76** after DMA waits;
previously 89 vs 76). Rendering
the oracle's own buffers matches every credits-screen pixel. The moving-text
discrepancy is upstream scheduling. Before the DMA-wait correction, exact CPU
traces diverged on VINT at record 768,522; that trace has not been rerun afterward.
CPU/VDP clock ratio is the correct rational master/7;
External DMA now stalls the CPU using the reference H32/H40 active/blank rates,
including VRAM's two accesses per word and CRAM/VSRAM refresh-slot adjustment.
DMA data remains immediate; fill/copy timing, FIFO contention, sub-instruction
port timing, some instruction timing and sound scheduling remain approximate.
The credits screen still differs at 4,412 pixels; it is not an exact fidelity gate.

Use `mdx render --snapshot REPORT.json --out DIR` to isolate rendering from CPU
execution. `tools/oracle_vdp_snapshot.py` normalizes the pinned little-endian
GPGX 1.7.6 MD save state layout. Start-triggered traces and folded-RAM observers
live in `crates/mdcore/examples/`; their limitations are explicit.

Reproduce the overworld:

```sh
target/release/mdx boot --rom rom/original/kings_bounty.md \
  --frames 4300 --instructions 80000000 --trace 0 \
  --input testdata/kb_world_replay.json --out .mdx/world
```

`testdata/kb_move_replay.json` supplies four-frame Right/Down pulses. Its former
49-pixel flag discrepancy is fixed by external DMA bus waits; frame 4500 now
matches exactly. These short pulses do **not** establish traversal: the scene
stays at the starting position. `testdata/kb_walk_replay.json` holds Right/Down
for 60 frames each, visibly moves the terrain, and also matches exactly at 4500.
Screenshots are `docs/screenshots/movement4500.png` and `walking4500.png`.

New reusable command:

```sh
target/release/mdx graphics --snapshot docs/evidence/kb-walk4500.json \
  --out .mdx/graphics-walk4500
target/release/mdx schema graphics
```

Exports all 2,048 unpacked 8x8 patterns, four palette sheets, complete A/B/window
name tables and all 80 sprites, with hashed artifacts and explicit limitations.
The overworld's window table is at `$D000` (64x32); its current window settings
cover the entire 320x224 screen. Both A/B tables alias `$E000` (32x32), and the
sprite table is at `$CC00`. Rider sprites 14/15 are 24x32 each, palette 0, at
(104,104)/(128,104), with tiles `$328..$33F`. These are observed snapshot
identities, not original cartridge asset locations.

`tools/kb_rider_assets.py` produces a transparent rider PNG and verifies all
**615 opaque pixels** against the independent walking capture. Preview:
`docs/screenshots/rider4500.png`. `docs/overworld-map.md` records the initial
asset/state mapping and reproduction commands. The observed rider frame byte
is folded WRAM `$E54C`: writers `$14854`/`$14864` increment/wrap 0..3 every ten
frames in this replay. `$E54A` is the countdown. These were sampled at writing
instructions, with A5=$312; the separate `$E548` counter is not a player coordinate.

Next useful work: extract all four rider animation phases and trace their ROM
source pointers, then map player/world coordinates and terrain IDs for a native
overworld slice. Credits timing remains a separate oracle discrepancy. There
is still no native game crate.

Commands, observations, and artifacts are recorded in
[`investigation-log.md`](investigation-log.md). Keep this file current at each
milestone. Generated evidence goes under `docs/evidence/` for small reproducible
reports and `.mdx/artifacts/` for larger regenerable files.

## Important investigation rules

- Use `mdx disasm` to see the current decoder's interpretation of cartridge
  instructions. Do not hand-decode hex dumps.
- Verify semantics independently with Genesis Plus GX's mask/pattern dispatch
  records and handler bodies, or the SingleStepTests corpus.
- Sample registers at the actual consuming instruction; record the writing PC
  for bus writes. Watch folded RAM offsets across aliases and all write widths.
- Compare aligned events/state, not a fixed instruction budget to an unrelated
  oracle frame. A passing short opcode audit is not a boot-fidelity gate.
- Prior notes and memories are hypotheses until their evidence is checked;
  several encoding claims currently disagree with the implementation.
