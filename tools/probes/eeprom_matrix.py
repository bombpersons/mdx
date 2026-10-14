#!/usr/bin/env python3
"""Bit-bang the header-declared (Sega board, X24C01) serial EEPROM at $200001:
write bytes, read them back sequentially, and record every SDA level read."""
import argparse
from pathlib import Path
from timing_matrix import ROOT, authored_rom, run_matrix

SET = lambda sda, scl: [0x13FC, (scl << 1) | sda, 0x0020, 0x0001]     # move.b #v,$200001
READ = [0x10F9, 0x0020, 0x0001]                                     # move.b $200001,(a0)+


def build(case):
    code = [0x41F9, 0x00FF, 0xF000]                                 # lea $FFF000,a0
    def start(): return SET(1, 1) + SET(0, 1) + SET(0, 0)
    def stop(): return SET(0, 0) + SET(0, 1) + SET(1, 1)
    def send(byte):
        out = []
        for i in range(7, -1, -1):
            b = byte >> i & 1
            out += SET(b, 0) + SET(b, 1) + SET(b, 0)
        return out + SET(1, 0) + SET(1, 1) + READ + SET(1, 0)        # ACK slot
    def receive(last):
        out = []
        for _ in range(8):
            out += SET(1, 1) + READ + SET(1, 0)
        a = 1 if last else 0
        return out + SET(a, 0) + SET(a, 1) + SET(a, 0)
    address = case['address']
    code += start() + send(address << 1)
    for value in case['values']:
        code += send(value)
    code += stop()
    code += start() + send(address << 1 | 1)
    for i in range(len(case['values']) + 1):
        code += receive(i == len(case['values']))
    code += stop()
    code += [0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]
    return code


def program(case):
    rom = bytearray(authored_rom(case['code']))
    rom[0x1B0:0x1BC] = b'RA\xe8\x40' + bytes.fromhex('0020000100200001')
    return bytes(rom)


def extract(data):
    return {'sda': data[0xF000:0xF040].hex()}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT / 'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--fixture', type=Path)
    args = p.parse_args()
    cases = [{'address': 3, 'values': [0xA5, 0x3C]}, {'address': 0x7E, 'values': [0x01, 0x80]}]
    cases = [{**case, 'code': build(case)} for case in cases]
    run_matrix(args, cases, program, extract)
