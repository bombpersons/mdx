#!/usr/bin/env python3
"""Observe STOP wakeup through physical VINT and masked idle device progression."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code=[0x33fc,0x8124,0x00c0,0x0004]
    if case['wide']:code += [0x33fc,0x8c81,0x00c0,0x0004]
    for i in range(32):
        code += [0x4e72,0x2001,0x3039,0x00c0,0x0008,0x33c0,0x00ff,0xf000+i*2]+[0x4e71]*13
    code += [0x33fc,0xc0de,0x00ff,0xf040,0x60fe]
    return authored_rom(code,[0x4e73])


def extract(data):return {'hv':list(struct.unpack('>32H',data[0xf000:0xf040]))}


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--fixture',type=Path)
    args=p.parse_args();args.frames=40
    run_matrix(args,[{'wide':False},{'wide':True}],program,extract)
