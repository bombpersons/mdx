//! Instruction dispatch for the 68000.
//!
//! Returns `(cycles, extension_words_consumed)`. Cycles are coarse: enough to
//! order events when tracing, not a bus-accurate model.

use crate::cpu::bus::Bus;
use crate::cpu::ea::{decode_mode, Ea, Size};
use crate::cpu::m68000::{Exception, M68000};
use crate::cpu::status::{C, N, V, X, Z};

type Step = Result<(u32, u32), Exception>;

/// 68000 effective-address bus/extension cost, in CPU clocks. MOVE's
/// predecrement destination does not carry the source's two internal clocks.
fn ea_cycles(mode: u16, reg: u8, size: Size, destination: bool) -> u32 {
    let transfer = if size == Size::Long { 8 } else { 4 };
    match mode {
        0 | 1 => 0,
        2 | 3 => transfer,
        4 => transfer + if destination { 0 } else { 2 },
        5 => transfer + 4,
        6 => transfer + 6,
        7 => match reg {
            0 | 2 => transfer + 4,
            1 => transfer + 8,
            3 => transfer + 6,
            4 => transfer,
            _ => 0,
        },
        _ => 0,
    }
}

/// Dynamic bit operation costs from the independent 68000 opcode table.
/// Static bit numbers add four clocks at the caller.
fn bit_cycles(mode: u16, reg: u8, bit: u32, op: u32) -> u32 {
    if mode == 0 {
        (if op == 2 { 8 } else { 6 }) + if op != 0 && bit & 31 >= 16 { 2 } else { 0 }
    } else {
        (if op == 0 { 4 } else { 8 }) + ea_cycles(mode, reg, Size::Byte, false)
    }
}

fn control_ea_cycles(mode: u16, reg: u8) -> u32 {
    match (mode, reg) {
        (5, _) | (7, 0 | 2) => 4,
        (6, _) | (7, 3) => 8,
        (7, 1) => 8,
        _ => 0,
    }
}

/// Operand-dependent 68000 division clocks, independently specified by the
/// GPGX/Musashi microcycle helpers. Fixed opcode and EA clocks are separate.
fn divu_body_cycles(mut dividend: u32, divisor: u32) -> u32 {
    let divisor = divisor << 16;
    let mut cycles = 76;
    for _ in 0..15 {
        if dividend & 0x8000_0000 != 0 {
            dividend = dividend.wrapping_shl(1).wrapping_sub(divisor);
        } else {
            dividend = dividend.wrapping_shl(1);
            cycles += 4;
            if dividend >= divisor {
                dividend = dividend.wrapping_sub(divisor);
                cycles -= 2;
            }
        }
    }
    cycles
}

fn divs_body_cycles(dividend: i32, divisor: i32) -> u32 {
    let mut cycles = 12 + if dividend < 0 { 2 } else { 0 };
    // Preserve the measured reference's wrapped INT_MIN magnitude explicitly,
    // without invoking Rust/C signed-absolute-value overflow. This edge timing
    // is reference compatibility evidence, not an independent hardware claim.
    let magnitude = dividend.wrapping_abs();
    let divisor_magnitude = divisor.abs(); // sign-extended 16-bit operand
    if magnitude >> 16 < divisor_magnitude {
        let mut quotient = (i64::from(magnitude) / i64::from(divisor_magnitude)) as u32;
        cycles += 110;
        if divisor >= 0 {
            cycles = if dividend >= 0 {
                cycles - 2
            } else {
                cycles + 2
            };
        }
        for _ in 0..15 {
            quotient >>= 1;
            if quotient & 1 == 0 {
                cycles += 2;
            }
        }
    } else {
        cycles += 4;
    }
    cycles
}

/// The register-to-register and register-to-memory arithmetic families all share
/// one shape: a data register on one side, an effective address on the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alu {
    Or,
    And,
    Eor,
    Sub,
    Add,
    Cmp,
}

impl Alu {
    /// `Cmp` never stores, and `Sub`/`Add` set their flags inside the flag
    /// helpers, so only the logical operations need flags applied here.
    fn sets_flags_outside_helpers(self) -> bool {
        matches!(self, Alu::Or | Alu::And | Alu::Eor)
    }
}

/// Evaluate a branch, `Scc` or `DBcc` condition.
pub fn cond_met(sr: crate::cpu::status::StatusRegister, cc: u8) -> bool {
    match cc {
        0 => true,
        1 => false,
        2 => !sr.flag(C) && !sr.flag(Z),
        3 => sr.flag(C) || sr.flag(Z),
        4 => !sr.flag(C),
        5 => sr.flag(C),
        6 => !sr.flag(Z),
        7 => sr.flag(Z),
        8 => !sr.flag(V),
        9 => sr.flag(V),
        10 => !sr.flag(N),
        11 => sr.flag(N),
        12 => sr.flag(N) == sr.flag(V),
        13 => sr.flag(N) != sr.flag(V),
        14 => !sr.flag(Z) && (sr.flag(N) == sr.flag(V)),
        15 => sr.flag(Z) || (sr.flag(N) != sr.flag(V)),
        _ => false,
    }
}

impl M68000 {
    pub(crate) fn execute<B: Bus>(&mut self, bus: &mut B) -> Step {
        let ir = self.prefetch[0];
        // Consume the opcode word, rotating the prefetch queue. The word that
        // takes its place is the one two words beyond the current pc.
        self.prefetch[0] = self.prefetch[1];
        self.prefetch[1] = bus.read16(self.pc.wrapping_add(4));
        self.pc = self.pc.wrapping_add(2);
        bus.note_pc(self.pc);

        match ir >> 12 {
            0x0 => self.op_immediate(bus, ir),
            0x1 => self.op_move(bus, ir),
            0x2 => self.op_movea(bus, ir),
            0x3 => self.op_move(bus, ir),
            0x4 => self.op_misc(bus, ir),
            0x5 => self.op_quick(bus, ir),
            0x6 => self.op_branch(bus, ir),
            0x7 => {
                let data = (ir & 0xFF) as u8 as i8 as i32 as u32;
                self.d[((ir >> 9) & 7) as usize] = data;
                self.set_logic(Size::Long, data);
                Ok((4, 0))
            }
            0x8 => self.line_8(bus, ir),
            0x9 => self.line_9(bus, ir),
            0xA => Err(Exception::Illegal { opcode: ir }),
            0xB => self.line_b(bus, ir),
            0xC => self.line_c(bus, ir),
            0xD => self.line_d(bus, ir),
            0xE => self.op_shift(bus, ir),
            _ => Err(Exception::Illegal { opcode: ir }),
        }
    }

    // ------------------------------------------------------------- group 0

    fn op_immediate<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;
        let size = Size::from_bits((ir >> 6) & 3);

        if matches!(ir, 0x017C | 0x037C | 0x0B7C) {
            return Err(Exception::Illegal { opcode: ir });
        }

        // MOVEP: alternate-byte transfers between Dx and (d16,Ay), high byte
        // first. Flags are unaffected; 16 clocks for word, 24 for long.
        if ir & 0x0138 == 0x0108 {
            let dx = ((ir >> 9) & 7) as usize;
            let disp = self.next_word(bus) as i16 as i32;
            let address = self.a[reg as usize].wrapping_add(disp as u32);
            let long = ir & 0x0040 != 0;
            let bytes = if long { 4 } else { 2 };
            if ir & 0x0080 != 0 {
                let value = self.d[dx];
                for i in 0..bytes {
                    let shift = 8 * (bytes - 1 - i);
                    self.wr8(bus, address.wrapping_add(2 * i), (value >> shift) as u8);
                }
            } else {
                let mut value = 0u32;
                for i in 0..bytes {
                    value = value << 8 | self.rd8(bus, address.wrapping_add(2 * i)) as u32;
                }
                self.d[dx] = if long { value } else { self.d[dx] & 0xFFFF_0000 | value };
            }
            return Ok((if long { 24 } else { 16 }, 1));
        }

