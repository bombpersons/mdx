# Timing investigation resumed — 2026-10-13

Current continuation: [kb-timing-2026-10-13.md](kb-timing-2026-10-13.md) supersedes
the remaining first-HV offsets below and isolates the first Z80 bus-contention gap.

## Earlier result: IRQ and shift corrections integrated

A subsequent first-boundary trace found that all status-register MOVE forms
were charged a fixed12 clocks. In particular, MOVE immediate to SR cost12
instead of16, shifting dispatch by28 master clocks before the IRQ could arise.
MOVE from SR to a data register also cost12 instead of6; memory/extension EA
costs were missing in the other forms. The corrected table covers all30 legal
addressing classes for MOVE from SR/to CCR/to SR. The dead immediate-only
handler after the generic status-move handler was removed.

For H32/startup-nops1, the unchanged reference dispatches the instruction
following MOVE #$2000,SR at master784854; old mdcore dispatches at784826.
With the status timing and E-clock fixes, the first handler starts at the exact
reference frame-relative master1122. Across all20 startup programs, first-handler
PC/SR/D0-D7/A0-A7 and master clocks agree, as do the first20 dispatch clocks/PCs.
All640 IRQ-entry HV samples now agree, including the seven former initial gaps.

Crucially, **status timing + E-clock passes all27 frozen image checkpoints**,
including KB6000. Adding the independently supported shift-refresh suppression
also passes all27, including EWJ6000. Both rules are therefore integrated;
the isolated failures below document earlier incomplete combinations. The shift
matrix still matches3840/3840 samples. IRQ and shift tests now run by default.
185 default Rust tests (23 ignored), four Python checks and real four-cartridge/
walking MCP validation pass. Golden inputs/references and original oracle
execution were not changed, and no synthetic data-bus substitutions are used.

King's Bounty's actual first counter reads improve from9C89/A036 to9C9D/A04A,
but still differ from independent9C9C/A049. Its next two initial RNG reads are
CF35/CF90 versusCF33/CF8F. These small timing differences still feed live RNG;
6000-frame WRAM remains divergent. Synthetic phase equality does not prove
integrated game/sound equality.

Evidence and reproduction:

```sh
cargo test -p mdcore --release --locked --test status_move_timing --test irq_counter_phase --test shift_refresh
cargo build -p mdcore --release --locked --example irq_boundary
python3 tools/probes/irq_boundary.py \
  --oracle-core cache/oracle/resume-timing-trace/genesis_plus_gx_libretro.so \
  --out .mdx/NEW-BOUNDARY-DIRECTORY
```

The boundary driver generates only authored ROMs, records ROM/core/binary
identities and refuses an existing output directory. The example's cumulative
clock is limited to these probes without DMA/FIFO waits; rounded instruction
costs must not be used as an exact general-purpose master-clock source.
Raw traces, tested binaries, source copies, real-game samples and logs are
under `.mdx/irq-boundary/`. The portable ledger is
[evidence/irq-boundary-session.json](evidence/irq-boundary-session.json).

The remainder records the previous investigation, before this correction.

## Active correction: external DMA resume timing

The active engine avoids charging the triggering instruction's fixed clocks a
second time when an external DMA continues into another scheduler update.
VBLANK transfers also use the reference's remaining blank-period update window,
rather than truncating capacity separately on each blank scanline.

An authored256-word transfer independently identified the discrepancy. The
unchanged GPGX dispatch starts at master785736 and resumes at794345:8609 master
clocks. The old model added the triggering opcode's196 master clocks again;
reported CPU-cycle rounding adds at most another6 clocks to a trace sum.
The transfer's32 actual HV readings now all agree.

The broader matrix covers H32/H40, VRAM/CRAM/VSRAM, active/blank display and
1/16/256/4096 words. **1472/1536 readings agree**. All46 completed profiles other
than the two4096-word active-VRAM cases are exact. Those two remaining gaps
cross the initial frame/viewport transition and are retained explicitly as a
known failing regression; their cause is not established by this pass.

