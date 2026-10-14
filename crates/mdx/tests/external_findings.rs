//! Public-contract regressions from the external-agent review. Authored ROM
//! bytes exercise addresses/termination; pinned fixtures exercise ingestion.
use mdx::{artifacts, context::Context, operations};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path, process::Command};

fn rom(path: &Path, program: &[u8]) {
    let mut bytes = vec![0; 1024];
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

fn call(name: &str, args: Value, context: &Context) -> Value {
    let descriptor = operations::descriptors()
        .into_iter()
        .find(|d| d.name == name)
        .unwrap();
    assert!(jsonschema::validator_for(&descriptor.input_schema)
        .unwrap()
        .is_valid(&args));
    let response = serde_json::to_value(operations::execute(name, args, context)).unwrap();
    let validator = jsonschema::validator_for(&descriptor.output_schema).unwrap();
    assert!(
        validator.is_valid(&response),
        "{name}: {:?}",
        validator.iter_errors(&response).collect::<Vec<_>>()
    );
    if response["data"]["result_artifact"].is_object() {
        let artifact = serde_json::from_value(response["data"]["result_artifact"].clone()).unwrap();
        serde_json::from_slice(&artifacts::verified(&artifact).unwrap()).unwrap()
    } else {
        response
    }
}

#[test]
fn header_exposes_interrupt_and_trap_slots_with_68000_names() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vectors.md");
    rom(&path, &[0x60, 0xFE]);
    let mut bytes = fs::read(&path).unwrap();
    for index in 2..64 {
        bytes[index * 4..index * 4 + 4]
            .copy_from_slice(&(0x200u32 + index as u32 * 2).to_be_bytes());
    }
    bytes[0x78..0x7C].copy_from_slice(&0xDB4u32.to_be_bytes());
    fs::write(&path, bytes).unwrap();
    let report = call("header", json!({"rom":path}), &context(root.path()));
    assert_eq!(report["ok"], true);
    let vectors = report["data"]["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 64);
    for (index, name) in [
        (9, "trace"),
        (12, "reserved_12"),
        (13, "reserved_13"),
        (14, "reserved_14"),
        (15, "uninitialized_interrupt"),
        (16, "reserved"),
        (24, "spurious_interrupt"),
        (25, "level_1_autovector"),
        (28, "level_4_autovector"),
        (30, "level_6_autovector"),
        (31, "level_7_autovector"),
        (32, "trap_0"),
        (47, "trap_15"),
        (63, "reserved"),
    ] {
        assert_eq!(vectors[index]["index"], index);
        assert_eq!(vectors[index]["name"], name);
    }
    assert_eq!(vectors[30]["address"], "0x00000DB4");
    // A 1 KiB image is padded with FF to 64 KiB (reference md_cart_init), so
    // the vector points into padding rather than a 1 KiB mirror.
    assert_eq!(vectors[30]["cart_offset"], "0x000DB4");
}

#[test]
fn disassembly_keeps_bus_pcs_and_maps_offsets_across_mirrors_and_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mirror.md");
    rom(&path, &[0x61, 0x00, 0x00, 0x10, 0x4E, 0x71]);
    let context = context(root.path());
    let report = call(
        "disasm",
        json!({"rom":path,"from":0x20200,"count":2}),
        &context,
    );
    assert_eq!(report["ok"], true);
    assert_eq!(report["data"]["from"], "0x020200");
    assert_eq!(report["data"]["cart_offset"], "0x000200");
    let instructions = &report["data"]["instructions"];
    assert_eq!(instructions[0]["cart_offset"], "0x000200");
    assert_eq!(instructions[0]["bytes"], "61 00 00 10");
    assert!(instructions[0]["text"]
        .as_str()
        .unwrap()
        .ends_with("0x20212"));
    assert_eq!(instructions[1]["address"], "0x020204");
    assert_eq!(instructions[1]["cart_offset"], "0x000204");
    let mut bytes = fs::read(&path).unwrap();
    bytes[0x3FE..0x400].copy_from_slice(&[0x4E, 0x71]);
    bytes[..2].copy_from_slice(&[0x4E, 0x71]);
    fs::write(&path, &bytes).unwrap();
    let edge = call(
        "disasm",
        json!({"rom":path,"from":0x3FE,"count":2}),
        &context,
    );
    assert_eq!(edge["data"]["instructions"][1]["address"], "0x000400");
    // Past the 1 KiB image is FF padding up to the 64 KiB mirror period.
    assert_eq!(edge["data"]["instructions"][1]["cart_offset"], "0x000400");
    let edge = call(
        "disasm",
        json!({"rom":path,"from":0x3FFFFE,"count":2}),
        &context,
    );
    assert_eq!(edge["exit"], "partial");
    assert_eq!(edge["data"]["instructions"].as_array().unwrap().len(), 1);
    let bad = call("disasm", json!({"rom":path,"from":0xFF0200}), &context);
    assert_eq!(bad["error"]["code"], "invalid_arguments");
    bytes.resize(768, 0);
    fs::write(&path, bytes).unwrap();
    // A 768-byte image is padded with FF to the 64 KiB minimum period and
    // mirrors there; 0x300.. is unbacked tail, not another copy of the start.
    let tail = call(
        "disasm",
        json!({"rom":path,"from":0x300,"count":1}),
        &context,
    );
    assert_eq!(tail["exit"], "partial");
    assert_eq!(tail["data"]["instructions"][0]["bytes"], "FF FF");
    assert_eq!(tail["data"]["cart_offset"], "0x000300");
    let non_power_of_two = call(
        "disasm",
        json!({"rom":path,"from":0x10000,"count":1}),
        &context,
    );
    assert_eq!(non_power_of_two["exit"], "ok");
    assert_eq!(
        non_power_of_two["data"]["instructions"][0]["bytes"],
        "4E 71"
    );
    assert_eq!(non_power_of_two["data"]["cart_offset"], "0x000000");
}

