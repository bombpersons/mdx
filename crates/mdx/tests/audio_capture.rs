//! WAV/state scope and limits at the CLI/MCP shared operation boundary.
use mdx::{context::Context, operations::execute};
use serde_json::{json, Value};
use std::{fs, path::Path};
fn rom(path: &Path) {
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(&0xff1000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    bytes[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    let code = [
        0x13, 0xfc, 0x00, 0x80, 0x00, 0xc0, 0x00, 0x11, 0x13, 0xfc, 0x00, 0x04, 0x00, 0xc0, 0x00,
        0x11, 0x13, 0xfc, 0x00, 0x90, 0x00, 0xc0, 0x00, 0x11, 0x60, 0xfe,
    ];
    bytes[0x200..0x200 + code.len()].copy_from_slice(&code);
    fs::write(path, bytes).unwrap();
}
fn pcm(report: &Value) -> Vec<u8> {
    fs::read(report["wav_artifact"]["path"].as_str().unwrap()).unwrap()[44..].to_vec()
}
#[test]
fn stereo_wav_and_split_state_preserve_exact_sample_continuation() {
    let root = tempfile::tempdir().unwrap();
    rom(&root.path().join("game.md"));
    let context = Context::default();
    let call = |frames, from_state: Option<Value>, warmup| {
        execute(
            "audio",
            json!({"project_root":root.path(),"rom":"game.md","frames":frames,"start_frame":warmup,"from_state":from_state,"save_state":true,"instructions":1000000,"out":"waves"}),
            &context,
        )
    };
    let whole = call(5, None, 0);
    let first = call(2, None, 0);
    assert!(whole.ok, "{whole:?}");
    assert!(first.ok, "{first:?}");
    let second = call(3, Some(first.data["state_artifact"]["id"].clone()), 0);
    assert!(second.ok, "{second:?}");
    let mut joined = pcm(&first.data);
    joined.extend(pcm(&second.data));
    assert_eq!(joined, pcm(&whole.data));
    assert_eq!(
        whole.data["state_artifact"]["id"],
        second.data["state_artifact"]["id"]
    );
    assert_eq!(
        first.data["next_sample_index"],
        second.data["first_sample_index"]
    );
    assert!(whole.data["nonzero_samples"].as_u64().unwrap() > 100);
    let bytes = fs::read(whole.data["wav_artifact"]["path"].as_str().unwrap()).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 44100);
    assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
    assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
    let warm = call(3, None, 2);
    assert!(warm.ok);
    assert_eq!(
        warm.data["wav_artifact"]["id"],
        second.data["wav_artifact"]["id"]
    );
    assert_eq!(
        warm.data["state_artifact"]["id"],
        whole.data["state_artifact"]["id"]
    );
    let stored = execute(
        "artifact.read",
        json!({"project_root":root.path(),"source":whole.data["wav_artifact"]["id"],"length":44}),
        &context,
    );
    assert!(stored.ok);
    assert_eq!(stored.data["length"], 44);
}
#[test]
fn incomplete_warmup_is_explicit_and_invalid_import_has_no_outputs() {
    let root = tempfile::tempdir().unwrap();
    rom(&root.path().join("game.md"));
    let context = Context::default();
    let short = execute(
        "audio",
        json!({"project_root":root.path(),"rom":"game.md","frames":1,"start_frame":120,"instructions":1}),
        &context,
    );
    assert_eq!(short.exit, mdx::envelope::Exit::Partial);
    assert_eq!(short.data["started"], false);
    assert_eq!(short.data["sample_frames"], 0);
    assert_eq!(short.data["frame_target_reached"], false);
    assert_eq!(
        short.data["first_sample_index"],
        short.data["next_sample_index"]
    );
    let bad = execute(
        "audio",
        json!({"project_root":root.path(),"rom":"game.md","frames":0,"out":"invalid-out"}),
        &context,
    );
    assert_eq!(bad.error.unwrap().code, "limit_exceeded");
    assert!(!root.path().join("invalid-out").exists());
    fs::write(root.path().join("bad.json"), "{}").unwrap();
    let bad = execute(
        "audio",
        json!({"project_root":root.path(),"rom":"game.md","from_state":"bad.json","out":"invalid-out"}),
        &context,
    );
    assert_eq!(bad.error.unwrap().code, "invalid_state");
    assert!(!root.path().join("invalid-out").exists());
    let schema = mdx::operations::descriptors()
        .into_iter()
        .find(|d| d.name == "audio")
        .unwrap();
    for field in [
        "from_state",
        "input",
        "out",
        "project_root",
        "artifact_store",
    ] {
        assert_eq!(
            schema.input_schema["properties"][field]["type"], "string",
            "{field}"
        );
        assert!(!schema.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!(field)));
    }
    assert!(schema.file_schemas.contains_key("controller_replay"));
}

#[test]
fn vgm_starts_from_current_chip_state_and_matches_across_saved_states() {
    let root = tempfile::tempdir().unwrap();
    rom(&root.path().join("game.md"));
    let context = Context::default();
    let call = |frames, from_state: Option<Value>, warmup| {
        execute(
            "audio",
            json!({"project_root":root.path(),"rom":"game.md","frames":frames,"start_frame":warmup,"from_state":from_state,"save_state":true,"instructions":1000000,"vgm":true}),
            &context,
        )
    };
    let warm = call(3, None, 2);
    assert!(warm.ok, "{warm:?}");
    let summary = &warm.data["vgm"];
    // The PSG writes happen during warmup; the capture holds none, but its
    // initial state reproduces them (tone 0 latch/data, then volume 0).
    assert_eq!(summary["psg_writes"], 0);
    assert_eq!(summary["fm_initial_state_known"], true);
    let bytes = fs::read(warm.data["vgm_artifact"]["path"].as_str().unwrap()).unwrap();
    assert_eq!(&bytes[..4], b"Vgm ");
    let data = &bytes[0x40..];
    let psg = data.windows(6).position(|w| w == [0x50, 0x80, 0x50, 0x04, 0x50, 0x90]);
    assert!(psg.is_some(), "initial PSG state missing");
    assert_eq!(*bytes.last().unwrap(), 0x66);
    let first = call(2, None, 0);
    let resumed = call(3, Some(first.data["state_artifact"]["id"].clone()), 0);
    assert!(resumed.ok, "{resumed:?}");
    assert_eq!(resumed.data["vgm_artifact"]["id"], warm.data["vgm_artifact"]["id"]);
}

#[test]
fn stems_isolate_each_voice_as_wav_artifacts() {
    let root = tempfile::tempdir().unwrap();
    rom(&root.path().join("game.md"));
    let report = execute(
        "audio",
        json!({"project_root":root.path(),"rom":"game.md","frames":4,"start_frame":1,"instructions":1000000,"stems":true,"out":"waves"}),
        &Context::default(),
    );
    assert!(report.ok, "{report:?}");
    let stems = report.data["stems"].as_array().unwrap();
    let voices: Vec<&str> = stems.iter().map(|s| s["voice"].as_str().unwrap()).collect();
    assert_eq!(voices, ["fm1", "fm2", "fm3", "fm4", "fm5", "fm6", "psg1", "psg2", "psg3", "psg4"]);
    // The ROM plays only PSG channel 1.
    for stem in stems {
        let audible = stem["peak"].as_u64().unwrap() > 0;
        assert_eq!(audible, stem["voice"] == "psg1", "{stem}");
        let bytes = fs::read(stem["wav_artifact"]["path"].as_str().unwrap()).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), fs::read(report.data["wav_artifact"]["path"].as_str().unwrap()).unwrap().len());
    }
}
