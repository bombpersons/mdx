//! A disassembler for mdcore's own 68000 decoder.
//!
//! This exists because an agent reverse-engineering a cartridge needs to ask
//! the emulator what *it* thinks an instruction is, and a tool that disagrees
//! with the emulator is worse than no tool: a trace shows the bytes it
//! executed while this shows what it believes them to mean, so any
//! disagreement names a decoder bug immediately.
//!
//! The tables here are deliberately kept in step with `exec.rs`. Where the
//! decode is subtle - LEA's brief-mode table, MOVE's immediate source bit,
//! the shift group's two layouts, MOVEM's size field - both sides carry a
//! comment saying so.
//!
//! An instruction this module cannot decode is reported as `??` with the
//! length it is willing to commit to, which for an unknown opcode is two
//! bytes. Guessing a length is how a disassembler sends you reading data.

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Mnemonic and operands, e.g. `move.l d0,(a1)`.
    pub text: String,
    /// Bytes consumed, always at least two.
    pub length: u8,
    /// True when this module is confident about `text`.
    pub decoded: bool,
}

/// A cursor over a byte slice that tracks how many bytes were consumed.
struct Cur<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// The address the instruction started at, for PC-relative printing.
    origin: u32,
}

impl<'a> Cur<'a> {
    fn new(bytes: &'a [u8], origin: u32) -> Self {
        Cur {
            bytes,
            pos: 0,
            origin,
        }
    }

    fn u8(&mut self) -> u8 {
        let v = self.bytes.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        v
    }

    fn u16(&mut self) -> u16 {
        let hi = self.u8() as u16;
        let lo = self.u8() as u16;
        (hi << 8) | lo
    }

    fn u32(&mut self) -> u32 {
        let a = self.u16() as u32;
        let b = self.u16() as u32;
        (a << 16) | b
    }

    fn s16(v: u16) -> i32 {
        v as i16 as i32
    }

    fn hex(v: u32) -> String {
        format!("0x{v:X}")
    }

    /// Absolute addresses print bare, the way they were written: `$FE26`.
    fn bare(v: u32) -> String {
        format!("${v:X}")
    }

    /// Words consumed so far, which is the instruction length once decoding
    /// finishes.
    fn len(&self) -> u8 {
        (self.pos as u8).max(2)
    }

    /// The address the *next* word would be read from. PC-relative forms
    /// measure from the address of the extension word itself, never from the
    /// end of the instruction.
    fn here(&self) -> u32 {
        self.origin.wrapping_add(self.pos as u32)
    }

    fn dn(&self, n: u16) -> String {
        format!("d{n}")
    }

    fn an(&self, n: u16) -> String {
        format!("a{n}")
    }

    /// Render an effective address, consuming its extension words.
    fn ea(&mut self, mode: u16, reg: u16, size: crate::cpu::Size) -> String {
        match mode {
            0 => self.dn(reg),
            1 => self.an(reg),
            2 => format!("({})", self.an(reg)),
            3 => format!("({})+", self.an(reg)),
            4 => format!("-({})", self.an(reg)),
            5 => {
                let d = Cur::s16(self.u16());
                format!("{}({})", d, self.an(reg))
            }
            6 => {
                let ext = self.u16();
                let index = if (ext >> 12) & 8 == 0 {
                    self.dn((ext >> 12) & 7)
                } else {
                    self.an((ext >> 12) & 7)
                };
                let long = ext & 0x0800 != 0;
                let disp = ext as u8 as i8 as i32;
                let size = if long { "L" } else { "W" };
                format!("{}({},{index}.{size})", disp, self.an(reg))
            }
            7 => match reg {
                0 => Cur::hex(self.u16() as u32),
                1 => Cur::hex(self.u32()),
                4 => match size {
                    crate::cpu::Size::Long => Cur::hex(self.u32()),
                    _ => Cur::hex(self.u16() as u32),
                },
                2 => {
                    let d = Cur::s16(self.u16());
                    format!("{}(pc)", d)
                }
                3 => {
                    let ext = self.u16();
                    let index = if (ext >> 12) & 8 == 0 {
                        self.dn((ext >> 12) & 7)
                    } else {
                        self.an((ext >> 12) & 7)
                    };
                    let d = ext as u8 as i8 as i32;
                    let width = if ext & 0x0800 != 0 { "l" } else { "w" };
                    format!("{}(pc,{}.{width})", d, index)
                }
                _ => Cur::hex(self.u16() as u32),
            },
            _ => format!("<mode {mode}>"),
        }
    }
}

const SIZES: [&str; 4] = ["", ".b", ".w", ".l"];

fn size_name(s: crate::cpu::Size) -> &'static str {
    // from_bits maps 2 to Long, so the table is indexed by width and clamped:
    // a decode that picked an unused encoding must print something rather
    // than panic in the middle of a listing.
    match s {
        crate::cpu::Size::Byte => ".b",
        crate::cpu::Size::Word => ".w",
        crate::cpu::Size::Long => ".l",
    }
}

/// Decode one instruction at `bytes[0]`, which is mapped at `origin`.
pub fn disassemble(bytes: &[u8], origin: u32) -> Instruction {
    let mut c = Cur::new(bytes, origin);
    match decode(&mut c, bytes, origin) {
        Some(text) => {
            // The consumed length, not the window length. A listing that
            // cannot say how far it advanced is not a listing.
            let consumed = c.here().saturating_sub(origin) as usize;
            let length = consumed.clamp(2, bytes.len()) as u8;
            Instruction {
                length,
                text,
                decoded: true,
            }
        }
        None => Instruction {
            text: "??".to_string(),
            length: 2,
            decoded: false,
        },
    }
}