#[test]
fn boot_treats_alternative_targets_as_success_and_names_missing_targets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("stop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let context = context(root.path());
    for (budget, stop, executed) in [(8, 0x200, 0), (0, 0x200, 0), (1, 0x202, 1)] {
        let report = call(
            "boot",
            json!({"rom":path,"instructions":budget,"frames":60,"stop_at":[stop]}),
            &context,
        );
        assert_eq!(report["ok"], true);
        assert_eq!(report["exit"], "ok");
        assert_eq!(report["data"]["termination"], "stop_point");
        assert_eq!(report["data"]["instructions"], executed);
    }
    let frames = call(
        "boot",
        json!({"rom":path,"instructions":0,"frames":0,"stop_at":[0x300]}),
        &context,
    );
    assert_eq!(frames["ok"], true);
    assert_eq!(frames["data"]["termination"], "frame_target");
    for args in [
        json!({"rom":path,"instructions":2,"stop_at":[0x300]}),
        json!({"rom":path,"instructions":2,"frames":1}),
    ] {
        let report = call("boot", args, &context);
        assert_eq!(report["exit"], "partial");
        assert_eq!(report["data"]["termination"], "instruction_budget");
        let limitation = report["coverage"]["limitations"][0].as_str().unwrap();
        assert!(limitation.contains("requested"));
        assert!(limitation.contains("not reached"));
    }
}

