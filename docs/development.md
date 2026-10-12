# Development and repository boundary

This standalone workspace was extracted from `reverse_kings` on 2026-10-12.
It includes reusable engine/tool source, synthetic tests, CPU regression fixtures,
generic observer utilities and API documentation. Game-specific captures,
assets, cartridge experiments and fidelity gates remain with their game project.

`mdx` is the toolkit source-of-truth after this split. The originating project
currently retains its working snapshot; changes should be ported explicitly
rather than relying on symlinks or a sibling checkout to compile this repository.

## Checks

```sh
cargo test --workspace --release --locked
python3 -B tools/test_observers.py
```

Default tests require no ROM or downloaded oracle. The C observer test compiles
the real observation hook against an independent register stub. It does not
execute or modify Genesis Plus GX. CPU regression fixtures retain their upstream
file hashes and case identities.

The optional private GPGX trace build requires your own checkout/build of the
oracle. `tools/build_trace_oracle.py` creates a new instrumented directory and
records source/hook/core digests; existing output directories are never replaced.
Never edit the reference emulator's execution to agree with `mdcore`.

## API conventions

- Add operations to `crates/mdx/src/operations.rs`; CLI and MCP use that registry.
- Generate schemas from typed inputs and serialization-mode typed outputs.
- Declare bounds, coverage and omissions; no silent truncation.
- Store large results as immutable artifacts, with bounded structured retrieval.
- Keep execution stateless; cancellation and exception paths must remain bounded.
- Capture registers at the actual consuming instruction; watch folded RAM across
  aliases and all bus widths.
- Use mdcore's disassembler for what its decoder believes a cartridge means.
  Check instruction semantics independently with oracle source or fixtures.
- Reachability or an equal finite prefix is not whole-program equivalence.
