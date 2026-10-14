//! Finite, independent first-image gate. This checks a real-ROM boot and the
//! decoded pixel bytes of a GPGX capture, not our own implementation's output.
use std::fs::File;
use std::path::Path;
use std::process::Command;

fn pixels(path: &Path) -> (u32, u32, Vec<u8>) {
    // Expand indexed screenshots so both sides compare as RGB pixels.
    let mut decoder = png::Decoder::new(File::open(path).expect("PNG exists"));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("valid PNG");
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).expect("complete PNG");
    assert_eq!(reader.output_color_type().0, png::ColorType::Rgb);
    bytes.truncate(info.buffer_size());
    (info.width, info.height, bytes)
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn unmodified_kings_bounty_frame400_matches_the_independent_oracle() {
    assert_frame(400, "oracle_kb_frame400_rgb.png", None);
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn unmodified_kings_bounty_title_matches_the_independent_oracle() {
    assert_frame(1200, "oracle_kb_frame1200_rgb.png", None);
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn controlled_character_selection_matches_the_independent_oracle() {
    assert_frame(3600, "oracle_kb_character3600_rgb.png", Some("kb_character_replay.json"));
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn controlled_difficulty_selection_matches_the_independent_oracle() {
    assert_frame(3900, "oracle_kb_difficulty3900_rgb.png", Some("kb_new_game_replay.json"));
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn controlled_overworld_start_matches_the_independent_oracle() {
    assert_frame(4300, "oracle_kb_world4300_rgb.png", Some("kb_world_replay.json"));
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn short_directional_replay_matches_the_independent_oracle() {
    assert_frame(4500, "oracle_kb_move4500_rgb.png", Some("kb_move_replay.json"));
}

#[test]
#[ignore = "live ROM/oracle gate; run explicitly with --ignored"]
fn sustained_overworld_walking_matches_the_independent_oracle() {
    assert_frame(4500, "oracle_kb_walk4500_rgb.png", Some("kb_walk_replay.json"));
}

fn assert_frame(frame: u64, reference: &str, input: Option<&str>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::temp_dir().join(format!("mdx-kb-golden-{}-{frame}-{reference}", std::process::id()));
    let binary = std::env::var_os("MDX_GOLDEN_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_mdx").into());
    let mut command = Command::new(binary);
    command.current_dir(&root)
        .args(["boot", "--rom", "rom/original/kings_bounty.md", "--frames", &frame.to_string(),
            "--instructions", "80000000", "--trace", "0", "--out"])
        .arg(&output);
    if let Some(input) = input { command.arg("--input").arg(root.join("testdata").join(input)); }
    let result = command.output().expect("run mdx");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).expect("JSON report");
    assert!(result.status.success(), "{report}");
    assert_eq!(report["data"]["vdp"]["frame"], frame);
    assert_eq!(report["data"]["exceptions"].as_array().unwrap().len(), 0);
    assert_eq!(report["data"]["input_replay"]["events_pending"], 0);
    let actual = pixels(Path::new(report["data"]["display"]["screenshot_path"].as_str().unwrap()));
    let reference = pixels(&root.join("testdata").join(reference));
    assert_eq!((actual.0, actual.1), (reference.0, reference.1));
    let changed = actual.2.chunks_exact(3).zip(reference.2.chunks_exact(3)).filter(|(a,b)| a != b).count();
    assert_eq!(changed, 0, "first image differs at {changed} pixels; artifacts in {}", output.display());
    std::fs::remove_dir_all(output).expect("clean successful golden capture");
}
