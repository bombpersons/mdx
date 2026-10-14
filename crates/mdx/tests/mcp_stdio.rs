//! Independent JSON-RPC client exercises the shipped stdio server, rather than
//! calling its handlers directly. No SDK-specific client conveniences are used.
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    messages: mpsc::Receiver<Result<Value, String>>,
    pending: VecDeque<Value>,
    notifications: Vec<Value>,
    next_id: u64,
}

impl Rpc {
    fn start(root: &Path, jobs: usize) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mdx"))
            .args(["--root"])
            .arg(root)
            .args(["--store", "store", "mcp", "--jobs", &jobs.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let parsed = line.map_err(|e| e.to_string()).and_then(|text| {
                    serde_json::from_str(&text)
                        .map_err(|e| format!("non-protocol stdout: {text:?}: {e}"))
                });
                if sender.send(parsed).is_err() {
                    break;
                }
            }
        });
        let mut rpc = Self {
            child,
            input: Some(input),
            messages,
            pending: VecDeque::new(),
            notifications: Vec::new(),
            next_id: 1,
        };
        let initialized=rpc.request("initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"mdx-independent-test-client","version":"1"}}));
        assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
        rpc.send(json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}));
        rpc
    }
    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }
    fn begin(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        id
    }
    fn next(&mut self, timeout: Duration) -> Value {
        self.messages
            .recv_timeout(timeout)
            .expect("MCP message within deadline")
            .expect("valid JSON-RPC stdout")
    }
    fn wait(&mut self, id: u64) -> Value {
        if let Some(index) = self.pending.iter().position(|v| v["id"] == id) {
            return self.pending.remove(index).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let value = self.next(deadline.saturating_duration_since(Instant::now()));
            if value["id"] == id {
                return value;
            }
            if value.get("id").is_some() {
                self.pending.push_back(value);
            } else {
                self.notifications.push(value);
            }
        }
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.begin(method, params);
        self.wait(id)
    }
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let response = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert!(response.get("error").is_none(), "{response}");
        let result = &response["result"];
        let envelope = result["structuredContent"].clone();
        assert_eq!(
            serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
            envelope
        );
        envelope
    }
    fn close(mut self) {
        self.input.take();
        // RMCP drains pending responses for up to five seconds on EOF. The
        // assertions below separately verify that CPU work was cancelled.
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            match self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => panic!("{e}"),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("MCP process did not stop after stdio EOF")
                }
            }
        }
        assert!(self.child.wait().unwrap().success());
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn create_rom(path: &Path) {
    let mut bytes = vec![0u8; 1024];
    bytes[..4].copy_from_slice(&0xFF1000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    bytes[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
    bytes[0x200..0x20A]
        .copy_from_slice(&[0x33, 0xFC, 0x12, 0x34, 0x00, 0xFF, 0xE5, 0x4A, 0x60, 0xFE]);
    fs::write(path, bytes).unwrap();
}

#[test]
fn template_creation_uses_the_explicit_client_workspace_not_server_cwd() {
    let server_root = tempfile::tempdir().unwrap();
    let agent_workspace = tempfile::tempdir().unwrap();
    let directory = agent_workspace.path().join("native-game");
    let mut rpc = Rpc::start(server_root.path(), 2);
    let result = rpc.tool(
        "template_create",
        json!({"directory":directory,"name":"client-native"}),
    );
    assert!(result["ok"].as_bool().unwrap(), "{result}");
    assert_eq!(result["data"]["directory"], directory.to_str().unwrap());
    assert!(directory.join("src/dev_mcp.rs").is_file());
    assert!(!server_root.path().join("native-game").exists());
    let manifest = fs::read_to_string(directory.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"client-native\""));
    assert!(manifest.contains("version = \"=0.20.0\""));
    let collision = rpc.tool(
        "template_create",
        json!({"directory":directory,"name":"other"}),
    );
    assert_eq!(collision["error"]["code"], "template_collision");
    assert_eq!(
        fs::read_to_string(directory.join("Cargo.toml")).unwrap(),
        manifest
    );
    rpc.close();
}

fn cli(root: &Path, name: &str, args: &Value) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_mdx"))
        .current_dir(root)
        .args([
            "--store",
            "store",
            "call",
            name,
            "--args",
            &args.to_string(),
        ])
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn advertised_schemas_and_all_operations_have_cli_mcp_parity() {
    let root = tempfile::tempdir().unwrap();
    create_rom(&root.path().join("loop.md"));
    let mut rpc = Rpc::start(root.path(), 2);
    let listed = rpc.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), mdx::operations::descriptors().len());
    let mut validators = std::collections::BTreeMap::new();
    for tool in tools {
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        validators.insert(
            tool["name"].as_str().unwrap().to_owned(),
            jsonschema::validator_for(&tool["outputSchema"]).unwrap(),
        );
    }
    let compare = |rpc: &mut Rpc, name: &str, args: Value| {
        let mcp_name = name.replace('.', "_");
        let result = rpc.tool(&mcp_name, args.clone());
        assert!(
            validators[&mcp_name].is_valid(&result),
            "invalid advertised {name} result: {result}"
        );
        assert_eq!(
            result,
            cli(root.path(), name, &args),
            "CLI/MCP parity for {name}"
        );
        result
    };
    let imported = compare(&mut rpc, "artifact.import", json!({"path":"loop.md"}));
    let id = imported["data"]["artifact"]["id"].clone();
    compare(&mut rpc, "artifact.read", json!({"source":id,"length":16}));
    compare(&mut rpc, "header", json!({"rom":id}));
    compare(&mut rpc, "disasm", json!({"rom":id,"from":0x200,"count":2}));
    compare(&mut rpc, "schema", json!({"command":"boot"}));
    compare(
        &mut rpc,
        "ram.dump",
        json!({"rom":id,"instructions":8,"range":[0xE54A,2]}),
    );
    let alias = compare(
        &mut rpc,
        "ram.dump",
        json!({"rom":id,"instructions":8,"range":["0XFFFFE54A","0x2"]}),
    );
    assert_eq!(alias["data"]["hex"][0]["wram_offset"], "0xE54A");
    assert_eq!(alias["data"]["hex"][0]["bytes"], "12 34");
    let stopped = compare(
        &mut rpc,
        "boot",
        json!({"rom":id,"instructions":8,"frames":60,"stop_at":[0x200],"out":"stopped"}),
    );
    assert_eq!(stopped["ok"], true);
    assert_eq!(stopped["data"]["termination"], "stop_point");
    fs::write(
        root.path().join("bundle.json"),
        include_bytes!("../testdata/cpu-regressions.json"),
    )
    .unwrap();
    let bundled = compare(
        &mut rpc,
        "cpu.conformance",
        json!({"suite":"bundle.json","compare_cycles":true}),
    );
    assert_eq!(bundled["data"]["cases_passed"], 12);
    compare(&mut rpc, "audio", json!({"rom":id,"frames":2,"instructions":100000,"save_state":true,"out":"waves"}));
    let boot = compare(
        &mut rpc,
        "boot",
        json!({"rom":id,"instructions":8,"trace":0,"out":"capture"}),
    );
    let snapshot = boot["data"]["snapshot_artifact"]["id"].clone();
    compare(
        &mut rpc,
        "artifact.view",
        json!({"source":snapshot,"pointer":"/data/vdp/registers","limit":4}),
    );
    compare(
        &mut rpc,
        "memory.read",
        json!({"snapshot":snapshot,"memory":"wram","address":0xFAE54Au32,"length":2}),
    );
    compare(
        &mut rpc,
        "memory.search",
        json!({"snapshot":snapshot,"memory":"wram","pattern":"1234"}),
    );
    compare(
        &mut rpc,
        "memory.diff",
        json!({"left":snapshot,"right":snapshot,"memory":"wram"}),
    );
    compare(
        &mut rpc,
        "render",
        json!({"snapshot":snapshot,"out":"render"}),
    );
    let body: Value = serde_json::from_slice(
        &fs::read(
            mdx::artifacts::resolve(&root.path().join("store"), snapshot.as_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut mode5 = body;
    mode5["data"]["vdp"]["registers"][1] = json!(0x44);
    mode5["data"]["vdp"]["registers"][12] = json!(1);
    fs::write(
        root.path().join("mode5.json"),
        serde_json::to_vec(&mode5).unwrap(),
    )
    .unwrap();
    compare(
        &mut rpc,
        "graphics",
        json!({"snapshot":"mode5.json","out":"graphics"}),
    );
    let traced = compare(
        &mut rpc,
        "trace",
        json!({"rom":id,"instructions":8,"max_records":2}),
    );
    let first = &traced["data"]["trace_preview"][0];
    assert_eq!(first["master_cycles"], 784462);
    assert_eq!(first["elapsed_master_cycles"], 140);
    assert_eq!(traced["data"]["trace_preview"][1]["master_cycles"], 784602);
    compare(
        &mut rpc,
        "watch",
        json!({"rom":id,"instructions":8,"watch":{"address":0xE54A,"length":2}}),
    );
    compare(
        &mut rpc,
        "survey",
        json!({"tasks":[{"id":"header","operation":"header","arguments":{"rom":id}},{"id":"bad","operation":"header","arguments":{"rom":"missing"}}]}),
    );
    compare(&mut rpc, "header", json!({"rom":"missing"}));
    compare(&mut rpc, "header", json!({"rom":id,"unknown_field":true}));
    let suite = root.path().join("suite/v1");
    fs::create_dir_all(&suite).unwrap();
    let fixtures: Value =
        serde_json::from_str(include_str!("../testdata/cpu-regressions.json")).unwrap();
    let mut gz = flate2::write::GzEncoder::new(
        fs::File::create(suite.join("fixture.json.gz")).unwrap(),
        flate2::Compression::default(),
    );
    gz.write_all(&serde_json::to_vec(&json!([fixtures["cases"][0]["case"]])).unwrap())
        .unwrap();
    gz.finish().unwrap();
    compare(
        &mut rpc,
        "cpu.conformance",
        json!({"suite":"suite","compare_cycles":true}),
    );
    let unknown = rpc.request("tools/call", json!({"name":"missing","arguments":{}}));
    assert_eq!(unknown["error"]["code"], -32601);
    rpc.close();
}

#[test]
fn progress_cancellation_and_teardown_release_the_worker_slot() {
    let root = tempfile::tempdir().unwrap();
    create_rom(&root.path().join("loop.md"));
    let mut rpc = Rpc::start(root.path(), 1);
    let id=rpc.begin("tools/call",json!({"name":"boot","arguments":{"rom":"loop.md","instructions":100000000,"trace":0},"_meta":{"progressToken":"bounded-boot"}}));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let message = rpc.next(deadline.saturating_duration_since(Instant::now()));
        assert!(
            message.get("id").is_none(),
            "long run finished before cancellation test: {message}"
        );
        if message["method"] == "notifications/progress"
            && message["params"]["progress"].as_f64().unwrap() > 0.0
        {
            assert_eq!(message["params"]["progressToken"], "bounded-boot");
            break;
        }
    }
    rpc.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"test cancellation"}}));
    let start = Instant::now();
    let header = rpc.tool("header", json!({"rom":"loop.md"}));
    assert_eq!(header["ok"], true);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "cancelled worker still owns the sole slot"
    );
    // Disconnect while another long call runs: the runtime must not wait for
    // its whole instruction budget after the transport disappears.
    rpc.begin("tools/call",json!({"name":"boot","arguments":{"rom":"loop.md","instructions":100000000,"trace":0,"out":"teardown-capture"},"_meta":{"progressToken":"teardown"}}));
    loop {
        let message = rpc.next(Duration::from_secs(5));
        if message["method"] == "notifications/progress"
            && message["params"]["progressToken"] == "teardown"
            && message["params"]["progress"].as_f64().unwrap() > 0.0
        {
            break;
        }
    }
    rpc.close();
    let captured = fs::read_dir(root.path().join("store/objects"))
        .unwrap()
        .filter_map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let value: Value = serde_json::from_slice(&bytes).ok()?;
            value["data"]["memory_artifacts"]["wram"]["path"]
                .as_str()
                .is_some_and(|path| path.contains("teardown-capture"))
                .then_some(value)
        })
        .collect::<Vec<_>>();
    assert!(
        !captured.is_empty(),
        "no stopped-state capture from the disconnected running call"
    );
    for value in captured {
        assert_eq!(value["data"]["termination"], "cancelled");
        assert!(
            value["data"]["instructions"].as_u64().unwrap() < 100000000,
            "abandoned call finished its whole budget"
        );
    }
}

