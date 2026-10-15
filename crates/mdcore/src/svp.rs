//! SVP (Samsung SSP1601 DSP) used by Virtua Racing, following the reference's
//! interpreter (GPGX cart_hw/svp, after notaz): 800 SSP instructions are run
//! after each scanline; the 68000 sees DRAM at $300000-$31FFFF, two
//! cell-arranged DRAM views at $390000/$3A0000 and XST/PM0 at $A15000.
//! Registers are 32-bit with the 16-bit view in the upper half (`h`), as in
//! the reference's register union.
use serde::{Deserialize, Serialize};

const GR0: usize = 0;
const X: usize = 1;
const Y: usize = 2;
const A: usize = 3;
const ST: usize = 4;
const STACK: usize = 5;
const PC: usize = 6;
const P: usize = 7;
const PM0: usize = 8;
const PM1: usize = 9;
const PM2: usize = 10;
const XST: usize = 11;
const PM4: usize = 12;
const PMC: usize = 14;
const AL: usize = 15;

const FLAG_L: u16 = 1 << 12;
const FLAG_Z: u16 = 1 << 13;
const FLAG_V: u16 = 1 << 14;
const FLAG_N: u16 = 1 << 15;

const PMC_HAVE_ADDR: u32 = 0x0001;
const PMC_SET: u32 = 0x0002;
const WAIT_PM0: u32 = 0x2000;
const WAIT_30FE06: u32 = 0x4000;
const WAIT_30FE08: u32 = 0x8000;
const WAIT_MASK: u32 = 0xF000;

/// Instructions run per scanline (reference SVP_cycles).
pub const CYCLES_PER_LINE: i32 = 800;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Svp {
    /// Instruction RAM (words 0-0x3FF; program ROM follows in the cartridge).
    iram: Vec<u16>,
    /// 128 KiB DRAM as words.
    pub dram: Vec<u16>,
    /// Two internal 256-word RAM banks.
    ram: Vec<u16>,
    gr: [u32; 16],
    ptr: [u8; 8],
    stack: [u16; 6],
    pmac: [[u32; 6]; 2],
    emu_status: u32,
}

impl Default for Svp {
    fn default() -> Self {
        Self::new()
    }
}

/// Running context: the SVP plus the cartridge ROM and the current PC.
struct Run<'a> {
    s: &'a mut Svp,
    rom: &'a [u8],
    pc: u16,
    cycles: i32,
}

impl Svp {
    pub fn new() -> Self {
        let mut svp = Self {
            iram: vec![0; 0x400],
            dram: vec![0; 0x10000],
            ram: vec![0; 512],
            gr: [0; 16],
            ptr: [0; 8],
            stack: [0; 6],
            pmac: [[0; 6]; 2],
            emu_status: 0,
        };
        svp.reset();
        svp
    }

    /// ssp1601_reset.
    pub fn reset(&mut self) {
        self.emu_status = 0;
        self.gr[GR0] = 0xFFFF_0000;
        self.set_h(PC, 0x400);
        self.set_h(STACK, 0);
        self.set_h(ST, 0);
    }

    pub(crate) fn valid(&self) -> bool {
        self.iram.len() == 0x400 && self.dram.len() == 0x10000 && self.ram.len() == 512
    }

    fn h(&self, r: usize) -> u16 {
        (self.gr[r] >> 16) as u16
    }

    fn set_h(&mut self, r: usize, v: u16) {
        self.gr[r] = (self.gr[r] & 0xFFFF) | (u32::from(v) << 16);
    }

    fn l(&self, r: usize) -> u16 {
        self.gr[r] as u16
    }

    fn set_l(&mut self, r: usize, v: u16) {
        self.gr[r] = (self.gr[r] & 0xFFFF_0000) | u32::from(v);
    }

    // ------------------------------------------------------------ 68000 side

    /// DRAM word at a byte offset (0..0x20000).
    pub fn dram_word(&self, offset: u32) -> u16 {
        self.dram[((offset >> 1) & 0xFFFF) as usize]
    }

