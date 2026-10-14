#!/usr/bin/env python3
"""Observe PAL HV counter and VDP status across one frame (European header)."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code = [0x33FC, case['mode'], 0x00C0, 0x0004]                 # VDP register 1
    for i in range(32):
        code += [0x3239, 0x00C0, 0x0004, 0x33C1, 0x00FF, 0xF100 + i * 2]   # status
        code += [0x3039, 0x00C0, 0x0008, 0x33C0, 0x00FF, 0xF000 + i * 2]   # HV
        code += [0x323C, case['delay'], 0x51C9, 0xFFFE]                   # spread samples
    code += [0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]
    rom = bytearray(authored_rom(code))
    rom[0x1F0:0x1F3] = b'E  '
    return bytes(rom)


def extract(data):
    return {'hv': list(struct.unpack('>32H', data[0xF000:0xF040])),
            'status': list(struct.unpack('>32H', data[0xF100:0xF140]))}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT / 'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--fixture', type=Path)
    args = p.parse_args()
    args.frames = 4
    cases = [{'mode': m, 'delay': d} for m in (0x8144, 0x814C) for d in (470, 473)]
    run_matrix(args, cases, program, extract)
