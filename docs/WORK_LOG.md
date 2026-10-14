# MDX development work log

This log records durable checkpoints for a possible transition to Codex. Start
with `ACTIVE_HANDOFF.md` for workspace locations and command recipes. Canonical
toolkit source is `../mdx`; ROMs, independent GPGX evidence and external image
gates are in this research checkout. Changes are uncommitted.

## 2026-10-13 — initial finite-video milestone

- Reviewed Hokuto-agent notes, rejected incorrect PC+4 branch targets and VDP
  register/absolute-short interpretations, and fixed confirmed API issues.
- Fixed shared reset/bit-operation/refresh/FIFO/VINT behavior while preserving
  the original12 image gates.17 additional surveys represented27 distinct exact
  frame/input video checkpoints across four ROMs.
- Evidence: `docs/evidence/fidelity-review.json`, frozen
  `testdata/oracle_fidelity/manifest.json`, `docs/fidelity-review.md`.

## 2026-10-13 — remaining live RAM investigation

- EWJ's `$B4C4` palette-fade buffer was consumed live. The first differing CRAM
  read was frame497 at `$24CCE0`. Missing unused readback bits from the next VDP
  FIFO entry caused64/128 byte differences. Canonical readback now matches those
  bits, including VSRAM and byte-VRAM behavior; full WRAM deltas fell77→13 at2800
  and150→22 at6000 with unchanged video.
- KB's large residual was live RNG-generated state. Four actual HV reads and
  two IRQ-tick consumers were identified. Real HV counter reads, aliases and
  latch/rollover behavior are implemented;6840 reference HCounter clocks pass.
  Counter *sampling time* remains different and is not hidden by injecting
  oracle values. The private supplied-input experiment is evidence only.
- An interrupted worker left supported Z80 condition/LD/stack/flags fixes.
  Nine new tests and a seeded512-record actual-driver replay pass. Integrated
  initial-state/audio-stream progress remains a separate open investigation.
- Evidence: `.mdx/residual-review/{ewj,kb,z80}/` and `ACTIVE_HANDOFF.md`.

## 2026-10-13 — native game starter and public generator

- Created independent `../mdx/templates/megadrive-bevy/`, using latest stable
  Bevy0.20.0 / Rust minimum1.97.1. Native sprites render into320×224, with a
  nearest-sampled, centered, aspect-preserving window presentation.
- Added keyboard/gamepad Mega Drive controls and fixed simulation-tick edges.
- Feature `dev-mcp` provides status, exact stepping, held input, pause, native
  screen and actual window PNGs over stdio. Default builds exclude its endpoint.
- Actual XRDP runs selected llvmpipe/Vulkan CPU rendering both automatically
  and with `--software`.20-tick input and repeated frozen-image checks passed.
- Found and fixed an initial black-screenshot race by waiting for rendered
  views/sprites/pipelines. Developer windows run continuously while unfocused.
- Added `mdx template --directory ... --name ...` and MCP `template_create`.
  Absolute client-workspace coordinates and collision refusal are tested with
  a server whose launch directory is elsewhere.10 embedded files include lock,
  source, README and AGENTS instructions; generation does not execute code.
- Final MSAA-off source rebuilt and runtime smoke passed in
  `.mdx/template-review/final-msaa-off/`. A real CLI-generated project is at
  `.mdx/template-review/generated-final/`; its independent build/runtime check
  remains to be completed.

## 2026-10-13 — completed ROM-library download

- Downloaded/checksummed/integrity-checked/extracted all2774 public Archive.org
  archives, with zero failures.2775 payloads total3,947,221,818 bytes.
- Library: `rom/library/nointro-md/`; complete manifest there preserves source
  metadata, archive SHA-1/SHA-256 and each extracted payload's SHA-256.
- Summary: `docs/evidence/rom-library.json`. Bulk files are gitignored.
- Downloader: `tools/fetch_archive_roms.py`, resumable with verified receipts.

## 2026-10-13 — frame-parameter confusion reproduced and clarified

- The quoted claim that numeric `frames:120` becomes string`"120"` was not
  supported. A live MCP call accepted numeric120, then hit the default200000-step
  cap at frame13. With budget5000000 it reached120 using1,587,871 steps and
  serviced61 VINTs. Thus frame count is not serviced interrupts.
- Kept the explicit-budget contract. Added input descriptions, top-level
  `reached_frames`, `frame_target_reached`, `instruction_budget`, and the
  actionable `frame_target_not_reached` warning. Added integer/string and
  masked-interrupt/frame-target regressions. The current default is unchanged.
- Source API rebuilt. The loaded MCP process still advertises17 old operations;
  restart to load template creation and new result fields. CLI is current.
- Artifacts: `.mdx/frame-argument-review/{default-budget,sufficient-budget}/`.

## 2026-10-13 — division/refresh experiment rejected from current engine

- Independent `m68kops.h` shows operand-dependent division costs and selective
  internal refresh suppression. The actual fixed opcode table gives register
  DIVU/DIVS0 fixed clocks and immediate forms4, not an assumed universal4.
