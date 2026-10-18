# Working in the game

`game/` is this title on top of `engine/`: per-type controllers, action
handlers, HUD, menus, save format, level order, and the baked content in
`assets/`. Reusable machinery belongs in `engine/` (read `engine/AGENTS.md`
for the placement rules). The template's crate `md-bevy/` is built on, and
edited only as `md-bevy/AGENTS.md` allows.

The goal is a Rust/Bevy version of a game that reads like a modern, first-time
implementation of it: something a person could pick up later and change with
ordinary Bevy techniques. It must not be an emulator in disguise. The Mega
Drive *look* comes from `engine::look`; everything else is normal Bevy code.

## Ground rules

- Write idiomatic, modern Bevy 0.20 code: entities and components, one plugin
  per feature, queries instead of global state. No CPU interpreters, VRAM or
  nametable buffers, VDP register writes, DMA queues or memory maps.
- Draw with the look (`md-bevy/README.md`): `TilePlane` + `PlaneScroll` for
  backgrounds and foregrounds, `IndexedSprite` for objects, `Palettes` for
  colour (fades and cycling are edits of it), `Depth` for priority order and
  `ScreenPosition` for top-left pixel placement. Plain `Sprite`s are fine for
  anything the original didn't draw with palettes.
- Keep gameplay in `FixedUpdate` / `GameSystems::Game` (inside an
  `EngineSet`), reading `ControllerState`. That is the boundary the developer
  MCP and the tests step.
- Keep game assets in `game/assets/`, produced by `tools/import` from the ROM
  or written by hand. Do not commit the ROM or anything copied from it
  verbatim outside the importer's outputs.
