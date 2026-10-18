//! The game: this title's controllers, rules, HUD and menus on top of the
//! engine. `src/main.rs` runs it; `tests/` drive it headless.
use bevy::prelude::*;
use engine::prelude::*;

pub mod demo;

/// Window title.
pub const TITLE: &str = "MDX game template — arrows, Z/X/C, Enter, F1 controls";
/// The original's video standard (`--region` overrides it).
pub const REGION: Region = Region::Ntsc;
/// Video frames per logic tick: 2 if the original's main loop waits for two
/// vertical blanks per update. Read it from the original's main loop.
pub const FRAMES_PER_TICK: u32 = 1;
/// The original's display size: 320x224 (H40) or 256x224 (H32), from its
/// VDP mode registers.
pub const SCREEN: UVec2 = UVec2::new(320, 224);

/// The engine plus every feature of the game.
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EnginePlugin, demo::DemoPlugin));
    }
}
