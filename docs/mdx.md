# mdx CLI and MCP handbook

Full machine save/load is available on boot, trace, watch and RAM dump.
See [save-states.md](save-states.md) for CLI/MCP examples and compatibility.

## Contract

`mdx` uses a shared Rust operation layer for CLI and MCP. Every analysis response
has `schema_version`, `tool`, `ok`, `exit`, `subject`, `coverage`, `warnings` and
`data`; failures also have a structured `error`. Exit categories are `ok`,
`partial`, `failed` and `usage`. Partial evidence is returned to MCP as a usable
result, not a protocol error; callers must inspect its coverage.

`mdx schema OPERATION` returns authoritative input/output schemas. Schemas use
portable numeric bounds, not Rust-specific integer-format annotations. The
default catalog returns up to four descriptors plus the complete list of
operation names and an explicit pagination cursor. It does no analysis or
filesystem writes. Use `mdx call OPERATION --args '{...}'` to invoke any operation
with exactly the same JSON arguments as MCP.

## Operations

| CLI operation | Raw MCP tool | Purpose |
|---|---|---|
| `artifact.import` | `artifact_import` | Copy a bounded input into the immutable store |
| `artifact.read` | `artifact_read` | Verified hex/UTF-8 byte page |
| `artifact.view` | `artifact_view` | JSON Pointer or paginated JSON/JSONL records |
| `header` | `header` | Cartridge header, vectors and declared layout |
| `disasm` | `disasm` | mdcore decoder’s interpretation of 68000 or Z80 code (ROM or state Z80 RAM) |
| `boot` | `boot` | Bounded replay, screenshot and memory snapshot |
| `audio` | `audio` | Stereo FM/PSG WAV capture, warmup and resumable chip/mixer phase |
| `ram.dump` | `ram_dump` | Bounded replay and RAM range inspection |
| `memory.read` | `memory_read` | Snapshot bytes, including folded WRAM aliases |
| `memory.search` | `memory_search` | Byte-pattern search, exact totals and pages |
| `memory.diff` | `memory_diff` | Changed-byte totals and paginated ranges |
| `render` | `render` | Render verified captured VDP buffers |
| `graphics` | `graphics` | Tile indices, palette sheets, planes and sprite layout |
| `image.compare` | `image_compare` | Pixel diff of two PNGs: changed counts, regions, bounds, diff PNG |
| `trace` | `trace` | Pre-instruction register-state JSONL |
| `watch` | `watch` | Folded WRAM bus-write JSONL, all widths and same-value writes |
| `survey` | `survey` | Batch named operations with per-item results |
| `cpu.conformance` | `cpu_conformance` | Bounded independent instruction fixtures |
| `schema` | `schema` | Discover the complete operation contracts |
| `guide` | `guide` | Built-in workflow guidance: `overview`, `port` |
| `template.create` | `template_create` | Create a native Rust/Bevy port workspace (engine + game) |
| `template.update` | `template_update` | Update a created workspace's template-owned files |

OpenCode prefixes raw tool names with the configured server name: for example,
`mdx_boot`, `mdx_memory_read` and `mdx_artifact_view`.

Audio details and examples: [audio-development.md](audio-development.md).

### Header and address conventions

`header` reports the **first 64 vector slots** at ROM offsets `0x00..0xFF`.
These include reset, exceptions, interrupt autovectors and all 16 TRAP slots;
the 68000's user-defined vector slots 64..255 are outside this header view.
VBlank uses level-6 autovector index 30 (`0x78`), HBlank level-4 index 28
(`0x70`). Reserved slots are named explicitly; uninitialized interrupt is
index 15, spurious interrupt index 24, and TRAP #0 starts at index 32.