#[test]
fn ram_ranges_preserve_numeric_and_text_aliases_and_wrap_row_labels() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ram.md");
    // MOVE.L #$12345678,$FFE21C
    rom(
        &path,
        &[
            0x23, 0xFC, 0x12, 0x34, 0x56, 0x78, 0x00, 0xFF, 0xE2, 0x1C, 0x60, 0xFE,
        ],
    );
    let context = context(root.path());
    for address in [
        json!(0xE21C),
        json!(0xFFE21C),
        json!(0xE0E21C),
        json!(0xFFFFE21Cu32),
        json!("0xFFE21C"),
        json!("0XFFFFE21C"),
        json!("16769564"),
    ] {
        let report = call(
            "ram.dump",
            json!({"rom":path,"instructions":1,"range":[address,"0x4"]}),
            &context,
        );
        assert_eq!(report["ok"], true);
        assert_eq!(report["data"]["hex"][0]["address"], "0x00FFE21C");
        assert_eq!(report["data"]["hex"][0]["wram_offset"], "0xE21C");
        assert_eq!(report["data"]["hex"][0]["bytes"], "12 34 56 78");
    }
    let folded = call(
        "ram.dump",
        json!({"rom":path,"instructions":0,"range":["0x0000E01C",4]}),
        &context,
    );
    assert_eq!(folded["data"]["hex"][0]["wram_offset"], "0xE01C");
    let wrap = call(
        "ram.dump",
        json!({"rom":path,"instructions":0,"range":["0xFFFF",32]}),
        &context,
    );
    assert_eq!(wrap["data"]["hex"][1]["wram_offset"], "0x000F");
    assert_eq!(wrap["data"]["hex"][1]["address"], "0x00FF000F");
    for address in [
        json!(0x400000),
        json!(0x2E01C),
        json!("0x100000000"),
        json!("E21C"),
    ] {
        let bad = call(
            "ram.dump",
            json!({"rom":path,"instructions":0,"range":[address,4]}),
            &context,
        );
        assert_eq!(bad["error"]["code"], "invalid_arguments");
    }
    for alias in ["0xFFE21C", "0XFFE21C"] {
        let output = Command::new(env!("CARGO_BIN_EXE_mdx"))
            .args(["ram", "dump", "--rom"])
            .arg(&path)
            .args(["--instructions", "1", "--range", &format!("{alias}:4")])
            .output()
            .unwrap();
        assert!(output.status.success());
        let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(cli["data"]["hex"][0]["bytes"], "12 34 56 78");
    }
}

#[test]
fn replay_file_schema_and_note_are_discoverable_without_loose_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let replay = json!({"note":"press A at reset", "events":[{"frame":0,"buttons":["a"]}]});
    for name in ["boot", "trace", "watch"] {
        let schema = call("schema", json!({"command":name}), &context);
        let validator = jsonschema::validator_for(
            &schema["data"]["operations"][0]["file_schemas"]["controller_replay"],
        )
        .unwrap();
        assert!(validator.is_valid(&replay));
        assert!(!validator.is_valid(&json!({"_comment":"typo", "events":[]})));
        assert!(!validator.is_valid(&json!({"events":[{"frame":0,"port":4,"buttons":[]}]})));
        assert!(validator.is_valid(&json!({"events":[{"frame":0,"port":3,"buttons":["a"]}]})));
    }
    assert!(!context.store.exists());
    let input = root.path().join("input.json");
    fs::write(&input, serde_json::to_vec(&replay).unwrap()).unwrap();
    let path = root.path().join("loop.md");
    rom(&path, &[0x4E, 0x71, 0x60, 0xFC]);
    let report = call(
        "boot",
        json!({"rom":path,"instructions":1,"input":input}),
        &context,
    );
    assert_eq!(report["data"]["input_replay"]["events_applied"], 1);
    assert_eq!(report["data"]["input_replay"]["final_pressed_masks"][0], 16);
    assert_eq!(
        report["data"]["input_replay"]["source_sha256"],
        artifacts::digest(&fs::read(&input).unwrap())
    );
    fs::write(&input, r#"{"_comment":"typo","events":[]}"#).unwrap();
    let bad = call(
        "boot",
        json!({"rom":path,"instructions":1,"input":input}),
        &context,
    );
    assert_eq!(bad["error"]["code"], "input_replay_invalid");
}

#[test]
fn start_pc_latches_instead_of_filtering_subsequent_trace_pcs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("gate.md");
    rom(&path, &[0x4E, 0x71, 0x4E, 0x71, 0x60, 0xFC]);
    let report = call(
        "trace",
        json!({"rom":path,"instructions":3,"start_pc":0x202}),
        &context(root.path()),
    );
    assert_eq!(report["data"]["observed_records"], 2);
    assert_eq!(report["data"]["trace_preview"][0]["pc"], 0x202);
    assert_eq!(report["data"]["trace_preview"][1]["pc"], 0x204);
}