    /// 68000 word write into DRAM ($300000-$30FFFF goes through the
    /// reference's handler, which also releases the SSP's tight loops).
    pub fn write_dram_word(&mut self, address: u32, data: u16) {
        self.dram[((address & 0x1FFFE) >> 1) as usize] = data;
        if data != 0 {
            match address & 0xFF_FFFF {
                0x30_FE06 => self.emu_status &= !WAIT_30FE06,
                0x30_FE08 => self.emu_status &= !WAIT_30FE08,
                _ => {}
            }
        }
    }

    pub fn write_dram_byte(&mut self, address: u32, data: u8) {
        let index = ((address & 0x1FFFF) >> 1) as usize;
        let word = self.dram[index];
        self.dram[index] = if address & 1 == 0 {
            (word & 0x00FF) | (u16::from(data) << 8)
        } else {
            (word & 0xFF00) | u16::from(data)
        };
    }

    /// Cell-arranged DRAM views ($390000 and $3A0000 word reads).
    pub fn read_cell(&self, address: u32) -> u16 {
        let offset = if (address >> 16) & 0xFF == 0x39 {
            (address & 0xE002) | ((address & 0x7C) << 6) | ((address & 0x1F80) >> 5)
        } else {
            (address & 0xF002) | ((address & 0x3C) << 6) | ((address & 0xFC0) >> 4)
        };
        self.dram_word(offset)
    }

    /// $A15000-$A150FF read: XST (0-3) or PM0 (4-5, clears its bit 0).
    pub fn read_reg(&mut self, address: u32) -> Option<u16> {
        if address & 0xFC == 0 {
            Some(self.h(XST))
        } else if address & 0xFE == 4 {
            let data = self.h(PM0);
            self.set_h(PM0, data & !1);
            Some(data)
        } else {
            None
        }
    }

    /// $A15000/$A15002 word write: XST, PM0 bit 1, releases the PM0 wait.
    pub fn write_reg(&mut self, address: u32, data: u16) -> bool {
        if address & 0xFD == 0 {
            self.set_h(XST, data);
            let pm0 = self.h(PM0) | 2;
            self.set_h(PM0, pm0);
            self.emu_status &= !WAIT_PM0;
            true
        } else {
            false
        }
    }

    /// ssp1601_run: up to `cycles` instructions, stopping early on a wait.
    pub fn run(&mut self, rom: &[u8], cycles: i32) {
        let pc = self.h(PC);
        let mut run = Run { s: self, rom, pc, cycles };
        run.run();
        let pc = run.pc;
        self.read_p();
        self.set_h(PC, pc);
    }

    fn read_p(&mut self) -> u16 {
        let m1 = i32::from(self.h(X) as i16);
        let m2 = i32::from(self.h(Y) as i16);
        self.gr[P] = (m1.wrapping_mul(m2).wrapping_mul(2)) as u32;
        self.h(P)
    }

    fn a32(&self) -> u32 {
        self.gr[A]
    }
}

