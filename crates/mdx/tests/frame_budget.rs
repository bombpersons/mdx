use mdx::{context::Context, operations};
use serde_json::json;

#[test]
fn numeric_frame_target_and_step_cap_are_distinct_from_interrupt_service() {
    let root = tempfile::tempdir().unwrap();
    let rom = root.path().join("loop.md");
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    bytes[0x200..0x202].copy_from_slice(&[0x60, 0xfe]);
    std::fs::write(&rom, bytes).unwrap();
    let context = Context {
        store: root.path().join("store"),
        ..Default::default()
    };
    let short = operations::execute(
        "boot",
        json!({"rom":rom,"frames":2,"instructions":1,"trace":0,"out":root.path().join("short")}),
        &context,
    );
    assert_eq!(short.data["requested_frames"], 2);
    assert_eq!(short.data["reached_frames"], 0);
    assert_eq!(short.data["frame_target_reached"], false);
    assert_eq!(short.data["instruction_budget"], 1);
    assert_eq!(short.data["termination"], "instruction_budget");
    assert!(short
        .warnings
        .iter()
        .any(|warning| warning.code == "frame_target_not_reached"));
    let reached = operations::execute(
        "boot",
        json!({"rom":rom,"frames":2,"instructions":40000,"trace":0,"out":root.path().join("reached")}),
        &context,
    );
    assert!(reached.ok);
    assert_eq!(reached.data["reached_frames"], 2);
    assert_eq!(reached.data["frame_target_reached"], true);
    assert_eq!(reached.data["termination"], "frame_target");
    assert_eq!(
        reached.data["vdp"]["vint_taken"], 0,
        "video advances while CPU interrupts are disabled"
    );
}

#[test]
fn json_integer_is_accepted_and_numeric_string_is_rejected_without_coercion() {
    let parsed: operations::BootArgs =
        serde_json::from_value(json!({"rom":"unused.md","frames":120})).unwrap();
    assert_eq!(parsed.frames, Some(120));
    let result = operations::execute(
        "boot",
        json!({"rom":"unused.md","frames":"120"}),
        &Context::default(),
    );
    assert_eq!(result.error.unwrap().code, "invalid_arguments");
}
