//! The mdx template's crate: the Mega Drive look, the tick loop, input and
//! rebinding, the launcher, headless testing and the developer MCP. It is
//! game-agnostic; `engine/` builds on it and `game/` builds on the engine.
//!
//! `mdx template update` replaces this crate with newer versions. Build on it
//! from `engine/` or `game/`; see `AGENTS.md` beside this crate before
//! editing it.
pub mod assets;
pub mod controller;
pub mod controls_menu;
#[cfg(feature = "dev-mcp")]
pub mod dev_mcp;
pub mod launcher;
pub mod look;
pub mod pixel_font;
pub mod rebind;
pub mod screen;
pub mod shell;
pub mod testing;

/// What game code usually needs.
pub mod prelude {
    pub use crate::assets::{AssetRoot, RequiredAssets};
    pub use crate::controller::{Button, ControllerState};
    pub use crate::look::{
        IndexedImage, IndexedSprite, MdColor, Palette, Palettes, PlaneScroll, TileMap, TilePlane, TileRef,
    };
    pub use crate::screen::{Depth, Screen, ScreenPosition};
    pub use crate::shell::{GameClock, GameStatus, GameSystems, Region, TickRate};
}
