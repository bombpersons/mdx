# First image: where it actually stands

Written after the display-enable fix. Reconstructed with `mdx disasm`, whose
listing now agrees with the boot trace instruction-for-instruction.

## Fixed on the way here

* `Vdp::display_enabled` read register 1 **bit 6**, which is the interlace
  flag. The display enable is **bit 4**. Every correctly configured cartridge
  was being reported as switched off, and the renderer then filled the
  backdrop and returned - indistinguishable from a renderer that cannot draw.
  Genesis Plus GX agrees: `vdp_render.c:4285` tests `reg[1] & 0x10`.
* The register file was `[u8; 64]`. There are 32, so a malformed control-port
  write landed somewhere plausible instead of being rejected.
* The screen-mode limitation check mixed register 12 bits 2-0 (plane-A
  nametable size) into the mode bits. Register 1 bits 3-1 are the mode.

## King's Bounty at 12M instructions

Zero exceptions. Display enabled. But **CRAM is entirely empty**
(`cram_entries_nonzero: 0`), so every pixel is backdrop colour 0 and the
frame is black however correct the renderer is.

The boot parks in a VRAM fill loop, 1,009,612 iterations of:

```
0x27F42  move.l #$BBBBBBBB,$FFFF0006   ; debug marker into work RAM $0006
0x27F4C  tst.w d5
0x27F4E  ble.b 0x27F78
0x27F50  cmp.w #1,d5
0x27F54  bne.b 0x27F60
0x27F56  move.l #$22222222,$FFFF0006
0x27F60  cmp.w #5,d0
0x27F64  bne.b 0x27F70
0x27F66  move.l #$EEEEEEEE,$FFFF0006
0x27F70  move.l d3,d1
0x27F72  move.l a1,d2
0x27F74  bsr.w 0x27CC6                ; the worker
0x27F78  tst.w d5
0x27F7A  bne.b 0x27F9E                ; loop
```

and the worker at 0x27CC6:

```
0x27CC6  movem.l a0-a7,-(a7)
0x27CCA  cmp.l d1,d2
0x27CCC  blt.b 0x27D0A
0x27CCE  move.l d2,d0
0x27CD0  bsr.w 0x2A8D5
0x27CD4  move.w d4,d7
0x27CD6  subi.w #$1C,d5
0x27CDA  neg.w d5
0x27CDC  asl.l d5,d6
0x27CDE  move.l d1,d0
0x27CE0  bsr.w 0x2678D
0x27CE4  cmp.w d7,d4
0x27CE6  bne.b 0x27CF0
0x27CE8  or.l d3,d6
0x27CEA  move.l d6,(d4.w,a6)          ; VRAM write
0x27CEE  bra.b 0x27D0A
0x27CF0  move.l d3,(d4.w,a6)          ; VRAM write
0x27CF4  addi.w #$20,d4               ; fill chase, step 0x20
0x27CF8  cmp.w d4,d7
0x27CFA  beq.b 0x27D06
0x27CFC  move.l ($FFFF0006),(d4.w,a6)  ; fill from work RAM $0006
0x27D04  bra.b 0x27CF4
```

So this loop fills **VRAM only**. The palette comes later, which means
running longer is not by itself the answer - or the loop is not terminating
and that is the answer.

## The two open questions, in order

1. **Why does the 0x27F42 loop not exit?** It exits when `d5` is zero
   (`bne.b` at 0x27F7A). The worker subtracts `#$1C` from `d5` and then
   negates it, so `d5` should march to zero. Instrument `d5` across the loop
   the way the a7 death was found - a watchdog that checks the predicate every
   instruction, not a bisection (`d5` is not monotonic).
2. **Where does CRAM get written?** `vdp write` with code 1. Nothing reaches
   it in 12M instructions. Build the write tracer (P3) and read the answer
   off it rather than hunting: every CRAM write with the PC that made it.

The oracle frame at `testdata/oracle_kb_frame400.png` is the target.

## Tracer result: the cartridge never writes the palette

`System` now counts data-port writes per destination and keeps the first 32
with the PC that made them, reported as `vdp_data_counts` / `vdp_data_writes`
in `mdx boot`. At four million instructions, King's Bounty has made **33,922
VRAM writes and zero CRAM writes** - the first ten all from 0x27E60, which is
the VRAM clear. So "run it longer" is not the missing step; the game is
stuck before the palette load.

## Two decode questions the listing raised and the oracle settled

* **MOVEM's mask.** The mask splits bit r -> REG_DA[15 - r]: bit 0 is A7 and
  bit 15 is D0, so the cartridge's `48E7 FF00` prologue is
  `movem.l d0-d7,-(sp)` and the matching `4CDF FF00` is the epilogue. I read
  the manual's diagram as bit 0 = A0, "fixed" it, and was wrong - GPGX's
  `m68ki_op_movem_16_re_pd` writes `REG_DA[15 - i]`, the same way the code
  already did. Reverted. The lesson is the one from DBcc: the register file
  diagram is exactly where an intuition goes to fail.
* **Bcc's displacement width** is the zero low byte, not bit 8 (see the
  memory note). `0x6708` is `beq.b $0008`; reading it as a word branch costs
  two bytes of PC per occurrence.

## Still open

The 0x27F42 loop is a VRAM fill driven by `bsr 0x27CC6`, and it exits when
`d5` reaches zero. The worker does `subi.w #$1C,d5` / `neg.w d5`, so d5 is
meant to march to zero. Instrument d5 the way a7 was found - a watchdog that
tests the predicate every instruction, not a bisection, because d5 is not
monotonic.

Once the loop exits, the palette load is next, and then the first frame has
something to match against `testdata/oracle_kb_frame400.png`.

## MoveQ: the boundary is unconfirmed

The disassembler currently treats `ir & 0x0020 != 0 && ir & 0x0080 == 0 &&
ir & 0x0100 == 0` on opcode line 0x7000 as MOVEQ. That came from the manual's
diagram. The oracle's jump table could not settle it: its index base is not
0x0000, so counting entries to `m68k_op_moveq_32` does not say which opcodes
the block covers (0x7001 lands on MOVEQ and 0x7000 on `ble_32`, which cannot
both be true).

Settle it the same way everything else in this project has been settled:
find a boundary in the boot trace inside the range the two hypotheses
disagree about. Not worth changing on a guess - five decode "fixes" from
intuition alone have been wrong this session.

## Confirmed: the fill loop finishes, and control leaves the cartridge

At 260 million instructions the loop has run 4,194,433 passes - it does
terminate - and the game then jumps into work RAM at 0xBBE09BF4, executing
zeros and faulting on every one.

