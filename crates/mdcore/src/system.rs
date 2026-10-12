//! The Mega Drive as a whole: cartridge, work RAM, VDP and I/O behind one bus.

use crate::cartridge::Cartridge;
use crate::cpu::bus::Bus;
use crate::cpu::m68000::{Exception, M68000};
use crate::fm::Fm;
use crate::memmap::{fold_to_cart_mirrored, fold_to_wram, region, Region};
use crate::vdp::Vdp;
use crate::z80::{Z80Bus, Z80};

/// The Z80's RAM is 8 KiB, mirrored four times to fill its 16 KiB window.
/// Modelled at 64 KiB this is invisible for single-byte accesses, but a word
/// access at the top of the mirror reads past the buffer instead of wrapping,
/// and the sound driver does exactly that when it copies its own tables.
pub const Z80_RAM_SIZE: usize = 0x2000;

/// 64 KiB of 68K work RAM, mirrored through the top of the map.
pub const WRAM_SIZE: usize = 0x1_0000;

/// Console I/O registers the 68000 can reach.
#[derive(Default)]
pub struct Io {
    /// Pressed-button masks for the two three-button controllers.
    pub pad_buttons: [u8; 2],
    pub data_a: u16,
    pub data_b: u16,
    pub data_c: u16,
    pub data_d: u16,
    pub ctrl_a: u16,
    pub ctrl_b: u16,
    pub ctrl_c: u16,
    pub ctrl_d: u16,
    /// True while the 68000 owns the Z80 bus, which stops the Z80.
    pub z80_bus_request: bool,
    /// True while the reset line is asserted. Active low on the wire.
    pub z80_reset: bool,
    /// The Z80 bank register: a 23-bit shift register whose output supplies the
    /// top bits of the 68K address visible at 0x8000-0xFFFF.
    pub zbank: u32,
    pub version_register: u8,
    pub serial_counter: u8,
}

impl Io {
    fn new() -> Self {
        Self {
            // Region code 0xA0: no expansion unit attached.
            version_register: 0xA0,
            ..Default::default()
        }
    }
}

/// A work-RAM write that carried one of the cartridge's own debug markers,
/// stamped with the instruction that made it.
#[derive(Debug, Clone, Copy)]
pub struct MarkerWrite {
    pub addr: u32,
    pub value: u16,
    pub pc: u32,
    /// The registers that formed the address. Four separate theories about
    /// this write have come from inferring them rather than reading them at
    /// the instruction, and every one was wrong.
    pub a6: u32,
    pub d4: u32,
    pub d1: u32,
    pub d2: u32,
}

pub struct System {
    pub cart: Cartridge,
    pub wram: Vec<u8>,
    pub vdp: Vdp,
    pub io: Io,
    pub cpu: M68000,
    /// Number of VDP accesses that hit an unmapped address, for coverage.
    pub unmapped_accesses: u64,
    pub unmapped_samples: Vec<crate::observe::UnmappedAccess>,
    /// Every write to the I/O area, in order, as (address, value).
    pub io_log: Vec<(u32, u8)>,
    /// Optional diagnostic retention cap. It never changes device behavior.
    pub observation_limit: usize,
    pub io_write_count: u64,
    pub z80_upload_count: u64,
    pub vdp_reg_write_count: u64,
    pub z80_sound_write_count: u64,
    pub wram_watch: Option<crate::observe::WramWatch>,
    wram_long_write: bool,
    cpu_bus_active: bool,
    z80_bus_active: bool,
    interrupt_bus_active: bool,
    stamp_full_registers: ([u32; 8], [u32; 8], u16),
    /// 64 KiB of Z80 address space as the 68000 sees it. The machine only
    /// uses the first 8 KiB, mirrored four times.
    pub z80_ram: Vec<u8>,
    /// The sound CPU.
    pub z80: Z80,
    /// Program counter of the sound CPU while it is mid-step. The Z80 is moved
    /// out of the struct for the duration of a step so the bus can be borrowed
    /// alongside it, which means `self.z80` is an empty placeholder exactly
    /// when a bus access needs to say who caused it.
    pub z80_active_pc: u16,
    /// Program counter of the 68000 while it is mid-step, for the same reason.
    pub cpu_active_pc: u32,
    /// The first writes to the VDP register file, with the PC that made each.
    pub vdp_reg_writes: Vec<(u8, u8, u32)>,
    /// Data-port writes per destination, indexed by the CD code: 0 VRAM,
    /// 1 CRAM, 2 VSRAM. A cartridge that fills VRAM beautifully and never
    /// touches the palette is one count here, not a search through the code.
    pub vdp_data_counts: [u64; 3],
    /// Off by default; the tests that chase the fill sweep switch it on.
    pub trace_marker_writes: bool,
    pub marker_writes: Vec<MarkerWrite>,
    /// The CPU's PC, stamped in before a step so the bus can attribute a
    /// write. The Z80 moves out of System for the duration of its own step,
    /// so the struct's own field is not what is running when this is read.
    pub stamp_pc: u32,
    /// a6, d4, d1, d2 at the same moment, for the same reason.
    pub stamp_regs: [u32; 4],
    /// (writing pc, offset within work RAM, byte) for the first four bytes.
    pub wram_zero_writes: Vec<(u32, u32, u16)>,
    pub trace_wram_zero: bool,
    /// The first data-port writes, with the PC that made each, so the
    /// cartridge can be caught in the act rather than inferred from output.
    pub vdp_data_writes: Vec<(u8, u16, u16, u32)>,
    /// The first Z80 I/O port writes, with the PC. The PSG lives here on
    /// 0xFF40/0xFF41, so a driver that never touches the memory window is
    /// still producing sound and a run that only watches 0x4000-0x5FFF will
    /// conclude the machine is silent.
    pub z80_port_writes: Vec<(u16, u8, u16)>,
    /// Writes per Z80 I/O port, low byte.
    pub z80_port_counts: Vec<u32>,
    /// The first writes into the sound window, with the PC that made each.
    /// Without the PC a port sequence is uninterpretable: it looks identical
    /// whether the driver is programming the FM chip or just using the window
    /// as scratch.
    pub z80_sound_writes: Vec<(u16, u8, u16)>,
    /// Every write the 68000 makes into that space, in order.
    pub z80_upload: Vec<u32>,
    /// The FM sound chip on the sound window.
    pub fm: Fm,
    /// External DMA waits queued by a control-port write during the current
    /// instruction. Account for device time before accepting the next IRQ.
    pending_dma_master_cycles: u32,
}