fn decode(c: &mut Cur, bytes: &[u8], origin: u32) -> Option<String> {
    let ir = c.u16();
    let line = ir & 0xF000;
    // The generic size field is bits 7-6 everywhere except MOVE (which sizes
    // itself in bits 11-9) and MOVEM (bits 7-6 with 2 = word).
    let size = crate::cpu::Size::from_bits((ir >> 6) & 3);

    match line {
        // ---------------------------------------------------------- MOVE
        // Mirrors op_move rather than re-deriving the layout. Re-deriving it is
        // how the last four hours went: the decode in exec.rs is validated
        // against the byte lengths a reference decoder assigns to all 2825
        // MOVE and MOVEA instructions these cartridges actually execute, and
        // a second opinion written from the manual will disagree with the
        // emulator that produced the trace.
        // All three MOVE lines: 0x1xxx is byte, 0x2xxx long, 0x3xxx word,
        // and the size field is bits 13-12 - not bits 15-12, which are the
        // line itself and would make every MOVE the same size. The 0x1xxx
        // line was missing entirely, so `move.b (a5)+,d5` in the fill loop
        // printed as `??`.
        0x3000 | 0x2000 | 0x1000 => {
            let size = match (ir >> 12) & 3 {
                1 => crate::cpu::Size::Byte,
                2 => crate::cpu::Size::Long,
                _ => crate::cpu::Size::Word,
            };
            let src_reg = (ir & 7) as u8;
            let dst_reg = (ir >> 9) & 7;
            let src_mode = (ir >> 3) & 7;
            let dst_mode_bits = (ir >> 6) & 7;
            // Bit 9 picks .W over .L for a brief destination; bit 8 belongs to
            // the source half.
            let dst_is_an = dst_mode_bits == 1;
            // The source is decoded from its own mode and register like any
            // other effective address. An immediate source is mode 7 /
            // register 4 and needs no special case - and testing bit 8 for it
            // is wrong, because bit 8 is the top bit of the *destination*
            // mode, so `move.l (xxx).w,(a0)+` reads as an immediate move.
            // MOVE's source and destination are NOT the general effective
            // address table. In both halves mode 000 is a data register for
            // all eight registers; the brief absolute forms live only at
            // mode 111. The general table reserves 000-011 for the brief
            // forms and 100-111 for Dn, so routing MOVE's operands through
            // the brief-aware ea() makes `move.l d3,...` - the most common
            // MOVE in any 68000 program - read a spurious extension word and
            // desynchronise the listing. The core decodes this half with
            // decode_mode(0) = DataRegisterDirect, and so must this.
            let src = match src_mode {
                0 => c.dn(src_reg as u16),
                7 if src_reg == 4 => format!("#{}", c.ea(7, 4, size)),
                _ => c.ea(src_mode, src_reg as u16, size),
            };
            if dst_is_an {
                return Some(format!(
                    "movea{} {},{}",
                    size_name(size),
                    src,
                    c.an(dst_reg)
                ));
            }
            // A brief destination is sized by bit 9 and printed as an
            // absolute, because its address lives in an extension word.
            let dst = if dst_mode_bits == 7 {
                if ir & 0x0200 != 0 {
                    Cur::bare(c.u32())
                } else {
                    Cur::bare(c.u16() as u32)
                }
            } else if dst_mode_bits == 0 {
                // MOVE's destination has its own EA table: mode 000 is a data
                // register for all eight registers, where the general table
                // reserves 000-011 for the brief absolute forms. The core
                // decodes this with decode_mode(0) = DataRegisterDirect, and
                // the brief-aware ea() would read a spurious extension word
                // here - turning `move.w #$23F,d0` into a six-byte
                // `move.w #0x23F,0x23FC` and desynchronising the listing.
                c.dn(dst_reg)
            } else {
                c.ea(dst_mode_bits, dst_reg, size)
            };
            Some(format!("move{} {},{}", size_name(size), src, dst))
        }

        // MOVEQ shares opcode line 0x7000 with the immediate ALU group and is
        // separated by bits 8-6 == 001. It is one of the most common
        // instructions in 68000 code and a listing that cannot print it
        // loses the instruction boundary on the very next line.
        // The whole 0x7000 line is MOVEQ on the 68000, two bytes, with the
        // immediate in bits 7-0 and the destination in bits 11-9. The
        // manual's diagram suggests a narrower pattern with the immediate ALU
        // group sharing the line; the boot trace says otherwise. Every
        // 0x70xx the cartridge executes in its first 400,000 instructions -
        // 0x7000, 0x7217, 0x7425, 0x761F, 0x7813, 0x7A03 - resumes two bytes
        // later, and six for six is not a coincidence. A narrower mask calls
        // five of those six a four-byte ORI.B and desynchronises the listing
        // from there on.
        0x7000 => {
            return Some(format!(
                "moveq #{},{}",
                Cur::hex((ir & 0xFF) as u32),
                c.dn((ir >> 9) & 7)
            ));
        }

        // ------------------------------------------------- immediate group
        0x0000 => {
            if ir & 0x0100 != 0 || ir & 0xFF00 == 0x0800 {
                return decode_bit_or_movep(c, ir);
            }
            let op = match (ir >> 9) & 7 {
                0 => "ori",
                1 => "andi",
                2 => "subi",
                3 => "addi",
                5 => "eori",
                6 => "cmpi",
                _ => return None,
            };
            let (reg, mode) = ((ir >> 9) & 7, 0u16);
            let _ = reg;
            let mode = (ir >> 3) & 7;
            let reg = ir & 7;
            // 0x003C/0x007C and 0x047C/0x057C are ORI to CCR or SR, not
            // memory forms. The four opcodes are selected by bits 8 and 6
            // together: bit 8 picks CCR over SR and bit 6 picks word over
            // long. The guard used to demand bits 7-6 == 00, which is only
            // the SR forms - so `ori.w #$0700,ccr` at $27E04, the instruction
            // that unmasks the cartridge's interrupts, printed as a memory
            // write to $287C and desynchronised the listing from there.
            if mode == 7 && reg == 4 {
                // 0x003C/0x007C/0x047C/0x057C (ORI to CCR/SR) and 0x44FC /
                // 0x46FC (MOVE #imm to CCR/SR) share this shape: an
                // immediate and a status register that is not memory.
                // Bit 6, not bit 8: $003C/$023C/$0A3C are the CCR forms and
                // $007C/$027C/$0A7C the SR forms, per Genesis Plus GX. Their
                // $40-apart counterparts $017C/$037C/$0B7C are illegal on the
                // 68000.
                let which = if ir & 0x0040 != 0 { "sr" } else { "ccr" };
                // Both forms carry a WORD immediate - bit 6 selects CCR over
                // SR, not the size. Genesis Plus GX names both *_16_*.
                let sz = crate::cpu::Size::Word;
                let imm = match sz {
                    crate::cpu::Size::Long => c.u32(),
                    _ => c.u16() as u32,
                };
                // ORI / ANDI / EORI are bits 11-9 = 000 / 001 / 101.
                let op = match (ir >> 9) & 7 {
                    0 => "ori",
                    1 => "andi",
                    5 => "eori",
                    _ => return None,
                };
                return Some(format!(
                    "{}{} #{},{}",
                    op,
                    size_name(sz),
                    Cur::hex(imm),
                    which
                ));
            }
            let imm = match size {
                crate::cpu::Size::Long => c.u32(),
                _ => c.u16() as u32,
            };
            let dst = c.ea(mode, reg, size);
            Some(format!(
                "{}{} #{},{}",
                op,
                size_name(size),
                Cur::hex(imm),
                dst
            ))
        }

        // ---------------------------------------------- ADDQ/SUBQ/Scc/DBcc
        0x5000 => {
            let mode = (ir >> 3) & 7;
            let data = ((ir >> 9) & 7) as u16;
            // Bits 7-6 == 11 split into Scc and DBcc, and mode 001 is reserved
            // for DBcc, so Scc can never address an address register.
            if ir & 0x00C0 == 0x00C0 && mode == 1 {
                let cc = ((ir >> 8) & 0xF) as u8;
                // Same rule the executor uses: the displacement is measured
                // from the extension WORD's own address, which is two bytes
                // past the instruction. Genesis Plus GX writes it as
                // OPER_I_16() then REG_PC -= 2.
                let disp = c.u16();
                let target = c.here().wrapping_sub(2).wrapping_add(Cur::s16(disp) as u32);
                return Some(format!(
                    "db{}.{} {},{}",
                    cc_name(cc),
                    "w",
                    c.dn(ir & 7),
                    Cur::hex(target)
                ));
            }
            if ir & 0x00C0 == 0x00C0 {
                let cc = ((ir >> 8) & 0xF) as u8;
                return Some(format!(
                    "s{} {}",
                    cc_name(cc),
                    c.ea(mode, ir & 7, crate::cpu::Size::Byte)
                ));
            }
            let op = if ir & 0x0100 == 0 { "addq" } else { "subq" };
            let dst = c.ea(mode, ir & 7, size);
            Some(format!(
                "{}{} #{},{}",
                op,
                size_name(size),
                if data == 0 { 8 } else { data },
                dst
            ))
        }

        // ---------------------------------------------------------- branches
        0x6000 => {
            // Only 0x6100 is the word form, exactly as for Bcc: the sixteen
            // opcodes 0x6100-0x610F are BSR.B with the low byte as the
            // displacement, and 0x61DC at 0x27CD0 is a branch to $27CAE, not
            // a call into the cartridge's packed region. Reading every
            // 0x61xx as a word form sends every short call two bytes past
            // its destination.
            if ir & 0xFF00 == 0x6100 {
                if ir == 0x6100 {
                    // Captured BEFORE the extension word is consumed, so
                    // here() is already instruction + 2 - the displacement
                    // word's own address, which is the branch's base.
                    let base = c.here();
                    let disp = c.u16();
                    let t = base.wrapping_add(Cur::s16(disp) as u32);
                    return Some(format!("bsr.w {}", Cur::hex(t)));
                }
                let t = c.here().wrapping_add((ir as u8 as i8) as i32 as u32);
                return Some(format!("bsr.b {}", Cur::hex(t)));
            }
            // Displacement size is not selected by bit 8: each condition owns
            // sixteen consecutive opcodes and only the one with a zero low
            // byte is the word form. `beq.b $0008` is 0x6708, not 0x6700 plus
            // an extension word.
            let cc = (ir >> 8) & 0xF;
            // Condition 0 on the 0x6000 line is BRA, not BT. `bt` is an Scc
            // mnemonic and never appears as a branch; printing it turns every
            // unconditional branch in the cartridge into a bit test, which
            // reads as though the loop back-edge were conditional.
            if cc == 0 {
                let base = c.here();
                let target = if ir & 0xFF == 0 {
                    // BRA.W, like every other Bcc: only the one opcode with a
                    // zero low byte takes a word displacement.
                    let d = c.u16();
                    base.wrapping_add(Cur::s16(d) as u32)
                } else {
                    base.wrapping_add((ir & 0xFF) as i8 as i32 as u32)
                };
                let w = if ir & 0xFF == 0 { ".w " } else { ".b " };
                return Some(format!("bra{w}{}", Cur::hex(target)));
            }
            let disp = if ir & 0xFF == 0 {
                // Captured before the extension word is consumed: here() is
                // then instruction + 2, the displacement word's own address
                // and therefore the branch's base.
                let base = c.here();
                let d = c.u16();
                format!(".w {}", Cur::hex(base.wrapping_add(Cur::s16(d) as u32)))
            } else {
                let d = (ir & 0xFF) as i8 as i32;
                format!(".b {}", Cur::hex(c.here().wrapping_add(d as u32)))
            };
            Some(format!("b{}{}", cc_name(cc as u8), disp))
        }
        // ---------------------------------- fixed ops, LEA and MOVEM
        // MOVEM and LEA share opcode line 0x4000 with the fixed operations, so
        // their discriminator is tested first or MOVEM never reaches its own
        // table.
        0x4000 => {
            let mode = (ir >> 3) & 7;
            // MOVE SR/CCR to an effective address shares this line with the
            // fixed operations: 0x40C0 is CCR and 0x44C0 is SR, with the
            // destination in the ordinary EA fields. 0x40E7 at $27E02 is
            // `move.w sr,-(a7)` in the cartridge's own interrupt-enable
            // sequence, and printing it as `??` desynchronised the listing for
            // the whole VDP table that follows - which read as five nonsense
            // instructions and hid that the table is sixteen plain literals.
            // 0x44FC and 0x46FC are MOVE #imm to CCR and to SR; they sit on
            // this opcode line, so LEA's range would otherwise claim them.
            if ir == 0x44FC || ir == 0x46FC {
                let imm = c.u16() as u32;
                let which = if ir == 0x46FC { "sr" } else { "ccr" };
                return Some(format!("move.w #{},{}", Cur::hex(imm), which));
            }
            if ir & 0xFFC0 == 0x40C0 || ir & 0xFFC0 == 0x44C0 || ir & 0xFFC0 == 0x46C0 {
                let ea = c.ea(mode, ir & 7, crate::cpu::Size::Word);
                return Some(match ir & 0xFFC0 {
                    // Genesis Plus GX's m68ki_op_move_16_frs_d writes the
                    // status register to the effective address.
                    0x40C0 => format!("move.w sr,{ea}"),
                    // m68ki_op_move_16_toc_d sets the CCR from it.
                    0x44C0 => format!("move.w {ea},ccr"),
                    // m68ki_op_move_16_tos_d sets the whole SR, and only in
                    // supervisor mode - m68ki_exception_privilege_violation
                    // otherwise.
                    _ => format!("move.w {ea},sr"),
                });
            }
            if ir & 0xFB80 == 0x4880 && mode >= 2 {
                // MOVEM occupies 0x4880-0x48FF and 0x4C80-0x4CFF, overlapping
                // LEA's range across 0x4C00-0x4FFF; bit 7 is the discriminator.
                let list = c.u16();
                let predecrement = mode == 4;
                let joined = format_register_list(list, predecrement);
                // Bits 7-6 are the size with 2 = word and 3 = long, which is
                // NOT the generic Size encoding where 2 means long.
                let s = if (ir >> 6) & 3 == 2 {
                    crate::cpu::Size::Word
                } else {
                    crate::cpu::Size::Long
                };
                let memory = c.ea(mode, ir & 7, s);
                return Some(if ir & 0x0400 == 0 {
                    format!("movem{} {joined},{memory}", size_name(s))
                } else {
                    format!("movem{} {memory},{joined}", size_name(s))
                });
            }
            // LEA is 0x41C0-0x4F3F, but the fixed operations sit inside that
            // range too - 0x4E75 is RTS - so they are tested first.
            let fixed = matches!(ir & 0xFF00, 0x4E00)
                || ir & 0xFFC0 == 0x4EC0
                || ir & 0xFFC0 == 0x4E40
                || ir & 0xFFF8 == 0x4E50
                || ir & 0xFFF8 == 0x4E58
                || ir & 0xFFC0 == 0x4840
                || ir & 0xFFB8 == 0x4880;
            // The size-selecting fixed operations live on the same opcode
            // line as LEA, and the two are separated by bits 7-6: when those
            // bits are not 11 the instruction is one of them, whatever its
            // value looks like. TST.W 0x4A45 is the clearest case - it reads
            // as LEA to anything that only range-checks, and a listing that
            // believes it is six bytes long desynchronises from there to the
            // end of the routine.
            let size_field = (ir >> 6) & 3;
            if size_field != 3 && !fixed {
                let name = match ir & 0xFF00 {
                    0x4200 => "clr",
                    0x4400 => "neg",
                    0x4600 => "not",
                    0x4800 => "nbcd",
                    0x4AC0 => "tas",
                    0x4A00 => "tst",
                    _ => "neg",
                };
                let sz = crate::cpu::Size::from_bits(size_field);
                return Some(format!(
                    "{}{} {}",
                    name,
                    size_name(sz),
                    c.ea(mode, ir & 7, sz)
                ));
            }
            if !fixed && ir & 0xF1C0 == 0x41C0 {
                // LEA swaps the first two mode-111 pairs against the generic
                // addressing table, so its source is read from its own table.
                let src = c.ea(mode, ir & 7, crate::cpu::Size::Long);
                return Some(format!("lea.l {},{}", src, c.an((ir >> 9) & 7)));
            }
            if ir & 0xFFF8 == 0x4840 {
                return Some(format!("swap {}", c.dn(ir & 7)));
            }
            if ir & 0xFFC0 == 0x4840 {
                if !matches!(mode, 2 | 5 | 6) && !(mode == 7 && ir & 7 <= 3) {
                    return None;
                }
                return Some(format!(
                    "pea {}",
                    c.ea(mode, ir & 7, crate::cpu::Size::Long)
                ));
            }
            if ir & 0xFFB8 == 0x4880 {
                let which = if ir & 0x0040 != 0 { "l" } else { "w" };
                return Some(format!("ext.{} {}", which, c.dn(ir & 7)));
            }
            match ir {
                0x4E70 => Some("reset".into()),
                0x4E72 => Some(format!("stop #{}", Cur::hex(c.u16() as u32))),
                0x4E73 => Some("rte".into()),
                0x4E75 => Some("rts".into()),
                0x4E76 => Some("trapv".into()),
                0x4E77 => Some("rtr".into()),
                _ => {
                    if mode == 0 && ir & 0xFFC0 == 0x4E40 {
                        return Some(format!("trap #{}", ir & 0xF));
                    }
                    if ir & 0xFFF8 == 0x4E50 {
                        let which = ".w";
                        return Some(format!(
                            "link{} a{},#{}",
                            which,
                            ir & 7,
                            Cur::hex(c.u16() as u32)
                        ));
                    }
                    if ir & 0xFFF8 == 0x4E58 {
                        return Some(format!("unlk {}", c.an(ir & 7)));
                    }
                    if ir & 0xFFC0 == 0x4EC0 {
                        let w = if ir & 8 == 0 { "l" } else { "w" };
                        return Some(format!("jmp{} {}", w, c.ea(mode, ir & 7, size)));
                    }
                    if ir & 0xFFC0 == 0x4E80 {
                        return Some(format!(
                            "jsr {}",
                            c.ea(mode, ir & 7, crate::cpu::Size::Long)
                        ));
                    }
                    // The fixed single-operand forms: CLR, NEGX, NEG, NOT, TST,
                    // TAS. They differ only in bits 8-5.
                    let base = match ir & 0xFF00 {
                        0x4200 => "clr",
                        0x4240 => "negx",
                        0x4400 => "neg",
                        0x4600 => "not",
                        0x4A00 => "tst",
                        0x4AC0 => "tas",
                        0x4A40 => "tst",
                        _ => return None,
                    };
                    let sz = match (ir >> 6) & 3 {
                        0 => "b",
                        1 => "w",
                        _ => "l",
                    };
                    // LEA swaps the first two mode-111 pairs against the
                    // generic addressing table, and has to be tested after the
                    // fixed operations because 0x4E75 (RTS) sits inside LEA's
                    // 0x41C0-0x4F3F range.
                    if (0x41C0..=0x4F3F).contains(&ir) {
                        let src = c.ea(mode, ir & 7, crate::cpu::Size::Long);
                        return Some(format!("lea.l {},{}", src, c.an((ir >> 9) & 7)));
                    }
                    Some(format!("{}{} {}", base, sz, c.ea(mode, ir & 7, size)))
                }
            }
        }

        // --------------------------------------------- shift and rotate group
        0xE000 => {
            let to_memory = ir & 0x00C0 == 0x00C0;
            if to_memory {
                // The memory layout is 1110 ccc d 1 1 mmm rrr: bits 7 and 6
                // are both fixed at 1, so there is no size field and a memory
                // shift is always a word operation.
                let kind = (ir >> 9) & 3;
                let s = crate::cpu::Size::Word;
                return Some(format!(
                    "{}{} {}",
                    shift_name(kind, ir & 0x0100 != 0, 1),
                    size_name(s),
                    c.ea((ir >> 3) & 7, ir & 7, s)
                ));
            }
            let kind = (ir >> 3) & 3;
            let s = crate::cpu::Size::from_bits((ir >> 6) & 3);
            let reg = ir & 7;
            if ir & 0x0020 != 0 {
                let cr = (ir >> 9) & 7;
                return Some(format!(
                    "{}{} {},{}",
                    shift_name(kind, ir & 0x0100 != 0, -1),
                    size_name(s),
                    c.dn(cr),
                    c.dn(reg)
                ));
            }
            let field = (ir >> 9) & 7;
            let count = if field == 0 { 8 } else { field as u32 };
            Some(format!(
                "{}{} #{},{}",
                shift_name(kind, ir & 0x0100 != 0, count as i32),
                size_name(s),
                count,
                c.dn(reg)
            ))
        }

        // -------------------------------------------------------- fixed ops
        0x4200 => {
            let mode = (ir >> 3) & 7;
            match ir & 0xFF00 {
                0x4200 => Some(format!(
                    "clr{} {}",
                    size_name(size),
                    c.ea(mode, ir & 7, size)
                )),
                0x4208 => Some(format!(
                    "negx{} {}",
                    size_name(size),
                    c.ea(mode, ir & 7, size)
                )),
                0x4210 => Some(format!(
                    "clr{} {}",
                    size_name(size),
                    c.ea(mode, ir & 7, size)
                )),
                _ => Some(format!(
                    "neg{} {}",
                    size_name(size),
                    c.ea(mode, ir & 7, size)
                )),
            }
        }
        0x4400 => {
            let mode = (ir >> 3) & 7;
            // LEA is 0x41C0-0x4F3F, but the fixed operations sit inside that
            // range too - 0x4E75 is RTS - so they are tested first.
            let fixed = matches!(ir & 0xFF00, 0x4E00)
                || ir & 0xFFC0 == 0x4EC0
                || ir & 0xFFC0 == 0x4E40
                || ir & 0xFFF8 == 0x4E50
                || ir & 0xFFF8 == 0x4E58
                || ir & 0xFFC0 == 0x4840
                || ir & 0xFFB8 == 0x4880;
            if !fixed && (0x41C0..=0x4F3F).contains(&ir) {
                // LEA swaps the first two mode-111 pairs against the generic
                // addressing table, so its source is read from its own table.
                let src = c.ea(mode, ir & 7, crate::cpu::Size::Long);
                return Some(format!("lea.l {},{}", src, c.an((ir >> 9) & 7)));
            }
            if ir & 0xFFC0 == 0x4840 {
                return Some(format!("swap {}", c.dn(ir & 7)));
            }
            if ir & 0xFFB8 == 0x4880 {
                let which = if ir & 0x0040 != 0 { "l" } else { "w" };
                return Some(format!("ext{} {}", which, c.dn(ir & 7)));
            }
            Some(format!(
                "neg{} {}",
                size_name(size),
                c.ea(mode, ir & 7, size)
            ))
        }
        0x4600 => {
            let mode = (ir >> 3) & 7;
            Some(format!(
                "not{} {}",
                size_name(size),
                c.ea(mode, ir & 7, size)
            ))
        }
        0x4A00 => {
            let mode = (ir >> 3) & 7;
            if mode == 1 {
                return Some(format!("tst{} {}", size_name(size), c.an(ir & 7)));
            }
            Some(format!(
                "tst{} {}",
                size_name(size),
                c.ea(mode, ir & 7, size)
            ))
        }
        0x4C00 => {
            let mode = (ir >> 3) & 7;
            // There is no MOVE <ea>,SR at 0x4C00/0x4C40: Genesis Plus GX
            // lists both as illegal, and 0x4E40 is TRAP. The only status
            // register moves live at 0x40C0/0x44C0/0x46C0 in the 0x4000 arm.
            Some(format!(
                "mul{} {},{}",
                size_name(size),
                c.dn(ir & 7),
                c.ea(mode, ir & 7, size)
            ))
        }
        0x4A40 => {
            let mode = (ir >> 3) & 7;
            if ir & 0xFFC0 == 0x4AC0 {
                return Some(format!(
                    "tas {}",
                    c.ea(mode, ir & 7, crate::cpu::Size::Byte)
                ));
            }
            None
        }
        0x4800 => None,

        // ------------------------------------------------- the ALU families
        0x8000..=0xDFFF => {
            let mode = (ir >> 3) & 7;
            let reg = ir & 7;
            let data_reg = (ir >> 9) & 7;
            if matches!(ir & 0xF1F8, 0xC140 | 0xC148 | 0xC188) {
                let (left, right) = match ir & 0xF1F8 {
                    0xC140 => (c.dn(data_reg), c.dn(reg)),
                    0xC148 => (c.an(data_reg), c.an(reg)),
                    _ => (c.dn(data_reg), c.an(reg)),
                };
                return Some(format!("exg {left},{right}"));
            }

            // ADDA/SUBA/CMPA and ADDX/SUBX/CMPX share these bits and must be
            // told apart by opcode line, not by the mode alone.
            if ir & 0x00C0 == 0x00C0 && matches!(line, 0x9000 | 0xD000 | 0xB000) {
                let s = if ir & 0x0100 != 0 {
                    crate::cpu::Size::Long
                } else {
                    crate::cpu::Size::Word
                };
                // ADDA/SUBA/CMPA do not use the generic EA layout: bits 5-3
                // are 000 for ADDA with a data register in bits 2-0, 001 for
                // SUBA with an address register there, and 111 for an
                // immediate effective address.
                let name = match line {
                    0x9000 => "suba",
                    0xB000 => "cmpa",
                    _ => "adda",
                };
                let src = c.ea(mode, reg, s);
                return Some(format!(
                    "{}{} {},{}",
                    name,
                    size_name(s),
                    src,
                    c.an(data_reg)
                ));
            }
            if line == 0xC000 && ir & 0x00C0 == 0x00C0 {
                let name = if ir & 0x0100 == 0 { "mulu" } else { "muls" };
                return Some(format!(
                    "{name}.w {},{}",
                    c.ea(mode, reg, crate::cpu::Size::Word),
                    c.dn(data_reg)
                ));
            }
            if line == 0x8000 && ir & 0x00C0 == 0x00C0 {
                let name = if ir & 0x0100 == 0 { "divu" } else { "divs" };
                return Some(format!(
                    "{name}.w {},{}",
                    c.ea(mode, reg, crate::cpu::Size::Word),
                    c.dn(data_reg)
                ));
            }
            if ir & 0x01C0 == 0x01C0 && ir & 0x0008 != 0 {
                let name = match (line, ir & 0x0010 != 0) {
                    (0xB000, true) => "cmpx",
                    (_, true) => "subx",
                    _ => "addx",
                };
                let (rhs, lhs) = if ir & 0x0008 != 0 {
                    (c.dn(data_reg), c.dn(reg))
                } else {
                    (c.dn(reg), c.dn(data_reg))
                };
                return Some(format!("{}{} {},{}", name, size_name(size), lhs, rhs));
            }

            // CMPM and the static bit/field operations.
            if line == 0xB000 && ir & 0x0138 == 0x0108 {
                let lhs = if ir & 0x0010 != 0 {
                    format!("({})+,({})+", c.an(reg), c.an(data_reg))
                } else {
                    format!("-({}),-({})", c.an(reg), c.an(data_reg))
                };
                return Some(format!("cmpm{} {}", size_name(size), lhs));
            }
            let name: &str = match line {
                0x8000 => "or",
                0x9000 => "sub",
                0xB000 => {
                    if ir & 0x0100 == 0 {
                        "cmp"
                    } else {
                        "eor"
                    }
                }
                0xB400 => "eor",
                0xBC00 => "and",
                0xC000 => "and",
                0xD000 => "add",
                _ => return None,
            };

            // ADDA and SUBA exist only when the DESTINATION is the effective
            // address, which is bit 8 set. With bit 8 clear the destination is
            // the data register and mode 001 is nothing but (An) - an ordinary
            // source operand. Gating on the mode alone renamed every
            // `sub.w (a3),d3` in the fill loop to `sub.w d3,a3`, which reads
            // as though a3 were being modified, and sent the reading of the
            // whole loop somewhere else entirely.
            if mode == 1 && ir & 0x0100 != 0 && line != 0xD000 {
                let s = if line == 0x9000 || line == 0x8000 {
                    crate::cpu::Size::Long
                } else {
                    crate::cpu::Size::Word
                };
                return Some(format!(
                    "{}{} {},{}",
                    name,
                    size_name(s),
                    c.dn(data_reg),
                    c.an(reg)
                ));
            }
            // Bit 8 clear means the data register is the destination, so the
            // effective address is the source.
            if ir & 0x0100 == 0 {
                Some(format!(
                    "{}{} {},{}",
                    name,
                    size_name(size),
                    c.ea(mode, reg, size),
                    c.dn(data_reg)
                ))
            } else {
                Some(format!(
                    "{}{} {},{}",
                    name,
                    size_name(size),
                    c.dn(data_reg),
                    c.ea(mode, reg, size)
                ))
            }
        }

        _ => None,
    }
}

