# mdx

**An agent-first Mega Drive reverse-engineering toolkit.** JSON-first analysis,
content-addressed artifacts, explicit coverage/limits, and a local MCP server.

The workspace contains `mdx` (CLI/API/MCP) and `mdcore` (analysis engine).
It is extracted from the King's Bounty reconstruction project; no game ROMs,
reference captures, downloaded oracles, or native game implementation are needed
to build or run the default tests.

## Build

Requires Rust **1.88+**. The observation-hook test additionally needs a C compiler
named `cc`. Python observer tests use Python 3's standard library.

```sh
cargo build -p mdx --release --locked
target/release/mdx schema
target/release/mdx schema watch
```

## CLI

```sh
target/release/mdx header --rom /path/to/game.md
target/release/mdx disasm --rom /path/to/game.md --from 0x200 --count 16
target/release/mdx boot --rom /path/to/game.md --frames 60 --instructions 3000000 --trace 0
```

Every analysis returns one JSON envelope, including errors, coverage and limits.
`schema` provides complete input/output JSON Schemas without loading a cartridge.
The catalog is paginated: follow `next_offset` or select an operation by name.
`mdx call OPERATION --args '{...}'` invokes the same typed API used by MCP.

There are **17 operations**: header, disassembly, bounded replay, RAM inspection,
memory read/search/diff, CPU tracing, RAM-write watching, rendering, graphics
export, artifact import/read/view, CPU conformance, batched surveys and schemas.

## MCP

Launch the local server over stdio:

```sh
target/release/mdx --root /absolute/path/to/your/analysis-project mcp
```

`--root` establishes where relative ROM/replay/output paths are resolved. It may
be any analysis directory; it need not be this source checkout. JSON-RPC is the
only stdout traffic. Calls support progress, cancellation and bounded worker
concurrency. CLI and MCP share their schemas and implementation.

Example **OpenCode** configuration; replace both absolute paths:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "mdx": {
      "type": "local",
      "command": ["/absolute/path/to/mdx/target/release/mdx", "--root", "/absolute/path/to/analysis-project", "mcp"],
      "enabled": true
    }
  }
}
```

Restart/reconnect the client after adding the server. OpenCode exposes tool names
such as `mdx_boot`, `mdx_watch` and `mdx_artifact_view`.

## Verify

```sh
cargo test --workspace --release --locked
python3 -B tools/test_observers.py
```

Default tests use authored synthetic cartridges/snapshots and pinned CPU
regression records. Real MCP stdio tests validate tool schemas, CLI parity,
progress, cancellation and disconnect cleanup. Optional cartridge tests are
ignored explicitly; their assets are not distributed here. Game-specific pixel
gates and captures remain in the originating reconstruction project.

## Evidence and limitations

Captured buffers and large responses are immutable SHA-256 artifacts. Reuse them
by ID and query bounded JSON/JSONL pages rather than rerunning emulation. Every
bounded response names what was observed, retained and omitted.

Snapshots are captured buffers, **not resumable CPU checkpoints**. The engine is
an analysis model with finite independent validation, not a claim of whole-game,
general hardware, cycle or audio equivalence. Read each response's limitations.
Game-specific probes belong outside the reusable core.

See [the API handbook](docs/mdx.md), [development notes](docs/development.md) and
[third-party fixture provenance](THIRD_PARTY.md).

Licensed under MIT OR Apache-2.0; see `LICENSE-MIT` and `LICENSE-APACHE`.
