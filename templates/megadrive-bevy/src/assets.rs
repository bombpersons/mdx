//! Where game assets are loaded from, and which of them must be ready before a
//! screenshot can stand for the real scene.
//!
//! Register every handle the scene depends on with `RequiredAssets::require`.
//! Developer screenshots then wait until those assets have loaded, and return
//! an error naming any that failed, instead of capturing an empty scene.
use bevy::{
    asset::{LoadState, UntypedHandle},
    prelude::*,
};
use std::path::{Path, PathBuf};

/// Environment variable naming the asset directory.
pub const ASSET_DIR_ENV: &str = "GAME_ASSET_DIR";

/// Absolute asset root, chosen in order: `--assets <dir>`, `$GAME_ASSET_DIR`,
/// `assets/` beside the executable (shipping layout: binary + assets),
/// `assets/` in the working directory, then this project's `assets/` (the
/// development tree). Bevy otherwise resolves relative paths differently for
/// `cargo run` and a directly launched binary.
pub fn asset_root() -> String {
    let mut args = std::env::args().skip_while(|arg| arg != "--assets");
    let explicit = args.nth(1).map(PathBuf::from);
    let from_env = std::env::var_os(ASSET_DIR_ENV).map(PathBuf::from);
    let beside_binary = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets")))
        .filter(|dir| dir.is_dir());
    let working = std::env::current_dir()
        .ok()
        .map(|dir| dir.join("assets"))
        .filter(|dir| dir.is_dir());
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let root = explicit
        .or(from_env)
        .or(beside_binary)
        .or(working)
        .unwrap_or(development);
    std::path::absolute(&root).unwrap_or(root).to_string_lossy().into_owned()
}

/// Handles the visible scene depends on.
#[derive(Resource, Default)]
pub struct RequiredAssets(Vec<UntypedHandle>);

// Starter API: the demo scene draws only colored sprites, so it registers none.
#[allow(dead_code)]
impl RequiredAssets {
    pub fn require<A: Asset>(&mut self, handle: &Handle<A>) {
        self.0.push(handle.clone().untyped());
    }

    /// Required assets (with their dependencies) not loaded yet.
    pub fn pending(&self, server: &AssetServer) -> usize {
        self.0.iter().filter(|h| !server.is_loaded_with_dependencies(h.id())).count()
    }

    /// Paths (or ids) of required assets that failed to load.
    pub fn failed(&self, server: &AssetServer) -> Vec<String> {
        self.0
            .iter()
            .filter(|h| matches!(server.get_load_state(h.id()), Some(LoadState::Failed(_))))
            .map(|h| h.path().map_or_else(|| format!("{:?}", h.id()), |p| p.to_string()))
            .collect()
    }
}
