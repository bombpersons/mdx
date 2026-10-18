//! Hand-editable data: RON files under the running crate's asset root.
//!
//! Design numbers inside a controller (speeds, damage, timers) live in a
//! per-type tuning file, so they can be modded without code changes; the
//! controller reads them from a resource. Structural details (update order,
//! rounding, fixed-point widths) stay in code.
//!
//! Files load synchronously when the app is built
//! ([`md_bevy::assets::AssetRoot::load_ron`]), so headless tests see the same
//! values as the game. Move to Bevy asset loaders when content needs
//! streaming or hot reload.
use bevy::prelude::*;
use md_bevy::assets::AssetRoot;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;

/// Inserts resource `T` from `<asset root>/<path>` (RON) when added. Panics
/// with the file and parse error if it is missing or invalid: a game without
/// its tuning is not worth running.
pub struct TuningPlugin<T> {
    pub path: &'static str,
    marker: PhantomData<fn() -> T>,
}

impl<T> TuningPlugin<T> {
    pub const fn new(path: &'static str) -> Self {
        Self { path, marker: PhantomData }
    }
}

impl<T: Resource + DeserializeOwned> Plugin for TuningPlugin<T> {
    fn build(&self, app: &mut App) {
        let root = app.world_mut().get_resource_or_init::<AssetRoot>().clone();
        let value: T = root.load_ron(self.path).unwrap_or_else(|e| panic!("tuning {}: {e}", self.path));
        app.insert_resource(value);
    }
}
