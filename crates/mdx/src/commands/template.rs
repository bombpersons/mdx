//! Embedded standalone project generation; no compiler or downloaded code runs.
use crate::{
    artifacts,
    context::Context,
    envelope::{Coverage, Envelope, Failure},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    /// Absolute destination in the calling agent's workspace. Must be absent or empty.
    /// An MCP server's launch directory is not the client's working directory.
    pub directory: PathBuf,
    /// Cargo binary package name (lowercase ASCII letters/digits/hyphen/underscore).
    #[serde(default = "default_name")]
    pub name: String,
}
fn default_name() -> String {
    "mdx-game".into()
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedFile {
    pub relative_path: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateData {
    pub directory: PathBuf,
    pub name: String,
    pub bevy_version: String,
    pub rust_version: String,
    pub files: Vec<CreatedFile>,
    pub run_command: String,
    pub developer_command: String,
    pub release_command: String,
}

const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/Cargo.toml"
        )),
    ),
    (
        "Cargo.lock",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/Cargo.lock"
        )),
    ),
    (
        ".gitignore",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/.gitignore"
        )),
    ),
    (
        "README.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/README.md"
        )),
    ),
    (
        "AGENTS.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/AGENTS.md"
        )),
    ),
    (
        "src/main.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/main.rs"
        )),
    ),
    (
        "src/controller.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/controller.rs"
        )),
    ),
    (
        "src/screen.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/screen.rs"
        )),
    ),
    (
        "src/demo.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/demo.rs"
        )),
    ),
    (
        "src/dev_mcp.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/dev_mcp.rs"
        )),
    ),
    (
        "src/rebind.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/rebind.rs"
        )),
    ),
    (
        "src/controls_menu.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/controls_menu.rs"
        )),
    ),
    (
        "src/pixel_font.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/pixel_font.rs"
        )),
    ),
    (
        "src/assets.rs",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/megadrive-bevy/src/assets.rs"
        )),
    ),
];

pub fn create(args: CreateArgs, context: &Context) -> Result<Envelope<CreateData>, Failure> {
    context.check()?;
    if !args.directory.is_absolute() {
        return Err(artifacts::failure(
            "invalid_arguments",
            "template destination must be absolute; pass the calling agent's workspace path",
        ));
    }
    if args.name.is_empty()
        || args.name.len() > 64
        || !args.name.as_bytes()[0].is_ascii_lowercase()
        || !args
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(artifacts::failure("invalid_arguments","package name must start with a lowercase ASCII letter and contain at most64 lowercase letters/digits/hyphens/underscores"));
    }
    let directory = &args.directory;
    if directory.exists() {
        if !directory.is_dir()
            || fs::read_dir(directory)
                .map_err(io_failure)?
                .next()
                .is_some()
        {
            return Err(artifacts::failure(
                "template_collision",
                "destination is not empty; choose a new subdirectory (no files were changed)",
            ));
        }
    }
    let parent = directory.parent().ok_or_else(|| {
        artifacts::failure("invalid_arguments", "destination needs a parent directory")
    })?;
    fs::create_dir_all(parent).map_err(io_failure)?;
    let staging = tempfile::Builder::new()
        .prefix(".mdx-template-")
        .tempdir_in(parent)
        .map_err(io_failure)?;
    let mut files = Vec::new();
    for (relative, source) in FILES {
        context.check()?;
        let content = if matches!(*relative, "Cargo.toml" | "Cargo.lock") {
            source.replace("name = \"mdx-game\"", &format!("name = {:?}", args.name))
        } else {
            source.to_string()
        };
        let path = staging.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).map_err(io_failure)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_failure)?;
        file.write_all(content.as_bytes()).map_err(io_failure)?;
        files.push(CreatedFile {
            relative_path: relative.to_string(),
            sha256: artifacts::digest(content.as_bytes()),
            bytes: content.len(),
        });
    }
    context.check()?;
    fs::rename(staging.path(), directory).map_err(io_failure)?;
    let directory = fs::canonicalize(directory).map_err(io_failure)?;
    Ok(Envelope::ok("mdx.template.create",None,Coverage::complete(vec![
        format!("{} embedded standalone project files published; existing nonempty destinations refused",files.len()),
        "native Bevy project generation only; no build, game, network or emulator execution".into(),
    ]),CreateData {directory,name:args.name,bevy_version:"0.20.0".into(),rust_version:"1.97.1".into(),files,
        run_command:"cargo run".into(),developer_command:"cargo run --features dev-mcp -- --mcp".into(),release_command:"cargo build --release".into()}))
}
fn io_failure(error: std::io::Error) -> Failure {
    artifacts::failure("template_write_failed", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_project_is_standalone_and_preserves_collisions() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("game");
        let response = create(
            CreateArgs {
                directory: directory.clone(),
                name: "native-kings".into(),
            },
            &Context::default(),
        )
        .unwrap();
        let data = response.data;
        assert_eq!(data.files.len(), FILES.len());
        let manifest = fs::read_to_string(directory.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("name = \"native-kings\""));
        assert!(manifest.contains("[workspace]"));
        assert!(!manifest.contains("mdcore"));
        let original = fs::read(directory.join("src/main.rs")).unwrap();
        let error = create(
            CreateArgs {
                directory: directory.clone(),
                name: "other".into(),
            },
            &Context::default(),
        )
        .unwrap_err();
        assert_eq!(error.code, "template_collision");
        assert_eq!(fs::read(directory.join("src/main.rs")).unwrap(), original);
        for file in data.files {
            assert_eq!(
                artifacts::digest(&fs::read(directory.join(file.relative_path)).unwrap()),
                file.sha256
            );
        }
    }
    #[test]
    fn destination_requires_explicit_workspace_coordinates() {
        let error = create(
            CreateArgs {
                directory: Path::new("game").to_path_buf(),
                name: default_name(),
            },
            &Context::default(),
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
    }
}