impl System {
    pub fn new(cart: Cartridge) -> Self {
        let mut system = Self {
            cart,
            wram: vec![0; WRAM_SIZE],
            vdp: Vdp::new(),
            io: Io::new(),
            cpu: M68000::new(0),
            unmapped_accesses: 0,
            unmapped_samples: Vec::new(),
            io_log: Vec::new(),
            observation_limit: usize::MAX,
            io_write_count: 0,
            z80_upload_count: 0,
            vdp_reg_write_count: 0,
            z80_sound_write_count: 0,
            wram_watch: None,
            wram_long_write: false,
            cpu_bus_active: false,
            z80_bus_active: false,
            interrupt_bus_active: false,
            stamp_full_registers: ([0; 8], [0; 8], 0),
            z80_ram: vec![0; Z80_RAM_SIZE],
            z80_upload: Vec::new(),
            z80: Z80::new(),
            z80_active_pc: 0,
            cpu_active_pc: 0,
            vdp_reg_writes: Vec::new(),
            vdp_data_counts: [0; 3],
            trace_marker_writes: false,
            marker_writes: Vec::new(),
            stamp_pc: 0,
            stamp_regs: [0; 4],
            wram_zero_writes: Vec::new(),
            trace_wram_zero: false,
            vdp_data_writes: Vec::new(),
            z80_port_writes: Vec::new(),
            z80_port_counts: vec![0; 256],
            z80_sound_writes: Vec::new(),
            fm: Fm::new(),
            pending_dma_master_cycles: 0,
        };
        system.reset();
        system
    }

    /// Reset the console from the cartridge's exception vector table.
    ///
    /// The first long is the initial stack pointer, the second the entry point.
    /// The 68000 only drives A1-A23, so the entry point is taken modulo 2^24:
    /// that fold is what decides where a cart with an unusual vector actually
    /// starts, and it is worth stating rather than hiding.
    pub fn reset(&mut self) {
        self.wram.fill(0);
        self.vdp.reset();
        self.pending_dma_master_cycles = 0;
        self.io = Io::new();
        let (sp, pc) = self.reset_vectors();
        self.cpu = M68000::new(pc);
        self.cpu.a[7] = sp;
        self.cpu.usp = sp;
        let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
        cpu.prime(self);
        self.cpu = cpu;
    }

    fn cart_long(&self, offset: usize) -> u32 {
        let len = self.cart.data.len();
        let b = |i: usize| self.cart.data[(offset + i) & (len - 1)] as u32;
        (b(0) << 24) | (b(1) << 16) | (b(2) << 8) | b(3)
    }

    /// The raw 32-bit long stored at `offset` in the cartridge.
    pub fn cart_long_public(&self, offset: usize) -> u32 {
        self.cart_long(offset)
    }

    /// The exception vectors as the hardware would use them: the stack pointer
    /// and entry point are masked to the 24-bit bus.
    pub fn reset_vectors(&self) -> (u32, u32) {
        (self.cart_long(0), self.cart_long(4) & 0x00FF_FFFF)
    }

    /// Execute one instruction, returning any exception that was raised.
    ///
    /// The processor is moved aside for the call so that it can borrow the same
    /// `System` as its bus.
    pub fn step(&mut self) -> Option<Exception> {
        self.step_instruction().1
    }

