//! The executable. Window, renderer and command line come from the engine's
//! launcher (`engine::launcher`, see its docs for the flags); the game itself
//! is the library in `src/lib.rs` (crate `game`).
use engine::launcher::{self, Game};

fn main() {
    launcher::run(Game {
        name: env!("CARGO_PKG_NAME"),
        title: game::TITLE,
        region: game::REGION,
        frames_per_tick: game::FRAMES_PER_TICK,
        screen: game::SCREEN,
        assets: concat!(env!("CARGO_MANIFEST_DIR"), "/assets"),
        add: |app| {
            app.add_plugins(game::GamePlugin);
        },
    });
}
