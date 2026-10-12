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
#[derive(Debug, Clone)]
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
        word
    }

    /// Take an autovectored interrupt at the given level.
    ///
    /// Stack the next instruction's PC followed by the old SR, so the six-byte
    /// frame starts with SR then PC. Enter supervisor mode and mask this level.
    pub fn interrupt<B: Bus>(&mut self, bus: &mut B, level: u8) {
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

    /// Write SR's implemented bits and swap active stacks when S changes.
    pub(crate) fn set_status(&mut self, value: u16) {
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
        if self.stopped {
            return StepOutcome::default();
        }
        let start = self.pc;
        match self.execute(bus) {
            Ok((cycles, ext)) => StepOutcome {
                cycles,
                extension_words: ext,
                exception: None,
            },
            Err(e) => {
                // An exception aborts the instruction; restore the queue.
                self.pc = start;
                self.prefetch[0] = bus.read16(start);
                self.prefetch[1] = bus.read16(start.wrapping_add(2));
                StepOutcome {
                    cycles: 0,
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
            Ea::Memory(addr) => read_sized(bus, *addr, size),
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
            Ea::Memory(addr) => write_sized(bus, *addr, size, value),
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
                let old = read_sized(bus, *addr, size);
                let new = f(old) & size.mask();
                write_sized(bus, *addr, size, new);
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

pub(crate) fn read_sized<B: Bus>(bus: &mut B, addr: u32, size: Size) -> u32 {
    let addr = addr & ADDR_MASK;
    match size {
        Size::Byte => bus.read8(addr) as u32,
        Size::Word => bus.read16(addr) as u32,
        Size::Long => bus.read32(addr),
    }
}

pub(crate) fn write_sized<B: Bus>(bus: &mut B, addr: u32, size: Size, value: u32) {
    let addr = addr & ADDR_MASK;
    match size {
        Size::Byte => bus.write8(addr, value as u8),
        Size::Word => bus.write16(addr, value as u16),
        Size::Long => bus.write32(addr, value),
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