    /// Advance one instruction and return its cost in 68000 cycles together
    /// with any exception it raised.
    pub fn step_instruction(&mut self) -> (u32, Option<Exception>) {
        self.stamp_pc = self.cpu.pc;
        self.stamp_full_registers = (self.cpu.d, self.cpu.a, self.cpu.sr.0);
        self.cpu_bus_active = true;
        self.interrupt_bus_active = false;
        self.stamp_regs = [self.cpu.a[6], self.cpu.d[4], self.cpu.d[1], self.cpu.d[2]];
        // Interrupts are only accepted at instruction boundaries and only when
        // the interrupt mask allows this level. The VDP drives level 6 for
        // vertical blank and level 4 for horizontal blank.
        let level = self.irq_level().filter(|l| self.cpu.sr.accepts(*l));
        if let Some(level) = level {
            self.interrupt_bus_active = true;
            if level == 6 {
                self.vdp.vint_taken = self.vdp.vint_taken.saturating_add(1);
                self.vdp.vint_pending = false;
            } else {
                self.vdp.hint_pending = false;
            }
            let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
            cpu.interrupt(self, level);
            self.cpu = cpu;
            self.cpu_bus_active = false;
            self.interrupt_bus_active = false;
            self.vdp.advance_cpu_cycles(44);
            return (44, None);
        }
        let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
        self.cpu_active_pc = cpu.pc;
        let outcome = cpu.step(self);
        self.cpu = cpu;
        self.cpu_bus_active = false;
        // The sound CPU is clocked at half the 68000 rate, so each 68000 cycle
        // buys it two T-states. Running it here rather than on its own schedule
        // keeps the two processors interleaved the way the hardware does, which
        // matters because the 68000 stops the Z80 whenever it takes the bus to
        // upload a driver.
        self.step_z80(outcome.cycles * 2);
        self.vdp.advance_cpu_cycles(outcome.cycles);
        let dma_wait = std::mem::take(&mut self.pending_dma_master_cycles);
        self.vdp.advance_master_cycles(dma_wait);
        self.vdp.dma_bus_stall_master_cycles += u64::from(dma_wait);
        (outcome.cycles + dma_wait.div_ceil(7), outcome.exception)
    }

    /// The autovector level currently asserted by the VDP, if any.
    pub fn irq_level(&self) -> Option<u8> {
        if self.vdp.vint_pending && self.vdp.registers[1] & 0x20 != 0 {
            Some(6)
        } else if self.vdp.hint_pending && self.vdp.registers[0] & 0x10 != 0 {
            Some(4)
        } else {
            None
        }
    }

    // ------------------------------------------------------------- VDP port

    fn vdp_control_write(&mut self, value: u16) {
        if self.vdp.control_pending {
            self.vdp.address_latch =
                (self.vdp.address_latch & 0x3FFF) | (((value & 3) as u32) << 14);
            self.vdp.code = (self.vdp.code & 3) | (((value >> 2) & 0x3C) as u8);
            self.vdp.control_pending = false;
            if self.vdp.code & 0x20 != 0 && self.vdp.registers[1] & 0x10 != 0 {
                self.vdp.dma_requests += 1;
                self.execute_vdp_dma();
            }
            return;
        }
        if value & 0xC000 == 0x8000 {
            self.vdp_reg_write_count += 1;
        }
        if value & 0xC000 == 0x8000 && self.vdp_reg_writes.len() < 48 {
            let index = (value >> 8) & 0x1F;
            self.vdp_reg_writes
                .push((index as u8, (value & 0xFF) as u8, self.cpu_active_pc));
        }
        // A control word with bits 15-13 == 100 is a register write: bits 13-8
        // choose the register and bits 7-0 carry the value. Bits 13-8 also
        // latch, so a data-port write straight afterwards keeps the address.
        if value & 0xC000 == 0x8000 {
            let index = ((value >> 8) & 0x1F) as usize;
            self.vdp.control_write(index, (value & 0xFF) as u8);
            return;
        }

        // Otherwise bit 14 is CD1 and bit 13 is CD2, which select VRAM, the
        // register file, CRAM or VSRAM; bits 13-0 are the address. The same
        // bits become the code register for data-port writes.
        // Numeric code is (CD2 << 1) | CD1, i.e. (bit 13 << 1) | bit 14.
        self.vdp.code = (self.vdp.code & 0x3C) | ((value >> 14) as u8 & 3);
        self.vdp.address_latch = (self.vdp.address_latch & 0xC000) | (value as u32 & 0x3FFF);
        self.vdp.control_pending = true;
    }

