//! Public operation contracts: independent authored inputs, bounded failure
//! paths, schemas, immutable artifacts, and real bus-write observations.
use mdx::{artifacts, context::Context, operations};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

#[test]
fn numeric_schemas_are_portable_and_keep_their_runtime_limits() {
    fn inspect(value: &Value) {
        match value {
            Value::Object(object) => {
                if let Some(format) = object.get("format").and_then(Value::as_str) {
                    assert!(
                        !format.starts_with("uint")
                            && !format.starts_with("int")
                            && format != "float"
                            && format != "double",
                        "nonportable numeric format {format}"
                    );
                }
                for child in object.values() {
                    inspect(child);
                }
            }
            Value::Array(values) => {
                for child in values {
                    inspect(child);
                }
            }
            _ => {}
        }
    }
    for descriptor in operations::descriptors() {
        inspect(&descriptor.input_schema);
        inspect(&descriptor.output_schema);
    }
    let boot = operations::descriptors()
        .into_iter()
        .find(|d| d.name == "boot")
        .unwrap();
    assert_eq!(
        boot.input_schema["properties"]["instructions"]["maximum"].as_f64(),
        Some(100000000.0)
    );
    let memory = operations::descriptors()
        .into_iter()
        .find(|d| d.name == "memory.read")
        .unwrap();
    assert_eq!(
        memory.input_schema["properties"]["address"]["maximum"],
        u32::MAX
    );
    let validator = jsonschema::validator_for(&memory.input_schema).unwrap();
    assert!(!validator
        .is_valid(&json!({"snapshot":"snapshot","memory":"wram","address":4294967296u64})));
}

#[test]
fn late_trace_gates_are_bounded_and_stop_idle_is_not_an_instruction() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let trace = call(
        "trace",
        json!({"rom":path,"instructions":12,"start_step":8,"start_pc":0x200,"max_records":4}),
        &context,
    );
    assert_eq!(trace["data"]["observed_records"], 4);
    assert_eq!(trace["data"]["trace_preview"][0]["step"], 8);
    let stopped = root.path().join("stop.md");
    rom(&stopped, &[0x4E, 0x72, 0x27, 0x00]);
    let trace = call(
        "trace",
        json!({"rom":stopped,"instructions":8,"max_records":16}),
        &context,
    );
    assert_eq!(trace["data"]["steps"], 8);
    assert_eq!(trace["data"]["observed_records"], 1);
}

#[test]
fn explicit_snapshot_ids_survive_capture_directory_removal() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let out = root.path().join("captures");
    let boot = call(
        "boot",
        json!({"rom":path,"instructions":8,"trace":0,"out":out}),
        &context,
    );
    let snapshot = boot["data"]["snapshot_artifact"]["id"].clone();
    fs::remove_dir_all(out).unwrap();
    assert_eq!(
        call(
            "memory.read",
            json!({"snapshot":snapshot,"memory":"wram","length":4}),
            &context
        )["ok"],
        true
    );
    assert_eq!(
        call(
            "render",
            json!({"snapshot":snapshot,"out":root.path().join("render")}),
            &context
        )["ok"],
        true
    );
    let snapshot_path = artifacts::resolve(&context.store, snapshot.as_str().unwrap()).unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(snapshot_path).unwrap()).unwrap();
    value["data"]["vdp"]["registers"][1] = json!(0x44);
    let mode5 = artifacts::put(&context.store, &serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        call(
            "graphics",
            json!({"snapshot":mode5.id,"out":root.path().join("graphics")}),
            &context
        )["ok"],
        true
    );
}

#[test]
fn unhandled_bus_samples_report_direction_and_caps_without_guessing_pc() {
    use mdcore::{Bus, Cartridge, System};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let mut system = System::new(Cartridge::load(path).unwrap());
    system.observation_limit = 2;
    system.read8(0x400000);
    system.write16(0x400002, 0x1234);
    system.read16(0x400004);
    assert_eq!(system.unmapped_accesses, 3);
    assert_eq!(system.unmapped_samples.len(), 2);
    assert!(!system.unmapped_samples[0].write);
    assert!(system.unmapped_samples[1].write);
    assert_eq!(system.unmapped_samples[1].width_bits, 16);
    assert_eq!(system.unmapped_samples[0].pc, None);
}

