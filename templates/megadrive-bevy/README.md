# Mega Drive port workspace

Created by `mdx template create`. A Rust / **Bevy 0.20.0** workspace with no
emulator, ROM or mdx dependency. Requires Rust 1.97.1 or newer.

| Crate | What it is |
|---|---|
| `md-bevy/` | The template's crate: Mega Drive look (palettes, indexed art, tile planes, priorities), tick loop, input and rebinding, launcher, headless testing, developer MCP. `mdx template update` replaces it. |
| `engine/` | This game's engine family: object type and action registries, update-order sets, level and tuning formats, and a sandbox game built only from the engine. Nothing specific to this game. |
| `game/` | This title: per-type controllers, rules, HUD, menus, baked assets in `game/assets/`. Starts as a demo to replace. |
| `tools/import/` | Offline importers: ROM data -> engine formats, with provenance. |
| `docs/` | `engine-model.md` (what the original treats as data and as code), `evidence.md` (what is verified against what), `template.md` (local edits to `md-bevy/`). |

```sh
cargo run                                           # the game
cargo run -p engine --example sandbox               # the engine's sandbox game
cargo test --workspace --all-features               # routine check
cargo run -p mdx-game --features dev-mcp -- --mcp   # developer MCP over stdio
cargo run -p import -- tiles --rom game.md --offset 0x1000 --count 32 --out game/assets/x.png
cargo build --release                               # shipping build, no developer MCP
```

Launcher flags (game and sandbox): `--headless` (no window or display
server), `--software` (software adapter, e.g. llvmpipe), `--assets DIR`,
`--region ntsc|pal`, `--mcp` (needs the `dev-mcp` feature).

`.mdx-template.toml` records the template version and file hashes for
`mdx template update`.
