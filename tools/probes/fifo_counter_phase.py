#!/usr/bin/env python3
"""Exercise FIFO-full waits in seven NTSC frame-relative CPU clock phases."""
import argparse
from pathlib import Path
import struct
from timing_matrix import ROOT, authored_rom, run_matrix


def program(case):
    code = [0x33fc, 0x8104, 0x00c0, 0x0004]
    if case["wide"]:
        code += [0x33fc, 0x8c81, 0x00c0, 0x0004]
    # Land well inside active video after several complete scheduler frames.
    loops = (case["frame"]*896040 + 138*3420 - 784462)//70
    while loops:
        chunk = min(loops, 65536)
        code += [0x3c3c, chunk-1, 0x51ce, 0xfffe]
        loops -= chunk
    code += [0x33fc, 0x8144, 0x00c0, 0x0004,
             0x23fc, {"vram": 0x4000, "cram": 0xc000, "vsram": 0x4000}[case["target"]],
             0x0010 if case["target"] == "vsram" else 0, 0x00c0, 0x0004]
    for sample in range(32):
        for _ in range(8):
            code += [0x33fc, 0, 0x00c0, 0]
        code += [0x3039, 0x00c0, 0x0008, 0x33c0, 0x00ff, 0xf000+2*sample]
        code += [0x4e71]*13
    code += [0x33fc, 0xc0de, 0x00ff, 0xf040, 0x60fe]
    return authored_rom(code)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT/"target/release/mdx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    args.frames = 10
    cases = [{"wide": wide, "target": target, "frame": frame}
             for wide in (False, True) for target in ("vram", "cram", "vsram") for frame in range(1,8)]
    # Frames7/8 are needed for the final cases, unlike the short default matrix.
    run_matrix(args, cases, program, lambda data: list(struct.unpack(">32H", data[0xf000:0xf040])))
