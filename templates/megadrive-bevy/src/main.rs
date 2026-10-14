//! Native game shell: fixed-resolution rendering, fixed-step logic and controllers.
mod assets;
mod controller;
mod controls_menu;
mod demo;
mod pixel_font;
#[cfg(feature = "dev-mcp")]
mod dev_mcp;
mod rebind;
mod screen;

use bevy::prelude::*;
use bevy::render::{RenderPlugin, settings::WgpuSettings};
use controller::{ControllerBindings, ControllerState};

#[derive(Resource, Default)]
pub struct GameClock {
    pub frame: u64,
    pub ticking: bool,
}

/// Game state the developer `game_status` tool reports under `game`. Insert
/// whatever makes invisible state testable (menu page, position, flags).
#[derive(Resource, Default)]
#[cfg_attr(not(feature = "dev-mcp"), allow(dead_code))]
pub struct GameStatus(pub serde_json::Map<String, serde_json::Value>);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameSystems {
    Input,
    Game,
    Finish,
}

fn main() {
    let mcp = std::env::args().any(|arg| arg == "--mcp");
    let software = std::env::args().any(|arg| arg == "--software");
    // --headless: no window or display server; renders only the game texture.
    let headless = std::env::args().any(|arg| arg == "--headless");
    #[cfg(not(feature = "dev-mcp"))]
    if mcp {
        eprintln!("--mcp requires a developer build: cargo run --features dev-mcp -- --mcp");
        std::process::exit(2);
    }
    let mut app = App::new();
    let plugins = DefaultPlugins
            .set(AssetPlugin {
                file_path: assets::asset_root(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: (!headless).then(|| Window {
                    title: "MDX game template — arrows, Z/X/C, Enter, F1 controls".into(),
                    resolution: (960, 672).into(),
                    ..default()
                }),
                exit_condition: if headless {
                    bevy::window::ExitCondition::DontExit
                } else {
                    bevy::window::ExitCondition::OnPrimaryClosed
                },
                ..default()
            })
            .set(ImagePlugin::default_nearest())
            // Normal WGPU selection prefers hardware and permits CPU adapters when
            // hardware is absent. GLES adds another backend; --software selects a
            // software adapter explicitly (e.g. Mesa lavapipe/llvmpipe).
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    force_fallback_adapter: software,
                    ..default()
                }
                .into(),
                synchronous_pipeline_compilation: true,
                ..default()
            });
    if headless {
        app.add_plugins(plugins.disable::<bevy::winit::WinitPlugin>())
            .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ));
    } else {
        app.add_plugins(plugins);
    }
    app.insert_resource(screen::Headless(headless))
    .init_resource::<assets::RequiredAssets>()
    .init_resource::<GameStatus>()
    .insert_resource(Time::<Fixed>::from_hz(60.0))
    .init_resource::<GameClock>()
    .init_resource::<ControllerBindings>()
    .init_resource::<ControllerState>()
    .init_resource::<screen::Screen>()
    .add_plugins((rebind::RebindPlugin, controls_menu::ControlsMenuPlugin))
    .configure_sets(
        FixedUpdate,
        (GameSystems::Input, GameSystems::Game, GameSystems::Finish).chain(),
    )
    .configure_sets(
        FixedUpdate,
        GameSystems::Game.run_if(|clock: Res<GameClock>| clock.ticking),
    )
    .add_systems(Startup, (screen::setup, demo::setup).chain())
    .add_systems(Update, screen::fit_window)
    .add_systems(FixedUpdate, begin_tick.in_set(GameSystems::Input))
    .add_systems(FixedUpdate, demo::tick.in_set(GameSystems::Game))
    .add_systems(FixedUpdate, finish_tick.in_set(GameSystems::Finish));
    #[cfg(feature = "dev-mcp")]
    if mcp {
        app.add_plugins(dev_mcp::GameMcpPlugin);
    }
    app.run();
}

fn begin_tick(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    bindings: Res<ControllerBindings>,
    mut state: ResMut<ControllerState>,
    mut clock: ResMut<GameClock>,
    menu: Res<controls_menu::ControlsMenu>,
    #[cfg(feature = "dev-mcp")] mut control: Option<ResMut<dev_mcp::Control>>,
) {
    // The controls menu pauses play; developer steps still advance.
    clock.ticking = !menu.open;
    #[cfg(not(feature = "dev-mcp"))]
    let injected = 0;
    #[cfg(feature = "dev-mcp")]
    let mut injected = 0;
    #[cfg(feature = "dev-mcp")]
    if let Some(ref mut control) = control {
        control.discard_cancelled_step();
        clock.ticking = (!control.paused && !menu.open) || control.has_step();
        injected = control.buttons;
    }
    if clock.ticking {
        let pad = pads
            .iter()
            .min_by_key(|(entity, _)| entity.to_bits())
            .map(|(_, pad)| pad);
        let physical = if menu.open {
            0
        } else {
            controller::sample(&keys, pad, &bindings)
        };
        state.update(physical | injected);
    }
}

fn finish_tick(
    mut clock: ResMut<GameClock>,
    #[cfg(feature = "dev-mcp")] control: Option<ResMut<dev_mcp::Control>>,
) {
    if !clock.ticking {
        return;
    }
    clock.frame += 1;
    #[cfg(feature = "dev-mcp")]
    if let Some(mut control) = control {
        control.finish_tick(clock.frame);
    }
}