- Implemented and unit-tested a timing experiment, but the full frozen image
  gate caught **1263 differing pixels at EWJ6000**. The fresh17-case survey also
  recorded differences. No input, oracle or frozen reference was adjusted.
- Preserved experimental source snapshots and its numeric test under
  `.mdx/residual-review/division-refresh-experiment/`, and the failing captures
  under `.mdx/residual-review/final-current/` (this directory is FAILED evidence).
- Restored only CPU exec/state/System to checkpoint
  `before-division-refresh-timing`. FIFO/HV/byte-readback/Z80/API/template fixes
  remain. Moved the division test out of the current canonical suite.
- Next: rebuild the restored source, verify all four-ROM gates, then finish
  generated-template verification and written Cadash review. Division/refresh
  needs a broader first-divergence scheduler investigation before reintegration.

## Latest verification at this checkpoint

- Before isolating division:176 canonical Rust tests passed,9 ignored;
  frozen EWJ6000 failed as described. Do not present that build as video-exact.
- Both ordinary and developer-template test suites had3 unit tests passing;
  software/automatic real MCP smoke evidence is recorded above.
- `mcp_stdio`:3 tests passed, including explicit client-workspace generation.
- Final restored-core full Rust/video verification is pending below this log.

## Recording further work

### 2026-10-13 — controlled HV phase probes close two real sampling defects

- Added reusable `tools/probes/hv_counter_phase.py`: three tiny authored programs
  read/store32 HV words, then compare with the unchanged oracle. This isolates
  reset/counter timing from game-specific logic and RNG control-flow changes.
- Baseline Mode5 H32/H40 each matched30/32; exact misses occurred at refresh
  boundaries. Mode4 had0/32 because its reset horizontal latch must be zero.
- Original GPGX `m68k_run` applies refresh before dispatch; MDX deferred device
  advancement until after reads. Moved those14 master clocks before bus accesses
  while preserving total elapsed clocks. Mode4 reset/switch latch corrected from
  original `vdp_ctrl.c`, without an arbitrary game/PC-dependent offset.
- `.mdx/hv-phase-review/pre-access-refresh/summary.json` now matches **96/96**
  actual readings across all three profiles. Portable literal reference values
  and `hv_access_phase` regression added to canonical mdcore; run the new test.
- Core rebuilt. Real KB915 sampler completed as
  `.mdx/residual-review/kb/hv-sampling-v2.jsonl`; inspect that consumed-input delta
  next. All-cartridge gate check is required after this last sampling change.


### 2026-10-13 — Cadash external report adjudicated

- New `.mdx/cadash-agent-review/summary.json` reproduces idle, late Start, held
  Start and in-press2110 snapshots against unmodified GPGX. All five have zero
  pixel/video/register differences. Start opens player selection; FF0208/209
  stays0D0D in both cores because helper738 masks identification/direction bits,
  not Start. Default-budget2200 request actually stops at15 with zero late events.
- Written verdict: `docs/cadash-agent-review.md`. Pad patch/TH-independent reads
  rejected; configured context/store behavior documented; malformed-size claim
  lacks the actual artifact; H32 label and illegal4E66 assertion corrected.
- `usp_privilege`16-form matrix passes after annotation fix. Also found actual
  NOP decode omission in the poller and added its decoder regression; run that
  last addition before final default-suite counts.
- Generated optimized shipping build completed. Developer transport remains
  absent in its ordinary dependency graph and `--mcp` is explicitly refused.


### 2026-10-13 — restored video gates and copied-template shipping checks

- `MDX_GOLDEN_BINARY=... cargo test -p mdx --release --test kings_bounty_boot
  --test cadash_boot --test cartridge_fidelity -- --ignored` passed all12
  original gates and the17-case frozen gate after rollback.27 distinct pixel
  checkpoints remain exact.
- Fresh17-case `.mdx/residual-review/restored-final/` also has zero changed pixels
  and equal VRAM/CRAM/VSRAM. It is still a **partial machine-state result**:
  KB4500/6000 last-DMA source registers21/22/23 differ, along with live RNG RAM.
  Do not call that `--verify` survey fully passed; it exits1 for register equality.
- CLI-generated `native-kit-smoke` built with `dev-mcp` and passed real software
  GPU/MCP smoke in `.mdx/template-review/generated-runtime/`.
- That copied project also built optimized release with `--no-default-features`.
  Ordinary feature graph has no RMCP/Tokio; invoking `--mcp` on its ordinary
  binary exits2, prints no protocol stdout and explains the missing feature.
- Cadash notes call4E66 invalid; it is actually MOVE A6,USP. Review exposed a
  wrong existing core comment/gate: reading USP is privileged too in GPGX.
  Both-direction gate/decoder correction is implemented; its new matrix test
  initially hit a Rust integer-type annotation error, now fixed. Run it before
  including this last correction in final counts/binary.


Append each meaningful milestone with actual commands, artifact paths, observed
results and unresolved questions. Never convert a failed/partial result into a
pass by changing requested frame numbers, replay events, oracle behavior or
reference images. Keep `ACTIVE_HANDOFF.md` synchronized with the latest entry.


### 2026-10-13 — consolidate toolkit development and separate game research

