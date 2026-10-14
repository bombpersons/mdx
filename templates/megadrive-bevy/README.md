# Mega Drive-sized native Bevy game

Standalone Rust / **Bevy 0.20.0** starter. No emulator, ROM or MDX dependency.
Requires Rust1.97.1 or newer and a desktop graphics driver. Linux gamepad support
uses libudev; the renderer uses Vulkan/OpenGL as available through Bevy/WGPU.

```sh
cargo run
cargo run -- --software               # force a software adapter
cargo run -- --headless               # no window/display; renders the game texture only
cargo run -- --assets /path/to/assets # explicit asset directory (or GAME_ASSET_DIR)
cargo build --release                 # developer MCP is excluded
cargo run --features dev-mcp -- --mcp  # developer MCP over stdin/stdout
```

## Game integration

Renderer selection prefers hardware and automatically permits software adapters
when hardware is unavailable. `--software` forces the fallback-adapter path;
Vulkan and GLES backends are included for remote desktop setups. Linux needs a
system software driver such as Mesa lavapipe/llvmpipe (`mesa-vulkan-drivers` on
Debian/Ubuntu). WGPU cannot render without either a hardware or software driver.
`game_status.renderer` reports the actual selected adapter/backend/device type.


- Replace `src/demo.rs` with native game logic. Register simulation systems in
  `FixedUpdate`, in `GameSystems::Game`. This gives deterministic60Hz simulation
  ticks, including developer stepping. `GameClock.frame` counts completed ticks.
- Spawn normal Bevy sprites on render layer0. They render into a **320×224**
  texture. Change `Screen::default()` to256×224 or320×240 when appropriate.
- A separate layer1 camera displays that texture, centered and nearest-sampled.
  Resizing uses the smaller width/height scale and black letterboxing; neither
  stretching nor cropping changes the game coordinate system. Render scale can
  be fractional to fit small or non-integer-sized windows. The aspect ratio here
  is the logical framebuffer's square-pixel ratio,320:224.
- Read `ControllerState::{held,pressed,released}(Button::...)`, not raw device
  keys. Edges are relative to simulation ticks, not redraws.
- `ControllerBindings` owns editable keyboard/gamepad bindings and stick deadzone;
  players can change them in game (see Remapping controls).
  The first connected gamepad (stable entity order) is used; unplugging it falls
  back to another pad or the keyboard. Keyboard and gamepad buttons are merged.

| Mega Drive | Keyboard | Conventional gamepad |
|---|---|---|
| D-pad | Arrow keys | D-pad or left stick |
| A | Z | West / left face button |
| B | X | South / bottom face button |
| C | C | East / right face button |
| Start | Enter | Start |

The example moves a blue rectangle two native pixels per tick, displays held
buttons along the bottom, and toggles its own movement pause with Start. It is
a wiring example, not a game architecture to preserve.

## Assets and readiness

Asset paths resolve against one absolute root, chosen in order: `--assets <dir>`,
`$GAME_ASSET_DIR`, `assets/` beside the executable (ship the binary with an
`assets/` directory next to it), `assets/` in the working directory, then this
project's `assets/`. This keeps `cargo run` and a directly launched binary
(for example as an MCP server) loading the same files.

Register every handle the visible scene depends on with
`RequiredAssets::require(&handle)`. Developer screenshots wait until those
assets have loaded and return an error naming any that failed, rather than
capturing a scene that is only clear color. `game_status` reports the asset
root and pending/failed required assets.

Insert game-specific values into the `GameStatus` resource (menu page,
selection, positions, flags); `game_status` returns them under `game`, which
makes invisible state testable without reading memory.

`--headless` runs without a window or display server: only the game texture
is rendered (`game_screenshot` target `screen`; `window` returns an error).
It works with software renderers such as llvmpipe in restricted environments.

## Remapping controls

`rebind.rs` is the logic with no UI: call `Rebind::start(Button::A)` and the
next key or gamepad button pressed is bound to it. A key already used by another
button is swapped rather than duplicated, Escape cancels, and `Rebind::reset`
restores the defaults. Bindings save to `bindings.json` in the platform config
directory (`~/.config/<package>/` on Linux, `%APPDATA%` on Windows,
`~/Library/Application Support` on macOS) and load at startup. While
`Rebind::capturing()` is `Some`, ignore your own menu input: that press is the
new binding.

`controls_menu.rs` is a minimal example built on it, drawn inside the 320×224
framebuffer in a Mega Drive style: palette-snapped colours, a bordered window
with a hard shadow, and the 5×7 bitmap font from `pixel_font.rs`. F1 or a
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
| `game_step` | `{"frames":30,"buttons":["right","b"]}` | Holds buttons for exactly30 ticks, releases injected input, then freezes |
| `game_set_input` | `{"buttons":["a"]}` | Replaces held virtual input; `[]` releases it |
| `game_pause` | `{"paused":false}` | Resumes continuous simulation / manual play |
| `game_screenshot` | `{"target":"screen"}` | Inline PNG from the actual GPU-rendered320×224 game texture |
| `game_screenshot` | `{"target":"window"}` | Actual window-rendered PNG, including scaling and letterboxing |

Buttons are `up/down/left/right/a/b/c/start`; frame requests are1..3600. Real
device input is merged with virtual input. Keep physical devices released during
automated driving. Requests are serialized, bounded and cancellable. Cancelled
steps release virtual input on the next fixed-tick boundary. Closing MCP stdin
closes the developer-owned application. Screenshot metadata records the request
frame; while continuous simulation is running, GPU readback is asynchronous.
Pause first when the image must represent a stable simulation state.
The first screenshot waits for actual render targets and compiled sprite
pipelines to complete a camera pass. Developer windows continue updating while
unfocused, including remote-desktop sessions. For elapsed simulation time use
`GameClock.frame / 60.0`; Bevy's wall/virtual elapsed clocks are not pause-aware
under developer stepping.

The last sampled `ControllerState` describes the last simulated tick. After a
step, its held indicators can remain visible while frozen even though injected
input has been released; the next tick updates the normal release edge.

All server code and direct MCP/Tokio/serialization dependencies are feature-gated.
An ordinary `cargo build --release` has no MCP endpoint. `--mcp` without the
feature exits with an explanatory error rather than silently enabling tooling.

## Checks

```sh
cargo test
cargo test --features dev-mcp
cargo check --release
```

Keep game-specific assets, state, persistence and gameplay in separate modules.
This shell supplies the desktop viewport, controller contract and optional
developer bridge; it does not impose an emulator or a generic game framework.