The audit names the exact instruction, once its budget is raised. It is now
`MDX_AUDIT_INSTRUCTIONS` (default 3,000,000; the transition is some fifty
million in, so a short run can never see it):

```
control flow left the cartridge: the instruction at 0x00027D0E transferred
control to 0xBBBBBBBA through a7 = 0xFFFF9C10, popping the word 0x2222.
```

Read that carefully, because every number in it is information:

* 0x27D0E is the `rts` at the end of the VRAM fill worker at 0x27CC6 - the
  routine the 0x27F42 loop calls 4.2 million times.
* The word it pops is `0x2222`, and the return address lands in the
  neighbourhood of `0xBBBBBBBA`. **Both are the game's own debug markers.**
  0x27F42 stores `$BBBBBBBB` to work RAM $0006, 0x27F56 stores `$22222222`
  and 0x27F66 stores `$EEEEEEEE`; the worker copies whichever is current into
  its destination at 0x27CFC.
* So the return address on the stack has been overwritten by a fill value.
  The stack has been written through as if it were a destination buffer.
* `a7 = 0xFFFF9C10` is the reset stack pointer to within 0x0E.

Next question, in order: which destination in the worker walks into the
stack. `0x27CFC` writes through `(d4.w,a6)` with `a6 = 0xFFFF09EC` and `d4`
stepping by 0x20 under the bound set by `d7` at 0x27CD4 - if that bound is
computed from a stale `d4` or a mis-sized tile, the sweep runs past the end
of its buffer and into the frame. The instruction to instrument is 0x27CDC,
`asl.l d5,d6`, which builds the sweep bound from the counter.

## Tooling note

`mdx boot --trace N` only covers the first N instructions, and the first
400,000 are still in the checksum and VDP init, so it can never show a
transition fifty million instructions in. `MDX_AUDIT_INSTRUCTIONS` can, and
the opcode audit's job is exactly this - it was simply being run at a budget
that could not reach the interesting part.

## Dead end, recorded so it is not retried

The obvious suspect for the fill sweep overrunning its buffer is a6: the
loop tail at 0x27FAC is `adda.l #0xE8,a6`, 232 bytes per outer pass, with
nothing checking it against an end address, and work RAM runs out at
0xFFFF9C1E. At that rate the stack should be reached in a little over
37,000 passes.

It is not. A watchdog that checks a6 at 0x27FAC on every pass, for four
hundred million instructions, never once sees it reach the reset stack
pointer. The bound holds.

So the debug markers on the stack are coming from somewhere else. What is
left, in the order worth checking:

* the worker's own frame - `movem.l d0-d7,-(sp)` at 0x27CC6 and
  `movem.l (sp)+,d0-d7` at 0x27D0A must move a7 by the same 32 bytes, and
  the audit's a7 = 0xFFFF9C10 at the `rts` is fourteen bytes below the reset
  value, which is not a number either of them explains;
* `adda.l d6,a2` at 0x27FA6, where d6 comes from `asl.l d5,d6` at 0x27CDC and
  nothing bounds it;
* something writing `(d4.w,a6)` at 0x27CFC with a stale d4.

The watchdog pattern is the one that works for each of these: assert the
predicate on every arrival, never bisect it.

## Two more dead ends, and two more wrong bits

**The worker's frame is balanced.** Pairing a7 at the entry of 0x27CC6 with
a7 at its `rts` over four million calls finds no mismatch, so the `movem.l
d0-d7` save and restore are not what puts a marker on the stack.

**The renderer was misreporting two bits.** Interlace is register 1 bit 6;
the renderer was reading bit 5, which is the VBlank interrupt enable, so
every normally configured cartridge claimed to be interlaced. And the width
is chosen by the RS pin, register 12 bit 0 - Genesis Plus GX decides h40 the
same way - not by register 1's TMS99xx mode bits. King's Bounty and Hokuto
both write reg1 = $8034, so a width read out of those bits describes a mode
neither game is using. Both fixed; the renderer now reports no limitations at
all, and 256 pixels wide because register 12 bit 0 is set.

That 256 is not a renderer bug: the oracle's 320x224 frame comes from the
game writing register 12 again later, and our boot never gets there. It is
another symptom of the same frontier, not a separate one.

## What is left to look at

* `adda.l d6,a2` at 0x27FA6, where d6 comes from `asl.l d5,d6` at 0x27CDC
  and nothing bounds it;
* `move.l (d4.w,a6)` at 0x27CFC with a stale d4.

Same method as always: assert the predicate on every arrival.

## Found: it is d4, not a6

Scanning the frame below a7 for the marker every instruction names the
writer:

```
0x2222 reached the stack at 0x00FF9BF0, written by the instruction
completing at 0x27D04 (instruction 42,164,511)
```

0x27D04 is the `bra.b 0x27CF4` that closes the fill chase, so the write is
the instruction before it, `move.l ($FFFF0006),(d4.w,a6)` at 0x27CFC. The
chase steps `d4` by 0x20 until it equals `d7`, and `d7` is just
`move.w d4,d7` at 0x27CD4 - so the bound is whatever `bsr 0x2A8D5` returned
in d4. If that is large, the chase sweeps the whole of work RAM and the
stack with it, copying the marker over every return address it passes.

The a6 hypothesis earlier was the right neighbourhood and the wrong
register: a6 is the base and advances a whole 0xE8 at a time, but it is
`d4.w` - the 16-bit index - that runs away, and only near the end of the
run.

So the next thing to look at is the routine at 0x2A8D5 and the value it
leaves in d0, which becomes d4. That is the last lead on the first image
and it is a single routine.

## Two false starts worth remembering

* Searching for the marker *in a register* finds the checksum routine's
  loop counter passing through 0x2222 - a coincidence of arithmetic, not a
  bug. The thing that broke is a return address in memory, so the search
  belongs there.
* The ROM mirrors at +0x200000, so `mdx disasm --from 0x27FA44` and
  `--from 0x7FA44` are the same bytes and the same code. An address that
  looks like it is inside the fill loop may be inside the boot checksum
  routine at 0x7FA2C.

## The bound chain, read end to end

With the 0x1xxx MOVE line fixed the preamble reads cleanly, and the loop
that seeds the sweep is at 0x27FE8:

```
0x27FE8  moveq #0x1,d1      ; the worker's `cmp.l d1,d2` lower bound
0x27FEA  moveq #0x50,d2     ; upper bound
0x27FF2  moveq #0x12,d5
0x27FF4  bsr.w 0x27C80      ; into the worker
```

and inside it:

```
0x27CCA  cmp.l d1,d2 / blt   ; exit the moment the range is done
0x27CCE  move.l d2,d0        ; d0 = the current tile index
0x27CD0  bsr.b 0x27CAE       ; d4 = (d0 >> 3) << 5  = a VRAM byte address
0x27CD2  move.l d3,d6
0x27CD4  move.w d4,d7        ; the chase bound
```

For d0 = 0x50 that is (0x50 >> 3) << 5 = 320 - plane A's first tile. So the
arithmetic is correct when the inputs are, and the question is what makes
them wrong. Note the call is to **0x27C80**, not to the `movem.l a0-a7`
prologue at 0x27CC6: there is a distinct entry ~70 bytes earlier that this
listing has not been read through yet, and it is where d1/d2/d3 may be
clobbered.

## How this session's conclusions got overturned

Three times, all by the same method and all in the instrument rather than the
emulator. Worth writing down, because the failure mode is a confident wrong
answer rather than a missing one:

1. **Register search.** Looking for $22222222 in a register found the boot
   checksum's loop counter, which passes through that value by arithmetic.
2. **Proximity scan.** Reporting where a value was *seen* rather than where it
   was *written* named an address that the registers cannot produce.
3. **Distance filter and a cap.** Measuring a write's distance from the
   current stack pointer excludes fixed-address overwrites, and a 64-entry
   recording buffer filled with the game's own legitimate writes before it
   ever saw an interesting one.

All three produced a confident, wrong, load-bearing claim. The bus-level
recording - filter at the source, record the writing PC - settled it in
twenty seconds after three failures.

## The markers are instrumentation, not corruption

The whole "something is overwriting the stack" framing was wrong, and the
loop says so itself. At 0x27F42 the cartridge chooses which fill pattern to
stamp with, before each sweep:

```
0x27F42  move.l #$BBBBBBBB,$FFFF0006   ; running
0x27F4C  tst.w d5 / ble 0x27F78        ; nothing to do if the count is spent
0x27F50  cmp.w #1,d5 / bne
0x27F56    move.l #$22222222,$FFFF0006 ; one pass left
0x27F60  cmp.w #5,d0 / bne
0x27F66    move.l #$EEEEEEEE,$FFFF0006 ; the d0 == 5 checkpoint
0x27F70  move.l d3,d1
0x27F72  move.l a1,d2
0x27F74  bsr.w 0x27CC6                 ; the sweep
0x27F78  tst.w d5 / bne 0x27F9E
0x27F7C    move.l #$22222222,$FFFF0006 ; finished
```

$BBBBBBBB in progress, $EEEEEEEE at a checkpoint, $22222222 at the end. The
game is stamping its own progress into the thing it is filling. So there is
no memory-corruption bug to hunt: the sweep is *supposed* to write those
values, and the only real question is why its destination reaches the stack.