- Moved toolkit logs, oracle data, test corpus and validation drivers into mdx.
- Preserved the older modified checkout and its Git history under ignored
  `.legacy/reverse_kings/`; ROMs and archives are local and ignored.
- Moved the routine inventory, overworld mapping and asset probes to
  `../kings_bounty/`. No native game crate existed and none was fabricated.
- Migrated optional cartridge gates into the canonical test suite and fixed
  live probe dependencies, survey default and symbol-extractor ROM location.
- Added a global Codex MCP installer. Registration is blocked in this session
  because `~/.codex/config.toml` is mounted read-only.
- Validation results are recorded in `reorganisation-validation.json`.

Reorganisation verification: release build, 178 default Rust tests, four Python
observer tests, real four-cartridge/walking MCP smoke and all 13 optional
cartridge tests passed (27 distinct frozen image checkpoints). All 2,780 ROM
images under `rom/` are ignored and all 27 pinned input/reference digests match.


### 2026-10-13 — resume timing investigation and project-local artifacts

- Corrected external DMA resume-clock overlap and blank-period batching. The
  46 verified authored transfer profiles match1472 actual HV samples; two large
  active-VRAM profiles remain explicit known gaps. All27 distinct cartridge
  image checkpoints passed after the active correction.
- Isolated shift-refresh suppression:3840/3840 synthetic samples match, but
  shift-only reproduces the1263-pixel EWJ6000 regression. Kept it out of the core.
- Isolated autovector E-clock timing:39/640→633/640 IRQ samples, with seven
  initial-edge gaps and a672-pixel KB6000 regression. Kept it out of the core.
- Probe completion markers now reject unexecuted zero-filled sample buffers.
  Portable fixtures and explicit ignored regressions preserve remaining gaps.
- Added mdx to maisyapp as an active personal project with repository/local path.
- Added per-call project_root/artifact_store to operation schemas and immutable
  contexts. Project inputs, exports, stores, survey children and artifact-ID
  lookups remain isolated across concurrent calls without changing process cwd.
- Updated workspace OpenCode config and prepared a tested global updater. Global
  OpenCode config is read-only here; the user must run the updater in a terminal.
- Full results are recorded in `docs/evidence/resumed-mdx-session.json`.

Final session verification:181 default Rust tests pass (26 explicitly ignored);
five real MCP transport tests and12 operation-contract tests pass, including
concurrent projects, same-project publication, survey scope inheritance and
CLI/MCP parity. Four Python checks pass. Fresh release MCP exercises18 tools,
four cartridges and walking. A per-call project-local KB4500 capture matches
the frozen image exactly and its snapshot ID is retrieved from that project.
The DMA engine passed all12 original and17 survey image gates before the
transport-only scope addition. No commits or commercial-asset additions.

The user updated both global Codex/OpenCode tool entries. Their configured
executables now point to this checkout; no installer step remains outstanding.


### 2026-10-13 — first IRQ boundary resolved; E-clock/shift integrated

- Found the first divergence before IRQ acceptance: MOVE immediate to SR was
  four CPU clocks short. Corrected all30 legal SR/CCR move timing classes.
- First-handler clocks/registers now match all20 independent startup traces;
  all640 IRQ and3840 shift HV samples agree. Enabled both regression suites.
- Status timing + E-clock and the combined shift correction each pass all27
  distinct frozen image checkpoints, reconciling earlier KB6000/EWJ6000 failures.
- 185 default Rust tests (23 ignored), four Python checks and real four-ROM/
  walking MCP pass. Real KB HV inputs improve but still diverge; RNG/sound and
  the two large active-VRAM DMA cases remain open. No reference changes.
- OpenCode/LongCat repeatedly emitted an unquoted project_root value before
  dispatch. Simplified project_root/artifact_store schemas from nullable types
  to optional plain strings, like rom; all18 advertised tools are checked.
  Fresh MCP accepts the exact Ren & Stimpy path and reaches frame120. This
  verifies server compatibility, not the unreproduced LongCat generation path.
- Evidence: docs/evidence/irq-boundary-session.json and .mdx/irq-boundary/.


### 2026-10-13 — KB dispatch timing and Cadash resumable-state priority

- Corrected control EA, MOVEM long refresh, operand-dependent division, SR-write
  IRQ acknowledgement and frame-relative FIFO wait phase. All authored matrices
  and27 frozen images pass;192 default Rust tests (23 ignored) and four Python
  checks passed before adding one ignored known Z80 bus-gap regression.
- Streamed CPU dispatch comparison isolates the first difference at ordinal
  10,184,208/frame943/PC0x133AC: missing70 master clocks on the first Z80 banked
  bus request. Observation-only oracle hook confirms both CPU and Z80 waits.
  The explicit known-gap regression fails as expected; no reference was changed.
- First four real KB HV reads and first five RNG entry/return seeds now agree.
  Call6 return/frame3400 and final live WRAM still differ. Large active DMA open.
- User confirmed the plain-string schema fixes LongCat/OpenCode path calls.
- Read the Cadash agents' feedback and prioritized full machine save/load. Added
  versioned ROM-bound state capture/restore and shared CLI/MCP from_state/save_state
  on boot/trace/watch/ram.dump; relative frame/replay/start gates, immutable IDs,
  state validation, independent concurrent loads and zero-step RAM inspection.
