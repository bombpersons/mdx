#!/usr/bin/env python3
"""Measure SR-write acknowledgement of an already pending VINT."""
import argparse
from pathlib import Path
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code = [0x33fc, 0x8124, 0x00c0, 0x0004]
    if case["wide"]:
        code += [0x33fc, 0x8c81, 0x00c0, 0x0004]
    # Keep SR masked while a fixed-count loop runs past the first VINT edge.
    code += [0x3c3c, 2000, 0x51ce, 0xfffe]
    code += [0x4e71]*case["nops"]
    form = case["form"]
    if form == "rte":
        return_pc = 0x200 + 2*(len(code)+6)
        code += [0x2f3c, return_pc >> 16, return_pc & 65535, 0x3f3c, 0x2000, 0x4e73]
    else:
        code += {"move": [0x46fc, 0x2000], "and": [0x027c, 0xf8ff],
                 "eor": [0x0a7c, 0x0700], "stop": [0x4e72, 0x2000]}[form]
    code += [0x60fe]
    handler = [0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000,
               0x33fc, 0xc0de, 0x00ff, 0xf040, 0x3ebc, 0x2700, 0x4e73]
    return authored_rom(code, handler)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT/"target/release/mdx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    cases = [{"wide": wide, "form": form, "nops": nops}
             for wide in (False, True) for form in ("move", "and", "eor", "stop", "rte")
             for nops in range(10)]
    run_matrix(args, cases, program, lambda data: int.from_bytes(data[0xf000:0xf002], "big"))