#[test]
fn bundled_conformance_matches_gzip_suite_and_rejects_bad_provenance() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/cpu-regressions.json");
    let bundle: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    let directory = root.path().join("68000");
    fs::create_dir_all(directory.join("v1")).unwrap();
    for source in bundle["sources"].as_array().unwrap() {
        let cases: Vec<_> = bundle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["file"] == source["file"])
            .map(|entry| entry["case"].clone())
            .collect();
        let file =
            fs::File::create(directory.join("v1").join(source["file"].as_str().unwrap())).unwrap();
        let mut gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        gzip.write_all(&serde_json::to_vec(&cases).unwrap())
            .unwrap();
        gzip.finish().unwrap();
    }
    let bundled = call(
        "cpu.conformance",
        json!({"suite":source,"compare_cycles":true}),
        &context,
    );
    assert_eq!(bundled["ok"], true);
    assert_eq!(bundled["data"]["cases_passed"], 12);
    assert_eq!(
        bundled["data"]["source_files"][0]["file_sha256"],
        artifacts::digest(&fs::read(&source).unwrap())
    );
    assert_eq!(bundled["data"]["bundled_sources"], bundle["sources"]);
    let upstream = call(
        "cpu.conformance",
        json!({"suite":directory,"compare_cycles":true}),
        &context,
    );
    for field in [
        "cases_run",
        "cases_passed",
        "cases_failed",
        "files_run",
        "files_total",
        "per_instruction",
        "failures",
    ] {
        assert_eq!(bundled["data"][field], upstream["data"][field], "{field}");
    }
    let filtered = call(
        "cpu.conformance",
        json!({"suite":source,"filter":"DBcc"}),
        &context,
    );
    assert_eq!(filtered["data"]["cases_passed"], 3);
    assert_eq!(filtered["data"]["files_run"], 1);
    assert_eq!(filtered["exit"], "partial");
    assert_eq!(
        call(
            "cpu.conformance",
            json!({"suite":source,"filter":"BRA"}),
            &context
        )["error"]["code"],
        "filter_matched_nothing"
    );
    let bad_path = root.path().join("invalid.json");
    let mut bad = bundle.clone();
    bad["cases"].as_array_mut().unwrap().pop();
    fs::write(&bad_path, serde_json::to_vec(&bad).unwrap()).unwrap();
    assert_eq!(
        call("cpu.conformance", json!({"suite":bad_path}), &context)["error"]["code"],
        "fixture_invalid"
    );
    let mut bad = bundle.clone();
    bad["cases"][0]["case"]["final"]["d0"] = json!(0);
    fs::write(&bad_path, serde_json::to_vec(&bad).unwrap()).unwrap();
    let failed = call(
        "cpu.conformance",
        json!({"suite":bad_path,"failures":0}),
        &context,
    );
    assert_eq!(failed["exit"], "partial");
    assert_eq!(failed["data"]["cases_failed"], 1);
    assert_eq!(failed["data"]["failure_details_omitted"], 1);
}

#[test]
fn frame_pokes_write_work_and_sound_ram_and_report_pending_and_invalid_pokes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("idle.md");
    rom(&path, &[0x60, 0xFE]);
    let context = context(root.path());
    let pokes = json!([
        {"frame":2,"space":"wram","address":0xFF1234,"bytes":[0xAA,0xBB]},
        {"frame":3,"space":"z80","address":0xA00100,"bytes":[0x55]},
        {"frame":10,"space":"wram","address":0,"bytes":[1]}
    ]);
    let report = call(
        "boot",
        json!({"rom":path,"frames":5,"instructions":2000000,"pokes":pokes}),
        &context,
    );
    assert_eq!(report["exit"], "ok");
    assert_eq!(report["data"]["input_replay"]["pokes_applied"], 2);
    assert_eq!(report["data"]["input_replay"]["pokes_pending"], 1);
    let memory = |name: &str| {
        let artifact = &report["data"]["memory_artifacts"][name];
        fs::read(artifact["path"].as_str().unwrap()).unwrap()
    };
    assert_eq!(memory("wram")[0x1234..0x1236], [0xAA, 0xBB]);
    assert_eq!(memory("z80_ram")[0x100], 0x55);
    let crossing = call(
        "boot",
        json!({"rom":path,"frames":1,"pokes":[{"frame":0,"space":"z80","address":0x1FFF,"bytes":[1,2]}]}),
        &context,
    );
    assert_eq!(crossing["error"]["code"], "input_replay_invalid");
}

