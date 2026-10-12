//! Render a digest-verified VDP snapshot without running the CPU.
use crate::envelope::{Coverage, Envelope, Failure, Subject};
use mdcore::vdp::Vdp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const TOOL: &str = "mdx/render";

pub(super) use crate::artifacts::Artifact;

pub(super) struct VerifiedSnapshot {
    pub vdp: Vdp,
    pub memory_artifacts: BTreeMap<String, Artifact>,
    pub subject: Subject,
}

#[derive(Deserialize)]
struct Registers {
    registers: Vec<u8>,
}

#[derive(Deserialize)]
struct Snapshot {
    vdp: Registers,
    memory_artifacts: BTreeMap<String, Artifact>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RenderData {
    screenshot_artifact: Artifact,
    memory_artifacts: BTreeMap<String, Artifact>,
    registers: Vec<u8>,
    width: usize,
    height: usize,
    screenshot_path: PathBuf,
    screenshot_sha256: String,
    renderer_limitations: Vec<String>,
}

fn failure(code: &str, message: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        message: message.into(),
        remediation: None,
    }
}

pub(super) fn load(path: &Path, store: &Path) -> Result<VerifiedSnapshot, Failure> {
    let bytes = crate::artifacts::read_bounded(path, 64 * 1024 * 1024)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| failure("snapshot_invalid", e.to_string()))?;
    // Accept the complete boot envelope or its data object. Paths are explicit
    // filesystem paths, resolved against the invocation's working directory.
    let mut snapshot: Snapshot =
        serde_json::from_value(value.get("data").unwrap_or(&value).clone())
            .map_err(|e| failure("snapshot_invalid", e.to_string()))?;
    if snapshot.vdp.registers.len() != 32 {
        return Err(failure(
            "snapshot_invalid",
            "expected exactly 32 VDP register bytes",
        ));
    }
    for artifact in snapshot.memory_artifacts.values_mut() {
        crate::artifacts::rebase(artifact, store)?;
        if !artifact.id.is_empty() && artifact.id != format!("sha256:{}", artifact.sha256) {
            return Err(failure(
                "artifact_digest_mismatch",
                "snapshot artifact ID does not match its declared SHA-256",
            ));
        }
        artifact.id = format!("sha256:{}", artifact.sha256);
        artifact.path = crate::artifacts::absolute(&artifact.path)?;
    }
    let mut vdp = Vdp::new();
    vdp.registers.copy_from_slice(&snapshot.vdp.registers);
    for (name, destination) in [
        ("vram", &mut vdp.vram),
        ("cram", &mut vdp.cram),
        ("vsram", &mut vdp.vsram),
    ] {
        let artifact = snapshot
            .memory_artifacts
            .get_mut(name)
            .ok_or_else(|| failure("snapshot_invalid", format!("missing {name} artifact")))?;
        let content = crate::artifacts::read_bounded(&artifact.path, 65536)?;
        if content.len() != destination.len() || content.len() != artifact.bytes {
            return Err(failure(
                "artifact_size_mismatch",
                format!(
                    "{name}: expected {} bytes, got {}",
                    destination.len(),
                    content.len()
                ),
            ));
        }
        if format!("{:x}", Sha256::digest(&content)) != artifact.sha256 {
            return Err(failure(
                "artifact_digest_mismatch",
                format!("{name}: bytes do not match the declared SHA-256"),
            ));
        }
        destination.copy_from_slice(&content);
        artifact.id = format!("sha256:{}", artifact.sha256);
        artifact.path = crate::artifacts::absolute(&artifact.path)?;
    }
    Ok(VerifiedSnapshot {
        vdp,
        memory_artifacts: snapshot.memory_artifacts,
        subject: Subject {
            path: path.display().to_string(),
            sha256: hash,
            bytes: bytes.len(),
            format: "vdp-snapshot-json".into(),
            architecture: None,
        },
    })
}

pub fn run(
    path: &Path,
    output: Option<&Path>,
    store: &Path,
) -> Result<Envelope<RenderData>, Failure> {
    let VerifiedSnapshot {
        mut vdp,
        memory_artifacts,
        subject,
    } = load(path, store)?;
    let default_output = PathBuf::from(".mdx/render").join(&subject.sha256[..16]);
    // No time-varying raster state is present in these buffers.
    let (frame, mut limitations) = vdp.render();
    limitations.push(
        "snapshot only: raster changes, FIFO contention, and scanline history are not captured"
            .into(),
    );
    let screenshot = crate::artifacts::named(
        output.unwrap_or(&default_output),
        "frame",
        "png",
        &super::boot::encode_png(&frame)
            .map_err(|e| failure("screenshot_write_failed", e.to_string()))?,
    )?;
    let screenshot_path = screenshot.path.clone();
    let screenshot_sha256 = screenshot.sha256.clone();
    let coverage = Coverage {
        limitations: limitations.clone(),
        ..Coverage::complete(vec![
            "digest-verified VRAM/CRAM/VSRAM and 32 register bytes".into(),
        ])
    };
    Ok(Envelope::ok(
        TOOL,
        Some(subject),
        coverage,
        RenderData {
            screenshot_artifact: screenshot,
            memory_artifacts,
            registers: vdp.registers.to_vec(),
            width: frame.width,
            height: frame.height,
            screenshot_path,
            screenshot_sha256,
            renderer_limitations: limitations,
        },
    ))
}
