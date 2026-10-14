#!/usr/bin/env python3
"""Ground NTSC HV sampling on tiny authored programs, independently of a game."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tools'))
from oracle_vdp_snapshot import normalize


def make_rom(mode,wide,samples=32,dma_words=None,shift_count=None,shift_opcode=0xe5ab,dma_target="vram",dma_display=False,movem_opcode=None,movem_mask=0):
    rom=bytearray(65536)
    rom[:8]=struct.pack('>II',0xff8000,0x200)
    rom[0x100:0x110]=b'SEGA MEGA DRIVE '
    rom[0x120:0x150]=b'HV counter phase probe'.ljust(48,b' ')
    rom[0x150:0x180]=b'HV counter phase probe'.ljust(48,b' ')
    rom[0x1a0:0x1a8]=struct.pack('>II',0,len(rom)-1)
    rom[0x1b0:0x1b8]=struct.pack('>II',0xff0000,0xffffff)
    rom[0x1f0:0x1f3]=b'U  '
    code=[]
    def word(value):code.append(value)
    def long(value):code.extend([value>>16,value&65535])
    if mode:
        word(0x33fc);word(0x8104);long(0xc00004) # MOVE.W immediate to VDP R1
    if wide:
        word(0x33fc);word(0x8c81);long(0xc00004) # standard H40 clocks
    if dma_words is not None:
        for register,value in [(1,0x54 if dma_display else 0x14),(15,2),(19,dma_words&255),(20,(dma_words>>8)&255),(21,0),(22,8),(23,0)]:
            word(0x33fc);word(0x8000|(register<<8)|value);long(0xc00004)
        word(0x23fc);long({"vram":0x40000080,"cram":0xc0000080,"vsram":0x40000090}[dma_target]);long(0xc00004) # external bus DMA
    if shift_count is not None:
        word(0x7400 | shift_count) # MOVEQ count,D2 (HV reads use D0)
        word(0x76ff)              # MOVEQ -1,D3
    if movem_opcode is not None:
        word(0x2c7c);long(0xff8000) # A6 is the transfer base, outside sample output
    sites=[]
    for index in range(samples):
        if shift_count is not None:
            word(shift_opcode)
        if movem_opcode is not None:
            word(movem_opcode);word(movem_mask)
        sites.append(0x200+2*len(code))
        word(0x3039);long(0xc00008)              # MOVE.W absolute-long HV,D0
        word(0x33c0);long(0xfff000+2*index)      # store observed word in WRAM
        code.extend([0x4e71]*13)               # public fixed-cost NOPs
    word(0x33fc);word(0xc0de);long(0xfff040)    # completion marker after all samples
    word(0x60fe)                               # BRA.B to self after sampling
    body=struct.pack('>'+'H'*len(code),*code)
    rom[0x200:0x200+len(body)]=body
    rom[0x18e:0x190]=struct.pack('>H',sum(struct.unpack('>'+'H'*((len(rom)-0x200)//2),rom[0x200:]))&65535)
    return bytes(rom),sites


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--dma-words',type=int,nargs='+',help='controlled external DMA transfer probes')
    parser.add_argument('--dma-widths',type=int,nargs='+',choices=[32,40],default=[40])
    parser.add_argument('--dma-target',choices=['vram','cram','vsram'],default='vram')
    parser.add_argument('--dma-display',action='store_true',help='DMA during enabled active display')
    parser.add_argument('--frames',type=int,default=4,help='frame budget; completion marker must be observed in both cores')
    parser.add_argument('--shift-counts',type=int,nargs='+',help='register-count shift/rotate probes, counts 0..63')
    parser.add_argument('--shift-opcode',type=lambda value:int(value,0),default=0xe5ab,help='register shift/rotate opcode using count D2, destination D3')
    parser.add_argument('--verify',action='store_true',help='fail if any completed counter samples differ')
    args=parser.parse_args()
    if args.frames<1 or args.frames>100:parser.error('--frames must be 1..100')
    if args.dma_words and args.shift_counts:parser.error('select DMA or shift profiles separately')
    if args.dma_words and any(not 1<=count<=65536 for count in args.dma_words):parser.error('DMA word count must be 1..65536')
    if args.shift_counts and any(not 0<=count<=63 for count in args.shift_counts):parser.error('shift count must be 0..63')
    if not 0<=args.shift_opcode<=0xffff or args.shift_opcode & 0xfe27 != 0xe423 or args.shift_opcode & 0xc0 == 0xc0:parser.error('shift opcode must use register count D2 and destination D3')
    args.out.mkdir(parents=True,exist_ok=True);results=[]
    profiles=[('mode4-h32',False,False,None),('mode5-h32',True,False,None),('mode5-h40',True,True,None)]
    if args.dma_words:profiles=[(f'mode5-h{width}-dma{count}-{args.dma_target}-{"active" if args.dma_display else "blank"}',True,width==40,count) for width in args.dma_widths for count in args.dma_words]
    if args.shift_counts:profiles=[(f'mode5-h40-shift{args.shift_opcode:04x}-count{count}',True,True,None) for count in args.shift_counts]
    for profile_index,(name,mode,wide,dma_words) in enumerate(profiles):
        folder=args.out/name;folder.mkdir();data,sites=make_rom(mode,wide,dma_words=dma_words,shift_count=args.shift_counts[profile_index] if args.shift_counts else None,shift_opcode=args.shift_opcode,dma_target=args.dma_target,dma_display=args.dma_display);rom=folder/'probe.md';rom.write_bytes(data)
        call=subprocess.run([str(args.binary.resolve()),'boot','--rom',str(rom),'--frames',str(args.frames),'--instructions','1000000','--trace','0','--out',str(folder/'mdx')],capture_output=True,text=True,check=True)
        report=json.loads(call.stdout);(folder/'mdx.json').write_text(call.stdout)
        subprocess.run([sys.executable,str(ROOT/'tools/oracle_gpgx.py'),str(rom),'--frames',str(args.frames),'--out',str(folder/'oracle.png'),'--state',str(folder/'oracle.state')],capture_output=True,text=True,check=True)
        snapshot=normalize(folder/'oracle.state',folder/'oracle-snapshot')
        left=Path(report['data']['memory_artifacts']['wram']['path']).read_bytes()[0xf000:0xf042]
        right=Path(snapshot['memory_artifacts']['wram']['path']).read_bytes()[0xf000:0xf042]
        completion={'mdx':left[64:66].hex(),'oracle':right[64:66].hex()}
        if left[64:66]!=b'\xc0\xde' or right[64:66]!=b'\xc0\xde':
            raise SystemExit(f'{name}: sample program incomplete: {completion}; increase --frames or inspect execution before comparing samples')
        ours=list(struct.unpack('>32H',left[:64]));oracle=list(struct.unpack('>32H',right[:64]))
        result={'profile':name,'rom_sha256':hashlib.sha256(data).hexdigest(),'sampler_pcs':[hex(pc) for pc in sites],'samples':[{'mdx':hex(a),'oracle':hex(b),'equal':a==b} for a,b in zip(ours,oracle)],'equal_samples':sum(a==b for a,b in zip(ours,oracle)),'total_samples':32,'completion':completion}
        results.append(result);print(json.dumps({'profile':name,'equal_samples':result['equal_samples'],'first':result['samples'][:8]}),flush=True)
    (args.out/'summary.json').write_text(json.dumps({'schema':'mdx.hv-phase-probe.v2','binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),'profiles':results},indent=2)+'\n')
    if args.verify and any(r['equal_samples']!=r['total_samples'] for r in results):raise SystemExit(1)


if __name__=='__main__':main()