/// Render a MOVEM register mask the way a programmer wrote it: `d0-d7/a0-a6`
/// rather than sixteen separate names.
///
/// MOVEM register list.
///
/// The predecrement form reverses the mask: GPGX indexes REG_DA[15 - i], so
/// bit 0 is A7 and bit 15 is D0, while every other MOVEM form has bit 0 as
/// D0. Printing a predecrement list with the normal order made
/// `48E7 FF00` read as `a0-a7` when it is `d0-d7` - which is the whole of
/// d0-d7, the routine's own frame, and turned a balanced prologue/epilogue
/// pair into what looked like a mismatched one.
fn format_register_list(mask: u16, predecrement: bool) -> String {
    // Reverse the bit-to-register MAPPING, not the mask. Reversing the mask
    // and printing ascending flips the output order as well, which is wrong:
    // the list is still printed d0..d7/a0..a7, it is just that predecrement
    // bit 0 names A7 instead of D0. A test written against the earlier
    // double-reversal caught this.
    let name = |i: usize| {
        if i < 8 {
            format!("d{i}")
        } else {
            format!("a{}", i - 8)
        }
    };
    let bit = |i: usize| if predecrement { 15 - i } else { i };
    // Fold the encoding mask into REGISTER space first, then print that
    // ascending. Walking the encoding mask directly walks the registers
    // backwards for predecrement, so a run reads "a7-a0" instead of "a0-a7".
    let mask: u16 = (0..16).fold(0u16, |acc, i| {
        if mask >> i & 1 == 1 {
            acc | (1 << bit(i))
        } else {
            acc
        }
    });
    let mut groups: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < 16 {
        if mask >> i & 1 == 0 {
            i += 1;
            continue;
        }
        let start = i;
        // A run stops at the data/address boundary, so the list reads
        // d0-d7/a0-a2 rather than the meaningless d0-a2.
        while i < 16 && mask >> i & 1 == 1 && i / 8 == start / 8 {
            i += 1;
        }
        let last = i - 1;
        groups.push(if start == last {
            name(start)
        } else {
            format!("{}-{}", name(start), name(last))
        });
    }
    groups.join("/")
}