    /// Apply DMA data and queue the external-bus wait. Fill/copy timing and
    /// sub-instruction FIFO contention remain approximate.
    fn execute_vdp_dma(&mut self) {
        let raw = u16::from_be_bytes([self.vdp.registers[20], self.vdp.registers[19]]) as u32;
        let count = if raw == 0 { 0x1_0000 } else { raw };
        let mut source =
            u16::from_be_bytes([self.vdp.registers[22], self.vdp.registers[21]]) as u32;
        match self.vdp.registers[23] >> 6 {
            2 => {
                self.vdp.pending_dm = true;
                return;
            }
            3 => {
                if self.vdp.code & 0x10 == 0 {
                    self.vdp.dma_unsupported = true;
                    return;
                }
                self.vdp.start_internal_dma(count, true);
                for _ in 0..count {
                    let value = self.vdp.vram[(source as usize ^ 1) & 0xFFFF];
                    let destination = (self.vdp.address_latch as usize ^ 1) & 0xFFFF;
                    self.vdp.vram[destination] = value;
                    source = source.wrapping_add(1) & 0xFFFF;
                    self.vdp.address_latch = self
                        .vdp
                        .address_latch
                        .wrapping_add(self.vdp.registers[15] as u32)
                        & 0xFFFF;
                }
                self.vdp.dma_bytes += count as u64;
            }
            _ => {
                self.pending_dma_master_cycles += self
                    .vdp
                    .external_dma_master_cycles(count, self.vdp.code & 0xF == 1);
                let bank = (self.vdp.registers[23] as u32 & 0x7F) << 17;
                for _ in 0..count {
                    let address = bank | (source << 1);
                    let data = self.read16(address);
                    self.vdp.data_write(data);
                    source = source.wrapping_add(1) & 0xFFFF;
                }
                self.vdp.dma_bytes += count as u64 * 2;
            }
        }
        self.vdp.registers[19] = 0;
        self.vdp.registers[20] = 0;
        self.vdp.registers[21] = source as u8;
        self.vdp.registers[22] = (source >> 8) as u8;
    }

    fn observe_unmapped(&mut self, address: u32, width_bits: u8, write: bool) {
        if self.unmapped_samples.len() >= self.observation_limit {
            return;
        }
        let pc = if self.cpu_bus_active {
            Some(self.stamp_pc)
        } else if self.z80_bus_active {
            Some(u32::from(self.z80_active_pc))
        } else {
            None
        };
        let writer = if self.interrupt_bus_active {
            "interrupt_entry"
        } else if self.cpu_bus_active {
            "m68000"
        } else if self.z80_bus_active {
            "z80"
        } else {
            "unattributed_bus"
        };
        self.unmapped_samples.push(crate::observe::UnmappedAccess {
            address,
            width_bits,
            write,
            pc,
            writer: writer.into(),
        });
    }

    fn observe_wram_write(&mut self, address: u32, value: u32, width_bits: u8) {
        if self.wram_long_write {
            return;
        }
        let pc = if self.cpu_bus_active {
            Some(self.stamp_pc)
        } else if self.z80_bus_active {
            Some(u32::from(self.z80_active_pc))
        } else {
            None
        };
        let Some(watch) = self
            .wram_watch
            .as_mut()
            .filter(|w| w.matches(address, width_bits, pc))
        else {
            return;
        };
        watch.observed += 1;
        if watch.records.len() >= watch.max_records {
            return;
        }
        let offset = fold_to_wram(address) as usize;
        let mut before = 0;
        for i in 0..usize::from(width_bits) / 8 {
            before = (before << 8) | u32::from(self.wram[(offset + i) & 65535]);
        }
        watch.records.push(crate::observe::WramWrite {
            pc,
            address,
            folded_offset: offset,
            width_bits,
            before,
            value,
            frame: self.vdp.frame,
            writer: if self.interrupt_bus_active {
                "interrupt_entry"
            } else if self.cpu_bus_active {
                "m68000"
            } else if self.z80_bus_active {
                "z80"
            } else {
                "unattributed_bus"
            }
            .into(),
            d: self.cpu_bus_active.then_some(self.stamp_full_registers.0),
            a: self.cpu_bus_active.then_some(self.stamp_full_registers.1),
            sr: self.cpu_bus_active.then_some(self.stamp_full_registers.2),
        });
    }
}

/// Map a CPU exception to its vector number.
pub fn exception_vector(e: &Exception) -> u8 {
    match e {
        Exception::Illegal { .. } => 4,
        Exception::DivideByZero => 5,
        Exception::Vector { vector } => *vector,
        Exception::AddressError { .. } => 3,
        Exception::Privileged => 8,
    }
}

