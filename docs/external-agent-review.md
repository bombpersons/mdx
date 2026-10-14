# External-agent MDX findings review

Reviewed 2026-10-13 against the canonical checkout at
`/home/maisy/opencode_projects/mdx`.

Source report: `external_agent_notes/MDX_HICCUPPS.md`, findings 1–14 and the
unnumbered source-read fallback. The original notes were left intact.

## Outcome

Implemented confirmed tooling defects and bounded usability improvements:

- Header output includes the first 64 vector slots with correct 68000 names.
- Disassembly reports effective cartridge offsets, states the bus window and
  mapping assumptions, and preserves bus PCs across ROM mirrors.
- Boot treats reached PC/frame targets as successful alternative stops and
  identifies genuinely missing targets correctly.
- RAM ranges accept numeric or decimal/hexadecimal text, validate full aliases,
  and wrap row labels consistently with the bytes read.
- Replay files accept an optional `note`; their schema is discoverable for
  boot/trace/watch. Trace start-PC semantics are explicit.
- CPU conformance accepts the bundled regression JSON offline, using the same
  case executor as the gzip-directory path and retaining honest provenance.

Changes are confined to canonical `mdx` tooling, tests and documentation, plus
this review. Existing uncommitted CPU, FM, system and oracle work was preserved.
No commits were created. `docs/verified-cartridges.md` was not edited.

## Finding-by-finding disposition

