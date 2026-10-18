Ghidra adds what linear disasm lacks: function extents, cross-references, a
call graph, jump-table recovery and (as a hypothesis) C-like pseudocode. It is
optional. Install Ghidra 12 with a JDK 21 and set MDX_GHIDRA_HOME (or pass
ghidra_home); ghidra_doctor checks the setup. Without it every ghidra_* call
fails with ghidra_unavailable and the rest of mdx is unaffected.

Start from an observation, not the ROM: a watch writer PC, a trace PC or a
vector. ghidra_decompile takes rom plus addr and imports automatically (about
a minute the first time, then cached by ROM, state, evidence and options).
Reuse the returned analysis path for further functions, xrefs and decompiles.

What the import sets up:
- the 68000 language 68000:BE:32:default (a 68040 superset; Ghidra has no
  plain-68000 variant), the cartridge window at 0x000000 as read-only code,
  and blocks for Z80 RAM (0xA00000), the Z80 bus/YM2612, I/O (0xA10000), the
  VDP (0xC00000, volatile) and work RAM (0xFF0000, also mapped at 0xFFFF0000
  so `($8000).w` references resolve). Addresses you pass and receive fold the
  same way: 0xFFFF8000, 0xE08000 and 0xFF8000 are one byte.
- the vector table and header as labelled data, and functions at Reset and at
  every exception handler (HBlank, VBlank, ExtInt, traps); a handler shared by
  three or more vectors is DefaultException.
- cartridge banks: the reset-time mapping by default. ROMs over 4 MiB
  (SSF2-style mappers) need bank SLOT=PAGE or a from_state captured while the
  code of interest was mapped.

Runtime evidence is the differentiator. Best is trace with coverage:true
(and max_records 0): it records every distinct PC and control transfer of the
whole bounded run, so a long run through gameplay costs one small artifact;
pass its coverage_artifact. Plain trace/watch records artifacts and boot
results also work, but per-step records are mostly wait loops. Every executed PC is disassembled;
observed targets of indirect `jmp`/`jsr` (e.g. `jmp (pc,d0.w)` tables, which
Ghidra often misses) become references, and two or more targets from one
jump become a decompiler switch. evidence_coverage then says how many
executed PCs lie inside analysed functions; outside_functions_sample lists
code Ghidra still did not attribute. Evidence covers executed paths only.

Base registers: compiled games often keep an address register constant (King's
Bounty holds A5=0x312: `jsr N(a5)` calls a jump table and globals sit at
negative A5 offsets). 68000 trace evidence reports register_hints (a register
holding one nonzero value in at least 90% of steps). Confirm it in the
listing, then pass register A5=312: Ghidra then resolves those calls and
globals, which turns unaff_A5 arithmetic into named work-RAM addresses.

Polled RAM: work RAM is ordinary memory to Ghidra, so a wait loop on a
counter that the VBlank handler changes (`move.l d0,FFE4EE` then
`tst.l FFE4EE; bne`) decompiles as an infinite loop. Pass volatile
ADDR[:LEN] (hex, e.g. FFE4EE:4) for such addresses; watch shows which PCs
write them. The read then survives in pseudocode. Hardware ports are
already volatile.

RAM-resident code: give from_state (a state captured after the code was
copied); work RAM is imported from it, and RAM PCs from evidence, extra
ranges (LO-HI) and RAM vector targets become code. Without a state, RAM PCs
are ignored with a warning.

Z80 sound drivers: cpu z80 with z80_rom_offset (and z80_origin/z80_size) for
the driver bytes in the ROM, or from_state for the uploaded Z80 RAM. Language
z80:LE:16:default; functions at 0x0000 and the 0x0038 interrupt; YM2612, bank
register and PSG are labelled. Z80 traces (trace cpu z80) are evidence.

Reading results. Most Mega Drive games are hand-written assembly, so the
pseudocode is a hypothesis: routines take arguments in arbitrary registers,
return status in CCR flags (Ghidra shows these as unexplained flag tests),
share tails or fall through into each other (one routine can appear as part
of another), and use data tables inline with code. decompile returns the
instruction listing beside the pseudocode; trust the listing, and confirm
behaviour with trace or watch. Pseudocode is most useful for C-compiled games.

Recording findings: ghidra_annotate renames functions and adds comments as a
new revision (functions inline or a JSON file). ghidra_export_symbols writes
.symbols.json with names bound to each body's bytes; pass it as symbols to a
later analysis (other evidence, another state) and names whose bytes differ
are withheld. Plain "ADDR NAME" files are accepted unverified.

Projects: Ghidra rejects paths with hidden components, so its database lives
in <project_root>/ghidra-projects (or MDX_GHIDRA_PROJECTS); manifests and
results live under .mdx/ghidra. Same-analysis queries queue within their
deadline; a project open in the Ghidra GUI is reported busy. Never delete
another query's lock to make a call succeed.
