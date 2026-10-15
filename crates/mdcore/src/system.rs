//! The Mega Drive as a whole: cartridge, work RAM, VDP and I/O behind one bus.

#[path = "state.rs"]
pub(crate) mod state;
pub use state::MachineState;

use crate::cartridge::Cartridge;
use crate::cpu::bus::Bus;
use crate::cpu::m68000::{Exception, M68000};
use crate::fm::Fm;
use crate::memmap::{fold_to_wram, region, Region};
use crate::vdp::Vdp;
use crate::z80::{Z80Bus, Z80};

/// The Z80's RAM is 8 KiB, mirrored twice in its 16 KiB RAM window.
pub const Z80_RAM_SIZE: usize = 0x2000;

/// 64 KiB of 68K work RAM, mirrored through the top of the map.
pub const WRAM_SIZE: usize = 0x1_0000;

/// Console I/O registers the 68000 can reach.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Io {
    /// Pressed-button masks for the two three-button controllers.
    pub pad_buttons: [u8; 2],
    /// Pressed-button masks for J-Cart controllers 3 and 4.
    #[serde(default)]
    pub extra_pad_buttons: [u8; 2],
    pub data_a: u16,
    pub data_b: u16,
    pub data_c: u16,
    pub data_d: u16,
    pub ctrl_a: u16,
    pub ctrl_b: u16,
    pub ctrl_c: u16,
    pub ctrl_d: u16,
    /// BUSREQ asserted; 68000 access is granted only with RESET released.
    pub z80_bus_request: bool,
    /// True while the reset line is asserted. Active low on the wire.
    pub z80_reset: bool,
    /// Nine serial bank bits, stored in address bits 23..15, supplying the
    /// 68K address visible through the Z80's 0x8000-0xFFFF window.
    pub zbank: u32,
    pub version_register: u8,
    pub serial_counter: u8,
    /// Pad logic behind ports A and B (TH state, 6-button counter, latency).
    #[serde(default)]
    pub pads: [crate::input::PadPort; 2],
    /// Serial registers $A1000F-$A1001F (TxData, RxData, S-Ctrl per port).
    #[serde(default = "serial_reset")]
    pub serial: [u8; 9],
    /// Light gun the reference attaches to port B for this title.
    #[serde(default)]
    pub gun: Option<crate::input::LightGun>,
    /// Level-2 external interrupt (gun HL), ORed into the VDP's level until
    /// the next acknowledge or VDP interrupt-line update (GPGX
    /// m68k_update_irq(2)).
    #[serde(default)]
    pub ext_irq: bool,
}

fn serial_reset() -> [u8; 9] {
    [0xFF, 0, 0, 0xFF, 0, 0, 0xFB, 0, 0]
}

