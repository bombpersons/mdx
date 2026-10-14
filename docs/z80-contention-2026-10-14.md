# Z80 decoding and shared-bus timing checkpoint

The first King's Bounty sound-bus stall now matches the independent reference.
Two consecutive comparisons of the same build cover **72,000,000 dispatches
from boot through frame6386**, with identical pre-dispatch PC/master clocks.
The frame6000 capture also matches unchanged GPGX in all65536 WRAM bytes,
all8192 Z80 RAM bytes, VRAM/CRAM/VSRAM, VDP registers and screenshot pixels.
This closes the previously observed RNG/live-WRAM discrepancy on that replay.

Evidence: [portable ledger](evidence/z80-contention-session.json). Private logs,
captures, experimental binaries and original commercial assets remain ignored
under `.mdx/z80-contention/`, `cache/` and `rom/`. No original oracle execution,
ROM, replay or frozen reference image was changed.

## Corrections

The original driver diverged before its first shared-bus request. An authored
flat64KiB harness executing unchanged GPGX Z80 code exposed indexed displacement,
IXH/IXL selection, ignored/repeated prefix, ED classification, EXX, CB bit/rotate,
arithmetic/undocumented flag, block-I/O and refresh-register defects. Warm RESET
now preserves general registers, stack and elapsed clocks while resetting control
state. The expected instruction observations are portable fixtures; execution
callback order and the reference's HALT opcode-PC representation are explicitly
normalized when comparing final memory and resume PC.

Unblocking the driver exposed a separate port bug: PSG writes were reaching VDP
control. VDP byte writes now duplicate the driven byte into both lanes without
reading the port first. Only data/control port aliases invoke those operations;
PSG addresses no longer change the VDP. PSG synthesis remains unimplemented.

Sound execution now retains an absolute master-clock cursor and runs to physical
scanline/VINT boundaries, preserving CPU instruction overshoot. Shared bank/VDP
accesses account for the reference's approximate68000 wait and three extra Z80
T states. BUSREQ/RESET transitions synchronize the running driver before stopping
it; release aligns its clock. Frame-relative bus phases retain the ending frame's
origin when a CPU instruction crosses the boundary.

Continuing external DMA absorbs intermediate CPU waits and sound-clock debt.
The model distinguishes each DMA update's endpoint from the aggregate transfer
end: a Z80 instruction that overshoots one endpoint may complete its access and
reach another opcode before a later update blocks it. Refresh advances the VDP
read phase before the opcode but defers any crossed sound batch until that opcode
retires. Repeating ED instructions retain their five-T-state tail after the bus
operation, including ignored index prefixes.

## Validation

- **10,752 instruction cases** agree with unchanged GPGX: seven opcode/prefix
  groups,256 opcodes and six flag seeds; registers, retired T states and final
  written bytes. This is one seeded memory/register pattern, not arbitrary data
  or complete MEMPTR-history validation.
- **350 authored two-CPU cases /11,200 HV samples** agree:224 RAM/FM controls,
  shared ROM/direct/banked PSG accesses, DMA0/8/256 and frame-wrap cases, plus98
  delayed second-access/DMA-boundary cases and28 LDIR cases with/without an ignored
  index prefix. Both emulators must reach a completion
  marker. Default Rust tests consume independently generated expected fixtures.
- **211 default Rust tests pass**,24 remain ignored for external assets or
  separately documented gaps. Four Python observer checks and all27 previously
  frozen image checkpoints pass. The external frame943 bus regression passes
  when explicitly enabled; its original expected clock was preserved.
- The contiguous dispatch ranges are0..64,000,000 and64,000,000..72,000,000.
  These checks compare PC/master clocks; they do not compare every register or
  complete machine state at every dispatch. The partial observer images are not
  reference captures. Frame6000 memory/video comparison is a separate completed
  run of the unchanged original core.
- Cadash2700→4500 save/load matches uninterrupted execution by full mdcore-state
  hash, every captured memory buffer and screenshot. Fresh MCP zero-step RAM
  loads and boot/trace/watch continuation agree. States use **format3**, retaining
  Z80 WZ and the absolute sound/DMA scheduler clocks; recapture format1/2 states.

The later [DMA viewport checkpoint](dma-viewport-2026-10-14.md) resolves the two
large active-DMA failures described below. That section records this checkpoint's
historical limits; current sound work is in [audio-development.md](audio-development.md).

## Remaining limits and next work

The two large active VRAM DMA cases crossing the initial viewport/frame change
remain open; the ignored regression was rerun and still fails against its
unchanged expectation. FM accesses within sound batches still lack precise
intra-instruction timestamps. Opcode seeds, finite images and this one replay
do not establish hardware, all-ROM, full sound-state or synthesized-audio
equivalence. No synthesis has been added by this checkpoint.

Resolve the large active-DMA boundary next, then the queued offline YM2612/PSG
WAV feature with deterministic save/load and independent audio checks. The user
also queued broader failure hunting across the local ROM corpus after sound.