impl Run<'_> {
    /// Program word: IRAM below 0x400, cartridge ROM above.
    fn word(&self, index: u16) -> u16 {
        if index < 0x400 {
            self.s.iram[index as usize]
        } else {
            let at = usize::from(index) * 2;
            u16::from_be_bytes([
                self.rom.get(at).copied().unwrap_or(0),
                self.rom.get(at + 1).copied().unwrap_or(0),
            ])
        }
    }

    fn fetch(&mut self) -> u16 {
        let w = self.word(self.pc);
        self.pc = self.pc.wrapping_add(1);
        w
    }

    /// Byte offset of the previous instruction word (GET_PPC_OFFS).
    fn ppc_offs(&self) -> u32 {
        u32::from(self.pc) * 2 - 2
    }

    fn st(&self) -> u16 {
        self.s.h(ST)
    }

    fn set_st(&mut self, v: u16) {
        self.s.set_h(ST, v);
    }

    fn upd_acc_zn(&mut self) {
        let a = self.s.a32();
        let mut st = self.st() & !(FLAG_Z | FLAG_N);
        if a == 0 {
            st |= FLAG_Z;
        } else {
            st |= ((a >> 16) as u16) & FLAG_N;
        }
        self.set_st(st);
    }

    fn upd_lzvn(&mut self) {
        let a = self.s.a32();
        let mut st = self.st() & !(FLAG_L | FLAG_Z | FLAG_V | FLAG_N);
        if a == 0 {
            st |= FLAG_Z;
        } else {
            st |= ((a >> 16) as u16) & FLAG_N;
        }
        self.set_st(st);
    }

    fn cmp_flags(&mut self, t: u32) {
        let mut st = self.st() & !(FLAG_L | FLAG_Z | FLAG_V | FLAG_N);
        if t == 0 {
            st |= FLAG_Z;
        } else {
            st |= ((t >> 16) as u16) & FLAG_N;
        }
        self.set_st(st);
    }

    fn cond(&self, op: u16) -> bool {
        let st = self.st();
        match op & 0xF0 {
            0x00 => true,
            0x50 => (st ^ (op << 5)) & FLAG_Z == 0,
            0x70 => (st ^ (op << 7)) & FLAG_N == 0,
            _ => false,
        }
    }

    // ---------------------------------------------------------- registers

    fn reg_read(&mut self, r: usize) -> u32 {
        if r <= 4 {
            return u32::from(self.s.h(r));
        }
        u32::from(match r {
            STACK => {
                let mut sp = self.s.h(STACK).wrapping_sub(1);
                if (sp as i16) < 0 {
                    sp = 5;
                }
                self.s.set_h(STACK, sp);
                self.s.stack[usize::from(sp) % 6]
            }
            PC => self.pc,
            P => self.s.read_p(),
            PM0 => match self.pm_io(0, false, 0) {
                Some(d) => d,
                None => {
                    let d = self.s.h(PM0);
                    let offs = self.ppc_offs();
                    if d & 2 == 0 && (offs == 0x800 || offs == 0x1851E) {
                        self.s.emu_status |= WAIT_PM0;
                    }
                    self.s.set_h(PM0, d & !2);
                    d
                }
            },
            PM1 => self.pm_io(1, false, 0).unwrap_or(self.s.h(PM1)),
            PM2 => self.pm_io(2, false, 0).unwrap_or(self.s.h(PM2)),
            XST => self.pm_io(3, false, 0).unwrap_or(self.s.h(XST)),
            PM4 => {
                let d = self.pm_io(4, false, 0);
                if d == Some(0) {
                    match self.ppc_offs() {
                        0x0854 => self.s.emu_status |= WAIT_30FE08,
                        0x4F12 => self.s.emu_status |= WAIT_30FE06,
                        _ => {}
                    }
                }
                d.unwrap_or(self.s.h(PM4))
            }
            PMC => {
                if self.s.emu_status & PMC_HAVE_ADDR != 0 {
                    self.s.emu_status |= PMC_SET;
                    self.s.emu_status &= !PMC_HAVE_ADDR;
                    let l = self.s.l(PMC);
                    ((l << 4) & 0xFFF0) | ((l >> 4) & 0xF)
                } else {
                    self.s.emu_status |= PMC_HAVE_ADDR;
                    self.s.l(PMC)
                }
            }
            AL => {
                if self.word(self.pc.wrapping_sub(1)) == 0x000F {
                    self.s.emu_status &= !(PMC_SET | PMC_HAVE_ADDR);
                }
                self.s.l(A)
            }
            _ => 0,
        })
    }

    fn reg_write(&mut self, r: usize, d: u32) {
        let d16 = d as u16;
        match r {
            0 => {}
            1..=3 => self.s.set_h(r, d16),
            ST => self.s.set_h(ST, d16),
            STACK => {
                let mut sp = self.s.h(STACK);
                if sp >= 6 {
                    sp = 0;
                }
                self.s.stack[usize::from(sp)] = d16;
                self.s.set_h(STACK, sp + 1);
            }
            PC => {
                self.pc = d16;
                self.cycles -= 1;
            }
            PM0 => {
                if self.pm_io(0, true, d16).is_none() {
                    self.s.set_h(PM0, d16);
                }
            }
            PM1 => {
                if self.pm_io(1, true, d16).is_none() {
                    self.s.set_h(PM1, d16);
                }
            }
            PM2 => {
                if self.pm_io(2, true, d16).is_none() {
                    self.s.set_h(PM2, d16);
                }
            }
            XST => {
                if self.pm_io(3, true, d16).is_none() {
                    let pm0 = self.s.h(PM0) | 1;
                    self.s.set_h(PM0, pm0);
                    self.s.set_h(XST, d16);
                }
            }
            PM4 => {
                if self.pm_io(4, true, d16).is_none() {
                    self.s.set_h(PM4, d16);
                }
            }
            PMC => {
                if self.s.emu_status & PMC_HAVE_ADDR != 0 {
                    self.s.emu_status |= PMC_SET;
                    self.s.emu_status &= !PMC_HAVE_ADDR;
                    self.s.set_h(PMC, d16);
                } else {
                    self.s.emu_status |= PMC_HAVE_ADDR;
                    self.s.set_l(PMC, d16);
                }
            }
            AL => self.s.set_l(A, d16),
            _ => {}
        }
    }

    /// Programmable memory access (pm_io); `None` = not a PM access.
    fn pm_io(&mut self, reg: usize, write: bool, d: u16) -> Option<u16> {
        let w = usize::from(write);
        if self.s.emu_status & PMC_SET != 0 {
            // Must be a blind read or write.
            let prev = self.word(self.pc.wrapping_sub(1));
            if prev & 0xFF0F != 0 && prev & 0xFFF0 != 0 {
                self.s.emu_status &= !PMC_SET;
                return Some(0);
            }
            self.s.pmac[w][reg] = self.s.gr[PMC];
            self.s.emu_status &= !PMC_SET;
            return Some(0);
        }
        if self.s.emu_status & PMC_HAVE_ADDR != 0 {
            self.s.emu_status &= !PMC_HAVE_ADDR;
        }
        if !(reg == 4 || self.st() & 0x60 != 0) {
            return None;
        }
        let mut result = d;
        if write {
            let addr = (self.s.pmac[1][reg] & 0xFFFF) as usize;
            let mode = self.s.pmac[1][reg] >> 16;
            if mode & 0x43FF == 0x0018 {
                let inc = get_inc(mode);
                overwrite(&mut self.s.dram[addr], d, mode & 0x0400 != 0);
                self.s.pmac[1][reg] = self.s.pmac[1][reg].wrapping_add(inc as u32);
            } else if mode & 0xFBFF == 0x4018 {
                overwrite(&mut self.s.dram[addr], d, mode & 0x0400 != 0);
                let step = if addr & 1 != 0 { 31 } else { 1 };
                self.s.pmac[1][reg] = self.s.pmac[1][reg].wrapping_add(step);
            } else if mode & 0x47FF == 0x001C {
                let inc = get_inc(mode);
                self.s.iram[addr & 0x3FF] = d;
                self.s.pmac[1][reg] = self.s.pmac[1][reg].wrapping_add(inc as u32);
            }
        } else {
            let addr = self.s.pmac[0][reg] & 0xFFFF;
            let mode = self.s.pmac[0][reg] >> 16;
            if mode & 0xFFF0 == 0x0800 {
                self.s.pmac[0][reg] = self.s.pmac[0][reg].wrapping_add(1);
                let at = ((addr | ((mode & 0xF) << 16)) as usize) * 2;
                result = u16::from_be_bytes([
                    self.rom.get(at).copied().unwrap_or(0),
                    self.rom.get(at + 1).copied().unwrap_or(0),
                ]);
            } else if mode & 0x47FF == 0x0018 {
                let inc = get_inc(mode);
                result = self.s.dram[addr as usize];
                self.s.pmac[0][reg] = self.s.pmac[0][reg].wrapping_add(inc as u32);
            } else {
                result = 0;
            }
        }
        // PMC tracks the last PM register accessed.
        self.s.gr[PMC] = self.s.pmac[w][reg];
        Some(result)
    }

    // ------------------------------------------------- pointer registers

    fn ptr1_read_(&mut self, ri: usize, isj2: usize, modi3: usize) -> u16 {
        let t = ri | isj2 | modi3;
        let bank = |t: usize| if t & 4 != 0 { 256 } else { 0 };
        let pi = |t: usize| (t & 4) | (t & 3);
        let st7 = self.st() & 7;
        match t {
            0x00..=0x02 | 0x04..=0x06 => self.s.ram[bank(t) + usize::from(self.s.ptr[pi(t)])],
            0x03 | 0x07 => self.s.ram[bank(t)],
            0x08..=0x0A | 0x0C..=0x0E => {
                let p = pi(t);
                let v = self.s.ram[bank(t) + usize::from(self.s.ptr[p])];
                self.s.ptr[p] = self.s.ptr[p].wrapping_add(1);
                v
            }
            0x0B | 0x0F => self.s.ram[bank(t) + 1],
            0x10..=0x12 | 0x14..=0x16 | 0x18..=0x1A | 0x1C..=0x1E => {
                let p = pi(t);
                let v = self.s.ram[bank(t) + usize::from(self.s.ptr[p])];
                let add: u8 = if t & 0x08 != 0 { 1 } else { 0xFF };
                if st7 == 0 {
                    self.s.ptr[p] = self.s.ptr[p].wrapping_add(add);
                } else {
                    let mask = ((1u32 << st7) - 1) as u8;
                    let r = self.s.ptr[p];
                    self.s.ptr[p] = (r & !mask) | (r.wrapping_add(add) & mask);
                }
                v
            }
            0x13 | 0x17 => self.s.ram[bank(t) + 2],
            0x1B | 0x1F => self.s.ram[bank(t) + 3],
            _ => 0,
        }
    }

    fn ptr1_read(&mut self, op: u16) -> u16 {
        let op = usize::from(op);
        self.ptr1_read_(op & 3, (op >> 6) & 4, (op << 1) & 0x18)
    }

    fn ptr1_write(&mut self, op: u16, d: u16) {
        let op = usize::from(op);
        let t = (op & 3) | ((op >> 6) & 4) | ((op << 1) & 0x18);
        let bank = if t & 4 != 0 { 256 } else { 0 };
        let p = (t & 4) | (t & 3);
        match t {
            0x00..=0x02 | 0x04..=0x06 => self.s.ram[bank + usize::from(self.s.ptr[p])] = d,
            0x03 | 0x07 => self.s.ram[bank] = d,
            0x08..=0x0A | 0x18..=0x1A | 0x0C..=0x0E | 0x1C..=0x1E => {
                self.s.ram[bank + usize::from(self.s.ptr[p])] = d;
                self.s.ptr[p] = self.s.ptr[p].wrapping_add(1);
            }
            0x0B | 0x0F => self.s.ram[bank + 1] = d,
            0x10..=0x12 | 0x14..=0x16 => {
                self.s.ram[bank + usize::from(self.s.ptr[p])] = d;
                self.s.ptr[p] = self.s.ptr[p].wrapping_sub(1);
            }
            0x13 | 0x17 => self.s.ram[bank + 2] = d,
            0x1B | 0x1F => self.s.ram[bank + 3] = d,
            _ => {}
        }
    }

    fn ptr2_read(&mut self, op: u16) -> u16 {
        let op = usize::from(op);
        let t = (op & 3) | ((op >> 6) & 4) | ((op << 1) & 0x18);
        let bank = if t & 4 != 0 { 256 } else { 0 };
        let p = (t & 4) | (t & 3);
        let index = match t {
            0x00..=0x02 | 0x04..=0x06 => bank + usize::from(self.s.ptr[p]),
            0x03 | 0x07 => bank,
            0x0B | 0x0F => bank + 1,
            0x13 | 0x17 => bank + 2,
            0x1B | 0x1F => bank + 3,
            _ => return 0,
        };
        let mv = self.s.ram[index];
        self.s.ram[index] = mv.wrapping_add(1);
        self.word(mv)
    }

    // ---------------------------------------------------------- execution

    fn alu(&mut self, kind: u16, x: u32, wide: bool) {
        let v = if wide { x } else { x << 16 };
        let a = self.s.a32();
        match kind {
            1 => {
                self.s.gr[A] = a.wrapping_sub(v);
                self.upd_lzvn();
            }
            3 => self.cmp_flags(a.wrapping_sub(v)),
            4 => {
                self.s.gr[A] = a.wrapping_add(v);
                self.upd_lzvn();
            }
            5 => {
                self.s.gr[A] = a & v;
                self.upd_acc_zn();
            }
            6 => {
                self.s.gr[A] = a | v;
                self.upd_acc_zn();
            }
            7 => {
                self.s.gr[A] = a ^ v;
                self.upd_acc_zn();
            }
            _ => {}
        }
    }

    fn ij(&self, op: u16) -> u32 {
        u32::from(self.s.ptr[usize::from(((op >> 6) & 4) | (op & 3))])
    }

    fn run(&mut self) {
        loop {
            let op = self.fetch();
            match op >> 9 {
                0x00 => {
                    if op != 0 {
                        if op == ((A as u16) << 4) | P as u16 {
                            self.s.read_p();
                            self.s.gr[A] = self.s.gr[P];
                        } else {
                            let v = self.reg_read(usize::from(op & 0x0F));
                            self.reg_write(usize::from((op & 0xF0) >> 4), v);
                        }
                    }
                }
                0x01 => {
                    let v = self.ptr1_read(op);
                    self.reg_write(usize::from((op & 0xF0) >> 4), u32::from(v));
                }
                0x02 => {
                    let v = self.reg_read(usize::from((op & 0xF0) >> 4));
                    self.ptr1_write(op, v as u16);
                }
                0x04 => {
                    let v = self.fetch();
                    self.reg_write(usize::from((op & 0xF0) >> 4), u32::from(v));
                }
                0x05 => {
                    let v = self.ptr2_read(op);
                    self.reg_write(usize::from((op & 0xF0) >> 4), u32::from(v));
                }
                0x06 => {
                    let v = self.fetch();
                    self.ptr1_write(op, v);
                }
                0x07 => self.s.ram[usize::from(op & 0x1FF)] = self.s.h(A),
                0x09 => {
                    let v = self.ij(op);
                    self.reg_write(usize::from((op & 0xF0) >> 4), v);
                }
                0x0A => {
                    let v = self.reg_read(usize::from((op & 0xF0) >> 4));
                    self.s.ptr[usize::from(((op >> 6) & 4) | (op & 3))] = v as u8;
                }
                0x0C..=0x0F => self.s.ptr[usize::from((op >> 8) & 7)] = op as u8,
                0x24 => {
                    if self.cond(op) {
                        let target = self.fetch();
                        let ret = u32::from(self.pc);
                        self.reg_write(STACK, ret);
                        self.reg_write(PC, u32::from(target));
                    } else {
                        self.pc = self.pc.wrapping_add(1);
                    }
                }
                0x25 => {
                    let v = self.word(self.s.h(A));
                    self.reg_write(usize::from((op & 0xF0) >> 4), u32::from(v));
                }
                0x26 => {
                    if self.cond(op) {
                        let target = self.fetch();
                        self.reg_write(PC, u32::from(target));
                    } else {
                        self.pc = self.pc.wrapping_add(1);
                    }
                }
                0x48 => {
                    if self.cond(op) {
                        let a = self.s.gr[A];
                        self.s.gr[A] = match op & 7 {
                            2 => ((a as i32) >> 1) as u32,
                            3 => a << 1,
                            6 => (a as i32).wrapping_neg() as u32,
                            7 if (a as i32) < 0 => (a as i32).wrapping_neg() as u32,
                            _ => a,
                        };
                        self.upd_acc_zn();
                    }
                }
                0x1B | 0x4B | 0x5B => {
                    match op >> 9 {
                        0x1B => {
                            self.s.read_p();
                            self.s.gr[A] = self.s.gr[A].wrapping_sub(self.s.gr[P]);
                            self.upd_acc_zn();
                        }
                        0x4B => {
                            self.s.read_p();
                            self.s.gr[A] = self.s.gr[A].wrapping_add(self.s.gr[P]);
                            self.upd_acc_zn();
                        }
                        _ => {
                            self.s.gr[A] = 0;
                            let st = self.st() & 0x0FFF;
                            self.set_st(st);
                        }
                    }
                    let o = usize::from(op);
                    let x = self.ptr1_read_(o & 3, 0, (o << 1) & 0x18);
                    self.s.set_h(X, x);
                    let y = self.ptr1_read_((o >> 4) & 3, 4, (o >> 3) & 0x18);
                    self.s.set_h(Y, y);
                }
                0x10 | 0x30 | 0x40 | 0x50 | 0x60 | 0x70 => {
                    let kind = op >> 13;
                    let src = usize::from(op & 0x0F);
                    if src == P {
                        self.s.read_p();
                        self.alu(kind, self.s.gr[P], true);
                    } else if src == A {
                        self.alu(kind, self.s.gr[A], true);
                    } else {
                        let v = self.reg_read(src);
                        self.alu(kind, v, false);
                    }
                }
                0x11 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 => {
                    let v = self.ptr1_read(op);
                    self.alu(op >> 13, u32::from(v), false);
                }
                0x03 => {
                    let v = self.s.ram[usize::from(op & 0x1FF)];
                    self.s.set_h(A, v);
                }
                0x13 | 0x33 | 0x43 | 0x53 | 0x63 | 0x73 => {
                    let v = self.s.ram[usize::from(op & 0x1FF)];
                    self.alu(op >> 13, u32::from(v), false);
                }
                0x14 | 0x34 | 0x44 | 0x54 | 0x64 | 0x74 => {
                    let v = self.fetch();
                    self.alu(op >> 13, u32::from(v), false);
                }
                0x15 | 0x35 | 0x45 | 0x55 | 0x65 | 0x75 => {
                    let v = self.ptr2_read(op);
                    self.alu(op >> 13, u32::from(v), false);
                }
                0x19 | 0x39 | 0x49 | 0x59 | 0x69 | 0x79 => {
                    let v = self.ij(op);
                    self.alu(op >> 13, v, false);
                }
                0x1C | 0x3C | 0x4C | 0x5C | 0x6C | 0x7C => {
                    self.alu(op >> 13, u32::from(op & 0xFF), false);
                }
                _ => {}
            }
            self.cycles -= 1;
            if self.cycles <= 0 || self.s.emu_status & WAIT_MASK != 0 {
                break;
            }
        }
    }
}

/// Address increment for a PM mode (get_inc): 0 1 2 4 8 16 32 128.
fn get_inc(mode: u32) -> i32 {
    let mut inc = ((mode >> 11) & 7) as i32;
    if inc != 0 {
        if inc != 7 {
            inc -= 1;
        }
        inc = 1 << inc;
        if mode & 0x8000 != 0 {
            inc = -inc;
        }
    }
    inc
}

/// Plain or nibble-overwrite (non-zero nibbles only) DRAM write.
fn overwrite(dst: &mut u16, d: u16, nibbles: bool) {
    if !nibbles {
        *dst = d;
        return;
    }
    for mask in [0xF000u16, 0x0F00, 0x00F0, 0x000F] {
        if d & mask != 0 {
            *dst = (*dst & !mask) | (d & mask);
        }
    }
}
