//! The 68000 core.
//!
//! Prefetch model. The invariant maintained between instructions is:
//!
//! ```text
//! prefetch[0] == read16(pc)      // the word about to be executed
//! prefetch[1] == read16(pc + 2)  // the word after it
//! ```
//!
//! Consuming a word rotates the queue and loads one new word, exactly as the
//! hardware does. Jumps reload the queue instead of rotating it.

use crate::cpu::bus::Bus;
use crate::cpu::ea::{Ea, Size};
use crate::cpu::status::{StatusRegister, C, N, V, X, Z};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    /// An odd address was used for a word or long operand.
    AddressError {
        address: u32,
        is_write: bool,
    },
    /// A supervisor-only instruction ran in user mode.
    Privileged,
    Illegal {
        opcode: u16,
    },
    DivideByZero,
    /// `CHK` or `TRAP`, with the vector number to take.
    Vector {
        vector: u8,
    },
}

/// The 68000 programmer's model state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M68000 {
    pub d: [u32; 8],
    pub a: [u32; 8],
    /// Address of the word in `prefetch[0]`.
    pub pc: u32,
    pub sr: StatusRegister,
    /// User stack pointer, saved whenever the processor drops from supervisor.
    pub usp: u32,
    /// Inactive supervisor stack while executing in user mode.
    pub ssp: u32,
    pub prefetch: [u16; 2],
    /// Set while stopped by a `STOP` instruction.
    pub stopped: bool,
    /// Analyst-forced outcomes for conditional branches, keyed by address.
    pub branch_override: Vec<(u32, bool)>,
    /// Variable processing/transfer clocks during which a refresh is skipped.
    pub(crate) refresh_block_cycles: Option<u32>,
    /// SR was written by the current instruction, requiring an IRQ recheck.
    pub(crate) status_written: bool,
    /// Clocks a trap-class exception (CHK, TRAPV, divide by zero) spent
    /// before exception processing. Transient within one step.
    #[serde(skip)]
    pub(crate) trap_cycles: u32,
    /// Address error raised by the current instruction (transient).
    #[serde(skip)]
    pub(crate) fault: Option<Fault>,
    /// The faulting instruction's register file and opcode, for the frame.
    #[serde(skip)]
    pub(crate) last_fault: Option<(Fault, u16)>,
    /// Inside address-error processing until RTE: a second fault halts.
    #[serde(default)]
    pub(crate) aerr_mode: bool,
    /// Interrupt processing since the last RTE (status word I/N bit).
    #[serde(default)]
    pub(crate) instr_mode_no: bool,
    /// Halted by a double fault; only reset clears it.
    #[serde(default)]
    pub halted: bool,
}

/// Reason a `step` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepOutcome {
    pub cycles: u32,
    /// Number of extension words consumed beyond the opcode.
    pub extension_words: u32,
    pub exception: Option<Exception>,
}

impl M68000 {
    /// Power-on state. `pc` must already point at an even address whose
    /// contents are a valid opcode.
    pub fn new(pc: u32) -> Self {
        Self {
            d: [0; 8],
            a: [0; 8],
            pc,
            sr: StatusRegister(0x2700),
            usp: 0,
            ssp: 0,
            prefetch: [0, 0],
            stopped: false,
            branch_override: Vec::new(),
            refresh_block_cycles: None,
            status_written: false,
            trap_cycles: 0,
            fault: None,
            last_fault: None,
            aerr_mode: false,
            instr_mode_no: false,
            halted: false,
        }
    }

    /// Force the conditional branch at `addr` to be taken or not taken,
    /// regardless of the condition codes.
    ///
    /// This exists for cartridges whose shipped integrity check fails on the
    /// image in hand: the fault branch is the faithful behaviour, but the
    /// alternate path is the one worth analysing. It is a diagnostic, not an
    /// emulation fix, and anything derived under an override is suspect.
    pub fn override_branch(&mut self, addr: u32, taken: bool) {
        self.branch_override.push((addr & !1, taken));
    }

    /// The analyst-chosen outcome for the branch at `pc`, if one was forced.
    pub fn branch_override_at(&self, pc: u32) -> Option<bool> {
        self.branch_override
            .iter()
            .rev()
            .find(|(a, _)| *a == (pc & !1))
            .map(|(_, taken)| *taken)
    }