#[test]
fn sound_command_log_separates_uploads_from_command_slots_and_counts_bus_control() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sound.md");
    let words: [u16; 33] = [
        0x13FC, 0x0011, 0x00A0, 0x1C0B, // Z80 still in reset: dropped write
        0x33FC, 0x0100, 0x00A1, 0x1200, // move.w #$100,$A11200 (release reset)
        0x33FC, 0x0100, 0x00A1, 0x1100, // move.w #$100,$A11100 (BUSREQ)
        0x41F9, 0x00A0, 0x0000, 0x7027, // lea $A00000,a0; moveq #39,d0
        0x10C0, 0x51C8, 0xFFFC, // upload: move.b d0,(a0)+; dbf d0
        0x223C, 0x0002, 0x0000, 0x5381, 0x66FC, // delay several frames
        0x13FC, 0x0042, 0x00A0, 0x1C0A, // move.b #$42,$A01C0A (command)
        0x33FC, 0x0000, 0x00A1, 0x1100, // release BUSREQ
        0x60FE,
    ];
    let program: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    rom(&path, &program);
    let context = context(root.path());
    let report = call(
        "boot",
        json!({"rom":path,"frames":30,"instructions":3000000,"sound_commands":{}}),
        &context,
    );
    let log = &report["data"]["sound_commands"];
    assert_eq!(log["observed"], 42);
    assert_eq!(log["bus_control_counted"], 3);
    assert_eq!(log["uploads"].as_array().unwrap().len(), 1);
    // Frame 0 holds the dropped write plus the 40-byte upload.
    assert_eq!(log["uploads"][0]["writes"], 41);
    let slot = &log["busiest_z80_addresses"][0];
    assert_eq!(slot["z80_address"], "0x1C0A");
    assert_eq!(slot["values"], json!(["0x42"]));
    assert_eq!(slot["writer_pcs"], json!(["0x000230"]));
    assert_eq!(log["busiest_z80_addresses"].as_array().unwrap().len(), 1);
    let artifact = log["records_artifact"]["path"].as_str().unwrap();
    let last: Value = serde_json::from_str(fs::read_to_string(artifact).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!((last["target"].as_str(), last["value"].as_u64()), (Some("z80_ram"), Some(0x42)));
    assert_eq!(last["z80_bus_granted"], true);
    let with_bus = call(
        "boot",
        json!({"rom":path,"frames":30,"instructions":3000000,"sound_commands":{"bus_control":true,"z80_start":0x1C00,"z80_length":0x20}}),
        &context,
    );
    assert_eq!(
        with_bus["data"]["sound_commands"]["by_target"],
        json!({"bus_request":2,"reset":1,"z80_ram":2})
    );
    let records = fs::read_to_string(
        with_bus["data"]["sound_commands"]["records_artifact"]["path"].as_str().unwrap(),
    )
    .unwrap();
    let first: Value = serde_json::from_str(records.lines().next().unwrap()).unwrap();
    assert_eq!((first["z80_address"].as_u64(), first["z80_bus_granted"].as_bool()), (Some(0x1C0B), Some(false)));
}

#[test]
fn z80_disassembly_reads_rom_with_origin_and_state_z80_ram() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("z80.md");
    // 68000: release Z80 reset, take the bus, store 5 bytes into Z80 RAM
    // (byte writes: word writes only store their high byte), then idle.
    let mut words = vec![0x33FC, 0x0100, 0x00A1, 0x1200, 0x33FC, 0x0100, 0x00A1, 0x1100];
    for (offset, byte) in [0xF3u16, 0xED, 0x56, 0x18, 0xFE].into_iter().enumerate() {
        words.extend([0x13FC, byte, 0x00A0, offset as u16]); // move.b #byte,$A0000n
    }
    let mut program: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    program.extend([0x60, 0xFE]);
    rom(&path, &program);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0x300..0x305].copy_from_slice(&[0xF3, 0xED, 0x56, 0x18, 0xFE]);
    fs::write(&path, &bytes).unwrap();
    let context = context(root.path());
    let listing = |report: &Value| -> Vec<String> {
        report["data"]["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| format!("{} {}", i["address"].as_str().unwrap(), i["text"].as_str().unwrap()))
            .collect()
    };
    let from_rom = call("disasm", json!({"rom":path,"cpu":"z80","from":0x300,"origin":0x1000,"count":3}), &context);
    assert_eq!(from_rom["exit"], "ok");
    assert_eq!(listing(&from_rom), ["0x1000 di", "0x1001 im 1", "0x1003 jr 0x1003"]);
    assert_eq!(from_rom["data"]["instructions"][1]["cart_offset"], "0x000301");
    let boot = call("boot", json!({"rom":path,"frames":2,"save_state":true}), &context);
    let state = boot["data"]["state_artifact"]["id"].clone();
    let from_ram = call("disasm", json!({"rom":path,"cpu":"z80","from_state":state,"from":0,"count":3}), &context);
    assert_eq!(listing(&from_ram), ["0x0000 di", "0x0001 im 1", "0x0003 jr 0x0003"]);
    for bad in [
        json!({"rom":path,"from":0x300,"origin":0x10}),
        json!({"rom":path,"cpu":"z80","from_state":state,"from":0x2000}),
    ] {
        assert_eq!(call("disasm", bad, &context)["error"]["code"], "invalid_arguments");
    }
}

