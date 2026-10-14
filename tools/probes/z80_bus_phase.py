#!/usr/bin/env python3
"""Completion-checked two-CPU bus-wait probes against the unchanged GPGX core."""
import argparse
from pathlib import Path
import struct
from timing_matrix import authored_rom,run_matrix
from hv_counter_phase import ROOT


def words(case):
    code=[0x4e71]*case['nops']
    def move(value,address):code.extend([0x33fc,value,address>>16,address&65535])
    move(0x8104,0xc00004);move(0x8c81 if case['wide'] else 0x8c00,0xc00004)
    move(0x100,0xa11100);move(0x100,0xa11200)
    kind=case['kind'];address={'psg-bank':0x8011,'psg-direct':0x7f11,'rom-read':0x8000,'ram':0x180,'fm':0x4000}[kind]
    operation=0x3a if kind=='rom-read' else 0x32
    driver=[0x3e,0x81,operation,address&255,address>>8]
    if case.get('driver_delay'):
        driver.extend([0x06,case['driver_delay'],0x00,0x10,0xfd])
    driver.extend([0x3e,0,operation,address&255,address>>8])
    driver.extend([0x18,0xf4] if case.get('burst') else [0x76])
    if case.get('block_move'):
        driver=[0x01,2,0,0x11,0x11,0x80,0x21,0x80,0x01]
        if case.get('index_prefix'):
            driver.append(0xdd)
        driver.extend([0xed,0xb0,0x76])
    for index,value in enumerate(driver):code.extend([0x13fc,value,0xa0,index])
    bank=0 if kind=='rom-read' else 0x180
    for bit in range(9):move(((bank>>bit)&1)<<8,0xa06000)
    move(0,0xa11200);move(0x100,0xa11200);move(0,0xa11100)
    if case['dma_words']:
        count=case['dma_words']
        for register,value in [(1,0x14),(15,2),(19,count&255),(20,count>>8),(21,0),(22,8),(23,0)]:
            move(0x8000|(register<<8)|value,0xc00004)
        code.extend([0x23fc,0x4000,0x0080,0x00c0,0x0004])
    if case.get('burst'):
        code.extend([0x3e3c,0x3fff,0x51cf,0xfffe])
    for index in range(32):
        code.extend([0x3039,0x00c0,0x0008,0x33c0,0x00ff,0xf000+index*2])
        code.extend([0x4e71]*13)
    move(0xc0de,0xfff040);code.append(0x60fe)
    return code


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=ROOT/'target/release/mdx')
    ap.add_argument('--out',type=Path,required=True)
    ap.add_argument('--fixture',type=Path)
    ap.add_argument('--dma-overshoot',action='store_true',help='delayed second access around DMA update boundaries')
    ap.add_argument('--block-move',action='store_true',help='shared-bus LDIR with and without an ignored DD prefix')
    args=ap.parse_args()
    cases=[]
    if args.block_move:
        for wide in [False,True]:
            for nops in range(7):
                for index_prefix in [False,True]:
                    case=dict(wide=wide,nops=nops,kind='psg-bank',dma_words=0,
                              block_move=True,index_prefix=index_prefix)
                    case['program_words']=words(case);cases.append(case)
        run_matrix(args,cases,lambda case:authored_rom(case['program_words']),
                   lambda raw:list(struct.unpack('>32H',raw[0xf000:0xf040])))
        return
    if args.dma_overshoot:
        for wide in [False,True]:
            for nops in range(49):
                case=dict(wide=wide,nops=nops,kind='psg-bank',dma_words=256,driver_delay=13)
                case['program_words']=words(case);cases.append(case)
        run_matrix(args,cases,lambda case:authored_rom(case['program_words']),
                   lambda raw:list(struct.unpack('>32H',raw[0xf000:0xf040])))
        return
    for wide in [False,True]:
        for nops in range(7):
            for kind in ['psg-bank','psg-direct','rom-read','ram','fm']:
                for dma_words in [0,8,256]:
                    case=dict(wide=wide,nops=nops,kind=kind,dma_words=dma_words)
                    case['program_words']=words(case);cases.append(case)
    for wide in [False,True]:
        for nops in range(7):
            case=dict(wide=wide,nops=nops,kind='psg-bank',dma_words=0,burst=True)
            case['program_words']=words(case);cases.append(case)
    run_matrix(args,cases,lambda case:authored_rom(case['program_words']),
               lambda raw:list(struct.unpack('>32H',raw[0xf000:0xf040])))


if __name__=='__main__':main()
