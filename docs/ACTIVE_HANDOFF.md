# Active mdx handoff — 2026-10-14

Canonical workspace: `/home/maisy/projects/reverse_engineering/megadrive/mdx`.
Native King's Bounty research is in sibling `kings_bounty/`; no game crate exists.
Old modified source/history is preserved under ignored `.legacy/reverse_kings/`.
ROMs, oracle caches, builds and artifacts are local and ignored. Preserve all
uncommitted work; do not modify original oracle execution, replays or references.

## Current engine checkpoint

Z80 decoding, PSG/VDP port separation and shared-bus scheduling are corrected.
The formerly missing70-master-clock KB stall at frame943 passes its unchanged
external regression. The prior final candidate matched72M contiguous dispatch
PC/master clocks from boot through frame6386. KB6000 had exact WRAM/Z80 RAM/
VRAM/CRAM/VSRAM, VDP registers and pixels against fresh unchanged original GPGX;
the prior RNG/live-WRAM gap is closed on that replay. This remains finite evidence.

Both initial-viewport large active-DMA cases are fixed and enabled in the default
suite; all96 expanded viewport/transfer cases pass. Independent authored fixtures
cover10,752 Z80 cases and350 two-CPU bus/DMA cases (11,200 HV samples).
See [z80-contention-2026-10-14.md](z80-contention-2026-10-14.md) and
[dma-viewport-2026-10-14.md](dma-viewport-2026-10-14.md).

251 default Rust tests pass (23 ignored), six Python checks and all27 distinct
frozen image checkpoints pass (29 comparisons including two duplicate gates).
Per-console FM/PSG synthesis and shared audio CLI/API/MCP WAV export are complete:
[audio-development.md](audio-development.md), [evidence/audio-session.json](evidence/audio-session.json).
Cadash1900..1960 yields44157 stereo samples with no clipping/drops; split30+30
PCM and final state equal continuous60 exactly. An ordinary boot checkpoint emits
the same WAV. Independent reference PCM has the same sample count and strongest
frequency components; analog filtering/gain and hardware PCM equivalence remain
outside the claim. FM access timestamps use modeled CPU/Z80 access phase.

Cargo network access works after the user enabled Full Access. The backend is
pinned ymfm-sys0.2.0 (C++14 build requirement, BSD notices under licenses/).
Local Cargo cache/private build/evidence live under ignored .mdx/audio-backend/.
The exact validated binary is published to target/release/mdx. Historical failed
binaries/logs remain preserved; no oracle execution, ROMs or references changed.

## Resumable states and Cadash sound driver

boot/trace/watch/ram.dump/audio accept from_state/save_state with immutable
project-local IDs. Current **format4** includes CPUs, memories, devices, absolute
sound/DMA clocks, native FM/host and PSG phase, pending writes and fractional
mixer state. ROM bytes and PCM/write/trace observer histories are excluded.
Formats1..3 need recapture. Frame/replay gates are relative to the loaded state;
absolute stamps and starting_frame/elapsed_frames remain. Audio frames is capture
duration after its relative start_frame warmup. Optional path schemas remain
plain strings for LongCat/OpenCode compatibility.

The format4 Cadash2700→4500 split matches uninterrupted full state, memory and
screenshot. Fresh MCP zero-step loads and boot/trace/watch agree. Historical
state chaining reached13500 over131M steps. See [save-states.md](save-states.md).
The earlier silent/waiting sound driver was resolved by finite Z80 VBlank INT and
corrected EI/IM0/IM2 entry; synthesized PCM now has its own verification above.
Existing Codex/OpenCode clients must reconnect to see 20 tools and new fields;
registration paths already point to target/release/mdx.

## Current project-context work

Every operation now accepts optional `project_root` and `artifact_store`.
Relative inputs/outputs resolve per call; a selected project owns its default
`.mdx/artifacts`. Context cloning keeps concurrent requests independent. Surveys
inherit scope, and snapshot/large-response finalization uses the selected store.
There is no per-request process-directory change. CLI launch flags remain valid.

[project-context.md](project-context.md) gives examples and registration details.
The Codex registration was installed by the user before this continuation.
The user updated both global Codex and OpenCode entries; their binary paths
were verified. Workspace path schemas now advertise optional plain strings;
a direct Ren & Stimpy MCP call reaches frame120. The user confirmed this fixes LongCat/OpenCode
tool-call generation. Workspace overrides and the reusable global updater remain
available. Restart/reconnect clients after replacing an already-running MCP.

## Completed earlier work

- Native Bevy template, software rendering and real generated-project MCP/
  shipping-build checks are complete. Source: `templates/megadrive-bevy/`.
- Cadash report adjudicated against unchanged GPGX; see `cadash-agent-review.md`.
- All2774 ROM archives were downloaded and checked; local corpus is `rom/library/`.
- Frame-budget errors are explicit; templates use the caller's absolute path.
- Real HV/FIFO readback and supported Z80 semantic corrections are present.

## Next investigation

1. ROM-library failure hunting (maisyapp task2) continues; see WORK_LOG. Done:
   CPU dispatch fixes, backup RAM/$A130F1/512K mapper, console region with PAL
   timing, open-bus reads, serial EEPROM, J-Cart. Whole library at frame 120 vs
   GPGX auto region: 2526/2768 exact. Open: SVP, lock-on, 9,311 opcodes mdx
   executes but GPGX treats as illegal, address-error emulation, status reads at
   instruction end (HBLANK edge), residual GPGX differences.
2. Cadash agent feedback (`../external_agent_tests/cadash/mdx_issues_new.md`)
   is complete: panic fix, pokes, sound-command log, VGM, 18000-frame audio,
   Z80 disassembler, Z80 trace/watch and per-voice stems. Clients must reconnect
   to see the new fields.
3. Older Cadash usability backlog: compact boot responses, trace PC ranges,
   multi-frame captures, xrefs/recursive disassembly, force-branch hit validation.

4. King's Bounty agent notes (`../external_agent_tests/kings_bounty/MDX_NOTES.md`):
   done: template asset root/readiness/headless/GameStatus, out wording, boot
   summary profile, indexed PNGs, sprite groups, image.compare. Open: a
   replay/capture manifest tying ROM replay frames to native MCP steps.

Latest chronology: [WORK_LOG.md](WORK_LOG.md). Frozen pixel equality is a finite
video result, not whole-game, RAM, sound, cycle or hardware equivalence.
