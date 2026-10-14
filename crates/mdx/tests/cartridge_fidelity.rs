//! New same-frame reference gates from the independent, unmodified GPGX core.
//! Research cartridges/references are external to the standalone toolkit.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
#[ignore = "external four-ROM gate; run explicitly with --ignored"]
fn reviewed_and_extended_checkpoints_match_frozen_oracle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let binary = std::env::var_os("MDX_GOLDEN_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mdx")));
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("testdata/oracle_fidelity/manifest.json")).unwrap(),
    )
    .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let game = case["game"].as_str().unwrap();
        let frame = case["frame"].as_u64().unwrap();
        if let Some(input) = case["input"].as_str() {
            assert_eq!(
                file_hash(&root.join(input)),
                case["input_sha256"].as_str().unwrap()
            );
        }
        assert_eq!(
            file_hash(&root.join(case["reference"].as_str().unwrap())),
            case["reference_sha256"].as_str().unwrap()
        );
        let out = std::env::temp_dir().join(format!(
            "mdx-reviewed-{}-{game}-{frame}",
            std::process::id()
        ));
        let mut command = Command::new(&binary);
        command
            .current_dir(&root)
            .args([
                "boot",
                "--rom",
                case["rom"].as_str().unwrap(),
                "--frames",
                &frame.to_string(),
                "--instructions",
                "100000000",
                "--trace",
                "0",
                "--out",
            ])
            .arg(&out);
        if let Some(input) = case["input"].as_str() {
            command.arg("--input").arg(input);
        }
        let result = command.output().unwrap();
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(result.status.success(), "{game}/{frame}: {report}");
        assert_eq!(report["subject"]["sha256"], case["rom_sha256"]);
        assert_eq!(report["data"]["vdp"]["frame"], frame);
        assert!(report["data"]["exceptions"].as_array().unwrap().is_empty());
        let actual = pixels(Path::new(
            report["data"]["display"]["screenshot_path"]
                .as_str()
                .unwrap(),
        ));
        let expected = pixels(&root.join(case["reference"].as_str().unwrap()));
        assert_eq!((actual.0, actual.1), (expected.0, expected.1));
        let changed = actual
            .2
            .chunks_exact(3)
            .zip(expected.2.chunks_exact(3))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            changed,
            0,
            "{game}/{frame}: {changed} changed pixels; {}",
            out.display()
        );
        std::fs::remove_dir_all(out).unwrap();
    }
}

fn pixels(path: &Path) -> (u32, u32, Vec<u8>) {
    // Expand indexed screenshots so both sides compare as RGB pixels.
    let mut decoder = png::Decoder::new(std::fs::File::open(path).unwrap());
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).unwrap();
    data.truncate(info.buffer_size());
    assert_eq!(reader.output_color_type().0, png::ColorType::Rgb);
    (info.width, info.height, data)
}

fn file_hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}