#[test]
fn z80_trace_and_z80_ram_watch_report_both_cpus() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("z80run.md");
    // Z80: ld a,0x42 ; ld (0x1000),a ; jr $   uploaded by 68000 byte writes.
    // Take the bus before releasing reset so the Z80 starts at 0 afterwards.
    let z80 = [0x3E, 0x42, 0x32, 0x00, 0x10, 0x18, 0xFE];
    let mut words = vec![0x33FC, 0x0100, 0x00A1, 0x1100, 0x33FC, 0x0100, 0x00A1, 0x1200];
    for (offset, byte) in z80.into_iter().enumerate() {
        words.extend([0x13FC, byte as u16, 0x00A0, offset as u16]);
    }
    words.extend([0x13FC, 0x0011, 0x00A0, 0x1000]); // 68000 primes 0x1000 first
    words.extend([0x33FC, 0x0000, 0x00A1, 0x1100, 0x60FE]); // release the bus, idle
    let program: Vec<u8> = words.iter().flat_map(|w: &u16| w.to_be_bytes()).collect();
    rom(&path, &program);
    let context = context(root.path());
    let watch = call(
        "watch",
        json!({"rom":path,"frames":3,"instructions":2000000,"watch":{"space":"z80","address":0xA01000,"length":1}}),
        &context,
    );
    assert_eq!(watch["exit"], "ok", "{watch}");
    let writes = watch["data"]["z80_write_preview"].as_array().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!((writes[0]["writer"].as_str(), writes[0]["value"].as_u64()), (Some("m68000"), Some(0x11)));
    assert_eq!(writes[1]["writer"], "z80");
    assert_eq!((writes[1]["pc"].as_u64(), writes[1]["before"].as_u64(), writes[1]["value"].as_u64()), (Some(2), Some(0x11), Some(0x42)));
    let trace = call(
        "trace",
        json!({"rom":path,"frames":3,"instructions":2000000,"cpu":"z80","max_records":3}),
        &context,
    );
    let steps: Vec<String> = trace["data"]["z80_trace_preview"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| format!("{:04X} {}", t["pc"].as_u64().unwrap(), t["text"].as_str().unwrap()))
        .collect();
    assert_eq!(steps, ["0000 ld a,0x42", "0002 ld (0x1000),a", "0005 jr 0x0005"]);
    let ranged = call(
        "trace",
        json!({"rom":path,"frames":3,"instructions":2000000,"cpu":"z80","z80_pc_range":[2,5],"max_records":5}),
        &context,
    );
    assert_eq!(ranged["data"]["observed_records"], 1);
    let bad = call("watch", json!({"rom":path,"frames":3,"cpu":"z80","watch":{"address":0,"length":1}}), &context);
    assert_eq!(bad["error"]["code"], "invalid_arguments");
}

