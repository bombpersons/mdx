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
listing, then pass register A5=312: globals at A5 offsets become work-RAM
addresses, and each `jsr d16(An)`/`jmp d16(An)` is resolved to An+d16. A
`jmp abs.l` stub there becomes a thunk of its target, so pseudocode reads
`Random(0x5a)` rather than `(*_DAT_00000312)(0x5a)`. base_register_calls
counts resolved sites and lists rejected ones: odd or non-code targets, which
are table data rather than routines and never become functions.

MacsBug names: games built with a Mac C toolchain (King's Bounty) keep each
routine's name after its rts as a length byte 0x80|n and the name. Pass
macsbug: true to name functions from them; an analysis without it warns
when it finds such names. Explicit symbols override them.

Polled RAM: work RAM is ordinary memory to Ghidra, so a wait loop on a
counter that the VBlank handler changes (`move.l d0,FFE4EE` then
`tst.l FFE4EE; bne`) decompiles as an infinite loop. Pass volatile
ADDR[:LEN] (hex, e.g. FFE4EE:4) for such addresses, or let evidence find
them: watch work RAM over a few frames (watch address 0, length 0x10000)
and pass its records_artifact as evidence. Writes with in_interrupt true
(made between an interrupt's entry and its rte) mark their bytes volatile;
writes near the handler's stack pointer are taken as its stack frame and
skipped. provenance.evidence.volatile_from_evidence lists each address with
its length, writer_pc and write count. The read then survives in
pseudocode. Hardware ports are already volatile. A from_state captured
inside a handler starts outside it, so its first writes read false.

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
pseudocode_repairs counts two edits mdx makes to Ghidra's text: a volatile
read Ghidra prints twice for one instruction (tst sets N and Z from two
loads) keeps only the used copy, and an address formed register-relative
(lea -1254(a5),a0) in pointer arithmetic reads (int)&DAT_fffffe2c instead of
-0x1d4.

Sweeps: ghidra_decompile_batch takes addrs (up to 200) and decompiles them in
one Ghidra run, returning one result; results are cached per function, so a
repeat costs nothing. It is brief by default, as is ghidra_decompile with
brief: pseudocode and callers/callees grouped per function ({to, to_name,
count, sites}) without the listing, mapping and provenance, which keeps large
functions out of result_artifact. brief:false gives one row per reference.

Recording findings: ghidra_annotate renames functions and adds comments as a
new revision (functions inline or a JSON file). labels:[{addr,name,size,comment}]
names data: RAM globals, ROM tables, SRAM. Give size 1/2/4 so the address
becomes byte/word/dword data; without it a long access reads `_name`. A size
the code disagrees with (size 2, read by move.b) prints name._0_1_ and is
listed in label_size_mismatches with the accessing sites. A work RAM name
also covers its absolute-short alias. ghidra_export_symbols writes
.symbols.json with function names bound to each body's bytes; pass it as
symbols to a later analysis (other evidence, another state) and names whose
bytes differ are withheld. Plain "ADDR NAME [function|data [SIZE]]" files are
accepted unverified; lines without a kind are functions inside code and data
elsewhere (e.g. "FFE800 player_x data 2").

Projects: Ghidra rejects paths with hidden components, so its database lives
in <project_root>/ghidra-projects (or MDX_GHIDRA_PROJECTS); manifests and
results live under .mdx/ghidra. Same-analysis queries queue within their
deadline; a project open in the Ghidra GUI is reported busy. Never delete
another query's lock to make a call succeed.