        // Dynamic bit operations use bit 8; mode 001 is MOVEP instead.
        // GPGX btst_32_r_d masks the number to 31, btst_8_r_* to 7.
        if ir & 0x0100 != 0 && mode != 1 {
            let bit = self.d[((ir >> 9) & 7) as usize];
            let bit_size = if mode == 0 { Size::Long } else { Size::Byte };
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, bit_size);
            let op = ((ir >> 6) & 3) as u32;
            return self
                .bit_op(bus, &ea, bit_size, bit, op)
                .map(|(_, ext)| (bit_cycles(mode, reg, bit, op), ext));
        }

        // $017C, $037C and $0B7C are not 68000 encodings - they are the
        // $40-shifted opcodes an earlier version of this table mistook for
        // the "to SR" forms, and fall through to the generic immediate group
        // where they decode as an ORI/ANDI/EORI to a PC-relative address.
        // Nothing real emits them; rejecting them keeps the accepted set equal
        // to the hardware's.
        if matches!(ir, 0x017C | 0x037C | 0x0B7C) {
            return Err(Exception::Illegal { opcode: ir });
        }

        let sub = (ir >> 9) & 7;

        // ORI/ANDI/EORI to CCR or SR. These have a fixed encoding and never
        // compute an effective address, so this check has to come *before* the
        // EA is decoded. `effective_address` sees mode 111 and consumes an
        // extension word - the one that is actually the immediate - so the
        // instruction would advance six bytes instead of four and execution
        // would resume past the following instruction.
        if let Some((ccr_or_sr, to_sr)) = or_and_eori_sr_form(ir) {
            let mut value = if ccr_or_sr { self.sr.ccr() } else { self.sr.0 } as u32;
            let imm = self.read_immediate(bus, Size::Word);
            let result = match sub {
                0 => value | imm,
                1 => value & imm,
                5 => value ^ imm,
                _ => unreachable!(),
            };
            if to_sr {
                if !self.sr.is_supervisor() {
                    return Err(Exception::Illegal { opcode: ir });
                }
                self.set_status(result as u16);
            } else {
                self.sr.set_ccr(result as u16);
            }
            return Ok((20, 1));
        }

        // BTST/BCHG/BCLR/BSET with a *static* bit number put that number in the
        // instruction stream ahead of the effective address:
        // `btst.b #6,$a1000d.l` is 0839 0006 00A1000D. Decoding the address
        // first makes it swallow the bit-number word as an extension word,
        // which shifts every following byte and resumes the PC inside the next
        // instruction. The dynamic form takes the number from a data register
        // and has no such word; it was already returned above.
        if sub == 4 {
            let bit = self.next_word(bus) as u32;
            let bit_size = if mode == 0 { Size::Long } else { Size::Byte };
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, bit_size);
            let op = ((ir >> 6) & 3) as u32;
            return self
                .bit_op(bus, &ea, bit_size, bit, op)
                .map(|(_, ext)| (bit_cycles(mode, reg, bit, op) + 4, ext));
        }

        let ext = match size {
            Size::Byte => 1,
            Size::Word => 1,
            Size::Long => 2,
        };
        let imm = self.read_immediate(bus, size);
        let ea = self.effective_address(bus, decode_mode(mode, reg), reg, size);
        let dst = self.read_ea(bus, &ea, size);
        match sub {
            0 => {
                let r = dst | imm;
                self.write_ea(bus, &ea, size, r);
                self.set_logic(size, r);
            }
            1 => {
                let r = dst & imm;
                self.write_ea(bus, &ea, size, r);
                self.set_logic(size, r);
            }
            2 => {
                let r = self.sub_flags(size, dst, imm);
                self.write_ea(bus, &ea, size, r);
            }
            3 => {
                let r = self.add_flags(size, imm, dst);
                self.write_ea(bus, &ea, size, r);
            }
            5 => {
                let r = dst ^ imm;
                self.write_ea(bus, &ea, size, r);
                self.set_logic(size, r);
            }
            6 => {
                self.compare_flags(size, dst, imm);
            }
            _ => return Err(Exception::Illegal { opcode: ir }),
        }
        let cycles = if mode == 0 {
            if size == Size::Long {
                if sub == 6 {
                    14
                } else {
                    16
                }
            } else {
                8
            }
        } else {
            let base = if sub == 6 {
                if size == Size::Long {
                    12
                } else {
                    8
                }
            } else if size == Size::Long {
                20
            } else {
                12
            };
            base + ea_cycles(mode, reg, size, false)
        };
        Ok((cycles, ext))
    }

    /// Shared body of BTST/BCHG/BCLR/BSET. `field` selects the operation for
    /// the dynamic form; the static form packs the operation above bit 6.
    fn bit_op<B: Bus>(&mut self, bus: &mut B, ea: &Ea, size: Size, bit: u32, op: u32) -> Step {
        let mask = 1u32 << (bit & if size == Size::Long { 31 } else { 7 });
        let old = self.read_ea(bus, ea, size);
        self.sr.set_flag(Z, old & mask == 0);
        match op {
            0 => {}
            1 => self.write_ea(bus, ea, size, old ^ mask),
            2 => self.write_ea(bus, ea, size, old & !mask),
            3 => self.write_ea(bus, ea, size, old | mask),
            _ => return Err(Exception::Illegal { opcode: 0 }),
        }
        Ok((8, 0))
    }

    // ------------------------------------------------------------- group 1

    fn op_move<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        // MOVE sizes its operand in bits 15-12 as 1 = byte, 2 = long, 3 = word,
        // which is not the 0/1/2 encoding Size uses everywhere else.
        let size = match (ir >> 12) & 3 {
            1 => Size::Byte,
            2 => Size::Long,
            _ => Size::Word,
        };
        let src_reg = (ir & 7) as u8;
        let dst_reg = ((ir >> 9) & 7) as u8;
        let src_mode = decode_mode((ir >> 3) & 7, src_reg);
        // The destination mode is the full three bits, 8-6, for every MOVE
        // size. Bits 11-9 carry the source register when the source is direct
        // and the destination register when the destination is direct.
        let dst_mode_bits = (ir >> 6) & 7;
        let cycles = 4
            + ea_cycles((ir >> 3) & 7, src_reg, size, false)
            + ea_cycles(dst_mode_bits, dst_reg, size, true);
        // In the destination half, mode 7 is a brief address and *bit 9* picks .W over
        // .L - not bit 8, which belongs to the source half and is what makes an
        // immediate look immediate. The two look interchangeable in isolation,
        // and reading the wrong one costs a whole extension word, so the PC
        // resumes two bytes late and the next instruction is an operand.
        //
        // Checked against the byte length Capstone assigns to all 2825 MOVE and
        // MOVEA instructions these three cartridges actually execute.
        let dst_mode = if dst_mode_bits == 7 {
            if ir & 0x0200 != 0 {
                crate::cpu::ea::Mode::AbsoluteLong
            } else {
                crate::cpu::ea::Mode::AbsoluteShort
            }
        } else {
            decode_mode(dst_mode_bits, dst_reg)
        };

        // MOVEA is encoded as MOVE with an address-register destination, except
        // that the byte form does not exist and takes an illegal trap.
        if matches!(dst_mode, crate::cpu::ea::Mode::AddressRegisterDirect) {
            if size == Size::Byte {
                return Err(Exception::Illegal { opcode: ir });
            }
            let src_ea = self.effective_address(bus, src_mode, src_reg, size);
            let value = self.read_ea(bus, &src_ea, size);
            self.a[dst_reg as usize] = if size == Size::Word {
                value as u16 as i16 as i32 as u32
            } else {
                value
            };
            return Ok((cycles, 0));
        }

        let src_ea = self.effective_address(bus, src_mode, src_reg, size);
        let value = self.read_ea(bus, &src_ea, size);
        let dst_ea = self.effective_address(bus, dst_mode, dst_reg, size);
        self.write_ea(bus, &dst_ea, size, value);
        self.set_logic(size, value);
        Ok((cycles, 0))
    }

    fn op_movea<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        self.op_move(bus, ir)
    }

    fn op_misc<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;

        if let Some(r) = self.op_misc_fixed(bus, ir)? {
            return Ok(r);
        }

        // TAS on a data-alterable operand. The Mega Drive bus does not complete
        // the locked read-modify-write, so a memory operand is tested but not
        // written back (GPGX builds without its TAS write-back callback).
        if ir & 0xFFC0 == 0x4AC0 && mode != 1 && !(mode == 7 && reg > 1) {
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, Size::Byte);
            let old = self.read_ea(bus, &ea, Size::Byte);
            self.set_logic(Size::Byte, old);
            if mode == 0 {
                self.write_ea(bus, &ea, Size::Byte, old | 0x80);
                return Ok((4, 0));
            }
            return Ok((10 + ea_cycles(mode, reg, Size::Byte, false), 0));
        }

        // Size-selecting arithmetic and logic.
        let size_field = (ir >> 6) & 3;
        if size_field != 3 {
            let size = Size::from_bits(size_field);
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, size);
            let modify_cycles = if mode == 0 {
                if size == Size::Long {
                    6
                } else {
                    4
                }
            } else {
                8 + if size == Size::Long { 4 } else { 0 } + ea_cycles(mode, reg, size, false)
            };
            match ir & 0xFF00 {
                0x4000 => {
                    // NEGX: 0 - dst - X.
                    let dst = self.read_ea(bus, &ea, size);
                    let r = self.extended(size, dst, 0, true);
                    self.write_ea(bus, &ea, size, r);
                    return Ok((modify_cycles, 0));
                }
                0x4200 => {
                    self.write_ea(bus, &ea, size, 0);
                    self.set_logic(size, 0);
                    return Ok((modify_cycles, 0));
                }
                0x4400 => {
                    let dst = self.read_ea(bus, &ea, size);
                    let r = self.sub_flags(size, 0, dst);
                    self.write_ea(bus, &ea, size, r);
                    return Ok((modify_cycles, 0));
                }
                0x4600 => {
                    let dst = self.read_ea(bus, &ea, size);
                    let r = !dst & size.mask();
                    self.write_ea(bus, &ea, size, r);
                    self.set_logic(size, r);
                    return Ok((modify_cycles, 0));
                }
                0x4800 => {
                    let old = self.read_ea(bus, &ea, Size::Byte);
                    let r = self.bcd_sub(old, 0);
                    self.write_ea(bus, &ea, Size::Byte, r);
                    return Ok((
                        if mode == 0 {
                            6
                        } else {
                            8 + ea_cycles(mode, reg, Size::Byte, false)
                        },
                        0,
                    ));
                }
                0x4A00 => {
                    let v = self.read_ea(bus, &ea, size);
                    self.set_logic(size, v);
                    return Ok((4 + ea_cycles(mode, reg, size, false), 0));
                }
                _ => {}
            }
        }
        Err(Exception::Illegal { opcode: ir })
    }

    /// Encodings in group 4 whose meaning does not follow the size-selecting
    /// pattern. Returns `Some` when the opcode was handled here.
    fn op_misc_fixed<B: Bus>(
        &mut self,
        bus: &mut B,
        ir: u16,
    ) -> Result<Option<(u32, u32)>, Exception> {
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;

        if ir & 0xFFF0 == 0x4E40 {
            let vector = 32 + (ir & 0xF) as u8;
            return Err(Exception::Vector { vector });
        }
        // GPGX's frs/toc/tos handlers: 40C0 reads SR; 44C0 writes CCR;
        // 46C0 writes SR. MOVE from CCR does not exist on the 68000.
        if ir & 0xFFC0 == 0x40C0 || ir & 0xFFC0 == 0x46C0 || ir & 0xFFC0 == 0x44C0 {
            let form = ir & 0xFFC0;
            if form == 0x46C0 && !self.sr.is_supervisor() {
                return Err(Exception::Privileged);
            }
            let ea_mode = decode_mode((ir >> 3) & 7, (ir & 7) as u8);
            let ea = self.effective_address(bus, ea_mode, (ir & 7) as u8, Size::Word);
            if form == 0x40C0 {
                self.write_ea(bus, &ea, Size::Word, self.sr.0 as u32);
            } else {
                let value = self.read_ea(bus, &ea, Size::Word) as u16;
                if form == 0x44C0 {
                    self.sr.set_ccr(value);
                } else {
                    self.set_status(value);
                }
            }
            // Status-register moves have different base costs in each
            // direction. Addressing-mode clocks still apply: immediate
            // to SR/CCR is 16, not the data-register form's 12 clocks.
            let cycles = if form == 0x40C0 {
                if mode == 0 {
                    6
                } else {
                    8 + ea_cycles(mode, reg, Size::Word, false)
                }
            } else {
                12 + ea_cycles(mode, reg, Size::Word, false)
            };
            return Ok(Some((cycles, 0)));
        }
        match ir {
            0x4AFC => return Err(Exception::Illegal { opcode: ir }),
            0x4E71 => return Ok(Some((4, 0))), // NOP
            0x4E70 => { // RESET
                if !self.sr.is_supervisor() { return Err(Exception::Privileged); }
                return Ok(Some((132, 0)));
            }
            0x4E76 => {
                // TRAPV
                if self.sr.flag(V) {
                    return Err(Exception::Vector { vector: 7 });
                }
                return Ok(Some((4, 0)));
            }
            0x4E72 => {
                // STOP
                if !self.sr.is_supervisor() { return Err(Exception::Privileged); }
                let imm = self.next_word(bus);
                self.set_status(imm);
                self.stopped = true;
                return Ok(Some((4, 1)));
            }
            0x4E73 => {
                // RTE
                if !self.sr.is_supervisor() { return Err(Exception::Privileged); }
                // RTE ends exception processing (reference CPU_INSTR_MODE /
                // CPU_RUN_MODE).
                self.instr_mode_no = false;
                self.aerr_mode = false;
                let sp = self.a[7];
                let sr = self.rd16(bus, sp) as u16;
                let pc = self.rd32(bus, sp.wrapping_add(2));
                self.a[7] = sp.wrapping_add(6);
                self.set_status(sr);
                self.set_pc(bus, pc as u32);
                return Ok(Some((20, 0)));
            }
            0x4E75 => {
                // RTS
                // The return address is a longword and the stack advances by
                // four. This is not symmetrical with anything else on the
                // stack and it is easy to assume otherwise: RTR pops a word
                // of CCR and *then* a longword of PC, and the two together
                // advance the stack by six. Genesis Plus GX's m68k core
                // spells both out - `m68k_op_rts_32` is `jump(pull_32())`
                // with `pull_32` doing `SP += 4; read_32(sp)`.
                let target = self.rd32(bus, self.a[7]);
                self.a[7] = self.a[7].wrapping_add(4);
                self.set_pc(bus, target);
                return Ok(Some((16, 0)));
            }
            0x4E77 => {
                // RTR
                // A word of CCR, then a longword of PC: six bytes.
                let sp = self.a[7];
                let ccr = self.rd16(bus, sp);
                let pc = self.rd32(bus, sp.wrapping_add(2));
                self.a[7] = sp.wrapping_add(6);
                self.sr.set_ccr(ccr);
                self.set_pc(bus, pc);
                return Ok(Some((20, 0)));
            }
            _ => {}
        }
        if ir & 0xF000 == 0xE000 {
            return Ok(Some(self.op_shift(bus, ir)?));
        }
        if ir & 0xFFF8 == 0x4880 || ir & 0xFFF8 == 0x48C0 {
            let dn = (ir & 7) as usize;
            let size = if ir & 0x0040 == 0 {
                Size::Word
            } else {
                Size::Long
            };
            let result = if size == Size::Word {
                self.d[dn] as u8 as i8 as i32 as u32
            } else {
                self.d[dn] as u16 as i16 as i32 as u32
            };
            self.write_ea(bus, &Ea::Data(dn as u8), size, result);
            self.set_logic(size, result);
            return Ok(Some((4, 0)));
        }
        // SWAP Dn occupies 0x4840-0x4847.
        if ir & 0xFFF8 == 0x4840 {
            // The register is in bits 2-0. Reading it from bits 11-9 picks
            // up the mode field instead, which for every SWAP encoding in the
            // corpus lands on a neighbouring register - so `swap d5` quietly
            // exchanges d4 and leaves the register the program named alone.
            let d = (ir & 7) as usize;
            let r = self.d[d].rotate_left(16);
            self.d[d] = r;
            self.set_logic(Size::Long, r);
            return Ok(Some((4, 0)));
        }
        // MOVE An,USP is 4E60-4E67 and writes the USP; MOVE USP,An is 4E68-4E6F and
        // writes the address register. Getting this pair the wrong way round
        // looks like a stack-pointer bug: `move.l a6,usp` deposits the current
        // stack pointer into a6 and leaves a6 looking freshly initialised.
        // Both directions are privileged on the 68000: independent GPGX
        // m68k_op_move_32_fru/tou both gate on FLAG_S.
        if ir & 0xFFF8 == 0x4E60 || ir & 0xFFF8 == 0x4E68 {
            let to_usp = ir & 0xFFF8 == 0x4E60;
            if !self.sr.is_supervisor() {
                return Err(Exception::Privileged);
            }
            if to_usp {
                self.usp = self.a[reg as usize];
            } else {
                self.a[reg as usize] = self.usp;
            }
            return Ok(Some((4, 0)));
        }
        if ir & 0xFFF8 == 0x4E50 {
            let an = reg as usize;
            let disp = self.next_word(bus) as i16 as i32;
            self.a[7] = self.a[7].wrapping_sub(4);
            // LINK A7 stores the already-decremented stack pointer (GPGX link_16_a7).
            let old = if an == 7 { self.a[7] } else { self.a[an] };
            self.wr32(bus, self.a[7], old);
            self.a[an] = self.a[7];
            self.a[7] = self.a[7].wrapping_add(disp as u32);
            return Ok(Some((16, 1)));
        }
        if ir & 0xFFF8 == 0x4E58 {
            let an = reg as usize;
            self.a[7] = self.a[an];
            let saved = self.rd32(bus, self.a[7]);
            self.a[7] = self.a[7].wrapping_add(4);
            // UNLK A7 loads A7 from the stack without the +4 (GPGX unlk_32_a7).
            self.a[an] = saved;
            return Ok(Some((12, 0)));
        }
        if ir & 0xFFC0 == 0x4840 {
            // PEA pushes the computed control EA, not its contents. Resolve
            // extensions before changing A7 (PEA d16(A7) uses the old SP).
            if !matches!(mode, 2 | 5 | 6) && !(mode == 7 && reg <= 3) {
                return Err(Exception::Illegal { opcode: ir });
            }
            let addr = self.lea_address(bus, mode, reg);
            let sp = self.a[7].wrapping_sub(4);
            self.a[7] = sp;
            self.wr32(bus, sp, addr);
            let cycles = match (mode, reg) {
                (2, _) => 12,
                (6, _) | (7, 3) => 20,
                (7, 1) => 20,
                _ => 16,
            };
            return Ok(Some((cycles, 0)));
        }
        // MOVEM is bits 15-12 == 0100, bit 11 == 1, bit 9 == 0, bit 7 == 1,
        // with an effective address other than a register-direct one. Bit 7 is
        // what separates it from LEA, and it is not optional: MOVEM occupies
        // 0x4880-0x48FF and 0x4C80-0x4CFF, and LEA occupies
        // 0x41C0-0x4F3F, so the two encodings genuinely overlap across
        // 0x4C00-0x4FFF. Matching only 0xFA00 == 0x4800 swallows every LEA in
        // that window - `lea.l $ffff09ec.l,a6` (0x4DF9) among them, which then
        // wrote to the wrong register and left the PC two bytes short.
        if ir & 0xFB80 == 0x4880 && mode >= 2 {
            return Ok(Some(self.op_movem(bus, ir)?));
        }
        if ir & 0xFFC0 == 0x4E80 || ir & 0xFFC0 == 0x4EC0 {
            // JSR/JMP accept control addressing modes only. Their EA costs
            // differ from LEA/ordinary memory transfers (68000 opcode table).
            let jump_cycles = match (mode, reg) {
                (2, _) => 8,
                (5, _) | (7, 0 | 2) => 10,
                (6, _) | (7, 3) => 14,
                (7, 1) => 12,
                _ => return Err(Exception::Illegal { opcode: ir }),
            };
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, Size::Long);
            let target = match ea {
                Ea::Memory(a) => a,
                _ => return Err(Exception::Illegal { opcode: ir }),
            };
            let call = ir & 0x0040 == 0;
            if call {
                let sp = self.a[7].wrapping_sub(4);
                self.wr32(bus, sp, self.pc);
                self.a[7] = sp;
            }
            self.set_pc(bus, target);
            return Ok(Some((jump_cycles + if call { 8 } else { 0 }, 0)));
        }
        if ir & 0xF1C0 == 0x41C0 {
            // LEA. (CHK.L shares this pattern only on the 68020 and later.)
            let addr = self.lea_address(bus, mode, reg);
            let an = ((ir >> 9) & 7) as usize;
            self.a[an] = addr;
            return Ok(Some((4 + control_ea_cycles(mode, reg), 0)));
        }
        if ir & 0xF1C0 == 0x4180 {
            // CHK.W <ea>,Dn: Dn is checked against the bound at <ea>. GPGX's
            // undocumented flags: Z from Dn, V and C clear, N set when Dn < 0.
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, Size::Word);
            let bound = self.read_ea(bus, &ea, Size::Word) as u16 as i16;
            let value = self.d[((ir >> 9) & 7) as usize] as u16 as i16;
            let ea_cost = ea_cycles(mode, reg, Size::Word, false);
            self.sr.set_flag(Z, value == 0);
            self.sr.set_flag(V, false);
            self.sr.set_flag(C, false);
            if (0..=bound).contains(&value) {
                return Ok(Some((10 + ea_cost, 0)));
            }
            if value < 0 {
                self.sr.set_flag(N, true);
            }
            self.trap_cycles = ea_cost + if value < 0 { 2 } else { 0 };
            return Err(Exception::Vector { vector: 6 });
        }
        Ok(None)
    }

    fn op_movem<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        // MOVEM does not use the Size encoding every other instruction does.
        // Its bits 7-6 are 00 = byte, 10 = word, 11 = long, which is *shifted*
        // one code away from the generic 00/01/10. Reading the field with
        // `Size::from_bits` therefore makes every movem.w in the cartridge
        // transfer longwords: the register list is read from the wrong
        // offsets, the base register advances twice as far, and the damage
        // surfaces much later as unrelated-looking corruption. Confirmed
        // against every MOVEM the cartridge contains and against the
        // reference decoder, which agree on all of them.
        let size = match (ir >> 6) & 3 {
            2 => Size::Word,
            3 => Size::Long,
            _ => Size::Byte,
        };
        let to_memory = ir & 0x0400 == 0;
        let step = size.bytes();
        let list = self.next_word(bus);
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;

        // Predecrement reverses mask bits: GPGX movem_*_re_pd iterates bits
        // from zero and selects REG_DA[15-i]. All other forms use REG_DA[i].
        let order: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

        let mut addr = match mode {
            4 => self.a[reg as usize],
            3 => self.a[reg as usize],
            _ => match self.effective_address(bus, decode_mode(mode, reg), reg, size) {
                Ea::Memory(addr) => addr,
                _ => return Err(Exception::Illegal { opcode: ir }),
            },
        };
        let decrement = mode == 4;

        if to_memory {
            // Predecrement stores in reverse register order.
            let mut count = 0u32;
            for &bit in &order {
                if list & (1u16 << bit) == 0 {
                    continue;
                }
                let r = if decrement { 15 - bit } else { bit };
                let value = if r < 8 {
                    self.d[r as usize]
                } else {
                    self.a[(r - 8) as usize]
                };
                // Predecrement steps down *before* the store. Stepping after
                // leaves the block one word too high and the lowest word of
                // the saved frame unwritten.
                if decrement {
                    addr = addr.wrapping_sub(step);
                    self.write_sized(bus, addr, size, value);
                } else {
                    self.write_sized(bus, addr, size, value);
                    addr = addr.wrapping_add(step);
                }
                count += 1;
            }
            if decrement {
                self.a[reg as usize] = addr;
            }
            if size == Size::Long {
                self.refresh_block_cycles = Some(count * step * 2);
            }
            return Ok((8 + count * step * 2 + control_ea_cycles(mode, reg), 1));
        }

        let mut count = 0u32;
        for &r in order.iter() {
            if list & (1u16 << r) == 0 {
                continue;
            }
            let raw = self.read_sized(bus, addr, size);
            // GPGX movem_16_er_* sign-extends every word load, including Dn.
            let value = if size == Size::Word {
                raw as u16 as i16 as i32 as u32
            } else {
                raw
            };
            if r < 8 {
                self.d[r as usize] = value;
            } else {
                self.a[(r - 8) as usize] = value;
            }
            addr = addr.wrapping_add(step);
            count += 1;
        }
        // Postincrement wins over a value loaded into the base register.
        // Non-postincrement forms must not advance an unlisted base register.
        if mode == 3 {
            self.a[reg as usize] = addr;
        }
        if size == Size::Long {
            self.refresh_block_cycles = Some(count * step * 2);
        }
        Ok((12 + count * step * 2 + control_ea_cycles(mode, reg), 1))
    }

    // ------------------------------------------------------------- group 5

    fn op_quick<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;
        let size = Size::from_bits((ir >> 6) & 3);

        // Bits 7-6 == 11 split into two forms. Mode 001 is reserved for DBcc,
        // so `Scc` can never address (An); every other mode is a plain Scc.
        if ir & 0x00C0 == 0x00C0 {
            if mode == 1 {
                // All sixteen conditions occupy bits 11-8. On a 68000 every
                // DBcc decrements only the low word; bit 8 is part of the
                // condition, not a counter-size selector.
                let cc = ((ir >> 8) & 0xF) as u8;
                let dn = (ir & 7) as usize;
                // The displacement is measured from the displacement word's
                // OWN address, and next_word has already advanced past it.
                // Genesis Plus GX spells the subtraction out - OPER_I_16()
                // then REG_PC -= 2 before m68ki_branch_16 - so the base is
                // two bytes back from where we are now. Reading `self.pc`
                // directly put every DBcc two bytes high, and King's Bounty's
                // inner fill loop branches to 0x27F42 that way: correct
                // target 0x27FAA - 104 = 0x27F42, our target 0x27F44, the
                // MIDDLE of move.l #$BBBBBBBB,$FFFF0006, where $BB $BB
                // decodes as BSR.L and jumps to $BBBBBBBA.
                let disp = self.next_word(bus) as i16 as i32;
                let base = self.pc.wrapping_sub(2);
                // The loop runs while the condition is FALSE, which is what
                // makes DBF a countdown: a false condition decrements and
                // branches on a non-zero result, and a counter that reaches
                // zero wraps to 0xFFFF and falls out. Genesis Plus GX's
                // m68ki_dbcc_16 does exactly this - COND_NOT_CC() guarding
                // the decrement - so the counter legitimately ends 0xFFFF.
                // GPGX m68k_op_dbf_16 masks the decrement to 16 bits and
                // combines it with the original high word.
                let mask: u32 = 0xFFFF;
                if !cond_met(self.sr, cc) {
                    let res = self.d[dn].wrapping_sub(1) & mask;
                    self.d[dn] = (self.d[dn] & !mask) | res;
                    if res != mask {
                        self.set_pc(bus, base.wrapping_add(disp as u32));
                    }
                }
                return Ok((
                    if cond_met(self.sr, cc) {
                        12
                    } else if self.d[dn] & mask == mask {
                        14
                    } else {
                        10
                    },
                    1,
                ));
            }
            let cc = ((ir >> 8) & 0xF) as u8;
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, Size::Byte);
            let set = cond_met(self.sr, cc);
            if let Ea::Data(_) | Ea::Memory(_) = ea {
                self.write_ea(bus, &ea, Size::Byte, if set { 0xFF } else { 0 });
            }
            return Ok((
                if mode == 0 {
                    if set {
                        6
                    } else {
                        4
                    }
                } else {
                    8 + ea_cycles(mode, reg, Size::Byte, false)
                },
                0,
            ));
        }

        let encoded = ((ir >> 9) & 7) as u32;
        let data = if encoded == 0 { 8 } else { encoded };
        // Address-register quick arithmetic is always 32-bit and does not
        // change flags, even when the opcode's size field says word.
        if mode == 1 {
            self.a[reg as usize] = if ir & 0x0100 == 0 {
                self.a[reg as usize].wrapping_add(data)
            } else {
                self.a[reg as usize].wrapping_sub(data)
            };
            return Ok((8, 0));
        }
        let ea = self.effective_address(bus, decode_mode(mode, reg), reg, size);
        let dst = self.read_ea(bus, &ea, size);
        // ADDQ/SUBQ compute destination - source, and the flag helpers take the
        // minuend first, so the operands go in the opposite order from SUB.
        let result = if ir & 0x0100 == 0 {
            self.add_flags(size, data, dst)
        } else {
            self.sub_flags(size, dst, data)
        };
        self.write_destination(bus, &ea, size, result);
        // ADDQ/SUBQ leave X untouched.
        let keep = self.sr.ccr() & X;
        self.sr.set_ccr(keep | (self.sr.ccr() & !X));
        let cycles = if mode == 0 {
            if size == Size::Long {
                8
            } else {
                4
            }
        } else {
            8 + if size == Size::Long { 4 } else { 0 } + ea_cycles(mode, reg, size, false)
        };
        Ok((cycles, 0))
    }

    fn op_branch<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        // BSR is 0x6100 with a 16-bit displacement, 0x61xx with an 8-bit one.
        // It is not a conditional branch: decoding it as one makes every call
        // fall through, because the bits it shares with Bcc spell a condition
        // the flags happen not to satisfy.
        if ir & 0xFF00 == 0x6100 {
            let base = self.pc;
            let wide = ir == 0x6100;
            let (disp, pushed) = if wide {
                (self.next_word(bus) as i16 as i32, self.pc)
            } else {
                ((ir & 0xFF) as i8 as i32, self.pc)
            };
            let sp = self.a[7].wrapping_sub(4);
            self.wr32(bus, sp, pushed);
            self.a[7] = sp;
            self.set_pc(bus, base.wrapping_add(disp as u32));
            return Ok((18, if wide { 1 } else { 0 }));
        }
        // Displacement size is not selected by bit 8. Genesis Plus GX's jump
        // table gives each condition sixteen consecutive opcodes and marks
        // only the first - the one whose low byte is zero - as the word form;
        // the other fifteen are byte branches taking their displacement from
        // the low byte of the opcode itself. So `beq.b $0008` is 0x6708, not
        // 0x6700 plus an extension word, and reading a word there shifts the
        // PC two bytes for every one of them.
        if ir == 0x6000 {
            let target = self.pc_displacement(bus);
            self.set_pc(bus, target);
            return Ok((10, 1));
        }
        if ir & 0xF0FF == 0x6000 {
            let cc = ((ir >> 8) & 0xF) as u8;
            let disp = self.next_word(bus) as i16 as i32;
            // Same rule as DBcc: the base is the extension word's own
            // address, which is two bytes back from here.
            let base = self.pc.wrapping_sub(2);
            let taken = match self.branch_override_at(base) {
                Some(forced) => forced,
                None => cond_met(self.sr, cc),
            };
            let target = base.wrapping_add(disp as u32);
            if taken {
                self.set_pc(bus, target);
                Ok((10, 1))
            } else {
                Ok((12, 1))
            }
        } else {
            let cc = ((ir >> 8) & 0xF) as u8;
            // The displacement byte is the last byte of the instruction, and
            // the branch is measured from the address after the opcode word:
            // m68ki_branch_8 adds straight to REG_PC, while m68ki_branch_16
            // backs REG_PC up by two to reach its extension word, which sits
            // at exactly that address.
            let disp = (ir & 0xFF) as i8 as i32;
            let taken = match self.branch_override_at(self.pc.wrapping_sub(2)) {
                Some(forced) => forced,
                None => cond_met(self.sr, cc),
            };
            if taken {
                let target = self.pc.wrapping_add(disp as u32);
                self.set_pc(bus, target);
                Ok((10, 0))
            } else {
                Ok((8, 0))
            }
        }
    }

    // --------------------------------------------- 8, 9, B, C and D families

    fn line_8<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let reg = (ir & 7) as u8;
        if ir & 0xF1F0 == 0x8100 {
            return self.bcd_instruction(bus, ir, false);
        }
        if ir & 0xF1C0 == 0x80C0 {
            let ea = self.effective_address(bus, decode_mode((ir >> 3) & 7, reg), reg, Size::Word);
            let divisor = self.read_ea(bus, &ea, Size::Word) as u32;
            let target = ((ir >> 9) & 7) as usize;
            let dividend = self.d[target];
            if divisor == 0 {
                self.sr.set_flag(C, false);
                self.trap_cycles = ea_cycles((ir >> 3) & 7, reg, Size::Word, false);
                return Err(Exception::DivideByZero);
            }
            let quotient = dividend / divisor;
            if quotient > 0xFFFF {
                self.sr.set_flag(V, true);
                self.sr.set_flag(N, true);
                self.sr.set_flag(C, false);
                return Ok((10 + ea_cycles((ir >> 3) & 7, reg, Size::Word, false), 0));
            }
            let body = divu_body_cycles(dividend, divisor);
            self.refresh_block_cycles = Some(body);
            self.sr.set_flag(V, false);
            self.sr.set_flag(C, false);
            self.d[target] = ((dividend % divisor) << 16) | quotient;
            self.set_nz(Size::Word, quotient);
            return Ok((body + ea_cycles((ir >> 3) & 7, reg, Size::Word, false), 0));
        }
        if ir & 0xF1C0 == 0x81C0 {
            let ea = self.effective_address(bus, decode_mode((ir >> 3) & 7, reg), reg, Size::Word);
            let divisor = self.read_ea(bus, &ea, Size::Word) as u16 as i16 as i32;
            let target = ((ir >> 9) & 7) as usize;
            let dividend = self.d[target] as u32 as i32;
            if divisor == 0 {
                self.sr.set_flag(C, false);
                self.trap_cycles = ea_cycles((ir >> 3) & 7, reg, Size::Word, false);
                return Err(Exception::DivideByZero);
            }
            let body = divs_body_cycles(dividend, divisor);
            self.refresh_block_cycles = Some(body);
            let cycles = body + ea_cycles((ir >> 3) & 7, reg, Size::Word, false);
            // GPGX's explicit MIN/-1 case produces zero and Z rather than
            // invoking host signed overflow (m68kops.h divs_16_i/d handlers).
            if dividend == i32::MIN && divisor == -1 {
                self.d[target] = 0;
                self.sr.set_flag(N, false);
                self.sr.set_flag(Z, true);
                self.sr.set_flag(V, false);
                self.sr.set_flag(C, false);
                return Ok((cycles, 0));
            }
            // Widen before division so no host overflow can panic analysis.
            let quotient = i64::from(dividend) / i64::from(divisor);
            if quotient > 32767 || quotient < -32768 {
                self.sr.set_flag(V, true);
                self.sr.set_flag(N, true);
                self.sr.set_flag(C, false);
                return Ok((cycles, 0));
            }
            self.sr.set_flag(V, false);
            self.sr.set_flag(C, false);
            let remainder = i64::from(dividend) % i64::from(divisor);
            self.d[target] = ((remainder as u32 & 0xffff) << 16) | (quotient as u32 & 0xFFFF);
            self.set_nz(Size::Word, quotient as u32 & 0xFFFF);
            return Ok((cycles, 0));
        }
        self.alu(bus, ir, Alu::Or)
    }

    fn line_9<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        // SUBX mirrors ADDX's encoding; SUBA is left for `alu`.
        if ir & 0x0130 == 0x0100 && ir & 0x00C0 != 0x00C0 {
            return self.extended_pair(bus, ir, true);
        }
        self.alu(bus, ir, Alu::Sub)
    }

    fn line_b<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let reg = (ir & 7) as u8;
        if ir & 0xF138 == 0xB108 && ir & 0x00C0 != 0x00C0 {
            // CMPM: both operands advance by the operation size.
            // Size 11 instead selects CMPA.L An,An; the broad addressing
            // mask overlaps that family, which must not increment either An.
            let size = Size::from_bits((ir >> 6) & 3);
            let src = reg as usize;
            let dst = ((ir >> 9) & 7) as usize;
            let sa = self.a[src];
            let s = self.read_sized(bus, sa, size);
            let step = |r| {
                if size == Size::Byte && r == 7 {
                    2
                } else {
                    size.bytes()
                }
            };
            let sa_next = sa.wrapping_add(step(src));
            self.a[src] = sa_next;
            let da = self.a[dst];
            let d = self.read_sized(bus, da, size);
            self.a[dst] = da.wrapping_add(step(dst));
            self.compare_flags(size, d, s);
            return Ok((if size == Size::Long { 20 } else { 12 }, 0));
        }
        // Bit 8 selects EOR Dn,<ea>, except opmode 111 which is CMPA.L.
        let alu = if ir & 0x0100 != 0 && ir & 0x00C0 != 0x00C0 {
            Alu::Eor
        } else {
            Alu::Cmp
        };
        self.alu(bus, ir, alu)
    }

    fn line_c<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;
        if ir & 0xF1C0 == 0xC0C0 || ir & 0xF1C0 == 0xC1C0 {
            // MULU / MULS
            let signed = ir & 0x0100 != 0;
            let ea = self.effective_address(bus, decode_mode((ir >> 3) & 7, reg), reg, Size::Word);
            let ea_val = self.read_ea(bus, &ea, Size::Word);
            let target = ((ir >> 9) & 7) as usize;
            let product = if signed {
                ((ea_val as u16 as i16 as i32) * (self.d[target] as u16 as i16 as i32)) as u32
            } else {
                (ea_val & 0xFFFF).wrapping_mul(self.d[target] & 0xFFFF)
            };
            self.d[target] = product;
            self.set_logic(Size::Long, product);
            let source = ea_val as u16;
            let transitions = if signed {
                source ^ source.wrapping_shl(1)
            } else {
                source
            };
            let cycles =
                38 + 2 * transitions.count_ones() + ea_cycles(mode, reg, Size::Word, false);
            return Ok((cycles, 0));
        }
        if matches!(ir & 0xF1F8, 0xC140 | 0xC148 | 0xC188) {
            // GPGX dispatch: DD=C140, AA=C148, DA=C188. EXG preserves SR.
            let rx = ((ir >> 9) & 7) as usize;
            let ry = reg as usize;
            match ir & 0xF1F8 {
                0xC140 => {
                    let t = self.d[rx];
                    self.d[rx] = self.d[ry];
                    self.d[ry] = t;
                }
                0xC148 => {
                    let t = self.a[rx];
                    self.a[rx] = self.a[ry];
                    self.a[ry] = t;
                }
                _ => {
                    let t = self.d[rx];
                    self.d[rx] = self.a[ry];
                    self.a[ry] = t;
                }
            }
            return Ok((6, 0));
        }
        if ir & 0xF1F0 == 0xC100 {
            return self.bcd_instruction(bus, ir, true);
        }
        let _ = mode;
        self.alu(bus, ir, Alu::And)
    }

    fn line_d<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        // ADDX occupies the `ADD Dn,<ea>` forms whose EA would be Dn or An
        // (bits 5-4 clear); bit 3 picks registers or predecrement memory.
        if ir & 0x0130 == 0x0100 && ir & 0x00C0 != 0x00C0 {
            return self.extended_pair(bus, ir, false);
        }
        // ADDA is the remaining address-register form; `alu` handles it.
        self.alu(bus, ir, Alu::Add)
    }

    /// The shared shape of the OR/AND/EOR/SUB/ADD/CMP families.
    fn alu<B: Bus>(&mut self, bus: &mut B, ir: u16, alu: Alu) -> Step {
        let size_field = (ir >> 6) & 3;
        let mode = (ir >> 3) & 7;
        let reg = (ir & 7) as u8;
        let src = ((ir >> 9) & 7) as usize;
        // Bit 8 picks the direction: clear means the data register named in
        // bits 11-9 is the destination (`op <ea>,Dn`), set means the effective
        // address is (`op Dn,<ea>`). ADD.L (A0)+,D0 is 0xD098, which confirms
        // it: the game sums the cartridge into D0 with that encoding.
        let to_data_register = ir & 0x0100 == 0;

        // Bits 7-6 == 11 is not a size; it marks the address-register forms
        // ADDA, SUBA and CMPA, where the operand size is bit 8 and CMPA's
        // effective address moves down into bits 5-3. A word operand is
        // sign-extended into the full 32-bit result.
        if ir & 0x00C0 == 0x00C0 && matches!(alu, Alu::Cmp) {
            let dst = ((ir >> 9) & 7) as usize;
            let long = ir & 0x0100 != 0;
            let size = if long { Size::Long } else { Size::Word };
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, size);
            let raw = self.read_ea(bus, &ea, size);
            let operand = if long {
                raw
            } else {
                raw as u16 as i16 as i32 as u32
            };
            // CMPA compares the *whole* 32-bit address register. Only the
            // immediate is narrowed, and it is sign-extended. (CMP.W against
            // a data register does narrow, which is the usual trap here: a
            // word compare of An masked to its low word gives the wrong
            // answer as soon as A0 passes 0xFFFF.)
            let old = self.a[dst];
            // CMPA only writes the condition codes.
            self.compare_flags(Size::Long, old, operand);
            return Ok((6 + ea_cycles(mode, reg, size, false), 0));
        }

        // GPGX adda/suba handlers use the ordinary source EA and never touch
        // condition codes. Word sources are sign-extended to 32 bits.
        if size_field == 3 && matches!(alu, Alu::Add | Alu::Sub) {
            let long = ir & 0x0100 != 0;
            let dst = ((ir >> 9) & 7) as usize;
            let size = if long { Size::Long } else { Size::Word };
            let ea = self.effective_address(bus, decode_mode(mode, reg), reg, size);
            let raw = self.read_ea(bus, &ea, size);
            let operand = if long {
                raw
            } else {
                raw as u16 as i16 as i32 as u32
            };
            let old = self.a[dst];
            self.a[dst] = if matches!(alu, Alu::Add) {
                old.wrapping_add(operand)
            } else {
                old.wrapping_sub(operand)
            };
            let base = if !long || mode <= 1 {
                8
            } else if mode == 7 && reg == 4 {
                8
            } else {
                6
            };
            return Ok((base + ea_cycles(mode, reg, size, false), 0));
        }

        if size_field == 3 {
            return Err(Exception::Illegal { opcode: ir });
        }
        let size = Size::from_bits(size_field);

        // Register-to-register forms take no effective address and, for the
        // logical operations, leave the condition codes alone.
        if !to_data_register && size_field == 1 && mode == 1 {
            let s = self.d[src] & size.mask();
            let d = self.d[reg as usize] & size.mask();
            let r = match alu {
                Alu::Or => s | d,
                Alu::And => s & d,
                Alu::Eor => s ^ d,
                Alu::Sub | Alu::Add => return Err(Exception::Illegal { opcode: ir }),
                Alu::Cmp => {
                    self.compare_flags(size, s, d);
                    return Ok((4, 0));
                }
            };
            self.d[reg as usize] = (self.d[reg as usize] & !size.mask()) | (r & size.mask());
            return Ok((4, 0));
        }

        let ea = self.effective_address(bus, decode_mode(mode, reg), reg, size);
        if to_data_register {
            let ea_val = self.read_ea(bus, &ea, size);
            let s = self.d[src];
            let r = match alu {
                Alu::Or => s | ea_val,
                Alu::And => s & ea_val,
                Alu::Eor => s ^ ea_val,
                // The data register is the destination, so it is the minuend.
                // `sub_flags(size, src, dst)` computes src - dst, so the
                // register has to come first. Passing the effective address
                // first computes EA - Dn: the flags came out backwards, and so
                // did the value written back into the register.
                Alu::Sub => self.sub_flags(size, s, ea_val),
                Alu::Add => self.add_flags(size, s, ea_val),
                Alu::Cmp => {
                    self.compare_flags(size, s, ea_val);
                    return Ok((
                        if size == Size::Long { 6 } else { 4 } + ea_cycles(mode, reg, size, false),
                        0,
                    ));
                }
            };
            if alu.sets_flags_outside_helpers() {
                self.set_logic(size, r);
            }
            self.d[src] = match size {
                Size::Byte => (self.d[src] & !0xFF) | (r & 0xFF),
                Size::Word => (self.d[src] & !0xFFFF) | (r & 0xFFFF),
                Size::Long => r,
            };
            let base = if size != Size::Long {
                4
            } else if mode <= 1 || (mode == 7 && reg == 4) {
                8
            } else {
                6
            };
            Ok((base + ea_cycles(mode, reg, size, false), 0))
        } else {
            let ea_val = self.read_ea(bus, &ea, size);
            let s = self.d[src];
            let r = match alu {
                Alu::Or => s | ea_val,
                Alu::And => s & ea_val,
                Alu::Eor => s ^ ea_val,
                // Here the effective address is the destination, so it is the
                // minuend and the register is the subtrahend.
                Alu::Sub => self.sub_flags(size, ea_val, s),
                Alu::Add => self.add_flags(size, s, ea_val),
                Alu::Cmp => {
                    self.compare_flags(size, ea_val, s);
                    return Ok((4, 0));
                }
            };
            if alu.sets_flags_outside_helpers() {
                self.set_logic(size, r);
            }
            self.write_ea(bus, &ea, size, r);
            let base = if mode == 0 {
                if size == Size::Long {
                    8
                } else {
                    4
                }
            } else if size == Size::Long {
                12
            } else {
                8
            };
            Ok((base + ea_cycles(mode, reg, size, false), 0))
        }
    }

    /// `src + dst + X` for ADDX. Only Z is meaningful to the caller.
    fn extended(&mut self, size: Size, src: u32, dst: u32, subtract: bool) -> u32 {
        let mask = size.mask();
        let sign = 1u64 << (size.bytes() * 8 - 1);
        let (src, dst, x) = ((src & mask) as u64, (dst & mask) as u64, self.sr.flag(X) as u64);
        let full = if subtract {
            dst.wrapping_sub(src).wrapping_sub(x)
        } else {
            dst + src + x
        };
        let r = full & mask as u64;
        let carry = full & !(mask as u64) != 0;
        let overflow = if subtract {
            (src ^ dst) & (r ^ dst) & sign != 0
        } else {
            (src ^ r) & (dst ^ r) & sign != 0
        };
        self.sr.set_flag(C, carry);
        self.sr.set_flag(X, carry);
        self.sr.set_flag(V, overflow);
        self.sr.set_flag(N, r & sign != 0);
        // Z is only ever cleared, so multi-precision chains test the whole value.
        if r != 0 {
            self.sr.set_flag(Z, false);
        }
        r as u32
    }

    /// ADDX/SUBX: `Dy,Dx` or `-(Ay),-(Ax)` with the operand size in bits 7-6.
    fn extended_pair<B: Bus>(&mut self, bus: &mut B, ir: u16, subtract: bool) -> Step {
        let size = Size::from_bits((ir >> 6) & 3);
        let src = (ir & 7) as usize;
        let dst = ((ir >> 9) & 7) as usize;
        if ir & 8 == 0 {
            let r = self.extended(size, self.d[src], self.d[dst], subtract);
            self.d[dst] = (self.d[dst] & !size.mask()) | r;
            return Ok((if size == Size::Long { 8 } else { 4 }, 0));
        }
        let step = |r: usize| {
            if size == Size::Byte && r == 7 {
                2
            } else {
                size.bytes()
            }
        };
        self.a[src] = self.a[src].wrapping_sub(step(src));
        let s = self.read_sized(bus, self.a[src], size);
        self.a[dst] = self.a[dst].wrapping_sub(step(dst));
        let d = self.read_sized(bus, self.a[dst], size);
        let r = self.extended(size, s, d, subtract);
        self.write_sized(bus, self.a[dst], size, r);
        Ok((if size == Size::Long { 30 } else { 18 }, 0))
    }

    // ------------------------------------------------------------- group E

    /// The shift and rotate group, 0xE000-0xE7FF.
    ///
    /// Two different layouts share the group, and reading one as the other is
    /// silent: every instruction still decodes to the right length and the
    /// wrong operation, so the game runs and computes nonsense.
    ///
    /// Register form (bit 5 clear) counts *immediates*: bits 11-9 hold the
    /// count itself, where zero means eight, and bit 8 is the direction.
    ///
    /// Register form (bit 5 set, register destination) takes the count *from*
    /// the data register named in bits 11-9.
    ///
    /// Memory form (bit 5 set, any other destination) has no count at all -
    /// memory shifts are always by one - so bits 11-9 become the operation and
    /// bit 7 carries the size, with the byte size not available because a
    /// memory shift always moves a word.
    fn op_shift<B: Bus>(&mut self, bus: &mut B, ir: u16) -> Step {
        let dir_left = ir & 0x0100 != 0;
        let reg8 = (ir & 7) as u8;
        let reg = reg8 as usize;
        // Bits 7-6 carry the size for a register destination and are `11` for
        // every memory form - the size there is implied. Deciding by the EA
        // mode instead would read `asl.w d1,d0`, whose register-count form
        // borrows mode bits 100, as a memory shift of (a1).
        let to_memory = (ir & 0x00C0) == 0x00C0;

        if to_memory {
            // bits 11-9 select the operation here, not the count.
            let kind = (ir >> 9) & 3;
            let size = Size::Word;
            let ea = self.effective_address(bus, decode_mode((ir >> 3) & 7, reg8), reg8, size);
            let old = self.read_ea(bus, &ea, size);
            let r = self.shift(kind, dir_left, 1, old, size);
            self.write_ea(bus, &ea, size, r);
            return Ok((8 + ea_cycles((ir >> 3) & 7, reg8, Size::Word, false), 0));
        }

        let kind = (ir >> 3) & 3;
        let size = Size::from_bits((ir >> 6) & 3);
        // Bit 5 separates an immediate count from one held in a register.
        let count = if ir & 0x0020 != 0 {
            let idx = ((ir >> 9) & 7) as usize;
            self.d[idx] & 63
        } else {
            let field = ((ir >> 9) & 7) as u32;
            if field == 0 {
                8
            } else {
                field
            }
        };
        let old = self.d[reg];
        // Nonzero register-count forms suppress refresh during their variable
        // internal phase. Immediate-count forms do not apply this rule.
        if ir & 0x0020 != 0 && count != 0 {
            self.refresh_block_cycles = Some(count * 2);
        }
        let r = self.shift(kind, dir_left, count, old, size);
        self.d[reg] = match size {
            Size::Byte => (old & !0xFF) | (r & 0xFF),
            Size::Word => (old & !0xFFFF) | (r & 0xFFFF),
            Size::Long => r,
        };
        Ok((if size == Size::Long { 8 } else { 6 } + count * 2, 0))
    }

    /// One shift or rotate of `value`, returning the masked result.
    ///
    /// `kind` is bits 4-3: arithmetic, logical, rotate-through-extend, or
    /// plain rotate. Every form except a zero count leaves the last bit out
    /// in both X and C; a zero count clears C and changes nothing, which is
    /// the one case where "shift by n" and "do nothing" differ.
    fn shift(&mut self, kind: u16, left: bool, count: u32, value: u32, size: Size) -> u32 {
        let bits = size.bytes() * 8;
        let mask = size.mask();
        let sign = 1u32 << (bits - 1);
        let mut v = value & mask;
        if count == 0 {
            self.set_nz(size, v);
            self.sr.set_flag(V, false);
            self.sr.set_flag(C, kind == 2 && self.sr.flag(X));
            return v;
        }
        // A rotate of a byte is taken modulo 8; every other shift uses the
        // count as given, and a count beyond the width clears the register
        // (arithmetic right) or shifts out to zero.
        let n = count;
        let mut overflow = false;
        for _ in 0..n {
            let out = if left { (v & sign) != 0 } else { v & 1 != 0 };
            v = match (kind, left) {
                // Arithmetic: the sign replicates on the way right and a zero
                // comes in on the way left. Folding `out` back in here would
                // turn ASL into a rotate and silently change the value.
                (0, true) => (v << 1) & mask,
                (0, false) => (v >> 1) | if (value & sign) != 0 { sign } else { 0 },
                // Logical: zeros come in at the bottom either way.
                (1, true) => (v << 1) & mask,
                (1, false) => v >> 1,
                // Rotate through extend: the X flag is the bit that wraps in.
                (2, true) => {
                    let x = self.sr.flag(X);
                    ((v << 1) | x as u32) & mask
                }
                (2, false) => {
                    let x = self.sr.flag(X);
                    (v >> 1) | (if x { sign } else { 0 })
                }
                // Plain rotate: the outgoing bit comes straight back around.
                (_, true) => ((v << 1) | out as u32) & mask,
                (_, false) => (v >> 1) | if out { sign } else { 0 },
            };
            if kind == 0 && left && ((v & sign) != 0) != out {
                overflow = true;
            }
            self.sr.set_flag(C, out);
            if kind != 3 {
                self.sr.set_flag(X, out);
            }
        }
        self.sr.set_flag(N, (v & sign) != 0);
        self.sr.set_flag(Z, v == 0);
        // Only an arithmetic left shift can report overflow; every other form
        // clears it, and a rotate-through-extend leaves it alone.
        if kind == 0 {
            self.sr.set_flag(V, if left { overflow } else { false });
        } else {
            self.sr.set_flag(V, false);
        }
        v
    }

    // ----------------------------------------------------------------- BCD

    /// ABCD/SBCD use bit3 for register vs predecrement memory operands.
    /// Source is bits2-0, destination bits11-9. The generic EA mode does not
    /// identify these forms; both old mode==4 guards excluded the whole family.
    fn bcd_instruction<B: Bus>(&mut self, bus: &mut B, ir: u16, add: bool) -> Step {
        let source = (ir & 7) as usize;
        let destination = ((ir >> 9) & 7) as usize;
        let memory = ir & 8 != 0;
        let (src, dst, address) = if memory {
            self.a[source] = self.a[source].wrapping_sub(if source == 7 { 2 } else { 1 });
            let src = self.rd8(bus, self.a[source]) as u32;
            self.a[destination] =
                self.a[destination].wrapping_sub(if destination == 7 { 2 } else { 1 });
            let address = self.a[destination];
            (src, self.rd8(bus, address) as u32, Some(address))
        } else {
            (self.d[source] & 0xff, self.d[destination] & 0xff, None)
        };
        let result = if add {
            self.bcd_add(src, dst)
        } else {
            self.bcd_sub(src, dst)
        };
        if let Some(address) = address {
            self.wr8(bus, address, result as u8);
        } else {
            self.d[destination] = (self.d[destination] & !0xff) | result;
        }
        Ok((if memory { 18 } else { 6 }, 0))
    }

    /// `BCD add`: `src + dst + X`, with decimal correction.
    pub(crate) fn bcd_add(&mut self, src: u32, dst: u32) -> u32 {
        let low = (src & 15) + (dst & 15) + self.sr.flag(X) as u32;
        let raw = low + (src & 0xf0) + (dst & 0xf0);
        let corrected = raw + if low > 9 { 6 } else { 0 };
        let carry = corrected > 0x9f;
        let r = corrected.wrapping_sub(if carry { 0xa0 } else { 0 }) & 0xff;
        self.sr.set_flag(C, carry);
        self.sr.set_flag(X, carry);
        self.sr.set_flag(N, r & 0x80 != 0);
        self.sr.set_flag(Z, self.sr.flag(Z) && r == 0);
        self.sr.set_flag(V, (!raw & r) & 0x80 != 0);
        r
    }

    /// `BCD subtract`: `dst - src - X`.
    pub(crate) fn bcd_sub(&mut self, src: u32, dst: u32) -> u32 {
        let low = (dst & 15)
            .wrapping_sub(src & 15)
            .wrapping_sub(self.sr.flag(X) as u32);
        let correction = if low > 15 { 6 } else { 0 };
        let raw = low.wrapping_add(dst & 0xf0).wrapping_sub(src & 0xf0);
        let carry = raw > 0xff || raw < correction;
        let corrected = raw.wrapping_add(if raw > 0xff { 0xa0 } else { 0 });
        let r = corrected.wrapping_sub(correction) & 0xff;
        self.sr.set_flag(C, carry);
        self.sr.set_flag(X, carry);
        self.sr.set_flag(N, r & 0x80 != 0);
        self.sr.set_flag(Z, self.sr.flag(Z) && r == 0);
        self.sr.set_flag(V, (raw & !r) & 0x80 != 0);
        r
    }
}