#[test]
fn console_region_follows_the_header_and_can_be_overridden_at_power_on() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("region.md");
    // move.b $A10001,$FF0000 ; bra $
    let program = [0x13, 0xF9, 0x00, 0xA1, 0x00, 0x01, 0x00, 0xFF, 0x00, 0x00, 0x60, 0xFE];
    rom(&path, &program);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0x1F0..0x1F3].copy_from_slice(b"J  ");
    fs::write(&path, &bytes).unwrap();
    let context = context(root.path());
    let version = |report: &Value| fs::read(report["data"]["memory_artifacts"]["wram"]["path"].as_str().unwrap()).unwrap()[0];
    let auto = call("boot", json!({"rom":path,"frames":1,"save_state":true}), &context);
    assert_eq!(auto["data"]["console_region"], "japan_ntsc");
    assert_eq!(version(&auto), 0x20);
    let usa = call("boot", json!({"rom":path,"frames":1,"region":"usa"}), &context);
    assert_eq!(usa["data"]["console_region"], "usa");
    assert_eq!(version(&usa), 0xA0);
    let europe = call("boot", json!({"rom":path,"frames":1,"region":"europe"}), &context);
    assert_eq!(version(&europe), 0xE0);
    let resumed = call(
        "boot",
        json!({"rom":path,"frames":1,"region":"usa","from_state":auto["data"]["state_artifact"]["id"]}),
        &context,
    );
    assert_eq!(resumed["error"]["code"], "invalid_arguments");
}

#[test]
fn boot_summary_profile_keeps_capture_essentials_and_drops_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("summary.md");
    rom(&path, &[0x60, 0xFE]);
    let context = context(root.path());
    let full = call("boot", json!({"rom":path,"frames":2,"save_state":true}), &context);
    let summary = call("boot", json!({"rom":path,"frames":2,"save_state":true,"profile":"summary"}), &context);
    for key in ["vdp", "final_registers", "first_addresses", "hottest_addresses", "observations", "input_replay"] {
        assert!(full["data"].get(key).is_some(), "full keeps {key}");
        assert!(summary["data"].get(key).is_none(), "summary drops {key}");
    }
    for key in ["termination", "reached_frames", "state_artifact", "memory_artifacts", "display", "exceptions"] {
        assert_eq!(summary["data"][key], full["data"][key], "summary keeps {key}");
    }
    assert!(serde_json::to_string(&summary).unwrap().len() < serde_json::to_string(&full).unwrap().len());
}

#[test]
fn image_compare_counts_changed_pixels_by_region_across_png_encodings() {
    let root = tempfile::tempdir().unwrap();
    let write = |name: &str, colour: png::ColorType, data: &[u8], palette: Option<Vec<u8>>| {
        let path = root.path().join(name);
        let file = fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, 4, 2);
        encoder.set_color(colour);
        encoder.set_depth(png::BitDepth::Eight);
        if let Some(palette) = palette {
            encoder.set_palette(palette);
        }
        encoder.write_header().unwrap().write_image_data(data).unwrap();
        path
    };
    // Left: indexed (all black). Right: RGB with pixels (1,0) and (3,1) changed.
    let left = write("left.png", png::ColorType::Indexed, &[0; 8], Some(vec![0, 0, 0]));
    let mut rgb = vec![0u8; 24];
    rgb[3..6].copy_from_slice(&[255, 0, 0]);
    rgb[21..24].copy_from_slice(&[0, 255, 0]);
    let right = write("right.png", png::ColorType::Rgb, &rgb, None);
    let context = context(root.path());
    let report = call(
        "image.compare",
        json!({"left":left,"right":right,"regions":[{"name":"top","x":0,"y":0,"width":4,"height":1},{"name":"left_half","x":0,"y":0,"width":2,"height":2}]}),
        &context,
    );
    assert_eq!(report["exit"], "ok", "{report}");
    assert_eq!(report["data"]["changed_pixels"], 2);
    assert_eq!(report["data"]["changed_bounds"], json!([1, 0, 3, 2]));
    assert_eq!(report["data"]["regions"][0]["changed_pixels"], 1);
    assert_eq!(report["data"]["regions"][1]["changed_pixels"], 1);
    let same = call("image.compare", json!({"left":left,"right":left}), &context);
    assert_eq!(same["data"]["changed_pixels"], 0);
    assert!(same["data"]["changed_bounds"].is_null());
    let outside = call("image.compare", json!({"left":left,"right":right,"regions":[{"name":"x","x":3,"y":0,"width":2,"height":1}]}), &context);
    assert_eq!(outside["error"]["code"], "invalid_arguments");
}
