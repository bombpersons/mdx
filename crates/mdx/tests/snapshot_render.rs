//! Snapshot integrity is a boundary: altered bytes must never be rendered as
//! if they were the captured machine state.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

#[test]
fn malformed_or_tampered_snapshot_fails_before_creating_an_image() {
    let root = std::env::temp_dir().join(format!("mdx-render-invalid-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut artifacts = serde_json::Map::new();
    for (name, size) in [("vram", 65536), ("cram", 128), ("vsram", 80)] {
        let bytes = vec![0u8; size];
        let path = root.join(format!("{name}.bin"));
        fs::write(&path, &bytes).unwrap();
        artifacts.insert(
            name.into(),
            json!({"path":path, "bytes":size,
            "sha256":format!("{:x}", Sha256::digest(&bytes))}),
        );
    }
    let original_vram = fs::read(root.join("vram.bin")).unwrap();
    for (name, error) in [
        ("digest", "artifact_digest_mismatch"),
        ("size", "artifact_size_mismatch"),
        ("registers", "snapshot_invalid"),
    ] {
        let mut snapshot = json!({"vdp":{"registers":vec![0u8;32]}, "memory_artifacts":artifacts});
        match name {
            "digest" => snapshot["memory_artifacts"]["vram"]["sha256"] = json!("0".repeat(64)),
            "size" => snapshot["memory_artifacts"]["cram"]["bytes"] = json!(127),
            _ => snapshot["vdp"]["registers"] = json!(vec![0u8; 31]),
        }
        let path = root.join(format!("{name}.json"));
        fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        for command in ["render", "graphics"] {
            let out = root.join(format!("output-{command}-{name}"));
            let result = Command::new(env!("CARGO_BIN_EXE_mdx"))
                .arg(command)
                .arg("--snapshot")
                .arg(&path)
                .arg("--out")
                .arg(&out)
                .output()
                .unwrap();
            let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(result.status.code(), Some(3), "{report}");
            assert_eq!(report["error"]["code"], error);
            assert_eq!(report["ok"], false);
            assert!(
                !out.exists(),
                "invalid snapshot produced an output directory"
            );
        }
    }
    assert_eq!(fs::read(root.join("vram.bin")).unwrap(), original_vram);
    fs::remove_dir_all(root).unwrap();
}