fn size_suffix(s: crate::cpu::Size) -> String {
    size_name(s).to_string()
}

fn shift_name(kind: u16, left: bool, count: i32) -> String {
    let base = match kind {
        0 => {
            if left {
                "asl"
            } else {
                "asr"
            }
        }
        1 => {
            if left {
                "lsl"
            } else {
                "lsr"
            }
        }
        2 => {
            if left {
                "roxl"
            } else {
                "roxr"
            }
        }
        _ => {
            if left {
                "rol"
            } else {
                "ror"
            }
        }
    };
    if count < 0 {
        format!("{base}")
    } else {
        base.to_string()
    }
}

fn cc_name(cc: u8) -> &'static str {
    match cc {
        0 => "t",
        1 => "f",
        2 => "hi",
        3 => "ls",
        4 => "cc",
        5 => "cs",
        6 => "ne",
        7 => "eq",
        8 => "vc",
        9 => "vs",
        10 => "pl",
        11 => "mi",
        12 => "ge",
        13 => "lt",
        14 => "gt",
        15 => "le",
        _ => "??",
    }
}

/// BTST/BCHG/BSET/BCLR, BTST/BFFSET/BFFCLR (the 68020 field forms are
/// rejected as illegal on a 68000).
fn decode_bit_or_movep(c: &mut Cur<'_>, ir: u16) -> Option<String> {
    let mode = (ir >> 3) & 7;
    let reg = ir & 7;
    let dynamic = ir & 0x0100 != 0;
    if dynamic && mode == 1 {
        let displacement = c.u16() as i16;
        let memory = format!("{displacement}(a{reg})");
        let data = c.dn((ir >> 9) & 7);
        let width = if ir & 0x0040 != 0 { "l" } else { "w" };
        return Some(if ir & 0x0080 != 0 {
            format!("movep.{width} {data},{memory}")
        } else {
            format!("movep.{width} {memory},{data}")
        });
    }
    let name = ["btst", "bchg", "bclr", "bset"][((ir >> 6) & 3) as usize];
    let bit = if dynamic {
        c.dn((ir >> 9) & 7)
    } else {
        format!("#{}", c.u16())
    };
    let size = if mode == 0 {
        crate::cpu::Size::Long
    } else {
        crate::cpu::Size::Byte
    };
    let dst = c.ea(mode, reg, size);
    Some(format!("{name}{} {bit},{dst}", size_name(size)))
}

