# md-bevy

The mdx template's crate: everything a Mega Drive port needs that is not this
game. `mdx template update` replaces it; build on it from `engine/` and
`game/` (see `AGENTS.md` here before editing it). No emulator, ROM or mdx
dependency.

| Module | What it gives you |
|---|---|
| `launcher` | `launcher::run(Game { .. })`: window, renderer, headless mode, command line |
| `shell` | The tick loop: `GameSystems`, `GameClock`, `TickRate`, `Region`, `GameStatus` |
| `screen` | The fixed-resolution screen texture, `ScreenPosition`, `Depth` |
| `look` | Mega Drive colours and palettes, indexed art, tile planes, indexed sprites |
| `controller`, `rebind`, `controls_menu`, `pixel_font` | Mega Drive buttons, device bindings, rebinding, an example controls menu |
| `assets` | `AssetRoot` (one asset folder for the game and the tests), `RequiredAssets` |
| `testing` | Headless tick tests and trajectory comparison against mdx recordings |
| `dev_mcp` | Developer MCP (`dev-mcp` feature): status, input, stepping, screenshots |

## Launcher

A game's `main.rs` describes itself and calls the launcher:

```rust
launcher::run(Game {
    name: env!("CARGO_PKG_NAME"),
    title: game::TITLE,
    region: game::REGION,
    frames_per_tick: game::FRAMES_PER_TICK,
    screen: game::SCREEN,
    assets: concat!(env!("CARGO_MANIFEST_DIR"), "/assets"),
    add: |app| { app.add_plugins(game::GamePlugin); },
});
```

| Flag | Effect |
|---|---|
| `--headless` | No window or display server; renders only the game texture |
| `--software` | Force a software adapter (e.g. Mesa lavapipe/llvmpipe) |
| `--assets DIR` | Explicit asset directory (or `$GAME_ASSET_DIR`) |
| `--region ntsc\|pal` | Override the game's region (tick rate) |
| `--mcp` | Developer MCP over stdin/stdout (`dev-mcp` builds only) |

Renderer selection prefers hardware and automatically permits software adapters
when hardware is unavailable. Vulkan and GLES backends are included for remote
desktop setups. Linux needs a system software driver such as Mesa
lavapipe/llvmpipe (`mesa-vulkan-drivers` on Debian/Ubuntu) when there is no
GPU. `game_status.renderer` reports the adapter actually selected.

## Ticks