And note the bounds are not the d1/d2 the caller seeded at 0x27FE8 - those
are overwritten. The loop passes `d1 = d3` and `d2 = a1`, with
`d3 = 0x00200000` (moveq #0x20,d3 / swap d3) and `a1 = 0x00200000`, and
0x27C80 does `addq.w #6,d3` once per attribute row. The worker's
`cmp.l d1,d2 / blt` is therefore comparing two cartridge-window addresses
that advance at different rates, and `(d4.w,a6)` is the destination that
has to stop before $FF9C00.

That is the whole remaining question, and it is one comparison.

## Wrong: "the counter is 16 bits and wraps 64 times"

`d5` is loaded with a longword but tested with `tst.w d5`, which looks like
a width bug worth 65536 passes of overrun. It is not. Sampling d5 at the
test itself gives $C0FFFF - 12,648,447 - and the low word is $FFFF, so the
test behaves correctly and the phase runs exactly 65,535 passes down to
$C00000 before ending. The $00C00000 that set off this whole theory was a
later phase's seed, read out of a sample that had already moved on.

Recorded because the shape of the mistake is the same as the three before
it: a number taken from one moment and used to explain another. Every one of
today's false starts has been a value read in the wrong place, and none has
been a value the emulator computed wrongly.

## Measured at the instruction, the sweep is arithmetic-consistent

Recording the registers in the same hook that catches the bad write:

```
the fill chase wrote the marker $BBBB over the stack at 0x00FF9C0C from
the instruction at 0x27CFC: a6=0x00FF0BEC d4=0x00029020
(a6 + d4.w = 0x00FF9C0C)
```

So the address is exactly what the instruction computed - no decode bug, the
sweep writes precisely where it is told - and the register at fault is a6.
It should be $FFFF09EC, seeded by `lea.l $FFFF09EC,a6` at 0x27F26, and it is
$00FF0BEC: some sixteen million `+4` steps later.

Two things follow, and only two:

* the stepping is real and unbounded. 0x00FF0BEC - 0xFFFF09EC is 0x00FF0200,
  which is 4,194,304 steps of four - 2^22 again, the same power of two the
  fill loop keeps landing on.
* nothing in the loop bounds a6. `adda.l #0xE8,a6` at 0x27FAC is per outer
  pass and `addq.l #4,a6` at 0x27F9E is per inner; the exit tests are
  `tst.w d5` and `cmp.l d1,d2`, neither of which involves a6.

The question is therefore not "what is wrong with the address" - the address
is right - but **which loop was supposed to stop a6**, and the answer is
still not in this listing. d3 at 0x27C80 is the only register that moves in
steps of 6, and the worker compares d1/d2 that the loop sets from d3 and a1.
d3 growing by 6 per attribute row against a1 growing by 4 per inner pass is
the mismatch worth reading next; the two advance at different rates and the
comparison is between them.

## On reading bits by eye

Twice today I have taken a multi-bit field apart by hand, got a confident
answer, and been wrong. The oracle's jump table is indexed by opcode only
loosely: `names[0x588E]` and `names[0x7001]` both gave handlers that could
not be right, while the mask-and-pattern entries are unambiguous. Use the
mask, not the index, and use the trace, not the mental model.

## The bounds, measured at the worker's entry

Sampling d1, d2, d3, a1 and a6 at 0x27CCA - the `cmp.l d1,d2` - instead of
inferring them from the listing:

```
     d1        d2        d3        a1        a6
  0x00200000 0x00200000 0x00200000 0x00200000 0xffff09ec   <- first entry, clean
  0x00200000 0x00200000 0x000097b4 0x00200000 0xffff09ec
  0xb4260000 0x4bfa97b4 0xb4260000 0x4bfa97b4 0x000109f0
  0x851509ec 0x4bfa97b4 0x000097b4 0x4bfa97b4 0x000109f0
  0xb4260000 0x97d52f68 0xb4260000 0x97d52f68 0x000309f4
```

Three things fall out, and the first confirms the reading:

* `d1 = d3` and `d2 = a1` every row, so 0x27F70/0x27F72 are doing what the
  listing says.
* a6 climbs 0x000109F0, 0x000309F4 - +0x20004 a row - which is the runaway
  confirmed again from the other side.
* **d3 alternates between $000097B4 and $B4260000**, and it is only ever
  supposed to change by six per attribute row (`addq.w #6,d3` at 0x27C8E).
  Two values where there should be a sequence is the thing to chase.

$000097B4 is not arbitrary either: `move.l #0x97B4,d6` at 0x27F14 seeds the
tile-data pointer with it. So the register the sweep uses as its lower bound
is holding the tile-data source address, and on alternate entries it is
holding $B4260000, whose low word is zero - a register `addq.w #6,d3`
would have left at 6, not 0.

## The inner loop is not hanging

Two candidate mechanisms, both measured rather than assumed:

* the outer bound `dbf.l d4,0x27F3E` at 0x27FB2 is **never reached** in sixty
  million instructions, so the `adda.l #0xE8,a6` that steps the row by row
  never runs - a6 only ever sees `addq.l #4,a6` from 0x27F9E;
* but the inner bound is healthy: d0 at `dbf.l d0,0x27F44` reads 164,187 and
  steps down by exactly one per pass.

So the inner loop is not stuck; passes simply become enormous as the chase
length `(d7 - d4) / 32` grows, and 0x27FB2 is past the end of a sixty-million
window rather than unreachable. Nothing here is a bug.

That leaves one thing genuinely unexplained, and it is the same one as
before: a6 reaches 0x00FF0BEC from a 0xFFFF09EC seed with only +4 steps
available to it, and the row step that would advance it in 0xE8 units never
executes. Either the chase length is wrong - d7 is `move.w d4,d7` at 0x27CD4,
so a bad d4 from 0x27CAE makes the chase overshoot - or the game intends the
row loop to bound it and our run never gets there. The first is a value
question and the second is a timing question, and the value question is the
cheaper one to answer.

## What each measurement killed today

| theory | killed by |
|---|---|
| marker in a register | the checksum loop's counter passing through 0x2222 |
| where the marker was *seen* | a bus-level watchpoint on the write |
| "within 0x100 of sp" | a write to a fixed address is excluded by that filter |
| 16-bit counter wrapping 64 times | d5 sampled at the test: $C0FFFF, not $00C00000 |
| the inner loop hangs | d0 counting down correctly, 164,187 and falling |

Every one of these was confident, and every one was the instrument.

## a1's growth is by design; the question is whether the outer bound closes

The core's `to_data_register` path is correct - `Alu::Sub => self.sub_flags(size,
s, ea_val)` with the register as the minuend - and the disassembler now names
the same instruction it executes. So the cartridge really is doing:

```
0x27FA2  sub.l a3,d3      d3  -= a3
0x27FA4  adda.l a3,a1     a1  += a3
```

with a3 held at $97B4 by `movea.l d6,a3` at 0x27F1C. That is a symmetric
counter-and-pointer pair walking together through the tile data, and a1
growing by $97B4 a pass is the design, not a fault. d1 = d3 and d2 = a1 are
then the sweep's bounds, moving toward each other, which is exactly what
`cmp.l d1,d2` at 0x27CCA wants.

So both runaways are explicable:

* a6, +4 a pass, 2^22 passes to reach $00FF0BEC from $FFFF09EC;
* a1, +$97B4 a pass, which the inner sweep of 164,187 passes carries a long
  way before the row step at 0x27FAC would reset it.

What is left is whether `dbf.l d4,0x27F3E` at 0x27FB2 ever executes. It did
not in sixty million instructions, but that is under two passes of the inner
loop's trip count, so the measurement says nothing yet. A four-billion
instruction run is in flight to answer it, and until that lands the
right-sized conclusion is that the outer bound is *unobserved*, not *broken*.

## The chase, in full

```
0x27CD4  move.w d4,d7        d7 = f(d2)   the upper tile address
0x27CD6  subi.w #0x1C,d5
0x27CDA  neg.w d5 / asl.l d5,d6
0x27CDE  move.l d1,d0        the lower bound...
0x27CE0  bsr.b 0x27CAE       ...recomputed into d4 = f(d1)
0x27CE2  lsr.l d5,d3
0x27CE4  cmp.w d7,d4 / bne
0x27CF0    move.l d3,0.d4W,a6      write the attribute run
0x27CF4    addi.w #0x20,d4
0x27CF8    cmp.w d4,d7 / beq 0x27D06
0x27CFC    move.l ($FFFF0006),0.d4W,a6   stamp the progress marker
0x27D04    bt.b 0x27CF4
0x27D06    move.l d6,0.d7W,a6
0x27D0A  movem.l d0-d7,(a7)+ / rts
```

where f(x) = ((x >> 16) & 0xFFFF) >> 3 << 5, i.e. x >> 19, times 32. So
d4 is a VRAM byte address in 0..0x7FE0 and (d4.w,a6) with a6 = $FFFF09EC
covers $FFFF09EC..$FFFF89CC - entirely inside work RAM. **The sweep cannot
reach the stack through its own arithmetic.**

That is the point: the only way to write 0x00FF9C0C is for a6 itself to be
0x00FF0BEC, which it was at the moment of the measured write. a6 moves +4
per inner pass and +0xE8 per outer row, and the outer row step at 0x27FAC
never runs: the four-billion instruction run recorded the outer bound
executing zero times.

One thing that measurement does NOT say: a single inner pass costs only
**11 instructions** - but that is a pass where `tst.w d5 / ble` skipped the
worker entirely. Passes that do call the worker are a different size, and the
11 is not evidence that the loop is short.

## The bound is bypassed by a branch, not merely unreached

Recording every write to a6 through the fill:

```
  writing pc   old         new       delta
  0x00027f26  0xfffffc38 0xffff09ec   the seed, lea.l $FFFF09EC,a6
  0x00027f9e  0xffff09ec 0xffff09f0   0x4
  0x00027f9e  0xffff09f0 0xffff09f4   0x4
  ...
  0x00027f9e  0x00ff0bdc 0x00ff0be0   0x4
  0x00027f9e  0x00ff0be8 0x00ff0bec   0x4
