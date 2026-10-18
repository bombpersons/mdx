//! Optional Ghidra operations: contracts that hold without Ghidra, and one live
//! import/evidence/decompile/annotate/export round trip that runs only when a
//! Ghidra installation is discoverable (MDX_GHIDRA_HOME, PATH, /opt, ~/tools).
use mdx::{context::Context, operations};
use serde_json::{json, Value};
use std::{fs, path::Path};

/// Reset at 0x200 calls 0x220, then `jsr (a0)` through a RAM pointer to 0x240.
fn rom(dir: &Path) -> String {
    let mut b = vec![0u8; 0x400];
    b[0..4].copy_from_slice(&0x00FF_1000u32.to_be_bytes());
    b[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    b[0x78..0x7C].copy_from_slice(&0x250u32.to_be_bytes());
    b[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    let code: &[(usize, &[u8])] = &[
        // jsr $220; movea.l ($FFFF0000).l,a0; jsr (a0); bra.s *
        (0x200, &[0x4E, 0xB9, 0, 0, 0x02, 0x20, 0x20, 0x79, 0xFF, 0xFF, 0, 0, 0x4E, 0x90, 0x60, 0xFE]),
        // move.w #5,d0; add.w d1,d0; move.w d0,($FF8000).l; rts
        (0x220, &[0x30, 0x3C, 0, 5, 0xD0, 0x41, 0x33, 0xC0, 0, 0xFF, 0x80, 0, 0x4E, 0x75]),
        // moveq #1,d0; rts
        (0x240, &[0x70, 0x01, 0x4E, 0x75]),
        // rte
        (0x250, &[0x4E, 0x73]),
    ];
    for (at, bytes) in code {
        b[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    fs::write(dir.join("game.md"), b).unwrap();
    "game.md".into()
}

fn call(name: &str, mut args: Value, root: &Path) -> Value {
    args["project_root"] = json!(root);
    let response = serde_json::to_value(operations::execute(name, args, &Context::default())).unwrap();
    let descriptor = operations::descriptors().into_iter().find(|d| d.name == name).unwrap();
    let validator = jsonschema::validator_for(&descriptor.output_schema).unwrap();
    assert!(validator.is_valid(&response), "schema mismatch for {name}: {response}");
    response
}

fn workspace() -> tempfile::TempDir {
    // A visible directory name: Ghidra rejects project paths with hidden components.
    tempfile::Builder::new().prefix("mdx-ghidra-test").tempdir().unwrap()
}

#[test]
fn ghidra_tools_are_registered_with_strict_inputs() {
    let names: Vec<String> = operations::descriptors().iter().map(|d| d.mcp_name.clone()).collect();
    for tool in [
        "ghidra_doctor",
        "ghidra_analyze",
        "ghidra_functions",
        "ghidra_decompile",
        "ghidra_xrefs",
        "ghidra_annotate",
        "ghidra_export_symbols",
    ] {
        assert!(names.contains(&tool.to_string()), "{tool} missing");
    }
    for d in operations::descriptors().into_iter().filter(|d| d.name.starts_with("ghidra.")) {
        assert_eq!(d.input_schema["additionalProperties"], false, "{}", d.name);
    }
    let decompile = operations::descriptors().into_iter().find(|d| d.name == "ghidra.decompile").unwrap();
    assert!(decompile.input_schema["required"].as_array().unwrap().contains(&json!("addr")));
    let unknown = call("ghidra.decompile", json!({"rom":"game.md","addr":"200","bogus":1}), workspace().path());
    assert_eq!(unknown["error"]["code"], "invalid_arguments");
}

#[test]
fn missing_ghidra_is_a_clear_failure_and_creates_nothing() {
    let dir = workspace();
    let rom = rom(dir.path());
    let response = call("ghidra.decompile", json!({"rom":rom,"addr":"200","ghidra_home":"/nonexistent/ghidra"}), dir.path());
    assert_eq!(response["ok"], false);
    assert_eq!(response["exit"], "failed");
    assert_eq!(response["error"]["code"], "ghidra_unavailable");
    assert!(response["error"]["remediation"].as_str().unwrap().contains("MDX_GHIDRA_HOME"));
    assert!(!dir.path().join(".mdx/ghidra").exists());
    let doctor = call("ghidra.doctor", json!({"ghidra_home":"/nonexistent/ghidra"}), dir.path());
    assert_eq!(doctor["error"]["code"], "ghidra_unavailable");
    let guide = call("guide", json!({"topic":"ghidra"}), dir.path());
    assert!(guide["data"]["text"].as_str().unwrap().contains("hypothesis"));
}

#[test]
fn live_ghidra_round_trip_uses_runtime_evidence() {
    let dir = workspace();
    let root = dir.path();
    let doctor = call("ghidra.doctor", json!({}), root);
    if doctor["error"]["code"] == "ghidra_unavailable" {
        eprintln!("skipping: Ghidra not installed ({})", doctor["error"]["message"]);
        return;
    }
    assert_eq!(doctor["data"]["status"], "available", "{doctor}");
    let rom = rom(root);
    // The indirect call's target exists only at run time.
    let evidence: String = [(0x200, 0x220), (0x220, 0x224), (0x22C, 0x206), (0x20C, 0x240), (0x240, 0x242)]
        .iter()
        .map(|(pc, next)| json!({"pc":pc,"next_pc":next,"exception":null}).to_string() + "\n")
        .collect();
    fs::write(root.join("trace.jsonl"), evidence).unwrap();
    let decompiled = call("ghidra.decompile", json!({"rom":rom,"addr":"242","evidence":["trace.jsonl"]}), root);
    assert_eq!(decompiled["ok"], true, "{decompiled}");
    let data = &decompiled["data"];
    assert_eq!(data["function"]["entry"], "0x00000240");
    assert_eq!(data["decompile_status"], "complete");
    assert!(data["pseudocode"].as_str().unwrap().contains("return"), "{}", data["pseudocode"]);
    assert!(data["listing"].as_array().unwrap().iter().any(|l| l["text"].as_str().unwrap().starts_with("moveq")));
    assert_eq!(data["evidence_coverage"]["executed_pcs"], 5);
    assert_eq!(data["evidence_coverage"]["in_functions"], 5);
    let caller = &data["callers"]["items"][0];
    assert_eq!((caller["from"].as_str(), caller["type"].as_str()), (Some("0x0000020C"), Some("COMPUTED_CALL")));
    // Rows name the functions at both ends, so no second lookup is needed.
    assert!(caller["function_name"].is_string(), "{caller}");
    let analysis = data["analysis"].as_str().unwrap().to_string();

    let reset = call("ghidra.decompile", json!({"analysis":analysis,"addr":"200"}), root);
    let callees = reset["data"]["callees"]["items"].as_array().unwrap();
    assert!(callees.iter().any(|c| c["to"] == "0x00000220" && c["to_name"].is_string()), "{callees:?}");

    let functions = call("ghidra.functions", json!({"analysis":analysis,"filter":"vblank"}), root);
    assert_eq!(functions["data"]["functions"]["items"][0]["entry"], "0x00000250");
    // Work RAM through the absolute-short alias folds to one address.
    let xrefs = call("ghidra.xrefs", json!({"analysis":analysis,"addr":"FFFF8000"}), root);
    assert_eq!(xrefs["data"]["addr"], "0x00FF8000");
    assert!(xrefs["data"]["references_to"]["items"].as_array().unwrap().iter().any(|r| r["from"] == "0x00000226"), "{xrefs}");

    let annotated = call("ghidra.annotate", json!({"analysis":analysis,"functions":[{"addr":"240","name":"ReturnOne","comment":"seen via jsr (a0)"}]}), root);
    assert_eq!(annotated["data"]["revision"], 1, "{annotated}");
    let exported = call("ghidra.export_symbols", json!({"analysis":analysis}), root);
    let symbols = exported["data"]["symbols"].as_str().unwrap().to_string();
    assert!(exported["data"]["function_count"].as_u64().unwrap() >= 3, "{exported}");
    // A fresh analysis without evidence still learns the name from the bound bytes.
    let seeded = call("ghidra.functions", json!({"rom":rom,"symbols":[symbols],"filter":"ReturnOne"}), root);
    assert_eq!(seeded["data"]["functions"]["items"][0]["entry"], "0x00000240", "{seeded}");
}

/// `Delay`-style poll: store a count, then spin until an interrupt clears it.
/// Without volatile the decompiler folds the reload into the stored value and
/// prints an infinite loop; with it the read survives.
#[test]
fn live_ghidra_volatile_keeps_polled_reads() {
    let dir = workspace();
    let root = dir.path();
    if call("ghidra.doctor", json!({}), root)["error"]["code"] == "ghidra_unavailable" {
        eprintln!("skipping: Ghidra not installed");
        return;
    }
    let mut b = vec![0u8; 0x400];
    b[0..4].copy_from_slice(&0x00FF_1000u32.to_be_bytes());
    b[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    b[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    // move.l #5,($FFE4EE).l; loop: tst.l ($FFE4EE).l; bne.s loop; rts
    let code = [0x23, 0xFC, 0, 0, 0, 5, 0, 0xFF, 0xE4, 0xEE, 0x4A, 0xB9, 0, 0xFF, 0xE4, 0xEE, 0x66, 0xF8, 0x4E, 0x75];
    b[0x200..0x200 + code.len()].copy_from_slice(&code);
    fs::write(root.join("poll.md"), b).unwrap();
    let plain = call("ghidra.decompile", json!({"rom":"poll.md","addr":"200"}), root);
    let marked = call("ghidra.decompile", json!({"rom":"poll.md","addr":"200","volatile":["FFE4EE:4"]}), root);
    let plain = plain["data"]["pseudocode"].as_str().unwrap().to_string();
    let marked = marked["data"]["pseudocode"].as_str().unwrap().to_string();
    assert!(marked.contains("Ram00ffe4ee = 5;") && marked.contains("!= 0);"), "plain:\n{plain}\nvolatile:\n{marked}");
    assert!(!marked.contains("while( true )"), "{marked}");
}
