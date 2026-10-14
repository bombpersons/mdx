# Open question: mdcore's MOVE size field is read from the wrong bits

`op_move` in `crates/mdcore/src/cpu/exec.rs` does:

```rust
let size = match (ir >> 12) & 3 { 1 => Byte, 2 => Long, _ => Word };
```

On the 0x3xxx MOVE line `(ir >> 12) & 3` is **always 3**, so every MOVE in
every cartridge decodes as a word move and this branch can never select
byte or long. The real 68000 field is bits 11-10, where 1 = byte,
2 = long, 3 = word.

Evidence this is a real bug, not a quirk of the cartridge:
- The cartridge's 0x3xxx opcodes span the whole 0x3000-0x3FFF range,
  including prefixes 0x3800 (192 uses), 0x3E00 (181), 0x3A00 (437) and
  0x3EC0 (162). Under a correct 2-bit size field those decode to a
  spread of byte/word/long moves.
- `0x3FC0` appears 146 times; under the current code that is a word move
  with a register destination and an extension word that is never read.

Why it may still have gone unnoticed: the game's boot path may only
exercise word moves, and a word move where a long was intended usually
still "runs" — it just writes two bytes instead of four. That is the
same class of silent failure as the shift group.

Do NOT change it casually. It was validated by scoring 2,825 executed
MOVE/MOVEA byte lengths against a reference decoder, which measures
lengths and not sizes - so the validation cannot speak to this.

How to settle it: disassemble a known-long move in a cartridge that
certainly uses one (Earthworm Jim is the largest at 3 MiB), compare the
bytes our decoder claims against what the ROM's own data layout requires,
or ask Genesis Plus GX for its VDP/DMA-relevant behaviour.

## Addendum, later the same session: the BSR.W mask was hiding this

`op_branch` matched `ir & 0xFF == 0xFF` to decide BSR.W. That swallows
every `0x6xFF` — which are ordinary byte branches with a displacement of
-1 — and turns them into subroutine calls. A `bne.s -1` is an infinite
loop; the cartridge has `bne.s -1` patterns, and we were returning from
them.

The fix splits the three forms the way Genesis Plus GX's jump table does:
each condition owns sixteen consecutive opcodes and only the one with a
zero low byte is the word form.

```
  0x6000                       BRA.W
  0x6100 / 0x61xx               BSR.W / BSR.B
  0x6c00 + 0x0000 displacement Bcc.W
  everything else              Bcc.B, displacement in the low byte
```

**Bit 8 is not the discriminator.** I "fixed" this to read bit 8 — the
first plausible-looking theory — and King's Bounty immediately stopped
booting, because `0x6708` is `beq.b $0008`, not `beq.w` plus the word
0x0378 that follows it. The cartridge has 55 such word-looking branches
in the first 8 KiB alone.

This is the fourth time this week an intuition about 68000 encoding lost
to a reference. The rule that keeps working: read the reference's dispatch
table, not the manual's summary.