| Finding | Disposition | Verification and action |
|---|---|---|
| **1 — word branch targets need PC+4** | **Rejected: incorrect hardware rule** | For original 68000 BRA.W/BSR.W/Bcc.W, displacement is relative to instruction PC+2, the extension-word address. DBcc also uses the extension-word address. The report's BSR at `0x200` with displacement `0x10` correctly targets `0x212`. Existing `mdcore` BRA/DBF branch-base tests pass; the new mirrored-disassembly test preserves this base. No CPU/decoder opcode changes. |
| **2 — disasm has no cartridge offset/window clarity** | **Accepted and implemented; severity overstated** | Previously only `from`/instruction addresses were exposed. Now top-level and per-instruction `cart_offset` accompany the bus coordinates, and coverage explicitly names `0x000000..0x3FFFFF`. A valid `0x020200` cannot be recognized as a caller's intended `0x200`; it is not evidence of encryption. An additional real defect was found: the old cursor folded to ROM length after each instruction, losing the requested bus coordinate. Fixed and tested across mirrors and aperture boundaries. |
| **3 — header truncates vectors and misnames 12–15** | **Accepted and implemented, with terminology correction** | `mdcore::cartridge::vectors` previously supplied only 16 slots and incorrect names. The tooling now reads ROM bytes `0x00..0xFF` directly, reports 64 entries, and names interrupt/trap/reserved slots correctly. VBlank is slot 30 at ROM offset `0x78`; HBlank is slot 28 at `0x70`. Slots 16–47 are not all interrupt vectors: 16–23 are reserved, 24–31 are spurious/autovectors, and 32–47 are TRAPs. The entire 68000 table has 256 slots; this API reports its first 64, not all 256. |
| **4 — bundled CPU fixtures cannot be run offline** | **Accepted and implemented** | The previous API required `suite/v1/*.json.gz`. It now also accepts the existing `sources`/`cases` bundle format. The 12 pinned CPU regression records pass with cycle comparison through both bundle and generated gzip-directory input. Filename filtering, unmatched filters, malformed membership, real comparison failures and bounded failure-detail retention are tested. `BRA` still legitimately matches no cases in this particular bundle. |
| **5 — boot response is verbose** | **Accepted usability concern; deferred API extension** | Boot contains bounded prefix/tail/device samples and counters. The registry already spills responses above 64 KiB to a verified `result_artifact`, and every boot supplies `snapshot_artifact`; this does not reduce a 20 KiB inline response. `trace: 0` controls the prefix only. A caller-selected compact/artifact result mode would be reasonable, but is not part of this tooling patch. Do not silently drop evidence or suppress legitimate counters. |
| **6 — Z80/FM register accesses are unmapped** | **Mixed: register-map claims rejected; hardware follow-up deferred** | `$A04000..$A04003` is the 68000 YM2612 window, not the Z80 bank register. Z80 RAM is mirrored through `$A00000..$A03FFF`; `$A02000` is not the YM2612 base. The serial bank register is at the Z80-side `$6000` window (68000 `$A06000` when accessible). Current canonical `memmap::region` identifies `$A04000` as FM and `$A11100` as Z80 control; `System::write8/write16` handle these writes. Thus the report's write-only explanation is not reproduced by current source. Control **reads** still fall through unhandled arms in `System::read8/read16`; that is a concrete parent-owned hardware follow-up. No hardware implementation or access-counter suppression was changed. |
| **7 — stop_at success becomes partial/frame_target** | **Accepted coverage bug; termination detail not reproduced verbatim** | Current source already sets `termination: "stop_point"` on a PC stop, but its final coverage incorrectly required both requested PC and frame targets to be reached and always blamed a frame target for missing stops. Fixed to make targets alternatives. A stop at reset, zero instruction budget at an already-reached PC, and arrival at a PC on the final budgeted step all succeed. `frames: 0` correctly means a reached frame target with zero executed steps. Missing frame/PC targets and timeout/cancellation remain explicit. |
| **8 — replay annotations unavailable and shape undiscoverable** | **Accepted usability improvement** | Strict unknown-field rejection is deliberate and useful for detecting typos. Added one supported optional `note` field, not arbitrary extra keys. Schema discovery includes `file_schemas.controller_replay` for boot/trace/watch; buttons, event fields, frame/port semantics and ordering constraints are documented. An annotation has no controller effect but remains part of the source-file hash. `_comment` remains a typed error. |
| **9 — Plane B should use register 3** | **Rejected: incorrect hardware rule** | Mode-5 Plane A uses R2, Window R3, Plane B R4. `graphics::run` correctly computes Plane B as `(R4 & 7) << 13`. A reported `0xE000` corresponds to R4=7, not a synthesized addition to R3. R17=0 alone does not establish a full-screen window; window position/direction and vertical selection matter. No graphics/VDP changes. |
| **10 — stdin EOF cancels an MCP request** | **Accepted observed lifecycle behavior; documented, not a defect fix** | The dedicated stdin pump cancels the server token on physical EOF, and worker cancellation follows transport lifetime. Existing MCP disconnect tests pass. The handbook now explicitly tells scripted clients to keep stdin open through initialization and pending responses. No change to shutdown/draining behavior. |
| **11 — RAM aliases are silently misparsed** | **Reported numerical corruption not reproduced; related real usability/label defects fixed** | Canonical `ram::run_controlled` already uses `fold_to_wram`, preserving low 16 bits: `0xFFE21C → 0xE21C`, `0x0000E01C → 0xE01C`. The original typed JSON shape required integer tuple elements; textual hex was not a supported input. Added explicit integer/decimal/hex string parsing, with overflow and malformed-text errors. Full non-WRAM aliases now fail instead of being blindly folded. A genuine wraparound defect in row `address`/`wram_offset` labels was corrected. Numeric, unmasked, textual, CLI and real MCP aliases now read the expected authored bytes. The old server/client responsible for the reported corrupted numbers remains unidentified. |
| **12 — SAT uses R22, h-scroll uses units of 0x200** | **Rejected: incorrect hardware rules** | SAT uses R5, in 0x200-byte units, with mode-dependent masking. H-scroll uses R13, in 0x400-byte units. R22 is DMA source-address middle bits, not a sprite-table register. With R5=`0x54` and R13=`0x2B`, the correct bases are `0xA800` and `0xAC00`, exactly the reported output. VRAM resemblance to a graphics queue/pixels does not override register selection. Exporting the complete 64/80-entry SAT does not imply all those entries are visible or linked. No graphics/renderer/VDP changes. |
| **13 — watch cannot observe VRAM/direct register changes** | **Accepted capability gap; deferred** | The existing watch hook observes folded WRAM bus writes; adding VRAM/CRAM/VSRAM would require parent-owned device observation, including DMA and other bus-master effects. No region-selector facade was added without real hooks. The report's correction is right: trace already includes pre-instruction CPU registers, so those can be inspected without a `trace_registers` flag. This is now explicit in the handbook. |
| **14 — start_pc gate semantics unclear** | **Accepted and implemented documentation/schema descriptions** | `probe::run` latches the conjunctive start gates; `start_pc` starts capture rather than filtering later records. Added the explicit statement to the argument schema and handbook. An authored test reaches `0x202` and confirms the next record at `0x204` is retained. |

### Unnumbered source-read fallback

An actual CPU read of `$0000E01C` would select cartridge. **That is not what the
instruction in these notes does.** `mdx disasm` at `$78D8` identifies an
absolute-short `MOVE.L`, with the encoded operand `$E01C`. Original 68000
absolute-short addressing sign-extends the word to `$FFFFE01C`, then the
24-bit bus selects WRAM `$FFE01C`. `cpu/ea.rs::effective_address` already
implements that correctly. The notes' entity-camera conclusion that this reads
ROM longword `0x303C0024` is rejected. The final review patch makes short
operands explicit as `0xFFFFE01C.w`, with an authored decode/execution test.
Structured operand-region annotations remain a separate possible extension.

### Additional corrections to `NOTES.md`

- Do not add two to any word-branch destination. This invalidates the adjusted
  function/dispatch addresses throughout those notes; rederive them from traces.