    /// Load the prefetch queue for the instruction about to be executed.
    pub fn prime<B: Bus>(&mut self, bus: &mut B) {
        self.prefetch[0] = bus.read16(self.pc);
        self.prefetch[1] = bus.read16(self.pc.wrapping_add(2));
    }

    /// Redirect execution, reloading the prefetch queue rather than rotating it.
    #[inline]
    pub fn set_pc<B: Bus>(&mut self, bus: &mut B, addr: u32) {
        self.pc = addr & !1;
        bus.note_pc(self.pc);
        self.prime(bus);
    }

    /// Consume the next instruction-stream word, rotating the prefetch queue.
    ///
    /// The opcode has already been consumed, so on entry `prefetch[0]` is the
    /// word at `pc` -- the first extension word. Consuming it moves the queue
    /// on by one, and the word to load afterwards is two beyond the current pc.
    #[inline]
    pub(crate) fn next_word<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let word = self.prefetch[0];
        self.prefetch[0] = self.prefetch[1];
        self.prefetch[1] = bus.read16(self.pc.wrapping_add(4));
        self.pc = self.pc.wrapping_add(2);
        bus.note_pc(self.pc);
        word
    }

    /// Take an autovectored interrupt at the given level.
    ///
    /// Stack the next instruction's PC followed by the old SR, so the six-byte
    /// frame starts with SR then PC. Enter supervisor mode and mask this level.
    pub fn interrupt<B: Bus>(&mut self, bus: &mut B, level: u8) {
        self.instr_mode_no = true;
        let old_sr = self.sr.0;
        self.set_status((old_sr & !0x8700) | 0x2000 | ((level as u16) << 8));
        let sp = self.a[7];
        bus.write32(sp.wrapping_sub(4), self.pc);
        bus.write16(sp.wrapping_sub(6), old_sr);
        self.a[7] = sp.wrapping_sub(6);
        self.stopped = false;
        let vector = u32::from(24 + level) * 4;
        let addr = bus.read32(vector);
        self.set_pc(bus, addr);
    }

    /// Group-0 address error entry (reference m68ki_exception_address_error):
    /// PC, SR, IR, access address and the R/W, I/N, FC status word, then
    /// vector 3. A second fault before RTE halts the processor.
    pub(crate) fn address_error_entry<B: Bus>(&mut self, bus: &mut B, fault: Fault, ir: u16) {
        if self.aerr_mode {
            self.halted = true;
            self.stopped = true;
            return;
        }
        self.aerr_mode = true;
        let old_sr = self.sr.0;
        let fc = if old_sr & 0x2000 != 0 { 5 } else { 1 };
        self.set_status((old_sr & !0x8000) | 0x2000);
        let status = if fault.write { 0 } else { 0x10 } | if self.instr_mode_no { 0x08 } else { 0 } | fc;
        let mut sp = self.a[7];
        sp = sp.wrapping_sub(4);
        bus.write32(sp, fault.pc);
        sp = sp.wrapping_sub(2);
        bus.write16(sp, old_sr);
        sp = sp.wrapping_sub(2);
        bus.write16(sp, ir);
        sp = sp.wrapping_sub(4);
        bus.write32(sp, fault.address);
        sp = sp.wrapping_sub(2);
        bus.write16(sp, status);
        self.a[7] = sp;
        self.stopped = false;
        let handler = bus.read32(3 * 4);
        self.set_pc(bus, handler);
    }

    /// Enter a synchronous handler with the three-word 68000 exception frame.
    /// The caller supplies the architecturally stacked fault/return PC.
    pub(crate) fn exception_entry<B: Bus>(&mut self, bus: &mut B, vector: u8, return_pc: u32) {
        debug_assert!((4..=47).contains(&vector));
        let old_sr = self.sr.0;
        self.set_status((old_sr & !0x8000) | 0x2000);
        let sp = self.a[7];
        bus.write32(sp.wrapping_sub(4), return_pc);
        bus.write16(sp.wrapping_sub(6), old_sr);
        self.a[7] = sp.wrapping_sub(6);
        self.stopped = false;
        let handler = bus.read32(u32::from(vector) * 4);
        self.set_pc(bus, handler);
    }

    /// Write SR's implemented bits and swap active stacks when S changes.
    pub(crate) fn set_status(&mut self, value: u16) {
        self.status_written = true;
        let value = value & 0xA71F;
        let was_supervisor = self.sr.is_supervisor();
        let supervisor = value & 0x2000 != 0;
        if was_supervisor && !supervisor {
            self.ssp = self.a[7];
            self.a[7] = self.usp;
        } else if !was_supervisor && supervisor {
            self.usp = self.a[7];
            self.a[7] = self.ssp;
        }
        self.sr.0 = value;
    }

    /// Execute exactly one instruction.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> StepOutcome {
        self.refresh_block_cycles = None;
        self.status_written = false;
        self.trap_cycles = 0;
        if self.stopped {
            return StepOutcome::default();
        }
        let start = self.pc;
        let ir = self.prefetch[0];
        let result = self.execute(bus);
        if let Some(f) = self.fault.take() {
            // Abort at the faulting access: keep only what happened before it.
            self.d = f.d;
            self.a = f.a;
            self.sr.0 = f.sr;
            self.usp = f.usp;
            self.ssp = f.ssp;
            self.pc = f.pc;
            self.last_fault = Some((f, ir));
            return StepOutcome {
                cycles: 0,
                extension_words: 0,
                exception: Some(Exception::AddressError { address: f.address, is_write: f.write }),
            };
        }
        match result {
            Ok((cycles, ext)) => StepOutcome {
                cycles,
                extension_words: ext,
                exception: None,
            },
            Err(e) => {
                // Trap-class exceptions complete the instruction and stack the
                // next PC; faults abort it and stack the instruction's own PC.
                let trap = matches!(e, Exception::DivideByZero | Exception::Vector { .. });
                if !trap {
                    self.pc = start;
                }
                self.prefetch[0] = bus.read16(self.pc);
                self.prefetch[1] = bus.read16(self.pc.wrapping_add(2));
                StepOutcome {
                    cycles: if trap { self.trap_cycles } else { 0 },
                    extension_words: 0,
                    exception: Some(e),
                }
            }
        }
    }

    // ---------------------------------------------------------------- flags

    /// Sign and zero only.
    #[inline]
    pub(crate) fn set_nz(&mut self, size: Size, value: u32) {
        let masked = value & size.mask();
        self.sr
            .set_flag(N, masked & (1 << (size.bytes() * 8 - 1)) != 0);
        self.sr.set_flag(Z, masked == 0);
    }

    /// Logical operation: N and Z from the result, V and C cleared.
    #[inline]
    pub(crate) fn set_logic(&mut self, size: Size, value: u32) {
        self.set_nz(size, value);
        self.sr.set_flag(V, false);
        self.sr.set_flag(C, false);
    }

    /// `dst = src + dst`, setting all condition codes including X.
    pub(crate) fn add_flags(&mut self, size: Size, src: u32, dst: u32) -> u32 {
        let mask = size.mask();
        let src = src & mask;
        let dst = dst & mask;
        let sign = 1u64 << (size.bytes() * 8 - 1);
        let full = src as u64 + dst as u64;
        let result = full & mask as u64;
        self.set_nz(size, result as u32);
        self.sr.set_flag(C, full > mask as u64);
        let (sa, sb, so) = (src as u64 & sign, dst as u64 & sign, result & sign);
        self.sr.set_flag(V, sa == sb && so != sa);
        self.sr.set_flag(X, self.sr.flag(C));
        result as u32
    }

    /// `dst = src - dst` with borrow semantics, setting all condition codes.
    pub(crate) fn sub_flags(&mut self, size: Size, src: u32, dst: u32) -> u32 {
        let mask = size.mask();
        let src = src & mask;
        let dst = dst & mask;
        let sign = 1u64 << (size.bytes() * 8 - 1);
        let result = src.wrapping_sub(dst) & mask;
        self.set_nz(size, result);
        self.sr.set_flag(C, src < dst);
        let (sa, sb, so) = (src as u64 & sign, dst as u64 & sign, result as u64 & sign);
        self.sr.set_flag(V, sa != sb && so != sa);
        self.sr.set_flag(X, self.sr.flag(C));
        result
    }

    /// Compare updates NZVC like subtraction while preserving X.
    pub(crate) fn compare_flags(&mut self, size: Size, left: u32, right: u32) {
        let extend = self.sr.flag(X);
        self.sub_flags(size, left, right);
        self.sr.set_flag(X, extend);
    }

    // ------------------------------------------------------------ EA access

    /// Read the operand an effective address denotes.
    pub(crate) fn read_ea<B: Bus>(&mut self, bus: &mut B, ea: &Ea, size: Size) -> u32 {
        match ea {
            Ea::Data(r) => self.d[*r as usize] & size.mask(),
            Ea::Address(r) => self.a[*r as usize] & size.mask(),
            Ea::Memory(addr) => self.read_sized(bus, *addr, size),
            Ea::Immediate(v) => *v & size.mask(),
        }
    }

    /// Write through an effective address. Direct-register operands are read
    /// only; silently ignoring the write matches the hardware.
    /// Write the result of an operation whose destination may be an address
    /// register, as ADDQ and SUBQ allow. `write_ea` ignores address
    /// registers because a plain MOVE cannot target one, which is right for
    /// data transfer and wrong here.
    pub(crate) fn write_destination<B: Bus>(
        &mut self,
        bus: &mut B,
        ea: &Ea,
        size: Size,
        value: u32,
    ) {
        match ea {
            Ea::Address(r) => {
                let v = value & size.mask();
                self.a[*r as usize] = match size {
                    Size::Byte => (self.a[*r as usize] & !0xFF) | v,
                    Size::Word => (self.a[*r as usize] & !0xFFFF) | v,
                    Size::Long => v,
                };
            }
            _ => self.write_ea(bus, ea, size, value),
        }
    }

    /// Write through an effective address. Direct-register operands are read
    /// only; silently ignoring the write matches the hardware.
    pub(crate) fn write_ea<B: Bus>(&mut self, bus: &mut B, ea: &Ea, size: Size, value: u32) {
        match ea {
            Ea::Data(r) => {
                let v = value & size.mask();
                self.d[*r as usize] = match size {
                    Size::Byte => (self.d[*r as usize] & !0xFF) | v,
                    Size::Word => (self.d[*r as usize] & !0xFFFF) | v,
                    Size::Long => v,
                };
            }
            Ea::Address(_) | Ea::Immediate(_) => {}
            Ea::Memory(addr) => self.write_sized(bus, *addr, size, value),
        }
    }

    /// Read-modify-write helper for instructions that can target a direct
    /// register: returns the old value, then applies `f` to produce the new one.
    pub(crate) fn modify_ea<B: Bus>(
        &mut self,
        bus: &mut B,
        ea: &Ea,
        size: Size,
        f: impl Fn(u32) -> u32,
    ) {
        match ea {
            Ea::Data(r) => {
                let old = self.d[*r as usize] & size.mask();
                let new = f(old) & size.mask();
                self.write_ea(bus, ea, size, new);
            }
            Ea::Memory(addr) => {
                let old = self.read_sized(bus, *addr, size);
                let new = f(old) & size.mask();
                self.write_sized(bus, *addr, size, new);
            }
            _ => {}
        }
    }
}

