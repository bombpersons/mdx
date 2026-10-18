# Working in the engine

`engine/` is the reusable machinery this game runs on. Someone should be able
to make a *different* game with it, without the original's content and without
this game's rules. It builds on `md-bevy` (re-exported as `engine::look`,
`engine::shell`, `engine::testing`, ...); build on it rather than editing it
(`md-bevy/AGENTS.md` says when an edit is fine).

## Cut it where the original cuts

Most Mega Drive games already separate:

- **Data the engine interprets**: level layouts (tiles, blocks, chunks),
  collision arrays, object placement lists, animation and mapping tables,
  palettes, event tables, sound sequences.
- **Code written per object type**: the player's routines, each enemy's and
  boss's routine, special objects, usually reached through a pointer table
  indexed by object type.

If the engine's formats express what the original's tables express, and
per-object code stays as code, generalising costs no accuracy. Accuracy breaks
when you invent abstractions the original doesn't have (a generic "enemy
behaviour" schema, a scripting language for events, a physics component that
replaces hand-written movement): they lose update order, timing within a
tick, fixed-point rounding and special cases. `docs/engine-model.md` records,
from evidence, what the original treats as data and what as code; this crate
follows it.

## Placing code

Decide where something goes before writing it:

1. **Would a different game on this engine need it?** Then it goes here,
   named generically (`Character`, `ObjectType`, `TriggerAction`), not after
   this game's characters or items.
2. **Did the original read it from a table?** Then it's data in an asset that
   engine code interprets. Don't hardcode it.
3. **Did the original hardcode it per object type?** Then it's a controller in
   `game/`, registered with the type registry and ported faithfully, quirks
   included. Don't generalise it into a schema.
4. **Is it a design number inside a controller** (speed, damage, timer)? Put it
   in a per-type tuning file the controller reads (`config::TuningPlugin`).
   Leave structural details (update order, rounding) in code.

Never here: this game's title, character or item names, asset paths, magic
ids from the original's tables, ROM addresses, or `cfg`s and branches that
test which game is running.

## Not an emulator

The engine reproduces the game's behaviour and look, not the console. No
68000 or Z80 interpreters, VRAM or nametable buffers, VDP register writes,
DMA queues or memory maps. Where the original uploads tiles or rewrites a
nametable, the engine changes a `TilePlane`'s map or an `IndexedSprite`'s
art; where it writes colour RAM, the engine edits `Palettes`. Object slots
become entities, routine pointers become registered types and systems.

## Extension points

The skeleton is a starting point; reshape it to match `docs/engine-model.md`.

- `schedule.rs`, **`EngineSet`**: named sets in the original's update order,
  inside `GameSystems::Game`. Game systems join a set rather than ordering
  themselves ad hoc. Rename and reorder the placeholders from evidence.
- `objects.rs`, **object type registry**: type id → spawn hook, plus metadata
  (name, tuning resource). Adding a type means registering it, never editing a
  central `match`. Unknown ids are counted in `UnknownObjectTypes`.
  **`Position`** is a placeholder in whole pixels: replace it with the
  original's fixed-point representation once it is known.
- `triggers.rs`, **action registry**: action id → one-shot handler system.
  Unknown actions are logged and counted in `UnknownActions`, never ignored or
  mapped to something else. `TriggerArea` is a placeholder detector: replace
  it with the original's trigger model and keep the registry.
- `level.rs`, **`LevelData`**: what a level holds, in the engine's format.
  Grow it with what the original's levels actually contain (tile maps,
  collision, camera bounds, events).
- `config.rs`: RON loading for tuning and tables.

## Formats

- Normal units: pixels for positions and sizes (keep the original's integer
  or fixed-point values; don't convert to floats where rounding matters),
  ticks or seconds for time.
- Readable files: RON for tables, maps and palettes (`md_bevy::look`'s
  `TileMap`, `Palettes`), indexed PNGs for art, WAV for sound, so people can
  author them by hand or with ordinary tools, and an editor can open and save
  them.
- The editor that opens them is a modern desktop application: full window
  resolution, mouse and keyboard, ordinary UI. It does not use md-bevy's
  low-resolution screen, bitmap font or pad menus, and should not look like a
  Mega Drive game (see the mdx `guide` topic `port`).
- Exact enough that trajectory tests still match.
- Keep registry metadata (names, tuning types) complete: it is how tools list
  and inspect the game's types without depending on `game/`.

## The sandbox keeps the engine honest

`examples/sandbox.rs` is a second, tiny game built only from this crate and
the hand-written files in `assets/sandbox/`: one room, one controllable
character, one object type with a trivial controller, one trigger. It must not
depend on `game/` or on anything read from the ROM.

Build and run it in every verification pass:

```sh
cargo run -p engine --example sandbox
cargo run -p engine --features dev-mcp --example sandbox -- --mcp --headless --software
```

If a feature of the port forces you to touch the sandbox in a way a new game's
author wouldn't expect, or the sandbox can't be made without reaching into
`game/`, the boundary is in the wrong place. Fix the boundary, not the sandbox.

## Checks

`cargo test -p engine` (registry and trigger tests), then
`cargo test --workspace --all-features`. Every restructuring must keep every
game trajectory test passing exactly.