- See docs/save-states.md and docs/evidence/kb-timing-session.json. Existing
  snapshots remain buffer-analysis artifacts; PSG/audio synthesis is not added.


### 2026-10-14 — resumable-state validation complete

- 196 default Rust tests (24 ignored) and four Python checks pass; all27 frozen
  image checkpoints pass again. The final transport subset passes seven tests,
  including inherited branch-override disclosure after restoring a state.
- Real Cadash split2700→4500 equals uninterrupted execution by full machine-state
  digest, all memory buffers and screenshot. Zero-step RAM load preserves the
  artifact; boot/trace/watch loaded runs have identical resulting state IDs.
- Chaining reaches frame13500 over131,278,868 total steps across bounded runs.
  No ROM bytes are embedded; memory-bearing state artifacts stay local/ignored.
- Global release binary rebuilt; final real MCP loads validated state IDs exactly.
  Existing Codex/OpenCode processes need reconnecting for the new schemas.
- Cadash Z80 VBlank interrupt remains unfixed and next in the queue. User asked
  explicitly and was told save states were prioritized; KB bus contention is
  a separate unresolved sound-timing problem. No oracle/reference changes.
- Evidence: docs/evidence/save-states-session.json; local .mdx/save-states/.


### 2026-10-14 — Cadash Z80 VBlank interrupt fixed

- Wired finite VDP INT assertion/clear at H32/H40 delayed VINT/scanline end,
  independent of68000 enable. Split device advancement at sound edges, preserving
  instruction overshoot and pulse expiry during BUSREQ/RESET/long waits.
- Fixed EI shadow, IM0 FF/RST38, IM2 19T entry and push-before-vector ordering,
  and duplicate interrupt clock accounting. All24 authored cases agree with
  unchanged GPGX. 203 default Rust tests (24 ignored), four Python checks and27
  frozen images pass. The later imported-deadline bound passes final core checks.
- Cadash2000/no input: counter0x1FFF3→0; FM port writes0→28038. Oracle counter0.
  Z80 RAM still differs in152 bytes; driver activity is not synthesized audio or
  whole-machine timing equivalence. Known KB70-master-clock gap still fails at
  its unchanged expected boundary when explicitly run; not part of default passes.
- Save format2 includes INT level/EI shadow and rejects format1; fresh captures
  needed. Cadash2700→4500 full-state/memory/image equality and loaded boot/trace/
  watch parity pass again. Final real MCP verifies driver progress and state loads.
- User queued synthesis after current interrupt/timing work; recorded maisyapp
  mdx task1 (offline YM2612/PSG WAV export proposed). No implementation yet.
- Evidence: docs/evidence/z80-vblank-session.json, .mdx/z80-vblank/. Original
  oracle execution, ROMs, replays and frozen images were preserved. No commits.


### 2026-10-14 — Z80 decode and shared-bus checkpoint

- Corrected indexed/ED/CB decoding, EXX, flags/block I/O, refresh R and warm reset;
  all10,752 authored instruction cases agree with unchanged external Z80 execution.
- Newly unblocked PSG writes exposed VDP control corruption. Corrected byte lane
  behavior and port decoding; PSG writes no longer mutate VDP control.
- Absolute sound scheduler now accounts for bank/VDP bus waits, physical line/VINT
  horizons, frame-relative phase, DMA update boundaries and refresh deferral.
  The original frame94370-clock regression passes without changing its expectation.
- Contiguous final PC/master comparisons cover72M dispatches through KB frame6386.
  Fresh original GPGX KB6000 agrees in WRAM, Z80 RAM, all video buffers/registers
  and screenshot. Previously divergent live RNG/WRAM now agrees on this replay.
- 211 default Rust tests (24 ignored), four Python checks and27 frozen images pass.
  350 authored bus/DMA cases contribute11,200 independent HV samples. The large
  active DMA gap remains explicitly failing; no reference was changed to hide it.
- Save format3 stores WZ and absolute sound/DMA clocks. Cadash split2700→4500,
  full-state/memory/image parity and fresh MCP execution/zero-step loads pass.
  Recapture format1/2 checkpoints; existing MCP clients need reconnecting.
- Synthesis is still queued after the remaining DMA investigation, followed by
  broad ROM-corpus failure hunting. No commits. Evidence:
  docs/evidence/z80-contention-session.json and ignored .mdx/z80-contention/.


### 2026-10-14 — initial-frame DMA resolved; sound groundwork

- Verified restarted live MCP serves format3. Fixed external DMA prediction's
  viewport-height latch across reset's192-line frame; both original failing
 4096-word cases now pass unchanged and their ignored gate is enabled by default.
- All96 expanded H32/H40,224/240-line, VRAM/CRAM/VSRAM,16..16384-word cases match
 3072 independent HV samples. All27 frozen images and Cadash2700→4500 complete
  state/memory/image and loaded execution parity still pass; save layout stays3.
- Added original standalone integrated-PSG pulse generator. All176 scenarios /
 3472 operations match unchanged GPGX pulse/phase observations. Serialized phase
  continuation and malformed-period rejection pass. It is not wired into System.