#[test]
fn concurrent_project_calls_keep_inputs_outputs_and_artifact_ids_in_their_own_context() {
    let server = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    create_rom(&server.path().join("game.md"));
    create_rom(&a.path().join("game.md"));
    create_rom(&b.path().join("game.md"));
    let mut bytes = fs::read(b.path().join("game.md")).unwrap();
    bytes[0x202..0x204].copy_from_slice(&[0x56, 0x78]);
    fs::write(b.path().join("game.md"), bytes).unwrap();
    fs::write(
        a.path().join("replay.json"),
        r#"{"events":[{"frame":0,"buttons":["right"]}]}"#,
    )
    .unwrap();
    let mut rpc = Rpc::start(server.path(), 2);
    let listed = rpc.request("tools/list", json!({}));
    for tool in listed["result"]["tools"].as_array().unwrap() {
        // Optional paths advertise the same simple value type as `rom`.
        // Nullable type arrays needlessly complicate provider schema adapters.
        for key in ["project_root", "artifact_store"] {
            assert_eq!(tool["inputSchema"]["properties"][key]["type"], "string");
            assert!(tool["inputSchema"]["required"]
                .as_array()
                .is_none_or(|fields| fields.iter().all(|field| field != key)));
        }
    }
    let args_a = json!({"project_root":a.path(),"rom":"game.md","instructions":8,
        "trace":0,"input":"replay.json","out":"captures"});
    let args_b = json!({"project_root":b.path(),"artifact_store":"analysis/artifacts",
        "rom":"game.md","instructions":8,"trace":0});
    let boot_schema = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "boot")
        .unwrap()["inputSchema"]
        .clone();
    let validator = jsonschema::validator_for(&boot_schema).unwrap();
    assert!(validator.is_valid(&args_a));
    assert!(validator.is_valid(&args_b));
    let id_a = rpc.begin("tools/call", json!({"name":"boot","arguments":args_a}));
    let id_b = rpc.begin("tools/call", json!({"name":"boot","arguments":args_b}));
    let boot_a = rpc.wait(id_a)["result"]["structuredContent"].clone();
    let boot_b = rpc.wait(id_b)["result"]["structuredContent"].clone();
    assert_eq!(boot_a["ok"], true, "{boot_a}");
    assert_eq!(boot_b["ok"], true, "{boot_b}");
    assert_ne!(boot_a["subject"]["sha256"], boot_b["subject"]["sha256"]);
    assert_eq!(boot_a["data"]["input_replay"]["final_pressed_masks"][0], 8);
    assert!(Path::new(
        boot_a["data"]["display"]["screenshot_path"]
            .as_str()
            .unwrap()
    )
    .starts_with(a.path().join("captures")));
    for (boot, store) in [
        (&boot_a, a.path().join(".mdx/artifacts")),
        (&boot_b, b.path().join("analysis/artifacts")),
    ] {
        assert!(
            Path::new(boot["data"]["snapshot_artifact"]["path"].as_str().unwrap())
                .starts_with(&store)
        );
    }
    for (root, store, boot, expected) in [
        (a.path(), Value::Null, &boot_a, "1234"),
        (b.path(), json!("analysis/artifacts"), &boot_b, "5678"),
    ] {
        let read = rpc.tool(
            "memory_read",
            json!({"project_root":root,"artifact_store":store,
            "snapshot":boot["data"]["snapshot_artifact"]["id"],"memory":"wram",
            "address":0xe54a,"length":2}),
        );
        assert_eq!(read["ok"], true, "{read}");
        assert_eq!(read["data"]["bytes"], expected);
    }
    let wrong = rpc.tool(
        "artifact_read",
        json!({"project_root":b.path(),
        "source":boot_a["data"]["snapshot_artifact"]["id"],"length":16}),
    );
    assert_eq!(wrong["ok"], false, "{wrong}");
    // A new scoped call and an unscoped call retain their independent defaults.
    let again = rpc.tool("header", json!({"project_root":a.path(),"rom":"game.md"}));
    assert_eq!(again["subject"]["sha256"], boot_a["subject"]["sha256"]);
    let unscoped = rpc.tool("artifact_import", json!({"path":"game.md"}));
    assert_eq!(unscoped["ok"], true, "{unscoped}");
    assert!(
        Path::new(unscoped["data"]["artifact"]["path"].as_str().unwrap())
            .starts_with(server.path().join("store"))
    );
    assert!(!server.path().join("captures").exists());
    assert!(!server.path().join(".mdx").exists());
    assert_eq!(cli(server.path(), "boot", &args_a), boot_a);
    assert_eq!(cli(server.path(), "boot", &args_b), boot_b);
    rpc.close();
}

