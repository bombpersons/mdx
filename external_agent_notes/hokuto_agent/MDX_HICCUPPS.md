# mdx tool hiccups and defects

Findings from driving the `mdx` MCP server (17 tools, v0.1.0) against
*Hokuto no Ken II* and against purpose-built synthetic cartridges.
Reproductions live in `fixtures/`.

Severity: **BLOCKER** = will mislead an analysis outright · **MAJOR** = costs
real time or hides coverage · **MINOR** = cosmetic / wording.

---

## 1. BLOCKER — word-displacement PC-relative branches are 2 bytes low

**Affects:** `disasm` (listing) *and* the CPU core (`boot`, `trace`).
Both the decoder and the executor share the same base calculation.

`target = instruction_address + 2 + displacement`

The 68000 rule is `target = address_of_extension_word + 2 + displacement`,
which for a 4-byte branch equals `instruction_address + 4 + displacement`.
`.S` (byte-displacement) branches come out right only by coincidence — for
them `address + 2` *is* the end of the instruction.

### Reproduction A — listing (`fixtures/branch_target_probe.md`)

`mdx_disasm(rom, from=0x200, count=6)`:

| Address | Bytes | mdx reports | Correct 68000 | Δ |
|---|---|---|---|---|
| `0x200` | `61 00 10 00` | `bsr.w 0x1202` | `0x1204` | −2 |
| `0x204` | `61 10` | `bsr.b 0x216` | `0x216` | ✔ |
| `0x206` | `60 00 20 00` | `bra.w 0x2208` | `0x220A` | −2 |
| `0x20A` | `60 20` | `bra.b 0x22C` | `0x22C` | ✔ |

### Reproduction B — execution (`fixtures/bsr_exec_probe.md`)

```
0x200  61 00 00 10   BSR.W +$10
0x212  4E 71         NOP              <- mdx lands here
0x214  20 40 00 01   MOVE.L #1,D0     <- architecturally correct target
```

`mdx_boot(rom, stop_at=[0x212])` returns:

```json
"stop_points": ["0x00000212"], "instructions": 0, "exit": "partial"
```

`a7` is still `0x00FFFE00`, so nothing executed past the stop. Running
unconstrained, `hottest_addresses` contains `0x00000212` — a PC that only the
branch target can reach — and **not** `0x00000214` as a branch origin.

The disassembler and the emulator agree with each other and are both wrong,
so `disasm` output *looks* self-consistent against `boot` traces. That makes
this worse than a listing-only bug: cross-checking the two tools will not
catch it.

### Consequence for this project

In the Hokuto init stub, `BNE.W` at `0x21E` (`66 00 00 3A`) is listed as
`0x25A`; the real target is `0x25C`, which lands **inside** the following
`MOVE.W #$8134,$C00004`. Stock Sega init code should not branch into the
middle of an instruction, which is worth double-checking against a known-good
emulator before trusting this ROM's control flow.

### Suggested fix

Use the instruction length (or the end of the extension word) as the base for
the displacement, and add `BRA`/`BSR`/`Bcc`/`DBcc` word-displacement cases to
the bundled `testdata/cpu-regressions.json` — note it currently ships `DBcc`
fixtures but nothing asserts a taken `DBcc` target.

---

## 2. BLOCKER — `disasm.from` is a CPU bus address but returns no `cart_offset`

**Affects:** `disasm`.

On a Mega Drive the cartridge is mapped at `0x000000`, so cartridge offset N is
bus address N. I passed `from = 0x020200` (intending "offset 0x200") assuming a
`0x200000` window. mdx happily disassembled cart offset `0x020200`, returned
`coverage.status: "complete"`, **no warning**, and produced plausible-looking
`bsr.w`/`ori.b` garbage.

I initially concluded from that output that **the ROM was encrypted**. It is
not — it is a completely standard, unencrypted Sega third-party init stub. The
`disasm` response carries `from` (bus) but no `cart_offset`, unlike `boot`,
whose `first_addresses` entries do carry both. That asymmetry is the trap.

**Suggestion:** echo `cart_offset` in the `disasm` response, and state the
window assumption in the coverage conditions (e.g. `cartridge window 0x000000`).

---

## 3. MAJOR — `header` reports only vectors 0–15, hiding the one that matters

**Affects:** `header`.

`coverage.conditions` says `exception_vectors[0..16]`, so it is explicit, but
the cut is wrong for reverse engineering. The 68000 vector table is 64 entries;
indices 16–47 are the auto-vectors and external IRQs. For a Mega Drive game the
only vector that normally exists is the **VBlank auto-vector, index 30
(`0x78`)**.