- Corrected reference harness audio callback ABI, added streamed reference WAV
  capture and range/stereo/consumption tests. Cadash1900..1960 gives44157 stereo
  sample frames at44100 Hz. Fresh KB6000 reference RAM/video digests are preserved.
- 216 default Rust tests (23 ignored) and six Python checks pass. Sound synthesis
  remains incomplete: selected pinned BSD ymfm-sys0.2.0 backend cannot be fetched
  because this workspace cannot resolve crates.io. Prepared tools/fetch_audio_backend.py
  and asked the user to run it from their terminal; main dependencies are unchanged.
- Next: validate fetched FM API, integrate per-console synthesis and WAV export,
  preserve deterministic state and independent gates, then broad ROM-library tests.
  Evidence: docs/evidence/dma-viewport-session.json; docs/audio-development.md.


### 2026-10-14 — command networking restored

- Direct IP sockets failed with EPERM under the restricted session. Ask-for-approval
  still rejected escalation before a dialog; its separate permission-request tool
  was unavailable. The user switched this session to Full Access.
- Direct crates.io HTTPS now returns200. Ran tools/fetch_audio_backend.py successfully:
  pinned ymfm-sys0.2.0 downloaded and compiled; nonzero FM tone and chip-state
  continuation pass (53267 native stereo rate,809 state bytes). Workspace manifests
  and main lockfile are still unchanged; audio integration can proceed normally.
- Local evidence: .mdx/audio-backend/fetched-backend.json and fetch-enabled.log.


## 2026-10-14 — deterministic FM/PSG WAV export

- Integrated pinned ymfm-sys0.2.0 with per-console timer/CSM host callbacks and
  independently checked PSG. CPU-visible FM status/timers stay unchanged. Timed
  writes render after safe sound batches; integer44.1kHz stereo integration and
  DC-removal retain fractional phase.
- Added shared audio CLI/API/MCP (19 tools), relative warmup/capture/replay, hard
  step/time/retention bounds, immutable project-local WAVs, optional digest-named
  output directories and explicit partial-run reporting.
- Format4 saves chip/host/mixer/pending-event state, excludes PCM observers and
  validates native blob layout/envelope enums before restoration. Formats1..3
  need recapture; reconnect existing clients after publication.
- 222 default Rust tests (23 ignored), six Python checks and27 distinct frozen
  images pass (29 comparisons including duplicate gates). Stereo panning has an
  idle-chip control; timer/CSM/pending writes survive state; retention bounds do
  not change state; simultaneous project-local recordings remain independent.
- Cadash1900..1960:44157 stereo frames, peak2851/RMS956.89, no clipping/drops.
  Independent PCM has the same sample count/strong frequency components with
  different gain/filtering. Split30+30 PCM/state equals continuous60, including
  a non-recording boot checkpoint. Cadash2700→4500 full state/memory/image and
  boot/trace/watch/zero-step parity also pass.
- Added BSD notices, build/provenance docs and portable audio-session evidence.
  Published exact validated binary; no original oracle execution, ROMs, replays,
  frozen expectations or commits changed. Next: broad ROM-corpus failure hunting.

## 2026-10-14 — ROM-library survey and CPU dispatch fixes

- Added `tools/library_survey.py` (bounded CLI triage of all 2,775 local cases;
  findings are smoke results, not fidelity). Fixed: ROM reads past an irregular
  image (Barney/Cadash +2-byte panic; unbacked tail reads FF, power-of-two
  mirror), exception vectors/software traps (64/64), privilege faults (18/18)
  and STOP/IRQ phase (2/2) against unchanged GPGX.
- CPU dispatch gaps, each reduced to authored GPGX probes in `tools/probes/`
  and frozen in `crates/mdcore/tests/authored_cpu_matrix.rs`:
  MOVEP was never implemented (ran as an immediate op; 6/6); TAS sat behind an
  unreachable arm (memory forms test without write-back, as GPGX/MD; 40/40);
  ADDX/SUBX/NEGX were wrong at every size (NEGX computed dst-dst); CHK had its
  operands swapped; LINK/UNLK A7 mishandled SP; divide-by-zero, CHK and TRAPV
  were never entered (the instruction re-ran forever). Trap-class exceptions now
  stack the following PC with GPGX exception clocks (8/8 family probes, incl.
  stacked frames and HV phase). `rom_tail.rs` freezes the tail mapping.
- Opcode-table sweep vs GPGX: no valid opcode is now rejected; 9,311 opcodes GPGX
  treats as illegal still execute as something in mdx (low priority). The
  SingleStepTests remainder is address-error emulation (not modeled) plus
  documented GPGX-vs-hardware undefined flags (ASR >= size, DIV overflow, CHK).
- 60-frame survey: 2,632 reach the frame target cleanly (was 2,567), 80 show
  software traps, 63 CPU exceptions, 0 incomplete. 171 changed ROMs: 123 equal
  GPGX in pixels/VRAM; with GPGX forced to NTSC-U (new opt-in
  `oracle_gpgx.py --region`), 40/48 of the rest are exact (old binary: 5/48), the
  other 8 all moved closer. Remaining failures mostly need SRAM/EEPROM, region
  selection or mappers (SVP, lock-on, J-Cart).