impl Bus for System {
    fn read8(&mut self, addr: u32) -> u8 {
        let a = addr & 0x00FF_FFFF;
        match region(a) {
            Region::Cartridge => {
                let i = fold_to_cart_mirrored(a, self.cart.data.len()) as usize;
                self.cart.data[i % self.cart.data.len()]
            }
            Region::Wram => self.wram[fold_to_wram(a) as usize],
            // Byte access to the Z80's RAM matters: cartridges upload the
            // sound driver with byte writes and verify each byte by reading it
            // straight back. Without this the read fell through to the
            // catch-all, returned 0, and the cartridge's verify loop retried
            // the same address forever.
            Region::Z80Ram => self.z80_ram[(a as usize) & (Z80_RAM_SIZE - 1)],
            Region::Vdp => {
                // The VDP is word-oriented; byte reads mirror the port.
                match a & 0x1E {
                    0x04 => {
                        let counter = 0u16;
                        let byte = if a & 1 == 1 {
                            (counter >> 8) as u8
                        } else {
                            counter as u8
                        };
                        byte
                    }
                    _ => {
                        let word = self.vdp.data_read();
                        if a & 1 == 1 {
                            (word >> 8) as u8
                        } else {
                            word as u8
                        }
                    }
                }
            }
            Region::Io => self.io_read(a),
            Region::Sram => 0xFF,
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 8, false);
                0
            }
        }
    }

    fn write8(&mut self, addr: u32, value: u8) {
        let a = addr & 0x00FF_FFFF;
        match region(a) {
            Region::Cartridge => {}
            Region::Wram => {
                self.observe_wram_write(addr, u32::from(value), 8);
                // Also watch the byte width. The marker reaches work RAM
                // offset zero by a path that is not a longword store: write32
                // composes write16, so a word-width watch that records only
                // the boot RAM clear proves nothing about it.
                if self.trace_wram_zero {
                    let off = fold_to_wram(a);
                    if off < 4 && self.wram_zero_writes.len() < 32 {
                        self.wram_zero_writes.push((self.stamp_pc, a, value as u16));
                    }
                }
                if self.trace_marker_writes
                    && (value == 0x22 || value == 0xBB)
                    && (0x00FF_9C00..0x00FF_9D00).contains(&(a & 0x00FF_FFFF))
                {
                    if self.marker_writes.len() < 4096 {
                        self.marker_writes.push(MarkerWrite {
                            addr: a,
                            value: value as u16,
                            pc: self.stamp_pc,
                            a6: self.stamp_regs[0],
                            d4: self.stamp_regs[1],
                            d1: self.stamp_regs[2],
                            d2: self.stamp_regs[3],
                        });
                    }
                }
                self.wram[fold_to_wram(a) as usize] = value;
            }
            Region::Vdp => {
                let existing = self.read16(a & !1);
                let word = if a & 1 == 1 {
                    (existing & 0x00FF) | ((value as u16) << 8)
                } else {
                    (existing & 0xFF00) | value as u16
                };
                self.write16(a & !1, word);
            }
            Region::Io => self.io_write(a, value),
            // The sound handshake lives in its own decode region, not in the
            // general I/O block, so handling it inside io_write_byte is dead
            // code: nothing routes it there.
            Region::Z80Ctrl => self.z80_ctrl_write(a, value),
            Region::Z80Ram => {
                let o = (a as usize) & (Z80_RAM_SIZE - 1);
                self.z80_upload_count += 1;
                if self.z80_upload.len() < self.observation_limit {
                    self.z80_upload.push(o as u32);
                }
                self.z80_ram[o] = value;
            }
            Region::Sram => {}
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 8, true);
            }
        }
    }

    fn read16(&mut self, addr: u32) -> u16 {
        let a = addr & 0x00FF_FFFF;
        match region(a) {
            Region::Cartridge => {
                let base = fold_to_cart_mirrored(a, self.cart.data.len()) as usize;
                let len = self.cart.data.len();
                ((self.cart.data[base % len] as u16) << 8) | self.cart.data[(base + 1) % len] as u16
            }
            Region::Wram => {
                let o = fold_to_wram(a) as usize;
                ((self.wram[o] as u16) << 8) | self.wram[o + 1] as u16
            }
            Region::Z80Ram => {
                let o = (a as usize) & (Z80_RAM_SIZE - 1);
                let hi = (o + 1) & (Z80_RAM_SIZE - 1);
                ((self.z80_ram[o] as u16) << 8) | self.z80_ram[hi] as u16
            }
            Region::Fm => ((self.fm.read(1) as u16) << 8) | self.fm.read(0) as u16,
            Region::Vdp => match a & 0x1E {
                // The status register is on the control port; a read of the
                // data port returns whatever the last write selected.
                0x04 | 0x06 => self.vdp_status(),
                0x00 | 0x02 => self.vdp.data_read(),
                _ => 0,
            },
            Region::Io => {
                let data = self.io_read(a) as u16;
                (data << 8) | data
            }
            Region::Sram => 0xFFFF,
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 16, false);
                0
            }
        }
    }

    fn write16(&mut self, addr: u32, value: u16) {
        let a = addr & 0x00FF_FFFF;
        match region(a) {
            Region::Cartridge => {}
            Region::Wram => {
                self.observe_wram_write(addr, u32::from(value), 16);
                // The sweep that stamps $BBBBBBBB over work RAM offset zero is
                // a move.l, and write32 composes two write16 calls, so this is
                // the only width that has to be watched. A byte-width watch
                // sees nothing at all here and looks like "nothing wrote it".
                // Work RAM is selected by A23-A19, so $FFFF0000 masks to
                // 0x00FF0000 and offset zero is at THAT address - testing
                // `(a & 0xFFFFFF) < 4` watches the wrong end of the map and
                // reports "nothing wrote it" for a word that is plainly
                // $BBBBBBBB. Same trap as the select-line bisection.
                // Test the FOLDED offset, not the address. fold_to_wram masks
                // with 0x003FFFFF, so $00FF0000, $00BF0000 and $00FFFFFE all
                // land on offset zero. Watching the address misses every alias
                // but one and reports "nothing wrote it" for a word that is
                // plainly $BBBBBBBB.
                let off = fold_to_wram(a);
                if self.trace_wram_zero && off < 4 && self.wram_zero_writes.len() < 32 {
                    self.wram_zero_writes.push((self.stamp_pc, a, value));
                }
                let o = fold_to_wram(a) as usize;
                // Work-RAM write provenance: the CPU's own PC is not available
                // inside the bus, so the caller stamps it in before a step.
                // Only the two marker values are recorded, which keeps this
                // free in the hot loop.
                if self.trace_marker_writes
                    && (value == 0x2222 || value == 0xBBBB)
                    && (0x00FF_9C00..0x00FF_9D00).contains(&(a & 0x00FF_FFFF))
                {
                    if self.marker_writes.len() < 4096 {
                        self.marker_writes.push(MarkerWrite {
                            addr: a,
                            value,
                            pc: self.stamp_pc,
                            a6: self.stamp_regs[0],
                            d4: self.stamp_regs[1],
                            d1: self.stamp_regs[2],
                            d2: self.stamp_regs[3],
                        });
                    }
                }
                self.wram[o] = (value >> 8) as u8;
                self.wram[o + 1] = (value & 0xFF) as u8;
            }
            Region::Z80Ram => {
                let o = (a as usize) & (Z80_RAM_SIZE - 1);
                self.z80_upload_count += 1;
                if self.z80_upload.len() < self.observation_limit {
                    self.z80_upload.push(o as u32);
                }
                let hi = (o + 1) & (Z80_RAM_SIZE - 1);
                self.z80_ram[o] = (value >> 8) as u8;
                self.z80_ram[hi] = (value & 0xFF) as u8;
            }
            Region::Fm => self.fm.write((a & 3) as usize, value as u8),
            Region::Vdp => match a & 0x1E {
                // 0xC00000/2 are the data port; 0xC00004/6 are the control
                // port, because a long write lands its high word there.
                0x00 | 0x02 => {
                    let target = match self.vdp.code & 0xF {
                        1 => 0,
                        3 => 1,
                        5 => 2,
                        _ => 3,
                    };
                    if target < 3 {
                        self.vdp_data_counts[target] += 1;
                        if self.vdp_data_writes.len() < 32 {
                            let addr = (self.vdp.address_latch as u16) & !1;
                            self.vdp_data_writes.push((
                                target as u8,
                                addr,
                                value,
                                self.cpu_active_pc,
                            ));
                        }
                    }
                    self.vdp.data_write(value);
                }
                _ => self.vdp_control_write(value),
            },
            Region::Io => {
                self.io_write(a, (value >> 8) as u8);
                self.io_write(a + 1, (value & 0xFF) as u8);
            }
            // A driver writes the handshake as a word - `move.w #$0100,
            // $A11100` - so this path has to split it into the two bytes the
            // hardware decodes separately, high byte first. Routing the word
            // to the unmapped arm is silent: nothing faults, the Z80 simply
            // never learns it has the bus and waits forever.
            Region::Z80Ctrl => {
                self.z80_ctrl_write(a, (value >> 8) as u8);
                self.z80_ctrl_write(a + 1, (value & 0xFF) as u8);
            }
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 16, true);
            }
        }
    }

    fn write32(&mut self, addr: u32, value: u32) {
        let watched = region(addr & 0xFF_FFFF) == Region::Wram;
        if watched {
            self.observe_wram_write(addr, value, 32);
            self.wram_long_write = true;
        }
        self.write16(addr, (value >> 16) as u16);
        self.write16(addr.wrapping_add(2), value as u16);
        if watched {
            self.wram_long_write = false;
        }
    }
}

