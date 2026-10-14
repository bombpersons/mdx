//! Independent finite Cadash pixel gates. External ROM and frozen GPGX captures
//! live in this game's research workspace, not in the distributed toolkit.
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
#[ignore = "external ROM/oracle gate: run explicitly with --ignored"]
fn player_selection_matches_oracle() {
    assert_frame(
        1000,
        "oracle_cadash_start1000.png",
        "cadash_start_replay.json",
    );
}

#[test]
#[ignore = "external ROM/oracle gate: run explicitly with --ignored"]
fn name_entry_matches_oracle() {
    assert_frame(
        1600,
        "oracle_cadash_name1600.png",
        "cadash_play_replay.json",
    );
}

#[test]
#[ignore = "external ROM/oracle gate: run explicitly with --ignored"]
fn first_castle_dialogue_matches_oracle() {
    assert_frame(
        2300,
        "oracle_cadash_castle2300.png",
        "cadash_play_replay.json",
    );
}

#[test]
#[ignore = "external ROM/oracle gate: run explicitly with --ignored"]
fn rightward_movement_attack_and_jump_match_oracle() {
    assert_frame(
        4500,
        "oracle_cadash_walk4500.png",
        "cadash_walk_replay.json",
    );
}

#[test]
#[ignore = "external ROM/oracle gate: run explicitly with --ignored"]
fn extended_leftward_camera_scroll_matches_oracle() {
    assert_frame(
        5500,
        "oracle_cadash_left5500.png",
        "cadash_exit_replay.json",
    );
}

fn assert_frame(frame: u64, reference: &str, input: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let binary = std::env::var_os("MDX_GOLDEN_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mdx")));
    let output =
        std::env::temp_dir().join(format!("mdx-cadash-golden-{}-{frame}", std::process::id()));
    let result = Command::new(binary)
        .current_dir(&root)
        .args([
            "boot",
            "--rom",
            "rom/original/Cadash (USA, Korea).md",
            "--frames",
            &frame.to_string(),
            "--instructions",
            "90000000",
            "--trace",
            "0",
            "--input",
        ])
        .arg(root.join("testdata").join(input))
        .arg("--out")
        .arg(&output)
        .output()
        .expect("run canonical mdx");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).expect("JSON report");
    assert!(result.status.success(), "{report}");
    assert_eq!(
        report["subject"]["sha256"],
        "85e0782411cf50ea92a9982dac1942679f738d46a58fdda5d7d82b943512afd8"
    );
    assert_eq!(report["data"]["vdp"]["frame"], frame);
    assert!(report["data"]["exceptions"].as_array().unwrap().is_empty());
    let replay: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("testdata").join(input)).unwrap()).unwrap();
    let events = replay["events"].as_array().unwrap();
    let applied = events
        .iter()
        .filter(|event| event["frame"].as_u64().unwrap() < frame)
        .count();
    assert_eq!(report["data"]["input_replay"]["events_applied"], applied);
    assert_eq!(
        report["data"]["input_replay"]["events_pending"],
        events.len() - applied
    );
    let a = pixels(Path::new(
        report["data"]["display"]["screenshot_path"]
            .as_str()
            .unwrap(),
    ));
    let b = pixels(&root.join("testdata").join(reference));
    assert_eq!((a.0, a.1), (b.0, b.1));
    let changed =
        a.2.chunks_exact(3)
            .zip(b.2.chunks_exact(3))
            .filter(|(x, y)| x != y)
            .count();
    assert_eq!(
        changed,
        0,
        "{changed} changed pixels; artifacts at {}",
        output.display()
    );
    std::fs::remove_dir_all(output).expect("clean successful gate");
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
