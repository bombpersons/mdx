//! Tile planes: a map of 8x8 tiles drawn from an indexed tileset with the
//! scene's [`Palettes`], scrolled as a whole.
//!
//! This is an ordinary tilemap renderer with the Mega Drive's conventions
//! (palette line, flips and a priority bit per tile, optional wrap-around),
//! not a model of the VDP: there are no nametable addresses, registers or
//! VRAM. The map is baked on the CPU into 256x256-pixel chunk images, one
//! set for low-priority and one for high-priority tiles, and only chunks on
//! screen are baked and drawn, so large level maps cost what is visible.
//!
//! Effects that are not here, and where they belong if the original uses
//! them: per-line or per-column scroll (split the plane into strips in the
//! engine, or bake the scrolled rows), the window plane (a second plane
//! fixed on screen), shadow/highlight (a [`Shade`] per draw), raster colour
//! changes. Sprite limits and flicker are hardware artefacts: leave them out
//! unless the game depends on them.
use super::color::{Palettes, Shade};
use super::indexed::{IndexedImage, TILE, rgba, rgba_image};
use crate::screen::{Depth, Screen, ScreenPosition};
use bevy::{prelude::*, sprite::Anchor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Tiles per chunk side.
const CHUNK_TILES: u32 = 32;
const CHUNK: i32 = (CHUNK_TILES * TILE) as i32;

/// One map cell: which tile, drawn with which palette line, how.
///
/// RON form: `(tile: 12)`, with `palette`, `flip_x`, `flip_y` and
/// `priority` only when set, e.g. `(tile: 12, palette: 1, priority: true)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileRef {
    pub tile: u16,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub palette: u8,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_x: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
    /// Drawn in the plane's high-priority band (in front of low sprites).
    #[serde(default, skip_serializing_if = "is_false")]
    pub priority: bool,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

/// A grid of [`TileRef`]s, row-major.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileMap {
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<TileRef>,
}

impl TileMap {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, tiles: vec![TileRef::default(); (width * height) as usize] }
    }
    pub fn get(&self, x: u32, y: u32) -> TileRef {
        self.tiles[(y * self.width + x) as usize]
    }
    pub fn set(&mut self, x: u32, y: u32, tile: TileRef) {
        self.tiles[(y * self.width + x) as usize] = tile;
    }
    /// Size in pixels.
    pub fn pixel_size(&self) -> IVec2 {
        IVec2::new((self.width * TILE) as i32, (self.height * TILE) as i32)
    }
}

/// A scrolling tile layer. Spawn it with an identity `Transform` (move it by
/// scrolling, not by its transform); its chunk sprites are its children.
#[derive(Component, Clone, Debug)]
#[require(PlaneScroll, Transform, Visibility)]
pub struct TilePlane {
    tileset: Arc<IndexedImage>,
    map: TileMap,
    /// Repeat the map in both directions, as the console's planes do.
    pub wrap: bool,
    /// `Transform` z of the low- and high-priority tiles (see [`Depth`]).
    pub depth: [f32; 2],
}

impl TilePlane {
    /// Checks that every cell names a tile in `tileset` and a palette line
    /// 0..=3.
    pub fn new(tileset: Arc<IndexedImage>, map: TileMap) -> Result<Self, String> {
        if map.tiles.len() != (map.width * map.height) as usize {
            return Err(format!("{}x{} map needs {} cells, got {}", map.width, map.height, map.width * map.height, map.tiles.len()));
        }
        if tileset.width % TILE != 0 || tileset.height % TILE != 0 {
            return Err(format!("tileset is {}x{}, not a whole number of tiles", tileset.width, tileset.height));
        }
        let count = tileset.tile_count();
        if let Some((i, cell)) = map.tiles.iter().enumerate().find(|(_, t)| t.tile as u32 >= count || t.palette > 3) {
            return Err(format!(
                "cell ({}, {}) uses tile {} palette {}; the tileset has {count} tiles and there are 4 palette lines",
                i as u32 % map.width,
                i as u32 / map.width,
                cell.tile,
                cell.palette
            ));
        }
        Ok(Self { tileset, map, wrap: false, depth: [Depth::PLANE_B_LOW, Depth::PLANE_B_HIGH] })
    }

    pub fn wrapping(mut self) -> Self {
        self.wrap = true;
        self
    }

