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

    let brief = call("ghidra.decompile", json!({"analysis":analysis,"addr":"242","brief":true}), root);
    assert!(brief["data"]["pseudocode"].is_string() && brief["data"]["listing"].is_null() && brief["data"]["mapping"].is_null(), "{brief}");
    // One Ghidra run for several functions; the cached 0x242 result is reused.
    let batch = call("ghidra.decompile_batch", json!({"analysis":analysis,"addrs":["220","242","220"]}), root);
    let rows = batch["data"]["results"].as_array().unwrap();
    assert_eq!(rows.iter().map(|r| r["function"]["entry"].as_str().unwrap()).collect::<Vec<_>>(), ["0x00000220", "0x00000240", "0x00000220"], "{batch}");
    assert!(rows.iter().all(|r| r["pseudocode"].is_string() && r["listing"].is_null()), "{batch}");
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
    // A plain symbol line names the RAM global; the size makes it a dword, so
    // the long accesses read `delay_counter` and not `_delay_counter`.
    fs::write(root.join("poll.sym"), "200 Delay\nFFE4EE delay_counter data 4\n").unwrap();
    let marked = call("ghidra.decompile", json!({"rom":"poll.md","addr":"200","volatile":["FFE4EE:4"],"symbols":["poll.sym"]}), root);
    let analysis = marked["data"]["analysis"].as_str().unwrap().to_string();
    let plain = plain["data"]["pseudocode"].as_str().unwrap().to_string();
    let marked = marked["data"]["pseudocode"].as_str().unwrap().to_string();
    assert!(marked.contains("delay_counter = 5;") && marked.contains("!= 0);"), "plain:\n{plain}\nvolatile:\n{marked}");
    assert!(!marked.contains("while( true )"), "{marked}");
    // tst.l sets N and Z from two loads; the unused volatile copy is dropped.
    assert_eq!(marked.matches("= delay_counter;").count(), 1, "{marked}");

    let annotated = call("ghidra.annotate", json!({"analysis":analysis,"labels":[{"addr":"FFFFE4EE","name":"vblank_wait","size":4,"comment":"VBlank decrements"}]}), root);
    assert_eq!(annotated["ok"], true, "{annotated}");
    // WRAM and its absolute-short alias both carry the name.
    assert_eq!(annotated["data"]["labels"].as_array().unwrap().len(), 2, "{annotated}");
    assert_eq!(annotated["data"]["labels"][0]["address"], "0x00FFE4EE", "{annotated}");
    assert!(annotated["data"]["label_size_mismatches"].as_array().unwrap().is_empty(), "{annotated}");
    let renamed = call("ghidra.decompile", json!({"analysis":analysis,"addr":"200"}), root);
    let text = renamed["data"]["pseudocode"].as_str().unwrap();
    assert!(text.contains("vblank_wait = 5;"), "{text}");
    // A word label read by tst.l prints as a partial field; say so.
    let narrow = call("ghidra.annotate", json!({"analysis":analysis,"labels":[{"addr":"FFE4EE","name":"vblank_wait","size":2}]}), root);
    let mismatch = &narrow["data"]["label_size_mismatches"][0];
    assert_eq!((mismatch["size"].as_u64(), mismatch["access_size"].as_u64()), (Some(2), Some(4)), "{narrow}");
    assert!(narrow["warnings"].as_array().unwrap().iter().any(|w| w["message"].as_str().unwrap().contains("vblank_wait")), "{narrow}");
    let bad = call("ghidra.annotate", json!({"analysis":analysis,"labels":[{"addr":"800000","name":"nowhere"}]}), root);
    assert_eq!(bad["error"]["code"], "invalid_arguments", "{bad}");
}