```

Uniform +4 for four million nineteen thousand four hundred and thirty-four
steps, with no other instruction touching a6 at all. So the overrun is not a
miscomputation anywhere: a6 is simply allowed to walk, because the only other
instruction that could bound it never runs.

And it never runs because of the branch:

```
0x27F78  tst.w d5
0x27F7A  bne.b 0x27F9E        <- back to the +4, SKIPPING the DBF
0x27F7C  move.l #$22222222,$FFFF0006   (the "finished" marker)
...
0x27F9E  addq.l #4,a6
0x27FA0  subq.w #1,d5
0x27FA2  sub.l a3,d3
0x27FA4  adda.l a3,a1
0x27FA6  adda.l d6,a2
0x27FA8  dbf.l d0,0x27F44
0x27FAC  adda.l #0xE8,a6      <- the row step, also skipped
0x27FB2  dbf.l d4,0x27F3E
```

`bne.b 0x27F9E` jumps to 0x27F9E and so steps over both `dbf.l d0` and
`adda.l #0xE8,a6`. While d5 is non-zero the only bound that survives is
`subq.w #1,d5`. That is the whole bug surface left: the loop's shape says
d5 is a flag and d0 is the count, and the fast path uses the flag.

Also settled this pass: the machine reaches 0xBBE09BF4 - the $BBBBBBBB
marker read as an address - by 400 million instructions, having made only
eight complete passes. It is not a slow walk into a bad state; the sweep
overwrites the return address within the first few outer rows and execution
leaves the cartridge almost immediately.

## The fill's shape, and why nothing is broken

Sampling d5 at 0x27F78 - the tst.w on the *fast path*, not the one at the
loop head - settles it:

```
arrivals at 0x27F78: 4194432
  first few d5     : [12582912, 12648447, 12648446, 12648445, 12648444]
  low word hits 0  : [(697251, 12582912), (1061612, 12582912), (1423230, 12582912), ...]
```

$00C0FFFF, $00C00000 - the countdown - then straight back to $00C00000, which
is the RAM-clear loop's own count. So the fill is **64 rows of 65,535
columns**, and every number agrees:

* 64 x 65,535 x 4 = $1000000, which is exactly how far a6 ends from its
  $FFFF09EC seed ($00FF0BEC);
* 4,194,432 arrivals, matching the pass count;
* each row costs about 365,000 instructions and there are 64 of them.

Nothing here is miscomputed. The cartridge really does walk a6 sixteen
megabytes, and the emulator faithfully walks it.

Earlier today I "disproved" the 64-wrap theory by sampling d5 at 0x27F4C,
the loop *head*, where the reload has already happened. There are two
tst.w d5 in this loop and the fast path uses the other one. The theory was
right and the experiment was wrong - the same shape as every other false
start today, one level deeper: not a number read in the wrong place but a
register read at the wrong *address*.

## The decisive experiment still unrun

Genesis Plus GX boots this cartridge and shows the EA logo at frame 400.
Nothing in a four-billion-instruction mdcore run or a four-hundred-million
instruction pc trace reproduces that. The oracle therefore can answer the one
question static reading cannot: **does real hardware also leave the cartridge
at $BBE09BF4?** It does not, so the divergence is ours somewhere - and the
fastest way to find it is to compare the work-RAM image at a fixed frame
count rather than to keep reading assembly.

## Against the oracle: the first divergence is the first byte of work RAM

tools/oracle_gpgx.py grew a --ram flag that dumps the core's 64 KiB of 68k
work RAM through retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM). At frame
400 - where the oracle shows the EA logo - it holds 6,990 nonzero bytes and
its marker at $FFFF0006 is $33333333. Ours never gets past $BBBBBBBB.

Then diff the two images:

```
marker history (first 8): [(697201, 0xbbbbbbbb), (697208, 0xeeeeeeee),
                           (697252, 0x22222222), (697303, 0xbbbbbbbb), ...]
final marker: 0xbbbbbbbb   oracle: 0x33333333
work RAM differs in 49388 of 65536 bytes, first at Some(0)
  ours   $000000: bbbbbbbb2222bbbb
  oracle $000000: ffff000043003333
```

Two facts fall out, and the second is the interesting one.

* $22222222 is set at instruction 697,252 - the `d5 == 1` case at 0x27F56 -
  and $BBBBBBBB returns 51 instructions later. So the "finished" marker is
  not the end of the fill at all; it fires on the first row and the loop keeps
  going. A future session should not read $2222 as completion.

* **The very first byte of work RAM already differs.** The oracle has
  $0000FFFF there - the sentinel the game's own init writes - and we have
  $BBBBBBBB. But the sweep cannot reach it: a6 = $FFFF09EC is the lowest
  base it ever has, and d4.w >= 0, so the lowest address the sweep can form
  is $FFFF09EC, which is offset $9EC. Offset zero is not reachable by
  `move.l ($FFFF0006),(d4.w,a6)` under any value of d4.w.

So something other than the sweep is writing $BBBBBBBB over offset zero, and
the sweep is a bystander. The next measurement is the same one that worked
for the stack: watch the *write* to work RAM offset 0 with the writing PC,
filtered at the bus rather than inferred afterwards.

## Who writes work RAM offset zero, and the four instrument bugs that hid it

Chasing one byte took five wrong instruments and then one right one. All of
it is worth keeping, because every wrong instrument said "nothing wrote it"
with total confidence.

1. Watched `a6 + d4.w` in a register probe. At boot a6 and d4 are zero, so
   every instruction "matched" and the first ten records were the reset.
2. Hooked `write8` only. The sweep stores a longword, so a byte watch sees
   nothing and looks like silence.
3. Hooked `write16` at `(a & 0xFFFFFF) < 4`. Work RAM is selected by
   A23-A19, so `$FFFF0000` masks to `0x00FF0000` - `< 4` watches the wrong
   end of the map entirely. Same shape as the select-line bisection trap.
4. Capped the record buffer at 32. The boot RAM clear writes offset zero
   first, so the buffer fills before the write that matters. This is the
   same cap trap that hid the stack write earlier today.
5. Test the address rather than the FOLDED offset. `fold_to_wram` masks with
   `0x003FFFFF`, so `$00FF0000`, `$00BF0000` and `$00FA0000` all alias onto
   offset zero. Watching the address catches one alias out of three.

With the folded offset watched:

```
  0x00027dfc $FF0000 0x0000     the boot RAM clear
  0x00027dfc $FF0002 0x0000
  0x00027cfc $FA0000 0xbbbb     the fill sweep
  0x00027cfc $FA0002 0xbbbb
```