- 230 default Rust tests (23 ignored) and six Python checks pass. Evidence under
  ignored `.mdx/library-survey/`. Cadash agent feedback triaged (see handoff).

## 2026-10-14 — Cadash agent requests: pokes and sound-command log; template remapping

- Frame pokes: `boot`/`audio`/`trace`/`watch` accept `pokes` (inline, in the
  replay file, or CLI `--poke FRAME:wram|z80:ADDR=HEX`), applied at the frame
  boundary where controller events take effect. Cadash `1830:z80:0x1C0A=98`
  vs `=A5` produce different audio once the driver is live (~frame 1800).
- Sound-command log: `boot`/`audio` `sound_commands` records 68000 writes to Z80
  RAM and the bank register (including writes dropped without the Z80 bus) to a
  JSONL artifact; BUSREQ/RESET are counted unless `bus_control`. The summary
  separates upload frames and ranks command slots: Cadash 0..6000 leads with
  `0x1C0A` from PC `0xA3C`, 13 sound ids. Observer only; not saved in states.
- Template: `rebind.rs` (capture next key/pad press, swap on conflict, persist
  `bindings.json`), `controls_menu.rs` (movable Mega Drive-style example inside
  the 320×224 framebuffer, F1/Select, pauses play) and `pixel_font.rs` (5×7).
  Template tests, dev-mcp and release builds and a generated `--locked` project pass.
- 232 default Rust tests, six Python checks and 13 cartridge image gates pass.
  Reconnect MCP clients to see the new `pokes`/`sound_commands` fields.

## 2026-10-14 — VGM export and longer audio captures

- `audio` `vgm` (CLI `--vgm`) exports YM2612/SN76489 register writes as VGM 1.60,
  starting with writes that recreate both chips' state at capture start. FM
  state comes from a new register shadow saved (optionally) in audio state;
  older format-4 states still load and report `fm_initial_state_known` false.
  Independent check with libgme on Cadash 1800..2700: equal sample count, 0.938
  loudness-envelope correlation, dominant pitches within 2-4 Hz; gain differs.
  The summary reports key-ons per FM channel (Cadash title never uses FM3).
- `audio` now allows 18000 frames (five NTSC minutes) and 1e9 steps: a
  3-minute Cadash capture with VGM takes 17 s with no dropped samples.
- 234 default Rust tests, six Python checks and 13 cartridge image gates pass.

## 2026-10-14 — Z80 disassembler

- `mdcore::z80::disasm` decodes all Z80 prefix groups (CB, ED, DD/FD, DDCB/FDCB,
  undocumented forms). Every non-branching opcode in every group has its length
  cross-checked against the independent executor; 33 representative mnemonics
  are pinned. `disasm` takes `cpu: "z80"` with a ROM offset plus `origin`, or
  `from_state` to read uploaded Z80 RAM. Cadash's live driver decodes as a
  standard SMPS layout (di/im 1, YM busy-wait at 0x0008, RST vectors).
- 237 default Rust tests pass.

## 2026-10-14 — Z80 trace and Z80 RAM watch

- `trace` `cpu: "z80"` records executed Z80 instructions with registers and
  disassembly (optional `z80_pc_range`); `watch` `space: "z80"` records Z80 RAM
  writes by either CPU with writer, PC and before/after values. Observers only,
  gated off the hot path unless installed. Cadash: 68000 PC 0xA3C writes 0x81 to
  0x1C0A at frame 383; Z80 PC 0x08B8 clears it ~8000 master cycles later.
- 238 default Rust tests and 13 cartridge image gates pass.

## 2026-10-14 — per-voice audio stems

- `audio` `stems` renders FM1..FM6 and PSG1..PSG4 WAVs by re-synthesizing the
  register log from the exact synthesis state at capture start (the log now
  carries it, and FM resets are logged events). Muting: FM total level 0x7F and
  key-off, DAC disabled unless FM6, PSG muted in state. With every voice kept the
  re-synthesis equals the capture bit-exactly from power-on. Isolated stems start
  with the output filter at rest on their own level: silent voices are exactly
  zero and removed voices cause no click. Stems sum to the mix within the YM2612
  ladder nonlinearity. Cadash 1800..2700: FM3/PSG stems exactly silent.
- All Cadash agent requests in mdx_issues_new.md are done.
- 240 default Rust tests, six Python checks and 13 cartridge image gates pass.

## 2026-10-14 — cartridge backup RAM, mappers and console region

- New `mdcore::cart_hw` following GPGX defaults: header `RA` backup RAM (with
  generic range fixes; EEPROM-declared headers excluded), default 64 KiB SRAM at
  $200000 for ROMs <= 2 MiB, the $A130F1 SRAM switch and write protect, and the
  $A130F3.. 512 KiB bank registers. The fallback multi-game mapper below $A13060
  is not applied and those addresses stay unmapped (pirate protection reads).
  Saved in machine states (older states restore power-on values). `boot` adds an
  `sram` memory artifact and `sram` input.