    pub fn with_depth(mut self, low: f32, high: f32) -> Self {
        self.depth = [low, high];
        self
    }

    pub fn map(&self) -> &TileMap {
        &self.map
    }

    pub fn tileset(&self) -> &Arc<IndexedImage> {
        &self.tileset
    }

    /// Changes one cell. Panics on a tile or palette line `new` would refuse.
    pub fn set(&mut self, x: u32, y: u32, tile: TileRef) {
        assert!((tile.tile as u32) < self.tileset.tile_count() && tile.palette < 4, "{tile:?} is out of range");
        self.map.set(x, y, tile);
    }

    /// RGBA pixels of chunk (`cx`, `cy`) for one priority, or `None` if no
    /// tile of that priority is in it.
    fn bake_chunk(&self, palettes: &Palettes, cx: u32, cy: u32, priority: bool) -> Option<Image> {
        let x0 = cx * CHUNK_TILES;
        let y0 = cy * CHUNK_TILES;
        let columns = CHUNK_TILES.min(self.map.width - x0);
        let rows = CHUNK_TILES.min(self.map.height - y0);
        let (width, height) = (columns * TILE, rows * TILE);
        let mut data = vec![0u8; (width * height * 4) as usize];
        let mut any = false;
        for ty in 0..rows {
            for tx in 0..columns {
                let cell = self.map.get(x0 + tx, y0 + ty);
                if cell.priority != priority {
                    continue;
                }
                any = true;
                let palette = &palettes.lines[cell.palette as usize];
                for py in 0..TILE {
                    let sy = if cell.flip_y { TILE - 1 - py } else { py };
                    for px in 0..TILE {
                        let sx = if cell.flip_x { TILE - 1 - px } else { px };
                        let index = self.tileset.tile_pixel(cell.tile as u32, sx, sy);
                        let at = (((ty * TILE + py) * width + tx * TILE + px) * 4) as usize;
                        data[at..at + 4].copy_from_slice(&rgba(palette, index, Shade::Normal));
                    }
                }
            }
        }
        any.then(|| rgba_image(width, height, data))
    }
}

/// The map pixel shown at the screen's top-left corner.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaneScroll(pub IVec2);

/// Baked chunk images and the child sprites currently showing them.
#[derive(Component, Default)]
pub(crate) struct PlaneChunks {
    baked: HashMap<(u32, u32, bool), Option<Handle<Image>>>,
    sprites: Vec<Entity>,
}