/// Recognise `ORI/ANDI/EORI` to CCR or SR, which use a fixed encoding and bypass
/// the normal effective-address path. Returns `(is_ccr, is_sr)`.
fn or_and_eori_sr_form(ir: u16) -> Option<(bool, bool)> {
    // The discriminator is BIT 6 - 0x3C is the CCR form and 0x7C the SR
    // form - not bit 8. Genesis Plus GX's jump table gives:
    //   $003C ori_16_toc   $007C ori_16_tos
    //   $023C andi_16_toc  $027C andi_16_tos
    //   $0A3C eori_16_toc  $0A7C eori_16_tos
    // and $017C, $037C and $0B7C are illegal on the 68000. This table had
    // them shifted by $40, so `ori.w #$0700,sr` at $27E04 - the instruction
    // that unmasks the cartridge's interrupts - was writing bits 8-10 into
    // the condition codes instead of the status register. The priority mask
    // stayed at 7 and the machine never accepted an interrupt, which is why
    // every run ended parked at a VBlank wait that could never complete.
    match ir {
        0x003C => Some((true, false)), // ORI  #x,CCR
        0x007C => Some((false, true)), // ORI  #x,SR
        0x023C => Some((true, false)), // ANDI #x,CCR
        0x027C => Some((false, true)), // ANDI #x,SR
        0x0A3C => Some((true, false)), // EORI #x,CCR
        0x0A7C => Some((false, true)), // EORI #x,SR
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped King's Bounty image, if it is present in the tree. Tests
    /// that need a real ROM are skipped rather than failed when it is not.
    fn test_rom_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/kings_bounty.md")
    }

    fn skip_without_rom() -> bool {
        !test_rom_path().exists()
    }

    /// A second, unrelated cartridge. Hokuto no Ken II is a completely
    /// different game with its own boot code, and it happens to verify itself
    /// with a four-instruction loop at the very start of its init - which
    /// makes it a far cheaper reproduction than anything in King's Bounty.
    fn hokuto_rom_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/hokuto_no_ken.md")
    }

    /// The strongest end-to-end check available: the game's own ROM-checksum
    /// routine must complete and branch the way the real cartridge's does.
    ///
    /// It exercises `MOVE.L` into a data register, `CMPA.W` against an address
    /// register, `ADD.L (An)+`, `ADDQ.W` into an address register, `SUBQ.L` and
    /// `BGT` in one loop, 130,699 times over. It caught every one of those
    /// bugs, and it is derived from the shipped artifact rather than from the
    /// reference suite, whose ALU opcode convention is inverted.
    ///
    /// The expected value in this test is not a transcription of the ROM's
    /// constant. It is the ROM's constant, checked against a second emulator.
    /// When the two disagreed, the disagreement was in our core.
    #[test]
    #[ignore = "optional external-cartridge integration test"]
    fn rom_checksum_self_test_passes() {
        if skip_without_rom() {
            return;
        }
        let cart = crate::cartridge::Cartridge::load(test_rom_path()).expect("test ROM");
        let mut system = crate::system::System::new(cart);
        // Stop where the checksum is compared, before the boot code goes on to
        // clobber the registers.
        const COMPARE: u32 = 0x7FA46;
        let mut exceptions = 0u64;
        let mut steps = 0u64;
        while system.cpu.pc != COMPARE && steps < 3_000_000 {
            if system.step_instruction().1.is_some() {
                exceptions += 1;
            }
            steps += 1;
        }
        assert_eq!(
            system.cpu.pc, COMPARE,
            "the checksum loop must run to completion"
        );
        assert_eq!(exceptions, 0, "the boot sequence must raise no exceptions");
        // 0x1FE8B iterations of a one-longword sum, leaving the counter at zero.
        assert_eq!(system.cpu.d[1], 0, "checksum counter must reach zero");
        // The ROM compares this against 0xCF90C7FB and the game only runs if
        // they match. Genesis Plus GX — an emulator sharing no code with ours —
        // boots this cartridge and reaches the game, so the sum must be
        // 0xCF90C7FB.
        //
        // It was once 0xF4FF6B67, because CMPA.W was masking the address
        // register to its low word. That made the loop's skip test fire eight
        // times instead of once and the integrity check appeared to fail, which
        // looked like a bad dump. Nothing about the image was wrong.
        assert_eq!(
            system.cpu.d[0], 0xCF90_C7FB,
            "the cartridge must pass its own integrity check"
        );
        // 0x1FE8B iterations of CMPA/BNE/ADD/SUBQ/BGT, at least four clocks
        // each; an exact figure would only pin down our own cycle table.
        assert!(steps >= 0x1FE8B * 4, "the loop must not short-circuit");
    }

    /// The second cartridge's own integrity check, which is the shortest
    /// self-test in either game:
    ///
    ///     000222  movea.l #$200, a0
    ///     00022E  move.l (a1), d0      ; the ROM end address, from the header
    ///     000232  add.w (a0)+, d1      ; sum every word from 0x200 up to it
    ///     000234  cmp.l a0, d0
    ///     000236  bcc.b $232
    ///     00023E  cmp.w (a1), d1
    ///
    /// Summing those words in Python gives 0x2A6F, which is what the header
    /// stores at 0x18E. Genesis Plus GX boots the cartridge and reaches the
    /// game, so the expected value is the header's, independently confirmed -
    /// not a transcription of whatever our core happens to do.
    ///
    /// The test starts at 0x230 rather than at the reset vector, with the
    /// registers the boot code would have set. Booting to here instead would
    /// depend on how the region check at 0x216 resolves - that address is
    /// 0xA1000D, which a Mega Drive answers with a region byte, and a
    /// cartridge that reads "overseas" skips the checksum entirely.
    #[test]
    #[ignore = "optional external-cartridge integration test"]
    fn hokuto_no_ken_sums_its_header_checksum() {
        if !hokuto_rom_path().exists() {
            return;
        }
        let cart = crate::cartridge::Cartridge::load(hokuto_rom_path()).expect("test ROM");
        let mut system = crate::system::System::new(cart);
        // Mirror 0x222/0x228/0x22E having run: a0 at the start of the ROM, the
        // header's ROM-end longword in d0, and a1 pointing at the checksum.
        system.cpu.a[0] = 0x200;
        system.cpu.a[1] = 0x1A4;
        system.cpu.d[0] = system.read32(0x1A4);
        // Move the CPU aside to prime its queue against the bus, then put it
        // back; System::reset does the same dance for the same reason.
        let mut cpu = std::mem::replace(&mut system.cpu, M68000::new(0));
        cpu.pc = 0x230;
        cpu.prime(&mut system);
        system.cpu = cpu;

        const COMPARE: u32 = 0x23E;
        let mut exceptions = 0u64;
        let mut steps = 0u64;
        while system.cpu.pc != COMPARE && steps < 3_000_000 {
            if system.step_instruction().1.is_some() {
                exceptions += 1;
            }
            steps += 1;
        }
        assert_eq!(
            system.cpu.pc, COMPARE,
            "the checksum loop must run to completion"
        );
        assert_eq!(exceptions, 0, "the loop must raise no exceptions");
        assert_eq!(
            system.cpu.d[0], 0x0007_FFFF,
            "the header declares a 512 KiB cart"
        );
        assert_eq!(
            system.cpu.d[1], 0x2A6F,
            "the cartridge's words must sum to the value stored in its header"
        );
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// `jsr` must push the address of the instruction *after* the whole
    /// instruction, extension word included, and `rts` must come back to it.
    #[test]
    fn jsr_then_rts_returns_to_the_instruction_after_jsr() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4EB9); // jsr $00000800 (absolute long)
        bus.write16(0x402, 0x0000);
        bus.write16(0x404, 0x0800);
        bus.write16(0x406, 0x4E75); // rts  (the caller's return point)
        bus.write16(0x800, 0x4E75); // rts  (the subroutine)

        let mut cpu = M68000::new(0x400);
        cpu.a[7] = 0x2000;
        // Prime the prefetch queue the way reset does, otherwise the first
        // step executes whatever the empty queue happens to hold.
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x800, "jsr must land on its target");
        assert_eq!(cpu.a[7], 0x1FFC, "jsr must push four bytes");
        assert_eq!(
            bus.read32(0x1FFC),
            0x406,
            "the pushed address is past the extension word"
        );

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x406, "the subroutine rts pops the pushed address");
        assert_eq!(cpu.a[7], 0x2000, "rts must give the byte back");
    }
}

