# Resumable machine states

`boot`, `trace`, `watch`, `ram.dump` and `audio` accept `save_state: true` and return
`data.state_artifact`. Pass that artifact's `id` as `from_state` on any of those
operations, together with the **same ROM**. Local JSON state paths also work.
The state is immutable and content addressed; concurrent agents can load it
independently without changing one another's machine or artifact.

```sh
mdx --root /path/to/project boot --rom game.md --frames 2700 \
  --instructions 100000000 --trace 0 --save-state

# Replace the placeholder with data.state_artifact.id from the previous result.
mdx --root /path/to/project boot --rom game.md \
  --from-state sha256:STATE_HASH --frames 120 --instructions 5000000 \
  --input next-input.json --trace 0 --save-state

mdx --root /path/to/project ram dump --rom game.md \
  --from-state sha256:STATE_HASH --instructions 0 --range 0xFF0A40:0x20

mdx --root /path/to/project trace --from-state sha256:STATE_HASH --save-state \
  --args '{"rom":"game.md","frames":2,"instructions":100000,"max_records":16}'

mdx --root /path/to/project watch --from-state sha256:STATE_HASH \
  --args '{"rom":"game.md","frames":2,"instructions":100000,"watch":{"address":16714304,"length":32}}'
```

MCP uses the same fields, with per-call project context:

```json
{
  "project_root": "/path/to/project",
  "rom": "game.md",
  "from_state": "sha256:STATE_HASH",
  "frames": 120,
  "instructions": 5000000,
  "input": "next-input.json",
  "trace": 0,
  "save_state": true
}
```

Use the same project/store for artifact-ID lookup. To share across project stores,
pass the absolute state path or import that file with `artifact_import` into the
recipient's store. States omit ROM bytes, so the matching cartridge must remain
available separately. States still contain game memory and are local analysis
artifacts; the default `.mdx/` store is ignored by Git.

## Time and input semantics

- `instructions` and `start_step` count new system-step attempts after loading.
  The100M step cap applies to each run; states allow chaining further runs.
- `frames` is a duration in completed video-frame boundaries from the starting
  state. A state at frame2700 plus `frames:120` targets frame2820.
  `start_frame` on trace/watch is relative to that same starting state.
  For audio, `start_frame` is warmup and `frames` is capture duration after it.
- `starting_frame` and `elapsed_frames` report the origin and new progress.
  Existing `reached_frames` (boot), `frames` (trace/watch), record frame stamps
  and master clocks retain absolute machine coordinates.
- Replay events are relative to the loaded state's frame. Frame0 applies before
  the first new step. Held controller buttons are preserved if no event changes
  them. The old replay file and its cursor are **not** stored in the machine.
  For a split replay at frame F, retain events at/after F and subtract F from
  their frame numbers. Events at the stopping boundary have not yet applied.
- `instructions:0` loads/inspects/saves without advancing or applying replay
  events. Save states capture the exact instruction boundary reached, including
  any small overshoot across a frame boundary. A mid-frame load's first frame
  boundary therefore takes less than one full frame interval.

## Contents and compatibility

Version4 stores both CPUs (including prefetch/interrupt flags and Z80 WZ), WRAM/Z80 RAM,
VDP memories/registers/latches/FIFO/DMA/interrupt and fractional clock phase,
modeled YM2612 registers/timers/BUSY phase, I/O/controller state, scheduler
absolute sound/DMA clocks, refresh/IRQ acknowledgement phase, and analyst branch overrides.
It also stores YM2612 backend/host state, integrated PSG phase, pending timed
writes and fractional stereo mixer/resampler phase. PCM buffers are excluded.
No CPU queue is reconstructed or reset after restoration. Tool observers and
bounded write/trace logs start fresh; device counters stored in VDP/FM persist.
Existing branch overrides persist until superseded at the same address.

A `snapshot_artifact` is the existing captured-buffer analysis envelope consumed
by memory/render/graphics. A `state_artifact` is the separate resumable machine
format consumed by execution operations. They are not interchangeable.

Restoration rejects mismatched ROM digests, unsupported format/engine versions,
and invalid buffer sizes/device indexes/clocks. Format or engine changes that
alter state meaning require a compatibility version change; cross-build or
future-version compatibility is not promised. Deterministic continuation means
matching **mdcore's modeled execution**, not hardware equivalence. Current
mapper/timing and analog audio limitations remain. Formats1..3 need recapture.

## Verification checkpoints

The initial format1 Cadash split2700→4500 matches uninterrupted execution by machine-state digest,
all captured memory and screenshot; boot/trace/watch also yield identical states
for the same loaded inputs. A checkpoint chain reaches frame13500 over131 million
steps across bounded runs. The196 default Rust tests, four Python checks and27
frozen image gates pass. See [validation evidence](evidence/save-states-session.json).
Cadash's missing Z80 VBlank interrupt is fixed in the subsequent
[interrupt checkpoint](z80-vblank-2026-10-14.md). States now retain its finite
INT-line level and the Z80 EI shadow. Format1 states are rejected because their
execution predates that correction; recapture checkpoints with the rebuilt tool.

The format2 interrupt checkpoint repeats the split-run full-state/memory/image
comparison successfully, with203 default Rust tests and the same27 image gates.

The historical [format3 bus checkpoint](z80-contention-2026-10-14.md) repeats that
comparison successfully and passes211 default Rust tests. Format1/2 states are
rejected because execution and scheduler state changed; recapture with the new
build. Reconnect existing MCP clients to load the rebuilt binary.

The [format4 audio checkpoint](audio-development.md) preserves chip/host, queued
writes and mixer phase. Cadash split audio is sample-exact, including a checkpoint
made through ordinary boot with no recording. Reconnect MCP clients for the
new `audio` operation and recapture format1..3 checkpoints.