/// King's Bounty calls library routines with `jsr d16(a5)` into a table of
/// `jmp abs.l` stubs. Ghidra loads a pointer from A5+d16 instead, which made
/// every callee 0x4EF9xxxx. With A5 known, the call names the stub's target;
/// a stub to an odd address is reported and not followed.
#[test]
fn live_ghidra_resolves_base_register_thunks() {
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
    let code: &[(usize, &[u8])] = &[
        // jsr 0(a5); jsr 6(a5); bra.s *
        (0x200, &[0x4E, 0xAD, 0, 0, 0x4E, 0xAD, 0, 6, 0x60, 0xFE]),
        // moveq #1,d0; rts
        (0x240, &[0x70, 0x01, 0x4E, 0x75]),
        // jmp $240.l; jmp $241.l (odd: data, never a routine)
        (0x300, &[0x4E, 0xF9, 0, 0, 0x02, 0x40, 0x4E, 0xF9, 0, 0, 0x02, 0x41]),
    ];
    for (at, bytes) in code {
        b[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    fs::write(root.join("thunk.md"), b).unwrap();
    fs::write(root.join("thunk.sym"), "240 ReturnOne\n").unwrap();
    let out = call("ghidra.decompile", json!({"rom":"thunk.md","addr":"200","registers":["A5=300"],"symbols":["thunk.sym"]}), root);
    assert_eq!(out["ok"], true, "{out}");
    let data = &out["data"];
    let text = data["pseudocode"].as_str().unwrap();
    assert!(text.contains("ReturnOne()"), "{text}");
    // The bad stub is still the call's destination, just not a function.
    assert!(!text.contains("(*") && text.contains("0x00000306"), "{text}");
    let base = &data["base_register_calls"];
    assert_eq!(base["resolved"], 1, "{base}");
    assert_eq!(base["rejected"][0]["from"], "0x00000204", "{base}");
    assert_eq!(base["rejected"][0]["target"], "0x00000306", "{base}");
    let callees = data["callees"]["items"].as_array().unwrap();
    assert!(callees.iter().any(|c| c["to"] == "0x00000300" && c["type"] == "CALL_OVERRIDE_UNCONDITIONAL"), "{callees:?}");
    let functions = call("ghidra.functions", json!({"analysis":data["analysis"]}), root);
    let entries: Vec<&str> = functions["data"]["functions"]["items"].as_array().unwrap().iter().map(|f| f["entry"].as_str().unwrap()).collect();
    assert!(!entries.contains(&"0x00000241") && !entries.contains(&"0x00000306"), "{entries:?}");
}

/// `lea -1254(a5),a0; move.w d1,0(a0,d0.w)` with A5=0x312 indexes the work
/// RAM table at 0xFFFFFE2C. Ghidra prints the folded base as `+ -0x1d4`; mdx
/// names it from the lea's reference.
#[test]
fn live_ghidra_names_register_relative_bases() {
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
    // add.w d0,d0; lea -1254(a5),a0; move.w d1,0(a0,d0.w); rts
    let code = [0xD0, 0x40, 0x41, 0xED, 0xFB, 0x1A, 0x31, 0x81, 0, 0, 0x4E, 0x75];
    b[0x200..0x200 + code.len()].copy_from_slice(&code);
    fs::write(root.join("lea.md"), b).unwrap();
    fs::write(root.join("lea.sym"), "200 Store\nFFFE2C table data 2\n").unwrap();
    let out = call("ghidra.decompile", json!({"rom":"lea.md","addr":"200","registers":["A5=312"],"symbols":["lea.sym"]}), root);
    assert_eq!(out["ok"], true, "{out}");
    let text = out["data"]["pseudocode"].as_str().unwrap();
    assert!(text.contains("&table") && !text.contains("0x1d4"), "{text}");
    assert_eq!(out["data"]["pseudocode_repairs"]["register_relative_addresses_named"], 1, "{}", out["data"]["pseudocode_repairs"]);
}

/// The same poll, but the volatile range comes from a watch run: the VBlank
/// handler's decrement is an interrupt-context write, so evidence marks the
/// counter volatile. The handler's own push/pop is a stack frame, not a global.
#[test]
fn live_ghidra_volatile_from_interrupt_writes() {
    let dir = workspace();
    let root = dir.path();
    if call("ghidra.doctor", json!({}), root)["error"]["code"] == "ghidra_unavailable" {
        eprintln!("skipping: Ghidra not installed");
        return;
    }
    let mut b = vec![0u8; 0x400];
    b[0..4].copy_from_slice(&0x00FF_1000u32.to_be_bytes());
    b[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    b[0x78..0x7C].copy_from_slice(&0x250u32.to_be_bytes());
    b[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    let code: &[(usize, &[u8])] = &[
        // move.w #$8164,($C00004).l (VBlank on); move #$2000,sr; jsr $220; bra.s *
        (0x200, &[0x33, 0xFC, 0x81, 0x64, 0, 0xC0, 0, 4, 0x46, 0xFC, 0x20, 0, 0x4E, 0xB9, 0, 0, 0x02, 0x20, 0x60, 0xFE]),
        // move.l #5,($FFE4EE).l; loop: tst.l ($FFE4EE).l; bne.s loop; rts
        (0x220, &[0x23, 0xFC, 0, 0, 0, 5, 0, 0xFF, 0xE4, 0xEE, 0x4A, 0xB9, 0, 0xFF, 0xE4, 0xEE, 0x66, 0xF8, 0x4E, 0x75]),
        // move.l d0,-(sp); tst.l ($FFE4EE).l; beq.s +6; subq.l #1,($FFE4EE).l; move.l (sp)+,d0; rte
        (0x250, &[0x2F, 0x00, 0x4A, 0xB9, 0, 0xFF, 0xE4, 0xEE, 0x67, 6, 0x53, 0xB9, 0, 0xFF, 0xE4, 0xEE, 0x20, 0x1F, 0x4E, 0x73]),
    ];
    for (at, bytes) in code {
        b[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    fs::write(root.join("vbl.md"), b).unwrap();
    let watch = call("watch", json!({"rom":"vbl.md","frames":10,"watch":{"address":0,"length":0x10000}}), root);
    let records = watch["data"]["records_artifact"]["path"].as_str().unwrap().to_string();
    let writes = watch["data"]["write_preview"].as_array().unwrap();
    assert!(writes.iter().any(|w| w["pc"] == 0x25A && w["in_interrupt"] == true), "{writes:?}");
    assert!(writes.iter().any(|w| w["pc"] == 0x220 && w["in_interrupt"] == false), "{writes:?}");

    fs::write(root.join("vbl.sym"), "220 Delay\nFFE4EE delay_counter data 4\n").unwrap();
    let out = call("ghidra.decompile", json!({"rom":"vbl.md","addr":"220","evidence":[records],"symbols":["vbl.sym"]}), root);
    assert_eq!(out["ok"], true, "{out}");
    let text = out["data"]["pseudocode"].as_str().unwrap();
    assert!(text.contains("delay_counter = 5;") && text.contains("!= 0);") && !text.contains("while( true )"), "{text}");
    let evidence = &out["data"]["provenance"]["evidence"];
    let marked = evidence["volatile_from_evidence"].as_array().unwrap();
    assert_eq!(marked.len(), 1, "{evidence}");
    assert_eq!((marked[0]["address"].as_u64(), marked[0]["length"].as_u64(), marked[0]["writer_pc"].as_u64()), (Some(0xFF_E4EE), Some(4), Some(0x25A)));
    assert!(evidence["interrupt_stack_writes"].as_u64().unwrap() > 0, "{evidence}");
}
