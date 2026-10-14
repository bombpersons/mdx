//! Import validation and project-local state ownership at the shared API boundary.
use mdx::{context::Context, operations::execute};
use serde_json::{json, Value};
use std::fs;

#[test]
fn state_validation_precedes_output_and_state_ids_follow_project_scope() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0xff1000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    rom[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    rom[0x200..0x204].copy_from_slice(&[0x4e, 0x71, 0x60, 0xfc]);
    fs::write(a.join("game.md"), &rom).unwrap();
    fs::write(b.join("game.md"), &rom).unwrap();
    rom[0x300] = 1;
    fs::write(b.join("other.md"), rom).unwrap();
    let context = Context::default();
    let saved = execute(
        "boot",
        json!({"project_root":a,"rom":"game.md","instructions":137,"trace":0,"save_state":true}),
        &context,
    );
    let state = saved.data["state_artifact"]["id"].clone();
    let path = saved.data["state_artifact"]["path"].clone();
    assert!(state.as_str().unwrap().starts_with("sha256:"));
    // An ID is resolved in the caller's store; no implicit cross-project lookup.
    let missing = execute(
        "ram.dump",
        json!({"project_root":b,"rom":"game.md","instructions":0,"from_state":state}),
        &context,
    );
    assert_eq!(missing.error.unwrap().code, "artifact_unreadable");
    assert!(!b.join(".mdx").exists());
    let wrong_rom = execute(
        "boot",
        json!({"project_root":b,"rom":"other.md","instructions":0,"from_state":path,"save_state":true,"out":"must-not-exist"}),
        &context,
    );
    assert_eq!(wrong_rom.error.unwrap().code, "invalid_state");
    assert!(!b.join("must-not-exist").exists());
    assert!(!b.join(".mdx").exists());
    let mut bad: Value =
        serde_json::from_slice(&fs::read(path.as_str().unwrap()).unwrap()).unwrap();
    bad["version"] = json!(999);
    fs::write(b.join("bad.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
    let invalid = execute(
        "boot",
        json!({"project_root":b,"rom":"game.md","instructions":0,"from_state":"bad.json","out":"must-not-exist"}),
        &context,
    );
    assert_eq!(invalid.error.unwrap().code, "invalid_state");
    assert!(!b.join("must-not-exist").exists());
    // Import publishes the same bytes into B, then the original ID works there.
    let imported = execute(
        "artifact.import",
        json!({"project_root":b,"path":path}),
        &context,
    );
    assert_eq!(imported.data["artifact"]["id"], state);
    let restored = execute(
        "ram.dump",
        json!({"project_root":b,"rom":"game.md","instructions":0,"from_state":state,"save_state":true}),
        &context,
    );
    assert_eq!(restored.data["state_artifact"]["id"], state);
    assert!(restored.data["state_artifact"]["path"]
        .as_str()
        .unwrap()
        .starts_with(b.to_str().unwrap()));
    // A captured-buffer snapshot is a different format and cannot resume a CPU.
    let snapshot = saved.data["snapshot_artifact"]["path"].clone();
    let invalid = execute(
        "ram.dump",
        json!({"project_root":b,"rom":"game.md","instructions":0,"from_state":snapshot}),
        &context,
    );
    assert_eq!(invalid.error.unwrap().code, "invalid_state");
    let forced = execute(
        "boot",
        json!({"project_root":a,"rom":"game.md","from_state":state,"instructions":0,"trace":0,"force_branch":["0x202=skip"],"save_state":true}),
        &context,
    );
    let forced_id = forced.data["state_artifact"]["id"].clone();
    for name in ["boot", "ram.dump", "trace", "watch", "audio"] {
        let mut args =
            json!({"project_root":a,"rom":"game.md","from_state":forced_id,"instructions":1});
        if name == "watch" {
            args["watch"] = json!({"address":0,"length":1});
        }
        let result = execute(name, args, &context);
        assert!(
            result
                .coverage
                .limitations
                .iter()
                .any(|s| s.contains("analyst-forced")),
            "{name}"
        );
    }
}
