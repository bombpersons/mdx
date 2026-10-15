#!/usr/bin/env python3
"""Generate crates/mdcore/src/cpu/legal.bin from the reference 68000 core.

GPGX's Musashi build fills its 65536-entry jump table with m68k_op_illegal,
then each m68k_opcode_handler_table entry claims the opcodes matching its
(mask, match) pair, later entries winning. Bit n of the output (LSB first)
is set when opcode n has a real handler (not illegal, line A or line F).
"""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
source = root / "cache/oracle/Genesis-Plus-GX/core/m68k/m68kops.h"
text = source.read_text()
table = text[text.index("m68k_opcode_handler_table[] ="):]
entries = re.findall(r"\{(m68k_op_\w+)\s*,\s*0x([0-9a-f]+),\s*0x([0-9a-f]+),", table)
legal = bytearray(65536)
for name, mask, match in entries:
    mask, match = int(mask, 16), int(match, 16)
    value = name not in ("m68k_op_illegal", "m68k_op_1010", "m68k_op_1111")
    for op in range(65536):
        if op & mask == match:
            legal[op] = value
bits = bytearray(8192)
for op in range(65536):
    if legal[op]:
        bits[op >> 3] |= 1 << (op & 7)
out = root / "crates/mdcore/src/cpu/legal.bin"
out.write_bytes(bytes(bits))
print(f"{len(entries)} handlers, {sum(legal)} legal opcodes -> {out}", file=sys.stderr)
