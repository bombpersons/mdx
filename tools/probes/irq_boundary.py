#!/usr/bin/env python3
"""Compare first IRQ dispatch clocks/registers with an observation-only GPGX core.

Build mdcore's irq_boundary release example and a private oracle using
tools/build_trace_oracle.py first. Both executables are pinned for this run.
No reference execution, fixtures, replay or commercial ROM is modified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys

from irq_counter_phase import make_irq_rom

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/irq_boundary")
    parser.add_argument("--oracle-core", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    binary, core, out = args.binary.resolve(), args.oracle_core.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    profiles = []
    for wide in (False, True):
        for nops in range(10):
            name = f"h{40 if wide else 32}-n{nops}"
            prefix = out / name
            rom = prefix.with_suffix(".md")
            rom.write_bytes(make_irq_rom(wide, nops))
            env = dict(os.environ, MDX_ORACLE_CORE=str(core),
                       MDX_ORACLE_TRACE=str(prefix.with_suffix(".trace")),
                       MDX_ORACLE_TRACE_TIMING=str(prefix.with_suffix(".timing")),
                       MDX_ORACLE_TRACE_LIMIT="1800")
            # A caller's trace selectors must not hide startup or the first IRQ.
            env.pop("MDX_ORACLE_TRACE_START_PC", None)
            env.pop("MDX_ORACLE_TRACE_START_COUNT", None)
            subprocess.run([sys.executable, str(ROOT / "tools/oracle_gpgx.py"), str(rom),
                            "--frames", "2", "--out", str(prefix.with_suffix(".png"))],
                           cwd=ROOT, env=env, check=True, capture_output=True)
            result = subprocess.run([str(binary), str(rom)], cwd=ROOT, check=True,
                                    capture_output=True, text=True)
            prefix.with_suffix(".jsonl").write_text(result.stdout)
            actual = list(map(json.loads, result.stdout.splitlines()))
            reference = list(struct.iter_unpack("<18I", prefix.with_suffix(".trace").read_bytes()))
            timing = list(struct.iter_unpack("<3I", prefix.with_suffix(".timing").read_bytes()))
            if len(reference) != len(timing):
                raise SystemExit(f"{name}: incomplete oracle timing records")
            ours = next(row for row in actual if row["pc"] == 0x1000)
            index = next(i for i, row in enumerate(reference) if row[0] == 0x1000)
            profile = {
                "name": name, "rom_sha256": digest(rom),
                "mdx_entry_master": ours["master"] % 896040,
                "oracle_entry_master": timing[index][2],
                "mdx_hv": ours["post_d"][0] & 65535,
                "oracle_hv": reference[index + 1][2] & 65535,
                "same_registers": [ours["pc"], ours["sr"]] + ours["d"] + ours["a"] == list(reference[index]),
                "initial_dispatches_equal": all(
                    actual[i]["master"] == timing[i][2] and actual[i]["pc"] == reference[i][0]
                    for i in range(20)),
            }
            profiles.append(profile)
    summary = {"binary_sha256": digest(binary), "oracle_core_sha256": digest(core),
               "profiles": profiles, "total": len(profiles),
               "entry_clocks_equal": sum(p["mdx_entry_master"] == p["oracle_entry_master"] for p in profiles),
               "entry_registers_equal": sum(p["same_registers"] for p in profiles),
               "initial_dispatches_equal": sum(p["initial_dispatches_equal"] for p in profiles),
               "hv_equal": sum(p["mdx_hv"] == p["oracle_hv"] for p in profiles)}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({k: v for k, v in summary.items() if k != "profiles"}))
    if any(summary[key] != len(profiles) for key in (
            "entry_clocks_equal", "entry_registers_equal", "initial_dispatches_equal", "hv_equal")):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
