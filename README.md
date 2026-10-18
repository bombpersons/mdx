# mdx

mdx is a toolkit for taking Mega Drive / Genesis games apart.

Under the hood it's a Mega Drive emulator, but not the kind you play games on.
It's built for *looking inside* games: run a cartridge for a few seconds, grab a
screenshot and a dump of every byte of memory, see which instruction wrote to
some RAM address, disassemble the sound driver, record the music as WAV, VGM or
one track per voice, save the whole machine state and pick it up again later.

It was made to help reconstruct an old game (King's Bounty), and it's designed
to be driven by AI agents as much as by people. Every command returns tidy JSON
that says exactly what it saw, what it left out, and how sure it is. That makes
it easy for an agent (or a script, or you) to dig through a game without
guessing.

## How accurate is it?

Very, and on purpose. mdx is constantly checked against
[Genesis Plus GX](https://github.com/ekeeke/Genesis-Plus-GX), a well-respected
emulator, which we use as a reference. When the two disagree, we trace both
instruction by instruction until we find out why, then make mdx match. That
covers the weird stuff too: odd timing quirks, open-bus reads, broken
prototypes crashing in exactly the same way.

Right now, run against a library of about 2,800 cartridges for two seconds each,
roughly 2,720 come out byte-for-byte identical to the reference: same picture,
same video memory, same work RAM. That includes Virtua Racing and its SVP chip.
Most of what's left is Mega-CD BIOS images, which need CD hardware mdx doesn't
emulate.

## What can it do?

- **Boot** a cartridge for N frames, with optional controller input, and get a
  screenshot, memory snapshots and lots of diagnostics.
- **Disassemble** 68000 and Z80 code.
- **Trace** instructions and **watch** memory writes to find the code that
  touched something.
- **Search, diff and read** memory snapshots.
- **Render** graphics: tiles, palettes, planes and sprites.
- **Record audio** as WAV, VGM, or separate stems per FM/PSG voice.
- **Save and resume** the whole machine at any point.
- **Compare images**, for example a remake against the original.
- **Start a native port** (Rust + Bevy): a workspace that keeps a reusable
  engine apart from the game, draws with Mega Drive palettes, tiles and
  priorities, and tests the port against per-frame recordings of the
  original (`mdx guide port`).

You can use all of this from the command line, or plug mdx into an AI coding
agent as an MCP server.

## Install

Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/bombpersons/mdx/main/install.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/bombpersons/mdx/main/install.ps1 | iex"
```

The installer puts `mdx` in `~/.local/bin` (Windows: `%LOCALAPPDATA%\mdx\bin`),
checks the download's SHA-256, and registers mdx as an MCP server with any of
Claude Code, Codex and OpenCode it finds (`MDX_NO_MCP=1` skips that; run
`mdx setup mcp` later to register again). Run it again to update. Releases
are on [GitHub](https://github.com/bombpersons/mdx/releases).

## Build from source

You'll need Rust (1.88 or newer) and a C++ compiler.

```sh
cargo build -p mdx --release --locked

target/release/mdx header --rom /path/to/game.md
target/release/mdx boot --rom /path/to/game.md --frames 60 --instructions 3000000
target/release/mdx audio --rom /path/to/game.md --frames 120 --out music
```

To use it from an agent, run it as an MCP server:

```sh
target/release/mdx --root /path/to/your/project mcp
```

`mdx setup mcp` registers that binary with Claude Code, Codex and OpenCode
(`--dry-run` shows what it would change).

No game ROMs are included, and none are needed to build mdx or run its tests.

## Where to go next

- [README_FOR_AGENTS.md](README_FOR_AGENTS.md): the detailed tour, covering
  architecture, how accuracy is checked, the tooling and the house rules.
- [docs/mdx.md](docs/mdx.md): the full command/API handbook.
- [docs/port-prompt.md](docs/port-prompt.md): a starting prompt for an agent
  porting a game.
- [docs/save-states.md](docs/save-states.md) and
  [docs/audio-development.md](docs/audio-development.md).

## See also

[psx](https://github.com/bombpersons/psx) is the same idea for the original
PlayStation: disc and EXE tools, emulator backends, PsyQ symbols and a
PS1-style game template.

## License

MIT or Apache-2.0, at your option. See `LICENSE-MIT`, `LICENSE-APACHE` and
[THIRD_PARTY.md](THIRD_PARTY.md).