#[test]
fn project_scope_is_inherited_by_surveys_and_shared_objects_publish_concurrently() {
    let server = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(a.path().join("payload.txt"), b"project-a").unwrap();
    fs::write(b.path().join("payload.txt"), b"project-b").unwrap();
    let mut rpc = Rpc::start(server.path(), 2);
    let batch = rpc.tool(
        "survey",
        json!({"project_root":a.path(),"tasks":[
            {"id":"a","operation":"artifact.import","arguments":{"path":"payload.txt"}},
            {"id":"b","operation":"artifact.import","arguments":{"project_root":b.path(),
                "artifact_store":"agent-b","path":"payload.txt"}}
        ]}),
    );
    assert_eq!(batch["ok"], true, "{batch}");
    let rows = batch["data"]["results"].as_array().unwrap();
    assert_eq!(rows[0]["response"]["ok"], true, "{batch}");
    assert_eq!(rows[1]["response"]["ok"], true, "{batch}");
    assert!(Path::new(
        rows[0]["response"]["data"]["artifact"]["path"]
            .as_str()
            .unwrap()
    )
    .starts_with(a.path().join(".mdx/artifacts")));
    assert!(Path::new(
        rows[1]["response"]["data"]["artifact"]["path"]
            .as_str()
            .unwrap()
    )
    .starts_with(b.path().join("agent-b")));
    let request = json!({"name":"artifact_import","arguments":{"project_root":a.path(),
        "path":"payload.txt"}});
    let first = rpc.begin("tools/call", request.clone());
    let second = rpc.begin("tools/call", request);
    let first = rpc.wait(first)["result"]["structuredContent"].clone();
    let second = rpc.wait(second)["result"]["structuredContent"].clone();
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(second, first);
    let invalid = rpc.tool(
        "artifact_import",
        json!({"project_root":"relative-project",
        "path":"payload.txt"}),
    );
    assert_eq!(invalid["error"]["code"], "invalid_arguments");
    assert_eq!(invalid["exit"], "usage");
    rpc.close();
}