`disasm.from` is an **even CPU bus address in `0x000000..0x3FFFFF`**. The
cartridge window starts at zero: ordinary ROM offset `0x200` is bus address
`0x200`, not `0x020200` or a `0x200000`-based address. The result echoes both
`from` and `cart_offset`; every instruction has its own `address` and
`cart_offset`. The bus PC continues advancing across cartridge mirrors, so
PC-relative targets retain the requested bus coordinate. Coverage states
mdcore's mapping assumption (power-of-two mirror period, then file-length
modulo); it does not validate a cartridge's mapper. Non-cartridge starts fail;
listings reaching the window boundary become partial.

`ram.dump.range` accepts `[address, byte_count]`. Each element can be a JSON
integer, decimal string, or `0x`/`0X`-prefixed hexadecimal string. For example:

```json
{"rom":"/path/to/game.md","instructions":200000,"range":["0xFFE21C",32]}
```

Folded offset `0xE21C`, full alias `0xFFE21C`, and unmasked alias `0xFFFFE21C`
select the same WRAM bytes. `0x0000E01C` is accepted as **folded WRAM offset**
`0xE01C` here; an actual CPU access to bus address `0xE01C` selects cartridge,
not WRAM. Larger non-WRAM addresses are rejected. Reads wrap within the 64 KiB
buffer, at most once; row labels wrap too and use canonical `0xFF0000` aliases.
The CLI equivalent is `ram dump --range 0xFFE21C:32`.

### Z80 disassembly

`disasm` with `cpu: "z80"` (CLI `--cpu z80`) decodes Z80 code such as a sound
driver. From the ROM, `from` is a file offset and `origin` the Z80 address that
byte occupies once uploaded (default 0): `--cpu z80 --from 0x7A000 --origin 0`.
With `from_state`, it reads the Z80 RAM of a saved machine state and `from` is a
Z80 address (0x0000..0x1FFF), so the listing shows the driver as uploaded,
including any self-modified bytes. All prefix groups are decoded, including
undocumented IXH/IXL, `sll`, `in (c)`, `out (c),0` and indexed bit operations
that also store to a register. Undefined ED opcodes and redundant prefixes are
listed as no-ops with `decoded: false`. Instruction lengths are cross-checked
against mdcore's Z80 executor for every opcode in every prefix group; mnemonics
are not independently validated. `cart_offset` is the offset in the source
(ROM file or Z80 RAM).

### Z80 trace and Z80 RAM watch

`trace` with `cpu: "z80"` records executed Z80 instructions (frame, master
cycle, PC, bytes, mdcore disassembly, AF/BC/DE/HL/IX/IY/SP before execution).
`z80_pc_range: [start, end]` keeps only PCs in that half-open range, and HALT
idling is not recorded. `watch` with `watch.space: "z80"` records writes to a
Z80 RAM range (`address` 0x0000..0x1FFF or 0xA00000..0xA01FFF) by either CPU,
with `writer` (`z80` or `m68000`), the writer's PC, and the value before and
after; `writing_pc` filters by that PC. Start gates (`start_frame`, `start_step`,
`start_pc`) and `stop_at` still refer to the 68000 run. Instruction bytes come
from Z80 RAM, so code running from the banked 68000 window is not decoded.

### Offline CPU conformance

`cpu.conformance.suite` accepts either a SingleStepTests `68000` directory
containing `v1/*.json.gz`, or a local bundled regression JSON file:

```sh
target/release/mdx cpu conformance --suite crates/mdx/testdata/cpu-regressions.json --compare-cycles
target/release/mdx cpu conformance --suite crates/mdx/testdata/cpu-regressions.json --filter DBcc
```

Bundled files contain `sources` (upstream filenames, declared SHA-256 and case
names) and `cases` (`file` plus the original `case` object). Source/case
membership must agree exactly. Filters are case-sensitive filename substrings;
`BRA` matches nothing in the current CPU regression bundle. The same canonical
case executor and timing option are used for both input forms.

`source_files` hashes the actual input bytes. `bundled_sources` retains declared
upstream provenance; it does not claim the original gzip files were available
or independently rehashed. For bundled input, `files_total` counts bundled
instruction groups. A complete bundled run means all selected local regression
records passed, not that the upstream suite or the entire CPU is covered.

