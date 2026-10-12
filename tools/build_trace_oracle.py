#!/usr/bin/env python3
"""Build a private, observation-instrumented GPGX core; preserve the original."""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--source", type=pathlib.Path, default=pathlib.Path("cache/oracle/Genesis-Plus-GX"))
    ap.add_argument("--output", type=pathlib.Path, default=pathlib.Path("cache/oracle/trace-build"))
    args = ap.parse_args()
    if args.output.exists():
        raise SystemExit("output must be absent (reuse its built core, or select a new directory)")
    shutil.copytree(args.source, args.output, ignore=shutil.ignore_patterns(".git"))
    source = args.output / "core/m68k/m68kcpu.c"
    original = source.read_bytes()
    text = original.decode().replace("\r\n", "\n")
    include = pathlib.Path(__file__).with_name("oracle_trace.inc").read_text()
    anchor = 'REG_IR = m68ki_read_imm_16();'
    # m68k_run and m68k_set_irq_delay both execute instructions. Instrument
    # both, otherwise VDP writes that enable IRQs silently omit one event.
    if text.count(anchor) != 2:
        raise SystemExit("unsupported oracle source: expected two instruction dispatch sites")
    text = text.replace('#include "m68kops.h"', '#include "m68kops.h"\n' + include, 1)
    source.write_text(text.replace(anchor, 'mdx_trace_instruction();\n      ' + anchor))
    source.with_suffix(".o").unlink(missing_ok=True)
    subprocess.run(["make", "-f", "Makefile.libretro", "-j2"], cwd=args.output, check=True)
    core = args.output / "genesis_plus_gx_libretro.so"
    manifest = {
        "original_cpu_sha256": hashlib.sha256(original).hexdigest(),
        "instrumented_cpu_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "hook_sha256": hashlib.sha256(include.encode()).hexdigest(),
        "core_sha256": hashlib.sha256(core.read_bytes()).hexdigest(),
        "record_layout": "18 little-endian u32: pc,sr,d0..d7,a0..a7; pre-instruction",
        "limitations": ["instrumented controlled replay; CPU state only; no cycle equivalence claim"],
    }
    (args.output / "trace-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"core": str(core.resolve()), "manifest": manifest}))


if __name__ == "__main__":
    main()