- Put values worth checking from outside into `GameStatus` (the developer
  MCP's `game_status` returns them under `game`), as the demo does with the
  player's position.

## Project layout

The game is a library (crate `game`, `src/lib.rs`); `src/main.rs` only calls
the engine's launcher. So tests in `tests/` can use every game module.

```
src/lib.rs            TITLE, REGION, FRAMES_PER_TICK, SCREEN, GamePlugin
src/main.rs           engine::launcher::run(...): window, renderer, --flags
src/<feature>/mod.rs  one plugin per feature: player, stage, enemies, hud...
src/<feature>/...     rules (components + systems, no rendering) apart from presentation
assets/               baked content in the engine's formats (levels, tuning, palettes, art)
tests/<feature>.rs    headless tick tests against recorded trajectories
tests/data/           mdx replays and samples those tests read
```

- Start a new game by deleting `src/demo/` and `assets/demo/`, removing the
  demo from `GamePlugin` and `lib.rs`, and adding your own feature modules.
  Replace or remove `tests/player.rs` and `tests/data/` in the same change.
- Per-type controllers register with the engine: `app.register_object_type(id,
  ObjectTypeInfo::new("Name", spawn).with_tuning::<Tuning>())`, systems in an
  `EngineSet`, design numbers in `assets/<type>.ron` through `TuningPlugin`.
  Event actions: `app.register_action(id, "name", handler)`.
  Placements come from level data (`LevelData` + `SpawnLevel`), not code.
- Split each feature into a *rules* plugin (state, movement, decisions; no
  sprites or asset loading) and a *presentation* part (sprites, planes,
  animation). Rules then run headless in tests. The demo dresses rules-only
  entities in `Update` (`dress_player`).
- The controls menu (`F1`) is the launcher's; open it from your own pause
  menu by setting `ControlsMenu::open`, or build your own UI on `Rebind`.

## Tick rate and region

- `lib.rs` sets `REGION` (NTSC ≈59.92 Hz, PAL ≈49.70 Hz) and
  `FRAMES_PER_TICK`. Set both from evidence: the original's main loop and how
  many vertical blanks it waits per update. Per-tick constants only mean the
  same thing at the same rate.
- `SCREEN` is the original's display size: 320x224 (H40) for most games,
  256x224 (H32) for some. Read it from the VDP mode registers (mdx `boot`
  reports them).
- Do not run logic every Nth tick by counting `GameClock.frame`: button edges
  (`pressed`) would fall on skipped ticks and be lost. Set the tick rate.
- Lag frames (the original missing a vertical blank under load) are not
  reproduced by default; if a behaviour depends on them, record it in
  `docs/engine-model.md` and model it explicitly.

## Testing gameplay

`engine::testing` runs rules headless for exact ticks with scripted buttons
(`tests/player.rs`), and compares them with the original
(`engine::testing::trajectory`):

1. Save a state at the point of interest (mdx `boot` with `save_state`),
   write a controller replay (`tests/data/jump.input.json`, mdx's replay
   format), then record from that state with one sample per value:
   mdx `boot` with `from_state`, `input`, `frames` and
   `samples: [{"address":"0xFFD00C","type":"s16","label":"y"}, ...]`.
   Copy the run's samples CSV (`data.samples.artifact.path`) to
   `tests/data/jump.samples.csv`.
2. In a test, spawn the recorded starting state (row 0), replay the same
   input and record the same names in the original's units:

```rust
let align = Alignment::new(game::FRAMES_PER_TICK, 0);
let original = Trajectory::load_mdx_samples("tests/data/jump.samples.csv", align)?;
let inputs = Replay::load("tests/data/jump.input.json")?.tick_inputs(align, original.ticks() - 1)?;
let rebuild = Trajectory::record(&mut app, &inputs, &["y"], |world| vec![/* y in the original's units */]);
compare(&original, &rebuild, &[("y", 0.0)])?.assert_passed();
```

Sample row N is memory at the start of vertical blank of run frame N, before
that frame's input takes effect. A failure prints each column's first
divergence. With more than one frame per tick, input may only change on tick
boundaries (`tick_inputs` refuses anything else). Use tolerance 0: Mega
Drive rules are integer arithmetic and should match exactly. Keep the
recording commands in `tools/`, and include at least one scenario that was
recorded after the rules were written and never used to fit them.

## From evidence to idiomatic code

mdx hands you evidence in the original's terms: RAM words in fixed point,
per-frame deltas, shifts and masks from 68000 code. The job is to turn that
into *rules* a reader recognises, not to transliterate the code.

- **Name the rule, then write it.** Write down what a routine *does* in one
  sentence, then implement that sentence with named constants. Cite the
  address of the variable and of the code that writes or tests it.
- **Keep the integers.** Positions and speeds are usually 16.16 or 16.8 fixed
  point; keep them as integers in a small documented type (or `i32` subpixels)
  so rounding, thresholds and durations match exactly. Convert to screen
  pixels only for presentation.
- **Write constants as the numbers they are**, with the raw value and its
  writer in the doc comment:

```rust
/// Initial jump speed: -0x680 (8.8 fixed) = -6.5 px/tick, written by the
/// jump routine at 0x0001A2F4.
pub const JUMP_SPEED: Subpixels = Subpixels(-0x680);
```

- **State machines**: the original keeps a routine counter per object and
  jumps through a table once per tick. Model the states, not the dispatch:
  an `enum` component with each state's data in its variant, a few systems
  chained in the order the original decides things within a tick, `match`
  inside each. No `routine: u8`, no `fn table[routine]()`.
- **Mappings are art data**: sprite mapping tables (pieces, offsets, flips)
  become an importer output the presentation reads, not code.
- **Cite evidence** in comments: the RAM address and width of the variable,
  the routine that writes or tests it, or the measurement. Say what was
  *not* verified. Someone must be able to re-check each number with mdx.
- If a literal model of the 68000 code is useful as evidence, keep it outside
  the game (in `tools/`) and test the game against recorded data, not the
  model.

## Verify

- `cargo test --workspace --all-features`, with per-tick trajectory
  comparisons (above).
- `cargo run -p engine --example sandbox` still builds and runs.
- Visual changes: build with `dev-mcp`, run with `--mcp` (add `--headless
  --software` without a desktop), `game_step` then `game_screenshot`, and
  look at the image. Compare it with an mdx screenshot of the same moment
  using mdx `image.compare`: the look's colours use mdx's exact levels, so
  matching scenes can match pixel for pixel.
- Build the shipping version without `dev-mcp`.
