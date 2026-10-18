//! The executable's side of a game: window, renderer, headless mode and the
//! command line every game built on the template shares. A game's `main.rs`
//! describes itself with [`Game`] and calls [`run`]:
//!
//! ```no_run
//! use bevy::math::UVec2;
//! use md_bevy::{launcher::{self, Game}, shell::Region};
//!
//! fn main() {
//!     launcher::run(Game {
//!         name: env!("CARGO_PKG_NAME"),
//!         title: "My game",
//!         region: Region::Ntsc,
//!         frames_per_tick: 1,
//!         screen: UVec2::new(320, 224),
//!         assets: concat!(env!("CARGO_MANIFEST_DIR"), "/assets"),
//!         add: |app| { /* app.add_plugins(game::GamePlugin); */ },
//!     });
//! }
//! ```
//!
//! Flags: `--mcp` (developer MCP, needs the `dev-mcp` feature), `--headless`
//! (no window or display server; renders only the game texture),
//! `--software` (force a software adapter), `--assets DIR`,
//! `--region ntsc|pal`.
use crate::assets::{self, RequiredAssets};
use crate::controls_menu::ControlsMenuPlugin;
use crate::rebind::{BindingsFile, RebindPlugin};
use crate::screen::{self, Headless, Screen};
use crate::shell::{Region, ShellPlugin, TickRate};
use bevy::prelude::*;
use bevy::render::{RenderPlugin, settings::WgpuSettings};
use std::path::Path;

/// What the launcher needs to know about a game.
pub struct Game<F: FnOnce(&mut App)> {
    /// Package name: names the settings folder (bindings.json).
    pub name: &'static str,
    /// Window title.
    pub title: &'static str,
    /// The original's video standard; `--region` overrides it.
    pub region: Region,
    /// Video frames per logic tick (2 when the original's main loop waits
    /// for two vertical blanks).
    pub frames_per_tick: u32,
    /// The original's display size in pixels: 320x224 (H40) for most games,
    /// 256x224 (H32) for some, 240 lines in PAL V30 mode. Read the real one
    /// from the original's VDP mode registers. Sizes [`Screen`] and the
    /// window (at 3x).
    pub screen: UVec2,
    /// The crate's own asset folder, used when nothing more specific exists
    /// (see [`assets::resolve`]). Pass
    /// `concat!(env!("CARGO_MANIFEST_DIR"), "/assets")`.
    pub assets: &'static str,
    /// Adds the game's plugins.
    pub add: F,
}

/// The parsed command line.
pub struct Args(Vec<String>);

impl Args {
    pub fn from_env() -> Self {
        Self(std::env::args().collect())
    }
    pub fn flag(&self, name: &str) -> bool {
        self.0.iter().any(|arg| arg == name)
    }
    pub fn value(&self, name: &str) -> Option<&str> {
        let at = self.0.iter().position(|arg| arg == name)?;
        self.0.get(at + 1).map(String::as_str)
    }
}

/// Builds and runs the game.
pub fn run<F: FnOnce(&mut App)>(game: Game<F>) {
    app(game).run();
}

/// Builds the game's app without running it, for games that need to adjust
/// it first.
pub fn app<F: FnOnce(&mut App)>(game: Game<F>) -> App {
    let args = Args::from_env();
    let mcp = args.flag("--mcp");
    let software = args.flag("--software");
    let headless = args.flag("--headless");
    #[cfg(not(feature = "dev-mcp"))]
    if mcp {
        eprintln!("--mcp requires a developer build: cargo run --features dev-mcp -- --mcp");
        std::process::exit(2);
    }
    let region = match args.value("--region") {
        None => game.region,
        Some(text) => Region::parse(text).unwrap_or_else(|| {
            eprintln!("--region takes ntsc or pal, not {text:?}");
            std::process::exit(2);
        }),
    };
    let tick = TickRate::every_nth_frame(region, game.frames_per_tick);
    let root = assets::resolve(Path::new(game.assets));
    let mut app = App::new();
    let plugins = DefaultPlugins
        .set(AssetPlugin {
            file_path: root.0.to_string_lossy().into_owned(),
            ..default()
        })
        .set(WindowPlugin {
            primary_window: (!headless).then(|| Window {
                title: game.title.into(),
                resolution: (game.screen.x * 3, game.screen.y * 3).into(),
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
            // One video frame per update, so fixed ticks still run at the game's rate.
            .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / region.frame_rate()),
            ));
    } else {
        app.add_plugins(plugins);
    }
    app.insert_resource(root)
        .insert_resource(Headless(headless))
        .insert_resource(Screen::new(game.screen.x, game.screen.y))
        .insert_resource(BindingsFile::for_package(game.name))
        .init_resource::<RequiredAssets>()
        .add_plugins((ShellPlugin { tick }, crate::look::LookPlugin))
        .add_plugins((RebindPlugin, ControlsMenuPlugin))
        .add_systems(Startup, screen::setup)
        .add_systems(Update, screen::fit_window);
    #[cfg(feature = "dev-mcp")]
    if mcp {
        app.add_plugins(crate::dev_mcp::GameMcpPlugin);
    }
    (game.add)(&mut app);
    app
}
