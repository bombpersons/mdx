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
    compare(
        &mut rpc,
        "trace",
        json!({"rom":id,"instructions":8,"max_records":2}),
    );
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