The active DMA correction passed all12 original image gates and the17-case
frozen cartridge gate, preserving27 distinct same-frame/input checkpoints.
Default Rust and Python tests plus a real four-cartridge/walking MCP smoke
also passed. Later project-context work changes transport/path handling only.

Reproduction and portable tests:

```sh
python3 tools/probes/hv_counter_phase.py --dma-words 1 16 256 4096 \
  --dma-widths 32 40 --dma-target cram --dma-display --verify --out .mdx/NEW-DIRECTORY
cargo test -p mdcore --release --locked --test dma_counter_phase
```

Run the ignored large-transfer regression explicitly to investigate that known
gap. Its reference bytes were not changed to make the active engine pass.

## Isolated: register-count shift/rotate refresh suppression

Independent opcode handlers suppress one refresh deadline crossed during the
variable internal phase of a nonzero register-count shift/rotate. Immediate
forms do not apply that rule. An isolated implementation makes **3840/3840 HV
samples** equal across all24 operation/size/direction forms and counts0/1/8/31/63.
The initial long-LSL baseline matched only67/160 corresponding samples.

However, shift suppression alone reproduces the prior **1263-pixel EWJ6000
regression**. The earlier combined division experiment was therefore insufficient
to attribute that regression to division costs. Adding the independently tested
DMA correction does not close it. The shift correction is **not active**.

Source and executable: `.mdx/resume-timing/shift-only-candidate/`.
The combined candidate is under `combined-candidate/`. Portable oracle samples
are in `crates/mdcore/tests/fixtures/shift_refresh_samples.json`; the explicit
ignored `shift_refresh` test records the discrepancy without breaking defaults.
No division implementation or synthetic oracle-input substitution was shipped.

## Isolated: autovector E-clock acknowledge timing

The reference uses phase-dependent50..59 CPU clocks for autovector entry,
compared with the active model's fixed44. A controlled implementation uses
`((master_cycles % 896040) / 7) % 10` to select the reference's table.

Twenty authored programs (H32/H40, ten startup phases) improve from **39/640**
to **633/640** matching IRQ-entry HV reads. All620 readings after the first IRQ
match; seven first-IRQ samples remain different. The first-boundary discrepancy
is retained separately, not discarded from the result.

This candidate changes the KB6000 image by **672 pixels**, so it is **not active**.
The first real King's Bounty HV reads only improve slightly (`9C89`→`9C8B`,
`A036`→`A038`) and still differ from the independent `9C9C`/`A049` values.
It does not close the live RNG/state discrepancy.

Source and executable: `.mdx/resume-timing/eclock-candidate/`.
Reproduce with `tools/probes/irq_counter_phase.py --binary PATH --out NEW-DIRECTORY`.
`irq_counter_phase` contains explicit ignored regressions and portable samples.

## Probe coverage and evidence

Counter programs now write a completion marker after collecting all32 samples.
Both emulators must reach it before the probe compares their output. Previously,
a4096-word DMA run stopped before either sampler executed, reporting32 matching
zero words. The new driver rejects that incomplete run, and `--verify` reports
real sample differences as a nonzero exit.

The cached original emulator, ROMs, replays and frozen images were left unchanged.
Reference-core/source identities are stored in each new JSON fixture. Synthetic
programs and observations are local under `.mdx/resume-timing/`; the active
correction needs no commercial ROM or oracle at default-test runtime.

Investigation directories: `dma-baseline/`, `dma-complete/`, `dma-overlap/`,
`dma-matrix/`, `shift-baseline/`, `shift-refresh/`, `shift-matrix/`,
`irq-baseline-v2/`, `irq-eclock/`. The timing-only oracle build is a private copy
at `cache/oracle/resume-timing-trace/`; its manifest binds the original source,
observation hook and produced core. Failed experiment logs remain failed evidence.

Next investigate the first IRQ acceptance edge and first divergent consumed
input in the real programs. Reconcile the independently supported timing rules
with the unchanged image gates before integrating the isolated candidates.
Z80 integrated sound-state differences remain a separate open question.
