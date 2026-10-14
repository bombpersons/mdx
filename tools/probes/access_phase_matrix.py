#!/usr/bin/env python3
"""Access-phase observations: VDP status sampled at dispatch plus the opcode's
table cost (HBLANK edge), the one-instruction interrupt delay after a register
write enables a pending VINT, and YM2612 BUSY seen through 68000 Z80-bus
latency with frame-relative rounding, the same through Z80 accesses
(dispatch plus opcode T-states), HINT counter sequencing at line 0 and the
VBLANK edge, and a DMA fill started so late in a line that the reference's
first-update budget wraps (whole fill at once, BUSY to the frame edge), and
3-/6-button pad reads (header-selected type, TH pulse counter and its
timeout, TH input switching latency), and control-port commands (a register
write retargets data writes to the invalid code 2; A15-A14 latch)."""
import argparse
from pathlib import Path
from timing_matrix import ROOT, authored_rom, run_matrix

LEA_A0_VDP = [0x41F9, 0x00C0, 0x0004]
LEA_A1_OUT = [0x43F9, 0x00FF, 0xF000]
NOP = 0x4E71
MARK = [0x33FC, 0xC0DE, 0x00FF, 0xF040, 0x60FE]


def wait_status(bit, set_):
    # move.w (a0),d0 ; btst #bit,d0 ; beq/bne back
    return [0x3010, 0x0800, bit, 0x67F8 if set_ else 0x66F8]


def status_case(case):
    code = LEA_A0_VDP + LEA_A1_OUT
    code += [0x30BC, 0x8C81 if case['h40'] else 0x8C00, 0x30BC, 0x8144]
    code += wait_status(3, True) + wait_status(3, False)
    code += [NOP] * case['pad']
    read = {'move_an': [0x32D0], 'move_abs': [0x32F9, 0x00C0, 0x0004],
            'move_d16': [0x32E8, 0x0000]}[case['form']]
    code += read * 24
    return code + MARK


def irq_case(case):
    code = LEA_A0_VDP + LEA_A1_OUT
    code += [0x30BC, 0x8144] + wait_status(3, True) + wait_status(7, True)
    code += [0x46FC, 0x2000]                       # unmask; VDP VINT still disabled
    code += [NOP] * case['pad']
    if case['write'] == 'move_w':
        code += [0x30BC, 0x8164]
    else:
        code += [0x20BC, 0x8164, 0x8F02]           # move.l #$81648F02,(a0)
    code += {'mask': [0x46FC, 0x2700], 'nop': [NOP]}[case['next']]
    code += [NOP, NOP, 0x46FC, 0x2700]
    return code + MARK


HANDLER = [0x23EF, 0x0002, 0x00FF, 0xF020,         # move.l 2(sp),$FFF020
           0x5278, 0xF030,                         # addq.w #1,$FFF030.w
           0x4E73]


def fm_case(case):
    code = LEA_A1_OUT + [0x45F9, 0x00A0, 0x4000]   # lea $A04000,a2
    code += [0x33FC, 0x0100, 0x00A1, 0x1200,       # release Z80 reset
             0x33FC, 0x0100, 0x00A1, 0x1100,       # request bus
             0x0839, 0x0000, 0x00A1, 0x1100, 0x66F6]
    code += [NOP] * case['pad']
    code += [0x14BC, 0x002B]                       # move.b #$2B,(a2)
    code += {'an': [0x157C, 0x0000, 0x0001],       # move.b #0,1(a2): data write
             'abs': [0x13FC, 0x0000, 0x00A0, 0x4001]}[case['write']]
    read = {'an': [0x12D2], 'abs': [0x12F9, 0x00A0, 0x4000]}[case['read']]
    code += read * 24                              # move.b status,(a1)+
    return code + MARK


HINT_HANDLER = [0xB3FC, 0x00FF, 0xF040,            # cmpa.l #$FFF040,a1
                0x6406,                            # bcc.s past the sample
                0x32F9, 0x00C0, 0x0008,            # move.w HV,(a1)+
                0x4E73]


def hint_case(case):
    code = LEA_A0_VDP + LEA_A1_OUT
    code += [0x30BC, 0x8144, 0x30BC, 0x8A00 | case['period']]
    code += [0x1039, 0x00C0, 0x0008, 0x0C00, case['start'], 0x66F4]  # wait V
    code += [0x30BC, 0x8014, 0x46FC, 0x2000]       # HINT on, unmask
    code += [0xB3FC, 0x00FF, 0xF040, 0x65F8]       # until 32 samples
    code += [0x46FC, 0x2700]
    return code + MARK


def fill_case(case):
    code = LEA_A0_VDP + LEA_A1_OUT
    code += [0x30BC, 0x8154, 0x30BC, 0x8C81, 0x30BC, 0x8F01,
             0x30BC, 0x9300 | case['length'] & 0xFF, 0x30BC, 0x9400 | case['length'] >> 8,
             0x30BC, 0x9780]
    code += wait_status(3, True) + wait_status(3, False)   # line 0
    code += wait_status(2, False) + wait_status(2, True)   # HBLANK rise
    code += [0x343C, case['delay'], 0x51CA, 0xFFFE]        # move.w #n,d2; dbra
    code += [NOP] * case['pad']
    code += [0x20BC, 0x4000, 0x0080]               # VRAM write + DMA
    code += [0x33FC, 0x0000, 0x00C0, 0x0000]       # fill data: starts the fill
    code += [0x7200, 0x5241, 0x3010, 0x0800, 0x0001, 0x66F6]  # count BUSY polls
    code += [0x32C1, 0x32F9, 0x00C0, 0x0008]       # count, HV at clear
    return code + MARK


