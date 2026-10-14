#!/usr/bin/env python3
"""Compare autovector/HV timing using authored programs and unchanged GPGX."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

from hv_counter_phase import make_rom

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from oracle_vdp_snapshot import normalize


def make_irq_rom(wide, startup_nops):
    rom = bytearray(make_rom(True, wide)[0])
    rom[0x78:0x7c] = struct.pack(">I", 0x1000)  # level6 autovector
    code = [0x33fc, 0x8124, 0x00c0, 0x0004]
    if wide:
        code += [0x33fc, 0x8c81, 0x00c0, 0x0004]
    code += [0x207c, 0x00ff, 0xf000, 0x7e00]  # A0=output, D7=count
    code += [0x4e71] * startup_nops + [0x46fc, 0x2000, 0x60fe]
    handler = [
        0x3039, 0x00c0, 0x0008,  # read HV
        0x30c0, 0x5247,          # store through A0+, increment D7
        0x0c47, 32, 0x660c,     # after32 samples, publish completion
        0x33fc, 0xc0de, 0x00ff, 0xf040,
        0x3ebc, 0x2700,         # mask future IRQs in saved SR
        0x4e73,                 # RTE
    ]
    for start, words in ((0x200, code), (0x1000, handler)):
        data = struct.pack(">" + "H" * len(words), *words)
        rom[start:start + len(data)] = data
    checksum = sum(struct.unpack(">32512H", rom[0x200:])) & 65535
    rom[0x18e:0x190] = struct.pack(">H", checksum)
    return bytes(rom)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mdx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--startup-nops", type=int, nargs="+", default=list(range(10)))
    parser.add_argument("--frames", type=int, default=40)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    if not 33 <= args.frames <= 60:
        parser.error("--frames must be33..60 to collect32 IRQ samples")
    if any(not 0 <= value <= 63 for value in args.startup_nops):
        parser.error("--startup-nops must be0..63")
    args.out.mkdir(parents=True, exist_ok=True)
    binary_hash = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    profiles = []
    for wide in (False, True):
        for nops in args.startup_nops:
            name = f"h{40 if wide else 32}-irq-nops{nops}"
            folder = args.out / name
            folder.mkdir()
            data = make_irq_rom(wide, nops)
            rom = folder / "probe.md"
            rom.write_bytes(data)
            result = subprocess.run([
                str(args.binary.resolve()), "boot", "--rom", str(rom.resolve()),
                "--frames", str(args.frames), "--instructions", "1000000",
                "--trace", "0", "--out", str((folder / "mdx").resolve()),
            ], cwd=ROOT, capture_output=True, text=True, check=True)
            (folder / "mdx.json").write_text(result.stdout)
            report = json.loads(result.stdout)
            subprocess.run([
                sys.executable, str(ROOT / "tools/oracle_gpgx.py"), str(rom.resolve()),
                "--frames", str(args.frames), "--out", str((folder / "oracle.png").resolve()),
                "--state", str((folder / "oracle.state").resolve()),
            ], cwd=ROOT, capture_output=True, text=True, check=True)
            snapshot = normalize(folder / "oracle.state", folder / "oracle-snapshot")
            actual = Path(report["data"]["memory_artifacts"]["wram"]["path"]).read_bytes()[0xf000:0xf042]
            reference = Path(snapshot["memory_artifacts"]["wram"]["path"]).read_bytes()[0xf000:0xf042]
            if actual[64:] != b"\xc0\xde" or reference[64:] != b"\xc0\xde":
                raise SystemExit(f"{name}: sample program incomplete in mdx or oracle")
            ours = list(struct.unpack(">32H", actual[:64]))
            oracle = list(struct.unpack(">32H", reference[:64]))
            profile = {
                "name": name, "wide": wide, "startup_nops": nops,
                "rom_sha256": hashlib.sha256(data).hexdigest(),
                "mdx": ours, "oracle": oracle,
                "equal": sum(a == b for a, b in zip(ours, oracle)),
            }
            profiles.append(profile)
            print(json.dumps({"profile": name, "equal": profile["equal"], "total": 32}), flush=True)
    summary = {
        "binary_sha256": binary_hash, "profiles": profiles,
        "equal": sum(p["equal"] for p in profiles), "total": len(profiles) * 32,
    }
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if args.verify and summary["equal"] != summary["total"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
