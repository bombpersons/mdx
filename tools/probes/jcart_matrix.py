#!/usr/bin/env python3
"""Read the J-Cart extra controller ports ($380000) with TH high and low."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code = []
    for i, th in enumerate((1, 0, 1)):
        code += [0x33FC, th, 0x0038, 0x0000]                         # TH via $380000
        code += [0x3039, 0x0038, 0x0000, 0x33C0, 0x00FF, 0xF000 + i * 4]  # word read
        code += [0x1039, 0x0038, 0x0001, 0x13C0, 0x00FF, 0xF002 + i * 4]  # odd byte read
        code += [0x1039, 0x0038, 0x0000, 0x13C0, 0x00FF, 0xF003 + i * 4]  # even byte read
    code += [0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]
    rom = bytearray(authored_rom(code))
    rom[0x180:0x18E] = b'GM T-120066-00'
    return bytes(rom)


def extract(data):
    return {'reads': data[0xF000:0xF00C].hex()}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT / 'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--fixture', type=Path)
    args = p.parse_args()
    presses = [
        [],
        [{'frame': 0, 'port': 2, 'buttons': ['up', 'a', 'start']}, {'frame': 0, 'port': 3, 'buttons': ['b', 'c', 'right']}],
        [{'frame': 0, 'port': 2, 'buttons': ['down', 'left', 'b', 'c']}, {'frame': 0, 'port': 3, 'buttons': ['up', 'a', 'start', 'left']}],
    ]
    run_matrix(args, [{'replay': r} for r in presses], program, extract)