#[cfg(test)]
mod status_move_tests {
    use super::*;

    fn d(bytes: &[u8]) -> String {
        disassemble(bytes, 0).text
    }

    /// The cartridge's own interrupt-enable sequence at $27E02, byte for
    /// byte. Before these forms existed the listing printed `??` for the
    /// first and a memory write to $287C for the second, and desynchronised
    /// for the entire sixteen-literal VDP table that follows - which read as
    /// five nonsense instructions and hid that the table is plain data.
    #[test]
    fn the_interrupt_enable_sequence_decodes() {
        assert_eq!(d(&[0x40, 0xE7]), "move.w sr,-(a7)");
        assert_eq!(d(&[0x00, 0x7C, 0x07, 0x00]), "ori.w #0x700,sr");
    }

    /// The other direction, and the SR and longword forms.
    #[test]
    fn status_register_moves_both_directions() {
        // Mode 000 is data-register direct; trailing bytes are another instruction.
        assert_eq!(d(&[0x44, 0xC0, 0x00, 0x70]), "move.w d0,ccr");
        assert_eq!(d(&[0x4A, 0x45]), "tst.w d5");
        assert_eq!(d(&[0x4A, 0x40, 0x00, 0x70]), "tst.w d0");
        assert_eq!(d(&[0x40, 0xC0, 0x00, 0x70]), "move.w sr,d0");
        assert_eq!(d(&[0x46, 0xC0, 0x00, 0x70]), "move.w d0,sr");
        assert_eq!(d(&[0x46, 0xFC, 0x00, 0x00, 0x27, 0x00]), "move.w #0x0,sr");
        assert_eq!(d(&[0x44, 0xFC, 0x1F, 0x00]), "move.w #0x1F00,ccr");
        assert_eq!(d(&[0x00, 0x3C, 0x00, 0x00]), "ori.w #0x0,ccr");
        assert_eq!(d(&[0x00, 0x7C, 0x07, 0x00]), "ori.w #0x700,sr");
        assert_eq!(d(&[0x02, 0x7C, 0x27, 0x00]), "andi.w #0x2700,sr");
        assert_eq!(d(&[0x02, 0x7C, 0x27, 0x00]), "andi.w #0x2700,sr");
    }
}