- `sram_matrix` probe: 4/4 authored cases (default SRAM, header SRAM, overlapping
  4 MiB switch + protect + switch back) match unchanged GPGX; frozen as a test.
- Console region: header auto-detection like GPGX (USA > Japan > Europe), `region`
  option on boot/audio/trace/watch/ram.dump, `console_region` in boot. Europe sets
  the PAL version bit only; PAL timing is not modeled.
- 728 SRAM/large ROMs at frame 120 vs GPGX (both NTSC-U): 693 exact pixels+VRAM
  (was 612), 84 closer, 0 worse. Found: GPGX returns open bus for unmapped reads
  beyond the cartridge window; mdx returns 0 (rom_tail 3 MiB case).
- 243 default Rust tests and 13 cartridge image gates pass.
- Japan-only ROMs (504) at frame 120, mdx auto region vs GPGX auto: 459 exact
  (309 with the old always-US console), 166 closer; Virtua Racing (SVP) differs.
- Open bus: unmapped 68000 reads return the next instruction-stream word (CPU
  reports its PC to the bus); unmapped Z80 bank-window reads return FF, as in
  GPGX. rom_tail probe now 4/4 including the 3 MiB case; frozen in rom_tail.rs.

## 2026-10-14 — PAL timing, status details, King's Bounty feedback (part 1)

- PAL consoles (europe/auto) run 313-line frames, PAL V-counter wrap points,
  status PAL bit, the reference PAL reset phase and the PAL master clock for
  audio/VGM. 312 PAL-only ROMs vs GPGX real PAL at frame 120: 282 exact pixels+
  VRAM (153 with NTSC timing), 141 closer, 0 worse. pal_phase probe frozen.
- VDP status: unused bits 15-10 return the 68000 open-bus word; HBLANK is the
  reference's per-line window (H32 280..860, H40 228..872 master clocks). Known
  limitation: the reference evaluates status at the end of the reading
  instruction (opcode cycle table), so HBLANK can differ within one instruction
  of an edge (masked in the frozen PAL test).
- Template (KB agent notes): asset root policy (--assets, GAME_ASSET_DIR, beside
  binary, cwd, project), RequiredAssets readiness gate with named failures,
  --headless (no window/display; verified via MCP with llvmpipe and no DISPLAY),
  GameStatus map in game_status.
- mdx: `out` documented as a directory, boot `profile: "summary"` (Cadash 33.7 KB
  to 5.2 KB), indexed max-compression PNGs (identical pixels, 4.5x smaller) with
  screenshot_pixels_sha256; test PNG readers expand palettes.
- 245 default Rust tests and 13 cartridge image gates pass.
- Whole library at frame 120 vs GPGX auto region: 2526 of 2768 exact pixels+VRAM
  (91%); status/HBLANK changes moved 4 closer, 0 worse.
- KB notes part 2: graphics `sprite_groups` (explicit indices, transparent RGBA,
  flips/palettes/front order), new `image.compare` operation (20 MCP tools).
- 247 default Rust tests and 13 cartridge image gates pass.

## 2026-10-14 — serial EEPROM and J-Cart

- `mdcore::eeprom` ports the reference I2C state machine; `cart_hw` adds its
  game table and header fallback, EA/Sega/Acclaim 16M/32M/Codemasters wiring and
  J-Cart controller ports (replay ports 2-3; oracle runner and probe harness
  accept them). eeprom_matrix (page wrap, sequential read rollover, every ACK and
  data level) and jcart_matrix (word/odd/even reads, TH high/low, three button
  sets) match unchanged GPGX and are frozen as tests.
- 108 EEPROM/J-Cart library ROMs at frame 300 vs GPGX: 100 exact (was 84), 0
  worse; remaining: 7 Game no Kanzume dumps (frame size differs), a Ristar beta.
- 251 default Rust tests and 13 cartridge image gates pass.

## 2026-10-14 — scanline renderer and access-phase fidelity

- `mdcore::vdp::scanline` ports GPGX's mode-5 per-line renderer (non-ALT, the
  libretro default): lines render at line start from live VRAM/VSRAM/registers
  with colours resolved from CRAM at that instant; internal sprite-attribute
  cache (writes into the current table only); sprite count/pixel limits and
  masking; shadow/highlight; window rules incl. the partial-column bug; H40
  left-column vscroll; HBLANK re-render/remap on reg 1/2/3/4/5/7/12 and CRAM
  writes; SOVR/SCOL status bits (cleared on read). Boot screenshots use it; the
  snapshot renderer stays for `render` of VDP snapshots. Mode-4 register lock
  (regs 11-23) as in vdp_reg_w.