For this cartridge `header` reported all 16 visible vectors as `0x00000200`,
which reads as "the game has no interrupt handlers at all". In fact the real
VBlank vector at index 30 is `0x00000DB4`, and the game installs no other.
Working out that the game was alive required a manual read of ROM bytes
`0x80–0xFF`.

**Suggestion:** report all 64 entries, or add an explicit
`autovector_start`/count argument.

Related: the reported names for indices 12–15 are
`uninitialized_interrupt`, `vector_12`, `vector_14`, `trace`, which do not
correspond to the 68000's auto-vector numbering.

---

## 4. MAJOR — `cpu_conformance` needs a network clone even though fixtures ship in-tree

**Affects:** `cpu_conformance`.

```
$ mdx_cpu_conformance(suite=".../testdata/cpu-regressions.json", filter="BRA")
→ suite_not_found: "no v1 directory under ..."
   remediation: git clone --depth 1 https://github.com/SingleStepTests/680x0.git
```

`crates/mdx/testdata/cpu-regressions.json` already contains 12 pinned
SingleStepTests cases (LEA, MOVEM.w, MOVEM.l, **DBcc**), with `file_sha256`
recorded per source file — but the operation only accepts a cloned upstream
suite directory and will not read the bundled JSON. So the one class of test
that would have caught finding #1 cannot be run offline, and the bundled
DBcc fixtures are apparently only used by the unit tests.

**Suggestion:** let `suite` accept the bundled regression JSON.

---

## 5. MAJOR — `boot` responses are very large and mostly duplicate

**Affects:** `boot`.

A 1100-frame run produced ~20 KB of JSON, dominated by:

* `last_addresses` — 64 records, nearly all the same 2-instruction poll loop
* `unmapped_access_samples` — 64 records, 64 near-identical repeated tuples
* `io_trace` — up to 512 strings
* `vdp.fm_reg_writes`, `z80_port_counts` — 128 mostly-zero entries

`trace: 0` only clears `first_addresses`; there is no way to suppress the other
streams. `artifact_view` can read them back in pages, but the initial response
already arrives in full.

**Suggestion:** a verbosity/stream selector, or omit these by default and
expose them only through `artifact_view`.

---

## 6. MAJOR — `unmapped_accesses` flags legitimate Z80-window registers

**Affects:** `boot`.

The game writes to `$A04000` and `$A11100`. On real Mega Drive hardware
`$A00000–$A01FFF` is 8 KiB Z80 RAM, `$A02000+` the YM2612, and `$A04000` the
**Z80 bank register**; `$A11100` is the Z80 bus latch in the I/O space.

mdx counts these as unmapped bus accesses — 21,467 of them in a 1100-frame run,
versus 144 in a 60-frame run. That number is misleading as a health signal, and
if the access is genuinely dropped the bank register is not modelled.

**Suggestion:** model the Z80 bank register, or at least exclude the documented
Z80 window from `unmapped`.

---

## 7. MINOR — `boot` mislabels termination when it stops at a `stop_at` point

**Affects:** `boot`.

`stop_at: [0x212]` stopped correctly at the very first instruction
(`"instructions": 0`), yet the response reports:

```json
"termination": "frame_target", "coverage": {"status": "partial"}, "ok": false
```

with the limitation *"requested frame target was not reached within the
instruction budget/stop points"*. Reaching a stop point is success, not a
partial failure, and `termination` should distinguish it from a frame target.
Note the `stop_points` field itself is correct and useful.

Also `frames: 0` yields `"conditions": ["instructions[0..0]", ...,
"frames[0..0]"]` and the same misleading `"frame_target"`.

---

## 8. MINOR — input-replay schema rejects any extra key

**Affects:** `boot`, `trace`, `watch` (`input` argument).

```json
{"_comment": "why these frames", "events": [...]}
→ input_replay_invalid: unknown field `_comment`, expected `events`
```

`additionalProperties: false`, so there is nowhere to annotate a replay file.
The error message itself is good — it carries a `remediation` string naming the
legal buttons — but the replay shape is not exposed in the paginated schema
catalogue, so the first attempt is guesswork.

**Suggestion:** allow an ignored `_comment`/`"note"` key, or publish the
replay schema.

---

## 9. MINOR — `graphics` reports a `plane b` base that matches no VDP register

**Affects:** `graphics`.

With `registers[2] = 48` (`0x30`) and `registers[3] = 44` (`0x2C`):

| mdx reports | address | corresponds to |
|---|---|---|
| `plane a` | `0xC000` | reg 2 ✔ |
| `window` | `0xB000` | reg 3 ✔ (reg 17 = 0, so the window is full-screen) |
| `plane b` | `0xE000` | **no register** |