Gameplay runs in `FixedUpdate`, in `GameSystems::Game`, at the original's
rate: `TickRate::every_nth_frame(region, frames_per_tick)`, with NTSC at
≈59.92 Hz and PAL at ≈49.70 Hz (the console's master clock, not 60/50).
`GameClock.frame` counts completed ticks. Read `ControllerState::{held,
pressed, released}(Button::...)`; edges are relative to ticks, not redraws.

## The screen

Everything on render layer 0 draws into a texture of the original's size
(`Screen`, 320×224 by default), which a layer 1 camera scales into the
window with nearest sampling and letterboxing. World coordinates are Bevy's
(centre origin, +y up); `ScreenPosition(IVec2)` places an entity's top-left
corner in screen pixels from the top-left, +y down, as the original's code
thinks (use it with `Anchor::TOP_LEFT`). `Depth` holds the `Transform` z of
each priority band: backdrop, plane B low, plane A low, sprites low, plane B
high, plane A high, sprites high, overlay.

## The look

`look` reproduces how a Mega Drive game looks, as ordinary Bevy rendering.
It models the art, not the console: no VRAM, nametables, registers, DMA or
timing. Express effects as data the game sets each tick.

- `MdColor` is a 9-bit colour (levels 0..=7), converted to sRGB with mdx's
  exact level mapping (also Genesis Plus GX's), so a native screenshot of the
  same scene compares pixel for pixel with mdx's. `Shade` gives the
  shadow/highlight levels.
- `Palettes` (a resource) holds four 16-colour lines and the backdrop colour.
  Changing it re-bakes what uses it: fades and colour cycling are edits.
- `IndexedImage` is palette-index art, loaded from indexed PNGs
  (`IndexedImage::load(&root, "x.png")`); only indices are used.
- `TilePlane::new(tileset, map)` + `PlaneScroll` draw a `TileMap` of
  `TileRef { tile, palette, flip_x, flip_y, priority }` cells from an
  indexed tileset (8×8 tiles left to right, top to bottom). Planes can
  `wrap`; low- and high-priority tiles go to the depths given by
  `with_depth`. Only on-screen 256×256 chunks are baked and drawn.
- `IndexedSprite { image, palette }` draws a `Sprite` from indexed art.

Not provided, and best built in `engine/` when the original uses them:
per-line or per-column scroll, the window plane, shadow/highlight drawing,
mid-frame palette changes. Sprite limits and flicker are hardware artefacts;
leave them out unless the game depends on them.

## Assets and readiness

`AssetRoot` is the one asset folder, used by Bevy's asset server and by the
synchronous loaders (`AssetRoot::load_ron`, `IndexedImage::load`) alike, so
headless tests read the same files as the game. The launcher picks, in order:
`--assets <dir>`, `$GAME_ASSET_DIR`, `assets/` beside the executable (ship
the binary with an `assets/` directory next to it), `assets/` in the working
directory, then the crate's own `assets/`. Without the launcher (tests) it
is `assets/` in the crate being tested.

Register every handle loaded through the asset server that the visible scene
depends on with `RequiredAssets::require(&handle)`. Developer screenshots wait
until those assets have loaded and return an error naming any that failed,
rather than capturing a scene that is only clear color.

Insert game-specific values into the `GameStatus` resource (menu page,
selection, positions, flags); `game_status` returns them under `game`, which
makes invisible state testable without reading memory.

## Testing

`testing::headless_app(tick)` builds an app with Bevy's minimal plugins and
the tick loop; `app.run_tick(&[Button::Right])` runs exactly one tick with
those buttons held. `testing::trajectory` replays an mdx controller replay
(`Replay`), reads an mdx samples CSV (`Trajectory::load_mdx_samples`),
records the rebuild (`Trajectory::record`) and compares them column by column
(`compare`). See the module docs for the timing rules.

## Remapping controls

`rebind` is the logic with no UI: call `Rebind::start(Button::A)` and the
next key or gamepad button pressed is bound to it. A key already used by another
button is swapped rather than duplicated, Escape cancels, and `Rebind::reset`
restores the defaults. Bindings save to `bindings.json` in the platform config
directory, in a folder named after the game's package (`~/.config/<package>/`
on Linux, `%APPDATA%` on Windows, `~/Library/Application Support` on macOS)
and load at startup. While
`Rebind::capturing()` is `Some`, ignore your own menu input: that press is the
new binding.

`controls_menu` is a minimal example built on it, drawn inside the 320×224
framebuffer in a Mega Drive style: palette-snapped colours, a bordered window
with a hard shadow, and the 5×7 bitmap font from `pixel_font`. F1 or a
gamepad's Select opens it, and simulation pauses while it is open. Its
navigation keys are fixed (arrows/D-pad, Enter/South, Esc/East), so a bad
binding cannot lock the player out. Move it with `POSITION`, restyle it with the
constants at the top of the file, open it from your own pause menu by setting
`ControlsMenu::open`, or delete the module and drive `Rebind` from your own UI.

## Agent driving

Build with `dev-mcp` and launch the binary with `--mcp` as a **stdio MCP server**.
Bevy logs go to stderr; stdout is reserved for JSON-RPC. The window still opens.
Developer mode starts with simulation paused, while rendering/window events
continue. This makes `game_step` followed by `game_screenshot` reproducible.

| Tool | Arguments | Behavior |
|---|---|---|
| `game_status` | `{}` | Frame, pause state, dimensions, sampled controller |
| `game_step` | `{"frames":30,"buttons":["right","b"]}` | Holds buttons for exactly 30 ticks, releases injected input, then freezes |
| `game_set_input` | `{"buttons":["a"]}` | Replaces held virtual input; `[]` releases it |
| `game_pause` | `{"paused":false}` | Resumes continuous simulation / manual play |
| `game_screenshot` | `{"target":"screen"}` | Inline PNG from the actual GPU-rendered game texture (native resolution) |
| `game_screenshot` | `{"target":"window"}` | Actual window-rendered PNG, including scaling and letterboxing |

Buttons are `up/down/left/right/a/b/c/start`; frame requests are 1..3600. Real
device input is merged with virtual input. Keep physical devices released during
automated driving. Requests are serialized, bounded and cancellable. Cancelled
steps release virtual input on the next fixed-tick boundary. Closing MCP stdin
closes the developer-owned application. Screenshot metadata records the request
frame; while continuous simulation is running, GPU readback is asynchronous.
Pause first when the image must represent a stable simulation state.
The first screenshot waits for actual render targets and compiled sprite
pipelines to complete a camera pass. Developer windows continue updating while
unfocused, including remote-desktop sessions. For elapsed simulation time use
`GameClock.frame` ticks; Bevy's wall/virtual elapsed clocks are not pause-aware
under developer stepping.

The last sampled `ControllerState` describes the last simulated tick. After a
step, its held indicators can remain visible while frozen even though injected
input has been released; the next tick updates the normal release edge.

All server code and direct MCP/Tokio/serialization dependencies are feature-gated.
An ordinary `cargo build --release` has no MCP endpoint. `--mcp` without the
feature exits with an explanatory error rather than silently enabling tooling.

## Checks

```sh
cargo test --workspace --all-features
cargo check --release
```
