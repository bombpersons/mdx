# mdx CLI and MCP handbook

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
| `disasm` | `disasm` | mdcore decoder’s interpretation of ROM instructions |
| `boot` | `boot` | Bounded replay, screenshot and memory snapshot |
| `ram.dump` | `ram_dump` | Bounded replay and RAM range inspection |
| `memory.read` | `memory_read` | Snapshot bytes, including folded WRAM aliases |
| `memory.search` | `memory_search` | Byte-pattern search, exact totals and pages |
| `memory.diff` | `memory_diff` | Changed-byte totals and paginated ranges |
| `render` | `render` | Render verified captured VDP buffers |
| `graphics` | `graphics` | Tile indices, palette sheets, planes and sprite layout |
| `trace` | `trace` | Pre-instruction register-state JSONL |
| `watch` | `watch` | Folded WRAM bus-write JSONL, all widths and same-value writes |
| `survey` | `survey` | Batch named operations with per-item results |
| `cpu.conformance` | `cpu_conformance` | Bounded independent instruction fixtures |
| `schema` | `schema` | Discover the complete operation contracts |

OpenCode prefixes raw tool names with the configured server name: for example,
`mdx_boot`, `mdx_memory_read` and `mdx_artifact_view`.

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
  active work. The SDK may briefly drain pending responses during shutdown.

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