### Controller replay files

Schema discovery for `boot`, `trace` and `watch` includes
`data.operations[0].file_schemas.controller_replay`. The file shape is:

```json
{
  "note":"hold Start for two frames",
  "events":[
    {"frame":0,"port":0,"buttons":["start"]},
    {"frame":2,"port":0,"buttons":[]}
  ]
}
```

Port 0/1 pads follow the reference's automatic type: a 6-button pad when the
cartridge header's I/O field (0x190..0x19D) lists `6`, otherwise 3-button. The
6-button TH pulse counter (reset 27 line starts after its last counted rising
edge) and the 172-clock TH input switching latency are modelled; replay files
cannot press X/Y/Z/Mode, which read released.

`note` is optional and has no execution effect. The source SHA-256 still includes
the annotation's bytes. Events are ordered by completed frame count (zero-based),
port defaults to 0 and is 0 or 1 (console) or 2 or 3 (J-Cart controllers 3 and 4,
ignored by other cartridges), and there is at most one event per port
per frame. Buttons are `up/down/left/right/a/b/c/start`; an empty list releases
them all, otherwise each event replaces that port's held-button state. There
are at most 10000 events. Unknown keys (including `_comment`) and duplicate
buttons are rejected; schemas express the structural shape, while the loader
also checks ordering and uniqueness.

### Output directories and compact results

`out` on `boot`, `audio`, `render` and `graphics` names an output directory for
the run's files (screenshot, memory dumps, WAV/VGM, images), not a report file;
the JSON envelope is the operation's return value. `boot` with `profile:
"summary"` keeps frame/stop status, state and snapshot IDs, memory artifacts,
the screenshot and exceptions, and omits traces, hot spots, registers, VDP and
sound diagnostics and observation tables (Cadash frame 300: 33.7 KB to 5.2 KB).
Screenshots and graphics images are indexed PNGs when they have at most 256
colours (all Mega Drive output), at maximum compression: decoders that expand
palettes see identical pixels; `display.screenshot_pixels_sha256` hashes the
decoded RGB pixels independently of encoding (Cadash: 22.3 KB to 5.0 KB).
MCP responses carry the envelope both as text and as `structuredContent`;
clients should read `structuredContent` rather than printing both.

### Screenshot renderer

`boot` screenshots come from a scanline renderer ported from the reference's
mode-5 renderer: each active line is drawn when the VDP reaches it, from the
VRAM/VSRAM/registers of that moment, with CRAM resolved at the same instant, so
mid-frame scroll, palette, plane-base and display changes appear as on the
reference. It models the internal sprite-attribute cache, sprite count/pixel
limits and masking, shadow/highlight, window rules (including the
partial-column bug), HBLANK re-render/remap of the current line, and the
sprite overflow/collision status bits. 68000-bus DMA data arrives per reference
update (per line, or the whole VBLANK at once). Limitations reported in
`display.limitations`: mode 4 graphics, and interlace mode 2 rendered as a
single field. `render` of a captured VDP snapshot still uses the end-of-frame
snapshot renderer, which cannot show raster effects.

### Sprite groups

`graphics` accepts `sprite_groups`: up to 16 `{name, sprites:[indices]}` entries
naming hardware sprite-table indices explicitly (grouping is never inferred
from neighbouring entries). Each group becomes one transparent RGBA PNG over the
union of its sprites' bounds, drawn with each sprite's own tiles, palette and
flips; pattern index 0 is transparent and earlier-listed sprites are in front.
The response gives the screen-space `x`, `y`, `width` and `height` of each image.

### Image comparison

`image.compare` takes two PNGs (`left`, `right`: project paths or artifact IDs,
for example an mdx screenshot and a native port's `game_screenshot`), decodes
both to RGB (palettes expanded, alpha composited on black) and reports absolute
`changed_pixels`, `changed_bounds`, optional named `regions` with their own
counts, and a `diff_artifact` PNG: changed pixels magenta over the dimmed left
image. Sizes must match. A full-frame gate catches what a region gate misses.

### Per-frame samples

`boot` with `samples` (CLI: repeated `--sample ADDRESS:TYPE:LABEL`) records
work RAM values at every frame boundary into a CSV artifact
(`data.samples.artifact`), for trajectory tests of a native port. It needs
`frames`. Types are `u8 s8 u16 s16 u32 s32` and the fixed-point `fx8.8`
(signed word / 256) and `fx16.16` (signed longword / 65536); values are
big-endian, as the 68000 stores them. Addresses are hexadecimal work RAM
addresses (`0xFFD008`, `0xD008` or any `0xE00000..` alias).

```json
{"rom":"/abs/game.md","from_state":"sha256:...","frames":60,"instructions":5000000,
 "input":"/abs/tests/data/walk.input.json",
 "samples":[{"address":"0xFFD008","type":"s16","label":"x"},
            {"address":"0xFFD00C","type":"s16","label":"y"}]}