#[test]
fn machine_states_resume_across_operations_cli_and_concurrent_mcp_calls() {
    let root = tempfile::tempdir().unwrap();
    create_rom(&root.path().join("loop.md"));
    let mut rpc = Rpc::start(root.path(), 2);
    let schemas = rpc.request("tools/list", json!({}));
    for name in ["boot", "trace", "watch", "ram_dump"] {
        let tool = schemas["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        assert_eq!(
            tool["inputSchema"]["properties"]["from_state"]["type"],
            "string"
        );
        assert!(!tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "from_state"));
    }
    fs::write(
        root.path().join("held.json"),
        r#"{"events":[{"frame":0,"port":0,"buttons":["a"]}]}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("release.json"),
        r#"{"events":[{"frame":0,"port":0,"buttons":[]}]}"#,
    )
    .unwrap();
    let saved = rpc.tool("boot", json!({"rom":"loop.md","instructions":100000,"frames":2,"trace":0,"input":"held.json","save_state":true}));
    assert_eq!(saved["ok"], true, "{saved}");
    assert_eq!(saved["data"]["elapsed_frames"], 2);
    let state = saved["data"]["state_artifact"]["id"].clone();
    let saved_bytes = fs::read(saved["data"]["state_artifact"]["path"].as_str().unwrap()).unwrap();
    let checkpoint: Value = serde_json::from_slice(&saved_bytes).unwrap();
    assert_ne!(checkpoint["io"]["pad_buttons"][0], 0);
    // Zero-step restore preserves every modeled field and the original artifact.
    let ram = rpc.tool("ram_dump", json!({"rom":"loop.md","from_state":state,"instructions":0,"save_state":true,"range":[0xe54a,2]}));
    assert_eq!(ram["ok"], true, "{ram}");
    assert_eq!(ram["data"]["state_artifact"]["id"], state);
    assert_eq!(ram["data"]["starting_frame"], 2);
    assert_eq!(ram["data"]["hex"][0]["bytes"], "12 34");
    let args = json!({"rom":"loop.md","from_state":state,"instructions":100000,"frames":1,"save_state":true,"input":"release.json","max_records":4});
    let mut watch = args.clone();
    watch["watch"] = json!({"address":0xe54a,"length":2});
    let first = rpc.begin("tools/call", json!({"name":"trace","arguments":args}));
    let second = rpc.begin("tools/call", json!({"name":"watch","arguments":watch}));
    let traced = rpc.wait(first)["result"]["structuredContent"].clone();
    let watched = rpc.wait(second)["result"]["structuredContent"].clone();
    for response in [&traced, &watched] {
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["data"]["starting_frame"], 2);
        assert_eq!(response["data"]["elapsed_frames"], 1);
        assert_eq!(response["data"]["frames"], 3);
        assert_eq!(response["data"]["input_events_applied"], 1);
    }
    assert_eq!(
        traced["data"]["state_artifact"]["id"],
        watched["data"]["state_artifact"]["id"]
    );
    let resumed: Value = serde_json::from_slice(
        &fs::read(traced["data"]["state_artifact"]["path"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(resumed["io"]["pad_buttons"][0], 0);
    let boot_args = json!({"rom":"loop.md","from_state":state,"instructions":100000,"frames":1,"save_state":true,"input":"release.json","trace":0});
    let boot = rpc.tool("boot", boot_args.clone());
    assert_eq!(boot, cli(root.path(), "boot", &boot_args));
    assert_eq!(
        boot["data"]["state_artifact"]["id"],
        traced["data"]["state_artifact"]["id"]
    );
    let uninterrupted = rpc.tool(
        "boot",
        json!({"rom":"loop.md","instructions":100000,"frames":3,"trace":0,"save_state":true}),
    );
    assert_eq!(
        boot["data"]["state_artifact"]["id"],
        uninterrupted["data"]["state_artifact"]["id"]
    );
    // Explicit CLI flags use the same contract, including trace/watch's JSON args.
    for command in [
        vec!["boot", "--rom", "loop.md", "--instructions", "0"],
        vec!["ram", "dump", "--rom", "loop.md", "--instructions", "0"],
        vec!["trace", "--args", r#"{"rom":"loop.md","instructions":0}"#],
        vec![
            "watch",
            "--args",
            r#"{"rom":"loop.md","instructions":0,"watch":{"address":0,"length":1}}"#,
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mdx"))
            .current_dir(root.path())
            .args(["--store", "store"])
            .args(command)
            .args(["--from-state", state.as_str().unwrap(), "--save-state"])
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["data"]["state_artifact"]["id"], state);
    }
    assert_eq!(
        fs::read(saved["data"]["state_artifact"]["path"].as_str().unwrap()).unwrap(),
        saved_bytes
    );
    rpc.close();
}

#[test]
fn concurrent_audio_captures_keep_console_and_output_scope_independent() {
    let server=tempfile::tempdir().unwrap();let a=tempfile::tempdir().unwrap();let b=tempfile::tempdir().unwrap();
    create_rom(&a.path().join("game.md"));create_rom(&b.path().join("game.md"));
    let mut rpc=Rpc::start(server.path(),2);
    let first=rpc.begin("tools/call",json!({"name":"audio","arguments":{"project_root":a.path(),"rom":"game.md","frames":10,"instructions":1000000,"save_state":true,"out":"waves"}}));
    let second=rpc.begin("tools/call",json!({"name":"audio","arguments":{"project_root":b.path(),"rom":"game.md","frames":20,"instructions":1000000,"save_state":true,"out":"waves"}}));
    let first=rpc.wait(first)["result"]["structuredContent"].clone();
    let second=rpc.wait(second)["result"]["structuredContent"].clone();
    for (report,root,frame) in [(&first,a.path(),10),(&second,b.path(),20)] {
        assert_eq!(report["ok"],true,"{report}");assert_eq!(report["data"]["reached_frames"],frame);
        for field in ["wav_artifact","state_artifact"] {
            assert!(Path::new(report["data"][field]["path"].as_str().unwrap()).starts_with(root));
        }
    }
    assert_ne!(first["data"]["wav_artifact"]["id"],second["data"]["wav_artifact"]["id"]);
    let repeated=rpc.tool("audio",json!({"project_root":a.path(),"rom":"game.md","frames":10,"instructions":1000000,"save_state":true,"out":"waves"}));
    assert_eq!(repeated,first);assert!(!server.path().join("store").exists());rpc.close();
}