There is no separate "plane B base" on a Mega Drive — Plane B's name table *is*
reg 3. `0xE000` looks synthesised (reg 3 + `0x3000`). Not harmful, but a
consumer building a VRAM map from `graphics` would allocate a phantom plane.

**Suggestion:** derive `plane b` from reg 3 (or omit it when the window covers
the screen), and say which register produced each base.

---

## 10. MINOR — MCP request cancelled if stdin reaches EOF mid-call

Not an mdx defect, but worth recording for anyone scripting it: piping a fixed
set of JSON-RPC frames into `mdx mcp` and closing stdin caused
`initialize` to return and then `tools/list` to be cancelled
(`mdx MCP: Cancelled`, exit 3) rather than answered. Holding stdin open until
the response arrives works. My own client hit a separate bug here first
(`list_tools` skipped session start) — worth knowing the server is sensitive to
stdin lifetime.

---

## Things that worked well

Worth recording, since the brief was to stress the tool:

* **`watch` is the strongest tool here.** Watching one WRAM byte (`$00F0A0`)
  immediately produced a per-frame table with writer PC, frame, width and
  registers, which turned "the game appears hung" into "the VBlank handler
  clears this flag every frame". Folded-alias handling, same-value writes and
  all three widths all worked as documented.
* **`graphics` + `render` + `boot` reproduce this cartridge faithfully.** The
  68k runs, Mode 5 tilemaps and Japanese text render correctly, palettes are
  exact, and the game reaches full gameplay — 1621 of 2048 tiles, 7 linked
  sprites, 32 screen colours. For a pixel-perfect port that is the bulk of the
  work, and it came out of the tool.
* **`stop_at` is precise** — it halted before executing the target instruction.
* **Coverage honesty is good.** `observed/retained/omitted` counts were present
  everywhere, `exit: "partial"` was used rather than silently claiming success,
  and the `graphics` limitation about tile index 0 being drawn opaque is
  exactly the caveat a porter needs.
* **Typed failures are genuinely useful** — `input_replay_invalid` and
  `suite_not_found` both named the exact offending field or missing layout.
* **Deterministic**: repeated boots with identical arguments produced identical
  artifact digests.

---

## Summary table

| # | Sev | Tool | Issue |
|---|---|---|---|
| 1 | BLOCKER | `disasm` + CPU | word-displacement branch targets 2 bytes low |
| 2 | BLOCKER | `disasm` | bus/cartridge ambiguity, no `cart_offset` returned |
| 3 | MAJOR | `header` | only vectors 0–15; VBlank autovector invisible |
| 4 | MAJOR | `cpu_conformance` | needs network clone; bundled fixtures unusable |
| 5 | MAJOR | `boot` | responses dominated by duplicate observation arrays |
| 6 | MAJOR | `boot` | Z80-window registers counted as unmapped |
| 7 | MINOR | `boot` | `stop_at` success reported as `partial` / `frame_target` |
| 8 | MINOR | `boot`/`trace`/`watch` | replay schema rejects any extra key |
| 9 | MINOR | `graphics` | `plane b` base matches no VDP register |
| 10 | MINOR | MCP | in-flight request cancelled on stdin EOF |
---

# 11. `ram_dump.range` silently mis-addresses full CPU aliases — Medium

`range` is documented as `[CPU address, byte count]`. Folded offsets work; a
full CPU alias does not, and the result carries no error or warning.

| `range[0]` requested | returned `address` | returned `wram_offset` | correct? |
|---|---|---|---|
| `0xE21C` (folded) | `0x00FFE21C` | `0xE21C` | yes |
| `0xFFE21C` (alias) | `0x00FF001C` | `0x001C` | **no** |
| `0x0000E01C` | `0x00FFC91C` | `0xC91C` | **no** |

`coverage.status` is `complete` and no warning is emitted, so a caller that
trusts `wram_offset` reads a different variable than it asked for. `memory_read`
advertises that WRAM "accepts folded offsets or full CPU aliases" — `ram_dump`
should either do the same or reject non-folded input explicitly.

**Workaround:** pass folded offsets to `ram_dump.range`, or use `memory_read`
with a snapshot id.

---

# 12. `graphics` derives sprite-table and h-scroll bases from the wrong registers — Medium

`layout-*.json` for the gameplay snapshot reports, with `registers`
`reg4=7, reg5=84, reg13=43, reg22=248`:

```
sprite_table_base      = 43008 = 0xA800
horizontal_scroll_base = 44032 = 0xAC00
```

Neither matches the Mega Drive register definitions:

* **Sprite attribute table** is `(reg22 & 0x7F) << 9` = `120 << 9` = **`0xF000`**.
* **H-scroll table** is `reg13 * 0x200` = `43 * 0x200` = **`0x5600`**
  (reg 2/3/4/5/23 use unit `0x400`, reg 13 uses `0x200`).

`0xA800` is `reg5 * 0x200` and `0xAC00` is `reg13 * 0x400` — two off-by-a-unit
lookups. Consequences: `layout.sprites[]` is decoded from the wrong memory
(it reports 80 entries at `x=-128,y=-128` for the story screen), and any sprite
address derived from the export is wrong.

Ground-truth check against the VRAM capture: `0xA800` does **not** hold sprite
attributes. It holds a **graphics job table** of 8-byte records
`[w0][w1][w2][dst]`; the 4th word is a VRAM destination address that advances
`0x00B8 -> 0x0138` when the player walks, i.e. it is the tile-upload queue, and
it lines up with the `A6 = 0x15860` staging cursor observed in the CPU trace.
`0xF000`, the address the hardware formula actually gives, holds raw 4bpp pixel
data (`0x4444`, `0x4414`, `0x1414` repeats), not sprite records.

**Recommendation:** mdx should compute the sprite table from reg 22 and use
`0x200` as reg 13's unit; if a region contains neither sprite-shaped records
nor the expected table, report that instead of emitting 80 zero sprites.

---

# Source-read fallbacks

The mdx checkout is read-only and must not be modified. Ideally nothing in it
needs to be read to do this work — the MCP surface should be enough. This is
the running log of occasions where I did have to open it.

| # | When | Question | What I read | Why no tool answered it |
|---|---|---|---|---|
| 1 | Section 11 of `NOTES.md`, while mapping the entity loop at `0x78D6` | Is `MOVE.L $0000E01C,D0` reading cartridge or work RAM? | `crates/mdcore/src/memmap.rs` (`region()`, `fold_to_wram()`, `fold_to_cart_mirrored()`) | `disasm` gives the encoded address but labels no region for it; `boot` only reports a region per *PC*, not per data address. `memory_read` validates its region but needs a snapshot, and `ram_dump.range` mishandles aliases (finding 11). |

Follow-up on the same question, done through the tools only: `region()`
classifies by **address bits, not magnitude** — `addr & 0x00C0_0000 == 0` selects
cartridge, otherwise `addr & 0x00E0_0000 == 0x00E0_0000` selects work RAM. So
`$0000E01C` (A23/A22 both clear) is **cartridge**, and the instruction is reading
the ROM longword `0x303C0024`. The comment at the top of that file notes this
is deliberate ("an address the CPU drove with unusual high bits can still land
somewhere entirely different"). Worth surfacing from the tool side: `disasm`
should be able to annotate a referenced absolute address with its region.

---

# 13. `mdx_watch` is WRAM-only — Medium (capability gap, partly retracted)

**Correction:** I originally wrote this as "registers cannot be observed". That
was **wrong**, and it is the correction I would most like a future reader to
notice.

`mdx_trace` **does** record pre-instruction registers on every record — `a`,
`d`, `sr`, `pc`, `next_pc`, `cycles`, `prefetch`, `frame`, `step`. There is no
`trace_registers` flag because it is simply always on; passing it is rejected as
an unknown field. I spent a stretch believing a register watch was impossible
and worked around a blocker that did not exist.

The technique that actually works, and that I should have reached for first:

```
mdx_trace(start_pc=<any PC inside the code of interest>, start_frame=<just before the event>)
```

`start_pc` is a **"begin recording when execution reaches here"** gate, *not* a
PC filter — after the gate, every instruction is recorded. The returned
`records_artifact` is a JSONL file of the whole captured window. Computing a PC
histogram over it is the fastest way to find which routine is actually hot, and
it is how I identified `0x3668` as the live map builder after decoding the
adjacent, decoy-looking `0x36C0`.

The gap that **is** real: `mdx_watch` folds every range through
`mdcore::memmap::fold_to_wram`, so **VRAM writes cannot be watched at all** and
registers cannot be watched directly. For a Mega Drive cartridge the interesting
streaming target is usually VRAM. Requested fix: a `region` parameter
(`wram` | `vram` | `cram` | `vsram`) on `watch`.

# 14. `mdx_trace` `start_pc` semantics are undocumented — Low

The tool description calls `start_pc` an optional start gate but does not say
whether it *filters* records to that PC or *begins* recording there. They behave
very differently: filtering reads as "0 records, so that code never runs", when
in fact it means "you get nothing after the gate". Worth a sentence in the tool
docs.
