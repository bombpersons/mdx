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

/// T-states an instruction has consumed when its memory accesses happen in
/// the reference core (MAME-derived: the opcode table cost is charged before
/// the handler runs). Taken from this core's cost of the same bytes with
/// neutral registers (BC=1, so block repeats and DJNZ are not taken).
pub fn instruction_states(bytes: [u8; 4]) -> u32 {
    struct Scratch([u8; 8]);
    impl Z80Bus for Scratch {
        fn read(&mut self, addr: u16) -> u8 {
            self.0.get(addr as usize).copied().unwrap_or(0)
        }
        fn write(&mut self, _addr: u16, _value: u8) {}
    }
    let mut scratch = Scratch([0; 8]);
    scratch.0[..4].copy_from_slice(&bytes);
    let mut cpu = Z80::new();
    cpu.c = 1;
    cpu.step(&mut scratch).unwrap_or(0)
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Current level of the maskable INT line; acceptance does not clear the wire.
    pub irq_pending: bool,
    /// EI inhibits maskable interrupt acceptance until one more instruction retires.
    #[serde(default)]
    pub after_ei: bool,
    /// Internal memory pointer supplying undocumented BIT memory flags.
    #[serde(default)]
    pub wz: u16,
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
            after_ei: false,
            wz: 0,
            cycles: 0,
        }
    }

    /// Reference-compatible warm reset: reset control state while retaining
    /// general registers, stack and elapsed clocks. System owns the RESET wire.
    pub fn reset(&mut self) {
        self.pc = 0;
        self.i = 0;
        self.r = 0;
        self.im = IM_0;
        self.iff1 = false;
        self.iff2 = false;
        self.halted = false;
        self.after_ei = false;
        self.wz = 0;
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
        // The high byte is written first, but the final SP points at the low byte.
        bus.write(self.sp.wrapping_add(1), (value >> 8) as u8);
        bus.write(self.sp, value as u8);
    }

    #[inline]
    pub fn pop(&mut self, bus: &mut impl Z80Bus) -> u16 {
        let lo = bus.read(self.sp) as u16;
        let hi = bus.read(self.sp.wrapping_add(1)) as u16;
        self.sp = self.sp.wrapping_add(2);
        (hi << 8) | lo
    }

    /// Fetch an immediate/displacement byte; opcode M1 reads separately update R.
    #[inline]
    fn fetch(&mut self, bus: &mut impl Z80Bus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch_opcode(&mut self, bus: &mut impl Z80Bus) -> u8 {
        let v = self.fetch(bus);
        self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7f);
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
        bus.write(addr, value as u8);
        bus.write(addr.wrapping_add(1), (value >> 8) as u8);
    }
}

/// Even parity of the low byte: set when a register holds a value with an even
/// number of set bits.
pub fn parity(v: u8) -> bool {
    (v as u8).count_ones() % 2 == 0
}

/// One T-state the CPU spends on an internal operation rather than the bus.
const INTERNAL: u32 = 4;