```

The CSV header is `run_frame,frame,<labels>`. Row N is memory when run frame
N begins (the frame counter advances at the start of vertical blank), before
frame N's controller events take effect; row 0 is the starting state. The
template's `md_bevy::testing::trajectory` reads it directly.

### Native port workspace

`template.create` writes a Cargo workspace for a native Bevy port into an
absolute directory, typically the folder holding the ROM: existing files are
kept, and it refuses only if a path it would write exists (an existing
`AGENTS.md`, `README.md` or `.gitignore` is kept and reported as skipped).
`md-bevy/` is the template's crate (Mega Drive look, ticks, input, launcher,
testing, developer MCP); `engine/` holds registries, update-order sets, level
and tuning formats and a sandbox game; `game/` the title; `tools/import/` the
ROM importers. `.mdx-template.toml` records the version and file hashes.
`template.update` (with `dry_run` first) replaces unedited `md-bevy/` files,
writes `<file>.mdx-new` beside edited ones and copies changed seeded files
under `.mdx/template-update/`. The workflow is in `mdx guide port`.

```sh
mdx template create --directory /abs/kings-bounty --name kings-bounty
mdx template update --directory /abs/kings-bounty --dry-run
```

### Frame pokes

`boot`, `audio`, `trace` and `watch` accept `pokes`, either inline or in the
replay file (`events` may then be empty or omitted). Each poke writes bytes
directly into memory once `frame` frames have completed, relative to reset or
the loaded state, at the same instruction boundary where that frame's
controller events take effect. JSON addresses and bytes are integers (here
0x1C0A and 0x98):

```json
{"frame":1830,"space":"z80","address":7178,"bytes":[152]}
```

`space` is `wram` (0x0000..0xFFFF or any 0xE00000..0xFFFFFF alias) or `z80`
(0x0000..0x1FFF or the 68000 view 0xA00000..0xA01FFF). Each poke writes 1..256
bytes and must not cross the end of its space. There are at most 1000 pokes,
ordered by frame. Inline pokes are merged after the file's pokes. The CLI form
is a repeatable `--poke FRAME:wram|z80:ADDRESS=HEXBYTES` on `boot` and `audio`,
for example `--poke 1830:z80:0x1C0A=98`. Reports include `pokes_applied` and
`pokes_pending`. Pokes bypass bus timing and are analyst interventions, not
hardware behaviour: a state saved after a poke carries its effect.

### Cartridge backup RAM, mappers and region

mdcore follows the reference's default cartridge handling. Backup RAM (64 KiB,
initialized to FF) is enabled when the header declares `RA` at 0x1B0, with the
reference's generic fixes for invalid ranges, and by default for ROMs of at
most 2 MiB without a declaration (at $200000). It occupies the 64 KiB bank of its
start address. SRAM that overlaps the ROM (Phantasy Star IV, World Series
Baseball) is switched in by bit 0 of $A130F1 and write-protected by bit 1.
Writes above $A130F1 select the standard 512 KiB banks, and writes below $A13060
apply the reference's fallback multi-game 64 KiB mapper. Like the reference,
these mappers index the ROM buffer without mirroring (0xFF padding up to the
power-of-two size, zeros beyond), and a mapper that repoints the SRAM bank
leaves SRAM data reads in place while instruction fetches see the ROM.

Serial (I2C) EEPROM saves follow the reference's game table (EA, Sega, Acclaim
16M/32M and Codemasters boards, chips X24C01 to 24C65) and its header fallback
(`RA` with byte 0x1B2 = E8, or a declared range under two bytes, gets a Sega
board with an X24C01). The EEPROM array is the same 64 KiB backup buffer, so the
`sram` artifact, `sram` input and machine states cover it. Codemasters J-Cart
titles expose controllers 3 and 4 at $380000; replay ports 2 and 3 drive them.
The reference's 71-entry cartridge database (protection registers, Realtec,
SF-001/002/004, Super Mario World 64 and other mappers), the SVP coprocessor
(Virtua Racing) and its light guns (Justifier, Menacer, attached by title) are
modeled; Sonic & Knuckles lock-on beyond ROM mirroring is not.
`boot` publishes an `sram` memory artifact when the cartridge has backup RAM,
and accepts `sram` (CLI `--sram`) to load up to 64 KiB of saved contents first.
Machine states carry SRAM and bank state; older states restore power-on values.

`region` (`auto`, `usa`, `japan`, `europe`; CLI `--region`) chooses the console
at power-on. `auto` follows the header like the reference: USA, then Japan,
then Europe. It sets the version register at $A10001. PAL consoles (`europe`, or
a Japan-PAL header) also run PAL timing: 313-line 50 Hz frames, the PAL
V-counter wrap points, the status PAL bit, the reference's PAL reset phase and
the 53,203,424 Hz master clock for audio (and VGM chip clocks). `boot` reports
`console_region`. With `from_state`, the state's region applies.

### Sound-command log

`boot` and `audio` accept `sound_commands` (`{}` for defaults; CLI
`--sound-commands`). It records every 68000 write into Z80 RAM and the Z80 bank
register, with absolute frame, master cycle, writing PC, bus address, Z80
offset, value and whether the 68000 held the Z80 bus. Writes made without the
bus are dropped by the console but still recorded, with `z80_bus_granted` false.
BUSREQ/RESET writes are only counted (`bus_control_counted`) unless
`bus_control` is true, since games toggle BUSREQ many times per frame. Options:
`max_records` (default 100000; counts continue past it), and `z80_start` with
`z80_length` to keep only a Z80 RAM window. `audio` logs during the capture only,
after `start_frame`.

Records go to a JSONL `records_artifact`. The response adds `by_target` counts,
`uploads` (frames with at least 32 Z80 RAM writes, such as a driver upload) and
up to 32 `busiest_z80_addresses` outside uploads, ranked by distinct frames
written, then writes, then recency, each with up to 32 values and 8 writer PCs.
A driver's command slot usually leads: over Cadash's first 6000 frames it is
`0x1C0A`, written from PC `0xA3C` with 13 sound ids.

### VGM export

`audio` with `vgm` (CLI `--vgm`) also publishes the capture's YM2612 and SN76489
register writes as a VGM 1.60 file (`vgm_artifact`, plus a digest-named `.vgm`
next to the WAV when `out` is given). The stream begins with writes that
recreate both chips' state at the capture start, then every write in synthesis
order with 44.1 kHz waits, so a capture can start mid-song. FM registers come
from a shadow of all writes, saved with machine states; a state saved by an
older build marks `fm_initial_state_known` false. The `vgm` summary gives
FM/PSG/DAC write counts and key-ons per FM channel. The VGM uses NTSC clocks.
An FM reset during a capture cannot be expressed in VGM and is counted in
`fm_resets`. Checked against libgme (an independent player): a Cadash capture
from frame 1800 has the same sample count, a 0.938 loudness-envelope
correlation and matching dominant pitches, with different gain.

### Per-voice stems

`audio` with `stems` (CLI `--stems`) also renders one WAV per voice: `fm1`..`fm6`
(`fm6` includes the DAC) and `psg1`..`psg4` (`psg4` is noise), each with peak
and RMS. Each stem re-synthesizes the capture's register writes from the exact
synthesis state at capture start, with the other voices silenced: FM channels
by maximum total-level attenuation and key-off, the DAC by disabling it, PSG
channels at attenuation 15. Re-synthesizing with every voice kept reproduces
the captured WAV exactly (checked from power-on). An isolated stem starts with
the output filter at rest on its own level, so voices that are silent produce
exact silence and removing sounding voices causes no click. The stems sum to the
mix only approximately: the YM2612 DAC's per-channel offset depends on each
channel's output sign (modeled "ladder" distortion), and the mix keeps the
filter history it had before the capture. Cadash 1800..2700: FM3 and all PSG
stems are exactly silent; after the first second the stem sum is within 200 of
a mix peaking near 3700.

## Artifacts and large results

An artifact has `id: "sha256:<64 lowercase hex digits>"`, an absolute `path`, byte
count and SHA-256. The default store is `.mdx/objects`; select another with the
global `--store` flag. Atomic non-overwriting publication verifies existing
objects. Corrupted IDs/files fail explicitly.

ROM and snapshot arguments accept a local path or a stored ID. Imports make
mutable source files into immutable inputs. Capture filenames contain full
content digests, so different replays cannot overwrite one another. Captured
memory, images, layouts and trace records are also registered in the store.
Snapshot memory references with explicit IDs resolve through that store, even
after their original named capture directory has been removed. Legacy path/hash
snapshots retain path-based integrity verification.

Responses exceeding 64 KiB return `data.result_artifact`, the complete original
JSON envelope, plus its byte count. No fields are silently dropped. Use
`artifact_view` to select a small field rather than rebuilding a large document
from text fragments:

```json
{"source":"sha256:...","pointer":"/data/vdp/registers","limit":32}
```

For a trace/watch artifact:

```json
{"source":"sha256:...","format":"jsonl","offset":0,"limit":16}
```

Views and searches report totals and `next_offset`. Byte pages use byte offsets
and exact hex by default; UTF-8 pages must land on valid character boundaries.
JSON Pointer follows RFC 6901. JSONL pointers apply to each selected record.

## Agent workflow

1. Import the ROM; use its ID for subsequent calls.
2. Inspect `header`, then `disasm` at its reset vector.
3. Run `boot` with a step/frame budget and an optional controller replay.
4. Use `data.snapshot_artifact.id` for `memory_read/search/diff`, `render` and
   `graphics`. Select fields from a spilled boot result through `artifact_view`.
5. Use `watch` to find the actual writer, then `disasm` and `trace` to inspect it.

Every capture starts from reset. Snapshots contain captured memory/VDP state;
they are **not resumable CPU checkpoints**.

For example, to watch a chosen WRAM range during a controller replay, use these
MCP/`call` arguments (choose a meaningful range for your own cartridge):

```json
{
  "rom":"/path/to/game.md",
  "frames":90,
  "instructions":3000000,
  "input":"examples/controller-replay.json",
  "start_frame":10,
  "watch":{"address":256,"length":256},
  "max_records":64
}
```

`256` is WRAM offset `$0100`. Watch matching uses folded memory offsets and
records the original address, transfer width, previous/new value, frame and
writer. Registers belong to the pre-step 68000 state; they are null for other
bus masters. Unattributed accesses have no invented PC. Boot also reports
bounded unhandled bus-access samples with read/write direction and exact
observed/retained/omitted counts.

Trace start gates (`start_frame`, `start_pc`, `start_step`) are conjunctive and
become latched once satisfied. `start_step` counts **system attempts**, including
IRQ and stopped-CPU steps, not oracle instruction ordinals. Instruction records
exclude IRQ-entry and stopped-idle pseudo-steps. `next_pc` is the observed PC
after the instruction, including branch/return targets.
`start_pc` **begins recording**, rather than filtering records to that address:
once the gates latch, subsequent instructions at other PCs are recorded too.
Trace always includes pre-instruction registers; it needs no `trace_registers`
flag. Watch currently observes WRAM writes only.

Boot's frame and PC stop targets are alternatives: reaching either target is a
successful bounded capture (`termination: "frame_target"` or `"stop_point"`).
PC stops occur before executing the target instruction, including when the
last budgeted step just reached it. `frames: 0` is an already-reached target,
so no instructions execute. If neither requested target is reached, the run
is partial and identifies the missing frame/PC target; timeout and cancellation
have their own termination reasons.

## Limits and lifecycle

- Analysis: at most 100 million system attempts, 100000 frames; default step
  budget 200000. Attempts count exceptions as well as successful instructions.
- Trace/watch: up to 100000 retained records; default 1024, inline preview 16.
- Boot: diagnostic retention is bounded; per-stream omission counts are explicit.
- File/JSON artifacts: up to 64 MiB; byte pages 16384 bytes; structured views
  256 rows; tool arguments 1 MiB; incoming MCP frames 2 MiB.
- Survey: 1–32 unique tasks, no nested surveys.
- Conformance: up to 4096 suite entries, 100000 cases/file and 20 million cases;
  malformed/empty fixtures fail instead of counting as passes.
- `timeout_ms`: optional, up to one hour, checked at computational boundaries.
  Timeout/cancellation captures are partial. Filesystem I/O is not interruptible.
- MCP: 1–8 worker slots, default 2. Requests cancel cooperatively; EOF cancels
  active work. Keep stdin open through initialization and until all requested
  responses arrive, then close it. A one-shot pipe that closes immediately can
  cancel requests still in flight. The SDK may briefly drain pending responses
  during shutdown.

## Independent oracle and scope

Genesis Plus GX remains separate and is not distributed here. The originating
reconstruction project has seven finite King's Bounty pixel gates; those
game-specific tests and captures remain there. This repository's default tests
use synthetic inputs and CPU regression records. Smoke success is not proof of
full CPU/VDP, sound, mapper or timing fidelity.

GPGX stores words in host order and compensates with its byte-access macros.
The oracle wrapper now exports WRAM in logical 68000 byte order; the pinned
save-state converter also includes normalized WRAM. Older raw `--ram` files on
little-endian hosts need pair-byte normalization before comparison. This is an
export representation correction, not a change to oracle execution.

The private observer hook supports `MDX_ORACLE_TRACE_START_PC` and
`MDX_ORACLE_TRACE_START_COUNT` together. Its retained-record limit is independent
of skipped instructions. Rebuild a **new** private trace directory after editing
the hook; existing build manifests identify earlier versions. Its count refers
to actual oracle dispatches, so align windows by verified state/PC, not by
equating it with mdx’s system-attempt counter.

`tools/mdx_trace_to_binary.py` converts JSONL to the oracle’s 18-word record;
`tools/compare_cpu_traces.py` compares exact aligned CPU state. An equal finite
prefix does not prove equal bus effects or whole-game behaviour.


## Project context and artifacts

All operation schemas include optional `project_root` and `artifact_store`.
Pass the calling agent's absolute project directory as `project_root` to resolve
relative inputs/outputs there and use its `.mdx/artifacts` by default. An explicit
`artifact_store` is absolute or relative to that root. Supply the same context
for subsequent artifact-ID queries. Surveys inherit scope and may override it
per task. Requests never change the server process directory. `out` still selects
capture/export destinations where supported. See [project-context.md](project-context.md).