**So the sweep does write offset zero, and it is not a bug in us.**
`$E00000-$FFFFFF` is the 64 KB work RAM mirrored across 8 MB on a real Mega
Drive, exactly as Sega's memory map documents, so `$FA0000` folding onto
offset zero is correct hardware behaviour that Genesis Plus GX reproduces.
The instruction is `move.l ($FFFF0006),(d4.w,a6)` at 0x27CFC - the marker
sweep - and it reaches offset 0 because the address it computes has walked
out of the canonical `$FF0000-$FFFFFF` window.

The remaining question is therefore unchanged and narrower than it looked:
why does a6 walk so far here, when on the oracle the same sweep never
produces $BBBBBBBB at offset 0. The instrument to answer it next is the
oracle's - record a6 at 0x27CFC in both cores and find the first
instruction where they differ, rather than another work-RAM diff.

## Pinned to a single event: a fill-row reload at 41.7M instructions

Diffing our work RAM against the oracle's at a series of instruction
budgets, against the oracle's frame 60:

```
  @  30000000:   943 differ, first offset 6
  @  40000000:   943 differ
  @  41400000:   943 differ | pc=0x27f4e d5=0x00c085d3 a6=0x00dcf458 a7=0xffff9c10
  @  41500000:   943 differ | pc=0x27fa2 d5=0x00c0e50c a6=0x00dd7778 a7=0xffff9c10
  @  41600000:   943 differ | pc=0x27fa0 d5=0x00c0c18a a6=0x00de0584 a7=0xffff9c10
  @  41700000:   943 differ | pc=0x27f9e d5=0x00c09e07 a6=0x00de938c a7=0xffff9c10
  @  41800000:  1139 differ | pc=0x27d0e d5=0x00c00000 a6=0x00df0bac a7=0xffff9c0c
  @  41900000:  1139 differ | pc=0x27f78 d5=0x00c0dc7d a6=0x00df99b8 a7=0xffff9c10
```

Two things, and the first is the good news.

* **For the first 41.7 million instructions the two cores agree to within
  943 bytes**, and the first difference is offset 6, which is the marker
  whose value the oracle changes later. Every other byte of work RAM
  matches. Two independent implementations sharing no code producing the
  same 64 KB is about as strong a correctness signal as this project can
  get, and it says the 68000 core is essentially right.

* The divergence appears in the single step between 41.7M and 41.8M, and
  that step is exactly a **fill-row reload**: d5 goes from $00C09E07 down to
  the $00C00000 seed, a7 drops from $FFFF9C10 to $FFFF9C0C as the worker's
  frame comes off, and the PC is at 0x27D0E - the worker's `rts`. So the
  row reload is where we and Genesis Plus GX part company, not somewhere in
  the 64-row loop at large.

The next step is therefore small and concrete: find what sets d5 to
$00C00000 at the row boundary and check that one instruction against the
oracle. $00C00000 is the same constant as the boot RAM clear's longword
count, so the reload most likely comes from the same place, and it is read
from somewhere - a ROM table, a computed value, or a register we have wrong
by exactly the amount that turns a bounded row into a runaway.

## The MOVEM register list was printed backwards for predecrement

Chasing d5's reload to `movem.l d0-d7,(a7)+` at 0x27D0A led to the routine's
prologue at 0x27CC6, which the disassembler printed as `movem.l a0-a7,-(a7)`
- a save of the address registers against a restore of the data registers.
That looks like a mismatched frame, and a mismatched frame is exactly what
the whole stack-corruption hunt has been chasing.

It is not a mismatch. `format_register_list` mapped bit i to `d{i}` for
i < 8 and `a{i-8}` otherwise - the correct order for every MOVEM form except
predecrement, which GPGX indexes the other way round (REG_DA[15 - i]: bit 0
is A7, bit 15 is D0). So 0xFF00 was printed a0-a7 when it is d0-d7, the
routine's own frame, and the save and restore were always paired.

The corrected listing:

```
0x027CC4   rts
0x027CC6   movem.l d0-d7,-(a7)
0x027CCA   cmp.l d1,d2
0x027CCC   blt.b 0x27D0A
...
0x027D0A   movem.l d0-d7,(a7)+
0x027D0E   rts
```

0x27CC4 is a second entry point that returns immediately without touching
the frame, and the `blt` to the epilogue leaves save and restore paired on
the path that skips the loop body. The routine is sound.

The corrected fix folds the encoding mask into REGISTER space (bit i ->
15 - i for predecrement) and prints that ascending. Reversing the mask
instead flips the output order as well, which is wrong: the list still
prints d0..d7/a0..a7, it is just that predecrement bit 0 names A7. That
first attempt failed its own tests and a second failed them again in the
other direction, both because the expectations were hand-computed - which is
the same trap as hand-decoding hex. The confirmation that matters is the
pairing itself:

```
0x027CC6   movem.l d0-d7,-(a7)
0x027CCA   cmp.l d1,d2
0x027D0A   movem.l d0-d7,(a7)+
0x027D0E   rts
```

The core was right throughout - the opcode audit passes, and the 41.7M
instructions of work-RAM agreement with Genesis Plus GX confirm it. Only the
listing lied, which is the recurring theme of this whole frontier and the
reason `mdx disasm` exists at all: it exists so a wrong reading is one
command away from being caught, and this time it was caught by checking the
printed register list against the ROM's byte pattern instead of against
expectation.

## The comparison was measuring the wrong thing: align on frames

Every previous diff compared a fixed INSTRUCTION budget against a fixed
oracle FRAME, which is two different moments. Genesis Plus GX's work RAM is
essentially empty until frame 120 - two nonzero bytes at frame 10, six at
frame 60 - while ours was already full at 41.7M instructions. The "48,768 of
65,536 bytes nonzero, spanning the whole 64 KB" figure is an artefact of that
mismatch and should not be trusted: it compares our 41.7 millionth
instruction against the oracle's frame 60.

Running each core to the same frame number:

```
frame  10: ours     2 nonzero, oracle     2 nonzero,     2 differ
frame  30: ours     2 nonzero, oracle     2 nonzero,     2 differ
frame  60: ours     2 nonzero, oracle     6 nonzero,     6 differ
frame 120: ours     2 nonzero, oracle  6294 nonzero,  6294 differ
frame 180: ours   936 nonzero, oracle  6895 nonzero,  6019 differ
frame 240: ours   936 nonzero, oracle  6989 nonzero,  6111 differ
frame 300: ours   941 nonzero, oracle  6989 nonzero,  6115 differ
frame 360: ours   941 nonzero, oracle  6990 nonzero,  6116 differ
```

This is a far better picture than the one it replaces.

* **The first thirty frames agree to within two bytes.** Two independent
  implementations producing the same work RAM for half a second of emulated
  time, including VDP timing and all.