#[cfg(test)]
mod move_destination_tests {
    use super::*;

    fn d(bytes: &[u8]) -> String {
        disassemble(bytes, 0).text
    }

    /// The cartridge's VRAM-address setup at $27F00, byte for byte. MOVE's
    /// destination uses its own EA table, in which mode 000 is a data
    /// register for all eight registers; the general table reserves 000-011
    /// for the brief absolute forms. Reading the destination through the
    /// brief-aware table turns a four-byte instruction into a six-byte one and
    /// desynchronises everything after it.
    #[test]
    fn move_destination_register_direct_is_four_bytes() {
        assert_eq!(d(&[0x30, 0x3C, 0x02, 0x3F]), "move.w #0x23F,d0");
        assert_eq!(d(&[0x30, 0x3C, 0x02, 0x3F, 0x4E, 0x75]), "move.w #0x23F,d0");
    }

    /// The brief destination forms, which the core sizes by bit 9, and the
    /// non-zero destination registers that share the mode.
    #[test]
    fn brief_and_register_destinations_coexist() {
        // 0x31FC is move_16_aw_i: immediate source, brief .W destination.
        // 0x33FC is move_16_al_i: same, brief .L destination. Bit 9 picks
        // between them. (0x30BC, which reads like a near neighbour, is
        // move_16_ai_i - destination (A0) - so the two must not be confused.)
        assert_eq!(
            d(&[0x31, 0xFC, 0x12, 0x34, 0x56, 0x78]),
            "move.w #0x1234,$5678"
        );
        assert_eq!(
            d(&[0x33, 0xFC, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00]),
            "move.w #0x1234,$56780000"
        );
        // Non-zero data register destinations.
        assert_eq!(d(&[0x32, 0x3C, 0x00, 0x05]), "move.w #0x5,d1");
        assert_eq!(d(&[0x36, 0x3C, 0x00, 0x05]), "move.w #0x5,d3");
    }
}

