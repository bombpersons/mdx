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

The PSG pulse fixture is produced by the authored `tools/oracle_psg_probe.c`
harness executing the caller's unchanged GPGX `psg.c`. No reference PSG source or
audio filter is vendored. `mdcore::psg` is original Rust code; attenuation levels
are calculated from the two-decibel law. Agreement covers logical pulse changes
and phase, not the reference's PCM filters or analog output.

## Dependencies

Rust dependency versions and checksums are pinned in `Cargo.lock`. Dependencies
retain their own licenses; no dependency source is vendored into this repository.


## Authored counter-timing reference samples

`crates/mdcore/tests/fixtures/{dma_hv_samples,irq_hv_samples,shift_refresh_samples}.json`
contains numeric observations from original authored programs executed by the
unchanged Genesis Plus GX core. Each fixture retains reference-core/source
SHA-256 identities. No commercial cartridge bytes or upstream emulator source
are included. Known timing gaps are retained as explicitly ignored regressions;
see `docs/timing-investigation-2026-10-13.md` for scope and failed experiments.

## Authored Z80 instruction observations

`crates/mdcore/tests/fixtures/z80_instructions.json` records authored flat-bus
instruction cases run by the unchanged external GPGX Z80 core. The local probe
compiles that caller-supplied source with an mdx-authored harness; neither oracle
source nor compiled reference code is distributed. Source/header/harness hashes,
fixed data/register initialization, and limits are retained in the fixture.
HALT PCs are reported as resume addresses; final written bytes are compared by
address, without claiming equivalent callback order or sub-instruction timing.
The cases are finite execution observations, not proof of full Z80 or hardware
accuracy. `tools/probes/z80_instruction_matrix.py` reproduces the observations.


## YM2612 synthesis dependency

`mdcore` uses [`ymfm-sys` 0.2.0](https://github.com/h1romas4/ymfm-sys),
a Rust/CXX adapter to [Aaron Giles's ymfm](https://github.com/aaronsgiles/ymfm).
The crate is pinned exactly in the manifest and checksummed in `Cargo.lock`.
The downloaded package bundles ymfm at commit
`81aec25ccbb98f4873a255f7551ac4dadac59b4a` (documented in the downloaded package README).
Both use BSD 3-Clause licenses: copyright2026 h1romas4 for the adapter and
copyright2021 Aaron Giles for ymfm. Required notices/disclaimers are retained
in [licenses/ymfm-sys.txt](licenses/ymfm-sys.txt) and
[licenses/ymfm.txt](licenses/ymfm.txt). No backend source is copied into mdx.
The C++ backend is linked into mdcore; mdcore's Rust unsafe-code prohibition
continues to apply to its own code. Audio quality/timing limitations are described
in [audio-development.md](docs/audio-development.md).