/// Chunks on screen: (chunk x, chunk y, top-left screen position).
fn visible_chunks(map: &TileMap, scroll: IVec2, screen: IVec2, wrap: bool) -> Vec<(u32, u32, IVec2)> {
    let size = map.pixel_size();
    if size.x == 0 || size.y == 0 {
        return Vec::new();
    }
    let chunks = (size + IVec2::splat(CHUNK - 1)) / CHUNK;
    // With wrap, every copy of the map that overlaps the screen; without it,
    // only the map itself.
    let copies = |axis: usize| {
        if wrap {
            scroll[axis].div_euclid(size[axis])..=(scroll[axis] + screen[axis] - 1).div_euclid(size[axis])
        } else {
            0..=0
        }
    };
    let mut out = Vec::new();
    for copy_y in copies(1) {
        for cy in 0..chunks.y {
            for copy_x in copies(0) {
                for cx in 0..chunks.x {
                    let origin = IVec2::new(copy_x * size.x + cx * CHUNK, copy_y * size.y + cy * CHUNK) - scroll;
                    let extent = (size - IVec2::new(cx, cy) * CHUNK).min(IVec2::splat(CHUNK));
                    let on_screen = origin.x < screen.x
                        && origin.y < screen.y
                        && origin.x + extent.x > 0
                        && origin.y + extent.y > 0;
                    if on_screen {
                        out.push((cx as u32, cy as u32, origin));
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn layout_planes(
    mut commands: Commands,
    screen: Res<Screen>,
    palettes: Option<Res<Palettes>>,
    mut images: ResMut<Assets<Image>>,
    mut planes: Query<(Entity, Ref<TilePlane>, Ref<PlaneScroll>, Option<&mut PlaneChunks>)>,
    mut warned: Local<bool>,
) {
    let Some(palettes) = palettes else {
        if !planes.is_empty() && !*warned {
            warn!("tile planes need a Palettes resource; insert one to draw them");
            *warned = true;
        }
        return;
    };
    let screen_size = IVec2::new(screen.width as i32, screen.height as i32);
    for (entity, plane, scroll, chunks) in &mut planes {
        let Some(mut chunks) = chunks else {
            commands.entity(entity).insert(PlaneChunks::default());
            continue;
        };
        let content_changed =
            chunks.is_added() || plane.is_changed() || palettes.is_changed() || screen.is_changed();
        if content_changed {
            chunks.baked.clear();
        } else if !scroll.is_changed() {
            continue;
        }
        let mut shown = 0;
        for (cx, cy, origin) in visible_chunks(&plane.map, scroll.0, screen_size, plane.wrap) {
            for (priority, z) in [(false, plane.depth[0]), (true, plane.depth[1])] {
                let image = chunks
                    .baked
                    .entry((cx, cy, priority))
                    .or_insert_with(|| plane.bake_chunk(&palettes, cx, cy, priority).map(|i| images.add(i)))
                    .clone();
                let Some(image) = image else { continue };
                let parts = (
                    Sprite::from_image(image),
                    Anchor::TOP_LEFT,
                    ScreenPosition(origin),
                    Transform::from_xyz(0.0, 0.0, z),
                    Visibility::Inherited,
                );
                match chunks.sprites.get(shown) {
                    Some(&sprite) => {
                        commands.entity(sprite).insert(parts);
                    }
                    None => {
                        let sprite = commands.spawn((parts, ChildOf(entity))).id();
                        chunks.sprites.push(sprite);
                    }
                }
                shown += 1;
            }
        }
        for &unused in &chunks.sprites[shown..] {
            commands.entity(unused).insert(Visibility::Hidden);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::color::MdColor;

    fn tileset() -> Arc<IndexedImage> {
        // Tile 0 blank, tile 1 has index 1 at its top-left pixel only.
        let mut pixels = vec![0; 16 * 8];
        pixels[8] = 1;
        Arc::new(IndexedImage::new(16, 8, pixels).unwrap())
    }

    #[test]
    fn maps_with_unknown_tiles_or_palettes_are_refused() {
        let mut map = TileMap::new(2, 2);
        map.set(1, 1, TileRef { tile: 2, ..default() });
        assert!(TilePlane::new(tileset(), map.clone()).unwrap_err().contains("cell (1, 1)"));
        map.set(1, 1, TileRef { tile: 1, palette: 4, ..default() });
        assert!(TilePlane::new(tileset(), map).is_err());
    }

    #[test]
    fn chunks_split_by_priority_and_honour_flips() {
        let mut map = TileMap::new(2, 1);
        map.set(0, 0, TileRef { tile: 1, palette: 2, flip_x: true, ..default() });
        map.set(1, 0, TileRef { tile: 1, palette: 2, priority: true, ..default() });
        let plane = TilePlane::new(tileset(), map).unwrap();
        let mut palettes = Palettes::default();
        palettes.lines[2].0[1] = MdColor::WHITE;
        let low = plane.bake_chunk(&palettes, 0, 0, false).unwrap();
        let high = plane.bake_chunk(&palettes, 0, 0, true).unwrap();
        let pixel = |image: &Image, x: usize| image.data.as_ref().unwrap()[x * 4 + 3];
        // Flipped: the low tile's lit pixel moves to x = 7; the high tile's stays at x = 8.
        assert_eq!((pixel(&low, 0), pixel(&low, 7), pixel(&low, 8)), (0, 255, 0));
        assert_eq!((pixel(&high, 7), pixel(&high, 8)), (0, 255));
    }

    #[test]
    fn wrapping_planes_repeat_to_cover_the_screen() {
        let map = TileMap::new(32, 32); // one 256x256 chunk
        let screen = IVec2::new(320, 224);
        let still = visible_chunks(&map, IVec2::ZERO, screen, false);
        assert_eq!(still, vec![(0, 0, IVec2::ZERO)]);
        let wrapped = visible_chunks(&map, IVec2::new(-16, 100), screen, true);
        let origins: Vec<IVec2> = wrapped.iter().map(|c| c.2).collect();
        assert_eq!(
            origins,
            [-100, 156]
                .into_iter()
                .flat_map(|y| [-240, 16, 272].map(|x| IVec2::new(x, y)))
                .collect::<Vec<_>>()
        );
    }
}
