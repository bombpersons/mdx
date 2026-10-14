#!/usr/bin/env python3
"""Observe cartridge backup RAM at $200000 and the $A130F1 switch/write protect."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code = [0x3039, 0x0020, 0x0000, 0x33C0, 0x00FF, 0xF000]            # read before writes
    code += [0x13FC, 0x0042, 0x0020, 0x0001, 0x13FC, 0x0043, 0x0020, 0x0000]
    code += [0x3039, 0x0020, 0x0000, 0x33C0, 0x00FF, 0xF002]           # after writes
    code += [0x13FC, 0x0001, 0x00A1, 0x30F1]                           # map SRAM
    code += [0x13FC, 0x0044, 0x0020, 0x0001]
    code += [0x3039, 0x0020, 0x0000, 0x33C0, 0x00FF, 0xF004]
    code += [0x13FC, 0x0003, 0x00A1, 0x30F1]                           # write-protect
    code += [0x13FC, 0x0055, 0x0020, 0x0001]
    code += [0x3039, 0x0020, 0x0000, 0x33C0, 0x00FF, 0xF006]
    code += [0x13FC, 0x0000, 0x00A1, 0x30F1]                           # back to ROM
    code += [0x3039, 0x0020, 0x0000, 0x33C0, 0x00FF, 0xF008]
    code += [0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]
    rom = bytearray(authored_rom(code))
    rom.extend(b'\x6c' * (case['length'] - len(rom)))
    if case['header']:
        rom[0x1B0:0x1BC] = b'RA\xf8\x20' + struct.pack('>II', 0x200001, 0x203FFF)
    return bytes(rom)


def extract(data):
    return {'words': list(struct.unpack('>5H', data[0xF000:0xF00A]))}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT / 'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--fixture', type=Path)
    args = p.parse_args()
    cases = [{'length': n, 'header': h} for n in (0x80000, 0x400000) for h in (False, True)]
    run_matrix(args, cases, program, extract)
