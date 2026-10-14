#!/usr/bin/env python3
"""Observe TRAP #0..15 stack frames, user/supervisor entry, RTE and HV phase.

Authored executable only; original GPGX source/execution and game images are not
modified. The independent core supplies expected registers/frame bytes/timing.
"""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code = [0x33fc, 0x8104, 0x00c0, 0x0004]
    if case['wide']:
        code += [0x33fc, 0x8c81, 0x00c0, 0x0004]
    if case['user']:
        code += [0x207c, 0x00ff, 0x6000, 0x4e60, 0x46fc, 0x0001]
    kind=case.get('kind','trap')
    opcode={'illegal':0x4afc,'line_a':0xa123,'line_f':0xf123,'privilege_reset':0x4e70,'privilege_stop':0x4e72,'privilege_rte':0x4e73}.get(kind,0x4e40 | case.get('trap',0))
    for sample in range(32):
        code += [0x44fc, 1, opcode]
        if kind=='privilege_stop':code += [0x2001]
        code += [0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000+sample*2]
        code += [0x4e71]*13
    code += [0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]
    handler = [0x3417, 0x262f, 2, 0x280f, 0x40c5,
               0x33c2, 0x00ff, 0xf100, 0x23c3, 0x00ff, 0xf104,
               0x23c4, 0x00ff, 0xf108, 0x33c5, 0x00ff, 0xf10c]
    if kind!='trap':handler += [0x58af if kind=='privilege_stop' else 0x54af,2] # skip the faulting two-byte opcode before RTE
    handler += [0x4e73]
    rom = bytearray(authored_rom(code, handler))
    vector={'illegal':4,'line_a':10,'line_f':11,'privilege_reset':8,'privilege_stop':8,'privilege_rte':8}.get(kind,32+case.get('trap',0))
    at = vector*4
    rom[at:at+4] = struct.pack('>I', 0x1000)
    rom[0x18e:0x190] = struct.pack('>H', sum(struct.unpack('>32512H', rom[0x200:])) & 65535)
    return bytes(rom)


def extract(data):
    return {'hv': list(struct.unpack('>32H', data[0xf000:0xf040])),
            'stacked_sr': struct.unpack_from('>H', data, 0xf100)[0],
            'return_pc': struct.unpack_from('>I', data, 0xf104)[0],
            'handler_sp': struct.unpack_from('>I', data, 0xf108)[0],
            'handler_sr': struct.unpack_from('>H', data, 0xf10c)[0]}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT/'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--fixture', type=Path)
    p.add_argument('--faults', action='store_true', help='ILLEGAL, line-A/F and user RESET privilege frames')
    args = p.parse_args()
    cases = [{'wide': wide, 'user': user, 'trap': n}
             for wide in (False, True) for user in (False, True) for n in range(16)]
    if args.faults:
        cases=[{'wide':wide,'user':user,'kind':kind} for wide in (False,True) for user in (False,True) for kind in ('illegal','line_a','line_f','privilege_reset','privilege_stop','privilege_rte') if not kind.startswith('privilege') or user]
    run_matrix(args, cases, program, extract)