- Access phases found with instruction/clock traces against the instrumented
  oracle (Streets of Rage, El.Viento, Puyo Puyo now exact in registers and master
  clocks to frames 198/125/121):
  - 68000 VDP status sampled at dispatch + opcode table cost (HBLANK, FIFO,
    DMA busy, early VINT flag); the old "status at instruction end" limitation.
  - Register write enabling a pending VINT/HINT delays acceptance by one
    instruction (not MOVE.L), and that instruction skips the refresh check.
  - YM2612 BUSY/timers: chip syncs to each access; 68000 accesses include the
    7-clock Z80-bus latency, Z80 accesses dispatch + opcode T-states; BUSY end
    rounded relative to the frame origin.
  - DMA fill started late in a line: the first-update budget wraps, the fill
    completes at once and BUSY lasts to the frame edge.
  - HINT: line 0 now evaluates its counter (was skipped at the 262->0 wrap);
    the VBLANK edge raises HINT when the count is zero; only line 261 reloads.
  - 68000-bus DMA data arrives per reference update (line / whole VBLANK) so
    the renderer and sprite cache see partial progress.
- New authored probe tools/probes/access_phase_matrix.py (146 cases: status
  phase x3 forms x H32/H40, IRQ delay, 68000 and Z80 FM BUSY, HINT sequencing,
  fill wrap), all equal to unchanged GPGX; frozen as tests/access_phase.rs. The
  pre-fix binary fails 60+ of them.
- Whole library at frame 120 vs GPGX (renderer + status + IRQ delay build):
  2603/2768 exact pixels+VRAM (was 2541), no regressions except Puyo Puyo
  (Japan), fixed by the HINT change.
- Further reference behaviours found by CPU/Z80 trace bisection:
  - Delayed interrupt acknowledge phase is sampled before the nested
    instruction's table cost (Pitfall).
  - Pads: automatic 3-/6-button type from the header I/O field, 6-button TH
    pulse counter with its 27-line timeout, 172-clock TH input switching latency
    on every pad (Pitfall; probe 'pad' cases fail 12/12 on the old build).
  - Control port: every first command word updates address (with the latched
    A15-A14) and code bits 1-0, so a register write leaves code 2 and later
    data writes are dropped (Shadow Dancer); mode 4 has no pending second word;
    a control word written during 68000-bus DMA waits for its completion.
  - Region: checksum-keyed overrides (Alisia Dragoon Europe, Radica Sensible
    Soccer, On Dal Jang Goon).
  - YM2612 chip advances only on access (and from reset release onward on the
    reference's frame-0 sample grid), never ahead of a sound-CPU read (Alisia
    Dragoon Z80 timer polling).
- access_phase probe: 160 cases (adds pad, ctrl), all equal; frozen.
- Library (pad build): 2671/2768 exact, no regressions. Alisia Dragoon, Pitfall,
  Shadow Dancer exact in both CPUs' traces to frame 121.
- More reference behaviours (trace-bisected, each confirmed exact afterwards):
  - Sonic & Knuckles leaves $200000-$3FFFFF unmirrored (FF), no default SRAM;
    ROMs pad with FF to a 64 KiB minimum mirror period; missing-header SRAM
    table (PGA Tour Golf, Winter Challenge, Buck Rogers, Xin Qigai Wangzi, Radica,
    SF-001/004, Psy-O-Blade).
  - Boards: QuackShot Rev A 512 KiB address wiring, Barkley Shut Up and Jam 2
    SRAM across the upper 2 MiB, Triple Play SRAM/ROM split.
  - BUSACK and Z80-window open bus use the word at the 68000's current PC
    (Time Killers).
  - Fill/copy scheduling re-latches the viewport at frame edges (Arnold Palmer
    Tournament Golf: 64 KiB fill across a mode change).
  - Region bytes >= $80 act as signed chars (Scrabble proto).
  - A refresh delay that crosses a line edge stays in the old line's run: the
    instruction sees that line's state while its accesses are timed later
    (deferred into elapsed time with an access bias; Scrabble).
  - VDP power-on: register 10 and the HINT counter start at $FF (Beastball).
  - Optional `System::vdp_control_log` records every control-port word.
- Library (DMA-fix build): 2697/2768 exact, no regressions.
- Cartridge hardware database: `mdcore::cart_db` carries the reference's 71
  `rom_database` entries, generated from its source (header + computed checksum),
  with its protection-register handlers (default, 16-bit, custom bit-swap,
  Tekken, Top Shooter), !TIME read/write handlers (registers, 128K/256K multi,
  Radica 64K/128K), the 32K/64K custom bank mappers, the default multi-game
  mapper below $A13060, and the Realtec boot ROM + /VRES pulse reset. Reset
  vectors are read through the cartridge mapping. SF-001/002/004 boards
  (per-bank open bus/SRAM, lock) and the Super Mario World 64 board. The whole
  $A130xx range now goes through !TIME handlers (open bus when none).
- Z80 MEMPTR (WZ) follows the reference core for every instruction that sets
  it (BIT n,(HL) flags 3/5).
- FM frame edge: the next frame's sample counter restarts at the first grid
  point after the edge, re-running samples an overshooting sound-CPU access
  already ran (Super Mario World 64 timer polling).
- Library (database build): 2728/2768 exact, no regressions. Remaining:
  Mega-CD/LaserActive BIOS (CD hardware, out of scope), Virtua Racing (SVP),
  Sega Channel, MD Soft Checker, Klax beta, Taipingyang, Top Pro Golf (capture
  taken after an instruction stalled across the frame edge by 68000 DMA), and 7
  titles the oracle harness cannot run.