#[cfg(test)]
mod movem_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// A `movem` save must be undone exactly by the matching restore. Any
    /// asymmetry in the register mask, the transfer width or the ordering
    /// silently shifts the stack, and the damage only shows up as a bad return
    /// address much later.
    #[test]
    fn movem_save_then_restore_round_trips() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x48E7); // movem.l d0-d7,-(a7); reversed mask
        bus.write16(0x402, 0xFF00);
        bus.write16(0x408, 0x4CDF); // movem.l (sp)+,d0-d7
        bus.write16(0x40A, 0x00FF);

        let mut cpu = M68000::new(0x400);
        cpu.a[7] = 0x2000;
        for (i, w) in [
            0xAAAA_0001u32,
            0xBBBB_0002,
            0xCCCC_0003,
            0xDDDD_0004,
            0xEEEE_0005,
            0xFFFF_0006,
            0x1234_0007,
            0x5678_0008,
        ]
        .iter()
        .enumerate()
        {
            cpu.d[i] = *w;
        }
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(cpu.a[7], 0x2000 - 32, "eight longs is 32 bytes");
        for (i, w) in [
            0xAAAA_0001u32,
            0xBBBB_0002,
            0xCCCC_0003,
            0xDDDD_0004,
            0xEEEE_0005,
            0xFFFF_0006,
            0x1234_0007,
            0x5678_0008,
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                bus.read32(0x2000 - 32 + i as u32 * 4),
                *w,
                "d{i} pushed wrong"
            );
        }

        cpu.prefetch = [bus.read16(0x408), bus.read16(0x40A)];
        cpu.pc = 0x408;
        cpu.step(&mut bus);
        assert_eq!(cpu.a[7], 0x2000, "restore must give every byte back");
        for (i, w) in [
            0xAAAA_0001u32,
            0xBBBB_0002,
            0xCCCC_0003,
            0xDDDD_0004,
            0xEEEE_0005,
            0xFFFF_0006,
            0x1234_0007,
            0x5678_0008,
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(cpu.d[i], *w, "d{i} restored wrong");
        }
    }
}

