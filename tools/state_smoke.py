#!/usr/bin/env python3
"""Compare real-cartridge split/resumed execution with uninterrupted execution.

Uses the CLI and actual MCP stdio server; never changes the ROM, input or oracle.
Outputs are local analysis artifacts, including memory-bearing machine states.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
from mcp_smoke import Client


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/release/mdx"))
    parser.add_argument("--rom", type=pathlib.Path, required=True)
    parser.add_argument("--input", type=pathlib.Path, required=True)
    parser.add_argument("--split", type=int, default=2700)
    parser.add_argument("--frames", type=int, default=4500)
    parser.add_argument("--extend", type=int, default=0)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    resolve = lambda path: path if path.is_absolute() else root / path
    binary, rom, replay, out = map(resolve, [args.binary, args.rom, args.input, args.out])
    assert 0 < args.split < args.frames and args.extend >= 0
    out.mkdir(parents=True, exist_ok=False)
    events = json.loads(replay.read_text())["events"]
    tail = out / "remaining-input.json"
    tail.write_text(json.dumps({"events": [{**e, "frame": e["frame"] - args.split}
                                            for e in events if e["frame"] >= args.split]}))
    store = out / "artifacts"

    def cli(name, frames):
        output = subprocess.run([str(binary), "--root", str(root), "--store", str(store),
            "boot", "--rom", str(rom), "--input", str(replay), "--frames", str(frames),
            "--instructions", "100000000", "--trace", "0", "--save-state"],
            capture_output=True, check=True)
        report = json.loads(output.stdout)
        assert report["data"]["frame_target_reached"], report
        (out / (name + ".json")).write_text(json.dumps(report, indent=2) + "\n")
        return report

    checkpoint = cli("checkpoint", args.split)
    full = cli("uninterrupted", args.frames)
    state_id = checkpoint["data"]["state_artifact"]["id"]
    client = Client(binary, root, store)
    try:
        common = {"project_root": str(root), "artifact_store": str(store), "rom": str(rom),
                  "from_state": state_id, "save_state": True, "instructions": 100000000}
        resumed = client.call("boot", {**common, "frames": args.frames - args.split,
                                       "input": str(tail), "trace": 0})
        assert resumed["data"]["frame_target_reached"], resumed
        assert resumed["data"]["starting_frame"] == args.split
        assert resumed["data"]["elapsed_frames"] == args.frames - args.split
        for field in ["state_artifact", "memory_artifacts"]:
            a, b = resumed["data"][field], full["data"][field]
            if field == "state_artifact":
                assert a["id"] == b["id"], field
            else:
                assert {k: v["sha256"] for k, v in a.items()} == {k: v["sha256"] for k, v in b.items()}, field
        assert resumed["data"]["display"]["screenshot_artifact"]["sha256"] == full["data"]["display"]["screenshot_artifact"]["sha256"]
        loaded = client.call("ram.dump", {**common, "instructions": 0, "range": [0xA40, 32]})
        assert loaded["data"]["state_artifact"]["id"] == state_id
        short = {**common, "frames": 2, "input": str(tail)}
        boot = client.call("boot", {**short, "trace": 0})
        trace = client.call("trace", {**short, "max_records": 16})
        watch = client.call("watch", {**short, "watch": {"address": 0xA40, "length": 32}, "max_records": 16})
        ids = [result["data"]["state_artifact"]["id"] for result in [boot, trace, watch]]
        assert len(set(ids)) == 1, ids
        extended = None
        if args.extend:
            extension_input = out / "extension-input.json"
            extension_input.write_text(json.dumps({"events": [{**e, "frame": e["frame"] - args.frames}
                for e in events if e["frame"] >= args.frames]}))
            extended = client.call("boot", {**common,
                "from_state": resumed["data"]["state_artifact"]["id"],
                "frames": args.extend, "input": str(extension_input), "trace": 0})
            assert extended["data"]["frame_target_reached"], extended
        report = {"schema": "mdx.state-smoke.v1", "binary_sha256": digest(binary),
            "rom_sha256": digest(rom), "input_sha256": digest(replay),
            "split_frame": args.split, "final_frame": args.frames,
            "state_bytes": checkpoint["data"]["state_artifact"]["bytes"],
            "full_state_equal": True, "memory_equal": True, "screenshot_equal": True,
            "zero_step_ram_state_equal": True, "boot_trace_watch_state_equal": True,
            "uninterrupted_steps": full["data"]["instructions"],
            "checkpoint_steps": checkpoint["data"]["instructions"],
            "resumed_steps": resumed["data"]["instructions"],
            "extended_frame": extended["data"]["reached_frames"] if extended else None,
            "extended_steps": extended["data"]["instructions"] if extended else None,
            "checkpoint_state": state_id,
            "final_state": resumed["data"]["state_artifact"]["id"],
            "calls": client.calls,
            "limitations": ["mdcore continuation comparison, not hardware/audio equivalence",
                            "ROM/replay/oracle execution and frozen images unchanged"]}
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({k: v for k, v in report.items() if k != "calls"}))
    finally:
        client.close()


if __name__ == "__main__":
    main()