- `$A148`/`$A1A2` are low cartridge addresses, distinct from `$A10000` I/O.
- Hardware version/TMSS checks are not 32X detection. `$A10003/05` are pad data
  registers; VDP ports are at `$C00000/04`. `$A1000D` is pad-C control, not region.
- VDP R22/R23 hold DMA source bits, not SAT/h-scroll/IRQ-vector registers.
  R12 bit2 is interlace control, not the choice between an h-scroll scalar/table;
  R13 supplies the h-scroll table base in all Mode5 scrolling modes.
- Nametable palette lines are bits14-13 (0..3). Values such as `0x6000` select
  palette3; there is no palette6/10/14. Window coverage needs both position and
  direction settings, not R17=0 alone.
- The graphics-codec and asset-map results have no supplied code/captures in this
  handoff; they remain leads, not independently verified reconstruction assets.

The original external notes are retained unchanged as the source report.

## Evidence and regression coverage

Canonical source inspected:

- `crates/mdx/src/commands/{header,disasm,ram,boot,input,cpu,probe,graphics}.rs`
- `crates/mdx/src/{operations,main,mcp}.rs`
- `crates/mdcore/src/{cartridge,memmap,system}.rs` and the existing
  `cpu/exec.rs::branch_base_tests` (read-only review)
- Existing pinned `crates/mdx/testdata/*-regressions.json`, public operation
  contract tests, and real stdio MCP tests

New `crates/mdx/tests/external_findings.rs` contains seven public-contract tests:

1. Authored vector table with a distinct VBlank entry and correct slot names.
2. Bus-vs-ROM coordinates across mirrors, non-power-of-two mapping and cartridge
   boundaries, with PC-relative decode targets and rejected non-ROM starts.
3. Successful alternative PC/frame targets and partial missing-target outcomes.
4. Known WRAM bytes through folded, full, unmasked and textual aliases, CLI
   parity, malformed addresses and wrapped row labels.
5. Discoverable replay-file schemas, note acceptance, actual controller state
   application, exact source hash and strict unknown-key rejection.
6. Start-PC latch semantics capturing subsequent different PCs.
7. Offline bundle/gzip-directory conformance equivalence, source-byte identity,
   upstream-provenance retention, filtering and malformed/failing cases.

`crates/mdx/tests/mcp_stdio.rs` additionally checks the textual RAM alias, reached
PC-stop success and bundled conformance through the actual stdio server, with
exact CLI/MCP parity and output-schema validation.

Completed checks in the canonical repository:

| Command/check | Result |
|---|---|
| `cargo test -p mdx --release --locked --test external_findings` | 7 passed |
| Final `cargo test -p mdx --release --locked` | 28 passed across unit/integration targets; no failures |
| `cargo test -p mdcore --release --locked branch_base_tests` | Existing BRA.W and DBF tests both passed; no core edits |
| Scoped `aft_inspect` before tests | Completed its available stat-verification phase; authoritative Rust diagnostics unavailable for the external canonical checkout |
| `git diff --check` | Passed |
| Scoped `rustfmt --check --edition 2021` on all changed Rust files | Passed |

Cargo test compilation is the authoritative compile gate here; the AFT result
is recorded as incomplete rather than interpreted as an error-free LSP report.

## Files changed by this review

In `/home/maisy/opencode_projects/mdx`:

- `crates/mdx/src/commands/header.rs`
- `crates/mdx/src/commands/disasm.rs`
- `crates/mdx/src/commands/boot.rs`
- `crates/mdx/src/commands/ram.rs`
- `crates/mdx/src/commands/input.rs`
- `crates/mdx/src/commands/cpu.rs`
- `crates/mdx/src/commands/probe.rs`
- `crates/mdx/src/operations.rs`
- `crates/mdx/tests/external_findings.rs` (new)
- `crates/mdx/tests/mcp_stdio.rs`
- `docs/mdx.md`

In the analysis project: `docs/external-agent-review.md` (this file).

## Residuals and parent handoff

- All-ROM oracle fidelity is the parent task. This review supplies tooling
  regressions and rejects incorrect hardware recommendations; it is not a new
  all-ROM pixel/audio/cycle-equivalence result. The notes' gameplay-fidelity
  praise was not independently re-established by these synthetic API tests.
- Preserve the pending parent CPU/FM/system/oracle changes when integrating
  this patch. Rebuild/reconnect an installed MCP binary to expose revised
  schemas and behavior; compiling the canonical checkout does not reconnect an
  already-running external server.
- Follow up the Z80 control read path and real serial-bank behavior in the
  hardware-owned work, using access direction/width and exact addresses.
- Compact boot responses, non-WRAM write observation, and structured operand
  region annotations remain deferred capability extensions.
- The historical RAM corruption/server-version mismatch remains unresolved;
  regression coverage verifies the current canonical API and executable.
- Bundled conformance proves only the finite supplied records. It does not
  supply missing BRA fixtures, full upstream coverage or bus-order proof.