impl Io {
    fn new() -> Self {
        Self {
            // Region code 0xA0: no expansion unit attached.
            version_register: 0xA0,
            z80_reset: true,
            serial: serial_reset(),
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
    /// Opt-in 68000 sound-side write log; observer only, never saved.
    pub sound_commands: Option<crate::observe::SoundCommandLog>,
    /// Cartridge SRAM and bank-register state.
    pub cart_hw: crate::cart_hw::CartHw,
    /// 68000 instruction-stream position for open-bus reads (transient).
    open_bus_pc: u32,
    /// Opt-in Z80 RAM write watch and Z80 instruction trace; observers only.
    pub z80_ram_watch: Option<crate::observe::Z80RamWatch>,
    pub z80_trace: Option<crate::observe::Z80Trace>,
    wram_long_write: bool,
    cpu_bus_active: bool,
    z80_bus_active: bool,
    interrupt_bus_active: bool,
    stamp_full_registers: ([u32; 8], [u32; 8], u16),
    /// Physical 8 KiB Z80 RAM, mirrored in both CPU address spaces.
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
    /// The first Z80 I/O port writes, with the PC. These ports have no device
    /// effects in Genesis mode; they are retained as diagnostic traffic.
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
    /// Per-console synthesis; PCM retention is an optional observer.
    pub audio: crate::audio::Audio,
    audio_step_events: Vec<crate::audio::EventKind>,
    /// External DMA waits queued by a control-port write during the current
    /// instruction. Account for device time before accepting the next IRQ.
    pending_dma_master_cycles: u32,
    /// Remainder of a 68000 bus stall split at a frame edge; the next step
    /// finishes it without dispatching.
    pub(crate) stall_carry: u32,
    dma_overlaps_instruction: bool,
    pending_bus_cpu_cycles: u32,
    /// Absolute next sound-dispatch clock, including held-bus synchronization.
    z80_master_clock: u64,
    /// External DMA owns the shared 68000 bus until this clock.
    z80_dma_until: u64,
    pending_z80_cpu_wait: u32,
    z80_bus_requests: u32,
    z80_sync_origin: u64,
    /// Dispatch clock and opcode bytes of the executing Z80 instruction.
    z80_access: (u64, [u8; 4]),
    /// System master clock minus the FM chip's scheduled clock (the chip
    /// starts at zero at reset release).
    pub(crate) fm_offset: u64,
    /// Header I/O field lists a 6-button pad ('6'): the reference's automatic
    /// pad type for both ports. Derived from the cartridge, not saved.
    pub six_button_pads: bool,
    /// Refresh clocks already elapsed for the executing instruction but not
    /// yet applied to the devices: the reference applies the refresh delay
    /// after its line-end check, so an instruction dispatched before a line
    /// edge still sees that line's state while its accesses are timed later.
    access_bias: u32,
    /// Opt-in log of every VDP control-port word: (frame, PC, value, a
    /// second command word). Bounded by `vdp_control_log_limit`.
    pub vdp_control_log: Option<Vec<(u64, u32, u16, bool)>>,
    pub vdp_control_log_limit: usize,
    /// SVP chip (Virtua Racing), when the header announces it.
    pub svp: Option<Box<crate::svp::Svp>>,
    /// Scanlines the SVP has been run for (it runs after each line).
    pub(crate) svp_line: u64,
    /// 68000 source addresses of the queued external DMA words (aligned to
    /// the queue's tail), so SVP DRAM words are read when the VDP takes them.
    dma_queue_sources: std::collections::VecDeque<u32>,
    z80_dma_timeframe_end: u64,
    deferred_z80_sync: Option<(u64, bool)>,
    sound_prefetch_word: u16,
    master_cycles: u64,
    refresh_deadline: u64,
    /// SR writes acknowledge an already-pending IRQ before fixed opcode clocks.
    pending_irq_ack: Option<(u8, u64)>,
    /// The interrupt in `pending_irq_ack` was already acknowledged by the SR
    /// write that unmasked it (the reference takes it inside that
    /// instruction, before the next line's interrupts are raised).
    pub(crate) irq_acked: bool,
    pending_fifo_master_cycles: u32,
}

impl System {
    pub fn new(cart: Cartridge) -> Self {
        let region = crate::cart_hw::ConsoleRegion::detect(&cart.data);
        Self::with_region(cart, region)
    }

    /// Power on as a console of `region`: version register, and PAL video
    /// and sound timing for PAL regions.
    pub fn with_region(cart: Cartridge, region: crate::cart_hw::ConsoleRegion) -> Self {
        let cart_hw = crate::cart_hw::CartHw::new(&cart);
        let six_button_pads = cart.data.get(0x190..0x19E).is_some_and(|io| io.contains(&b'6'));
        let has_svp = cart.data.get(0x1C8..0x1CA) == Some(b"SV");
        let mut system = Self {
            cart_hw,
            open_bus_pc: 0,
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
            sound_commands: None,
            z80_ram_watch: None,
            z80_trace: None,
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
            audio: crate::audio::Audio::new(),
            audio_step_events: Vec::new(),
            pending_dma_master_cycles: 0,
            stall_carry: 0,
            dma_overlaps_instruction: false,
            pending_bus_cpu_cycles: 0,
            z80_master_clock: 0,
            z80_dma_until: 0,
            pending_z80_cpu_wait: 0,
            z80_bus_requests: 0,
            z80_sync_origin: 0,
            z80_access: (0, [0; 4]),
            fm_offset: 0,
            six_button_pads,
            access_bias: 0,
            vdp_control_log: None,
            vdp_control_log_limit: 4096,
            svp: has_svp.then(|| Box::new(crate::svp::Svp::new())),
            svp_line: 0,
            dma_queue_sources: std::collections::VecDeque::new(),
            z80_dma_timeframe_end: 0,
            deferred_z80_sync: None,
            sound_prefetch_word: 0,
            master_cycles: 0,
            refresh_deadline: 896,
            pending_irq_ack: None,
            irq_acked: false,
            pending_fifo_master_cycles: 0,
        };
        system.vdp.pal = region.is_pal();
        system.audio.set_pal(region.is_pal());
        system.io.version_register = region.version_register();
        system.reset();
        system.io.version_register = region.version_register();
        system
    }

    /// Power the console back on as `region` (observers are discarded).
    pub fn set_region(&mut self, region: crate::cart_hw::ConsoleRegion) {
        *self = Self::with_region(self.cart.clone(), region);
    }

    pub fn region(&self) -> crate::cart_hw::ConsoleRegion {
        use crate::cart_hw::ConsoleRegion as R;
        match self.io.version_register & 0xC0 {
            0x00 => R::JapanNtsc,
            0x40 => R::JapanPal,
            0x80 => R::Usa,
            _ => R::Europe,
        }
    }

    /// Reset the console from the cartridge's exception vector table.
    ///
    /// The first long is the initial stack pointer, the second the entry point.
    /// The 68000 only drives A1-A23, so an entry point is decoded modulo 2^24
    /// (that fold decides where a cart with an unusual vector starts), but the
    /// PC register keeps all 32 bits, as in the reference.
    pub fn reset(&mut self) {
        self.wram.fill(0);
        if let Some(svp) = self.svp.as_mut() {
            svp.reset();
        }
        self.svp_line = 0;
        self.dma_queue_sources.clear();
        self.vdp.reset();
        self.pending_dma_master_cycles = 0;
        self.stall_carry = 0;
        self.pending_irq_ack = None;
        self.irq_acked = false;
        self.dma_overlaps_instruction = false;
        self.io = Io::new();
        self.io.gun = self.cart.data.get(0x150..0x180).and_then(crate::input::LightGun::detect);
        self.z80 = Z80::new();
        self.z80_master_clock = 0;
        self.z80_dma_until = 0;
        self.pending_z80_cpu_wait = 0;
        self.z80_bus_requests = 0;
        self.z80_dma_timeframe_end = 0;
        self.deferred_z80_sync = None;
        self.pending_bus_cpu_cycles = 0;
        self.fm.reset();
        self.audio = crate::audio::Audio::new();
        self.audio_step_events.clear();
        let (sp, _) = self.reset_vectors();
        // The reference loads the full 32-bit entry point; only bus decoding
        // folds it to 24 bits, so stacked return addresses keep the top byte.
        let pc = (4..8).fold(0u32, |v, at| (v << 8) | u32::from(self.cart_byte(at)));
        self.cpu = M68000::new(pc);
        // The reference's power-on USP stays 0: its reset only loads SSP. Its
        // zeroed flag storage reads as Z set (FLAG_Z holds "not zero").
        self.cpu.sr.0 = 0x2704;
        self.cpu.a[7] = sp;
        let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
        cpu.prime(self);
        self.cpu = cpu;
        // GPGX gen_reset: hardware-measured hard-reset phase (27 lines earlier
        // on PAL), followed by the 68000's 40-cycle reset exception. VDP
        // initially runs Mode 4.
        let lines = u64::from(self.vdp.lines_per_frame()) - 192 + 159 - if self.vdp.pal { 27 } else { 0 };
        let release = (lines * 3420 + 1004) / 7 * 7 + 40 * 7;
        self.master_cycles = release;
        // The reference's FM sample grid starts at the first frame's origin,
        // not at reset release: run the (idle) chip clock up to the release.
        if self.fm.clock() < release {
            self.fm.tick((release - self.fm.clock()) as u32);
            self.fm.sync_to(release);
        }
        self.fm_offset = release.wrapping_sub(self.fm.clock());
        self.z80_master_clock = release.div_ceil(15) * 15;
        self.refresh_deadline = (release / 896 + 1) * 896;
        self.pending_fifo_master_cycles = 0;
        self.vdp.advance_master_cycles(release as u32);
        self.audio.advance_to(release);
    }

    /// Set the pressed-button mask of controller `port` (0-1 console, 2-3 J-Cart).
    pub fn set_pad(&mut self, port: usize, mask: u8) {
        match port {
            0 | 1 => self.io.pad_buttons[port] = mask,
            2 | 3 => self.io.extra_pad_buttons[port - 2] = mask,
            _ => {}
        }
    }

    /// J-Cart ports 3/4 as the cartridge presents them: port 3 in D7-D0 (TH on
    /// D6), port 4's buttons in D13-D8.
    fn jcart_pads(&self) -> u16 {
        if !self.cart_hw.has_jcart() {
            return 0;
        }
        let th = if self.cart_hw.jcart_th { 0x40 } else { 0 };
        let pad = |buttons| u16::from(crate::input::read_pad(th, 0x40, buttons));
        (pad(self.io.extra_pad_buttons[0]) & 0x7F) | ((pad(self.io.extra_pad_buttons[1]) & 0x3F) << 8)
    }

    /// The 68000's next instruction-stream word, as an unmapped read returns.
    /// The reference's program view: the word at `addr` read straight from
    /// each bank's base pointer, bypassing read handlers. Instruction fetch,
    /// PC-relative operands and open bus (the word at the PC) all use it.
    /// The PC is never checked, so odd addresses read the byte-swapped host
    /// halfword (the bytes at a-1 and a+2), running off a bank's 64 KiB
    /// block into the next host buffer (work RAM is followed by the empty
    /// boot ROM buffer; ROM continues linearly). `None` for banks whose base
    /// is not modelled (SVP, open-bus mapper banks, Realtec boot).
    fn program_word(&self, addr: u32) -> Option<u16> {
        let a = addr & 0x00FF_FFFF;
        if a & 1 != 0 {
            let before = self.program_word(a - 1)?;
            let after = if a & 0xFFFF != 0xFFFF {
                self.program_word(a + 1)? & 0xFF
            } else if region(a) == Region::Cartridge {
                let base = self.cart_hw.raw_offset(a - 1);
                let linear = if base & crate::cartridge::RAW_OFFSET != 0 {
                    (base & !crate::cartridge::RAW_OFFSET) as usize
                } else {
                    base as usize % self.cart.mirror_period()
                };
                u16::from(self.cart.buffer_byte(linear + 2))
            } else {
                0
            };
            return Some((before >> 8) | (after << 8));
        }
        match a >> 16 {
            // Expansion, Z80, I/O and VDP banks have work RAM as their base
            // pointer (for VDP DMA), as does work RAM itself.
            0x80..=0xFF => {
                let o = (a & 0xFFFF) as usize;
                return Some(u16::from_be_bytes([self.wram[o], self.wram[o + 1]]));
            }
            // Unused area: the ROM buffer continues linearly (0xFF padding up
            // to the power-of-two size, empty beyond).
            0x40..=0x7F => {
                let byte = |o: usize| self.cart.buffer_byte(o);
                return Some(u16::from_be_bytes([byte(a as usize), byte(a as usize + 1)]));
            }
            _ => {}
        }
        let bank = (a >> 16) & 0x3F;
        (region(a) == Region::Cartridge
            && !self.svp_bank(a)
            && !self.cart_hw.open_bus_bank(bank)
            && !self.cart_hw.hw.realtec)
            .then(|| {
                let byte = |a: u32| match self.cart_hw.read_base(a) {
                    Ok(byte) => byte,
                    Err(offset) => self.cart.offset_byte(offset),
                };
                u16::from_be_bytes([byte(a), byte(a | 1)])
            })
    }

    fn open_bus_word(&self) -> u16 {
        if let Some(word) = self.program_word(self.open_bus_pc) {
            return word;
        }
        let pc = self.open_bus_pc & 0xFF_FFFE;
        match region(pc) {
            // The reference reads the bank base directly (no read handlers).
            Region::Cartridge => u16::from_be_bytes([self.cart_byte_raw(pc), self.cart_byte_raw(pc | 1)]),
            Region::Wram => {
                let o = fold_to_wram(pc) as usize;
                u16::from_be_bytes([self.wram[o], self.wram[o + 1]])
            }
            _ => 0xFFFF,
        }
    }

    /// Cartridge-window byte through SRAM and bank mapping.
    fn cart_byte(&self, address: u32) -> u8 {
        match self.cart_hw.read_kind(address) {
            crate::cart_hw::CartRead::Byte(byte) => byte,
            crate::cart_hw::CartRead::Rom(offset) => self.cart.offset_byte(offset),
            crate::cart_hw::CartRead::OpenBus => self.open_bus_word().to_be_bytes()[(address & 1) as usize],
        }
    }

    /// SVP memory banks: DRAM ($30/$31) and the cell views ($39/$3A).
    fn svp_bank(&self, address: u32) -> bool {
        self.svp.is_some() && matches!((address >> 16) & 0x3F, 0x30 | 0x31 | 0x39 | 0x3A)
    }

    fn svp_read16(&self, address: u32) -> u16 {
        let Some(svp) = self.svp.as_ref() else { return 0xFFFF };
        match (address >> 16) & 0x3F {
            0x30 | 0x31 => svp.dram_word(address & 0x1FFFE),
            _ => svp.read_cell(address),
        }
    }

    /// ROM byte through the bank bases only (open bus / prefetch view).
    fn cart_byte_raw(&self, address: u32) -> u8 {
        match self.cart_hw.read(address) {
            Ok(byte) => byte,
            Err(offset) => self.cart.offset_byte(offset),
        }
    }

    fn cart_long(&self, offset: usize) -> u32 {
        let b = |i: usize| u32::from(self.cart.data.get(offset + i).copied().unwrap_or(0xff));
        (b(0) << 24) | (b(1) << 16) | (b(2) << 8) | b(3)
    }

    /// The raw 32-bit long stored at `offset` in the cartridge.
    pub fn cart_long_public(&self, offset: usize) -> u32 {
        self.cart_long(offset)
    }

    /// The exception vectors as the hardware would use them: the stack pointer
    /// and entry point are masked to the 24-bit bus.
    pub fn reset_vectors(&self) -> (u32, u32) {
        // Through the cartridge mapping: a board can map a boot ROM over the
        // vectors at power-on (Realtec).
        let long = |at: u32| (0..4).fold(0u32, |v, i| (v << 8) | u32::from(self.cart_byte(at + i)));
        (long(0), long(4) & 0x00FF_FFFF)
    }

    /// Elapsed console master clocks, including reset phase and exact bus waits.
    /// Unlike summed instruction costs, this does not round DMA/FIFO waits up
    /// to whole 68000 clocks. The origin is the initial VDP VBLANK boundary.
    pub fn master_cycles(&self) -> u64 {
        self.master_cycles
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
        let step_start = self.master_cycles;
        if std::mem::take(&mut self.vdp.irq_line_written) {
            self.io.ext_irq = false;
        }
        self.refresh_dma_sources();
        self.vdp.flush_frame_start_dma();
        if self.stall_carry != 0 {
            let rest = std::mem::take(&mut self.stall_carry);
            self.advance_stall(rest);
            return ((self.master_cycles - step_start).div_ceil(7) as u32, None);
        }
        self.dma_overlaps_instruction = false;
        self.pending_bus_cpu_cycles = 0;
        self.pending_fifo_master_cycles = 0;
        self.sound_prefetch_word = self.cpu.prefetch[0];
        self.stamp_pc = self.cpu.pc;
        self.stamp_full_registers = (self.cpu.d, self.cpu.a, self.cpu.sr.0);
        self.cpu_bus_active = true;
        self.interrupt_bus_active = false;
        self.stamp_regs = [self.cpu.a[6], self.cpu.d[4], self.cpu.d[1], self.cpu.d[2]];
        // Interrupts are only accepted at instruction boundaries and only when
        // the interrupt mask allows this level. The VDP drives level 6 for
        // vertical blank and level 4 for horizontal blank.
        if self.vdp.hold_vint && !self.vdp.irq_delay && self.pending_irq_ack.is_none() {
            // The delayed-interrupt chain ended without an interrupt entry.
            self.vdp.release_vint_hold();
        }
        let delayed = std::mem::take(&mut self.vdp.irq_delay);
        let pending_ack = self.pending_irq_ack.take();
        let acked = std::mem::take(&mut self.irq_acked);
        let level = if acked {
            pending_ack.map(|(level, _)| level)
        } else {
            self.irq_level().filter(|l| !delayed && !self.cpu.halted && self.cpu.sr.accepts(*l))
        };
        if let Some(level) = level {
            self.interrupt_bus_active = true;
            if !acked {
                self.acknowledge_irq();
            }
            self.vdp.release_vint_hold();
            let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
            cpu.interrupt(self, level);
            self.cpu = cpu;
            self.cpu_bus_active = false;
            self.interrupt_bus_active = false;
            // Autovector acknowledge waits for the E-clock phase. Match the
            // independent reference's frame-relative phase (including the
            // fractional CPU clock carried across each NTSC frame boundary).
            let acknowledge_clock = pending_ack
                .filter(|(irq, _)| *irq == level)
                .map_or(self.master_cycles, |(_, clock)| clock);
            let phase = ((acknowledge_clock % self.vdp.frame_master()) / 7 % 10) as usize;
            let cycles = [50, 59, 58, 57, 56, 55, 54, 53, 52, 51][phase];
            self.advance_devices(cycles * 7);
            return ((self.master_cycles - step_start).div_ceil(7) as u32, None);
        }
        if self.cpu.stopped {
            // STOP holds instruction execution, not the VDP/Z80/FM clocks.
            // Yield to the next physical line/VINT boundary, then check the
            // interrupt mask again on the next system step.
            self.cpu_bus_active = false;
            let idle = self.vdp.line_event_in_master_cycles()
                .min(self.vdp.sound_event_in_master_cycles());
            self.advance_devices(idle);
            return ((self.master_cycles - step_start).div_ceil(7) as u32, None);
        }
        // Refresh is a console bus wait, not an opcode cost. STOP/IRQ entry
        // does not dispatch an instruction and must not introduce a refresh.
        // The delayed instruction runs nested inside the reference's write
        // handler, outside its dispatch loop's refresh check.
        let refresh = if !self.cpu.stopped && !delayed && self.master_cycles >= self.refresh_deadline {
            self.refresh_deadline = self.master_cycles + 896;
            2
        } else {
            0
        };
        // Refresh precedes the instruction's bus accesses in GPGX's dispatcher.
        // Account it now so an HV/FM/FIFO read observes the right instant, not
        // merely the right total clocks at the instruction's end.
        let refresh_master = refresh * 7;
        let defer_refresh =
            refresh_master != 0 && refresh_master >= self.vdp.line_event_in_master_cycles();
        if refresh_master != 0 && !defer_refresh {
            self.advance_devices_inner(refresh_master, true);
        }
        self.access_bias = if defer_refresh { refresh_master } else { 0 };
        let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
        self.cpu_active_pc = cpu.pc;
        let outcome = cpu.step(self);
        let mut instruction_cycles = outcome.cycles;
        // Trap-class exceptions (TRAP, TRAPV, CHK, divide by zero) leave the
        // CPU at the following PC and stack it. Faults stack the faulting
        // instruction's PC, allowing the handler to inspect it. Clocks follow
        // GPGX's exception table on top of the instruction's own cost.
        if let (Some(Exception::AddressError { .. }), Some((fault, ir))) =
            (outcome.exception, cpu.last_fault.take())
        {
            cpu.address_error_entry(self, fault, ir);
            // The reference charges the exception less the opcode's table
            // cost and never adds that cost back (longjmp out of the opcode).
            let base = crate::cpu::m68000::base_cycles(ir);
            instruction_cycles = 50u32.saturating_sub(base);
        }
        let entry = match outcome.exception {
            Some(Exception::Vector { vector }) => {
                Some((vector, if matches!(vector, 5 | 6) { 38 } else { 34 }))
            }
            Some(Exception::DivideByZero) => Some((5, 38)),
            Some(Exception::Illegal { opcode }) => Some((match opcode >> 12 {
                0xa => 10, // line-A emulator exception
                0xf => 11, // line-F emulator exception
                _ => 4,
            }, 34)),
            Some(Exception::Privileged) => Some((8, 34)),
            _ => None,
        };
        if let Some((vector, cycles)) = entry {
            // Illegal instruction and privilege violation (like interrupts)
            // leave the reference's instruction mode until RTE.
            if matches!(vector, 4 | 8) {
                cpu.instr_mode_no = true;
            }
            let return_pc = cpu.pc;
            cpu.exception_entry(self, vector, return_pc);
            instruction_cycles = outcome.cycles + cycles;
        }
        if outcome.exception.is_none() && cpu.status_written && !cpu.halted {
            if let Some(level) = self.irq_level().filter(|level| cpu.sr.accepts(*level)) {
                self.pending_irq_ack = Some((
                    level,
                    self.master_cycles + u64::from(self.pending_bus_cpu_cycles) * 7,
                ));
                // Acknowledged now, before this instruction's time can reach
                // the next line (whose HINT the reference raises afresh).
                self.acknowledge_irq();
                self.irq_acked = true;
            }
        }
        if outcome.exception.is_none() {
            if let Some(body) = cpu.refresh_block_cycles {
                // The reference checks after the variable internal clocks,
                // before the fixed opcode cost, and skips one crossed deadline.
                if self.master_cycles + u64::from(body) * 7 >= self.refresh_deadline {
                    self.refresh_deadline += 896;
                }
            }
        }
        self.cpu = cpu;
        self.cpu_bus_active = false;
        // 68000 cycles cost 7 master clocks; Z80 T-states cost 15. Carry sound
        // instruction overshoot instead of rounding up a fresh budget each step.
        let cycles = if self.dma_overlaps_instruction {
            // A continuing external DMA update sets the CPU's resume clock.
            // The triggering opcode's fixed clocks have already overlapped
            // that bus ownership interval; do not charge them a second time.
            refresh
        } else {
            instruction_cycles + self.pending_bus_cpu_cycles + refresh
        };
        if self.vdp.irq_delay && !delayed {
            // A write armed the one-instruction interrupt delay: the
            // reference runs that instruction and the interrupt inside it.
            self.vdp.hold_vint = true;
        }
        let dma_wait = std::mem::take(&mut self.pending_dma_master_cycles);
        self.vdp.dma_bus_stall_master_cycles += u64::from(dma_wait);
        let fifo_wait = self.pending_fifo_master_cycles;
        let deferred = std::mem::take(&mut self.access_bias);
        let elapsed = (cycles - refresh) * 7 + dma_wait + fifo_wait + deferred;
        if dma_wait != 0 && !delayed {
            self.advance_stall(elapsed);
        } else {
            self.advance_devices(elapsed);
        }
        if std::mem::take(&mut self.cart_hw.reset_request) {
            self.pulse_cpu_reset();
        }
        if delayed {
            let mut taken = self.irq_acked;
            if outcome.exception.is_none() && !taken {
                // The reference takes the delayed interrupt inside the write
                // handler, before charging this nested instruction's table cost,
                // so its acknowledge phase is sampled that much earlier.
                if let Some(level) = self.irq_level().filter(|level| self.cpu.sr.accepts(*level)) {
                    let cost = u64::from(crate::cpu::m68000::base_cycles(self.sound_prefetch_word)) * 7;
                    self.pending_irq_ack = Some((level, self.master_cycles - cost));
                    taken = true;
                }
            }
            if !taken {
                self.vdp.release_vint_hold();
            }
        }
        (
            (self.master_cycles - step_start).div_ceil(7) as u32,
            outcome.exception,
        )
    }

    /// Advance through a 68000 bus stall, pausing at the frame edge (as the
    /// reference's frame loop does) and carrying the rest to the next step.
    fn advance_stall(&mut self, master_cycles: u32) {
        let lines = if self.vdp.line < self.vdp.visible_lines {
            self.vdp.visible_lines - self.vdp.line - 1
        } else {
            self.vdp.lines_per_frame() - self.vdp.line + self.vdp.visible_lines - 1
        };
        let edge = lines * crate::vdp::MASTER_CYCLES_PER_LINE + self.vdp.line_event_in_master_cycles();
        if master_cycles <= edge {
            self.advance_devices(master_cycles);
            return;
        }
        let frame = self.vdp.frame;
        self.advance_devices(edge);
        if self.vdp.frame != frame {
            self.stall_carry = master_cycles - edge;
        } else {
            self.advance_devices(master_cycles - edge);
        }
    }

    /// GPGX vdp_68k_irq_ack: VINT if it is asserting, else HINT; the IPL
    /// lines are then rewritten, dropping any external level.
    fn acknowledge_irq(&mut self) {
        if self.vdp.vint_pending && self.vdp.registers[1] & 0x20 != 0 {
            self.vdp.vint_taken = self.vdp.vint_taken.saturating_add(1);
            self.vdp.vint_pending = false;
        } else {
            self.vdp.hint_pending = false;
        }
        self.io.ext_irq = false;
    }

    /// Whether the next system step enters an interrupt instead of
    /// dispatching an instruction (respecting the one-instruction delay).
    pub fn interrupt_due(&self) -> bool {
        self.irq_acked
            || (!self.vdp.irq_delay && self.irq_level().is_some_and(|level| self.cpu.sr.accepts(level)))
    }

    /// /VRES from the cartridge (reference m68k_pulse_reset): supervisor
    /// mode, mask 7, SP and PC reloaded from the vectors now mapped, reset
    /// exception clocks, refresh resynchronized to the frame-relative grid.
    fn pulse_cpu_reset(&mut self) {
        let sp = self.read32(0);
        let pc = self.read32(4);
        let mut cpu = std::mem::replace(&mut self.cpu, M68000::new(0));
        cpu.stopped = false;
        cpu.set_status((cpu.sr.0 & 0x00FF) | 0x2700);
        cpu.a[7] = sp;
        cpu.set_pc(self, pc);
        self.cpu = cpu;
        self.advance_devices(40 * 7);
        let frame = self.vdp.frame_master();
        let origin = self.master_cycles / frame * frame;
        self.refresh_deadline = origin + ((self.master_cycles - origin) / 896) * 896 + 896;
    }

    /// The autovector level currently asserted by the VDP, if any.
    pub fn irq_level(&self) -> Option<u8> {
        let vdp = if self.vdp.vint_pending && self.vdp.registers[1] & 0x20 != 0 {
            Some(6)
        } else if self.vdp.hint_pending && self.vdp.registers[0] & 0x10 != 0 {
            Some(4)
        } else {
            None
        };
        // The reference ORs the external level into its IPL bits.
        if self.io.ext_irq { Some(vdp.unwrap_or(0) | 2) } else { vdp }
    }

    // ------------------------------------------------------------- VDP port

    fn vdp_control_write(&mut self, value: u16) {
        if let Some(log) = self.vdp_control_log.as_mut() {
            if log.len() < self.vdp_control_log_limit {
                log.push((self.vdp.frame, self.cpu_active_pc, value, self.vdp.control_pending));
            }
        }
        if self.vdp.control_pending {
            self.vdp.address_high = u32::from(value & 3) << 14;
            self.vdp.address_latch = (self.vdp.address_latch & 0x3FFF) | self.vdp.address_high;
            self.vdp.code = (self.vdp.code & 3) | (((value >> 2) & 0x3C) as u8);
            self.vdp.control_pending = false;
            if self.vdp.code & 0x20 != 0 && self.vdp.registers[1] & 0x10 != 0 {
                self.vdp.dma_requests += 1;
                self.execute_vdp_dma();
            }
            return;
        }
        // The 68000 is frozen during 68000-bus DMA: a second word of the same
        // MOVE.L waits for the transfer to complete (GPGX cached_write).
        if !self.vdp.dma_queue.is_empty() {
            self.vdp.cached_write = Some(value);
            return;
        }
        if value & 0xC000 == 0x8000 {
            self.vdp_reg_write_count += 1;
            if self.vdp_reg_writes.len() < 48 {
                let index = (value >> 8) & 0x1F;
                self.vdp_reg_writes
                    .push((index as u8, (value & 0xFF) as u8, self.cpu_active_pc));
            }
        }
        let delayed = self.vdp.irq_delay;
        if self.vdp.control_first_word(value) {
            // The interrupt delay applies to 68000 writes other than MOVE.L,
            // and only once (the reference prevents re-entrance).
            if !delayed
                && (!self.cpu_bus_active || self.sound_prefetch_word & 0xF000 == 0x2000)
            {
                self.vdp.irq_delay = false;
            }
        }
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
                self.vdp.begin_internal_dma(count, true, source as u16);
                source = source.wrapping_add(count) & 0xFFFF;
            }
            _ => {
                // SVP memory: the first word written is the data previously on
                // the bus; the reference skips it and drops the last source word,
                // before timing the transfer.
                let mut count = count;
                if self.svp.is_some() && self.vdp.registers[23] & 0x60 == 0 {
                    self.vdp.address_latch =
                        self.vdp.address_latch.wrapping_add(u32::from(self.vdp.registers[15])) & 0xFFFF;
                    count -= 1;
                }
                let (wait, continues) = self.vdp.external_dma_wait(count, self.vdp.code & 0xF == 1);
                self.pending_dma_master_cycles += wait;
                self.z80_dma_until = self.z80_dma_until.max(self.master_cycles + u64::from(wait));
                self.dma_overlaps_instruction |= continues;
                // A new DMA replaces the remaining length of a fill/copy.
                self.vdp.internal_dma = None;
                let bank = (self.vdp.registers[23] as u32 & 0x7F) << 17;
                let first = self.vdp.external_dma_first_words(count, self.vdp.code & 0xF == 1);
                for index in 0..count {
                    let address = bank | (source << 1);
                    let data = self.read16(address);
                    if index < first {
                        self.vdp.data_write(data);
                    } else {
                        self.vdp.dma_queue.push_back(data);
                        self.dma_queue_sources.push_back(address);
                    }
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
    fn note_pc(&mut self, pc: u32) {
        self.open_bus_pc = pc;
    }

    fn fetch16(&mut self, addr: u32) -> u16 {
        match self.program_word(addr) {
            Some(word) => word,
            None => self.read16(addr),
        }
    }

    fn read8(&mut self, addr: u32) -> u8 {
        let a = addr & 0x00FF_FFFF;
        if Self::sound_window(a) {
            if !self.z80_bus_granted() {
                return self.sound_open_bus_byte(a);
            }
            if self.cpu_bus_active {
                self.pending_bus_cpu_cycles += 1;
            }
        }
        match region(a) {
            Region::Cartridge if self.svp_bank(a) => {
                let word = self.svp_read16(a);
                if a & 1 != 0 { word as u8 } else { (word >> 8) as u8 }
            }
            Region::Cartridge if self.cart_hw.hw.regs_bank(a, false) => {
                let bus = self.open_bus_word();
                self.cart_hw.regs_read(a, bus, self.io.pad_buttons[0]).map_or(0xFF, |v| v as u8)
            }
            Region::Cartridge => {
                match self.cart_hw.special_read(a, false, self.jcart_pads()) {
                    Some(crate::cart_hw::Special::Value(v)) => v as u8,
                    Some(crate::cart_hw::Special::OpenBus) => self.open_bus_word().to_be_bytes()[(a & 1) as usize],
                    None => self.cart_byte(a),
                }
            }
            Region::Wram => self.wram[fold_to_wram(a) as usize],
            // Byte access to the Z80's RAM matters: cartridges upload the
            // sound driver with byte writes and verify each byte by reading it
            // straight back. Without this the read fell through to the
            // catch-all, returned 0, and the cartridge's verify loop retried
            // the same address forever.
            Region::Z80Ram => self.z80_ram[(a as usize) & (Z80_RAM_SIZE - 1)],
            Region::Vdp => {
                let word = self.read16(a & !1);
                if a & 1 == 0 {
                    (word >> 8) as u8
                } else {
                    word as u8
                }
            }
            Region::Io => self.io_read(a),
            Region::Fm => {
                let (time, _) = self.cpu_z80_area_clock();
                self.fm.sync_to(time);
                self.fm.read_at((a & 3) as usize, time)
            }
            Region::Z80Ctrl => {
                if a >> 16 == 0xa0 {
                    return 0xff;
                }
                let bus = self.sound_open_bus_byte(a);
                if a & 0x100 == 0x100 && a & 1 == 0 {
                    (bus & !1) | u8::from(!self.z80_bus_granted())
                } else {
                    bus
                }
            }
            Region::Sram => {
                let bus = self.open_bus_word();
                match self.cart_hw.time_read(a, bus) {
                    Some(v) if a & 1 != 0 => v as u8,
                    Some(v) => (v >> 8) as u8,
                    None => bus.to_be_bytes()[(a & 1) as usize],
                }
            }
            _ if self.svp.is_some() && a & 0xFF_FF00 == 0xA1_5000 => {
                let bus = self.open_bus_word();
                match self.svp.as_mut().and_then(|svp| svp.read_reg(a)) {
                    Some(v) if a & 1 != 0 => v as u8,
                    Some(v) => (v >> 8) as u8,
                    None => bus.to_be_bytes()[(a & 1) as usize],
                }
            }
            _ if self.cart_hw.hw.regs_bank(a, false) => {
                let bus = self.open_bus_word();
                self.cart_hw.regs_read(a, bus, self.io.pad_buttons[0]).map_or(0xFF, |v| v as u8)
            }
            _ if self.cart_hw.smw64.is_some() && (a >> 20) == 6 => match self.cart_hw.smw64_read(a) {
                Some(crate::cart_hw::CartRead::Rom(offset)) => self.cart.offset_byte(offset),
                Some(crate::cart_hw::CartRead::Byte(v)) => v,
                _ => 0xFF,
            },
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 8, false);
                // Open bus: the Z80 bank window reads FF; the 68000 reads the
                // byte of its next instruction-stream word (reference model).
                if self.z80_bus_active {
                    0xFF
                } else {
                    self.open_bus_word().to_be_bytes()[(a & 1) as usize]
                }
            }
        }
    }

    fn write8(&mut self, addr: u32, value: u8) {
        let a = addr & 0x00FF_FFFF;
        if Self::sound_window(a) {
            if !self.z80_bus_granted() {
                // Dropped by the bus, but still visible to the sound-command log.
                use crate::observe::SoundCommandTarget as T;
                match region(a) {
                    Region::Z80Ram => {
                        let o = (a as usize) & (Z80_RAM_SIZE - 1);
                        self.observe_sound_command(T::Z80Ram, a, Some(o as u16), value);
                    }
                    Region::Z80Ctrl if a >> 16 == 0xa0 => {
                        self.observe_sound_command(T::BankRegister, a, None, value);
                    }
                    _ => {}
                }
                return;
            }
            if self.cpu_bus_active {
                self.pending_bus_cpu_cycles += 1;
            }
        }
        match region(a) {
            Region::Cartridge if self.svp_bank(a) => {
                if let (Some(svp), 0x30 | 0x31) = (self.svp.as_mut(), (a >> 16) & 0x3F) {
                    svp.write_dram_byte(a, value);
                }
            }
            Region::Cartridge if self.cart_hw.regs_write(a, u16::from(value)) => {}
            Region::Cartridge if self.cart_hw.sf_write(a, u16::from(value)) => {}
            Region::Cartridge => {
                if !self.cart_hw.special_write(a, u16::from(value), false) {
                    self.cart_hw.write(a, value);
                }
            }
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
                // Both VDP lanes receive the byte; this is not read/modify/write.
                // PSG, unused and TEST addresses must never become VDP commands.
                match a & 0x1c {
                    0x00 | 0x04 => self.write16(a & !1, u16::from(value) * 0x101),
                    0x10 | 0x14 => {
                        if a & 1 != 0 { self.sound_event(crate::audio::EventKind::PsgWrite { value }); }
                    }
                    _ => {
                        self.unmapped_accesses += 1;
                        self.observe_unmapped(a, 8, true);
                    }
                }
            }
            Region::Io => self.io_write(a, value),
            Region::Fm => {
                let (time, origin) = self.cpu_z80_area_clock();
                if a & 1 != 0 {
                    self.fm.sync_to(time);
                }
                self.fm.write_at((a & 3) as usize, value, time, origin);
                self.sound_event(crate::audio::EventKind::FmWrite { port: (a & 3) as u8, value });
            }
            // The sound handshake lives in its own decode region, not in the
            // general I/O block, so handling it inside io_write_byte is dead
            // code: nothing routes it there.
            Region::Z80Ctrl => {
                use crate::observe::SoundCommandTarget as T;
                if a >> 16 == 0xa0 {
                    self.observe_sound_command(T::BankRegister, a, None, value);
                    self.z80_bank_write(value);
                } else {
                    self.z80_ctrl_write(a, value);
                }
            }
            Region::Z80Ram => {
                let o = (a as usize) & (Z80_RAM_SIZE - 1);
                self.observe_sound_command(
                    crate::observe::SoundCommandTarget::Z80Ram,
                    a,
                    Some(o as u16),
                    value,
                );
                self.observe_z80_ram_write(o as u16, value, "m68000", self.cpu_active_pc);
                self.z80_upload_count += 1;
                if self.z80_upload.len() < self.observation_limit {
                    self.z80_upload.push(o as u32);
                }
                self.z80_ram[o] = value;
            }
            Region::Sram => {
                let mask = self.cart.mirror_period() as u32 - 1;
                self.cart_hw.time_write(a, u16::from(value), mask);
            }
            _ if self.cart_hw.regs_write(a, u16::from(value)) => {}
            _ if self.cart_hw.smw64_write(a, u16::from(value)) => {}
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 8, true);
            }
        }
    }

    fn read16(&mut self, addr: u32) -> u16 {
        let a = addr & 0x00FF_FFFF;
        match region(a) {
            Region::Cartridge if self.svp_bank(a) => self.svp_read16(a),
            Region::Cartridge if self.cart_hw.hw.regs_bank(a, false) => {
                let bus = self.open_bus_word();
                self.cart_hw.regs_read(a, bus, self.io.pad_buttons[0]).unwrap_or(0xFFFF)
            }
            Region::Cartridge => {
                if self.cart_hw.hw.realtec {
                    self.cart_hw.realtec_word_read(a);
                }
                if let Some(crate::cart_hw::Special::Value(v)) =
                    self.cart_hw.special_read(a, true, self.jcart_pads())
                {
                    return v;
                }
                u16::from_be_bytes([self.cart_byte(a), self.cart_byte(a.wrapping_add(1))])
            }
            Region::Wram => {
                let o = fold_to_wram(a) as usize;
                ((self.wram[o] as u16) << 8) | self.wram[(o + 1) & (WRAM_SIZE - 1)] as u16
            }
            Region::Z80Ram | Region::Fm => {
                if !self.z80_bus_granted() {
                    return self.sound_open_bus_word();
                }
                let byte = self.read8(a);
                u16::from(byte) * 0x101
            }
            Region::Z80Ctrl => {
                if a >> 16 == 0xa0 {
                    return if self.z80_bus_granted() {
                        u16::from(self.read8(a)) * 0x101
                    } else {
                        self.sound_open_bus_word()
                    };
                }
                let bus = self.sound_open_bus_word();
                if a & 0x100 == 0x100 {
                    (bus & !0x100) | if self.z80_bus_granted() { 0 } else { 0x100 }
                } else {
                    bus
                }
            }
            Region::Vdp => match a & 0x1E {
                // The status register is on the control port; a read of the
                // data port returns whatever the last write selected.
                // Unused status bits 15-10 return the 68000's prefetched bus
                // word (reference model); the Z80 sees the plain status.
                0x04 | 0x06 if self.z80_bus_active => self.vdp_status(),
                0x04 | 0x06 => (self.vdp_status() & 0x03FF) | (self.open_bus_word() & 0xFC00),
                0x00 | 0x02 => self.vdp.data_read(),
                0x08 | 0x0a | 0x0c | 0x0e => self.vdp.hv_counter_at(self.access_bias),
                _ => 0,
            },
            Region::Io => {
                let data = self.io_read(a) as u16;
                (data << 8) | data
            }
            Region::Sram => {
                let bus = self.open_bus_word();
                self.cart_hw.time_read(a, bus).unwrap_or(bus)
            }
            _ if self.svp.is_some() && a & 0xFF_FF00 == 0xA1_5000 => {
                let bus = self.open_bus_word();
                self.svp.as_mut().and_then(|svp| svp.read_reg(a)).unwrap_or(bus)
            }
            _ if self.cart_hw.hw.regs_bank(a, false) => {
                let bus = self.open_bus_word();
                self.cart_hw.regs_read(a, bus, self.io.pad_buttons[0]).unwrap_or(bus)
            }
            _ if self.cart_hw.smw64.is_some() && (a >> 20) == 6 => match self.cart_hw.smw64_read(a) {
                Some(crate::cart_hw::CartRead::Rom(offset)) => {
                    u16::from_be_bytes([self.cart.offset_byte(offset), self.cart.offset_byte(offset | 1)])
                }
                Some(crate::cart_hw::CartRead::Byte(v)) => u16::from(v),
                _ => 0xFFFF,
            },
            _ => {
                self.unmapped_accesses += 1;
                self.observe_unmapped(a, 16, false);
                if self.z80_bus_active {
                    0xFFFF
                } else {
                    self.open_bus_word()
                }
            }
        }
    }

    fn write16(&mut self, addr: u32, value: u16) {
        let a = addr & 0x00FF_FFFF;
        match region(a) {
            Region::Cartridge if self.svp_bank(a) => {
                if let Some(svp) = self.svp.as_mut() {
                    match (a >> 16) & 0x3F {
                        0x30 => svp.write_dram_word(a, value),
                        0x31 => {
                            svp.write_dram_byte(a & !1, (value >> 8) as u8);
                            svp.write_dram_byte(a | 1, value as u8);
                        }
                        _ => {}
                    }
                }
            }
            Region::Cartridge if self.cart_hw.regs_write(a, value) => {}
            Region::Cartridge if self.cart_hw.sf_write(a, value) => {}
            Region::Cartridge if self.cart_hw.special_write(a, value, true) => {}
            Region::Cartridge => {
                self.cart_hw.write(a & !1, (value >> 8) as u8);
                self.cart_hw.write(a | 1, value as u8);
            }
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
                self.wram[(o + 1) & (WRAM_SIZE - 1)] = (value & 0xFF) as u8;
            }
            Region::Z80Ram | Region::Fm => self.write8(a, (value >> 8) as u8),
            Region::Vdp => match a & 0x1E {
                // 0xC00000/2 are the data port; 0xC00004/6 are the control
                // port, because a long write lands its high word there.
                0x00 | 0x02 => {
                    if self.cpu_bus_active {
                        self.pending_fifo_master_cycles +=
                            self.vdp.fifo_write_wait(self.pending_fifo_master_cycles + self.access_bias);
                    }
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
                0x04 | 0x06 => self.vdp_control_write(value),
                0x10 | 0x12 | 0x14 | 0x16 => { self.sound_event(crate::audio::EventKind::PsgWrite { value: value as u8 }); }
                _ => {
                    self.unmapped_accesses += 1;
                    self.observe_unmapped(a, 16, true);
                }
            },
            Region::Io => {
                self.io_write(a, (value >> 8) as u8);
                self.io_write(a + 1, (value & 0xFF) as u8);
            }
            Region::Sram => {
                let mask = self.cart.mirror_period() as u32 - 1;
                self.cart_hw.time_write(a, value, mask);
            }
            // Only D8 controls BUSREQ/RESET on a word write; the low byte is ignored.
            Region::Z80Ctrl => {
                if a >> 16 == 0xa0 {
                    self.write8(a, (value >> 8) as u8);
                } else {
                    self.z80_ctrl_write(a, (value >> 8) as u8);
                }
            }
            _ if self.svp.is_some() && a & 0xFF_FF00 == 0xA1_5000 => {
                if let Some(svp) = self.svp.as_mut() {
                    svp.write_reg(a, value);
                }
            }
            _ if self.cart_hw.regs_write(a, value) => {}
            _ if self.cart_hw.smw64_write(a, value) => {}
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
    /// FM-domain clock for an absolute master clock. The chip clock starts at
    /// zero at reset release while the VDP starts at the release offset.
    fn fm_domain(&self, absolute: u64) -> u64 {
        // Wrapping: a frame origin may precede the chip's zero; differences
        // of two converted clocks stay exact.
        absolute.wrapping_sub(self.fm_offset)
    }

    /// Clock of a 68000 access to the Z80 area: GPGX adds one CPU cycle of
    /// bus latency per access to the dispatch clock before the access, and its
    /// cycle counter is relative to the current frame. Returns FM-domain
    /// (access time, frame origin).
    fn cpu_z80_area_clock(&self) -> (u64, u64) {
        if !self.cpu_bus_active {
            return (self.fm.clock(), 0);
        }
        let frame = self.vdp.frame_master();
        let start = self.vdp.master_clock() + u64::from(self.access_bias);
        // A deferred refresh can carry the access past a frame edge the
        // reference has not processed yet: keep the dispatching frame's origin.
        let origin = self.vdp.master_clock() / frame * frame;
        (
            self.fm_domain(start + u64::from(self.pending_bus_cpu_cycles) * 7),
            self.fm_domain(origin),
        )
    }

    /// FM-domain (access time, frame origin) of the executing Z80 instruction:
    /// its dispatch clock plus the opcode's T-states (charged before memory
    /// accesses in the reference core), relative to the Z80's frame.
    fn z80_fm_clock(&self) -> (u64, u64) {
        if !self.z80_bus_active {
            return (self.fm.clock(), 0);
        }
        let (start, bytes) = self.z80_access;
        let time = start + u64::from(crate::z80::instruction_states(bytes)) * 15;
        (self.fm_domain(time), self.fm_domain(self.z80_sync_origin))
    }

    fn vdp_status(&mut self) -> u16 {
        // DMA busy is bit1, HBLANK bit2, VBLANK bit3, VINT bit7. The first
        // three are live device levels; reading does not clear them. A status
        // read resets the two-word control-command latch. FIFO full/empty uses
        // live deadlines.
        self.vdp.control_pending = false;
        // A 68000 read samples at dispatch plus the opcode's table cost.
        let offset = if self.cpu_bus_active && !self.interrupt_bus_active {
            crate::cpu::m68000::base_cycles(self.sound_prefetch_word) * 7 + self.access_bias
        } else {
            0
        };
        let mut status = self.vdp.status_bits_at(offset) | u16::from(self.vdp.pal);
        if self.vdp.vblank || !self.vdp.display_enabled() {
            status |= 0x0008;
        }
        // Sprite overflow/collision from the scanline renderer; read clears.
        status |= u16::from(std::mem::take(&mut self.vdp.raster.sprite_status));
        status
    }

    fn io_read(&self, a: u32) -> u8 {
        // GPGX io_68k_read: each byte register occupies an even/odd pair.
        // Default three-button pad, all buttons released; TH is an output
        // only when its control bit is set. D7 always reflects the data latch.
        match (a >> 1) & 0xF {
            0 => self.io.version_register,
            port @ (1 | 2) => {
                let index = port as usize - 1;
                let (data, control) = if index == 0 {
                    (self.io.data_a as u8, self.io.ctrl_a as u8)
                } else {
                    (self.io.data_b as u8, self.io.ctrl_b as u8)
                };
                let now = self.vdp.master_clock() + u64::from(self.access_bias);
                let pins = match self.io.gun.as_ref().filter(|_| index == 1) {
                    Some(gun) => gun.read(),
                    None => self.io.pads[index].read(self.io.pad_buttons[index], now, now / 3420),
                };
                let mask = 0x80 | control;
                (data & mask) | (pins & !mask)
            }
            3 => {
                // Nothing on the expansion port (GPGX dummy_read), but output
                // pins still read back the data latch.
                let mask = 0x80 | self.io.ctrl_c as u8;
                (self.io.data_c as u8 & mask) | (0x7F & !mask)
            }
            4 => self.io.ctrl_a as u8,
            5 => self.io.ctrl_b as u8,
            6 => self.io.ctrl_c as u8,
            reg @ 7..=15 => self.io.serial[reg as usize - 7],
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
        let now = self.vdp.master_clock() + u64::from(self.access_bias);
        let six = self.six_button_pads;
        match (a >> 1) & 0xF {
            1 => {
                self.io.data_a = value as u16;
                self.io.pads[0].write(value, self.io.ctrl_a as u8, six, now, now / 3420);
            }
            2 => {
                self.io.data_b = value as u16;
                if let Some(gun) = self.io.gun.as_mut() {
                    gun.write(value, self.io.ctrl_b as u8);
                } else {
                    self.io.pads[1].write(value, self.io.ctrl_b as u8, six, now, now / 3420);
                }
            }
            3 => self.io.data_c = value as u16,
            4 => {
                if self.io.ctrl_a as u8 != value {
                    self.io.ctrl_a = value as u16;
                    self.io.pads[0].write(self.io.data_a as u8, value, six, now, now / 3420);
                }
            }
            5 => {
                if self.io.ctrl_b as u8 != value {
                    self.io.ctrl_b = value as u16;
                    if let Some(gun) = self.io.gun.as_mut() {
                        gun.write(self.io.data_b as u8, value);
                    } else {
                        self.io.pads[1].write(self.io.data_b as u8, value, six, now, now / 3420);
                    }
                }
            }
            6 => self.io.ctrl_c = value as u16,
            // GPGX io_68k_write: TxData stored, S-Ctrl keeps its writable bits.
            reg @ (7 | 10 | 13) => self.io.serial[reg as usize - 7] = value,
            reg @ (9 | 12 | 15) => self.io.serial[reg as usize - 7] = value & 0xF8,
            _ => {}
        }
    }

    fn z80_bus_granted(&self) -> bool {
        self.io.z80_bus_request && !self.io.z80_reset
    }

    fn sound_window(address: u32) -> bool {
        matches!(region(address), Region::Z80Ram | Region::Fm)
            || (address >> 16 == 0xa0 && region(address) == Region::Z80Ctrl)
    }

    fn sound_open_bus_word(&self) -> u16 {
        // GPGX m68k_read_bus_* and BUSACK: the word at the 68000's current PC
        // (past any extension words already fetched), as for other open bus.
        self.open_bus_word()
    }

    fn sound_open_bus_byte(&self, address: u32) -> u8 {
        let word = self.sound_open_bus_word();
        if address & 1 == 0 {
            (word >> 8) as u8
        } else {
            word as u8
        }
    }

    fn z80_bank_write(&mut self, value: u8) {
        self.io.zbank = ((self.io.zbank >> 1) | ((u32::from(value) & 1) << 23)) & 0xff8000;
    }

    /// BUSREQ is A111xx, RESET is A112xx. Only the even byte lane is wired;
    /// the low byte of a word write cannot affect either signal.
    fn observe_z80_ram_write(&mut self, offset: u16, value: u8, writer: &'static str, pc: u32) {
        if self.z80_ram_watch.is_none() {
            return;
        }
        let before = self.z80_ram[usize::from(offset)];
        let (frame, master_cycle) = (self.vdp.frame, self.master_cycles.max(self.z80_master_clock));
        if let Some(watch) = self.z80_ram_watch.as_mut().filter(|w| w.matches(offset, pc)) {
            watch.observed += 1;
            if watch.records.len() < watch.max_records {
                watch.records.push(crate::observe::Z80RamWrite {
                    frame,
                    master_cycle,
                    z80_address: offset,
                    before,
                    value,
                    writer,
                    pc,
                });
            }
        }
    }

    fn observe_sound_command(
        &mut self,
        target: crate::observe::SoundCommandTarget,
        address: u32,
        z80_address: Option<u16>,
        value: u8,
    ) {
        let granted = self.z80_bus_granted();
        let (frame, master_cycle, pc) = (self.vdp.frame, self.master_cycles, self.cpu_active_pc);
        if let Some(log) = self.sound_commands.as_mut().filter(|log| log.wants(target, z80_address)) {
            log.push(crate::observe::SoundCommand {
                frame,
                master_cycle,
                pc,
                target,
                address,
                z80_address,
                value,
                z80_bus_granted: granted,
            });
        }
    }

    fn z80_ctrl_write(&mut self, a: u32, value: u8) {
        self.io_write_count += 1;
        if self.io_log.len() < self.observation_limit {
            self.io_log.push((a, value));
        }
        if a & 1 != 0 {
            return;
        }
        use crate::observe::SoundCommandTarget as T;
        match a & 0x00FF_FF00 {
            0x00A1_1100 => self.observe_sound_command(T::BusRequest, a, None, value),
            0x00A1_1200 => self.observe_sound_command(T::Reset, a, None, value),
            _ => {}
        }
        match a & 0x00FF_FF00 {
            0x00A1_1100 => {
                let request = value & 1 != 0;
                if request && !self.io.z80_bus_request && !self.io.z80_reset {
                    self.sync_z80(self.master_cycles);
                } else if !request && self.io.z80_bus_request && !self.io.z80_reset {
                    self.z80_master_clock = self.master_cycles.div_ceil(15) * 15;
                }
                self.io.z80_bus_request = request;
            }
            0x00A1_1200 => {
                let reset = value & 1 == 0;
                if reset && !self.io.z80_reset && !self.io.z80_bus_request {
                    self.sync_z80(self.master_cycles);
                }
                if !reset && self.io.z80_reset {
                    if !self.io.z80_bus_request {
                        self.z80_master_clock = self.master_cycles.div_ceil(15) * 15;
                    }
                    self.z80.reset();
                }
                if reset || self.io.z80_reset {
                    self.fm.reset();
                    self.sound_event(crate::audio::EventKind::FmReset);
                }
                self.io.z80_reset = reset;
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

    #[test]
    fn irregular_rom_lengths_preserve_raw_reset_vectors_and_first_instruction() {
        // Authored vectors/code, including odd and non-power-of-two lengths.
        // The vector bytes must agree with the literal file header, rather
        // than being remapped by a power-of-two mask before execution.
        for length in [1026, 1067, 196608, 957482, 1015094] {
            let mut bytes = vec![0; length];
            bytes[..4].copy_from_slice(&0xffff1000u32.to_be_bytes());
            bytes[4..8].copy_from_slice(&0x00000200u32.to_be_bytes());
            bytes[0x200..0x204].copy_from_slice(&[0x70, 0x2a, 0x60, 0xfe]);
            let mut system = System::new(Cartridge::from_bytes("authored-irregular.md", bytes).unwrap());
            assert_eq!(system.reset_vectors(), (0xffff1000, 0x200), "length {length}");
            assert_eq!(system.cpu.pc, 0x200);
            assert_eq!(system.cpu.prefetch[0], 0x702a);
            assert!(system.step_instruction().1.is_none());
            assert_eq!(system.cpu.d[0], 42);
            assert_eq!(system.cpu.pc, 0x202);
        }
    }

    #[test]
    fn diagnostic_word_helpers_are_bounded_at_odd_wram_mirror_edges() {
        let mut s = system();
        // Raw bus helpers may be reached by malformed vectors/diagnostics.
        // CPU address-error delivery is a separate concern; a mirrored-RAM
        // boundary must never index outside the host buffer.
        s.wram[0xffff] = 0xab;
        s.wram[0] = 0xcd;
        assert_eq!(s.read16(0xe0ffff), 0xabcd);
        s.write16(0xe1ffff, 0x1234);
        assert_eq!(s.read8(0xe0ffff), 0x12);
        assert_eq!(s.read8(0xe10000), 0x34);
    }

    fn grant_sound_bus(s: &mut System) {
        s.write16(0xa11100, 0x0100);
        s.write16(0xa11200, 0x0100);
    }

    #[test]
    fn busack_uses_even_lane_and_preserves_undriven_prefetch_bits() {
        let mut s = system();
        // Undriven bits come from the word at the 68000's current PC.
        s.wram[..2].copy_from_slice(&[0xa4, 0xb6]);
        s.open_bus_pc = 0xff0000;
        assert!(s.io.z80_reset);
        assert_eq!(s.read8(0xa11100), 0xa5);
        assert_eq!(s.read8(0xa11101), 0xb6, "odd lane is open bus, not BUSACK");
        assert_eq!(s.read16(0xa11100), 0xa5b6);
        s.write16(0xa11100, 0x0100);
        assert_eq!(s.read8(0xa11100) & 1, 1, "RESET prevents a bus grant");
        s.write16(0xa11200, 0x0100);
        assert_eq!(s.read8(0xa11100), 0xa4);
        assert_eq!(s.read8(0xa11101), 0xb6);
        assert_eq!(s.read16(0xa11100), 0xa4b6);
        assert_eq!(s.read16(0xa11200), 0xa4b6, "RESET reads are open bus");
        s.write8(0xa11101, 0);
        s.write8(0xa11201, 0);
        assert!(s.io.z80_bus_request);
        assert!(!s.io.z80_reset);
        s.write8(0xa11102, 0); // mirrored BUSREQ, not RESET
        assert!(!s.io.z80_bus_request);
        assert!(!s.io.z80_reset);
        s.write8(0xa112fe, 0); // mirrored RESET
        assert!(s.io.z80_reset);
    }

    #[test]
    fn sound_upload_word_lane_mirrors_and_access_grant_match_gpgx() {
        let mut s = system();
        s.wram[..2].copy_from_slice(&[0x12, 0x34]);
        s.open_bus_pc = 0xff0000;
        s.write8(0xa00000, 0xff);
        s.write8(0xa04000, 0x24);
        assert_eq!(s.z80_ram[0], 0);
        assert_eq!(s.fm.write_count, 0);
        assert_eq!(s.read8(0xa00000), 0x12);
        assert_eq!(s.read16(0xa00000), 0x1234);
        grant_sound_bus(&mut s);
        s.write16(0xa00000, 0xabc5);
        assert_eq!(&s.z80_ram[..2], &[0xab, 0]);
        assert_eq!(
            s.read16(0xa02000),
            0xabab,
            "one byte is replicated to both lanes"
        );
        assert_eq!(s.read8(0xa08000), 0xab);
        s.write16(0xa0c000, 0x24ee);
        s.write8(0xa05ffd, 0xff);
        s.write8(0xa04002, 0x30);
        s.write8(0xa04003, 0x77);
        assert_eq!(s.fm.timer_a, 0x3fc);
        assert_eq!(s.fm.registers[0x130], 0x77);
        assert_eq!(s.read16(0xa04000), 0x8080);
        s.write16(0xa11100, 0x0001); // D0 is ignored; D8 releases BUSREQ
        s.write8(0xa00000, 0x55);
        assert_eq!(s.z80_ram[0], 0xab);
        assert_eq!(s.read16(0xa04000), 0x1234);
    }

    #[test]
    fn each_granted_68000_sound_bus_access_adds_one_cpu_cycle() {
        let mut s = system();
        grant_sound_bus(&mut s);
        s.cpu_bus_active = true;
        s.pending_bus_cpu_cycles = 0;
        s.write16(0xa00000, 0x1234);
        s.read16(0xa00000);
        s.write8(0xa04000, 0x22);
        s.read8(0xa04000);
        s.write8(0xa06000, 1);
        assert_eq!(s.pending_bus_cpu_cycles, 5);
        s.write8(0xa11100, 0);
        s.read8(0xa04000);
        assert_eq!(
            s.pending_bus_cpu_cycles, 5,
            "no latency on ungranted access"
        );
    }

    #[test]
    fn bus_request_stops_z80_without_inventing_driver_ram_handshakes() {
        let mut s = system();
        s.z80_ram[0] = 0x00; // NOP
        s.z80_ram[0x1ffb] = 0x35;
        s.z80_ram[0x1fff] = 0xca;
        assert_eq!(s.step_z80(4), 0);
        s.write8(0xa11200, 1);
        assert_eq!(s.step_z80(4), 4);
        assert_eq!(s.z80.pc, 1);
        s.write8(0xa11100, 1);
        let ram = s.z80_ram.clone();
        assert_eq!(s.step_z80(40), 0);
        assert_eq!(s.z80.pc, 1);
        s.write8(0xa11100, 0);
        assert_eq!(s.z80_ram, ram);
        assert_eq!(s.z80_ram[0x1ffb], 0x35);
        assert_eq!(s.z80_ram[0x1fff], 0xca);
        assert_eq!(s.step_z80(4), 4);
        assert_eq!(s.z80.pc, 2);
        s.write8(0xa11200, 0);
        assert_eq!(s.step_z80(4), 0);
        s.write8(0xa11200, 1);
        assert_eq!(s.z80.pc, 0, "RESET release restarts the sound CPU at zero");
    }

    #[test]
    fn z80_memory_fm_reads_share_status_and_unused_io_ports_have_no_effect() {
        let mut s = system();
        Z80Bus::write(&mut s, 0x4000, 0x24);
        Z80Bus::write(&mut s, 0x4001, 0xff);
        Z80Bus::write(&mut s, 0x4000, 0x25);
        Z80Bus::write(&mut s, 0x4001, 3);
        Z80Bus::write(&mut s, 0x4000, 0x27);
        Z80Bus::write(&mut s, 0x4001, 5);
        assert_eq!(Z80Bus::read(&mut s, 0x4000), 0x80);
        s.advance_devices(2016);
        assert_eq!(Z80Bus::read(&mut s, 0x5ffc), 1);
        assert_eq!(Z80Bus::read(&mut s, 0x4000), 1);
        for address in 0x4001..=0x4003 {
            assert_eq!(Z80Bus::read(&mut s, address), 0);
        }
        let writes = s.fm.write_count;
        let data_a = s.io.data_a;
        for port in [0x01, 0x40, 0x80, 0x81, 0xf0, 0xf1] {
            Z80Bus::port_out(&mut s, port, 0xff);
            assert_eq!(Z80Bus::port_in(&mut s, port), 0xff);
        }
        assert_eq!(s.fm.write_count, writes);
        assert_eq!(s.io.data_a, data_a);
        s.write8(0xa11200, 0);
        assert_eq!(Z80Bus::read(&mut s, 0x4000), 0, "RESET clears timer status");
    }

    #[test]
    fn bank_serial_register_matches_from_both_cpu_views() {
        let mut s = system();
        grant_sound_bus(&mut s);
        let bank = 0x1a5u32;
        for bit in 0..9 {
            s.write8(0xa060ff, ((bank >> bit) & 1) as u8);
        }
        assert_eq!(s.io.zbank, bank << 15);
        s.io.zbank = 0;
        for bit in 0..9 {
            Z80Bus::write(&mut s, 0x6000, ((bank >> bit) & 1) as u8);
        }
        assert_eq!(s.io.zbank, bank << 15);
        assert_eq!(s.read8(0xa06000), 0xff);
    }

    #[test]
    fn sound_clock_budget_carries_instruction_overshoot_at_master_divide_fifteen() {
        let mut s = system();
        s.write8(0xa11200, 1);
        for _ in 0..150 {
            s.write8(0xa11100, 0); // unchanged signal must not erase timing debt
            s.advance_devices(7);
        }
        // 1050 master clocks permit 70 T-states, rounded once to 18 NOPs.
        s.write8(0xa11100, 1); // force reference synchronization before BUSACK
        assert_eq!(s.z80.cycles, 72);
        assert_eq!(s.z80.pc, 18);
        s.advance_devices(1500);
        assert_eq!(s.z80.cycles, 72);
        s.write8(0xa11100, 0);
        s.advance_devices(60);
        s.write8(0xa11100, 1);
        assert_eq!(s.z80.cycles, 76, "stopped time is not runnable Z80 credit");
    }
}
// ---------------------------------------------------------------------------
// The Z80's view of the machine
// ---------------------------------------------------------------------------

/// The Mega Drive's Z80 memory map, as Genesis Plus GX implements it:
/// 0x0000-0x3FFF is the 8 KiB of Z80 RAM mirrored twice, 0x4000-0x5FFF is
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
        if self.z80_bus_active && (addr >= 0x8000 || addr >> 8 == 0x7f) {
            self.z80_bus_requests += 1;
        }
        match (addr >> 13) & 7 {
            0 | 1 => self.z80_ram[(addr as usize) & (Z80_RAM_SIZE - 1)],
            2 => {
                let (time, _) = self.z80_fm_clock();
                self.fm.sync_to(time);
                self.fm.read_at((addr & 3) as usize, time)
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
        if self.z80_bus_active && (addr >= 0x8000 || addr >> 8 == 0x7f) {
            self.z80_bus_requests += 1;
        }
        match (addr >> 13) & 7 {
            0 | 1 => {
                let o = (addr as usize) & (Z80_RAM_SIZE - 1);
                self.observe_z80_ram_write(o as u16, value, "z80", u32::from(self.z80_active_pc));
                self.z80_ram[o] = value;
            }
            2 => {
                self.z80_sound_write_count += 1;
                if self.z80_sound_writes.len() < 32 {
                    self.z80_sound_writes
                        .push((addr, value, self.z80_active_pc));
                }
                let (time, origin) = self.z80_fm_clock();
                if addr & 1 != 0 {
                    self.fm.sync_to(time);
                }
                self.fm.write_at(addr as usize & 3, value, time, origin);
                self.sound_event(crate::audio::EventKind::FmWrite { port: (addr & 3) as u8, value });
            }
            3 => match addr >> 8 {
                0x60 => {
                    self.z80_bank_write(value);
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
        // GPGX Genesis mode uses z80_unused_port_w; SMS port decoding does
        // not apply here. The Genesis PSG is memory-mapped at 0x7f11.
    }

    fn in_reset(&self) -> bool {
        self.io.z80_reset
    }
}

impl System {
    /// Run sound at the reference's scanline/VINT synchronization boundaries.
    /// CPU overshoot is retained: a Z80 batch can stall the already-retired CPU
    /// while its own target remains the physical boundary, not that overshoot.
    fn sound_event(&mut self, event: crate::audio::EventKind) {
        if self.z80_bus_active {
            self.audio_step_events.push(event);
        } else {
            self.audio.enqueue(self.master_cycles, event);
        }
    }

    fn advance_devices(&mut self, master_cycles: u32) {
        self.advance_devices_inner(master_cycles, false);
    }

    fn advance_devices_inner(&mut self, master_cycles: u32, defer_sound: bool) {
        let mut target = self.master_cycles
            + u64::from(master_cycles)
            + u64::from(std::mem::take(&mut self.pending_z80_cpu_wait));
        let mut position = self.master_cycles;
        if !defer_sound {
            if let Some((horizon, irq)) = self.deferred_z80_sync.take() {
                self.master_cycles = target;
                self.z80.irq_pending = irq;
                self.sync_z80_in_frame(
                    horizon,
                    horizon.saturating_sub(1) / self.vdp.frame_master() * self.vdp.frame_master(),
                );
                if horizon % self.vdp.frame_master() == 0 {
                    self.fm.frame_edge(self.fm_domain(horizon));
                }
                self.z80.irq_pending = self.vdp.z80_irq;
                self.audio.advance_to(horizon);
                target += u64::from(std::mem::take(&mut self.pending_z80_cpu_wait));
            }
        }
        while position < target {
            let line = self.vdp.line_event_in_master_cycles();
            let sound = self.vdp.sound_event_in_master_cycles();
            let span = (target - position).min(u64::from(line.min(sound))) as u32;
            let irq = self.vdp.z80_irq;
            self.z80_dma_timeframe_end =
                position + u64::from(self.vdp.external_dma_timeframe_cycles());
            if span == line && self.svp.is_some() {
                // The reference runs the SSP after each line, before the next
                // line's DMA update reads its DRAM.
                self.run_svp_to((self.vdp.master_clock() + u64::from(span)) / 3420);
                self.refresh_dma_sources();
            }
            let frame = self.vdp.frame;
            self.vdp.advance_master_cycles(span);
            position += u64::from(span);
            if span == line && self.io.gun.is_some() {
                self.gun_refresh(frame != self.vdp.frame);
            }
            self.master_cycles = target;
            if (span == line || span == sound) && defer_sound {
                // Refresh changes the console/read phase before an opcode, but
                // must not run the sound batch until that opcode has retired.
                self.deferred_z80_sync = Some((position, irq));
            } else if span == line || span == sound {
                self.z80.irq_pending = irq;
                // An instruction may overshoot a frame boundary. The original
                // sound batch still uses the ending frame's clock origin.
                self.sync_z80_in_frame(
                    position,
                    position.saturating_sub(1) / self.vdp.frame_master() * self.vdp.frame_master(),
                );
                if position % self.vdp.frame_master() == 0 {
                    self.fm.frame_edge(self.fm_domain(position));
                }
                self.z80.irq_pending = self.vdp.z80_irq;
                self.audio.advance_to(position);
                target += u64::from(std::mem::take(&mut self.pending_z80_cpu_wait));
            }
            self.fm.tick(span);
        }
        self.master_cycles = target;
        // The reference runs the SSP after both CPUs finish each line.
        self.run_svp_to(self.vdp.master_clock() / 3420);
    }

    /// GPGX input_refresh for a light gun at a line start (and the libretro
    /// aim update at a frame edge): on the gun's line with port B's HL
    /// enabled, latch the HV counter and raise the external interrupt.
    fn gun_refresh(&mut self, frame_edge: bool) {
        let Some(gun) = self.io.gun.as_mut() else { return };
        if frame_edge {
            let width = if self.vdp.registers[12] & 1 != 0 { 320 } else { 256 };
            gun.frame_edge(width, self.vdp.visible_lines as i32);
        }
        let h40 = self.vdp.registers[12] & 1 != 0;
        match gun.hit(self.vdp.line as i32, self.vdp.visible_lines as i32, h40) {
            Some(hv) => {
                if self.io.ctrl_b & 0x80 != 0 {
                    if self.vdp.registers[11] & 0x08 != 0 {
                        if std::mem::take(&mut self.vdp.irq_line_written) {
                            self.io.ext_irq = false;
                        }
                        self.io.ext_irq = true;
                    }
                    self.vdp.gun_latch(hv);
                }
            }
            None => self.vdp.release_forced_latch(),
        }
    }

    fn run_svp_to(&mut self, line: u64) {
        if let Some(svp) = self.svp.as_mut() {
            while self.svp_line < line {
                svp.run(&self.cart.data, crate::svp::CYCLES_PER_LINE);
                self.svp_line += 1;
            }
        }
    }

    /// Re-read queued DMA words whose source is SVP DRAM (the SSP keeps
    /// writing it while the transfer waits for its line updates).
    fn refresh_dma_sources(&mut self) {
        let Some(svp) = self.svp.as_ref() else { return };
        let queued = self.vdp.dma_queue.len();
        while self.dma_queue_sources.len() > queued {
            self.dma_queue_sources.pop_front();
        }
        let offset = queued - self.dma_queue_sources.len();
        for (index, &address) in self.dma_queue_sources.iter().enumerate().take(4096) {
            if matches!((address >> 16) & 0x3F, 0x30 | 0x31) {
                self.vdp.dma_queue[offset + index] = svp.dram_word(address & 0x1FFFE);
            }
        }
    }

    fn sync_z80(&mut self, horizon: u64) {
        self.sync_z80_in_frame(horizon, horizon / self.vdp.frame_master() * self.vdp.frame_master());
    }

    fn sync_z80_in_frame(&mut self, horizon: u64, origin: u64) {
        if self.io.z80_reset || self.io.z80_bus_request || self.z80_bus_active {
            return;
        }
        self.z80_sync_origin = origin;
        while self.z80_master_clock < horizon {
            if self.step_sound_instruction().is_none() {
                break;
            }
        }
    }

    fn step_sound_instruction(&mut self) -> Option<u32> {
        let mut cpu = std::mem::take(&mut self.z80);
        let pc = cpu.pc;
        // Peek before execution: a block copy can overwrite its own bytes.
        let code = |offset: u16| self.z80_ram[pc.wrapping_add(offset) as usize & 8191];
        let mut prefix_bytes = 0;
        while prefix_bytes < 8192 && matches!(code(prefix_bytes), 0xdd | 0xfd) {
            prefix_bytes += 1;
        }
        let repeat_pc = pc.wrapping_add(prefix_bytes);
        let repeating = !cpu.halted
            && code(prefix_bytes) == 0xed
            && matches!(code(prefix_bytes + 1), 0xb0..=0xb3 | 0xb8..=0xbb);
        if !cpu.halted && self.z80_trace.is_some() {
            let frame = self.vdp.frame;
            let master_cycle = self.z80_master_clock;
            let bytes = [code(0), code(1), code(2), code(3)];
            if let Some(trace) = self.z80_trace.as_mut().filter(|t| {
                t.pc_range.is_none_or(|(start, end)| (start..end).contains(&pc))
            }) {
                trace.observed += 1;
                if trace.records.len() < trace.max_records {
                    trace.records.push(crate::observe::Z80TraceRecord {
                        frame,
                        master_cycle,
                        pc,
                        bytes,
                        af: cpu.af(),
                        bc: cpu.bc(),
                        de: cpu.de(),
                        hl: cpu.hl(),
                        ix: cpu.ix,
                        iy: cpu.iy,
                        sp: cpu.sp,
                    });
                }
            }
        }
        self.z80_active_pc = pc;
        self.z80_access = (self.z80_master_clock, [code(0), code(1), code(2), code(3)]);
        self.z80_bus_requests = 0;
        let cpu_active = self.cpu_bus_active;
        self.cpu_bus_active = false;
        self.z80_bus_active = true;
        let stepped = cpu.step(self);
        self.z80_bus_active = false;
        self.cpu_bus_active = cpu_active;
        if let Some(states) = stepped {
            // Repeating ED instructions charge their extra five T states after
            // the memory operation. Other supported RAM-driver operands occur
            // after the opcode's fixed processing clocks in the reference.
            let tail = if repeating && cpu.pc == repeat_pc {
                5
            } else {
                0
            };
            let mut access_clock = self.z80_master_clock + u64::from(states - tail) * 15;
            let requests = std::mem::take(&mut self.z80_bus_requests);
            let dma_frame_end = self.z80_dma_until.min(self.z80_dma_timeframe_end);
            let dma_continues =
                requests != 0 && self.dma_overlaps_instruction && access_clock < dma_frame_end;
            if dma_continues {
                // Subsequent DMA updates overwrite both the CPU wait and the
                // Z80's temporary debt while it remains waiting for the bus.
                // At the aggregate dispatch boundary it resumes at DMA end.
                access_clock = self.z80_dma_until;
            }
            for _ in 0..if dma_continues { 0 } else { requests } {
                access_clock = access_clock.max(dma_frame_end);
                let phase = access_clock.saturating_sub(self.z80_sync_origin) % 7;
                // An instruction can overshoot this DMA update's endpoint.
                // Its bus access completes now; a later DMA update absorbs
                // this CPU wait without blocking the next Z80 opcode early.
                if !(self.dma_overlaps_instruction && dma_frame_end < self.z80_dma_until) {
                    self.pending_z80_cpu_wait += ((phase + 72) / 7 * 7) as u32;
                }
                access_clock += 45;
            }
            for event in self.audio_step_events.drain(..) {
                self.audio.enqueue(access_clock, event);
            }
            self.z80_master_clock = access_clock
                + if dma_continues {
                    0
                } else {
                    u64::from(tail) * 15
                };
            cpu.cycles += u64::from(requests) * 3;
            self.z80 = cpu;
            Some(states + requests * 3)
        } else {
            debug_assert!(self.audio_step_events.is_empty());
            self.z80 = cpu;
            None
        }
    }

    /// Run a requested number of Z80 T states, retaining instruction overshoot.
    pub fn step_z80(&mut self, cycles: u32) -> u32 {
        if self.io.z80_bus_request || self.io.z80_reset {
            return 0;
        }
        self.z80_sync_origin = self.master_cycles / self.vdp.frame_master() * self.vdp.frame_master();
        let mut spent = 0;
        while spent < cycles {
            if let Some(n) = self.step_sound_instruction() {
                spent += n;
            } else {
                break;
            }
        }
        spent
    }
}

#[cfg(test)]
mod sound_irq_tests {
    use super::*;
    fn machine(wide: bool) -> System {
        let mut s = System::new(crate::test_support::cartridge());
        s.vdp.control_write(12, u8::from(wide));
        // DI; LD SP,1FFD; IM1; EI; HALT; JR HALT.
        s.z80_ram[..11].copy_from_slice(&[
            0xf3, 0x31, 0xfd, 0x1f, 0xed, 0x56, 0xfb, 0x76, 0x18, 0xfd, 0,
        ]);
        // Count pulses; a DJNZ delay outlasts the interrupt scanline.
        s.z80_ram[0x38..0x43].copy_from_slice(&[
            0x21, 0x80, 0x01, 0x34, 0x06, 0x00, 0x10, 0xfe, 0xfb, 0xed, 0x4d,
        ]);
        s.io.z80_reset = false;
        s.observation_limit = 0;
        s
    }
    #[test]
    fn vblank_int_is_delayed_finite_and_independent_of_68k_enable() {
        for (wide, delay) in [(false, 770), (true, 788)] {
            let mut s = machine(wide);
            assert_eq!(s.vdp.registers[1] & 0x20, 0);
            s.advance_devices((896040 - s.master_cycles()) as u32);
            assert_eq!(s.vdp.frame, 1);
            assert!(s.z80.halted);
            s.advance_devices(delay - 1);
            assert!(!s.z80.irq_pending);
            s.advance_devices(1);
            assert!(s.z80.irq_pending && s.vdp.z80_irq);
            assert_eq!(s.z80_ram[0x180], 0);
            s.advance_devices(3420 - delay);
            assert!(!s.z80.irq_pending && !s.vdp.z80_irq);
            assert_eq!(s.z80_ram[0x180], 1);
            // A large wait crosses multiple assertions/clears in order.
            s.advance_devices(3 * 896040);
            assert_eq!(s.z80_ram[0x180], 4);
        }
    }
    #[test]
    fn busreq_and_reset_do_not_latch_an_expired_interrupt() {
        for reset in [false, true] {
            let mut s = machine(false);
            s.advance_devices((896040 - s.master_cycles()) as u32);
            if reset {
                s.io.z80_reset = true;
            } else {
                s.io.z80_bus_request = true;
            }
            s.advance_devices(3420);
            s.io.z80_reset = false;
            s.io.z80_bus_request = false;
            s.advance_devices(10000);
            assert_eq!(s.z80_ram[0x180], 0);
            assert!(!s.z80.irq_pending);
            s.advance_devices(896040);
            assert_eq!(s.z80_ram[0x180], 1);
        }
    }
    #[test]
    fn state_preserves_int_level_and_ei_shadow_across_both_edges() {
        for offset in [769, 770, 800, 3419, 3420] {
            let mut live = machine(false);
            live.advance_devices((896040 - live.master_cycles()) as u32 + offset);
            live.z80.after_ei = true;
            let bytes = serde_json::to_vec(&live.save_state().unwrap()).unwrap();
            let mut restored =
                System::from_state(live.cart.clone(), serde_json::from_slice(&bytes).unwrap())
                    .unwrap();
            for span in [1, 50, 3000, 900000] {
                live.advance_devices(span);
                restored.advance_devices(span);
                assert_eq!(
                    serde_json::to_value(live.save_state().unwrap()).unwrap(),
                    serde_json::to_value(restored.save_state().unwrap()).unwrap()
                );
            }
        }
    }
    #[test]
    fn authored_irq_modes_match_unchanged_oracle_counter_samples() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/z80_vblank.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let mut s = machine(case["wide"].as_bool().unwrap());
            s.vdp.control_write(1, 4);
            s.z80_ram[5] = [0x46, 0x56, 0x5e][case["mode"].as_u64().unwrap() as usize];
            s.z80_ram[0xff] = 0x38;
            s.z80_ram[0x100] = 0;
            s.advance_devices(
                (case["frames"].as_u64().unwrap() * 896040 - s.master_cycles()) as u32,
            );
            assert_eq!(
                u64::from(s.z80_ram[0x180]),
                case["expected_counter"].as_u64().unwrap(),
                "{case}"
            );
        }
    }
}

#[cfg(test)]
mod psg_decode_tests {
    use super::*;
    #[test]
    fn psg_writes_from_both_cpus_do_not_corrupt_the_vdp() {
        let mut s = System::new(crate::test_support::cartridge());
        s.vdp.control_pending = true;
        s.vdp.address_latch = 0x1234;
        s.vdp.vint_pending = true;
        let original = serde_json::to_value(&s.vdp).unwrap();
        for address in 0xc00010..=0xc00017 {
            s.write8(address, 0x81);
            s.write16(address & !1, 0xa100);
        }
        s.io.zbank = 0xc00000;
        for address in [0x8010, 0x8011, 0x8014, 0x8017, 0x7f11, 0x7f15] {
            Z80Bus::write(&mut s, address, 0x9f);
        }
        assert_eq!(serde_json::to_value(&s.vdp).unwrap(), original);
    }
    #[test]
    fn vdp_byte_data_write_duplicates_the_byte_without_reading_the_port() {
        let mut s = System::new(crate::test_support::cartridge());
        s.vdp.control_write(15, 2);
        s.vdp.code = 1;
        s.vdp.address_latch = 0x100;
        s.write8(0xc00000, 0x5a);
        assert_eq!(&s.vdp.vram[0x100..0x104], &[0x5a, 0x5a, 0, 0]);
        assert_eq!(s.vdp.address_latch, 0x102);
        s.write8(0xc00001, 0xa5);
        assert_eq!(&s.vdp.vram[0x102..0x104], &[0xa5, 0xa5]);
        assert_eq!(s.vdp.address_latch, 0x104);
    }
}
