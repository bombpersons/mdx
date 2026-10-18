# mdx — guide for agents

This is the detailed, technical companion to [README.md](README.md). It is written
for coding/analysis agents (and humans who want the full picture) working *on*
mdx or *with* it. For the exhaustive API contract see [docs/mdx.md](docs/mdx.md).

## What mdx is

mdx is a Sega Mega Drive / Genesis reverse-engineering toolkit built around an
accurate emulation core. It has two crates:

| Crate | Role |
|---|---|
| `crates/mdcore` | The machine: 68000, Z80, VDP (with scanline renderer), YM2612 + SN76489 audio, I/O, cartridge hardware, SVP, save states, observation hooks. No I/O, no CLI. |
| `crates/mdx` | The toolkit: one typed operation registry exposed identically as a CLI (`mdx ...`, `mdx call OP --args JSON`) and an MCP server over stdio (`mdx mcp`). Content-addressed artifacts, JSON envelopes, schemas. |

Design goals, in priority order:

1. **Evidence, not vibes.** Every response is a JSON envelope with `coverage`,
   `limitations`, explicit bounds and omission counts. Nothing is silently
   truncated; large results spill to immutable SHA-256 artifacts.
2. **Fidelity to an independent reference.** The emulation target is the
   Genesis Plus GX (GPGX) libretro core, used as an *oracle*. mdcore is not a
   copy of GPGX, but where behaviour is observable it is ported to match GPGX
   exactly (timing, open bus, quirks, even GPGX-specific artifacts).
3. **Agent-first ergonomics.** Stateless calls, per-call `project_root`,
   machine save states for resuming, schemas discoverable without a ROM.