impl System {
    fn vdp_status(&mut self) -> u16 {
        // DMA busy is bit1, HBLANK bit2, VBLANK bit3, VINT bit7. The first
        // three are live device levels; reading does not clear them. A status
        // read resets the two-word control-command latch. FIFO contention and
        // sprite collision/overflow status are not fully modelled here.
        self.vdp.control_pending = false;
        let mut status = 0x0200u16; // FIFO empty (FIFO contention not yet modelled)
        if self.vdp.dma_busy() {
            status |= 0x0002;
        }
        if self.vdp.vblank || !self.vdp.display_enabled() {
            status |= 0x0008;
        }
        if self.vdp.hblank {
            status |= 0x0004;
        }
        if self.vdp.vint_pending {
            status |= 0x0080;
        }
        status
    }

    fn io_read(&self, a: u32) -> u8 {
        // GPGX io_68k_read: each byte register occupies an even/odd pair.
        // Default three-button pad, all buttons released; TH is an output
        // only when its control bit is set. D7 always reflects the data latch.
        match (a >> 1) & 0xF {
            0 => self.io.version_register,
            1 => crate::input::read_pad(
                self.io.data_a as u8,
                self.io.ctrl_a as u8,
                self.io.pad_buttons[0],
            ),
            2 => crate::input::read_pad(
                self.io.data_b as u8,
                self.io.ctrl_b as u8,
                self.io.pad_buttons[1],
            ),
            3 => 0x7F,
            4 => self.io.ctrl_a as u8,
            5 => self.io.ctrl_b as u8,
            6 => self.io.ctrl_c as u8,
            7 | 10 => 0xFF,
            13 => 0xFB,
            _ => 0,
        }
    }