fn rom(path: &Path, program: &[u8]) {
    let mut bytes = vec![0u8; 1024];
    bytes[..4].copy_from_slice(&0x00FF1000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    bytes[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    bytes[0x200..0x200 + program.len()].copy_from_slice(program);
    fs::write(path, bytes).unwrap();
}

fn context(root: &Path) -> Context {
    Context {
        store: root.join("store"),
        ..Context::default()
    }
}

fn complete(value: Value) -> Value {
    if value["data"]["result_artifact"].is_object() {
        let artifact = serde_json::from_value(value["data"]["result_artifact"].clone()).unwrap();
        serde_json::from_slice(&artifacts::verified(&artifact).unwrap()).unwrap()
    } else {
        value
    }
}

fn call(name: &str, args: Value, context: &Context) -> Value {
    let response = serde_json::to_value(operations::execute(name, args, context)).unwrap();
    let descriptor = operations::descriptors()
        .into_iter()
        .find(|d| d.name == name)
        .unwrap();
    let validator = jsonschema::validator_for(&descriptor.output_schema).unwrap();
    assert!(
        validator.is_valid(&response),
        "schema mismatch for {name}: {:?}",
        validator
            .iter_errors(&response)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
    complete(response)
}

#[test]
fn discovery_is_side_effect_free_and_all_cli_errors_are_single_json_documents() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let catalog = call("schema", json!({}), &context);
    assert!(
        catalog["data"]["available_operations"]
            .as_array()
            .unwrap()
            .len()
            >= 16
    );
    assert_eq!(catalog["data"]["next_offset"], 4);
    assert!(
        !context.store.exists(),
        "schema discovery touched the artifact store"
    );
    for arguments in [
        vec!["disasm", "--rom", "missing", "--from", "not-an-address"],
        vec!["unknown-command"],
        vec!["header"],
        vec!["call", "header", "--args", "{broken"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mdx"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["ok"], false);
        assert_eq!(report["exit"], "usage");
    }
}

#[test]
fn budgets_count_exceptions_and_cancellation_timeout_and_missing_stops_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("illegal.md");
    rom(&path, &[0x4A, 0xFC, 0x60, 0xFC]);
    let context = context(root.path());
    let report = call("ram.dump", json!({"rom":path,"instructions":64}), &context);
    assert_eq!(report["data"]["instructions"], 64);
    assert!(report["data"]["exceptions"].as_u64().unwrap() > 0);
    let stopped = call(
        "ram.dump",
        json!({"rom":path,"instructions":4,"stop_at":0x800}),
        &context,
    );
    assert_eq!(stopped["exit"], "partial");
    assert_eq!(stopped["data"]["termination"], "instruction_budget");
    let bad = call(
        "boot",
        json!({"rom":path,"force_branch":["0x200=maybe"]}),
        &context,
    );
    assert_eq!(bad["exit"], "usage");
    let huge = call(
        "boot",
        json!({"rom":path,"instructions":100000001}),
        &context,
    );
    assert_eq!(huge["error"]["code"], "limit_exceeded");
    let timeout = call("boot", json!({"rom":path,"timeout_ms":0}), &context);
    assert_eq!(timeout["error"]["code"], "timeout");
    let cancelled = context.clone();
    cancelled.cancellation.cancel();
    let result = call("header", json!({"rom":path}), &cancelled);
    assert_eq!(result["error"]["code"], "cancelled");
}

#[test]
fn files_are_immutable_reusable_and_concurrent_calls_do_not_collide() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let context = context(root.path());
    let imported = call("artifact.import", json!({"path":path}), &context);
    let source = imported["data"]["artifact"]["id"].as_str().unwrap();
    let header = call("header", json!({"rom":source}), &context);
    assert_eq!(header["ok"], true);
    let mut workers = Vec::new();
    for steps in [16u64, 16, 24, 24] {
        let context = context.clone();
        let source = source.to_string();
        let out = root.path().join("shared-output");
        workers.push(std::thread::spawn(move || {
            call(
                "boot",
                json!({"rom":source,"instructions":steps,"trace":0,"out":out}),
                &context,
            )
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results[0], results[1]);
    assert_eq!(results[2], results[3]);
    assert_ne!(
        results[0]["data"]["snapshot_artifact"]["id"],
        results[2]["data"]["snapshot_artifact"]["id"]
    );
    for result in &results {
        assert_eq!(result["ok"], true);
        for artifact in result["data"]["memory_artifacts"]
            .as_object()
            .unwrap()
            .values()
        {
            artifacts::verified(&serde_json::from_value(artifact.clone()).unwrap()).unwrap();
            let read = call(
                "artifact.read",
                json!({"source":artifact["id"],"length":4}),
                &context,
            );
            assert_eq!(read["data"]["length"], 4);
        }
    }
    let id = results[0]["data"]["snapshot_artifact"]["id"]
        .as_str()
        .unwrap();
    let rendered = call(
        "render",
        json!({"snapshot":id,"out":root.path().join("render")}),
        &context,
    );
    assert_eq!(rendered["ok"], true);
    let stored = artifacts::resolve(&context.store, source).unwrap();
    fs::write(stored, b"tampered").unwrap();
    let failed = call("header", json!({"rom":source}), &context);
    assert_eq!(failed["error"]["code"], "artifact_digest_mismatch");
}

#[test]
fn snapshot_memory_pages_search_and_diff_have_exact_counts_and_folded_aliases() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let mut bytes = vec![0u8; 65536];
    bytes[65534..].copy_from_slice(&[0xaa, 0xbb]);
    bytes[..2].copy_from_slice(&[0xcc, 0xdd]);
    let a = artifacts::put(&context.store, &bytes).unwrap();
    let snapshot_a = artifacts::put(
        &context.store,
        &serde_json::to_vec(&json!({"memory_artifacts":{"wram":a}})).unwrap(),
    )
    .unwrap();
    let read = call(
        "memory.read",
        json!({"snapshot":snapshot_a.id,"memory":"wram","address":0x00FAFFFEu32,"length":4}),
        &context,
    );
    assert_eq!(read["data"]["bytes"], "aabbccdd");
    assert_eq!(read["data"]["folded_offset"], 65534);
    let search = call(
        "memory.search",
        json!({"snapshot":snapshot_a.id,"memory":"wram","pattern":"00 00","limit":3}),
        &context,
    );
    assert_eq!(search["data"]["matches_total"], 65531);
    assert_eq!(search["data"]["next_offset"], 3);
    assert_eq!(search["data"]["matches"], json!([2, 3, 4]));
    bytes[10] = 7;
    bytes[11] = 8;
    let b = artifacts::put(&context.store, &bytes).unwrap();
    let snapshot_b = artifacts::put(
        &context.store,
        &serde_json::to_vec(&json!({"memory_artifacts":{"wram":b}})).unwrap(),
    )
    .unwrap();
    let diff = call(
        "memory.diff",
        json!({"left":snapshot_a.id,"right":snapshot_b.id,"memory":"wram"}),
        &context,
    );
    assert_eq!(diff["data"]["changed_bytes"], 2);
    assert_eq!(diff["data"]["ranges_total"], 1);
    assert_eq!(
        diff["data"]["ranges"][0],
        json!({"offset":10,"before":"0000","after":"0708"})
    );
}

#[test]
fn bus_watch_counts_all_widths_same_value_writes_and_ram_aliases_once() {
    use mdcore::{observe::WramWatch, Bus, Cartridge, System};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let mut system = System::new(Cartridge::load(path).unwrap());
    system.wram_watch = WramWatch::new(0x6100, 4, 3, None);
    system.write8(0xFFFF6100, 0x11);
    system.write16(0x00FA6100, 0x2233);
    system.write32(0x00F06100, 0x44556677);
    system.write32(0x00FF6100, 0x44556677);
    let watch = system.wram_watch.unwrap();
    assert_eq!(watch.observed, 4);
    assert_eq!(watch.records.len(), 3);
    assert_eq!(
        watch
            .records
            .iter()
            .map(|r| r.width_bits)
            .collect::<Vec<_>>(),
        [8, 16, 32]
    );
    assert_eq!(watch.records[2].before, 0x22330000);
    assert_eq!(watch.records[2].value, 0x44556677);
    assert!(watch.records.iter().all(|r| r.folded_offset == 0x6100));
}

#[test]
fn probes_preserve_instruction_boundary_state_and_report_every_record_cap() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("write.md");
    // Authored MOVE.W #$1234,$FFE54A, then a self-loop.
    rom(
        &path,
        &[0x33, 0xFC, 0x12, 0x34, 0x00, 0xFF, 0xE5, 0x4A, 0x60, 0xFE],
    );
    let context = context(root.path());
    let watch = call(
        "watch",
        json!({"rom":path,"instructions":8,"watch":{"address":0xE54A,"length":2}}),
        &context,
    );
    assert_eq!(watch["data"]["observed_records"], 1);
    let record = &watch["data"]["write_preview"][0];
    assert_eq!(record["pc"], 0x200);
    assert_eq!(record["value"], 0x1234);
    assert_eq!(record["width_bits"], 16);
    assert_eq!(record["writer"], "m68000");
    assert_eq!(record["a"][7], 0xFF1000);
    let trace = call(
        "trace",
        json!({"rom":path,"instructions":8,"max_records":2}),
        &context,
    );
    assert_eq!(trace["data"]["observed_records"], 8);
    assert_eq!(trace["data"]["retained_records"], 2);
    assert_eq!(trace["data"]["omitted_records"], 6);
    assert_eq!(trace["exit"], "partial");
    assert_eq!(trace["data"]["trace_preview"][0]["pc"], 0x200);
    assert_eq!(trace["data"]["trace_preview"][0]["next_pc"], 0x208);
    let artifact = serde_json::from_value(trace["data"]["records_artifact"].clone()).unwrap();
    assert_eq!(
        artifacts::verified(&artifact)
            .unwrap()
            .iter()
            .filter(|b| **b == b'\n')
            .count(),
        2
    );
}

#[test]
fn trace_coverage_counts_every_distinct_pc_and_transfer_of_the_whole_run() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("loop.md");
    // lea $312,a5; loop: jsr $220; bra.s loop ... $220: rts
    let mut program = vec![0x4B, 0xF9, 0, 0, 0x03, 0x12, 0x4E, 0xB9, 0, 0, 0x02, 0x20, 0x60, 0xF8];
    program.resize(0x20, 0);
    program.extend([0x4E, 0x75]);
    rom(&path, &program);
    let context = context(root.path());
    let trace = call(
        "trace",
        json!({"rom":path,"instructions":1000,"max_records":0,"coverage":true}),
        &context,
    );
    // max_records 0 with coverage declines per-step records without truncation.
    assert_eq!(trace["exit"], "ok", "{trace}");
    assert_eq!(trace["data"]["retained_records"], 0);
    let summary = &trace["data"]["coverage"];
    assert_eq!(summary["steps"], 1000);
    assert_eq!(summary["distinct_pcs"], 4);
    assert_eq!(summary["distinct_edges"], 3);
    assert_eq!(summary["hottest"][0]["hits"], 333);
    let artifact = serde_json::from_value(summary["coverage_artifact"].clone()).unwrap();
    let rows: Vec<Value> = String::from_utf8(artifacts::verified(&artifact).unwrap())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let pcs: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "pc").collect();
    assert_eq!(pcs.iter().map(|r| r["pc"].as_u64().unwrap()).collect::<Vec<_>>(), [0x200, 0x206, 0x20C, 0x220]);
    assert_eq!(pcs.iter().map(|r| r["hits"].as_u64().unwrap()).sum::<u64>(), 1000);
    assert_eq!((pcs[0]["first_step"].as_u64(), pcs[0]["hits"].as_u64()), (Some(0), Some(1)));
    let edges: Vec<(u64, u64)> = rows
        .iter()
        .filter(|r| r["kind"] == "edge")
        .map(|r| (r["pc"].as_u64().unwrap(), r["next_pc"].as_u64().unwrap()))
        .collect();
    assert_eq!(edges, [(0x206, 0x220), (0x20C, 0x206), (0x220, 0x20C)]);
    let registers = rows.last().unwrap();
    assert_eq!(registers["kind"], "registers");
    assert_eq!(registers["a"][5][0], json!([0x312, 999]));
    // The artifact survives into the call's store like other trace outputs.
    assert!(context.store.join("objects").join(summary["coverage_artifact"]["sha256"].as_str().unwrap()).is_file());
    let watch = call(
        "watch",
        json!({"rom":path,"instructions":8,"coverage":true,"watch":{"address":0xE54A,"length":2}}),
        &context,
    );
    assert_eq!(watch["error"]["code"], "invalid_arguments");
}

