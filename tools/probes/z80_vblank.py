#!/usr/bin/env python3
"""Independent finite-INT regression on authored IM0/IM1/IM2 cartridges."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
from hv_counter_phase import ROOT, make_rom


def program(mode, wide):
    rom=bytearray(make_rom(True,wide)[0])
    driver=bytearray(0x101)
    driver[:10]=bytes([0xf3,0x31,0xfd,0x1f,0xed,[0x46,0x56,0x5e][mode],0xfb,0x76,0x18,0xfd])
    driver[0x38:0x43]=bytes([0x21,0x80,0x01,0x34,0x06,0x00,0x10,0xfe,0xfb,0xed,0x4d])
    driver[0xff:0x101]=bytes([0x38,0])
    words=[]
    def move_word(value, address):words.extend([0x33fc,value,address>>16,address&65535])
    def move_byte(value, address):words.extend([0x13fc,value,address>>16,address&65535])
    move_word(0x8104,0xc00004) # Mode5, 68k VINT disabled.
    move_word(0x8c81 if wide else 0x8c00,0xc00004)
    move_word(0x100,0xa11100) # BUSREQ
    move_word(0x100,0xa11200) # release RESET, grant upload access
    for index,value in enumerate(driver):move_byte(value,0xa00000+index)
    move_word(0,0xa11200)
    move_word(0x100,0xa11200)
    move_word(0,0xa11100)
    move_word(0xc0de,0xfff040)
    words.append(0x60fe)
    rom[0x200:0x200+2*len(words)]=struct.pack('>'+'H'*len(words),*words)
    rom[0x18e:0x190]=struct.pack('>H',sum(struct.unpack('>32512H',rom[0x200:]))&65535)
    return bytes(rom)


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    ap.add_argument('--out',type=Path,required=True)
    ap.add_argument('--fixture',type=Path)
    args=ap.parse_args();args.out=args.out.resolve();args.out.mkdir(parents=True,exist_ok=False)
    core=ROOT/'cache/oracle/Genesis-Plus-GX/genesis_plus_gx_libretro.so'
    env={k:v for k,v in os.environ.items() if not k.startswith('MDX_ORACLE_TRACE')};env['MDX_ORACLE_CORE']=str(core)
    rows=[]
    for mode in range(3):
        for wide in [False,True]:
            folder=args.out/f'im{mode}-h{40 if wide else 32}';folder.mkdir()
            rom=folder/'probe.md';rom.write_bytes(program(mode,wide))
            for frames in [1,2,3,6]:
                state=folder/f'oracle-{frames}.state'
                subprocess.run([sys.executable,str(ROOT/'tools/oracle_gpgx.py'),str(rom),'--frames',str(frames),'--state',str(state),'--out',str(folder/f'oracle-{frames}.png')],env=env,cwd=ROOT,check=True,capture_output=True)
                raw=state.read_bytes();assert raw[:16].rstrip(b'\0')==b'GENPLUS-GX 1.7.6'
                expected=raw[16+65536+0x180] # state_save: header, WRAM, physical ZRAM
                output=subprocess.run([str(args.binary.resolve()),'boot','--rom',str(rom),'--frames',str(frames),'--instructions','1000000','--trace','0'],cwd=ROOT,check=True,capture_output=True)
                report=json.loads(output.stdout);assert report['data']['frame_target_reached']
                actual=report['data']['vdp']['z80_head'][0x180]
                wram=Path(report['data']['memory_artifacts']['wram']['path']).read_bytes();assert wram[0xf040:0xf042]==b'\xc0\xde'
                assert raw[16+0xf040:16+0xf042]==b'\xde\xc0'
                rows.append({'mode':mode,'wide':wide,'frames':frames,'expected_counter':expected,'actual_counter':actual,'equal':actual==expected,'rom_sha256':hashlib.sha256(rom.read_bytes()).hexdigest()})
    report={'schema':'mdx.z80-vblank.v1','binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),'oracle_core_sha256':hashlib.sha256(core.read_bytes()).hexdigest(),'oracle_system_sha256':hashlib.sha256((core.parent/'core/system.c').read_bytes()).hexdigest(),'oracle_z80_sha256':hashlib.sha256((core.parent/'core/z80/z80.c').read_bytes()).hexdigest(),'cases':rows,'equal':sum(r['equal'] for r in rows),'total':len(rows)}
    (args.out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    if args.fixture:
        fixture={k:v for k,v in report.items() if k!='binary_sha256'};fixture['cases']=[{k:v for k,v in row.items() if k not in ['equal','actual_counter']} for row in rows]
        args.fixture.write_text(json.dumps(fixture,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k!='cases'}))
    assert report['equal']==report['total'],rows


if __name__=='__main__':main()
