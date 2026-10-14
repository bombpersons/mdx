#!/usr/bin/env python3
"""Collect completion-checked MOVEM/HV phases from unchanged GPGX and mdx."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

from hv_counter_phase import ROOT, make_rom, normalize


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mdx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    cases = []
    # Word/long, load/store, indirect and postincrement/predecrement modes.
    # Read masks exclude A6/A7 so subsequent samples retain a safe WRAM base.
    for wide in (False, True):
        for opcode in (0x4896, 0x48a6, 0x4c96, 0x4c9e,
                       0x48d6, 0x48e6, 0x4cd6, 0x4cde):
            for mask in (0, 1, 0xff, 0x3fff):
                name = f"h{40 if wide else 32}-{opcode:04x}-{mask:04x}"
                folder = out / name
                folder.mkdir()
                rom = folder / "probe.md"
                rom.write_bytes(make_rom(True, wide, movem_opcode=opcode, movem_mask=mask)[0])
                result = subprocess.run([str(args.binary.resolve()), "boot", "--rom", str(rom),
                                         "--frames", "4", "--instructions", "1000000", "--trace", "0",
                                         "--out", str(folder / "mdx")], cwd=ROOT,
                                        capture_output=True, text=True, check=True)
                (folder / "mdx.json").write_text(result.stdout)
                report = json.loads(result.stdout)
                subprocess.run([sys.executable, str(ROOT / "tools/oracle_gpgx.py"), str(rom),
                                "--frames", "4", "--out", str(folder / "oracle.png"),
                                "--state", str(folder / "oracle.state")], cwd=ROOT,
                               capture_output=True, text=True, check=True)
                snapshot = normalize(folder / "oracle.state", folder / "oracle-snapshot")
                actual = Path(report["data"]["memory_artifacts"]["wram"]["path"]).read_bytes()[0xf000:0xf042]
                reference = Path(snapshot["memory_artifacts"]["wram"]["path"]).read_bytes()[0xf000:0xf042]
                if actual[64:] != b"\xc0\xde" or reference[64:] != b"\xc0\xde":
                    raise SystemExit(f"{name}: incomplete probe")
                ours, expected = (list(struct.unpack(">32H", data[:64])) for data in (actual, reference))
                cases.append({"name": name, "wide": wide, "opcode": opcode, "mask": mask,
                              "rom_sha256": digest(rom), "actual": ours, "expected": expected,
                              "equal": sum(a == b for a, b in zip(ours, expected))})
    source = ROOT / "cache/oracle/Genesis-Plus-GX"
    summary = {"binary_sha256": digest(args.binary), "oracle_core_sha256": digest(source / "genesis_plus_gx_libretro.so"),
               "oracle_cpu_sha256": digest(source / "core/m68k/m68kcpu.c"),
               "oracle_opcodes_sha256": digest(source / "core/m68k/m68kops.h"),
               "cases": cases, "equal": sum(case["equal"] for case in cases), "total": len(cases)*32}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if args.fixture:
        fixture = {k: v for k, v in summary.items() if k != "binary_sha256"}
        fixture["cases"] = [{k: v for k, v in case.items() if k not in ("actual", "equal")} for case in cases]
        args.fixture.write_text(json.dumps(fixture, indent=2) + "\n")
    print(json.dumps({k: v for k, v in summary.items() if k != "cases"}))
    if summary["equal"] != summary["total"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