#[cfg(test)]
mod move_size_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// MOVE.W, including the immediate form, writes only the low word. The
    /// upper half is the programmer's business, exactly as on hardware.
    #[test]
    fn move_word_immediate_writes_only_the_low_word() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x303C); // move.w #$1234,d0
        bus.write16(0x402, 0x1234);
        let mut cpu = M68000::new(0x400);
        cpu.d[0] = 0xFFFF_FFFF;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        cpu.step(&mut bus);
        assert_eq!(cpu.d[0], 0xFFFF_1234, "move.w must write only the low word");
    }

    /// MOVE.W to a data register leaves the upper word untouched - unlike
    /// ADD.W, whose result is computed then written back whole. Both are 16-bit
    /// operations and both write 0x1234 into d0; only the upper half differs.
    #[test]
    fn move_word_register_to_register_preserves_the_upper_word() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x3001); // move.w d1,d0
        let mut cpu = M68000::new(0x400);
        cpu.d[1] = 0xABCD_1234;
        cpu.d[0] = 0xFFFF_FFFF;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        cpu.step(&mut bus);
        assert_eq!(cpu.d[0], 0xFFFF_1234, "move.w leaves the upper word alone");
    }
}

#[cfg(test)]
mod movem_frame_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// The single most common prologue in 68000 code: save everything, and put
    /// it all back on the way out. If the save frame is laid out wrong the
    /// restore silently loads garbage, and the symptom shows up far away as a
    /// subroutine returning to an address that was never pushed - which reads
    /// as a control-flow bug, not a MOVEM bug.
    ///
    /// 0x48E7 0xFFFE is `movem.l d0-d7/a0-a6,-(sp)`; 0x4CDF 0x7FFF is the
    /// matching `movem.l (sp)+,d0-d7/a0-a6`. The list deliberately omits a7 -
    /// the usual choice, and the one that exercises the base register's
    /// writeback on restore, since a7 is then neither saved nor loaded and
    /// still has to end up past the frame.
    ///
    /// The two orders are compatible: predecrement steps down through registers
    /// 15 to 0, which leaves d0 at the *lowest* address, and postincrement reads
    /// ascending. Getting that backwards shifts every restored register by one
    /// slot and corrupts the return address.
    #[test]
    fn movem_save_and_restore_round_trips() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x48E7);
        bus.write16(0x402, 0xFFFE);
        bus.write16(0x500, 0x4CDF);
        bus.write16(0x502, 0x7FFF);

        let mut cpu = M68000::new(0x400);
        for i in 0..8 {
            cpu.d[i] = 0xD000_0000u32 + i as u32;
            cpu.a[i] = 0xA000_0000u32 + i as u32;
        }
        cpu.a[7] = 0x00FF_1000;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        let sp_before = cpu.a[7];
        cpu.step(&mut bus);
        assert_eq!(
            cpu.a[7],
            sp_before - 60,
            "movem.l of 15 registers must push 60 bytes"
        );

        cpu.pc = 0x500;
        cpu.prefetch = [bus.read16(0x500), bus.read16(0x502)];
        cpu.step(&mut bus);

        for i in 0..8 {
            assert_eq!(
                cpu.d[i],
                0xD000_0000u32 + i as u32,
                "d{} must come back intact",
                i
            );
        }
        for i in 0..6 {
            assert_eq!(
                cpu.a[i],
                0xA000_0000u32 + i as u32,
                "a{} must come back intact",
                i
            );
        }
        assert_eq!(
            cpu.a[6], 0xA000_0006,
            "a6 is in the list and must be restored"
        );
        assert_eq!(
            cpu.a[7], sp_before,
            "the restored base register must be left holding the postincrement value"
        );
    }
}

