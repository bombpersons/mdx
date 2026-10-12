//! Immutable, digest-addressed files, published atomically for concurrent calls.
use crate::envelope::Failure;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Artifact {
    #[serde(default)]
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: usize,
}

pub fn failure(code: &str, message: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        message: message.into(),
        remediation: None,
    }
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn absolute(path: &Path) -> Result<PathBuf, Failure> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| failure("path_unavailable", e.to_string()))?
            .join(path))
    }
}

pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, Failure> {
    use std::io::Read;
    if !fs::metadata(path)
        .map_err(|e| failure("artifact_unreadable", e.to_string()))?
        .is_file()
    {
        return Err(failure("artifact_invalid", "input must be a regular file"));
    }
    let file = fs::File::open(path).map_err(|e| failure("artifact_unreadable", e.to_string()))?;
    if !file
        .metadata()
        .map_err(|e| failure("artifact_unreadable", e.to_string()))?
        .is_file()
    {
        return Err(failure("artifact_invalid", "input must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| failure("artifact_unreadable", e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(failure(
            "limit_exceeded",
            format!("file exceeds {limit} bytes"),
        ));
    }
    Ok(bytes)
}

fn publish(path: PathBuf, bytes: &[u8], sha256: String) -> Result<Artifact, Failure> {
    let path = absolute(&path)?;
    let parent = path
        .parent()
        .ok_or_else(|| failure("artifact_invalid", "missing artifact directory"))?;
    fs::create_dir_all(parent).map_err(|e| failure("artifact_write_failed", e.to_string()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| failure("artifact_write_failed", e.to_string()))?;
    temporary
        .write_all(bytes)
        .map_err(|e| failure("artifact_write_failed", e.to_string()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|e| failure("artifact_write_failed", e.to_string()))?;
    match temporary.persist_noclobber(&path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = read_bounded(&path, bytes.len() as u64)?;
            if digest(&existing) != sha256 {
                return Err(failure(
                    "artifact_digest_mismatch",
                    "existing content-addressed object is corrupt",
                ));
            }
        }
        Err(error) => return Err(failure("artifact_write_failed", error.error.to_string())),
    }
    Ok(Artifact {
        id: format!("sha256:{sha256}"),
        path,
        bytes: bytes.len(),
        sha256,
    })
}

pub fn put(store: &Path, bytes: &[u8]) -> Result<Artifact, Failure> {
    let sha256 = digest(bytes);
    publish(store.join("objects").join(&sha256), bytes, sha256)
}

pub fn named(
    directory: &Path,
    name: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<Artifact, Failure> {
    let sha256 = digest(bytes);
    publish(
        directory.join(format!("{name}-{sha256}.{extension}")),
        bytes,
        sha256,
    )
}

pub fn resolve(store: &Path, source: &str) -> Result<PathBuf, Failure> {
    if let Some(hash) = source.strip_prefix("sha256:") {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(failure(
                "artifact_invalid",
                "artifact IDs require 64 lowercase SHA-256 hex digits",
            ));
        }
        let path = absolute(&store.join("objects").join(hash))?;
        let bytes = read_bounded(&path, 64 * 1024 * 1024)?;
        if digest(&bytes) != hash {
            return Err(failure(
                "artifact_digest_mismatch",
                "stored object does not match its ID",
            ));
        }
        Ok(path)
    } else {
        let path = absolute(Path::new(source))?;
        let metadata =
            fs::metadata(&path).map_err(|e| failure("artifact_unreadable", e.to_string()))?;
        if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
            return Err(failure(
                "limit_exceeded",
                "input must be a regular file <= 64 MiB",
            ));
        }
        Ok(path)
    }
}

/// Prefer immutable objects for explicit IDs; legacy path/hash inputs keep
/// their original path-based integrity checks.
pub fn rebase(artifact: &mut Artifact, store: &Path) -> Result<(), Failure> {
    if artifact.id.is_empty() {
        return Ok(());
    }
    if artifact.id != format!("sha256:{}", artifact.sha256) {
        return Err(failure(
            "artifact_digest_mismatch",
            "artifact ID disagrees with its SHA-256",
        ));
    }
    if artifact.sha256.len() != 64
        || !artifact
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(failure(
            "invalid_artifact_id",
            "expected a lowercase SHA-256 ID",
        ));
    }
    let candidate = store.join("objects").join(&artifact.sha256);
    match fs::metadata(&candidate) {
        Ok(_) => artifact.path = resolve(store, &artifact.id)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failure("artifact_unreadable", error.to_string())),
    }
    Ok(())
}

pub fn verified(artifact: &Artifact) -> Result<Vec<u8>, Failure> {
    if !artifact.id.is_empty() && artifact.id != format!("sha256:{}", artifact.sha256) {
        return Err(failure(
            "artifact_digest_mismatch",
            "artifact ID disagrees with its declared SHA-256",
        ));
    }
    let bytes = read_bounded(&artifact.path, 64 * 1024 * 1024)?;
    if bytes.len() != artifact.bytes || digest(&bytes) != artifact.sha256 {
        return Err(failure(
            "artifact_digest_mismatch",
            "artifact bytes do not match their declared identity",
        ));
    }
    Ok(bytes)
}
