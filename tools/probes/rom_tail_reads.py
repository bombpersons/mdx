#!/usr/bin/env python3
"""Read undumped tails and mirrors of authored irregular-sized linear ROMs."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def addresses(length):
    period=1<<(length-1).bit_length()
    return [0,2,length-2,length,length+2,period-2,period,period+2]


def program(case):
    length=case['length'];code=[]
    for i,address in enumerate(addresses(length)):
        code += [0x3039,address>>16,address&65535,0x33c0,0x00ff,0xf000+i*2]
    code += [0x33fc,0xc0de,0x00ff,0xf040,0x60fe]
    rom=bytearray(authored_rom(code));rom.extend(b'\x6c'*(length-len(rom)))
    rom[-2:]=b'\x12\x34';rom[0x1a4:0x1a8]=struct.pack('>I',length-1)
    checksum=sum(struct.unpack('>'+str((length-0x200)//2)+'H',rom[0x200:]))&65535
    rom[0x18e:0x190]=struct.pack('>H',checksum)
    return bytes(rom)


def extract(data):return {'words':list(struct.unpack('>8H',data[0xf000:0xf010]))}


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--fixture',type=Path)
    args=p.parse_args();run_matrix(args,[{'length':n} for n in (196610,262146,1048580,3145728)],program,extract)