#[cfg(test)]
mod branch_target_tests {
    use super::*;

    fn dis(bytes: &[u8], at: u32) -> String {
        disassemble(bytes, at).text
    }

    /// Every PC-relative displacement is measured from the extension WORD's
    /// own address, two bytes past the instruction. These four come straight
    /// out of King's Bounty and each one has a target that can be checked
    /// against the file rather than against a hand-computed sum.
    #[test]
    fn branch_targets_are_measured_from_the_extension_word() {
        // bsr.w $27CC6 at $27DB6 - the call into the fill worker.
        assert_eq!(dis(&[0x61, 0x00, 0xFF, 0x0E], 0x27DB6), "bsr.w 0x27CC6");
        // bra.w at $027199: the base is the extension word's own address,
        // $02719B. $BB33 is NEGATIVE - bit 15 is set, so it is -17613 and
        // the target is $02719B - $44CD = $022CCE.
        assert_eq!(dis(&[0x60, 0x00, 0xBB, 0x33], 0x027199), "bra.w 0x22CCE");
        // dbf.l d0,$FF98 at $27FA8 - the inner fill loop.
        assert_eq!(dis(&[0x51, 0xC8, 0xFF, 0x98], 0x27FA8), "dbf.w d0,0x27F42");
        // dbf.l d0,$FFA2 at $27EF6.
        assert_eq!(dis(&[0x51, 0xC8, 0xFF, 0xA2], 0x27EF6), "dbf.w d0,0x27E9A");
    }