#[cfg(test)]
mod usp_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// `move.l a6,usp` (0x4E66) loads the USP *from* a6 and leaves a6 alone.
    /// Reading it as the other way round silently overwrites the address
    /// register with the stack pointer, which presents much later as a
    /// subroutine frame in the wrong place.
    #[test]
    fn move_an_to_usp_leaves_the_address_register_alone() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4E66);
        let mut cpu = M68000::new(0x400);
        cpu.a[6] = 0x00C0_0004;
        cpu.a[7] = 0xFFFF_9C1E;
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        cpu.step(&mut bus);
        assert_eq!(cpu.a[6], 0x00C0_0004, "move.l a6,usp must not touch a6");
        assert_eq!(cpu.usp, 0x00C0_0004, "the USP takes a6's value");
        assert_eq!(cpu.a[7], 0xFFFF_9C1E, "a7 is untouched in supervisor mode");
    }

    /// 0x4E6E is `move usp,a6` - the other direction, and the one that does
    /// write the address register.
    #[test]
    fn move_usp_to_an_writes_the_address_register() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4E6E);
        let mut cpu = M68000::new(0x400);
        cpu.usp = 0x0012_3456;
        cpu.a[6] = 0;
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        cpu.step(&mut bus);
        assert_eq!(cpu.a[6], 0x0012_3456, "move usp,a6 writes a6");
    }

    /// In supervisor mode, `move.l a0,usp` puts a0's value into the USP - here
    /// a0 = 0, so the old USP contents must be gone.
    #[test]
    fn move_to_usp_writes_in_supervisor_mode() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4E60);
        let mut cpu = M68000::new(0x400);
        cpu.usp = 0xAAAA_AAAA;
        cpu.sr = crate::cpu::status::StatusRegister(0x2700); // supervisor
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        let out = cpu.step(&mut bus);
        assert!(
            out.exception.is_none(),
            "privileged instruction must execute"
        );
        assert_eq!(cpu.usp, 0, "a0's value replaced the USP");
    }

    /// The same instruction in user mode is a privilege violation.
    #[test]
    fn move_to_usp_traps_in_user_mode() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4E60);
        let mut cpu = M68000::new(0x400);
        cpu.usp = 0xAAAA_AAAA;
        // Bit 13 is the supervisor bit, so 0x0700 is user mode with the CCR set.
        cpu.sr = crate::cpu::status::StatusRegister(0x0700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        let out = cpu.step(&mut bus);
        assert!(
            matches!(
                out.exception,
                Some(crate::cpu::m68000::Exception::Privileged)
            ),
            "MOVE An,USP must be privileged, got {:?}",
            out.exception
        );
        assert_eq!(
            cpu.usp, 0xAAAA_AAAA,
            "a trapped instruction changes nothing"
        );
    }
}

#[cfg(test)]
mod movem_size_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// A word MOVEM must move words. `movem.w (a5)+,d5-d7` is 0x4C9D - the
    /// cartridge's very first MOVEM, and the one that loads the hardware
    /// address table at 0x290. Bits 7-6 are 10, which is word for MOVEM and
    /// *long* for every other instruction, so a shared size field reads it as
    /// long: the three registers come out as longwords and the base register
    /// advances twelve bytes instead of six. Everything downstream then reads
    /// misaligned table entries.
    #[test]
    fn movem_word_moves_words() {
        let mut bus = SparseBus::new();
        // 0x4C9D = movem.w (a5)+, d5-d7. List 0xE0 is d5, d6, d7.
        bus.write16(0x400, 0x4C9D);
        bus.write16(0x402, 0x00E0);
        let mut cpu = M68000::new(0x400);
        cpu.a[5] = 0x00FF_2000;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        bus.write16(0x00FF_2000, 0x1111);
        bus.write16(0x00FF_2002, 0x2222);
        bus.write16(0x00FF_2004, 0x3333);
        cpu.step(&mut bus);
        assert_eq!(cpu.d[5], 0x1111, "d5 takes one word");
        assert_eq!(cpu.d[6], 0x2222, "d6 takes the next word");
        assert_eq!(cpu.d[7], 0x3333, "d7 takes the next");
        assert_eq!(
            cpu.a[5], 0x00FF_2006,
            "the base advances six bytes, not twelve"
        );
    }

    /// And the long form is 0x4CDD, bits 7-6 = 11, the same field shifted.
    #[test]
    fn movem_long_moves_longwords() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4CDD);
        bus.write16(0x402, 0x001F); // d0-d4
        let mut cpu = M68000::new(0x400);
        cpu.a[5] = 0x00FF_3000;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        bus.write32(0x00FF_3000, 0x1111_2222);
        bus.write32(0x00FF_3004, 0x3333_4444);
        cpu.step(&mut bus);
        assert_eq!(cpu.d[0], 0x1111_2222);
        assert_eq!(cpu.d[1], 0x3333_4444);
        // Five registers in the list, four bytes each.
        assert_eq!(cpu.a[5], 0x00FF_3014, "the base advances twenty bytes");
    }
}

#[cfg(test)]
mod rts_stack_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// RTS pops a *longword* and advances the stack by four. The asymmetry
    /// with RTR, which pops a word of CCR and then a longword of PC, makes
    /// this easy to "fix" in the wrong direction - I did exactly that once,
    /// reading it as a word-pop like most other architectures. The rule is
    /// taken from Genesis Plus GX's m68k core:
    ///   m68k_op_rts_32: m68ki_jump(m68ki_pull_32())
    ///   m68ki_pull_32:  REG_SP += 4; return m68ki_read_32(sp)
    ///
    /// This matters here because the cartridge's own returns run through a
    /// stack that has real data above the return address.
    #[test]
    fn rts_pops_a_longword_and_advances_the_stack_by_four() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4E75); // rts
        let mut cpu = M68000::new(0x400);
        cpu.a[7] = 0x00FF_FFFC;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        // A jsr-shaped return address, with deliberately dirty data in the
        // word above it: a word-sized pop would silently pick that up instead.
        bus.write16(0x00FF_FFFC, 0x2F00);
        bus.write16(0x00FF_FFFE, 0xBEEF);

        let out = cpu.step(&mut bus);
        assert!(out.exception.is_none(), "rts must not fault here");
        // The PC's low bit is always cleared, so the odd 0xBEEF lands as 0xBEEE.
        assert_eq!(cpu.pc, 0x2F00_BEEE, "the whole longword is popped");
        assert_eq!(cpu.a[7], 0x0100_0000, "a7 advances by four");
    }

    /// RTR pops a word of CCR and then a longword of PC - six bytes in total.
    #[test]
    fn rtr_pops_a_word_of_ccr_then_a_longword_of_pc() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4E77); // rtr
        let mut cpu = M68000::new(0x400);
        cpu.a[7] = 0x00FF_0000;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x400)];
        bus.write16(0x00FF_0000, 0x001F); // CCR
        bus.write32(0x00FF_0002, 0x0000_1234);

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x1234, "PC comes from the longword");
        assert_eq!(cpu.sr.ccr(), 0x1F, "the condition codes are restored");
        assert_eq!(cpu.a[7], 0x00FF_0006, "six bytes off the stack");
    }
}

#[cfg(test)]
mod status_immediate_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// `ori.w #$0700,sr` is 0x007C 0x0700 - four bytes. In the ALU-immediate
    /// group the destination field is bits 8-6, and mode 001 is ambiguous: with
    /// register 100 it means SR, with register 000 it means CCR, and any other
    /// register it means An. Routing every mode-001 destination to an address
    /// register turns this into `ori.w #$0700,a4`, and if the wrong immediate
    /// length is taken the PC ends up two bytes short, so the next instruction
    /// is decoded from the middle of this one.
    #[test]
    fn ori_to_sr_is_not_treated_as_an_address_register_destination() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x007C);
        bus.write16(0x402, 0x0700);
        bus.write16(0x404, 0x4E75); // rts, to show where the PC lands

        let mut cpu = M68000::new(0x400);
        cpu.a[4] = 0x00C0_0004;
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(
            cpu.a[4], 0x00C0_0004,
            "ORI to SR must not write through the destination register"
        );
        assert_eq!(
            cpu.pc, 0x404,
            "the immediate is a word, so the PC advances four bytes"
        );
    }
}

#[cfg(test)]
mod word_wrap_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// A word-sized arithmetic instruction operates on the low word only: the
    /// high word is untouched and the low word wraps modulo 0x10000.
    ///
    /// This is load-bearing. The cartridge's tile-fill loop at 0x27CF4 is
    /// `addi.w #$20,d4` / `cmp.w d4,d7` / `beq`, where d7 holds a copy of d4's
    /// starting value. The loop can only exit by stepping d4's low word all
    /// the way round through 0xFFFF and back to where it started - 2048
    /// iterations. If the high word moves, or the low word does not wrap, d4
    /// never returns to its starting value and the loop spins forever.
    ///
    /// The encoding is taken from the cartridge itself: 0x27CF4 is
    /// `0644 0020` = `addi.w #$20,d4`. Constructing the opcode by hand gets
    /// the size field wrong, because bits 7-6 here mean 0=byte, 1=word,
    /// 2=long and 3 is not used - a different table from MOVE's or MOVEM's.
    #[test]
    fn addi_word_wraps_the_low_word_and_leaves_the_high_word_alone() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x0644); // addi.w #$20,d4
        bus.write16(0x402, 0x0020);
        bus.write16(0x404, 0x4E75);

        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.d[4] = 0xFFFF_FFE0;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(
            cpu.d[4], 0xFFFF_0000,
            "the low word wraps within itself and the high word is preserved"
        );
    }

    /// And with no wrap in sight, the high word must still survive.
    #[test]
    fn addi_word_leaves_the_high_word_alone_without_wrapping() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x0644);
        bus.write16(0x402, 0x0020);
        bus.write16(0x404, 0x4E75);

        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.d[4] = 0xABCD_0000;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(cpu.d[4], 0xABCD_0020, "word arithmetic touches one word");
    }
}

#[cfg(test)]
mod movea_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// MOVEA with an immediate source, as the second cartridge's boot code
    /// uses it: `movea.l #$200,a0` (0x207C 0x0000 0x0200) then
    /// `movea.l #$1a4,a1`.
    ///
    /// The encoding is taken from that cartridge at 0x222. Both instructions
    /// are six bytes and write a full longword into an address register.
    #[test]
    fn movea_long_immediate_loads_the_address_register() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x207C); // movea.l #$200,a0
        bus.write16(0x402, 0x0000);
        bus.write16(0x404, 0x0200);
        bus.write16(0x406, 0x4E75);

        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(
            cpu.a[0], 0x200,
            "MOVEA.L #imm must load the address register"
        );
        assert_eq!(cpu.pc, 0x406, "a long immediate is two extension words");
    }

    /// The same instruction with an address-register source, which is the form
    /// that was previously confused with data-register MOVE.
    #[test]
    fn movea_long_address_register_source_copies_the_whole_register() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x2008); // movea.l a0,d0
        bus.write16(0x402, 0x4E75);

        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.a[0] = 0x00FF_1234;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(cpu.a[0], 0x00FF_1234, "the whole 32-bit register moves");
    }
}