#[path = "z80/disasm.rs"]
pub mod disasm;
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
                self.f = HF;
                self.set_szp(self.a);
            }
            0x5 => {
                self.a = a ^ operand;
                self.f = 0;
                self.set_szp(self.a);
            }
            0x6 => {
                self.a = a | operand;
                self.f = 0;
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
        self.set_flag(CF, u16::from(a) < u16::from(operand) + u16::from(carry_in));
        self.set_flag(PF, ((a ^ operand) & (a ^ r) & 0x80) != 0);
        if !keep {
            self.a = r;
        }
        self.set_sz(r);
    }

    fn cmp(&mut self, operand: u8) {
        self.sub(operand, false, true);
        self.f = (self.f & !(XF | YF)) | (operand & (XF | YF));
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

    /// ADD HL,rr preserves S/Z/PV; X/Y come from the result's high byte.
    fn add_hl(&mut self, value: u16) {
        let hl = self.hl();
        let r = hl.wrapping_add(value);
        self.f = (self.f & (SF | ZF | PF)) | ((r >> 8) as u8 & (XF | YF));
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
        let carry = self.f & CF;
        let (r, c) = match op {
            0 => (v.rotate_left(1), v >> 7),
            1 => (v.rotate_right(1), v & 1),
            2 => ((v << 1) | carry, v >> 7),
            3 => ((v >> 1) | (carry << 7), v & 1),
            4 => (v << 1, v >> 7),
            5 => ((v >> 1) | (v & 0x80), v & 1),
            6 => ((v << 1) | 1, v >> 7),
            _ => (v >> 1, v & 1),
        };
        self.f = c;
        self.set_szp(r);
        r
    }

    fn daa(&mut self) {
        let old = self.a;
        let mut correction = 0;
        if old & 15 > 9 || self.flag(HF) {
            correction |= 6;
        }
        let carry = self.flag(CF) || old > 0x99;
        if carry {
            correction |= 0x60;
        }
        self.a = if self.flag(NF) {
            old.wrapping_sub(correction)
        } else {
            old.wrapping_add(correction)
        };
        self.f = (self.f & NF) | if carry { CF } else { 0 } | ((old ^ self.a) & HF);
        self.set_szp(self.a);
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
                Index::Ix => (self.ix >> 8) as u8,
                Index::Iy => (self.iy >> 8) as u8,
            },
            5 => match idx {
                Index::None => self.l,
                Index::Ix => self.ix as u8,
                Index::Iy => self.iy as u8,
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
            1 => self.flag(ZF),
            2 => !self.flag(CF),
            3 => self.flag(CF),
            4 => !self.flag(PF),
            5 => self.flag(PF),
            6 => !self.flag(SF),
            _ => self.flag(SF),
        }
    }

    /// Accept a maskable interrupt if one is pending and enabled. RST 38 is the
    /// IM1 vector; IM2 fetches through I:FF. On the Mega Drive the undriven
    /// interrupt data bus supplies FF (RST38), also defining IM0 behavior.
    fn service_interrupt(&mut self, bus: &mut impl Z80Bus) -> Option<u32> {
        if !self.irq_pending || !self.iff1 || self.after_ei {
            return None;
        }
        self.iff1 = false;
        self.iff2 = false;
        self.halted = false;
        // The reference's take_interrupt does not advance R.
        self.push(bus, self.pc);
        let vector = match self.im {
            IM_1 => 0x38,
            IM_2 => {
                let page = self.i as u16;
                let addr = (page << 8) | 0xFF;
                self.read_word(bus, addr)
            }
            _ => 0x38,
        };
        self.pc = vector;
        self.wz = vector;
        Some(if self.im == IM_2 { 19 } else { 13 })
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
        // accepts an unmasked interrupt and resumes; testing `halted` first would
        // park the CPU forever the moment it halted with interrupts enabled.
        if let Some(n) = self.service_interrupt(bus) {
            self.cycles += n as u64;
            return Some(n);
        }
        self.after_ei = false;
        let states = if self.halted {
            self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7f);
            4
        } else {
            self.execute(bus)
        };
        self.cycles += states as u64;
        Some(states)
    }

    fn execute(&mut self, bus: &mut impl Z80Bus) -> u32 {
        let op = self.fetch_opcode(bus);
        match op {
            0xCB => self.execute_cb(bus, Index::None),
            0xED => self.execute_ed(bus),
            // DD/FD substitute an index register for HL. The prefix reaches
            // register pairs as well as memory: DD 21 nn nn is LD IX,nn and
            // DD E9 is JP (IX), so this is threaded through every group rather
            // than bolted onto the memory operands.
            0xDD | 0xFD => {
                let idx = if op == 0xDD { Index::Ix } else { Index::Iy };
                let second = self.fetch_opcode(bus);
                if second == 0xDD || second == 0xFD {
                    self.pc = self.pc.wrapping_sub(1);
                    self.r = (self.r & 0x80) | (self.r.wrapping_sub(1) & 0x7f);
                    return 4 + self.execute(bus);
                }
                if second == 0xED {
                    return 4 + self.execute_ed(bus);
                }
                if second == 0xCB {
                    return self.execute_index_cb(bus, idx);
                }
                let states = self.execute_op(bus, second, idx);
                let (x, y, z) = (second >> 6, (second >> 3) & 7, second & 7);
                let memory = (x == 1 && second != 0x76 && (y == 6 || z == 6))
                    || (x == 2 && z == 6)
                    || (x == 0 && y == 6 && (4..=6).contains(&z));
                states
                    + if memory {
                        0
                    } else if (x == 0 && (y == 4 || y == 5) && (4..=6).contains(&z))
                        || (x == 1 && ((y == 4 || y == 5) || (z == 4 || z == 5)))
                        || (x == 2 && (z == 4 || z == 5))
                    {
                        5
                    } else {
                        4
                    }
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
                // HALT keeps its meaning after an ignored index prefix.
                if op == 0x76 {
                    self.halted = true;
                    return 4;
                }
                if z == 6 {
                    // LD r,(IX+d) consumes exactly one displacement byte.
                    if prefixed {
                        let d = self.fetch(bus) as i8;
                        let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                        self.wz = addr;
                        let v = bus.read(addr);
                        self.set_reg(bus, Index::None, y, v);
                        return 19;
                    }
                    let v = bus.read(self.hl());
                    self.set_reg(bus, Index::None, y, v);
                    return 7;
                }
                if y == 6 && prefixed {
                    let d = self.fetch(bus) as i8;
                    let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                    self.wz = addr;
                    let v = self.get_reg(bus, Index::None, z);
                    bus.write(addr, v);
                    return 19;
                }
                let v = self.get_reg(bus, idx, z);
                self.set_reg(bus, idx, y, v);
                if y == 6 {
                    7
                } else {
                    4
                }
            }
            2 => {
                if z == 6 && prefixed {
                    let d = self.fetch(bus) as i8;
                    let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                    self.wz = addr;
                    let v = bus.read(addr);
                    self.alu(y, v);
                    return 19;
                }
                let v = self.get_reg(bus, idx, z);
                self.alu(y, v);
                if z == 6 {
                    7
                } else {
                    4
                }
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
                        self.wz = self.pc;
                        13
                    } else {
                        8
                    }
                }
                _ => {
                    let d = self.fetch(bus) as i8;
                    if y == 3 || self.condition(y - 4) {
                        self.pc = self.pc.wrapping_add(d as i16 as u16);
                        self.wz = self.pc;
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
                    let original = self.hl();
                    self.wz = self.get_pair(2, idx).wrapping_add(1);
                    self.set_hl(self.get_pair(2, idx));
                    self.add_hl(v);
                    let result = self.hl();
                    self.set_hl(original);
                    self.set_pair(2, idx, result);
                    11
                }
            }
            2 => match (p, q) {
                (0, 0) => {
                    let a = self.a;
                    bus.write(self.bc(), a);
                    self.wz = (self.bc().wrapping_add(1) & 0xFF) | (u16::from(a) << 8);
                    7
                }
                (1, 0) => {
                    let a = self.a;
                    bus.write(self.de(), a);
                    self.wz = (self.de().wrapping_add(1) & 0xFF) | (u16::from(a) << 8);
                    7
                }
                (2, 0) | (3, 0) => {
                    let addr = self.fetch_word(bus);
                    if p == 2 {
                        self.write_word(bus, addr, self.get_pair(2, idx));
                        self.wz = addr.wrapping_add(1);
                        16
                    } else {
                        bus.write(addr, self.a);
                        self.wz = (addr.wrapping_add(1) & 0xFF) | (u16::from(self.a) << 8);
                        13
                    }
                }
                (0, 1) => {
                    self.a = bus.read(self.bc());
                    self.wz = self.bc().wrapping_add(1);
                    7
                }
                (1, 1) => {
                    self.a = bus.read(self.de());
                    self.wz = self.de().wrapping_add(1);
                    7
                }
                (2, 1) | (3, 1) => {
                    let addr = self.fetch_word(bus);
                    self.wz = addr.wrapping_add(1);
                    if p == 2 {
                        let v = self.read_word(bus, addr);
                        self.set_pair(2, idx, v);
                        16
                    } else {
                        self.a = bus.read(addr);
                        13
                    }
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
                    self.wz = addr;
                    let v = bus.read(addr);
                    let r = if z == 4 { self.inc(v) } else { self.dec(v) };
                    bus.write(addr, r);
                    return 23;
                }
                let v = self.get_reg(bus, idx, y);
                let r = if z == 4 { self.inc(v) } else { self.dec(v) };
                self.set_reg(bus, idx, y, r);
                if y == 6 {
                    11
                } else {
                    4
                }
            }
            6 => {
                if y == 6 && idx != Index::None {
                    let d = self.fetch(bus) as i8;
                    let v = self.fetch(bus);
                    let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
                    self.wz = addr;
                    bus.write(addr, v);
                    return 19;
                }
                let v = self.fetch(bus);
                self.set_reg(bus, idx, y, v);
                if y == 6 {
                    10
                } else {
                    7
                }
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
                    self.f = (self.f & (SF | ZF | PF)) | (r & (XF | YF)) | new_carry;
                    self.a = r;
                    4
                }
                4 => {
                    self.daa();
                    4
                }
                5 => {
                    self.a = !self.a;
                    self.f = (self.f & (SF | ZF | PF | CF)) | HF | NF | (self.a & (XF | YF));
                    4
                }
                6 => {
                    self.f = (self.f & (SF | ZF | PF)) | CF | ((self.f | self.a) & (XF | YF));
                    4
                }
                _ => {
                    let carry = self.flag(CF);
                    self.f = (self.f & (SF | ZF | PF))
                        | ((self.f | self.a) & (XF | YF))
                        | if carry { HF } else { CF };
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
                    self.wz = self.pc;
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
                        _ => self.set_af(v),
                    }
                    10
                } else {
                    match p {
                        0 => {
                            self.pc = self.pop(bus);
                            self.wz = self.pc;
                            10
                        }
                        1 => {
                            let (bc, de, hl) = (self.bc(), self.de(), self.hl());
                            self.set_bc(self.bc_);
                            self.set_de(self.de_);
                            self.set_hl(self.hl_);
                            self.bc_ = bc;
                            self.de_ = de;
                            self.hl_ = hl;
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
                self.wz = addr;
                if self.condition(y) {
                    self.pc = addr;
                }
                10
            }
            3 => match y {
                0 => {
                    self.pc = self.fetch_word(bus);
                    self.wz = self.pc;
                    10
                }
                2 => {
                    let port = self.fetch(bus) as u16;
                    let a = self.a;
                    bus.port_out(port, a);
                    self.wz = (port.wrapping_add(1) & 0xFF) | (u16::from(a) << 8);
                    11
                }
                3 => {
                    let port = self.fetch(bus) as u16;
                    self.wz = ((u16::from(self.a) << 8) | port).wrapping_add(1);
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
                    self.wz = v;
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
                    self.after_ei = true;
                    4
                }
            },
            4 => {
                let addr = self.fetch_word(bus);
                self.wz = addr;
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
                    self.wz = addr;
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
                self.wz = self.pc;
                11
            }
        }
    }

    /// CB prefix: the bit operations and rotates, all of which operate on a
    /// single byte addressed by operand y.
    fn bit(&mut self, bit: u8, value: u8, xy: u8) {
        let clear = value & (1 << bit) == 0;
        self.f = (self.f & CF)
            | HF
            | (xy & (XF | YF))
            | if clear {
                ZF | PF
            } else if bit == 7 {
                SF
            } else {
                0
            };
    }

    fn execute_cb(&mut self, bus: &mut impl Z80Bus, _idx: Index) -> u32 {
        let op = self.fetch_opcode(bus);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let v = self.get_reg(bus, Index::None, z);
        if x == 1 {
            self.bit(y, v, if z == 6 { (self.wz >> 8) as u8 } else { v });
            return if z == 6 { 12 } else { 8 };
        }
        let result = match x {
            0 => self.rotate(y, v),
            2 => v & !(1 << y),
            _ => v | (1 << y),
        };
        self.set_reg(bus, Index::None, z, result);
        if z == 6 {
            15
        } else {
            8
        }
    }

    fn execute_index_cb(&mut self, bus: &mut impl Z80Bus, idx: Index) -> u32 {
        let d = self.fetch(bus) as i8;
        let op = self.fetch(bus);
        let addr = self.ea_addr(idx).wrapping_add(d as i16 as u16);
        self.wz = addr;
        self.wz = addr;
        let v = bus.read(addr);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        if x == 1 {
            self.bit(y, v, (addr >> 8) as u8);
            return 20;
        }
        let result = match x {
            0 => self.rotate(y, v),
            2 => v & !(1 << y),
            _ => v | (1 << y),
        };
        bus.write(addr, result);
        if z != 6 {
            self.set_reg(bus, Index::None, z, result);
        }
        23
    }

    /// ED prefix: 16-bit arithmetic, block moves, and the block I/O the sound
    /// drivers lean on for the YM2612 and PSG.
    fn execute_ed(&mut self, bus: &mut impl Z80Bus) -> u32 {
        let op = self.fetch_opcode(bus);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;

        if x == 1 {
            match z {
                0 => {
                    let v = bus.port_in(self.bc());
                    if y == 7 {
                        self.wz = self.bc().wrapping_add(1);
                    }
                    if y != 6 {
                        self.set_reg(bus, Index::None, y, v);
                    }
                    self.f &= CF;
                    self.set_szp(v);
                    return 12;
                }
                1 => {
                    let v = if y == 6 {
                        0
                    } else {
                        self.get_reg(bus, Index::None, y)
                    };
                    bus.port_out(self.bc(), v);
                    if y == 7 {
                        self.wz = self.bc().wrapping_add(1);
                    }
                    return 12;
                }
                2 => {
                    let v = self.get_pair(p, Index::None);
                    self.wz = self.hl().wrapping_add(1);
                    if q == 0 {
                        self.sbc_hl(v);
                    } else {
                        self.adc_hl(v);
                    }
                    return 15;
                }
                3 => {
                    let addr = self.fetch_word(bus);
                    if q == 0 {
                        self.write_word(bus, addr, self.get_pair(p, Index::None));
                    } else {
                        let v = self.read_word(bus, addr);
                        self.set_pair(p, Index::None, v);
                    }
                    self.wz = addr.wrapping_add(1);
                    return 20;
                }
                _ => {}
            }
        }

        match op {
            0x44 | 0x4C | 0x54 | 0x5C | 0x64 | 0x6C | 0x74 | 0x7C => {
                // NEG
                let v = self.a;
                self.a = 0;
                self.sub(v, false, false);
                return 8;
            }
            0x45 | 0x4D | 0x55 | 0x65 | 0x75 | 0x5D | 0x6D | 0x7D => {
                // RETN / RETI
                self.pc = self.pop(bus);
                self.wz = self.pc;
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
                self.f &= CF;
                self.set_sz(self.a);
                self.set_flag(PF, self.iff2);
                return 9;
            }
            0x5F => {
                self.a = self.r;
                self.f &= CF;
                self.set_sz(self.a);
                self.set_flag(PF, self.iff2);
                return 9;
            }
            0x67 | 0x6F => {
                let addr = self.hl();
                let v = bus.read(addr);
                let low = self.a & 15;
                let (memory, a) = if op == 0x67 {
                    ((low << 4) | (v >> 4), (self.a & 0xf0) | (v & 15))
                } else {
                    ((v << 4) | low, (self.a & 0xf0) | (v >> 4))
                };
                bus.write(addr, memory);
                self.a = a;
                self.f &= CF;
                self.set_szp(a);
                self.wz = addr.wrapping_add(1);
                return 18;
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
                let sum = v.wrapping_add(self.a);
                self.f &= !(PF | NF | HF);
                if bc != 0 {
                    self.set_flag(PF, true);
                }
                self.f = (self.f & !(XF | YF)) | (sum & XF) | ((sum & 2) << 4);
                // A repeating block step leaves the program counter on the
                // instruction itself so the next fetch re-enters it. Without
                // stepping back over both prefix bytes the block runs exactly
                // once and the copy silently stops after one byte.
                return if repeat && bc != 0 {
                    self.pc = self.pc.wrapping_sub(2);
                    self.wz = self.pc.wrapping_add(1);
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
                let bc = self.bc().wrapping_sub(1);
                self.set_bc(bc);
                self.wz = if forward { self.wz.wrapping_add(1) } else { self.wz.wrapping_sub(1) };
                let adjusted = r.wrapping_sub(u8::from(half));
                self.f = (self.f & CF) | NF | if half { HF } else { 0 };
                self.set_flag(SF, r & 0x80 != 0);
                self.set_flag(ZF, r == 0);
                self.set_flag(PF, bc != 0);
                self.f |= (adjusted & XF) | ((adjusted & 2) << 4);
                self.set_hl(if forward {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                });
                return if repeat && bc != 0 && r != 0 {
                    self.pc = self.pc.wrapping_sub(2);
                    self.wz = self.pc.wrapping_add(1);
                    21
                } else {
                    16
                };
            }
            0xA2 | 0xAA | 0xB2 | 0xBA => {
                let forward = op & 8 == 0;
                let repeat = op & 0x10 != 0;
                let v = bus.port_in(self.bc());
                self.wz = if forward { self.bc().wrapping_add(1) } else { self.bc().wrapping_sub(1) };
                bus.write(self.hl(), v);
                self.b = self.b.wrapping_sub(1);
                self.set_hl(self.hl().wrapping_add(if forward { 1 } else { u16::MAX }));
                let c = self.c.wrapping_add(if forward { 1 } else { u8::MAX });
                let sum = u16::from(v) + u16::from(c);
                self.block_io_flags(v, sum);
                // The reference schedules IN's bus phase with four extra T states.
                return if repeat && self.b != 0 {
                    self.pc = self.pc.wrapping_sub(2);
                    25
                } else {
                    20
                };
            }
            0xA3 | 0xAB | 0xB3 | 0xBB => {
                let forward = op & 8 == 0;
                let repeat = op & 0x10 != 0;
                let v = bus.read(self.hl());
                self.b = self.b.wrapping_sub(1);
                self.wz = if forward { self.bc().wrapping_add(1) } else { self.bc().wrapping_sub(1) };
                bus.port_out(self.bc(), v);
                self.set_hl(self.hl().wrapping_add(if forward { 1 } else { u16::MAX }));
                self.block_io_flags(v, u16::from(v) + u16::from(self.l));
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

    fn block_io_flags(&mut self, value: u8, sum: u16) {
        self.f = if value & 0x80 != 0 { NF } else { 0 };
        self.set_sz(self.b);
        if sum > 255 {
            self.f |= HF | CF;
        }
        self.set_flag(PF, ((sum as u8 & 7) ^ self.b).count_ones() % 2 == 0);
    }

    fn adc_hl(&mut self, value: u16) {
        let hl = self.hl();
        let carry = if self.flag(CF) { 1u16 } else { 0 };
        let r = hl.wrapping_add(value).wrapping_add(carry);
        self.f &= !(CF | NF);
        self.set_flag(HF, (hl & 0x0FFF) + (value & 0x0FFF) + carry > 0x0FFF);
        self.set_flag(
            CF,
            u32::from(hl) + u32::from(value) + u32::from(carry) > 65535,
        );
        self.set_flag(PF, ((hl ^ r) & (value ^ r) & 0x8000) != 0);
        self.set_flag(NF, false);
        self.set_flag(SF, r & 0x8000 != 0);
        self.set_flag(ZF, r & 0xFFFF == 0);
        self.f = (self.f & !(XF | YF)) | ((r >> 8) as u8 & (XF | YF));
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
        self.set_flag(CF, u32::from(hl) < u32::from(value) + u32::from(carry));
        self.set_flag(PF, ((hl ^ value) & (hl ^ r) & 0x8000) != 0);
        self.set_flag(SF, r & 0x8000 != 0);
        self.set_flag(ZF, r & 0xFFFF == 0);
        self.f = (self.f & !(XF | YF)) | ((r >> 8) as u8 & (XF | YF));
        self.set_hl(r);
    }
}
