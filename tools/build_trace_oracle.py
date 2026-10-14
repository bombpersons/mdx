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
    ap.add_argument("--bus-trace", action="store_true", help="observe Z80/68000 bus contention without changing execution")
    ap.add_argument("--z80-trace", action="store_true", help="observe pre-dispatch sound CPU state without changing execution")
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
    text = text.replace('#include "m68kops.h"', '#include "m68kops.h"\n#define MDX_TRACE_CYCLES ((unsigned int)m68k.cycles)\n' + include, 1)
    source.write_text(text.replace(anchor, 'mdx_trace_instruction();\n      ' + anchor))
    source.with_suffix(".o").unlink(missing_ok=True)
    bus_manifest = {}
    if args.bus_trace:
        bus_source = args.output / "core/memz80.c"
        original_bus = bus_source.read_bytes()
        bus_text = original_bus.decode().replace("\r\n", "\n")
        definition = "static void z80_request_68k_bus_access(void)"
        if bus_text.count(definition) != 1:
            raise SystemExit("unsupported oracle bus source")
        bus_text = bus_text.replace(definition, "static void z80_request_68k_bus_access_unobserved(void)", 1)
        anchor = "unsigned char z80_memory_r(unsigned int address)"
        if bus_text.count(anchor) != 1:
            raise SystemExit("unsupported oracle bus-handler source")
        hook = pathlib.Path(__file__).with_name("oracle_bus_trace.inc").read_text()
        bus_source.write_text(bus_text.replace(anchor, hook + "\n" + anchor, 1))
        bus_source.with_suffix(".o").unlink(missing_ok=True)
        bus_manifest = {"original_bus_sha256": hashlib.sha256(original_bus).hexdigest(),
                        "instrumented_bus_sha256": hashlib.sha256(bus_source.read_bytes()).hexdigest(),
                        "bus_hook_sha256": hashlib.sha256(hook.encode()).hexdigest(),
                        "bus_trace": "MDX_ORACLE_BUS_TRACE: first4096 original bus requests, before/after CPU and Z80 clocks"}
    z80_manifest = {}
    if args.z80_trace:
        z80_source = args.output / "core/z80/z80.c"
        original_z80 = z80_source.read_bytes()
        z80_text = original_z80.decode().replace("\r\n", "\n")
        hook = pathlib.Path(__file__).with_name("oracle_z80_trace.inc").read_text()
        anchor = "void z80_run(unsigned int cycles)"
        if z80_text.count(anchor) != 1:
            raise SystemExit("unsupported oracle Z80 source")
        z80_text = z80_text.replace(anchor, hook + "\n" + anchor, 1)
        anchor = "    /* check for IRQs before each instruction */"
        if z80_text.count(anchor) != 1:
            raise SystemExit("unsupported oracle Z80 dispatch loop")
        z80_source.write_text(z80_text.replace(anchor, "    mdx_trace_z80();\n" + anchor, 1))
        z80_source.with_suffix(".o").unlink(missing_ok=True)
        z80_manifest = {"original_z80_sha256": hashlib.sha256(original_z80).hexdigest(),
                        "instrumented_z80_sha256": hashlib.sha256(z80_source.read_bytes()).hexdigest(),
                        "z80_hook_sha256": hashlib.sha256(hook.encode()).hexdigest()}
    subprocess.run(["make", "-f", "Makefile.libretro", "-j2"], cwd=args.output, check=True)
    core = args.output / "genesis_plus_gx_libretro.so"
    manifest = {
        **bus_manifest,
        **z80_manifest,
        "original_cpu_sha256": hashlib.sha256(original).hexdigest(),
        "instrumented_cpu_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "hook_sha256": hashlib.sha256(include.encode()).hexdigest(),
        "core_sha256": hashlib.sha256(core.read_bytes()).hexdigest(),
        "record_layout": "18 little-endian u32: pc,sr,d0..d7,a0..a7; pre-instruction",
        "optional_timing_layout": "MDX_ORACLE_TRACE_TIMING: 3 little-endian u32 per retained record: dispatch ordinal, supplied zero-based replay frame, pre-instruction master cycles within oracle frame",
        "limitations": ["instrumented controlled replay; CPU state only; no cycle equivalence claim"],
    }
    (args.output / "trace-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"core": str(core.resolve()), "manifest": manifest}))


if __name__ == "__main__":
    main()
