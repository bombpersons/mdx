# FM/PSG audio capture

`audio` is available through CLI, typed API and MCP (`mdx_audio` in OpenCode).
It runs the console's sound driver and produces signed 16-bit stereo WAV at
44,100 Hz. Every console has its own chips, pending writes and mixer. Multiple
agents can capture independently using their project-local artifact stores.

```sh
mdx audio --rom 'rom/original/Cadash (USA, Korea).md' \
  --start-frame 1900 --frames 60 --instructions 100000000 --out .mdx/music

mdx audio --rom game.md --from-state sha256:STATE_HASH \
  --frames 120 --instructions 5000000 --save-state
```

MCP example:

```json
{
  "project_root": "/path/to/project",
  "rom": "game.md",
  "from_state": "sha256:STATE_HASH",
  "frames": 120,
  "instructions": 5000000,
  "out": ".mdx/music",
  "save_state": true
}
```

`start_frame` is warmup relative to reset or the loaded state's completed frame.
`frames` is the **capture duration after warmup**. Both warmup and capture consume
the hard `instructions` budget; the default is 5M, maximum 1e9. Replay events
are relative to the starting state, including warmup. Frames are 1..18000
(five NTSC minutes), start_frame <=100000; timeout/cancellation and retention bounds are explicit.
An unfinished run returns partial coverage and its retained audio, including an
empty WAV if warmup was never reached. Inspect `frame_target_reached`,
`termination`, `started`, `sample_frames`, clipping and dropped-sample counts.

`wav_artifact` always has an immutable SHA-256 ID resolvable in the selected
store. With `out`, a digest-named `.wav` is also published there; without it,
read the object at the returned artifact path. Relative paths resolve in the
calling project's scope, never by changing the shared server's directory.

## Synthesis and state

FM uses pinned `ymfm-sys=0.2.0`, the BSD-licensed Rust/CXX adapter to Aaron Giles's
ymfm YM2612. The native cadence is 144 YM clocks /1008 console master clocks;
its reported integer native rate is 53267 Hz. FM host timer/CSM callbacks,
register/address/DAC/envelope state and native sample phase are retained.
CPU-visible FM timer/status remains the separately validated `Fm` model.

The original Rust integrated PSG models serial latch/data writes, zero tone
periods, two-decibel attenuation, tone/noise clocks and 16-bit white/periodic
feedback. The integer mixer integrates piecewise-constant FM/PSG signals into
44.1 kHz stereo samples and removes DC. The backend's YM2612 ladder DAC has a
DC offset, including on muted channels; panning tests compare against its idle
output rather than assuming a zero startup transient.

Machine **format4** includes complete synthesis, timer-host and fractional mixer
phase, plus writes pending beyond the current safe render horizon. Formats1..3
need recapture. PCM capture buffers and their counters are observers, excluded
from states. Even `boot`/`trace`/`watch`/`ram.dump` advance sound synthesis, so a
checkpoint made without recording can later produce the same audio as a warmed
continuous run. Restoration checks backend identity, native blob size/layout,
envelope enums, clock bounds and mixer/PSG phase before native restoration.

Audio advances after scanline/VINT sound batches, preserving writes whose clocks
overshoot the batch. Outputs report audio and console clock endpoints and
absolute sample indexes. Chip phase is deterministic; FM/PSG access timestamps
still use the modeled bus/Z80 dispatch phase. Box resampling, DC removal and
relative gains are not a model of analog board filtering. This is not a claim
of hardware or Genesis Plus GX PCM equivalence.

## Independent checks

The authored PSG harness executes caller-supplied, unchanged GPGX `psg.c`.
All176 scenarios /3472 operations agree in logical pulse deltas and generator
phase. Expected observations are in
`crates/mdcore/tests/fixtures/psg_edges.json`; filtering/PCM is outside that gate.

`tools/oracle_audio.py` independently captures unchanged GPGX stereo callback
PCM while emulating every preceding frame. Separate single-sample/batch libretro
callbacks are correct; batches return consumed frames even outside the retained
interval. Six Python observer tests cover signed stereo, exclusions and format.

The Cadash1900..1960 clip from mdcore contains44157 stereo sample frames,
peak2851, RMS956.89, no clipping or dropped frames. The original reference has
the same sample count, peak4324, RMS1441.21; both have strong components near
785–787 Hz and1979–1980 Hz. These are finite signal checks, not listening or
bit-equivalence claims. Split30+30 PCM equals uninterrupted60 exactly, including
final state; an ordinary boot checkpoint produces the same WAV and final state.

Reproduce through CLI and actual MCP:

```sh
python3 tools/audio_smoke.py \
  --rom 'rom/original/Cadash (USA, Korea).md' \
  --start-frame 1900 --frames 60 --split 30 --out .mdx/new-audio-check
```

`--reference path.wav` optionally checks the independently captured sample count
and reports spectral peaks when NumPy is installed. See
[evidence/audio-session.json](evidence/audio-session.json) for the validated build
and gates. ROMs, captures, downloaded dependencies and private builds stay ignored.
The main workspace pins dependencies in `Cargo.lock`; building needs a C++14
compiler. Licenses and provenance are in [THIRD_PARTY.md](../THIRD_PARTY.md).
