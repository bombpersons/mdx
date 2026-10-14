#!/usr/bin/env python3
"""Observe ADDX/SUBX/NEGX, CHK, divide-by-zero, TRAPV and LINK/UNLK A7 in GPGX.

Each case is a list of items: a CCR/register setup, one instruction, then SR,
D1, D2, A1 and A2 stored through A5. Vectors 5-7 enter a handler that records
the stacked SR and return PC through A6 and returns with RTE. Cases carry their
exact program words so the frozen fixture rebuilds identical images.
"""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix

HANDLER = [0x3cd7, 0x2cef, 0x0002, 0x4e73]  # move.w (sp),(a6)+; move.l 2(sp),(a6)+; rte
OUT = 0xff9000
VALUES = [(0x00000000, 0x00000000), (0x7fffffff, 0x00000001), (0x80000000, 0xffffffff),
          (0x12345678, 0x9abcdef0), (0x000000ff, 0x00000001), (0x00008000, 0x00008000)]
CCRS = [0x00, 0x14, 0x10, 0x04]


def long(value): return [value >> 16 & 0xffff, value & 0xffff]


def item(setup, instruction, d1=0, d2=0, ccr=0):
    words = [0x223c, *long(d1), 0x243c, *long(d2)] + setup + [0x44fc, ccr] + instruction
    return words + [0x40dd, 0x2ac1, 0x2ac2, 0x2ac9, 0x2aca]  # sr,d1,d2,a1,a2 -> (a5)+


def memory_setup(d1, d2):
    return [0x23c1, 0x00ff, 0xe00c, 0x23c2, 0x00ff, 0xe01c,
            0x43f9, 0x00ff, 0xe010, 0x45f9, 0x00ff, 0xe020]


def items(family):
    out = []
    if family in ("addx", "subx", "negx"):
        base = {"addx": 0xd501, "subx": 0x9501, "negx": 0x4002}[family]
        for size in range(3):
            for d1, d2 in VALUES:
                for ccr in CCRS:
                    out.append(item([], [base | size << 6], d1, d2, ccr))
    elif family == "xmem":
        for base in (0xd509, 0x9509):
            for size in range(3):
                for d1, d2 in VALUES[1:4]:
                    for ccr in (0x00, 0x14):
                        out.append(item(memory_setup(d1, d2), [base | size << 6, 0x2439, 0x00ff, 0xe01c], d1, d2, ccr))
        for size in range(3):  # NEGX -(A2)
            for d1, d2 in VALUES[1:4]:
                out.append(item(memory_setup(d1, d2), [0x4022 | size << 6, 0x2439, 0x00ff, 0xe01c], d1, d2, 0x10))
    elif family == "chk":
        for value, bound in [(5, 10), (0, 10), (10, 10), (11, 10), (-1, 10), (-5, -10), (5, -1), (0x7fff, 0x7fff)]:
            for ccr in (0x00, 0x1f):
                out.append(item([], [0x4382], value & 0xffff | 0x55550000, bound & 0xffff, ccr))
                out.append(item([0x33c2, 0x00ff, 0xe030, 0x41f9, 0x00ff, 0xe030], [0x4390],
                                value & 0xffff | 0x55550000, bound & 0xffff, ccr))
    elif family == "div":
        for opcode in (0x82c2, 0x83c2, 0x82d0, 0x83d0, 0x82fc, 0x83fc):
            for ccr in (0x00, 0x1f):
                setup = [0x33c2, 0x00ff, 0xe030, 0x41f9, 0x00ff, 0xe030]
                tail = [0x0000] if opcode & 0xff == 0xfc else []
                out.append(item(setup, [opcode] + tail, 0x12345678, 0, ccr))
                tail = [0x0007] if opcode & 0xff == 0xfc else []
                out.append(item(setup, [opcode] + tail, 0x12345678, 7, ccr))
    elif family == "trapv":
        for ccr in (0x00, 0x02, 0x1f, 0x1d):
            out.append(item([], [0x4e76], 1, 2, ccr))
    elif family == "link":
        # Save SP in A4, LINK/UNLK A7, record SP and the linked word, restore.
        out.append(item([0x284f], [0x4e57, 0xfff8, 0x2acf, 0x2ad7, 0x2e4c]))
        out.append(item([0x284f, 0x2f3c, 0x00ff, 0x7f00], [0x4e5f, 0x2acf, 0x2e4c]))
        out.append(item([0x284f], [0x4e57, 0x0010, 0x2acf, 0x2ad7, 0x4e5f, 0x2acf, 0x2e4c]))
    return out


def build(family):
    code = [0x4bf9, *long(OUT), 0x4df9, 0x00ff, 0xe100]  # lea OUT,a5; lea $FFE100,a6
    for words in items(family):
        code += words
    code += [0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf00a, 0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]
    return code


def program(case):
    rom = bytearray(authored_rom(case["code"]))
    rom[0x1000:0x1000 + 2 * len(HANDLER)] = struct.pack(">%dH" % len(HANDLER), *HANDLER)
    for vector in (5, 6, 7):
        rom[vector * 4:vector * 4 + 4] = struct.pack(">I", 0x1000)
    rom[0x18e:0x190] = struct.pack(">H", sum(struct.unpack(">32512H", rom[0x200:])) & 65535)
    return bytes(rom)


def extract(data):
    return {"results": data[0x9000:0xa000].hex(), "frames": data[0xe100:0xe200].hex(),
            "hv": struct.unpack(">H", data[0xf00a:0xf00c])[0]}


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", type=Path, default=ROOT / "target/release/mdx")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--fixture", type=Path)
    args = p.parse_args()
    families = ["addx", "subx", "negx", "xmem", "chk", "div", "trapv", "link"]
    run_matrix(args, [{"family": f, "code": build(f)} for f in families], program, extract)
