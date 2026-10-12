//! Zilog Z80, modelled for the Mega Drive's sound CPU.
//!
//! The Mega Drive runs a bare Z80 with no ROM of its own: the 68000 uploads a
//! driver into Z80 RAM and starts it at an address it writes to the top of that
//! RAM. Everything the driver can touch is therefore reachable through this
//! core, and the driver is ordinary Z80 code - so a partial implementation
//! fails on real drivers in ways that read as bugs in this emulator rather than
//! as missing instructions. Hence the full base set plus the CB, ED, DD and FD
//! prefixes.
//!
//! Two hardware details differ from a standalone Z80 and are deliberately *not*
//! modelled here, because they belong to the cartridge rather than the chip:
//! the bus request handshake and the reset line. Both are in `System`, which
//! owns the arbitration; this core only sees memory and I/O ports.

use std::fmt;

/// Flag bits in F. The undocumented bits 3 and 5 mirror bits 5 and 3 of the
/// result on many arithmetic operations; sound drivers do not read them, but
/// `EX AF,AF'` copies F wholesale, so they have to exist to stay faithful.
pub const CF: u8 = 0x01;
pub const NF: u8 = 0x02;
pub const PF: u8 = 0x04;
pub const XF: u8 = 0x08;
pub const HF: u8 = 0x10;
pub const YF: u8 = 0x20;
pub const ZF: u8 = 0x40;
pub const SF: u8 = 0x80;

/// Interrupt modes, as programmed with IM 0/1/2.
pub const IM_0: u8 = 0;
pub const IM_1: u8 = 1;
pub const IM_2: u8 = 2;

/// The memory and port interface a Z80 sees.
///
/// On the Mega Drive this is a window onto Z80 RAM, the YM2612, the PSG and a
/// bank of 68K ROM - not a 64 KiB address space - so the bus is abstract rather
/// than an array.
pub trait Z80Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);
    fn port_in(&mut self, port: u16) -> u8 {
        let _ = port;
        0xFF
    }
    fn port_out(&mut self, port: u16, value: u8) {
        let _ = (port, value);
    }
    /// True while the Z80 is held in reset. The core does not run until this
    /// clears, which is what makes the 68000's reset handshake work.
    fn in_reset(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy)]
pub struct Z80 {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    /// Shadow set, reachable only through `EX AF,AF'` and `EXX`.
    pub a_: u8,
    pub f_: u8,
    pub bc_: u16,
    pub de_: u16,
    pub hl_: u16,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    /// Interrupt vector high byte, and the refresh register `R`, which is
    /// incremented by every instruction fetch even when its value is never
    /// read - some drivers poll it as a cheap delay.
    pub i: u8,
    pub r: u8,
    pub im: u8,
    pub iff1: bool,
    pub iff2: bool,
    /// HALT parks the CPU until an interrupt; it is not a no-op.
    pub halted: bool,
    /// Set by an edge on the interrupt line, cleared when the CPU accepts it.
    pub irq_pending: bool,
    /// Total T-states retired. The Mega Drive clocks the Z80 at half the 68000
    /// rate, so this drives the YM2612's sample clock.
    pub cycles: u64,
}

impl Default for Z80 {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Z80 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Z80 {{ pc={:04X} sp={:04X} af={:04X} bc={:04X} de={:04X} hl={:04X} \
             ix={:04X} iy={:04X} r={:02X} im={} iff1={} halted={} }}",
            self.pc,
            self.sp,
            self.af(),
            self.bc(),
            self.de(),
            self.hl(),
            self.ix,
            self.iy,
            self.r,
            self.im,
            self.iff1,
            self.halted
        )
    }
}

// Register pairs. These are the accessors used everywhere; the fields stay
// separate because the prefix instructions substitute IX or IY for HL without
// disturbing it, which the paired form would hide.
impl Z80 {
    pub fn new() -> Self {
        Z80 {
            a: 0,
            f: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            a_: 0,
            f_: 0,
            bc_: 0,
            de_: 0,
            hl_: 0,
            ix: 0,
            iy: 0,
            sp: 0,
            pc: 0,
            i: 0,
            r: 0,
            im: IM_0,
            iff1: false,
            iff2: false,
            halted: false,
            irq_pending: false,
            cycles: 0,
        }
    }

    /// Power-on reset: everything undefined is zero, and the CPU starts
    /// halted-by-reset until the 68000 releases it.
    pub fn reset(&mut self) {
        let fresh = Z80::new();
        self.a = fresh.a;
        self.f = fresh.f;
        self.sp = 0xFFFF;
        self.pc = 0;
        self.im = IM_0;
        self.iff1 = false;
        self.iff2 = false;
        self.halted = false;
        self.irq_pending = false;
    }

