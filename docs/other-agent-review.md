# Reviewed parallel Cadash investigation

Reviewed `../test_reverse_kings/space_bunny_journal.md` and source differences on
2026-10-12. The original directory is preserved.

## Adopted and verified in this tree

- Late trace-window idea: `start_step` added alongside existing PC/frame gates,
  with a conjunction test and explicit system-attempt semantics.
- JSONL-to-oracle-record conversion, with shape/range checks and digest output.
- Unhandled-access diagnostics, strengthened with read/write direction,
  CPU/master attribution and complete retention/omission counts.
- Oracle start-count observer idea, corrected to initialize PC, preserve PC
  gating and use separate seen/retained counters. A C stub executes the actual
  hook and checks no-gate, count, PC and combined gates.
- The useful endian observation, with its interpretation corrected: GPGX's
  raw host-word WRAM requires export normalization. Its `READ_BYTE`/`WRITE_BYTE`
  macros use `ADDR ^ 1` on little-endian hosts; this is not a CPU/hardware defect.

## Preserved findings and follow-up

The journal reports Cadash's VRAM matching at frame 360 and 369, then differing
substantially at 370, with frozen bars instead of the title at a later capture.
CRAM already differs at 369, so 370 is a VRAM frontier, not an established first
divergence of all state. Different final register snapshots do not yet prove
different control flow. These findings have not been independently reproduced
by this review; use the journal's commands with verified requested/actual frames.

The proposed open-bus correction is supported by GPGX source, but was not copied:
its PC is only published after the opcode, then becomes stale as extension words
advance the PC. Byte-sized VDP status access is also not covered. It needs
instruction-level tests before integration. King's Bounty gates alone cannot
validate those unexercised paths.

The other tree's latest `oracle_trace.inc` is unfinished: PC is uninitialized,
PC gating is ignored and skipped calls consume the record budget. All three
private build manifests there refer to older hooks. Earlier trace evidence may
remain useful if anchored to its actual build; it does not validate that edit.

The frame-comparison helpers were not copied: caches keyed only by frame and
first-glob-match buffer selection can reuse unrelated/stale results. A reusable
replacement should key on ROM/core/operation digests and exact run parameters,
consume the response's artifact references and verify lengths before diffing.

Next Cadash probe: capture an instruction-aligned window near the reported
transition, compare CPU state and bus writes, and locate the exact consumed
value before changing timing or decoding. Keep the oracle's execution unmodified.
