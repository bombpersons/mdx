# Cadash-agent findings review

Reviewed against current canonical MDX and independently executed Genesis Plus
GX. Original notes remain unchanged at
`external_agent_notes/cadash_agent/mdx-tools.md`.

## Controller claim: rejected, with exact reproductions

The unpatched ROM receives Start correctly. All five new requested-frame/input
scenarios match every oracle pixel, VRAM/CRAM/VSRAM byte and VDP register:

| Scenario | Frame | `$FF0208..209`, MDX / oracle | Pixels |
|---|---:|---|---:|
| no input | 2200 | `0D0D` / `0D0D` | 0 |
| Start2100–2130 | 2200 | `0D0D` / `0D0D` | 0 |
| Start0–2199 | 2200 | `0D0D` / `0D0D` | 0 |
| no input | 2110 | `0D0D` / `0D0D` | 0 |
| Start held since2100 | 2110 | `0D0D` / `0D0D` | 0 |

No-input2200 is the title screen. Start2100–2130 produces player selection.
Equal `0D` at these addresses is therefore not evidence of missing input.

`mdx disasm` identifies the caller at `$716` and helper **`$738`**, not `$737`.
The helper writes TH low/high at the **data register** and masks only the low
four direction/identification bits at `$75E..76A` and `$78C..798`. It does not
read Start's TH-low bit5 in that result. `0D` is consistent with the standard
three-button identification path, and the unchanged oracle produces it too.
These bytes are not the complete button-state bytes claimed in the notes.

Current `System::io_write` already updates data-port latches at `$A10003/05/07`
and direction control at `$A10009/0B/0D`. `read_pad` resolves TH from both. Data
and direction control are complementary hardware registers, not alternative
access methods. TH-independent eight-button reads would be wrong; moving data
writes into direction registers changes the protocol. No pad workaround applied.

The notes' reproduction also omits the instruction budget. A fresh equivalent
call requesting2200 with the default200000-step cap reaches **frame15**, applies
**zero** late events and leaves two pending. The output is explicitly partial.
The correct budgeted runs above reach2200 and apply both events. The notes may
have used another budget/version, which is not supplied; their "always hangs"
conclusion is nevertheless disproved by the unchanged-ROM/oracle reproductions.

Evidence: `.mdx/cadash-agent-review/summary.json`, each scenario's comparison
JSON, PNGs, raw buffers and original replay files. ROM identity is
`85e0782411cf50ea92a9982dac1942679f738d46a58fdda5d7d82b943512afd8`.

## Shared artifact store: context concern, existing configuration

Update: every operation now accepts per-call `project_root`/`artifact_store`;
see [project-context.md](project-context.md). The following describes the earlier
launch-context behavior and the original report adjudication.

The store belongs to the selected server/process context. It is not an MDX
global owned by whichever ROM/project ran first. A harness can reuse one MCP
server for multiple projects; relative paths then resolve in that server's root.
CLI already provides `--root` and `--store`, and snapshots retain subject/input
identities plus digest-addressed buffer references. Identical bytes can share an
object across ROMs; that is intentional deduplication, not corrupt provenance.

For example, launch a dedicated context with:

```sh
mdx --root /absolute/agent/workspace --store .mdx/artifacts mcp
```

For a shared server, pass explicit absolute input/output paths. The new
`template_create` operation requires an absolute caller-workspace destination,
with a transport test proving it writes outside a different server launch root.

No corruption was demonstrated by the notes. Per-ROM inventory/retention tools
remain potential extensions, not prerequisites for correct immutable storage.

## Patched size/path claim: not reproduced as a size rule

No patch script or malformed file accompanies the notes. Increasing length by
appending two bytes does not rewrite reset-vector bytes. A reset vector changing
to `$FF0200` requires inspection of actual bytes/path/mapping, not a generic
power-of-two prohibition. Real ROMs need not have power-of-two sizes. Preserve
source digests and run `header` before/after patching; do not reject valid sizes.

## Disassembly and register corrections

- `$4E66` is legal **MOVE A6,USP**, not proof of an invalid opcode. Both USP
  directions are supervisor-only according to original GPGX handlers. Review
  found a wrong existing MDX from-USP privilege gate and missing disassembly;
  both are corrected with a16-form privilege/decode matrix regression.
- Valid NOP `$4E71` was also displayed as undecoded in this polling routine.
  Added NOP decode and a regression. `decoded:false` means decoder coverage is
  unknown; it cannot prove bytes are an illegal instruction or executable data.
- `[12]=129` selects **H40**, not H32. The listed plane/SAT/h-scroll bases agree
  for this register state, but that agreement does not validate the H32 label.
- The68000 vector table begins at0, not200. Some ROMs place executable boot code
  in otherwise unused upper vector slots. Trace/function-boundary evidence is
  needed before treating every apparent vector value as a disassembly seed.

CFG traversal, first-unknown stopping, multi-frame capture and explicit store
inventory are sensible capability requests. They have not been implemented as
facades over unsupported behavior. The five fresh input gates and decoder fixes
address the demonstrated findings; original references/replays were preserved.
