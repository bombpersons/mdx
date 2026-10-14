#!/usr/bin/env python3
"""Verify mdx's real MCP stdio interface against the local cartridge set.

This is a protocol test driver, not an analysis implementation: every analysis
and artifact query runs through the advertised MCP tools. Python stdlib only.
"""
import argparse
import hashlib
import json
import pathlib
import queue
import subprocess
import threading
import time


class Client:
    def __init__(self, executable, root, store):
        self.process = subprocess.Popen(
            [str(executable), "--root", str(root), "--store", str(store), "mcp"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, bufsize=1,
        )
        self.messages = queue.Queue()
        self.next_id = 0
        self.notifications = []
        self.calls = []
        self.stderr = []
        threading.Thread(target=self.read, daemon=True).start()
        threading.Thread(target=lambda: self.stderr.extend(self.process.stderr), daemon=True).start()
        result = self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "mdx-cartridge-smoke", "version": "1"}})
        self.server = result["result"]
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
        self.tools = self.request("tools/list", {})["result"]["tools"]

    def read(self):
        try:
            for line in self.process.stdout:
                self.messages.put(json.loads(line))
        except Exception as error:
            self.messages.put(error)
        finally:
            self.messages.put(EOFError("MCP stdout closed"))

    def send(self, message):
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        identity = self.next_id
        self.send({"jsonrpc": "2.0", "id": identity, "method": method, "params": params})
        deadline = time.monotonic() + 120
        while True:
            result = self.messages.get(timeout=max(0, deadline-time.monotonic()))
            if isinstance(result, Exception):
                raise result
            if result.get("id") == identity:
                return result
            if "id" in result:
                raise RuntimeError(f"unexpected response ID: {result}")
            self.notifications.append(result)

    def call(self, operation, arguments):
        tool = operation.replace(".", "_")
        result = self.request("tools/call", {"name": tool, "arguments": arguments,
            "_meta": {"progressToken": f"call-{self.next_id+1}"}})
        if "error" in result:
            raise RuntimeError(result["error"])
        envelope = result["result"]["structuredContent"]
        if json.loads(result["result"]["content"][0]["text"]) != envelope:
            raise RuntimeError("text/structured result disagreement")
        self.calls.append({"operation": operation, "arguments": arguments, "response": envelope})
        return envelope

    def field(self, envelope, pointer):
        if "result_artifact" not in envelope["data"]:
            value = envelope
            for component in pointer.strip("/").split("/"):
                value = value[component]
            return value
        view = self.call("artifact.view", {"source": envelope["data"]["result_artifact"]["id"], "pointer": pointer})
        if "result_artifact" in view["data"]:
            raise RuntimeError("select a smaller field instead of requesting another large view")
        return view["data"]["value"]

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise RuntimeError("MCP process did not stop after EOF")
        if self.process.returncode != 0:
            raise RuntimeError("".join(self.stderr))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/release/mdx"))
    parser.add_argument("--store", type=pathlib.Path, default=pathlib.Path(".mdx/mcp-smoke"))
    parser.add_argument("--out", type=pathlib.Path, default=pathlib.Path("docs/evidence/mcp-smoke.json"))
    parser.add_argument("--walking", action="store_true", help="also exercise the controlled walking/asset/watch workflow")
    args = parser.parse_args()
    root = args.root.resolve()
    binary = args.binary.resolve()
    client = Client(binary, root, args.store)
    report = {"schema": "mdx.mcp-smoke.v1", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "server": client.server, "tool_names": [tool["name"] for tool in client.tools],
        "cartridges": [], "limitations": ["bounded cross-cartridge smoke checks, not full hardware/whole-game fidelity verification"]}
    try:
        cartridges = ["rom/original/kings_bounty.md", "rom/original/Cadash (USA, Korea).md",
            "rom/original/ewj/Earthworm Jim (USA).md",
            "rom/original/hokuto/Hokuto no Ken - Shin Seikimatsu Kyuuseishu Densetsu (Japan).md"]
        ids = {}
        for path in cartridges:
            imported = client.call("artifact.import", {"path": path})
            assert imported["ok"], imported
            identity = imported["data"]["artifact"]["id"]
            ids[path] = identity
            header = client.call("header", {"rom": identity})
            assert header["ok"], header
            entry = int(header["data"]["vectors"][1]["address"], 16)
            disasm = client.call("disasm", {"rom": identity, "from": entry & 0xffffff, "count": 8})
            assert disasm["ok"], disasm
            boot = client.call("boot", {"rom": identity, "frames": 1, "instructions": 200000, "trace": 0})
            row = {"rom": path, "id": identity, "entry": entry, "boot_exit": boot["exit"]}
            if boot["data"] is None:
                row["error"] = boot["error"]
            else:
                row.update({"termination": client.field(boot, "/data/termination"),
                    "frames": client.field(boot, "/data/vdp/frame"),
                    "steps": client.field(boot, "/data/instructions"),
                    "exceptions": client.field(boot, "/data/exceptions")})
            report["cartridges"].append(row)
        if args.walking:
            identity = ids[cartridges[0]]
            boot = client.call("boot", {"rom": identity, "frames": 4500, "instructions": 80000000,
                "trace": 0, "input": "testdata/kb_walk_replay.json", "timeout_ms": 120000})
            assert boot["ok"], boot
            snapshot = client.field(boot, "/data/snapshot_artifact")["id"]
            display = client.field(boot, "/data/display")
            memory = client.call("memory.read", {"snapshot": snapshot, "memory": "wram", "address": 0xffe54c, "length": 1})
            graphics = client.call("graphics", {"snapshot": snapshot})
            assert graphics["ok"], graphics
            layout = client.call("artifact.view", {"source": graphics["data"]["layout"]["id"],
                "pointer": "/sprites", "offset": 14, "limit": 2})
            assert layout["ok"], layout
            watch = client.call("watch", {"rom": identity, "frames": 4500, "start_frame": 4300,
                "instructions": 80000000, "max_records": 64,
                "input": "testdata/kb_walk_replay.json", "watch": {"address": 0xe54c, "length": 1}, "timeout_ms": 120000})
            assert watch["ok"], watch
            records = client.call("artifact.view", {"source": watch["data"]["records_artifact"]["id"], "format": "jsonl", "limit": 4})
            assert records["ok"], records
            assert watch["data"]["omitted_records"] == 0
            report["walking"] = {"snapshot_id": snapshot, "display": display, "phase_read": memory,
                "rider_sprites": layout["data"]["value"], "watch": watch, "first_writes": records["data"]["value"]}
        report["calls"] = client.calls
        report["progress_notifications"] = len(client.notifications)
    finally:
        client.close()
    output = args.out if args.out.is_absolute() else root / args.out
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"report": str(output), "tools": len(client.tools), "cartridges": report["cartridges"],
        "walking": "walking" in report, "progress_notifications": report["progress_notifications"]}))


if __name__ == "__main__":
    main()
