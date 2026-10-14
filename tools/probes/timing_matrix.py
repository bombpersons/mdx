"""Shared completion-checked runner for authored instruction timing matrices."""
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys

from hv_counter_phase import ROOT, make_rom, normalize


def authored_rom(code, handler=None):
    rom = bytearray(make_rom(True, False)[0])
    if handler is not None:
        rom[0x78:0x7c] = struct.pack(">I", 0x1000)
    for at, words in ((0x200, code), (0x1000, handler or [])):
        rom[at:at+2*len(words)] = struct.pack(">"+"H"*len(words), *words)
    rom[0x18e:0x190] = struct.pack(">H", sum(struct.unpack(">32512H", rom[0x200:])) & 65535)
    return bytes(rom)


def run_matrix(args, cases, program, extract):
    def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source = ROOT / "cache/oracle/Genesis-Plus-GX"
    env = {k: v for k, v in os.environ.items() if not k.startswith("MDX_ORACLE_TRACE")}
    env["MDX_ORACLE_CORE"] = str(source / "genesis_plus_gx_libretro.so")
    frames = str(getattr(args, "frames", 3))
    rows = []
    for index, case in enumerate(cases):
        folder = out / str(index)
        folder.mkdir()
        rom = folder / "probe.md"
        rom.write_bytes(program(case))
        # Optional controller replay, given to both emulators identically.
        replay = []
        if "replay" in case:
            (folder / "replay.json").write_text(json.dumps({"events": case["replay"]}))
            replay = ["--input", str(folder / "replay.json")]
        call = subprocess.run([str(args.binary.resolve()), "boot", "--rom", str(rom),
                               "--frames", frames, "--instructions", "1000000", "--trace", "0",
                               "--out", str(folder / "mdx")] + replay, cwd=ROOT,
                              capture_output=True, text=True, check=True)
        (folder / "mdx.json").write_text(call.stdout)
        report = json.loads(call.stdout)
        subprocess.run([sys.executable, str(ROOT / "tools/oracle_gpgx.py"), str(rom),
                        "--frames", frames, "--out", str(folder / "oracle.png"),
                        "--state", str(folder / "oracle.state")] + replay, cwd=ROOT,
                       env=env, capture_output=True, text=True, check=True)
        snapshot = normalize(folder / "oracle.state", folder / "oracle-snapshot")
        actual = Path(report["data"]["memory_artifacts"]["wram"]["path"]).read_bytes()
        reference = Path(snapshot["memory_artifacts"]["wram"]["path"]).read_bytes()
        if any(data[0xf040:0xf042] != b"\xc0\xde" for data in (actual, reference)):
            raise SystemExit(f"case {index} incomplete: {case}")
        ours, expected = extract(actual), extract(reference)
        rows.append({**case, "rom_sha256": digest(rom), "actual": ours,
                     "expected": expected, "equal": ours == expected})
    summary = {"binary_sha256": digest(args.binary),
               "oracle_core_sha256": digest(source / "genesis_plus_gx_libretro.so"),
               "oracle_opcodes_sha256": digest(source / "core/m68k/m68kops.h"),
               "cases": rows, "equal": sum(row["equal"] for row in rows), "total": len(rows)}
    (out / "summary.json").write_text(json.dumps(summary, indent=2)+"\n")
    if args.fixture:
        fixture = {k: v for k, v in summary.items() if k != "binary_sha256"}
        fixture["cases"] = [{k: v for k, v in row.items() if k not in ("actual", "equal")} for row in rows]
        args.fixture.write_text(json.dumps(fixture, indent=2)+"\n")
    print(json.dumps({k: v for k, v in summary.items() if k != "cases"}))
    if summary["equal"] != summary["total"]:
        raise SystemExit(1)