    fn io_write(&mut self, a: u32, value: u8) {
        // I/O traffic is logged so the boot trace can show what the game
        // programs and in what order.
        self.io_write_count += 1;
        if self.io_log.len() < self.observation_limit {
            self.io_log.push((a & 0x00FF_FFFF, value));
        }
        match (a >> 1) & 0xF {
            1 => self.io.data_a = value as u16,
            2 => self.io.data_b = value as u16,
            3 => self.io.data_c = value as u16,
            4 => self.io.ctrl_a = value as u16,
            5 => self.io.ctrl_b = value as u16,
            6 => self.io.ctrl_c = value as u16,
            _ => {}
        }
        // The sound handshake is two independent bytes, and decoding either
        // one on its own gets it backwards. 0xA11100 bit 0 clear means the
        // 68000 holds the bus with the Z80 stopped; 0xA11102 bit 0 is the
        // reset line, active low. A driver writes `move.w #$0100,$A11100` to
        // release reset while keeping the bus, then polls 0xA11101 for the
        // acknowledge - so the value 0x0100 lands in the *second* byte, and
        // reading the whole word as one control register puts the reset line
        // in the wrong place.
        self.z80_ctrl_write(a, value as u8);
    }

    /// The 68000's side of the sound handshake.
    ///
    /// Two bytes, decoded separately. 0xA11100 bit 0 *set* is the 68000
    /// asking for the bus, which stops the Z80; bit 8 of the same word - which
    /// lands in 0xA11102 - is the reset line, active low. A driver writing
    /// `move.w #$0100,$A11100` therefore both releases reset and leaves the Z80
    /// holding the bus, which is exactly what it means.
    fn z80_ctrl_write(&mut self, a: u32, value: u8) {
        self.io_write_count += 1;
        if self.io_log.len() < self.observation_limit {
            self.io_log.push((a, value));
        }
        // The mask has to preserve the A1 page and every byte below it:
        // `a & 0x00F0_1F00` collapses $A11100 to $A01100 and therefore never
        // matches, so the whole handshake decodes to nothing.
        // The page mask alone is not enough: `0xA11100` and `0xA11102` differ
        // only in the low byte, and they are different signals. Masking off the
        // low byte collapses both onto one case and the reset line never
        // decodes at all - the Z80 then sits released-but-never-reset and the
        // 68000's release has no effect.
        match a & 0x00FF_FF00 {
            0x00A1_1100 => match a & 0xFF {
                0x00 => {
                    // Bit 0 *set* is the 68000 asking for the bus, which stops the
                    // Z80. The driver's `move.w #$0100` therefore both releases
                    // reset (bit 8, landing in the second byte) and leaves the Z80
                    // holding the bus.
                    self.io.z80_bus_request = value & 1 != 0;
                    // The bus arbitration logic reports itself to the Z80
                    // through two bytes of its own RAM. A driver cannot see
                    // the 68000's register at all - the Z80 has no path to it -
                    // so these two bytes are the only way it learns it now owns
                    // the bus, and a driver that does not see them spins forever
                    // in a poll loop indistinguishable from a working idle
                    // loop.
                    self.zram_bus_flags(value & 1 != 0);
                }
                0x02 => self.io.z80_reset = value & 1 == 0,
                _ => {}
            },
            // 0xA11200 is the older register driving the same reset line.
            // Only the first byte is a control: 0xA11201 reads back the bus
            // acknowledge, and a word write to the register walks into it -
            // honouring that byte re-asserts reset immediately after the
            // driver released it, and the Z80 freezes mid-program.
            0x00A1_1200 => {
                if a & 0xFF == 0x00 {
                    self.io.z80_reset = value & 1 == 0;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> System {
        System::new(crate::test_support::cartridge())
    }

    #[test]
    fn cartridge_reads_mirror_across_the_aperture() {
        let mut s = system();
        let first = s.read8(0x0000_0000);
        let mirrored = s.read8(0x0008_0000);
        assert_eq!(first, mirrored, "a 512 KiB cart repeats at 0x80000");
    }

    #[test]
    fn work_ram_is_one_linear_64_kib_window() {
        let mut s = system();
        s.write8(0x00FF_0000, 0xAB);
        // E00000-FFFFFF is one aperture: the same cell, seen from two decodes.
        assert_eq!(s.read8(0x00FF_0000), 0xAB);
        assert_eq!(s.read8(0x00E0_0000), 0xAB);
        // The top of the aperture is the far end of the RAM, not a mirror of
        // its start, because the bus folds to the low 16 address bits.
        assert_eq!(s.read8(0x00FF_FFFF), 0x00);
    }

    #[test]
    fn reset_vectors_are_reported_from_the_cartridge() {
        let s = system();
        let (sp, pc) = s.reset_vectors();
        assert_eq!(sp, 0xFFFF_1000);
        // The cart's entry point is 0x00000200; masked to the 24-bit bus that
        // is a cartridge address, which is where the trace starts from.
        assert_eq!(pc, 0x0000_0200);
    }
}
// ---------------------------------------------------------------------------
// The Z80's view of the machine
// ---------------------------------------------------------------------------

/// The Mega Drive's Z80 memory map, as Genesis Plus GX implements it:
/// 0x0000-0x3FFF is the 8 KiB of Z80 RAM mirrored four times, 0x4000-0x5FFF is
/// the YM2612, 0x6000-0x60FF the bank register, 0x7F00-0x7FFF the VDP through
/// the 68000's window, and 0x8000-0xFFFF a 32 KiB window onto banked 68K
/// address space.
///
/// This is not a flat 64 KiB space, which is why `Z80Bus` is a trait rather than
/// an array: the sound driver can and does reach outside its own RAM into
/// cartridge ROM, and a flat model would silently return zeros for code it is
/// actually executing.
impl Z80Bus for System {
    fn read(&mut self, addr: u16) -> u8 {
        match (addr >> 13) & 7 {
            0 | 1 => self.z80_ram[(addr as usize) & (Z80_RAM_SIZE - 1)],
            2 => {
                // The FM chip's status ports. Returns open bus on the two
                // register ports, which is what makes driver write loops such
                // as "write a register then poll it" terminate.
                match addr & 3 {
                    0 | 2 => 0x00,
                    _ => 0xFF,
                }
            }
            3 => {
                if addr >> 8 == 0x7F {
                    // The VDP, reached through the 68000's own window.
                    self.read8(0xC0_0000 | ((addr as u32) & 0x1FFF))
                } else {
                    0xFF
                }
            }
            _ => {
                let a = self.io.zbank | ((addr as u32) & 0x7FFF);
                self.read8(a)
            }
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match (addr >> 13) & 7 {
            0 | 1 => {
                let o = (addr as usize) & (Z80_RAM_SIZE - 1);
                self.z80_ram[o] = value;
            }
            2 => {
                self.z80_sound_write_count += 1;
                if self.z80_sound_writes.len() < 32 {
                    self.z80_sound_writes
                        .push((addr, value, self.z80_active_pc));
                }
                self.fm.write(addr as usize & 3, value);
            }
            3 => match addr >> 8 {
                0x60 => {
                    // A one-bit shift register feeding the bank window. Bit 0
                    // of the written value lands at the top of the address.
                    self.io.zbank = ((self.io.zbank >> 1) | ((value as u32 & 1) << 23)) & 0xFF8000;
                }
                0x7F => self.write8(0xC0_0000 | ((addr as u32) & 0x1FFF), value),
                _ => {}
            },
            _ => {
                let a = self.io.zbank | ((addr as u32) & 0x7FFF);
                self.write8(a, value);
            }
        }
    }

    fn port_out(&mut self, port: u16, value: u8) {
        self.z80_port_counts[port as usize % 256] += 1;
        if self.z80_port_writes.len() < 32 {
            self.z80_port_writes.push((port, value, self.z80_active_pc));
        }
        // Mega Drive mode decodes only the top two bits and the low bit.
        match port & 0xC1 {
            // The PSG of an unmodified Mega Drive is not fitted; the pin
            // reads as open bus, which is what a driver probes for.
            0x40 | 0x41 => {}
            0x80 => self.write8(0xC0_0000, value),
            0x81 => self.write8(0xC0_0004, value),
            0x01 => self.io.data_a = value as u16,
            _ => {
                if (port & 0xFF) >= 0xF0 {
                    self.fm.write(port as usize & 3, value);
                }
            }
        }
    }

    fn in_reset(&self) -> bool {
        self.io.z80_reset
    }
}

impl System {
    /// Publish a bus-arbitration state change into the Z80's RAM.
    ///
    /// $1FFF bit 7 is set while the 68000 holds the bus and the Z80 is
    /// stopped; $1FFB bit 7 is set once the Z80 has been released. Drivers
    /// poll both before doing anything else, which is why they are set here
    /// rather than only when the Z80 runs: the Z80 has to be able to observe
    /// the transition that releases it.
    fn zram_bus_flags(&mut self, stopped: bool) {
        let stopped_flag = Z80_RAM_SIZE - 1;
        let ack = Z80_RAM_SIZE - 5;
        if stopped {
            self.z80_ram[stopped_flag] |= 0x80;
            self.z80_ram[ack] &= !0x80;
        } else {
            self.z80_ram[stopped_flag] &= !0x80;
            self.z80_ram[ack] |= 0x80;
        }
    }

    /// Run the sound CPU for `cycles` Z80 T-states. The Mega Drive clocks the
    /// Z80 at half the 68000 rate, so one 68000 cycle is two of these.
    pub fn step_z80(&mut self, cycles: u32) -> u32 {
        let mut spent = 0;
        while spent < cycles {
            // The CPU and the bus it drives are both fields of this struct, so
            // the CPU is moved out for the duration of the step and put back.
            // Borrowing them separately is not an option without splitting the
            // machine in two, and the alternative - a raw pointer - would be
            // unsound for no benefit.
            let mut cpu = std::mem::take(&mut self.z80);
            self.z80_active_pc = cpu.pc;
            self.z80_bus_active = true;
            let stepped = cpu.step(self);
            self.z80_bus_active = false;
            self.z80 = cpu;
            match stepped {
                Some(n) => spent += n,
                // Held in reset: the CPU burns no time at all, which is the
                // whole point of the line. Returning rather than spinning
                // keeps a driver that is waiting for its own upload from
                // looking like a hang.
                None => break,
            }
        }
        spent
    }
}
