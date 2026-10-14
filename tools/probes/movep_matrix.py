#!/usr/bin/env python3
"""Observe MOVEP word/long transfers both ways at even/odd WRAM bases, plus HV phase."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    disp=case['displacement']
    code=[0x41f9,0x00ff,0xe000,0x223c,0x1122,0x3344]
    code += [0x03c8,disp,0x0388,disp+9]                     # movep.l/w d1 -> mem
    code += [0x243c,0xaaaa,0xaaaa,0x263c,0x5555,0x5555]
    code += [0x0508,disp,0x0748,disp+9]                     # movep.w/l mem -> d2/d3
    code += [0x23c2,0x00ff,0xf000,0x23c3,0x00ff,0xf004]
    code += [0x03c8,disp]*case['repeat']
    code += [0x3039,0x00c0,0x0008,0x33c0,0x00ff,0xf008]
    code += [0x33fc,0xc0de,0x00ff,0xf040,0x60fe]
    return authored_rom(code)


def extract(data):
    return {'transfer':data[0xe000:0xe020].hex(),'registers':data[0xf000:0xf008].hex(),
            'hv':struct.unpack('>H',data[0xf008:0xf00a])[0]}


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--fixture',type=Path)
    args=p.parse_args()
    run_matrix(args,[{'displacement':d,'repeat':r} for d in (0,1) for r in (0,8,31)],program,extract)