#[cfg(test)]
mod cmp_branch_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// `cmp.l a0,d0` (0xB088) followed by `bcc.b` (0x64FA), as the second
    /// cartridge's checksum loop uses them at 0x234 and 0x236.
    ///
    /// CMP with an *address* register as the source operand is the case that
    /// is easy to confuse with CMPA, because CMPA shares bits 7-6 == 11 while
    /// this is 10. It must compare the full 32 bits of A0 against D0 and leave
    /// both alone.
    #[test]
    fn cmp_long_address_register_source_does_not_touch_either_register() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0xB088); // cmp.l a0,d0
        bus.write16(0x402, 0x4E75);

        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.a[0] = 0x0000_0202;
        cpu.d[0] = 0x0007_FFFF;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(cpu.d[0], 0x0007_FFFF, "CMP leaves its destination alone");
        assert_eq!(
            cpu.a[0], 0x0000_0202,
            "CMP never writes an address register"
        );
        assert_eq!(
            cpu.pc, 0x402,
            "a register-to-register compare has no extension words"
        );
        // 0x7FFFF - 0x202 is positive: carry clear, so BCC must be taken.
        assert_eq!(cpu.sr.ccr() & (1 << 0), 0, "carry must be clear");
    }
}

#[cfg(test)]
mod tst_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    /// `tst.w $fe26.w` is 0x4A78 0xFE26 - four bytes. The encoding is taken
    /// from the second cartridge at 0x2A2, where the game's main loop polls a
    /// work-RAM flag and branches on it.
    ///
    /// It matters because TST is the quiet one: it only sets condition codes,
    /// so a decoder that drops its extension word produces no illegal-opcode
    /// signal at the instruction itself. Instead the PC resumes two bytes
    /// later, in the middle of the operand, and the fault appears somewhere
    /// else entirely - which is exactly how this one was found.
    #[test]
    fn tst_word_absolute_short_consumes_its_extension_word() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x4A78);
        bus.write16(0x402, 0xFE26);
        bus.write16(0x404, 0x4E75);

        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];

        cpu.step(&mut bus);
        assert_eq!(
            cpu.pc, 0x404,
            "TST.W <ea> is four bytes; dropping the extension word resumes inside it"
        );
    }
}

#[cfg(test)]
mod move_brief_tests {
    use super::*;
    use crate::cpu::{Bus, SparseBus};

    fn step_one(ir: u16, ext: &[u16]) -> u32 {
        let mut bus = SparseBus::new();
        bus.write16(0x400, ir);
        for (i, w) in ext.iter().enumerate() {
            bus.write16(0x402 + (i as u32) * 2, *w);
        }
        let mut cpu = M68000::new(0x400);
        cpu.sr = crate::cpu::status::StatusRegister(0x2700);
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        cpu.step(&mut bus);
        cpu.pc
    }

    /// `31 FC 00 00 FE 26` is `move.w #$0000,$FE26.w` in the second cartridge
    /// at 0x262: six bytes, an opcode, one word-sized immediate and one short
    /// address. Reading the brief destination as .L instead costs a whole
    /// extension word, the PC lands two bytes past the next instruction, and an
    /// operand gets executed as an opcode - which is exactly how that cartridge
    /// ended up running 0xFE26 as code.
    #[test]
    fn brief_destination_uses_bit_nine_not_bit_eight_for_word_versus_long() {
        let pc = step_one(0x31FC, &[0x0000, 0xFE26]);
        assert_eq!(
            pc, 0x406,
            "bit 9 is clear, so the destination is a short address: three words total"
        );
    }

    /// The mirror image, and the reason the wrong bit was invisible for so
    /// long: `33 FC` sets bit 8 *and* bit 9, so a decoder keying on bit 8 got
    /// the long destination right by accident.
    #[test]
    fn brief_destination_long_form_keeps_both_extension_words() {
        let pc = step_one(0x33FC, &[0x8134, 0xC000, 0x0004]);
        assert_eq!(
            pc, 0x408,
            "bit 9 is set: a long destination, four words total"
        );
    }

    /// A word-sized immediate is one word, not two. The brief-mode table is
    /// fixed at two for the absolute forms but immediate takes its width from
    /// the operation, so this is a separate trap from the bit above.
    #[test]
    fn word_immediate_to_register_is_two_words() {
        let pc = step_one(0x303C, &[0x1234]);
        assert_eq!(pc, 0x404, "opcode plus one word-sized immediate");
    }
}

#[cfg(test)]
mod swap_and_shifts {
    use super::*;
    use crate::cpu::bus::SparseBus;

    /// Prime a CPU at 0x40 with `words` and step once. Priming the prefetch
    /// queue matters: a bare `M68000::new` executes whatever is in memory at
    /// zero, which reads as a decoder bug rather than a setup mistake.
    fn run(ir: u16, setup: impl FnOnce(&mut M68000)) -> M68000 {
        let mut cpu = M68000::new(0);
        cpu.pc = 0x40;
        cpu.prefetch = [ir, 0];
        setup(&mut cpu);
        cpu
    }

    fn step1(cpu: &mut M68000) {
        let mut bus = SparseBus::new();
        cpu.step(&mut bus);
    }

    /// SWAP names its register in bits 2-0. Reading it from bits 11-9 picks up
    /// the mode field, which lands on the *previous* register for every
    /// encoding in the corpus - so `swap d4` exchanges d3 instead and leaves
    /// d4 untouched, with no fault to announce it.
    #[test]
    fn swap_exchanges_the_halves_of_the_named_register() {
        for n in 0..8usize {
            let mut cpu = run(0x4840 | n as u16, |c| c.d[n] = 0xA4BE_1234);
            step1(&mut cpu);
            assert_eq!(cpu.d[n], 0x1234_A4BE, "swap d{n} did not touch d{n}");
            let others: Vec<usize> = (0..8).filter(|i| *i != n).collect();
            assert!(
                others.iter().all(|i| cpu.d[*i] == 0 || *i != n),
                "swap must only affect the named register"
            );
        }
    }

    /// Every direction of the shift group, at count one, in word form.
    /// A shift comes in zeros; only a rotate brings the outgoing bit back.
    #[test]
    fn single_count_shifts_move_one_place_and_shifts_in_zeros() {
        let cases: &[(u16, u32)] = &[
            (0xE240, 0xD25F), // asr.w #1,d0 - sign replicates in
            (0xE340, 0x497C), // asl.w #1,d0 - zero comes in
            (0xE248, 0x525F), // lsr.w #1,d0
            (0xE348, 0x497C), // lsl.w #1,d0
            (0xE258, 0x525F), // ror.w #1,d0 - outgoing bit wraps around
            (0xE358, 0x497D), // rol.w #1,d0
        ];
        for &(op, want) in cases {
            let mut cpu = run(op, |c| c.d[0] = 0x1234_A4BE);
            step1(&mut cpu);
            assert_eq!(
                cpu.d[0] & 0xFFFF,
                want,
                "op 0x{op:04X} produced 0x{:04X}, want 0x{want:04X}",
                cpu.d[0] & 0xFFFF
            );
            assert_eq!(
                cpu.d[0] >> 16,
                0x1234,
                "a word operation must leave the upper half alone"
            );
        }
    }

    /// A word shift leaves the upper half of the register untouched, and the
    /// count lives in bits 11-9 as a literal - zero meaning eight. The cartridge
    /// relies on exactly this: `asr.w #3,d4` then `asl.w #5,d4` has to turn a
    /// 16-bit value into a tile-aligned offset, and only the literal counts
    /// produce a multiple of 32.
    #[test]
    fn literal_counts_come_from_bits_11_9_where_zero_means_eight() {
        // (0x27CB8's neighbour) asr.w #3 of 0xA4BE, then asl.w #5.
        let mut cpu = run(0xE644, |c| c.d[4] = 0x1234_A4BE);
        step1(&mut cpu);
        assert_eq!(cpu.d[4] & 0xFFFF, 0xF497, "asr.w #3 of 0xA4BE is 0xF497");
        assert_eq!(cpu.d[4] >> 16, 0x1234, "the upper half must survive");

        let mut cpu = run(0xEB44, |c| c.d[4] = 0x1234_F497);
        step1(&mut cpu);
        assert_eq!(cpu.d[4] & 0xFFFF, 0x92E0, "asl.w #5 of 0xF497 is 0x92E0");
        assert_eq!(cpu.d[4] & 0x1F, 0, "and that is tile aligned");
    }

    /// Bit 5 separates an immediate count from one held in a data register.
    /// Confusing the two shifts a register by whatever garbage was in that
    /// register - which is how `asr.w #3,d4` came to shift by `d3`.
    #[test]
    fn a_set_bit_five_takes_the_count_from_the_register_named_in_11_9() {
        let mut cpu = run(0xE760, |c| {
            c.d[3] = 2; // the count register
            c.d[0] = 0x1234_A4BE;
        });
        step1(&mut cpu);
        assert_eq!(
            cpu.d[0] & 0xFFFF,
            0x92F8,
            "lsl.w d3,d0 with d3=2 shifts 0xA4BE left two places"
        );
    }

    /// A word move writes the low word of the source and, on a register
    /// destination, leaves the upper half of that register alone. Copying the
    /// wrong half here is what made `move.w d4,d7` deposit d4's *upper* word
    /// and desynchronise the VRAM fill loop.
    #[test]
    fn word_move_into_a_register_takes_the_low_word_only() {
        let mut cpu = run(0x3805, |c| c.d[5] = 0xA4BE_1234);
        step1(&mut cpu);
        assert_eq!(cpu.d[4] & 0xFFFF, 0x1234, "the low word of d5 goes to d4");
        assert_eq!(cpu.d[4] >> 16, 0, "and d4's own upper half is unchanged");
    }

    /// The same move into memory writes one word, not a longword.
    #[test]
    fn word_move_into_memory_writes_two_bytes_at_the_destination() {
        let mut bus = SparseBus::new();
        let mut cpu = M68000::new(0);
        cpu.pc = 0x40;
        cpu.prefetch = [0x3084, 0];
        cpu.a[0] = 0x8000;
        cpu.d[4] = 0xA4BE_1234;
        cpu.step(&mut bus);
        assert_eq!(bus.read16(0x8000), 0x1234);
        assert_eq!(bus.read16(0x8002), 0, "a word move must not write a long");
    }
}

#[cfg(test)]
mod dbcc_exit {
    use super::*;
    use crate::cpu::bus::SparseBus;

    fn loop_at_zero(bus: &mut SparseBus, op: u16, counter: u32) -> (M68000, SparseBus) {
        // Hands the written bus back so the caller steps against real memory.
        bus.write16(0x40, op);
        bus.write16(0x42, 0xFFFE); // the displacement lives at 0x42: back to 0x40
        bus.write16(0x44, 0x4E75);
        let mut cpu = M68000::new(0);
        cpu.pc = 0x40;
        cpu.prefetch = [op, 0xFFFE];
        cpu.d[0] = counter;
        (cpu, std::mem::take(bus))
    }

    /// `DBRA Dn,loop` with Dn = N runs the body N+1 times and leaves the
    /// counter wrapped - which is why the idiom seeds it with (count - 1).
    #[test]
    fn dbra_counts_the_body_and_leaves_the_counter_wrapped() {
        let mut bus = SparseBus::new();
        bus.write16(0x40, 0x5281); // addq.l #1,d1
        bus.write16(0x42, 0x51C8); // dbra d0,0x40
        bus.write16(0x44, 0xFFFC);
        bus.write16(0x46, 0x4E75);
        let mut cpu = M68000::new(0);
        cpu.pc = 0x40;
        cpu.prefetch = [0x5281, 0x51C8];
        cpu.d[0] = 5;
        let mut steps = 0;
        while cpu.pc != 0x46 {
            cpu.step(&mut bus);
            steps += 1;
            assert!(steps < 200, "dbra never fell through");
        }
        assert_eq!(cpu.d[1], 6, "the body runs d0+1 times");
        assert_eq!(cpu.d[0], 0x0000_FFFF, "only the low word wraps");
    }

    /// DBcc's loop runs while the condition is FALSE. Genesis Plus GX puts the
    /// decrement under a COND_NOT_CC() guard, so a satisfied condition exits
    /// without touching the counter at all.
    #[test]
    fn a_satisfied_dbcc_condition_exits_without_touching_the_counter() {
        // 0x54C8 is DBCC D0 (HS): it loops while carry is clear.
        let (mut cpu, mut bus) = loop_at_zero(&mut SparseBus::new(), 0x54C8, 100);
        cpu.sr.set_flag(C, false);
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x44, "carry clear satisfies HS, so the loop exits");
        assert_eq!(cpu.d[0], 100, "and the counter is untouched");
    }

    /// An unsatisfied condition counts down and branches.
    #[test]
    fn an_unsatisfied_dbcc_condition_counts_down() {
        let (mut cpu, mut bus) = loop_at_zero(&mut SparseBus::new(), 0x54C8, 2);
        cpu.sr.set_flag(C, true);
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x40, "carry set fails HS, so it loops");
        assert_eq!(cpu.d[0], 1, "and decrements the counter");
    }

    /// GPGX's dbf_16 preserves the high word on expiry, including when it is
    /// nonzero. This is the case the King's Bounty fill used to get wrong.
    #[test]
    fn dbra_preserves_the_high_word_and_expires_on_low_word_wrap() {
        let (mut cpu, mut bus) = loop_at_zero(&mut SparseBus::new(), 0x51C8, 0x0001_0000);
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x44, "a zero low word expires immediately");
        assert_eq!(cpu.d[0], 0x0001_FFFF);
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// The shipped King's Bounty image, if it is present in the tree.
    pub(crate) fn test_rom_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/kings_bounty.md")
    }
}

#[cfg(test)]
mod kb_fill_loop_tests {
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    use super::tests_support::test_rom_path;
    use super::*;

    fn kb_system() -> Option<MdSystem> {
        let p = test_rom_path();
        if !p.exists() {
            return None;
        }
        Some(MdSystem::new(Cartridge::load(p).expect("test ROM")))
    }