/// Size-aware read.
///
/// Word and long operands are composed from bytes without an alignment check.
/// The 68000 raises an address error for an odd word or long operand; that
/// exception is not yet wired through, and silently dropping such an access
/// would hide bugs rather than surface them.
/// The 68000 drives only A1-A23; bits above that do not reach the bus.
pub const ADDR_MASK: u32 = 0x00FF_FFFF;

/// Register file captured at a faulting access (68000 address error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault {
    pub address: u32,
    pub write: bool,
    pub d: [u32; 8],
    pub a: [u32; 8],
    pub sr: u16,
    pub usp: u32,
    pub ssp: u32,
    pub pc: u32,
}

impl M68000 {
    /// An odd word/long data access aborts the instruction at that point
    /// (the reference longjmps out): later accesses in the same instruction
    /// are suppressed and the registers as of the fault are restored after it.
    fn fault_check(&mut self, addr: u32, write: bool) -> bool {
        if self.fault.is_some() {
            return true;
        }
        if addr & 1 != 0 {
            self.fault = Some(Fault {
                address: addr,
                write,
                d: self.d,
                a: self.a,
                sr: self.sr.0,
                usp: self.usp,
                ssp: self.ssp,
                pc: self.pc,
            });
            return true;
        }
        false
    }

    pub(crate) fn rd8<B: Bus>(&mut self, bus: &mut B, addr: u32) -> u8 {
        if self.fault.is_some() { 0 } else { bus.read8(addr) }
    }
    pub(crate) fn rd16<B: Bus>(&mut self, bus: &mut B, addr: u32) -> u16 {
        if self.fault_check(addr, false) { 0 } else { bus.read16(addr) }
    }
    pub(crate) fn rd32<B: Bus>(&mut self, bus: &mut B, addr: u32) -> u32 {
        if self.fault_check(addr, false) { 0 } else { bus.read32(addr) }
    }
    pub(crate) fn wr8<B: Bus>(&mut self, bus: &mut B, addr: u32, value: u8) {
        if self.fault.is_none() { bus.write8(addr, value) }
    }
    pub(crate) fn wr16<B: Bus>(&mut self, bus: &mut B, addr: u32, value: u16) {
        if !self.fault_check(addr, true) { bus.write16(addr, value) }
    }
    pub(crate) fn wr32<B: Bus>(&mut self, bus: &mut B, addr: u32, value: u32) {
        if !self.fault_check(addr, true) { bus.write32(addr, value) }
    }