    #[inline]
    pub fn af(&self) -> u16 {
        ((self.a as u16) << 8) | self.f as u16
    }
    #[inline]
    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = v as u8;
    }
    #[inline]
    pub fn bc(&self) -> u16 {
        ((self.b as u16) << 8) | self.c as u16
    }
    #[inline]
    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    #[inline]
    pub fn de(&self) -> u16 {
        ((self.d as u16) << 8) | self.e as u16
    }
    #[inline]
    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    #[inline]
    pub fn hl(&self) -> u16 {
        ((self.h as u16) << 8) | self.l as u16
    }
    #[inline]
    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }

    // -- flag helpers -------------------------------------------------------

    #[inline]
    pub fn flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }
    #[inline]
    pub fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }

    /// Set S, Z and the undocumented X/Y bits from a result. The documented
    /// flags are what software branches on; X and Y ride along so that code
    /// copying F between registers keeps its behaviour.
    #[inline]
    pub fn set_sz(&mut self, v: u8) {
        self.f &= !(SF | ZF | XF | YF);
        self.f |= v & (SF | XF | YF);
        if v == 0 {
            self.f |= ZF;
        }
    }

    #[inline]
    pub fn set_szp(&mut self, v: u8) {
        self.set_sz(v);
        self.set_flag(PF, parity(v));
    }

    #[inline]
    pub fn push(&mut self, bus: &mut impl Z80Bus, value: u16) {
        self.sp = self.sp.wrapping_sub(2);
        bus.write(self.sp, (value >> 8) as u8);
        bus.write(self.sp.wrapping_add(1), value as u8);
    }

    #[inline]
    pub fn pop(&mut self, bus: &mut impl Z80Bus) -> u16 {
        // push writes the high byte at SP, so pop reads it from SP. Reading the
        // pair the other way round returns every 16-bit value byte-swapped,
        // which looks like a corrupt stack rather than a stack convention.
        let hi = bus.read(self.sp) as u16;
        let lo = bus.read(self.sp.wrapping_add(1)) as u16;
        self.sp = self.sp.wrapping_add(2);
        (hi << 8) | lo
    }

    /// Fetch the byte at PC and advance. R increments on every opcode fetch, so
    /// it happens here rather than at the dispatch site - that is what makes
    /// its value differ between a taken and an untaken branch.
    #[inline]
    fn fetch(&mut self, bus: &mut impl Z80Bus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        self.r = self.r.wrapping_add(1) & 0x7F;
        v
    }

    #[inline]
    fn fetch_word(&mut self, bus: &mut impl Z80Bus) -> u16 {
        let lo = self.fetch(bus) as u16;
        let hi = self.fetch(bus) as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn read_word(&self, bus: &mut impl Z80Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi = bus.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn write_word(&self, bus: &mut impl Z80Bus, addr: u16, value: u16) {
        bus.write(addr, (value >> 8) as u8);
        bus.write(addr.wrapping_add(1), value as u8);
    }
}

/// Even parity of the low byte: set when a register holds a value with an even
/// number of set bits.
pub fn parity(v: u8) -> bool {
    (v as u8).count_ones() % 2 == 0
}

/// One T-state the CPU spends on an internal operation rather than the bus.
const INTERNAL: u32 = 4;

#[cfg(test)]
#[path = "z80/tests.rs"]
mod tests;

/// The eight ALU operations `ADD/ADC/SUB/SBC/AND/XOR/OR/CP` share.
///
/// Each takes the operand rather than reaching for the register file, because
/// the same operation has to serve `ALU A,r`, `ALU A,n`, the accumulator forms
/// `ADD A,r` and `ADD A,(HL)`, and the register-immediate variants - all with
/// identical flag behaviour, which is exactly the part that is easy to get
/// subtly wrong per call site.
impl Z80 {
    fn alu(&mut self, op: u8, operand: u8) {
        let a = self.a;
        match op {
            0x0 => self.add(operand, false),
            0x1 => self.add(operand, true),
            0x2 => self.sub(operand, false, false),
            0x3 => self.sub(operand, true, false),
            0x4 => {
                self.a = a & operand;
                self.f &= !(CF | NF);
                self.set_szp(self.a);
            }
            0x5 => {
                self.a = a ^ operand;
                self.f &= !(CF | NF);
                self.set_szp(self.a);
            }
            0x6 => {
                self.a = a | operand;
                self.f &= !(CF | NF);
                self.set_szp(self.a);
            }
            _ => self.cmp(operand),
        }
    }

    fn add(&mut self, operand: u8, with_carry: bool) {
        let a = self.a;
        let carry_in = if with_carry { self.f & CF } else { 0 } as u8;
        // Computed in u16: doing this in u8 wraps the result before the carry
        // test, so carry out of an 8-bit add can never be observed and every
        // chain that depends on it silently produces the wrong answer.
        let wide = a as u16 + operand as u16 + carry_in as u16;
        let r = wide as u8;
        let half = (a & 0x0F)
            .wrapping_add(operand & 0x0F)
            .wrapping_add(carry_in)
            > 0x0F;
        self.f &= !(CF | NF);
        self.set_flag(HF, half);
        self.set_flag(CF, wide > 0xFF);
        // Overflow is signed overflow: both operands share a sign that the
        // result does not.
        self.set_flag(PF, ((a ^ r) & (operand ^ r) & 0x80) != 0);
        self.a = r;
        self.set_sz(self.a);
    }

    /// `SUB`, `SBC` and `CP` are one routine: CP is SUB with the result thrown
    /// away, which is why CP leaves A untouched but still sets every flag.
    fn sub(&mut self, operand: u8, with_carry: bool, keep: bool) {
        let a = self.a;
        let carry_in = if with_carry { self.f & CF } else { 0 };
        let r = a.wrapping_sub(operand).wrapping_sub(carry_in);
        let half = (a & 0x0F)
            .wrapping_sub(operand & 0x0F)
            .wrapping_sub(carry_in)
            > 0x0F;
        self.f &= !(CF | NF);
        self.set_flag(NF, true);
        self.set_flag(HF, half);
        self.set_flag(CF, a < operand.wrapping_add(carry_in));
        self.set_flag(PF, ((a ^ operand) & (a ^ r) & 0x80) != 0);
        if !keep {
            self.a = r;
        }
        self.set_sz(r);
    }

    fn cmp(&mut self, operand: u8) {
        self.sub(operand, false, true);
    }

    fn inc(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        // INC clears the subtract and half-carry flags but *preserves* carry.
        // Clearing CF here is the single easiest flag bug on the chip, because
        // every carry-clocking loop in a driver depends on it.
        self.f &= !NF;
        self.set_flag(HF, (v & 0x0F) == 0x0F);
        self.set_flag(PF, v == 0x7F);
        self.set_sz(r);
        r
    }

    fn dec(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        // As with INC: carry is untouched.
        self.f &= !NF;
        self.set_flag(NF, true);
        self.set_flag(HF, (v & 0x0F) == 0x00);
        self.set_flag(PF, v == 0x80);
        self.set_sz(r);
        r
    }

    /// ADD HL,rr. Half-carry comes out of bit 11, and neither carry nor the
    /// zero flag is touched - this is a 16-bit add on top of 8-bit flag state.
    fn add_hl(&mut self, value: u16) {
        let hl = self.hl();
        let r = hl.wrapping_add(value);
        self.f &= !(CF | NF | ZF | SF);
        self.set_flag(HF, ((hl & 0x0FFF) + (value & 0x0FFF)) > 0x0FFF);
        self.set_flag(CF, r < hl);
        self.set_hl(r);
    }

    fn add_sp_offset(&mut self, offset: i8) -> u16 {
        let sp = self.sp;
        // Bit 3 and bit 11 of SP drive half-carry and carry, taken from the
        // *unsigned* low byte, and the sign-extended offset is added.
        self.f &= !(CF | NF | ZF | SF);
        self.set_flag(HF, (sp & 0x000F) + u16::from(offset as u8 & 0x0F) > 0x0F);
        self.set_flag(CF, (sp & 0x00FF) + u16::from(offset as u8) > 0xFF);
        sp.wrapping_add(offset as i16 as u16)
    }

    /// The eight rotates and shifts, shared by the CB-prefixed forms and by
    /// the undocumented `ED`-prefixed duplicates.
    fn rotate(&mut self, op: u8, v: u8) -> u8 {
        let r = match op {
            0 => {
                // RLC: carry out of bit 0, bit 7 in
                let r = v.rotate_left(1);
                self.f = (self.f & CF) | (r & (SF | XF | YF));
                self.set_flag(CF, v & 0x80 != 0);
                r
            }
            1 => {
                // RRC
                let r = v.rotate_right(1);
                self.f = (self.f & CF) | (r & (SF | XF | YF));
                self.set_flag(CF, v & 0x01 != 0);
                r
            }
            2 => {
                // RL: carry into bit 0, old carry out of bit 7
                let carry_in = self.f & CF;
                let r = (v << 1) | (carry_in >> 3);
                self.f = (v & (SF | XF | YF)) | (self.f & (CF | PF));
                self.set_flag(CF, v & 0x80 != 0);
                self.set_szp(r);
                r
            }
            3 => {
                // RR
                let carry_in = self.f & CF;
                let r = (v >> 1) | (carry_in << 7);
                self.f = (v & (SF | XF | YF)) | (self.f & (CF | PF));
                self.set_flag(CF, v & 0x01 != 0);
                self.set_szp(r);
                r
            }
            4 => {
                // SLA: bit 0 always cleared
                let r = v << 1;
                self.f = (self.f & (CF | PF)) | (r & (SF | XF | YF));
                self.set_flag(CF, v & 0x80 != 0);
                self.set_szp(r);
                r
            }
            5 => {
                // SRA: bit 0 always set, sign preserved
                let r = (v >> 1) | (v & 0x80);
                self.f = (self.f & (CF | PF)) | (r & (SF | XF | YF));
                self.set_flag(CF, v & 0x01 != 0);
                self.set_szp(r);
                r
            }
            6 => {
                // SLL, undocumented: like SLA but bit 0 always set
                let r = (v << 1) | 0x01;
                self.f = (self.f & (CF | PF)) | (r & (SF | XF | YF));
                self.set_flag(CF, v & 0x80 != 0);
                self.set_szp(r);
                r
            }
            _ => {
                // SRL, undocumented: bit 7 always cleared
                let r = v >> 1;
                self.f = (self.f & (CF | PF)) | (r & (SF | XF | YF));
                self.set_flag(CF, v & 0x01 != 0);
                self.set_szp(r);
                r
            }
        };
        self.set_szp(r);
        r
    }

    fn daa(&mut self) {
        // Decimal adjust after a byte add or subtract, in BCD. The correction
        // differs by operation, which is what NF means here.
        let mut a = self.a;
        let mut adjust = 0;
        let mut carry = self.f & CF;
        if (a & 0x0F) > 9 || self.f & HF != 0 {
            adjust |= 0x06;
        }
        if a > 0x99 || self.f & CF != 0 {
            adjust |= 0x60;
            carry = CF;
        }
        if self.f & NF != 0 {
            a = a.wrapping_sub(adjust);
        } else {
            a = a.wrapping_add(adjust);
        }
        self.f &= !(HF | CF);
        self.f |= carry;
        self.a = a;
        self.set_sz(self.a);
    }
}

/// Which index register, if any, a DD or FD prefix installs in place of HL.
#[derive(Clone, Copy, PartialEq)]
enum Index {
    None,
    Ix,
    Iy,
}

/// 0 = B, 1 = C, 2 = D, 3 = E, 4 = H, 5 = L, 6 = (HL) or (IX+d), 7 = A.
impl Z80 {
    fn get_reg(&self, bus: &mut impl Z80Bus, idx: Index, y: u8) -> u8 {
        match y {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => match idx {
                Index::None => self.h,
                Index::Ix => self.ix as u8,
                Index::Iy => self.iy as u8,
            },
            5 => match idx {
                Index::None => self.l,
                Index::Ix => (self.ix >> 8) as u8,
                Index::Iy => (self.iy >> 8) as u8,
            },
            6 => bus.read(self.ea_addr(idx)),
            _ => self.a,
        }
    }

    fn set_reg(&mut self, bus: &mut impl Z80Bus, idx: Index, y: u8, value: u8) {
        match y {
            0 => self.b = value,
            1 => self.c = value,
            2 => self.d = value,
            3 => self.e = value,
            4 => match idx {
                Index::None => self.h = value,
                Index::Ix => self.ix = (self.ix & 0x00FF) | ((value as u16) << 8),
                Index::Iy => self.iy = (self.iy & 0x00FF) | ((value as u16) << 8),
            },
            5 => match idx {
                Index::None => self.l = value,
                Index::Ix => self.ix = (self.ix & 0xFF00) | value as u16,
                Index::Iy => self.iy = (self.iy & 0xFF00) | value as u16,
            },
            6 => bus.write(self.ea_addr(idx), value),
            _ => self.a = value,
        }
    }

    /// The plain address an operand refers to: HL, or the index register a
    /// prefix installed. No displacement byte is involved; callers that want one
    /// fetch it themselves, because whether a displacement exists at all
    /// depends on the opcode and not just on the prefix.
    fn ea_addr(&self, idx: Index) -> u16 {
        match idx {
            Index::None => self.hl(),
            Index::Ix => self.ix,
            Index::Iy => self.iy,
        }
    }

    /// As `ea_addr`, but consuming a signed displacement byte. Used where the
    /// opcode is known to take one even without a prefix.
    fn effective(&mut self, bus: &mut impl Z80Bus, idx: Index) -> u16 {
        match idx {
            Index::None => self.hl(),
            _ => {
                let d = bus.read(self.pc) as i8;
                self.pc = self.pc.wrapping_add(1);
                let base = if idx == Index::Ix { self.ix } else { self.iy };
                base.wrapping_add(d as i16 as u16)
            }
        }
    }

    /// The address of a (IX+d)-style operand without advancing PC, used by the
    /// DD/FD-prefixed `LD r,(IX+d)` forms which fetch the displacement once and
    /// use it for both sides.
    fn index_addr(&self, bus: &mut impl Z80Bus, idx: Index) -> u16 {
        match idx {
            Index::None => self.hl(),
            _ => {
                let d = bus.read(self.pc) as i8;
                let base = if idx == Index::Ix { self.ix } else { self.iy };
                base.wrapping_add(d as i16 as u16)
            }
        }
    }

    fn get_pair(&self, p: u8, idx: Index) -> u16 {
        match p {
            0 => self.bc(),
            1 => self.de(),
            2 => match idx {
                Index::None => self.hl(),
                Index::Ix => self.ix,
                Index::Iy => self.iy,
            },
            _ => self.sp,
        }
    }

    fn set_pair(&mut self, p: u8, idx: Index, value: u16) {
        match p {
            0 => self.set_bc(value),
            1 => self.set_de(value),
            2 => match idx {
                Index::None => self.set_hl(value),
                Index::Ix => self.ix = value,
                Index::Iy => self.iy = value,
            },
            _ => self.sp = value,
        }
    }

    fn condition(&self, cc: u8) -> bool {
        match cc {
            0 => !self.flag(ZF),
            1 => !self.flag(SF),
            2 => !self.flag(PF),
            3 => !self.flag(CF),
            4 => self.flag(ZF),
            5 => self.flag(SF),
            6 => self.flag(PF),
            _ => self.flag(CF),
        }
    }

    /// Accept a maskable interrupt if one is pending and enabled. RST 38 is the
    /// IM 1 vector, IM 2 uses I:opaque, IM 0 is whatever is on the bus - the
    /// latter is not a mode we can execute meaningfully, so it pushes 0xFF,
    /// which is what a Z80 with no interrupt controller on the data bus does.
    fn service_interrupt(&mut self, bus: &mut impl Z80Bus) -> Option<u32> {
        if !self.irq_pending || !self.iff1 {
            return None;
        }
        self.iff1 = false;
        self.iff2 = false;
        self.halted = false;
        self.r = self.r.wrapping_add(1) & 0x7F;
        let vector = match self.im {
            IM_1 => 0x38,
            IM_2 => {
                let page = self.i as u16;
                let addr = (page << 8) | 0xFF;
                self.read_word(bus, addr)
            }
            _ => 0xFF,
        };
        self.push(bus, self.pc);
        self.pc = vector;
        self.cycles += 13;
        Some(13)
    }
}

impl Z80 {
    /// Execute one instruction. Returns the T-states it retired, or None when
    /// the CPU is held in reset and did nothing.
    pub fn step(&mut self, bus: &mut impl Z80Bus) -> Option<u32> {
        if bus.in_reset() {
            return None;
        }
        // A halted CPU with no interrupt is not idle: it keeps executing NOPs
        // and keeps R moving. Returning early instead would make any driver
        // that polls R during HALT drift.
        // The interrupt check comes before the HALT check. A halted CPU still
        // accepts a masked interrupt and resumes; testing `halted` first would
        // park the CPU forever the moment it halted with interrupts enabled.
        if let Some(n) = self.service_interrupt(bus) {
            self.cycles += n as u64;
            return Some(n);
        }
        let states = if self.halted {
            self.r = self.r.wrapping_add(1) & 0x7F;
            4
        } else {
            self.execute(bus)
        };
        self.cycles += states as u64;
        Some(states)
    }

    fn execute(&mut self, bus: &mut impl Z80Bus) -> u32 {
        let op = self.fetch(bus);
        match op {
            0xCB => self.execute_cb(bus, Index::None),
            0xED => self.execute_ed(bus),
            // DD/FD substitute an index register for HL. The prefix reaches
            // register pairs as well as memory: DD 21 nn nn is LD IX,nn and
            // DD E9 is JP (IX), so this is threaded through every group rather
            // than bolted onto the memory operands.
            0xDD | 0xFD => {
                let idx = if op == 0xDD { Index::Ix } else { Index::Iy };
                let second = self.fetch(bus);
                // A prefix directly in front of another prefix is a NOP that
                // spans both bytes.
                if second == 0xDD || second == 0xFD || second == 0xED {
                    return 4;
                }
                self.execute_op(bus, second, idx)
            }
            _ => self.execute_op(bus, op, Index::None),
        }
    }

    fn execute_op(&mut self, bus: &mut impl Z80Bus, op: u8, idx: Index) -> u32 {
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;
        let prefixed = idx != Index::None;

        match x {
            1 => {
                // HALT is 0x76; under a prefix it is not an opcode at all.
                if op == 0x76 && !prefixed {
                    self.halted = true;
                    return 4;
                }
                if z == 6 {
                    // LD r,(IX+d) consumes exactly one displacement byte.
                    if prefixed {
                        let d = self.fetch(bus) as i8;
                        let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                        let v = bus.read(addr);
                        self.set_reg(bus, Index::None, y, v);
                        return 19;
                    }
                    let v = bus.read(self.hl());
                    self.set_reg(bus, Index::None, y, v);
                    return 7;
                }
                let v = self.get_reg(bus, idx, z);
                self.set_reg(bus, idx, y, v);
                4
            }
            2 => {
                if z == 6 && prefixed {
                    let d = self.fetch(bus) as i8;
                    let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                    let v = bus.read(addr);
                    self.alu(y, v);
                    return 19;
                }
                let v = self.get_reg(bus, idx, z);
                self.alu(y, v);
                4
            }
            0 => self.group_0(bus, op, y, z, p, q, idx),
            _ => self.group_3(bus, op, y, z, p, q, idx),
        }
    }

    fn group_0(
        &mut self,
        bus: &mut impl Z80Bus,
        _op: u8,
        y: u8,
        z: u8,
        p: u8,
        q: u8,
        idx: Index,
    ) -> u32 {
        match z {
            0 => match y {
                0 => 4, // NOP
                1 => {
                    std::mem::swap(&mut self.a, &mut self.a_);
                    std::mem::swap(&mut self.f, &mut self.f_);
                    4
                }
                2 => {
                    let d = self.fetch(bus) as i8;
                    self.b = self.b.wrapping_sub(1);
                    if self.b != 0 {
                        self.pc = self.pc.wrapping_add(d as i16 as u16);
                        13
                    } else {
                        8
                    }
                }
                _ => {
                    let d = self.fetch(bus) as i8;
                    if y == 3 || self.condition(y - 4) {
                        self.pc = self.pc.wrapping_add(d as i16 as u16);
                        12
                    } else {
                        7
                    }
                }
            },
            1 => {
                if q == 0 {
                    let v = self.fetch_word(bus);
                    self.set_pair(p, idx, v);
                    10
                } else {
                    let v = self.get_pair(p, idx);
                    self.add_hl(v);
                    11
                }
            }
            2 => match (p, q) {
                (0, 0) => {
                    let a = self.a;
                    bus.write(self.bc(), a);
                    7
                }
                (1, 0) => {
                    let a = self.a;
                    bus.write(self.de(), a);
                    7
                }
                (2, 0) | (3, 0) => {
                    let addr = self.fetch_word(bus);
                    let v = if p == 2 {
                        self.get_pair(2, idx)
                    } else {
                        self.a as u16
                    };
                    bus.write(addr, (v >> 8) as u8);
                    bus.write(addr.wrapping_add(1), v as u8);
                    16
                }
                (0, 1) => {
                    self.a = bus.read(self.bc());
                    7
                }
                (1, 1) => {
                    self.a = bus.read(self.de());
                    7
                }
                (2, 1) | (3, 1) => {
                    let addr = self.fetch_word(bus);
                    let v = self.read_word(bus, addr);
                    if p == 2 {
                        self.set_pair(2, idx, v);
                    } else {
                        self.a = v as u8;
                    }
                    16
                }
                _ => 4,
            },
            3 => {
                let v = self.get_pair(p, idx);
                let v = if q == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.set_pair(p, idx, v);
                6
            }
            4 | 5 => {
                // INC/DEC on a register, or on (IX+d) which consumes a
                // displacement the other register forms do not.
                if y == 6 && idx != Index::None {
                    let d = self.fetch(bus) as i8;
                    let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                    let v = bus.read(addr);
                    let r = if z == 4 { self.inc(v) } else { self.dec(v) };
                    bus.write(addr, r);
                    return 23;
                }
                let v = self.get_reg(bus, idx, y);
                let r = if z == 4 { self.inc(v) } else { self.dec(v) };
                self.set_reg(bus, idx, y, r);
                4
            }
            6 => {
                let v = self.fetch(bus);
                self.set_reg(bus, idx, y, v);
                7
            }
            _ => match y {
                0 | 1 | 2 | 3 => {
                    // The four accumulator rotates deliberately leave Z and P
                    // alone, which is why they cannot share the CB helper.
                    // Arithmetic is done in u16 because a left shift of a byte
                    // overflows, and an emulator that panics on the game under
                    // test is worse than useless.
                    let a = self.a as u16;
                    let carry_in = if self.flag(CF) { 1u16 } else { 0 };
                    let (r, new_carry) = match y {
                        0 => ((a << 1 | a >> 7) as u8, (a >> 7) as u8),
                        1 => ((a >> 1 | a << 7) as u8, (a & 1) as u8),
                        2 => ((a << 1 | carry_in) as u8, (a >> 7) as u8),
                        _ => ((a >> 1 | carry_in << 7) as u8, (a & 1) as u8),
                    };
                    self.f = (self.f & (SF | ZF | PF | XF | YF)) | new_carry;
                    self.a = r;
                    4
                }
                4 => {
                    self.daa();
                    4
                }
                5 => {
                    self.a = !self.a;
                    self.f |= HF | NF;
                    self.set_szp(self.a);
                    4
                }
                6 => {
                    self.f &= !(HF | NF);
                    self.set_flag(CF, true);
                    4
                }
                _ => {
                    self.f &= !(HF | NF);
                    self.set_flag(CF, !self.flag(CF));
                    4
                }
            },
        }
    }

    fn group_3(
        &mut self,
        bus: &mut impl Z80Bus,
        _op: u8,
        y: u8,
        z: u8,
        p: u8,
        q: u8,
        idx: Index,
    ) -> u32 {
        match z {
            0 => {
                if self.condition(y) {
                    self.pc = self.pop(bus);
                    11
                } else {
                    5
                }
            }
            1 => {
                if q == 0 {
                    let v = self.pop(bus);
                    match p {
                        0 => self.set_bc(v),
                        1 => self.set_de(v),
                        2 => self.set_pair(2, idx, v),
                        _ => self.a = v as u8,
                    }
                    10
                } else {
                    match p {
                        0 => {
                            self.pc = self.pop(bus);
                            10
                        }
                        1 => {
                            let (bc_, de_, hl_) = (self.bc_, self.de_, self.hl_);
                            self.set_bc(self.bc());
                            self.set_de(self.de());
                            self.set_hl(self.hl());
                            self.bc_ = bc_;
                            self.de_ = de_;
                            self.hl_ = hl_;
                            4
                        }
                        2 => {
                            self.pc = self.get_pair(2, idx);
                            4
                        }
                        _ => {
                            self.sp = self.get_pair(2, idx);
                            6
                        }
                    }
                }
            }
            2 => {
                let addr = self.fetch_word(bus);
                if self.condition(y) {
                    self.pc = addr;
                }
                10
            }
            3 => match y {
                0 => {
                    self.pc = self.fetch_word(bus);
                    10
                }
                2 => {
                    let port = self.fetch(bus) as u16;
                    let a = self.a;
                    bus.port_out(port, a);
                    11
                }
                3 => {
                    let port = self.fetch(bus) as u16;
                    self.a = bus.port_in(port);
                    11
                }
                4 => {
                    // EX (SP),HL or EX (SP),IX under a prefix.
                    let v = self.read_word(bus, self.sp);
                    let (lo, hi) = match idx {
                        Index::None => (self.l, self.h),
                        Index::Ix => (self.ix as u8, (self.ix >> 8) as u8),
                        Index::Iy => (self.iy as u8, (self.iy >> 8) as u8),
                    };
                    self.set_pair(2, idx, v);
                    bus.write(self.sp, lo);
                    bus.write(self.sp.wrapping_add(1), hi);
                    19
                }
                5 => {
                    let de = self.de();
                    self.set_de(self.hl());
                    self.set_hl(de);
                    4
                }
                6 => {
                    self.iff1 = false;
                    self.iff2 = false;
                    4
                }
                _ => {
                    self.iff1 = true;
                    self.iff2 = true;
                    4
                }
            },
            4 => {
                let addr = self.fetch_word(bus);
                if self.condition(y) {
                    self.push(bus, self.pc);
                    self.pc = addr;
                    17
                } else {
                    10
                }
            }
            5 => {
                if q == 0 {
                    let value = match p {
                        0 => self.bc(),
                        1 => self.de(),
                        2 => self.get_pair(2, idx),
                        _ => self.af(),
                    };
                    self.push(bus, value);
                    11
                } else if p == 0 {
                    let addr = self.fetch_word(bus);
                    self.push(bus, self.pc);
                    self.pc = addr;
                    17
                } else {
                    4
                }
            }
            6 => {
                let v = self.fetch(bus);
                self.alu(y, v);
                7
            }
            _ => {
                self.push(bus, self.pc);
                self.pc = (y as u16) * 8;
                11
            }
        }
    }

    /// CB prefix: the bit operations and rotates, all of which operate on a
    /// single byte addressed by operand y.
    fn execute_cb(&mut self, bus: &mut impl Z80Bus, idx: Index) -> u32 {
        let op = self.fetch(bus);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;

        // In the CB page the operand and the opcode fields swap places between
        // the rotate group and the bit-op groups: for BIT/RES/SET the *bit
        // number* is in bits 5-3 and the *register* in bits 2-0, whereas for
        // the rotates it is the other way round. CB 7E is BIT 7,(HL) - not
        // BIT 6,A - and reading the fields the wrong way round makes every
        // bit operation in a driver operate on the wrong register.
        let (bit_number, reg) = if x == 0 { (z, y) } else { (y, z) };
        if x == 1 {
            // BIT b,r - the only read-only form, and the only one that leaves
            // the operand alone. Bit 7 of (HL) reads the byte at the address
            // rather than the address itself, and the undocumented X/Y half of
            // the flags comes from that value.
            let value = self.get_reg(bus, idx, reg);
            let bit = value & (1 << bit_number);
            self.f &= !(CF | NF);
            self.f |= HF;
            if bit == 0 {
                self.f |= ZF | PF;
                self.f &= !(SF | XF | YF);
            } else {
                self.f |= SF;
                // A clear bit means a *set* Z; leaving the previous Z in place
                // makes BIT inherit the flags of whatever ran before it.
                self.f &= !(ZF | PF);
                self.f |= if value & 0x40 != 0 { XF } else { 0 };
                self.f |= if value & 0x20 != 0 { YF } else { 0 };
            }
            return if reg == 6 { 12 } else { 8 };
        }

        // Every remaining CB form reads, transforms and writes back.
        // Only a prefixed form has a displacement byte to consume; plain
        // (HL) is a single byte and taking one here would desynchronise the
        // program counter from the instruction stream for every CB opcode.
        let (value, addr, states) = if reg == 6 {
            let addr = self.effective(bus, idx);
            (
                bus.read(addr),
                Some(addr),
                if idx == Index::None { 11 } else { 15 },
            )
        } else {
            (self.get_reg(bus, idx, reg), None, 8)
        };

        let result = match x {
            0 => self.rotate(bit_number, value),
            2 => value & !(1 << bit_number),
            _ => value | (1 << bit_number),
        };
        match addr {
            Some(a) => bus.write(a, result),
            None => self.set_reg(bus, idx, reg, result),
        }
        states
    }

    /// ED prefix: 16-bit arithmetic, block moves, and the block I/O the sound
    /// drivers lean on for the YM2612 and PSG.
    fn execute_ed(&mut self, bus: &mut impl Z80Bus) -> u32 {
        let op = self.fetch(bus);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;

        if x == 1 {
            if z <= 3 && y >= 4 {
                // IN r,(C) - the port address is BC whatever the destination,
                // and B is the only register that takes the full byte.
                let v = bus.port_in(self.bc());
                match y {
                    4 => self.b = v,
                    5 => self.c = v,
                    6 => self.halted = true, // IN (C) is a NOP except for its
                    _ => self.a = v,         // interrupt-triggering side effect
                }
                self.set_sz(v);
                self.f &= !NF;
                return 12;
            }
            if z <= 3 && y < 4 {
                // OUT (C),r
                let v = match y {
                    0 => self.b,
                    1 => self.c,
                    2 => self.d,
                    _ => self.e,
                };
                bus.port_out(self.bc(), v);
                return 12;
            }
            // SBC HL,rp (z = 4) and ADC HL,rp (z = 5). These live in the same
            // x = 1 space as the block instructions' siblings, so they are
            // selected by z here rather than by a wider pattern.
            if z == 4 || z == 5 {
                let v = self.get_pair(p, Index::None);
                if z == 4 {
                    self.sbc_hl(v);
                } else {
                    self.adc_hl(v);
                }
                return 15;
            }
        }

        match op {
            0x43 | 0x53 | 0x63 | 0x73 => {
                let addr = self.fetch_word(bus);
                let v = self.get_pair(z, Index::None);
                bus.write(addr, (v >> 8) as u8);
                bus.write(addr.wrapping_add(1), v as u8);
                return 20;
            }
            0x4B | 0x5B | 0x6B | 0x7B => {
                let addr = self.fetch_word(bus);
                let v = self.read_word(bus, addr);
                self.set_pair(z, Index::None, v);
                return 20;
            }
            0x44 | 0x4C | 0x54 | 0x5C | 0x64 | 0x6C | 0x74 | 0x7C => {
                // NEG
                let v = self.a;
                self.a = 0;
                self.sub(v, false, false);
                return 8;
            }
            0x45 | 0x55 | 0x65 | 0x75 | 0x5D | 0x6D | 0x7D => {
                // RETN / RETI
                self.pc = self.pop(bus);
                self.iff1 = self.iff2;
                return 14;
            }
            0x46 | 0x4E | 0x66 | 0x6E => {
                // IM 0
                self.im = IM_0;
                return 8;
            }
            0x56 | 0x76 => {
                self.im = IM_1;
                return 8;
            }
            0x5E | 0x7E => {
                self.im = IM_2;
                return 8;
            }
            0x47 => {
                self.i = self.a;
                return 9;
            }
            0x4F => {
                self.r = self.a;
                return 9;
            }
            0x57 => {
                self.a = self.i;
                return 9;
            }
            0x5F => {
                self.a = self.r;
                return 9;
            }
            0xA0 | 0xA8 | 0xB0 | 0xB8 => {
                // LDI/LDD/LDIR/LDDR. Bit 3 selects the direction and bit 4
                // makes it repeat - comparing the opcode against 0xB0 would
                // call CPIR backward, because CPIR is 0xB1 and CPDR is 0xB9.
                let forward = op & 0x08 == 0;
                let repeat = op & 0x10 != 0;
                let v = bus.read(self.hl());
                bus.write(self.de(), v);
                let hl = self.hl();
                let de = self.de();
                self.set_hl(if forward {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                });
                self.set_de(if forward {
                    de.wrapping_add(1)
                } else {
                    de.wrapping_sub(1)
                });
                let bc = self.bc().wrapping_sub(1);
                self.set_bc(bc);
                let n = (v.wrapping_add(self.a)) & 0x02 != 0;
                self.f &= !(PF | NF | HF);
                if bc != 0 {
                    self.set_flag(PF, true);
                }
                self.set_flag(YF, (v >> 3) & 1 != 0);
                self.set_flag(XF, (v >> 1) & 1 != 0);
                self.set_flag(NF, n);
                // A repeating block step leaves the program counter on the
                // instruction itself so the next fetch re-enters it. Without
                // stepping back over both prefix bytes the block runs exactly
                // once and the copy silently stops after one byte.
                return if repeat && bc != 0 {
                    self.pc = self.pc.wrapping_sub(2);
                    21
                } else {
                    16
                };
            }
            0xA1 | 0xA9 | 0xB1 | 0xB9 => {
                // CPI/CPD/CPIR/CPDR - BC is a byte counter, HL a search pair.
                let forward = op & 0x08 == 0;
                let repeat = op & 0x10 != 0;
                let a = self.a;
                let hl = self.hl();
                let v = bus.read(hl);
                let r = a.wrapping_sub(v);
                let half = (a & 0x0F).wrapping_sub(v & 0x0F) > 0x0F;
                self.f &= !(PF | NF | HF | CF);
                self.set_flag(NF, true);
                self.set_flag(HF, half);
                self.set_flag(CF, a < v);
                self.set_flag(PF, ((a ^ v) & (a ^ r) & 0x80) != 0);
                self.set_szp(r);
                let bc = self.bc().wrapping_sub(1);
                self.set_bc(bc);
                self.set_hl(if forward {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                });
                return if repeat && bc != 0 && r != 0 {
                    self.pc = self.pc.wrapping_sub(2);
                    21
                } else {
                    16
                };
            }
            0xA2 | 0xAA | 0xB2 | 0xBA => {
                // INI/IND/INIR/INDR
                let forward = op & 0x08 == 0;
                let v = bus.port_in(self.bc());
                bus.write(self.hl(), v);
                self.b = self.b.wrapping_sub(1);
                self.set_hl(self.hl().wrapping_add(if forward { 1 } else { u16::MAX }));
                return 16;
            }
            0xA3 | 0xAB | 0xB3 | 0xBB => {
                // OUTI/OUTD/OTIR/OTDR
                let forward = op & 0x08 == 0;
                let repeat = op & 0x10 != 0;
                let v = bus.read(self.hl());
                bus.port_out(self.bc(), v);
                self.b = self.b.wrapping_sub(1);
                self.set_hl(self.hl().wrapping_add(if forward { 1 } else { u16::MAX }));
                return if repeat && self.b != 0 {
                    self.pc = self.pc.wrapping_sub(2);
                    21
                } else {
                    16
                };
            }
            _ => {}
        }
        let _ = (p, q);
        // Everything else in the ED page is a double NOP on real hardware.
        8
    }

    fn adc_hl(&mut self, value: u16) {
        let hl = self.hl();
        let carry = if self.flag(CF) { 1u16 } else { 0 };
        let r = hl.wrapping_add(value).wrapping_add(carry);
        self.f &= !(CF | NF);
        self.set_flag(HF, (hl & 0x0FFF) + (value & 0x0FFF) + carry > 0x0FFF);
        self.set_flag(CF, r < hl);
        self.set_flag(PF, ((hl ^ r) & (value ^ r) & 0x8000) != 0);
        self.set_flag(NF, false);
        self.set_flag(SF, r & 0x8000 != 0);
        self.set_flag(ZF, r & 0xFFFF == 0);
        self.set_hl(r);
    }

    fn sbc_hl(&mut self, value: u16) {
        let hl = self.hl();
        let carry = if self.flag(CF) { 1u16 } else { 0 };
        let r = hl.wrapping_sub(value).wrapping_sub(carry);
        self.f &= !(CF | NF);
        self.set_flag(NF, true);
        self.set_flag(
            HF,
            (hl & 0x0FFF)
                .wrapping_sub(value & 0x0FFF)
                .wrapping_sub(carry)
                > 0x0FFF,
        );
        self.set_flag(CF, hl < value.wrapping_add(carry));
        self.set_flag(PF, ((hl ^ value) & (hl ^ r) & 0x8000) != 0);
        self.set_flag(SF, r & 0x8000 != 0);
        self.set_flag(ZF, r & 0xFFFF == 0);
        self.set_hl(r);
    }
}