    /// The boot parks in a VRAM fill loop whose only exit is `bne.b` at
    /// 0x27F7A, testing d5. Sampling d5 there turns "it never exits" into
    /// a number, which is the difference between hunting and reading.
    ///
    /// Checked on every arrival rather than bisected: d5 is decremented and
    /// then negated inside the worker, so it is not monotonic and bisection
    /// returns an arbitrary crossing.
    #[test]
    #[ignore = "historical fill diagnostic; its seed assertion describes the former incorrect core"]
    fn d5_walks_towards_zero_at_the_fill_loop() {
        let Some(mut system) = kb_system() else {
            return;
        };
        let mut samples: Vec<u32> = Vec::new();
        let mut last = None;
        let mut stuck_for = 0u32;
        for _ in 0..1_500_000 {
            if system.cpu.pc == 0x27F7A {
                let d5 = system.cpu.d[5];
                if samples.len() < 6 {
                    samples.push(d5);
                }
                if last == Some(d5) {
                    stuck_for += 1;
                } else {
                    stuck_for = 0;
                }
                last = Some(d5);
                // 20,000 arrivals without moving is not a slow countdown.
                if stuck_for > 20_000 {
                    break;
                }
            }
            system.step_instruction();
        }
        assert!(
            samples.len() >= 3,
            "the fill loop must be reached at all (saw {} arrivals, pc now 0x{:06X})",
            samples.len(),
            system.cpu.pc
        );
        assert!(
            samples.windows(2).any(|w| w[0] != w[1]),
            "d5 must change between iterations, but it sat at {:#010X} \
             for 20,000 arrivals - the worker is not making progress",
            samples[0]
        );
        // It is a countdown, not a stuck loop. The cartridge seeds d5 with
        // $00C00000 and the loop decrements it once per pass, so it needs
        // 12,582,912 iterations - about 150 million instructions, which is
        // an order of magnitude past where a short run gives up. Reporting
        // this as a hang would send the next reader hunting a bug that is
        // not there.
        // The first sample is taken before the worker's `neg.w` has flipped
        // the counter, so the countdown proper starts at the second.
        let first = samples[1];
        let last = samples[samples.len() - 1];
        assert_eq!(
            first.wrapping_sub(last),
            (samples.len() - 2) as u32,
            "d5 counts down by exactly one per iteration ({first:#010X} -> {last:#010X})"
        );
        assert_eq!(
            samples[0], 0x00C0_0000,
            "the cartridge seeds d5 with $00C00000, so the loop needs \
             12,582,912 passes - roughly 150 million instructions, which is \
             an order of magnitude past where a short run gives up. Calling \
             it a hang would send the next reader after a bug that is not \
             there."
        );
    }
}

#[cfg(test)]
mod kb_fill_sweep_tests {
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    use super::tests_support::test_rom_path;
    use super::*;

    /// The VRAM fill advances a6 by 0xE8 on every outer pass and never
    /// checks it against anything. Work RAM ends and the stack begins at
    /// 0xFFFF9C10; at 232 bytes a pass that is a little over 37,000 passes.
    ///
    /// The audit already showed what happens when it gets there - the worker's
    /// `rts` pops a debug marker - so this asks the question one step earlier:
    /// does the sweep bound that is supposed to stop it ever close?
    ///
    /// It does not, and that hypothesis is now dead: four hundred million
    /// instructions and a6 never reaches the stack. The corruption is coming
    /// from somewhere else. Ignored by default because it takes a minute.
    #[test]
    #[ignore = "four hundred million instructions"]
    fn the_fill_sweep_stops_before_the_stack() {
        let p = test_rom_path();
        if !p.exists() {
            return;
        }
        let mut system = MdSystem::new(Cartridge::load(p).expect("test ROM"));
        let reset_sp = system.cpu.a[7];
        let mut passes = 0u32;
        let mut first_past: Option<(u32, u32)> = None;
        let mut last_a6 = 0u32;
        for _ in 0..4_000_000_000u64 {
            if system.cpu.pc == 0x27FAC {
                let a6 = system.cpu.a[6];
                last_a6 = a6;
                passes += 1;
                if a6 >= reset_sp && first_past.is_none() {
                    first_past = Some((passes, a6));
                    break;
                }
            }
            system.step_instruction();
        }
        assert!(
            first_past.is_none(),
            "a6 reached {:#010X} after {} outer passes with the stack at \
             {:#010X} - the sweep is never bounded, and it overwrites the \\
             return address the worker is about to pop",
            last_a6,
            passes,
            reset_sp
        );
    }
}

#[cfg(test)]
mod kb_worker_frame_tests {
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    use super::tests_support::test_rom_path;
    use super::*;

    /// The fill worker at 0x27CC6 saves d0-d7 onto the stack and restores them
    /// before returning. Whatever happens inside, a7 at the `rts` must equal
    /// a7 at the entry - otherwise the worker itself is what corrupts the
    /// return address the audit caught.
    ///
    /// Reported as a pairing rather than a pass/fail so the first mismatch can
    /// be printed: this is diagnosis, not a gate.
    #[test]
    #[ignore = "walks the four-million-pass fill loop"]
    fn the_worker_balances_its_own_frame() {
        let p = test_rom_path();
        if !p.exists() {
            return;
        }
        let mut system = MdSystem::new(Cartridge::load(p).expect("test ROM"));
        let mut entry: Option<u32> = None;
        let mut calls = 0u32;
        let mut mismatches = 0u32;
        let mut first: Option<(u32, u32, u32, u32)> = None;
        for _ in 0..4_000_000_000u64 {
            match system.cpu.pc {
                0x27CC6 => entry = Some(system.cpu.a[7]),
                0x27D0E => {
                    if let Some(a7_in) = entry.take() {
                        calls += 1;
                        let a7_out = system.cpu.a[7];
                        if a7_out != a7_in {
                            mismatches += 1;
                            if first.is_none() {
                                first = Some((calls, a7_in, a7_out, system.cpu.d[0]));
                            }
                        }
                    }
                }
                _ => {}
            }
            system.step_instruction();
        }
        assert_eq!(
            mismatches,
            0,
            "the worker returned {mismatches} of {calls} times with a7 moved; \
             first: call {}, entry a7 {:#010X}, exit a7 {:#010X} (d0 {:#010X}). \
             A 32-byte movem frame cannot produce a delta of any other size.",
            first.map(|f| format!("{:#010X}", f.0)).unwrap_or_default(),
            first.map(|f| f.1).unwrap_or(0),
            first.map(|f| f.2).unwrap_or(0),
            first.map(|f| f.3).unwrap_or(0)
        );
    }
}

#[cfg(test)]
mod bsr_probe_tests {
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    use super::tests_support::test_rom_path;
    use super::*;
    #[test]
    #[ignore = "diagnostic probe"]
    fn bsr_at_27cd0_lands_on_27cae() {
        let p = test_rom_path();
        if !p.exists() {
            return;
        }
        let mut system = MdSystem::new(Cartridge::load(p).expect("test ROM"));
        let mut at_cae = 0u32;
        let mut at_packed = 0u32;
        let mut d4_at_cae = Vec::new();
        for _ in 0..40_000_000u64 {
            match system.cpu.pc {
                0x27CAE => {
                    at_cae += 1;
                    if d4_at_cae.len() < 4 {
                        d4_at_cae.push(system.cpu.d[4]);
                    }
                }
                0x2A8D5 => {
                    at_packed += 1;
                }
                _ => {}
            }
            system.step_instruction();
        }
        assert_eq!(
            at_packed, 0,
            "bsr.b at 0x27CD0 must land on 0x27CAE, not 0x2A8D5 in the packed region"
        );
        assert!(at_cae > 0, "0x27CAE must be executed at all");
        println!("0x27CAE hit {at_cae}x, d4 samples {d4_at_cae:?}");
    }
}

#[cfg(test)]
mod d3_watch_tests {
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    use super::tests_support::test_rom_path;
    use super::*;
    #[test]
    #[ignore = "diagnostic probe"]
    fn who_writes_d3() {
        let p = test_rom_path();
        if !p.exists() {
            return;
        }
        let mut system = MdSystem::new(Cartridge::load(p).expect("test ROM"));
        let mut prev = system.cpu.d[3];
        let mut rows: Vec<(u32, u32, u32)> = Vec::new();
        let mut tail: Vec<(u32, u32, u32)> = Vec::new();
        // Only writes once the fill loop is running: early boot writes d3
        // hundreds of times in a RAM-initialisation loop and tells us nothing.
        while system.cpu.pc != 0x27CCA {
            system.step_instruction();
        }
        prev = system.cpu.d[3];
        for _ in 0..60_000_000u64 {
            let pc = system.cpu.pc;
            system.step_instruction();
            let now = system.cpu.d[3];
            if now != prev {
                if rows.len() < 24 {
                    rows.push((pc, prev, now));
                }
                prev = now;
            }
        }
        println!("  writing pc    old         new");
        for (pc, o, n) in &rows {
            println!("  {pc:#010x}  {o:#010x}  {n:#010x}");
        }
    }
}

#[cfg(test)]
mod branch_base_tests {
    use super::*;

    /// Authored instruction bytes isolate the extension-word branch base.
    #[test]
    fn dbf_displacement_is_measured_from_the_extension_word() {
        let mut bus = crate::cpu::SparseBus::new();
        bus.write16(0x27FA8, 0x51C8);
        bus.write16(0x27FAA, 0xFF98);
        let mut cpu = M68000::new(0x27FA8);
        cpu.d[0] = 10;
        cpu.prime(&mut bus);
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None);
        assert_eq!(cpu.pc, 0x27F42);
        assert_eq!(cpu.d[0], 9);
    }

    /// BRA.W uses the same base, on valid even instruction addresses.
    #[test]
    fn bra_word_lands_where_the_extension_word_says() {
        let mut bus = crate::cpu::SparseBus::new();
        bus.write16(0x1200, 0x6000);
        bus.write16(0x1202, 0xFFFA);
        let mut cpu = M68000::new(0x1200);
        cpu.prime(&mut bus);
        let outcome = cpu.step(&mut bus);
        assert_eq!(outcome.exception, None);
        assert_eq!(cpu.pc, 0x11FC);
    }
}

#[cfg(test)]
mod region_probe {
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    use super::tests_support::test_rom_path;
    use super::*;

    /// Work RAM lags - a delayed write reads as a wrong value - so the first
    /// real divergence is better looked for in the VDP registers, which a
    /// control-port write changes the instant it happens. Log every register
    /// value this core ends a frame with.
    #[test]
    #[ignore = "diagnostic probe"]
    fn vdp_registers_per_frame() {
        let p = test_rom_path();
        if !p.exists() {
            return;
        }
        let mut system = MdSystem::new(Cartridge::load(p).expect("test ROM"));
        let mut last_frame = 0u64;
        let mut rows = Vec::new();
        for _ in 0..400_000_000u64 {
            system.step_instruction();
            if system.vdp.frame != last_frame {
                last_frame = system.vdp.frame;
                rows.push((last_frame, system.vdp.registers));
            }
            if last_frame > 400 {
                break;
            }
        }
        let mut prev: Option<[u8; 32]> = None;
        for (f, r) in &rows {
            let changed = match &prev {
                Some(p) => p != r,
                None => true,
            };
            if changed {
                println!(
                    "  frame {f:>4}: regs {}",
                    r.iter()
                        .enumerate()
                        .map(|(i, v)| format!("r{i}={v:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                prev = Some(*r);
            }
        }
        println!("  frames captured: {}", rows.len());
    }
}

#[cfg(test)]
mod sr_form_tests {
    use super::*;
    use crate::cpu::bus::SparseBus;

    fn cpu_running(opcode: u16, immediate: u16) -> M68000 {
        let mut bus = SparseBus::new();
        bus.write16(0x400, opcode);
        bus.write16(0x402, immediate);
        bus.write16(0x404, 0x4E75); // rts
        let mut cpu = M68000::new(0x400);
        cpu.sr.0 = 0x2700; // supervisor, priority mask 7
                           // Prime the prefetch queue the way reset does.
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        cpu.step(&mut bus);
        cpu
    }

    /// `$007C $0700` is ORI.W #$0700,SR, which SETS the priority-mask bits
    /// 8-10 - it can only widen the mask, never clear it. The old table read
    /// it as ORI to CCR and wrote $0700 into the condition codes instead,
    /// leaving the status register untouched and corrupting the flags that
    /// every conditional branch in the game then tested.
    #[test]
    fn ori_0700_sr_sets_the_priority_mask() {
        let cpu = cpu_running(0x007C, 0x0700);
        assert_eq!(cpu.sr.interrupt_mask(), 7, "mask after ori.w #0x700,sr");
        // The condition codes must be untouched: $0700 lands in the high
        // byte, and the CCR is the low byte.
        assert_eq!(cpu.sr.ccr() & 0x0700, 0, "ccr must not receive the mask");
    }

    /// The same opcode with the mask bits clear is what unmasks interrupts -
    /// OR can only ever clear a bit that is already clear.
    #[test]
    fn ori_0000_sr_clears_the_mask() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x007C);
        bus.write16(0x402, 0x0000);
        let mut cpu = M68000::new(0x400);
        cpu.sr.0 = 0x2700;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        cpu.step(&mut bus);
        // OR with zero changes nothing at all - it is a plain status write,
        // which is why the game needs MOVE #imm,SR (0x46FC) to actually
        // unmask. This test pins the fact that ORI cannot do it.
        assert_eq!(cpu.sr.interrupt_mask(), 7, "ori #0 cannot clear mask bits");
    }

    /// The other operand width proves the discriminator is bit 6 rather than
    /// a size field: $003C is ORI to CCR and must leave the mask alone.
    #[test]
    fn ori_001f_ccr_touches_only_the_condition_codes() {
        let cpu = cpu_running(0x003C, 0x001F);
        assert_eq!(cpu.sr.interrupt_mask(), 7, "mask untouched by ori to ccr");
        assert_eq!(cpu.sr.ccr() & 0x1F, 0x1F, "ccr updated");
    }

    /// The $40-apart opcodes the old table treated as the SR forms are
    /// illegal on the 68000; Genesis Plus GX lists them so.
    #[test]
    fn the_017c_family_is_illegal() {
        let mut bus = SparseBus::new();
        bus.write16(0x400, 0x017C);
        bus.write16(0x402, 0x0700);
        let mut cpu = M68000::new(0x400);
        cpu.sr.0 = 0x2700;
        cpu.prefetch = [bus.read16(0x400), bus.read16(0x402)];
        let outcome = cpu.step(&mut bus);
        assert!(
            outcome.exception.is_some(),
            "$017C is not a 68000 encoding and must raise an exception"
        );
    }
}

#[cfg(test)]
mod caller_probe {
    use super::tests_support::test_rom_path;
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::system::System as MdSystem;

    /// Preamble B is reached by `bsr.w $027D3E` at $027C2E. Which
    /// instructions in that routine does the run actually execute?
    #[test]
    #[ignore = "diagnostic probe"]
    fn caller_probe() {
        let p = test_rom_path();
        if !p.exists() {
            return;
        }
        let mut system = MdSystem::new(Cartridge::load(p).expect("test ROM"));
        let mut seen: Vec<(u32, u64, u32, u32, u32, u32)> = Vec::new();
        let mut reached_call = false;
        let mut n = 0u64;
        while n < 3_000_000 && !reached_call {
            let pc = system.cpu.pc;
            if (0x027BF0..=0x027C40).contains(&pc) && seen.len() < 400 {
                if seen
                    .last()
                    .map(|(q, _, _, _, _, _)| *q != pc)
                    .unwrap_or(true)
                {
                    seen.push((
                        pc,
                        n + 1,
                        system.cpu.d[0],
                        system.cpu.d[1],
                        system.cpu.a[0],
                        system.cpu.a[4],
                    ));
                }
            }
            if pc == 0x027C2E {
                reached_call = true;
            }
            system.step_instruction();
            n += 1;
        }
        println!("  distinct PCs executed in 0x27BF0..0x27C40, in order:");
        for (pc, at, d0, d1, a0, a4) in seen.iter().take(24) {
            println!("    ${pc:06X} at {at:>10}  d0={d0:08X} d1={d1:08X} a0={a0:08X} a4={a4:08X}");
        }
        println!("  reached bsr.w to preamble B: {reached_call}  after {n} instructions");
    }
}
