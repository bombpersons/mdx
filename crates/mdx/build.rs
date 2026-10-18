//! Embeds every file of `templates/megadrive-bevy` for `template.create` and
//! `template.update`, so a file added to the template can't be left out.
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Build output and local tool state, never part of the template.
const SKIP_DIRS: &[&str] = &["target", ".mdx", "captures"];

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                collect(root, &path, out);
            }
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../templates/megadrive-bevy").canonicalize().unwrap();
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let mut code = String::from("&[\n");
    for file in files {
        let relative = file.to_string_lossy().replace('\\', "/");
        let absolute = root.join(&file);
        writeln!(code, "    ({relative:?}, include_bytes!({:?}).as_slice()),", absolute.display().to_string()).unwrap();
    }
    code.push(']');
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("template_files.rs");
    std::fs::write(out, code).unwrap();
}
