#!/usr/bin/env python3
"""Observe TAS flags, operand write-back and HV phase over every alterable mode."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix

# mode name -> (A0 setup address, TAS opcode words)
MODES={'d':(0xffe000,[0x4ac1]),'ai':(0xffe000,[0x4ad0]),'pi':(0xffe000,[0x4ad8]),
       'pd':(0xffe001,[0x4ae0]),'di':(0xffdff0,[0x4ae8,0x0010]),'ix':(0xffdff0,[0x4af0,0x2000]),
       'aw':(0xffe000,[0x4af8,0xe000]),'al':(0xffe000,[0x4af9,0x00ff,0xe000])}


def program(case):
    base,tas=MODES[case['mode']];value=case['value']
    code=[0x13fc,value,0x00ff,0xe000,0x223c,0x5555,0x5500|value]
    code += [0x41f9,base>>16,base&65535,0x7410,0x44fc,0x000f]+tas
    code += [0x40f9,0x00ff,0xf000,0x23c1,0x00ff,0xf002,0x23c8,0x00ff,0xf006]
    code += tas*case['repeat']
    code += [0x3039,0x00c0,0x0008,0x33c0,0x00ff,0xf00a]
    code += [0x33fc,0xc0de,0x00ff,0xf040,0x60fe]
    return authored_rom(code)


def extract(data):
    return {'operand':data[0xe000],'sr':struct.unpack('>H',data[0xf000:0xf002])[0],
            'd1':data[0xf002:0xf006].hex(),'a0':data[0xf006:0xf00a].hex(),
            'hv':struct.unpack('>H',data[0xf00a:0xf00c])[0]}


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--fixture',type=Path)
    args=p.parse_args()
    cases=[{'mode':m,'value':v,'repeat':0} for m in MODES for v in (0x00,0x05,0x85)]
    cases+=[{'mode':m,'value':5,'repeat':r} for m in MODES for r in (8,31)]
    run_matrix(args,cases,program,extract)