    pub(crate) fn read_sized<B: Bus>(&mut self, bus: &mut B, addr: u32, size: Size) -> u32 {
        match size {
            Size::Byte => self.rd8(bus, addr) as u32,
            Size::Word => self.rd16(bus, addr) as u32,
            Size::Long => self.rd32(bus, addr),
        }
    }

    pub(crate) fn write_sized<B: Bus>(&mut self, bus: &mut B, addr: u32, size: Size, value: u32) {
        match size {
            Size::Byte => self.wr8(bus, addr, value as u8),
            Size::Word => self.wr16(bus, addr, value as u16),
            Size::Long => self.wr32(bus, addr, value),
        }
    }
}

#[cfg(test)]
mod interrupt_tests {
    use super::*;
    use crate::cpu::SparseBus;

    #[test]
    fn interrupt_frame_matches_gpgx_and_rte_resumes_the_next_instruction() {
        let mut bus = SparseBus::new();
        bus.write32((24 + 6) * 4, 0x100);
        bus.write16(0x100, 0x4E73); // RTE
        bus.write16(0x402, 0x4E71); // interrupted next instruction
        let mut cpu = M68000::new(0x402);
        cpu.a[7] = 0x2000;
        cpu.sr.0 = 0x2301;
        cpu.stopped = true;
        cpu.interrupt(&mut bus, 6);
        assert_eq!(cpu.pc, 0x100);
        assert_eq!(cpu.sr.0, 0x2601);
        assert_eq!(cpu.a[7], 0x1FFA);
        assert_eq!(bus.read16(0x1FFA), 0x2301);
        assert_eq!(bus.read32(0x1FFC), 0x402);
        assert!(!cpu.stopped);
        assert!(cpu.step(&mut bus).exception.is_none());
        assert_eq!((cpu.pc, cpu.a[7], cpu.sr.0), (0x402, 0x2000, 0x2301));
    }
}

/// Opcode cost with zeroed registers and memory: the reference's
/// `CYC_INSTRUCTION[IR]` for fixed-cost instructions (memory reads included).
/// GPGX samples the VDP status at dispatch time plus this cost.
pub fn base_cycles(opcode: u16) -> u32 {
    static TABLE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=0xFFFFu32)
            .map(|op| {
                let mut bus = super::bus::SparseBus::new();
                bus.write16(0x1000, op as u16);
                let mut cpu = M68000::new(0x1000);
                cpu.sr.0 = 0x2700;
                cpu.prime(&mut bus);
                let outcome = cpu.step(&mut bus);
                if outcome.exception.is_some() { 0 } else { outcome.cycles as u16 }
            })
            .collect()
    })[usize::from(opcode)]
    .into()
}
