//! Indexed sprites: an [`IndexedImage`] drawn with one palette line of the
//! scene's [`Palettes`], baked into the entity's ordinary `Sprite`.
//!
//! Everything else about the sprite (anchor, flips, position, z) is the
//! normal Bevy `Sprite` and `Transform`. Animate by swapping `image` for
//! another frame; recolour by changing `palette` or the palettes.
use super::color::{Palettes, Shade};
use super::indexed::IndexedImage;
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Component, Clone)]
#[require(Sprite)]
pub struct IndexedSprite {
    pub image: Arc<IndexedImage>,
    /// Palette line, 0..=3.
    pub palette: u8,
}

/// Baked images by (indexed image, palette line). Holding the `Arc` keeps
/// the pointer used as a key from being reused.
#[derive(Default)]
pub(crate) struct Baked(HashMap<(usize, u8), (Arc<IndexedImage>, Handle<Image>)>);

pub(crate) fn bake_sprites(
    palettes: Option<Res<Palettes>>,
    mut images: ResMut<Assets<Image>>,
    mut baked: Local<Baked>,
    mut sprites: Query<(Ref<IndexedSprite>, &mut Sprite)>,
) {
    let Some(palettes) = palettes else { return };
    let all = palettes.is_changed();
    if all {
        baked.0.clear();
    }
    for (indexed, mut sprite) in &mut sprites {
        if !all && !indexed.is_changed() {
            continue;
        }
        let line = indexed.palette.min(3);
        let key = (Arc::as_ptr(&indexed.image) as usize, line);
        let (_, handle) = baked.0.entry(key).or_insert_with(|| {
            let image = indexed.image.bake(&palettes.lines[line as usize], Shade::Normal);
            (indexed.image.clone(), images.add(image))
        });
        sprite.image = handle.clone();
    }
}
