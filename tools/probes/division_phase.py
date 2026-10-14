#!/usr/bin/env python3
"""Measure DIVU/DIVS operand/EA timing, quotient/remainder and condition codes."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    dividend, divisor = case["dividend"], case["divisor"]
    code = [0x33fc, 0x8104, 0x00c0, 0x0004]
    if case["wide"]:
        code += [0x33fc, 0x8c81, 0x00c0, 0x0004]
    code += [0x263c, 0, divisor, 0x2c7c, 0x00ff, 0x8000,
             0x33fc, divisor, 0x00ff, 0x8000]
    for sample in range(32):
        code += [0x243c, dividend >> 16, dividend & 65535]
        opcode = (0x85c0 if case["signed"] else 0x84c0) | {"reg": 3, "imm": 0x3c, "mem": 0x16}[case["ea"]]
        code += [opcode] + ([divisor] if case["ea"] == "imm" else [])
        code += [0x40c4, 0x33c4, 0x00ff, 0xf200+sample*2,
                 0x23c2, 0x00ff, 0xf100+sample*4,
                 0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000+sample*2]
        code += [0x4e71]*13
    code += [0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]
    return authored_rom(code)


def extract(data):
    return {"hv": list(struct.unpack(">32H", data[0xf000:0xf040])),
            "results": list(struct.unpack(">32I", data[0xf100:0xf180])),
            "sr": list(struct.unpack(">32H", data[0xf200:0xf240]))}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT/"target/release/mdx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    cases = []
    for wide in (False, True):
        for signed, operands in [(False, [(0,1), (1,1), (65535,1), (65536,1), (0x12345678,0xabcd), (0xffffffff,0xffff)]),
                                 (True, [(0,1), (1,1), (-1,1), (-1,-1), (0x7fff0000,1), (-0x80000000,-1), (-0x80000000,-32768), (65536,3)])]:
            for dividend, divisor in operands:
                for ea in ("reg", "imm", "mem"):
                    cases.append({"wide": wide, "signed": signed, "dividend": dividend & 0xffffffff,
                                  "divisor": divisor & 65535, "ea": ea})
    run_matrix(args, cases, program, extract)