#[test]
fn survey_reports_each_failure_and_malformed_conformance_is_not_a_pass() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let result = call(
        "survey",
        json!({"tasks":[{"id":"one","operation":"header","arguments":{"rom":"missing"}},{"id":"two","operation":"schema","arguments":{"command":"header"}}]}),
        &context,
    );
    assert_eq!(result["exit"], "partial");
    assert_eq!(result["data"]["results"].as_array().unwrap().len(), 2);
    assert_eq!(result["data"]["results"][1]["response"]["ok"], true);
    let suite = root.path().join("suite/v1");
    fs::create_dir_all(&suite).unwrap();
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(
        fs::File::create(suite.join("NOP.json.gz")).unwrap(),
        flate2::Compression::default(),
    );
    encoder.write_all(b"[{}]").unwrap();
    encoder.finish().unwrap();
    let report = call(
        "cpu.conformance",
        json!({"suite":root.path().join("suite")}),
        &context,
    );
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "fixture_invalid");
}

#[test]
fn large_results_are_complete_artifacts_with_structured_views_and_explicit_paging() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let response = serde_json::to_value(operations::execute(
        "boot",
        json!({"rom":path,"instructions":4096,"trace":4096}),
        &context,
    ))
    .unwrap();
    assert!(response["data"]["result_artifact"].is_object());
    let source = response["data"]["result_artifact"]["id"].clone();
    let view = call(
        "artifact.view",
        json!({"source":source,"pointer":"/data/first_addresses","limit":3}),
        &context,
    );
    assert_eq!(view["data"]["total_items"], 4096);
    assert_eq!(view["data"]["next_offset"], 3);
    assert_eq!(view["data"]["value"].as_array().unwrap().len(), 3);
    let missing = call(
        "artifact.view",
        json!({"source":source,"pointer":"/missing"}),
        &context,
    );
    assert_eq!(missing["error"]["code"], "json_pointer_not_found");
    let jsonl = artifacts::put(
        &context.store,
        b"{\"pc\":512}\n{\"pc\":514}\n{\"pc\":512}\n",
    )
    .unwrap();
    let records = call(
        "artifact.view",
        json!({"source":jsonl.id,"format":"jsonl","pointer":"/pc","offset":1,"limit":1}),
        &context,
    );
    assert_eq!(records["data"]["value"], json!([514]));
    assert_eq!(records["data"]["next_offset"], 2);
    // A JSON object's own path/digest fields are plain data, not instructions
    // to follow an unrelated filesystem reference during output finalization.
    let plain = artifacts::put(
        &context.store,
        br#"{"path":"missing","bytes":8,"sha256":"untrusted-data"}"#,
    )
    .unwrap();
    let view = call("artifact.view", json!({"source":plain.id}), &context);
    assert_eq!(view["ok"], true);
}