def z80fm_case(case):
    z80 = [0x11, 0x00, 0x10] + [0x00] * case['pad']
    z80 += [0x3E, 0x2B, 0x32, 0x00, 0x40, 0xAF, 0x32, 0x01, 0x40]
    z80 += [0x3A, 0x00, 0x40, 0x12, 0x13] * 24
    z80 += [0x3E, 0x01, 0x32, 0x00, 0x1F, 0x18, 0xFE]
    code = [0x33FC, 0x0100, 0x00A1, 0x1100,        # request bus
            0x33FC, 0x0100, 0x00A1, 0x1200,        # release reset
            0x0839, 0x0000, 0x00A1, 0x1100, 0x66F6,
            0x45F9, 0x00A0, 0x0000]                # lea $A00000,a2
    code += [w for b in z80 for w in (0x14FC, b)]  # move.b #b,(a2)+
    code += [0x33FC, 0x0000, 0x00A1, 0x1200,       # reset, release bus, run
             0x33FC, 0x0000, 0x00A1, 0x1100,
             0x33FC, 0x0100, 0x00A1, 0x1200,
             0x343C, 0x1000, 0x51CA, 0xFFFE,       # wait
             0x33FC, 0x0100, 0x00A1, 0x1100,
             0x0839, 0x0000, 0x00A1, 0x1100, 0x66F6]
    code += LEA_A1_OUT + [0x45F9, 0x00A0, 0x1000, 0x7217, 0x12DA, 0x51C9, 0xFFFC]
    code += [0x13F9, 0x00A0, 0x1F00, 0x00FF, 0xF03F]  # completion flag
    return code + MARK


def pad_case(case):
    code = LEA_A1_OUT + [0x45F9, 0x00A1, 0x0003]   # lea $A10003,a2
    code += [0x13FC, 0x0040, 0x00A1, 0x0009]       # TH is an output
    cycle = [0x14BC, 0x0040, 0x12D2, 0x14BC, 0x0000, 0x12D2]
    code += cycle * 4
    code += [0x343C, case['delay'], 0x51CA, 0xFFFE]
    code += cycle * 2
    code += [0x14BC, 0x0000, 0x13FC, 0x0000, 0x00A1, 0x0009]  # TH low, then input
    code += [0x12D2] * 4
    return code + MARK


def ctrl_case(case):
    code = LEA_A0_VDP + LEA_A1_OUT + [0x30BC, 0x8104, 0x30BC, 0x8F02]
    data = lambda value: [0x33FC, value, 0x00C0, 0x0000]
    read = [0x32F9, 0x00C0, 0x0000]                # move.w $C00000,(a1)+
    if case['form'] == 'register':
        code += [0x20BC, 0x5000, 0x0000] + data(0x1111)
        code += [0x30BC, 0x8F02] + data(0x2222) + data(0x3333)
        code += [0x20BC, 0x1000, 0x0000] + read * 3
    else:
        code += [0x20BC, 0x7FFC, 0x0000] + data(0xAAAA) + data(0xBBBB) + data(0xCCCC)
        code += [0x30BC, 0x4010] + data(0xDDDD)    # first word only
        code += [0x20BC, 0x0010, 0x0000] + read
        code += [0x20BC, 0x0010, 0x0001] + read
        code += [0x20BC, 0x3FFC, 0x0000] + read * 3
    return code + MARK


def program(case):
    if case['kind'] == 'pad':
        rom = bytearray(authored_rom(case['code']))
        rom[0x190:0x192] = b'J6' if case['six'] else b'J '
        return bytes(rom)
    if case['kind'] == 'hint':
        rom = bytearray(authored_rom(case['code'], HINT_HANDLER))
        rom[0x70:0x74] = rom[0x78:0x7c]
        rom[0x78:0x7c] = bytes(4)
        return bytes(rom)
    return authored_rom(case['code'], HANDLER if case['kind'] == 'irq' else None)


def extract(data):
    return {'out': data[0xF000:0xF040].hex()}


def cases():
    rows = []
    for h40 in (False, True):
        for form in ('move_an', 'move_abs', 'move_d16'):
            for pad in range(0, 8):
                rows.append({'kind': 'status', 'h40': h40, 'form': form, 'pad': pad})
    for write in ('move_w', 'move_l'):
        for nxt in ('mask', 'nop'):
            for pad in (0, 1):
                rows.append({'kind': 'irq', 'write': write, 'next': nxt, 'pad': pad})
    for write in ('an', 'abs'):
        for read in ('an', 'abs'):
            for pad in range(0, 12):
                rows.append({'kind': 'fm', 'write': write, 'read': read, 'pad': pad})
    for period in (0, 1, 3):
        for start in (0xC8, 0xD8):
            rows.append({'kind': 'hint', 'period': period, 'start': start})
    for pad in range(0, 24):
        rows.append({'kind': 'fill', 'length': 0x0400, 'delay': 31, 'pad': pad})
    for pad in range(0, 12):
        rows.append({'kind': 'z80fm', 'pad': pad})
    for six in (False, True):
        for delay in (0, 1200, 1400):
            for buttons in ([], ['start', 'a', 'c', 'up', 'left']):
                rows.append({'kind': 'pad', 'six': six, 'delay': delay,
                             'replay': [{'frame': 0, 'buttons': buttons}] if buttons else []})
    for form in ('register', 'latch'):
        rows.append({'kind': 'ctrl', 'form': form})
    builders = {'ctrl': ctrl_case, 'pad': pad_case, 'status': status_case, 'irq': irq_case, 'fm': fm_case, 'hint': hint_case,
                'fill': fill_case, 'z80fm': z80fm_case}
    for row in rows:
        row['code'] = builders[row['kind']](row)
    return rows


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, default=ROOT / 'target/release/mdx')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--fixture', type=Path)
    args = p.parse_args()
    run_matrix(args, cases(), program, extract)