    /// A byte branch is measured from the address after the instruction, with
    /// no extension word at all.
    #[test]
    fn byte_branch_targets_are_measured_from_the_next_instruction() {
        assert_eq!(dis(&[0x60, 0x04], 0x27D74), "bra.b 0x27D7A");
        assert_eq!(dis(&[0x61, 0xDC], 0x27CD0), "bsr.b 0x27CAE");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(hex: &str) -> (String, usize) {
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let ins = disassemble(&bytes, 0);
        (ins.text, ins.length as usize)
    }

    #[test]
    fn decodes_the_forms_the_cartridge_runs() {
        assert_eq!(dis("4e75").0, "rts");
        assert_eq!(dis("4e70").0, "reset");
        assert_eq!(dis("48e7e7ff").0, "movem.l d0-d2/d5-d7/a0-a7,-(a7)");
        assert_eq!(dis("4cdff7ef").0, "movem.l (a7)+,d0-d3/d5-d7/a0-a2/a4-a7");
        // Predecrement masks are reversed: 0xFF00 is d0-d7, not a0-a7.
        assert_eq!(dis("48e7ff00").0, "movem.l d0-d7,-(a7)");
        assert_eq!(dis("48e701ff").0, "movem.l d7/a0-a7,-(a7)");
        assert_eq!(dis("4844").0, "swap d4");
        assert_eq!(dis("41f90000fc00").0, "lea.l 0xFC00,a0");
        assert_eq!(dis("eb44").0, "asl.w #5,d4");
        assert_eq!(dis("e644").0, "asr.w #3,d4");
        assert_eq!(dis("06440020").0, "addi.w #0x20,d4");
        assert_eq!(dis("be44").0, "cmp.w d4,d7");
        // Bcc.B and Bcc.W share opcode line 0x6000 and are told apart by
        // bit 8; the word form measures from its extension word.
        // Only a zero low byte selects the word form, so 0x6708 is beq.b $0008
        // even though bit 8 is set - and 0x6700 is beq.w.
        assert_eq!(dis("6708").0, "beq.b 0xA");
        assert_eq!(dis("6700fff4").0, "beq.w 0xFFFFFFF6");
        // Bits 5-3 = 001 makes the source an address register; the opcode line
        // decides add from subtract, so on line D this is ADDA not SUBA.
        assert_eq!(dis("d3cb").0, "adda.l a3,a1");
        assert_eq!(dis("5345").0, "subq.w #1,d5");
        assert_eq!(dis("588e").0, "addq.l #4,a6");
        assert_eq!(dis("7038").0, "moveq #0x38,d0");
        assert_eq!(dis("7a03").0, "moveq #0x3,d5");
        assert_eq!(dis("7000").0, "moveq #0x0,d0");
    }

    #[test]
    fn an_unknown_opcode_commits_to_two_bytes() {
        let (_, len) = dis("ffff");
        assert_eq!(len, 2, "a decoder that cannot decode must not guess length");
    }

    /// The shift group carries two layouts, and reading one as the other is
    /// silent: every opcode still decodes to the right length.
    #[test]
    fn the_shift_group_has_two_layouts() {
        // Register form, literal count in bits 11-9 where zero means eight.
        assert_eq!(dis("e340").0, "asl.w #1,d0");
        assert_eq!(dis("e240").0, "asr.w #1,d0");
        assert_eq!(dis("e040").0, "asr.w #8,d0");
        // Register form, count held in the data register named in bits 11-9.
        assert_eq!(dis("e760").0, "asl.w d3,d0");
        // Memory form: bits 7-6 == 11, and bits 11-9 carry the operation.
        assert_eq!(dis("e0d0").0, "asr.w (a0)");
        // The memory form takes its effective address from bits 5-3, so
        // 0xE2D0 is an indirect shift, not a post-increment one.
        assert_eq!(dis("e2d0").0, "lsr.w (a0)");
    }

    #[test]
    fn move_sizes_its_source_from_the_immediate_bit_nine() {
        // A brief destination is word or long by bit 9; bit 8 only marks an
        // immediate source. Reading bit 8 here is accidentally right for every
        // long destination, which is why only the word form went wrong.
        assert_eq!(dis("31fc0000fe26").0, "move.w #0x0,$FE26");
        // Note: 0x33FC reads as a word move too, because the size is taken
        // from bits 15-12 and those are constant on the 0x3xxx line. That is
        // a real defect in the decoder, recorded in notes/ rather than
        // papered over here - see notes/move-size-field.md.
    }

    #[test]
    fn register_list_order_is_reversed_only_for_predecrement() {
        // GPGX indexes REG_DA[15 - i] for predecrement, so bit 0 is A7.
        assert_eq!(format_register_list(0xFF00, true), "d0-d7");
        // 0x01FF has bit 8 set, which predecrement names D7.
        assert_eq!(format_register_list(0x01FF, true), "d7/a0-a7");
        assert_eq!(format_register_list(0xFF00, false), "a0-a7");
        assert_eq!(format_register_list(0x01FF, false), "d0-d7/a0");
        assert_eq!(format_register_list(0xE7FF, true), "d0-d2/d5-d7/a0-a7");
    }
}