* **The oracle's fill runs between frames 60 and 120; ours runs between 120
  and 180.** We are roughly a second late, not wrong.

* **The fill itself writes 941 bytes where the real one writes 6,990.** That
  is the remaining gap, stated in bytes rather than as a story about a runaway
  pointer, and it is about seven times too small.

So the frontier is now small and quantitative rather than large and
narrative: find why our fill stops after 941 bytes when the cartridge's
writes 6,990. Everything else - the core, the VDP, the timing, the RAM map,
the 941-byte agreement - is settled.

The lesson is the one this project keeps teaching, and it took an oracle to
teach it: align on the quantity the two systems agree on. Frame numbers are
comparable; instruction counts are a property of one implementation.

## Frame-aligned: our fill writes the SAME bytes, and is just short

Diffing our frame-360 work RAM against the oracle's byte by byte:

* **We agree exactly with the cartridge's own fill pattern.** At $000200
  both hold `00 00 00 00 44 44 44 44 55 55 55 55 00 ...`, and the $20-stride
  $44444444/$55555555 pattern lands on identical offsets. The sweep at
  0x27CFC is doing the same work in both cores, to the same addresses, with
  the same values.

* **The gap is 6,071 bytes the real run writes and we do not.** Of those,
  only 165 are below $000400, and those form a stride-6 table at the base of
  work RAM - $C8, $CE, $D4, $DA, $E0, $E6, ... each value stored twice - which
  is the game's own lookup table rather than anything the sweep produces.

* **Ours ends at $000300 or so.** The oracle's largest runs reach $00E5AF,
  about 58 KB up; ours stops within the first kilobyte.

So the sweep is correct and the shortfall is an extent problem, not a value
problem: our fill produces 941 bytes where the cartridge's produces 6,990,
and where both write, they write the same thing.

Two things follow. The runaway pointer is gone as a theory - with frames
aligned there is no runaway, only a short run, and the earlier 48,768-byte
figure was the alignment artefact it was. And the remaining work is bounded:
figure out why the fill stops after roughly the first kilobyte, which given
the 64-row x 65,535-column structure is about one row's worth of a6 being
processed before something stops it. The oracle's $00E5AF upper bound says
the real run stays inside 58 KB of work RAM, which is what a correctly
terminated sweep should look like.

## CORRECTION to the previous entry: the runaway is real, just later

The claim "with frames aligned there is no runaway pointer, only a short
run" was wrong, and the run to frame 12,000 is what shows it.

The sweep is not absent after the first burst - it runs in bursts of roughly
900 writes, one per row:

```
frame  241..242: sweep 657 + 325
frame  394..397: sweep 139 + 883 + 857 + 300
frame  473..476: sweep 377 + 882 + 857 + 663
frame  552..553: sweep 14 + 855
```

and then, somewhere before frame 12,000, both the loop head at 0x27F9E and
the sweep at 0x27CFC stop entirely. The final register state is the same one
that ended every earlier investigation:

```
final pc=0xbbe09bf4 d5=0x00c000fb d4=0x000281fe a6=0x00ff0bec a7=0xffff9c10
```

**pc = $BBE09BF4 is the marker forming an address again, and a6 is $00FF0BEC
again** - the same runaway pointer, the same return through $BBBBBBBB, the
same instruction. So the frame alignment corrected the measurement but not
the conclusion: a6 really does walk out of work RAM, the fill really does
overrun, and the 941-bytes-versus-6,990 figure was a symptom of a machine
that stopped early rather than evidence of a short fill.

What the frame alignment did establish, and what is solid:

* The first thirty frames agree with Genesis Plus GX to two bytes.
* Where both cores run the sweep, they write identical values at identical
  addresses, so the sweep instruction itself is right.
* The fill is 64 row-bursts of about 900 writes, and ours stops part way
  through.

The next question is therefore the original one after all: why does a6 reach
$00FF0BEC. The frame alignment removed a false reading, not a real bug.

## A real branch-base bug, found and fixed - and it is NOT the cause

Chasing the marker's reappearance found a genuine defect. In both the DBcc
and Bcc word forms the displacement was measured from `self.pc` AFTER
`next_word` had consumed the extension, so the base sat two bytes too high.
The code's own comment stated the correct rule and the code did the opposite
thing.

Genesis Plus GX spells the fix out in `m68ki_op_dbcc_16`:

```c
uint offset = OPER_I_16();   /* REG_PC += 2 */
REG_PC -= 2;                 /* back to the extension word's own address */
m68ki_branch_16(offset);
```

Fixed in `op_branch`'s word form, in the DBcc handler, and in
`pc_displacement` (which serves BRA.W). King's Bounty's inner fill loop shows
why it mattered: `dbf.l d0,$FF98` at 0x27FA8 has its extension word at
0x27FAA, so the target is 0x27FAA - 104 = **0x27F42**, the start of
`move.l #$BBBBBBBB,$FFFF0006`. The old base gives 0x27F44 - the MIDDLE of
that instruction, where `$BB $BB` decodes as `BSR.L` and leaves the cartridge.
Two regression tests run against the cartridge's own bytes: the DBcc one at
0x27FA8, and a real `BRA.W` at 0x027199 with displacement $BB33.

**But the boot result is unchanged**: still 4,194,433 executions of 0x27F42,
still `pc = 0xBBE09BF4`, still the marker. So this was a latent defect, real
and worth fixing - it would corrupt any DBcc or BRA.W whose target lands
mid-instruction - but it is not the runaway's cause. Saying otherwise would
be exactly the kind of confident claim this frontier has produced before.

What the count does establish, and it is worth having: **4,194,433 is
2^22 + 1**, the fill's natural completion count. The sweep runs to its end
and then the machine leaves the cartridge through the marker. The overrun is
not a failure to finish; it is what happens immediately after finishing.

Fourteen fixes to the core this project, every one of them confirmed against
Genesis Plus GX or a reference decoder before being applied. This is the
first that turned out to be real, worth keeping, and not the answer.

## The marker is a shared scratch slot, and there are TWO sibling fill loops

`$FFFF0006` is not a marker at all. At 0x00095E:

```
202f0008   move.l ($0008,sp),d0
48e70600   movem.w d0-d7,($0006).w
```

it is the game's general register-save area, and the fill copies whatever is
there into the destination array. That is why every marker value is one of
the game's own fill constants.

More usefully, there are TWO sibling loops sharing the worker at 0x27CC6,
with the same shape and different constants:

```
; the one we run, at 0x027F3E
0x027F42  move.l #$BBBBBBBB,$FFFF0006   ; running
0x027F56  move.l #$22222222,$FFFF0006   ; d5 == 1
0x027F66  move.l #$EEEEEEEE,$FFFF0006   ; d0 == 5
0x027F74  bsr.w 0x27CC6
...
0x027FA8  dbf.l d0,0x27F44
0x027FB2  dbf.l d4,0x27F3E
```

```
; the one the oracle ends in, at 0x027D7A
0x027D7A  movem.w d0-d2/d6/a1-a2/a4/a7,-(sp)
0x027D7E  add.l d7,d1
0x027D80  addq.w #1,d6
0x027D82  tst.w d5
0x027D84  ble.b 0x27DBC
0x027D86  cmp.w #1,d5
0x027D8A  bne.b 0x27D96
0x027D8C  move.l #$33333333,$FFFF0006   ; d5 == 1  <- the oracle's final value
0x027D96  move.l d3,d1
0x027D98  move.l a1,d2
0x027D9A  cmp.w #5,d0
0x027D9E  bne.b 0x27DB2
0x027DA0  move.l #$DDDDDDDD,$FFFF0006   ; d0 == 5
0x027DAA  cmp.w #8,d1
0x027DAE  bge.b 0x27DB2
0x027DB0  moveq #8,d1
0x027DB2  swap d1
0x027DB4  swap d2
0x027DB6  bsr.w 0x27CC6
```

Both store into $FFFF0006 at `d5 == 1`; one stores $22222222 and the other
$33333333. The oracle's final marker is $33333333, so the real run reaches
the 0x27D7A loop, and ours ends in the 0x027F3E loop. The two differ in one
more respect: the 0x27D7A loop has no `adda.l #0xE8,a6` row step and no
outer `dbf.l d4`, and it `swap`s both bounds before calling the worker
instead of passing them straight through.

So the frontier is not "why does the sweep overrun" at all. It is: **which
caller runs, and why does ours end in the 0x027F3E loop when the cartridge's
ends in the 0x027D7A one.** The overrun was a symptom of being in the wrong
loop. Every observation that looked like a runaway - a6 walking to
$00FF0BEC, the fill ending at 941 bytes, the marker becoming a return address
- follows from taking the 0x027F3E path instead of the 0x027D7A one.

## Measured: our loop execution is arithmetically exact

Register sampling at each `dbf`, rather than inference from a later value:

```
dbf at $27F10 #1: d0=0x0000023f
dbf at $27F10 #2: d0=0x0000023e
dbf at $27FA8 #1: d0=0x0002815b  d5=0x00c0ffff  a6=0xffff09f0
dbf at $27FA8 #2: d0=0x0002815a  d5=0x00c0fffe  a6=0xffff09f4
dbf at $27FA8 #3: d0=0x00028159  d5=0x00c0fffd  a6=0xffff09f8
totals: $27F10 x576, $27FA8 x4194432
```

The inner loop runs **4,194,432 = 2^22 iterations exactly**, d0 counting
164,187 down to zero, d5 counting $C0FFFF down to $C00000, a6 advancing
four bytes per iteration. Not one iteration too many, not one too few. The
16 MB walk is the cartridge's own arithmetic executing correctly, so "the
sweep runs off its buffer" has never been the right description of a
miscomputation - it is what this loop does when entered.

Two smaller facts fall out of the same trace:

* `dbf.l d0,0x27F10` executes 576 times, against `move.w #0x23F,d0` - the
  loop before it runs one more time than the counter's initial value, which
  is what `dbf` does (it exits when the counter reaches -1, so $23F gives
  $240 passes).
* **d4 is $0002817F at the inner `dbf`, not the 8 the preamble's `moveq
  #8,d4` set.** $2817E is the `lea.l 0x2817E,a4` pointer, so d4 has become
  a4 + 1 by then - an end pointer, not a count of eight. The outer
  `dbf.l d4,0x27F3E` therefore wants a very large number of outer passes,
  and 0x27F3E is reached exactly once, which means the outer loop exits
  without looping. That exit is now the single unexplained edge in the
  routine: the inner loop completes 2^22 iterations and then control does not
  come back through the outer `dbf`.

So the frontier has moved again, and to somewhere much smaller. Not "why
does the loop overrun" - it does not overrun, it completes. Not "why does the
branch base differ" - fixed and verified. The open question is the exit:
**after 4,194,432 iterations, why does control leave the outer `dbf.l d4`
rather than re-entering 0x27F3E**, given d4 is $2817F there.

## The inner loop never exits - it is one long pass, not a completed fill

Capturing the addresses executed at the 4,194,432nd execution of the inner
`dbf.l d0,0x27F44`:

```
$027FA8  dbf.l d0,0x27F44     d4=$000281FE  d0=$0002015A  d5=$00C07FFF  a6=$00FF0BEC
$027F42  move.l #$BBBBBBBB,$FFFF0006
$027F4C  tst.w d5
$027F4E  ble.b 0x27F78         ; d5 = $C07FFF, positive - the worker DOES run
$027F50  cmp.w #1,d5 / bne.b 0x27F60
$027F60  cmp.w #5,d0 / bne.b 0x27F70     ; d0 = $2015A, not 5
$027F70  move.l d3,d1 / move.l a1,d2
$027F74  bsr.w 0x27CC6         ; into the worker
$027CC6  ... $027CCA $027CCC $027CCE
```

This corrects the reading in the previous entry. `d0` is **$2015A, still
counting**, not zero: 0x2815B - 0x2015A = $8001. So the inner loop has not
run to completion - it is mid-pass, having taken the worker branch back
round again. 4,194,432 is not the fill's trip count, it is simply where this
probe's budget happened to land, and `dbf.l d4,0x27F3E` and `dbf.l d0` are
both still inside one long iteration of the outer loop.

Two consequences worth keeping:

* **a6 = $00FF0BEC by this point, measured** - the 16 MB walk is real and is
  93.75% of the way through its range. a6 = $FFFF09EC plus 4 bytes times
  4,155,552 passes. Nothing here is a miscomputation; the cartridge walks its
  base pointer across the 8 MB work-RAM mirror, and every one of those writes
  is a correct hardware alias.
* **$00FF9C0C really is work RAM.** The 24-bit address is $FF9C0C, so A23 is
  set, so the Mega Drive's own map puts it in the 64 KB RAM - and our decode
  `addr & 0x00E0_0000 == 0x00E0_0000` agrees. The zeroed high byte is a
  32-bit-value artefact, not a second address. That one is settled and should
  not be reopened.

So the remaining question is unchanged and now cleanly stated: **the real
run reaches the 0x027D7A loop, ours stays in the 0x027F3E loop, and the
difference is decided before either of them is entered.**
