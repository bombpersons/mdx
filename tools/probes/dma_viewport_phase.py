#!/usr/bin/env python3
"""Completion-checked large transfers across the reset viewport/frame latch."""
import argparse
from pathlib import Path
import struct
from timing_matrix import authored_rom, run_matrix
from hv_counter_phase import ROOT


def words(case):
    code = [0x4e71] * case['nops']
    def move(value, address):
        code.extend([0x33fc, value, address >> 16, address & 65535])
    move(0x815c if case['height'] == 240 else 0x8154, 0xc00004)
    move(0x8c81 if case['wide'] else 0x8c00, 0xc00004)
    count = case['words']
    for register, value in [(15,2),(19,count & 255),(20,count >> 8),(21,0),(22,8),(23,0)]:
        move(0x8000 | (register << 8) | value, 0xc00004)
    high, low = {'vram':(0x4000,0x0080),'cram':(0xc000,0x0080),'vsram':(0x4000,0x0090)}[case['target']]
    code.extend([0x23fc, high, low, 0x00c0, 0x0004])
    for index in range(32):
        code.extend([0x3039,0x00c0,0x0008,0x33c0,0x00ff,0xf000+index*2])
        code.extend([0x4e71] * 13)
    move(0xc0de, 0xfff040)
    code.append(0x60fe)
    return code


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary', type=Path, default=ROOT/'target/release/mdx')
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--fixture', type=Path)
    args = ap.parse_args()
    args.frames = 8
    cases = []
    for height in [224,240]:
        for wide in [False,True]:
            for target in ['vram','cram','vsram']:
                for count in [16,4096,8192,16384]:
                    for nops in [0,4]:
                        case = dict(height=height,wide=wide,target=target,words=count,nops=nops)
                        case['program_words'] = words(case)
                        cases.append(case)
    run_matrix(args, cases, lambda case: authored_rom(case['program_words']),
               lambda raw: list(struct.unpack('>32H', raw[0xf000:0xf040])))


if __name__ == '__main__':
    main()