A sister project for the PlayStation 1 is
[bombpersons/psx](https://github.com/bombpersons/psx).

## Build and test

```sh
cargo build -p mdx --release --locked          # Rust 1.88+, C++14 compiler (YM2612)
cargo test --workspace --release --locked      # default tests: no ROMs, no oracle
python3 -B tools/test_observers.py             # observer hook test (needs `cc`)
```

Default tests use authored synthetic cartridges, frozen oracle observations
(`crates/mdcore/tests/fixtures/*.json`) and pinned SingleStepTests CPU records
(`crates/mdx/testdata/`). Tests that need commercial ROMs are `#[ignore]`d.

The release build of the whole workspace is slow-ish; when iterating on mdcore,
`cargo build --release -q` then use the examples in `crates/mdcore/examples/`.

## Operations (22)

`template.create`, `template.update`, `guide`, `artifact.import`,
`artifact.read`, `artifact.view`, `memory.read`, `memory.search`,
`image.compare`, `memory.diff`, `survey`, `trace`, `watch`, `header`,
`disasm`, `audio`, `boot`, `ram.dump`, `cpu.conformance`, `render`,
`graphics`, `schema`.

- Discover contracts: `mdx schema` (paginated) / `mdx schema boot`.
- Invoke anything: `mdx call boot --args '{"rom":"/abs/game.md","frames":60}'`.
- MCP: `mdx --root /abs/project mcp`; `mdx setup mcp` registers the running
  binary with Claude Code, Codex and OpenCode. Releases: push a `v*` tag
  matching `[workspace.package] version` (`.github/workflows/release.yml`);
  `install.sh` / `install.ps1` install the latest release.
- Always pass `project_root` (absolute) per call; artifacts live in
  `<project_root>/.mdx/artifacts`. ROM paths should be absolute.
- `boot` is the workhorse: bounded run, screenshot (scanline renderer), memory
  snapshot, diagnostics, `--save-state` / `--from-state` for resuming,
  `--sample` for per-frame RAM values (trajectory tests).
- Native ports: `mdx guide port`, then `template.create`. The template
  (`templates/megadrive-bevy`) is a workspace: `md-bevy/` (template-owned,
  replaced by `template.update`), `engine/` (with a sandbox that must build
  without the game), `game/`, `tools/import/`. `build.rs` embeds every file
  under the template directory except `target/`; keep the template building
  and its tests passing (`cargo test --workspace --all-features` there).
- `audio` captures stereo WAV, optional VGM and per-voice stems.
- `trace`/`watch` give pre-instruction register JSONL and bus-write logs (68000
  and Z80).

Details, limits and examples: [docs/mdx.md](docs/mdx.md),
[docs/save-states.md](docs/save-states.md), [docs/audio-development.md](docs/audio-development.md),
[docs/project-context.md](docs/project-context.md).

## mdcore architecture

| File | Contents |
|---|---|
| `system.rs` | `System`: the bus, scheduler and glue. `step_instruction()` runs one 68000 dispatch (or interrupt entry / stall step) and advances all devices by its master clocks. Interrupt delivery and acknowledge, refresh, DMA stalls, open bus, I/O ports, light guns, SVP scheduling, the program (instruction-fetch) view. |
| `cpu/` | 68000: `m68000.rs` (state, exceptions, address errors, prefetch), `exec.rs` (instruction semantics and cycle costs), `ea.rs`, `disasm.rs`, `legal.bin` (opcode legality generated from the reference). |
| `z80.rs` | Z80 with MEMPTR/WZ, undocumented behaviour, per-opcode T-states. |
| `vdp.rs`, `vdp/scanline.rs` | VDP timing, ports, FIFO, DMA (68000-bus, fill, copy; progressive per-line updates), HV counters/latch, interrupts; mode-5 scanline renderer. |
| `fm.rs`, `psg.rs`, `audio.rs` | YM2612 bus/timers/busy (lazy, access-timed), PSG, mixing/synthesis. |
| `cartridge.rs`, `memmap.rs`, `cart_hw.rs`, `cart_db.rs`, `eeprom.rs` | ROM image/mirroring, address decode, SRAM, I²C EEPROM boards, J-Cart, mappers (512 KiB, multi-game, Realtec, SF-00x, SMW64…), the generated 71-entry reference cartridge database. |
| `svp.rs` | SSP1601 (Virtua Racing): interpreter, DRAM, registers, DMA quirk. |
| `input.rs` | 3/6-button pads (TH counter, timeouts, latency), Justifier/Menacer light guns. |
| `state.rs` | Versioned machine save states (serde; new fields use `#[serde(default)]`). |
| `observe.rs` | Trace/watch records. |

Conventions that matter:

- **Master clocks.** Everything is scheduled in master cycles (68000 = 7, Z80 =
  15 per cycle; 3420 per line; 896040 per NTSC frame). A frame edge is the
  start of line 224 (VBLANK), matching the oracle's `retro_run` boundary.
- **Program view.** Instruction fetches, PC-relative operands and open bus read
  each bank's *base pointer* (GPGX `memory_map[].base`), bypassing read
  handlers: EEPROM/SRAM handlers, mapper registers, VDP/I/O (whose base is WRAM).
  The PC is never checked or aligned; odd fetches emulate GPGX's byte-swapped
  host read. See `System::program_word`.
- **Prefetch.** The reference keeps one cached word; the word after an opcode is
  fetched after the previous instruction's writes. The SingleStepTests harness
  sets `hardware_prefetch` to keep the real two-word queue.
- **No guessing behaviour.** If a behaviour isn't in the GPGX source or a frozen
  oracle observation, don't invent it. Port the source line by line and say so.

## The oracle and validation workflow

The oracle lives in `cache/oracle/` (not distributed): the stock GPGX libretro
core and an instrumented trace build (`cache/oracle/z80-contention-late`, built by
`tools/build_trace_oracle.py`). **Never modify the oracle's execution, replays,
ROMs or frozen references to make mdx agree.**

Tools (`tools/` and `.mdx/library-survey/`, the latter local and ignored):

| Tool | Use |
|---|---|
| `tools/oracle_gpgx.py ROM --frames N --out x.png --state s.state` | Run the oracle; `MDX_ORACLE_CORE`, `MDX_ORACLE_TRACE*`, `MDX_ORACLE_Z80_TRACE*` env vars select the trace core and outputs. |
| `tools/oracle_vdp_snapshot.py` | Normalize a GPGX state into CRAM/VSRAM/VRAM/WRAM artifacts. |
| `.mdx/library-survey/trace_game.sh ROM DIR LIMIT FRAMES` | 68000 trace of both cores, reports the first register/timing divergence (`trace_at.py`). |
| `.mdx/library-survey/z80_diff.sh` + `z80_align.py` | Same for the Z80 (alignment skips IRQ-entry records the oracle doesn't hook). |
| `.mdx/library-survey/bisect_cram.py ROM f1,f2,...` | Compare CRAM/VSRAM/WRAM/VRAM at frames. |
| `.mdx/library-survey/compare_two.py cases.json OUT OLD_BIN NEW_BIN '{}' auto` | Whole-library regression: pixels/VRAM/WRAM at frame 120 for two builds. |
| `tools/probes/*.py` | Author tiny probe ROMs and record oracle observations; frozen into `crates/mdcore/tests/fixtures` and checked by tests such as `access_phase.rs`. |
| `tools/gen_opcode_legality.py` | Regenerates `cpu/legal.bin` from the reference's opcode table. |
| `crates/mdcore/examples/*` | Small debugging programs (timing trace, Z80 trace, DMA/VRAM watches, cartridge peek, opcode legality sweep). |

Typical loop for a divergent game:

1. `bisect_cram.py` to find the first differing frame and memory.
2. `trace_game.sh` to find the first differing instruction (registers or `dt`).
3. Read the instruction, then the corresponding GPGX source (`cache/oracle/Genesis-Plus-GX/core/...`).
4. Port the exact behaviour, add/extend a unit test or frozen probe, run
   `cargo test --release`, then a library comparison against the previous build.
   No regressions are acceptable.

## Current status

Library comparison at frame 120 against the oracle (2,768 No-Intro Mega Drive
images): about 2,720 match exactly in pixels, VRAM and WRAM. The rest are mostly
Mega-CD/LaserActive BIOS images (CD hardware is out of scope), seven images the
oracle itself refuses to load, and a handful of small residual differences
tracked in [docs/WORK_LOG.md](docs/WORK_LOG.md). Virtua Racing (SVP) matches
through at least frame 3600.

Implemented reference behaviour includes (non-exhaustive): mode-5 scanline
rendering; status/HV sampling at dispatch plus instruction cost; one-instruction
IRQ delay; interrupt acknowledge inside SR-writing instructions; E-clock
acknowledge phase; refresh deferral; FM busy/timer timing through bus latency;
progressive 68000/fill/copy DMA and its stall split at the frame edge; control
port, FIFO and cached writes; 6-button pads and light guns; open bus; 68000
address errors (including odd-stack halts); the reference's opcode legality;
cartridge database, mappers, EEPROM, SRAM base/handler split; SVP; region
detection quirks; Z80 MEMPTR and refresh register.

## Repository map

```
crates/mdcore      emulation core (+ tests/, examples/)
crates/mdx         CLI/MCP toolkit (+ tests/, testdata/)
templates/         template.create sources (native Bevy port workspace)
tools/             oracle wrappers, probes, generators, installers
docs/              handbook (mdx.md), work log, investigations, handoffs
examples/          sample controller replay
rom/, cache/, .mdx/  local only (ROMs, oracle, artifacts) — ignored by git
```

## Rules of engagement

- Preserve uncommitted work; commit only when the maintainer asks.
- Don't modify oracle execution, replays, ROMs or frozen fixtures to pass.
- Don't edit other agents' game projects (reading is fine).
- New behaviour needs evidence: GPGX source, a frozen probe observation, or a
  trace match. Cite it in comments ("the reference ..."), matching the
  surrounding comment style.
- Keep responses bounded and honest; add limitations rather than overclaiming.
- Update [docs/WORK_LOG.md](docs/WORK_LOG.md) with milestones.

Licensed under MIT OR Apache-2.0.
