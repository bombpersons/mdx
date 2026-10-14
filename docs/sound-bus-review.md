# FM / sound-bus evidence review

This is the historical2026-10-13 checkpoint. Subsequent
[VBlank delivery](z80-vblank-2026-10-14.md) and
[Z80/shared-bus timing](z80-contention-2026-10-14.md) corrections supersede its
IRQ/banked-bus scheduler limitations; audio synthesis is still pending.

Reviewed canonical `/home/maisy/opencode_projects/mdx` on 2026-10-13 against
independent `cache/oracle/Genesis-Plus-GX`, revision
`393f70cd64c7334564c19a424c383f53986c13f4`. Changes are confined to
`crates/mdcore/src/{fm,system,memmap}.rs`; CPU/VDP implementation files were not
edited. No commits or agents. Existing work was preserved in AFT checkpoint
`sound-review-start` (all three files), stored at
`/home/maisy/.local/share/cortexkit/aft/opencode/checkpoints/271a1458ff4c02e0/sound-review-start`.

## Independent sources and conclusions

All paths in this table are relative to the oracle checkout above.

| Source | Observed behavior / implemented correction |
|---|---|
| `core/sound/sound.c:45-46,85-156,320-329` | Internal clock is master/42; MAME sample/timer clock is master/1008. Data writes alone set BUSY until `(ceil(master/42)+32)*42`; address writes do not extend it. Discrete-chip status reads work only on port 0; other ports return zero. Existing busy ratio and formula were correct and retained. Timer progression now preserves the wrapper's absolute, upward-rounded sample horizon. |
| `core/sound/ym2612.c:782-862,1540-1553,1912-2003,2151-2167` | A is ten bits, period `1024-A` samples; B period `(256-B)*16` samples. Status bits are A=1/B=2; reads do not clear them. Register 0x27 bits 0/1 load, 2/3 enable status, 4/5 reset flags. Only load rising edges reload; changing a period while running changes the next reload, not the in-flight count. Corrected this last case and replaced the obsolete read-to-clear test. |
| `core/sound/ym2612.c:1957-1998` | There is **one shared bank-tagged address latch**, not an independent operational latch per port pair. Either data port uses it. Corrected cross-pair writes and kept bank-1 diagnostic indices distinct (`0x100..0x1ff`); bank-1 addresses cannot program bank-0 timers. |
| `core/mem68k.c:141-224` | Granted 68000 Z80-bus accesses cost one extra 68000 cycle, including RAM and bank accesses, not just FM. A word read replicates one byte into both lanes; a word write writes only its high byte. Corrected RAM word behavior and FM/RAM mirrors throughout the A0 aperture. Bank writes use the same serial latch from either CPU. |
| `core/mem68k.c:335-353,466-472,496-509,610-617,642-660,1007-1016` | BUSREQ/BUSACK is the A111xx page; RESET is A112xx. Even byte writes use bit 0; word writes use D8. A11100 reads drive only D0; word reads drive only D8. **A11101 is open bus**, not an acknowledge byte. Added both control pages to decoding, preserved undriven lanes, ignored odd writes, and removed erroneous A11102-as-RESET handling. |
| `core/genesis.c:302-310,471-572`; `core/system.c:497-521` | Initial Z80 reset is asserted. A bus grant requires BUSREQ asserted AND RESET released (`zstate==3`); Z80 execution requires reset and BUSREQ both released (`zstate==1`). RESET assertion clears FM; release resets Z80/FM. Removed fabricated writes to RAM 0x1ffb/0x1fff; BUSREQ now actually stops Z80 execution. RESET release restarts PC at zero. Z80 T-states cost 15 master clocks versus 7 per 68000 cycle; replaced `68000_cycles*2` with a signed clock budget carrying instruction overshoot. |
| `core/memz80.c:120-224,228-256`; `core/genesis.c:157-158` | Z80 FM memory reads/writes route to the same chip, low address bits selecting the port. Replaced hard-coded Z80 FM reads. Genesis I/O ports are unused (read 0xff, writes have no device effects); removed SMS-style I/O decoding and fictitious high-port FM writes, retaining diagnostic counts. |

`fm.registers`, `reg_writes`, and `reg_last` now have 512 bank-aware entries;
boot observation vectors consequently expose 512 entries. This is a diagnostic
shape change. Register mirrors are not a model of every analog FM register effect.

## Verification

- An isolated C assertion harness at `/tmp/mdx-sound-review/probe.c` compiled
  **unchanged** oracle `core/sound/ym2612.c`, using a minimal temporary platform
  header and the original core headers. Five scenario groups passed: shared
  cross-pair latch/status persistence; bank-1 timer isolation; A/B periods and
  reset pulses; in-flight count preservation across period/control rewrites;
  stopped/disabled status and chip reset. This executes the independent FM core,
  not a reimplementation of its timer equations. BUSY, wrapper sample rounding,
  and bus arbitration were verified from the source paths above, not exercised
  by that isolated core probe. The oracle working tree remained clean.
- Thirteen sound-focused Rust tests now cover: five FM cases (latch/banks,
  BUSY boundaries, A/B status/reset, load edges, tick partitioning/chip reset),
  seven System cases (BUSACK lanes, upload/access grants, extra latency,
  stopping/reset without RAM mutation, shared FM reads/unused I/O, serial bank,
  clock budget), and one memmap case covering port/control mirrors. These include
  assertions against numeric oracle-derived expectations, not just snapshots.
- `cargo test -p mdcore --release --locked`: **121 passed, 10 ignored, 0 failed**
  across unit/integration tests (107 unit passes plus 14 integration passes).
- `cargo check --workspace --release --locked`: passed; existing warnings outside
  the sound scope remain. AFT inspection could not provide authoritative Rust
  diagnostics for the sibling canonical checkout; Cargo supplied the gate.

Oracle source SHA-256 anchors:

```text
67bca180dd13c61a1e68901035bba871bb0124fe09a4b79192d6242013672a04  core/sound/ym2612.c
5c6b121097d5018e43ce3d1228281d3e80850a5579275964fddfed32b07b0472  core/sound/sound.c
5a16c625ea77934fee90cf5d5eaa72cf92fc478811e20cb0af3138b247e9f022  core/mem68k.c
280fc4da41dbb0800c7e579a765e8013eb5af1f6f5059709fbb66d3a9a8a61a6  core/genesis.c
142e79e651c56ef74b2557e4736734ec5592498597143ee58cee4ed39bf108ef  core/memz80.c
5fd68af02371a8a03d05d06711b68403e05b448d82cc29dbf7c1827f29c4d23f  /tmp/mdx-sound-review/probe.c
```

## Limits

This is discrete-YM2612 register/status verification plus synthetic sound-bus
regressions, not all-ROM or hardware-cycle equivalence. No synthesis was added.
The reference wrapper rounds its sample horizon up, potentially making a timer
flag visible up to 1007 master clocks before an ideal continuous-count model;
the Rust tests explicitly preserve this reference behavior.

Sound scheduling remains instruction-granular: FM accesses do not yet receive
exact intra-instruction timestamps, Z80 accesses within a batch share an FM
timestamp, and BUSREQ/RESET transitions do not resynchronize mid-instruction.
Undriven 68000 lanes use a boundary prefetch snapshot, not GPGX's exact current-PC
bus value at each access. Z80 banked-68K wait states, DMA bus contention, Z80 IRQ
delivery, PSG behavior, and A0 miscellaneous lockup semantics are still incomplete.
Integrated/enhanced YM variants and Nuked OPN2 behavior are outside this model.
The temporary C harness does not establish whole-emulator bus/runtime equivalence.
