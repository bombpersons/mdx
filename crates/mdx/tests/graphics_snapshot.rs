//! Known mode-5 encodings exercise the exporter as a native-asset boundary.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

fn verify_artifact(value: &Value) -> Vec<u8> {
    let bytes = fs::read(value["path"].as_str().unwrap()).unwrap();
    assert_eq!(bytes.len(), value["bytes"].as_u64().unwrap() as usize);
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        value["sha256"].as_str().unwrap()
    );
    bytes
}

#[test]
fn exports_complete_tiles_maps_and_linked_sprite_attributes_deterministically() {
    let root = std::env::temp_dir().join(format!("mdx-graphics-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut vram = vec![0u8; 65536];
    vram[32..36].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    // Plane A: priority, palette 2, both flips, tile 1.
    vram[0xE000..0xE002].copy_from_slice(&0xD801u16.to_be_bytes());
    // Sprite 0: y=12, x=20, 2 columns * 3 rows, link 2, palette 1, tile 5.
    for (i, word) in [140u16, 0x0602, 0xA005, 148].into_iter().enumerate() {
        vram[0xCC00 + i * 2..0xCC02 + i * 2].copy_from_slice(&word.to_be_bytes());
    }
    let mut regs = vec![0u8; 32];
    for (i, value) in [(1, 0x44), (2, 0x38), (3, 0x34), (4, 7), (5, 0x66), (12, 1)] {
        regs[i] = value;
    }
    let mut artifacts = serde_json::Map::new();
    for (name, bytes) in [
        ("vram", vram),
        ("cram", vec![0; 128]),
        ("vsram", vec![0; 80]),
    ] {
        let path = root.join(format!("{name}.bin"));
        fs::write(&path, &bytes).unwrap();
        artifacts.insert(name.into(),json!({"path":path,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
    }
    let snapshot = root.join("snapshot.json");
    fs::write(
        &snapshot,
        serde_json::to_vec(&json!({"vdp":{"registers":regs},"memory_artifacts":artifacts}))
            .unwrap(),
    )
    .unwrap();
    let output = root.join("out");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_mdx"))
            .args(["graphics", "--snapshot"])
            .arg(&snapshot)
            .arg("--out")
            .arg(&output)
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["coverage"]["status"], "complete");
    let data = &report["data"];
    assert_eq!(data["tile_count"], 2048);
    assert_eq!(data["linked_sprites"], json!([0, 2]));
    let tiles = verify_artifact(&data["tile_indices"]);
    assert_eq!(tiles.len(), 131072);
    assert_eq!(&tiles[64..72], &[1, 2, 3, 4, 5, 6, 7, 8]);
    let layout: Value = serde_json::from_slice(&verify_artifact(&data["layout"])).unwrap();
    let cell = &layout["planes"][0]["cells"][0];
    assert_eq!(cell["tile"], 1);
    assert_eq!(cell["palette"], 2);
    assert_eq!(cell["horizontal_flip"], true);
    assert_eq!(cell["vertical_flip"], true);
    assert_eq!(cell["high_priority"], true);
    assert_eq!(layout["planes"][0]["cells"].as_array().unwrap().len(), 1024);
    assert_eq!(layout["planes"][2]["cells"].as_array().unwrap().len(), 2048);
    let sprite = &layout["sprites"][0];
    assert_eq!(sprite["x"], 20);
    assert_eq!(sprite["y"], 12);
    assert_eq!(sprite["columns"], 2);
    assert_eq!(sprite["rows"], 3);
    assert_eq!(sprite["tiles"], json!([5, 6, 7, 8, 9, 10]));
    for sheet in data["tile_sheets"].as_array().unwrap() {
        verify_artifact(sheet);
        let reader =
            png::Decoder::new(fs::File::open(Path::new(sheet["path"].as_str().unwrap())).unwrap())
                .read_info()
                .unwrap();
        assert_eq!((reader.info().width, reader.info().height), (256, 512));
    }
    let second = run();
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    regs[12] = 3;
    fs::write(
        &snapshot,
        serde_json::to_vec(&json!({"vdp":{"registers":regs},"memory_artifacts":artifacts}))
            .unwrap(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_mdx"))
        .args(["graphics", "--snapshot"])
        .arg(&snapshot)
        .arg("--out")
        .arg(root.join("unsupported"))
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(!result.status.success());
    assert_eq!(report["error"]["code"], "graphics_mode_unsupported");
    assert!(!root.join("unsupported").exists());
    fs::remove_dir_all(root).unwrap();
}
