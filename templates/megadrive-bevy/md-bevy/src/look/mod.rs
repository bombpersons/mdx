//! The Mega Drive look, as ordinary Bevy rendering: 9-bit colours, four
//! 16-colour palette lines, indexed art, tile planes and the original's draw
//! priorities, inside the fixed-resolution [`crate::screen::Screen`].
//!
//! It reproduces how the game *looks*, not how the console works. Positions,
//! scroll and art are plain data a game sets from its own state; nothing
//! here models VRAM, the VDP's registers, DMA or timing. If a port finds
//! itself writing tile data to addresses or emulating register writes, the
//! code is an emulator in disguise: express the effect as data instead.
//!
//! - [`MdColor`], [`Palette`], [`Palettes`]: colours with mdx's exact level
//!   mapping, so native screenshots compare pixel for pixel with mdx's.
//! - [`IndexedImage`]: palette-index art from indexed PNGs.
//! - [`TilePlane`] + [`PlaneScroll`]: scrolling tile layers.
//! - [`IndexedSprite`]: a `Sprite` drawn from indexed art and a palette line.
//! - [`crate::screen::ScreenPosition`] and [`crate::screen::Depth`]: top-left
//!   pixel placement and priority order.
mod color;
mod indexed;
mod plane;
mod sprite;

pub use color::{MdColor, Palette, Palettes, Shade};
pub use indexed::{IndexedImage, TILE};
pub use plane::{PlaneScroll, TileMap, TilePlane, TileRef};
pub use sprite::IndexedSprite;

use crate::screen::{ScreenCamera, place_on_screen};
use bevy::prelude::*;
use bevy::transform::TransformSystems;

/// Draws planes and indexed sprites, places [`crate::screen::ScreenPosition`]
/// entities and shows [`Palettes::backdrop`]. The launcher adds it.
pub struct LookPlugin;

impl Plugin for LookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (plane::layout_planes, sprite::bake_sprites, backdrop, place_on_screen)
                .chain()
                .before(TransformSystems::Propagate),
        );
    }
}

fn backdrop(palettes: Option<Res<Palettes>>, mut cameras: Query<&mut Camera, With<ScreenCamera>>) {
    let Some(palettes) = palettes.filter(|p| p.is_changed()) else { return };
    for mut camera in &mut cameras {
        camera.clear_color = ClearColorConfig::Custom(palettes.backdrop.to_color());
    }
}
