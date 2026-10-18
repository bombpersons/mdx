//! Where game assets are loaded from, and which of them must be ready before a
//! screenshot can stand for the real scene.
//!
//! [`AssetRoot`] is the one directory both Bevy's asset server and the
//! synchronous loaders ([`AssetRoot::load_ron`], indexed images) read from,
//! so headless tests see the same files as the game.
//!
//! Register every handle the scene depends on with `RequiredAssets::require`.
//! Developer screenshots then wait until those assets have loaded, and return
//! an error naming any that failed, instead of capturing an empty scene.
use bevy::{
    asset::{LoadState, UntypedHandle},
    prelude::*,
};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// Environment variable naming the asset directory.
pub const ASSET_DIR_ENV: &str = "GAME_ASSET_DIR";

/// The absolute asset directory. The launcher chooses it with [`resolve`];
/// without the launcher (tests) it is `assets/` in the crate being run.
#[derive(Resource, Clone, Debug)]
pub struct AssetRoot(pub PathBuf);

impl Default for AssetRoot {
    fn default() -> Self {
        let crate_dir = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
        let base = crate_dir.or_else(|| std::env::current_dir().ok()).unwrap_or_default();
        Self(base.join("assets"))
    }
}

impl AssetRoot {
    /// `<root>/<relative>`.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    pub fn read(&self, relative: &str) -> Result<Vec<u8>, String> {
        let path = self.path(relative);
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Reads and parses `<root>/<relative>` as RON.
    pub fn load_ron<T: DeserializeOwned>(&self, relative: &str) -> Result<T, String> {
        let path = self.path(relative);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The asset root, chosen in order: `--assets <dir>`, `$GAME_ASSET_DIR`,
/// `assets/` beside the executable (shipping layout: binary + assets),
/// `assets/` in the working directory, then `development` (the crate's own
/// `assets/`, which the launcher's caller passes in). Bevy otherwise resolves
/// relative paths differently for `cargo run` and a directly launched binary.
pub fn resolve(development: &Path) -> AssetRoot {
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
    let root = explicit
        .or(from_env)
        .or(beside_binary)
        .or(working)
        .unwrap_or_else(|| development.to_path_buf());
    AssetRoot(std::path::absolute(&root).unwrap_or(root))
}

/// Handles the visible scene depends on.
#[derive(Resource, Default)]
pub struct RequiredAssets(Vec<UntypedHandle>);

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
