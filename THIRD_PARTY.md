# Third-party provenance

## CPU regression records

The four JSON files in `crates/mdx/testdata` contain selected independent test
records from [SingleStepTests/680x0](https://github.com/SingleStepTests/680x0).
Each preserves its upstream compressed-file SHA-256 and selected case names;
some RAM/register addresses were relocated for this tool's fixture harness.
These are validation data, not generated expectations from mdcore itself.

No upstream LICENSE file was returned by GitHub's license endpoint when the
standalone repository was prepared on 2026-10-12. The project's MIT OR Apache-2.0
declaration does not relicense those independent records. Upstream attribution
and original record identities are retained here.

## Optional reference emulator

Genesis Plus GX is an optional, separately obtained reference emulator.
No downloaded oracle source or binaries are included. Utilities under `tools/`
operate on a caller-supplied local checkout/build. Consult that emulator's own
license before distributing its source or binaries.

## Dependencies

Rust dependency versions and checksums are pinned in `Cargo.lock`. Dependencies
retain their own licenses; no dependency source is vendored into this repository.
